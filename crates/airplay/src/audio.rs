//! Audio stream (SETUP stream type 96): RTP over UDP, AES-CBC encrypted,
//! decoded here to PCM and handed to the application's [`AudioSink`].
//!
//! Data packets: 12-byte RTP header (payload type 96, big-endian sequence
//! number and timestamp) followed by the encrypted frame. Only whole 16-byte
//! blocks are encrypted; the tail is sent in the clear. While mirroring, iOS
//! sends AAC-ELD and transmits every packet three times (0, 0 1, 0 1 2, ...),
//! so duplicates are dropped by sequence number.

use std::sync::Arc;

use aes::Aes128;
use aes::cipher::{BlockDecrypt, KeyInit};
use rusty_aac::AacDecoder;
use tokio::net::UdpSocket;

use crate::AudioSink;

pub const SAMPLE_RATE: u32 = 44_100;
pub const CHANNELS: u16 = 2;

/// Sent before the first real frame instead of audio data.
const NO_DATA_MARKER: [u8; 4] = [0x00, 0x68, 0x34, 0x00];
/// Control-port packet type carrying a retransmitted data packet.
const RETRANSMIT: u8 = 0x56;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioCodec {
    AacEld,
    AacLc,
    Alac,
}

impl AudioCodec {
    /// From the `ct` (compression type) field of the stream SETUP.
    pub fn from_compression_type(ct: u64) -> Option<Self> {
        match ct {
            2 => Some(Self::Alac),
            4 => Some(Self::AacLc),
            8 => Some(Self::AacEld),
            _ => None,
        }
    }

    /// MPEG-4 AudioSpecificConfig: 44.1 kHz stereo; ELD with 480-sample frames.
    fn audio_specific_config(self) -> Option<&'static [u8]> {
        match self {
            Self::AacEld => Some(&[0xF8, 0xE8, 0x50, 0x00]),
            Self::AacLc => Some(&[0x12, 0x10]),
            Self::Alac => None,
        }
    }
}

pub struct AudioStream {
    pub data: UdpSocket,
    pub control: UdpSocket,
    pub key: [u8; 16],
    pub iv: [u8; 16],
    pub codec: AudioCodec,
}

pub async fn run(stream: AudioStream, sink: Arc<dyn AudioSink>) {
    let decoder = stream.codec.audio_specific_config().map(AacDecoder::with_config_bytes);
    let mut decoder = match decoder {
        Some(Ok(decoder)) => Some(decoder),
        Some(Err(e)) => {
            tracing::error!("cannot set up the {:?} decoder: {e}", stream.codec);
            None
        }
        None => {
            tracing::warn!("{:?} audio is not supported yet; playing video only", stream.codec);
            None
        }
    };

    if decoder.is_some() {
        sink.start(SAMPLE_RATE, CHANNELS);
    }
    let _stop = StopGuard(sink.clone());
    let cipher = Aes128::new(&stream.key.into());
    let mut last_seq: Option<u16> = None;
    let mut errors = 0u32;
    let (mut data_buf, mut control_buf) = (vec![0u8; 2048], vec![0u8; 2048]);

    loop {
        let packet = tokio::select! {
            r = stream.data.recv(&mut data_buf) => match r {
                Ok(n) => &data_buf[..n],
                Err(_) => continue,
            },
            r = stream.control.recv(&mut control_buf) => match r {
                Ok(n) if n >= 16 && control_buf[1] & 0x7F == RETRANSMIT => &control_buf[4..n],
                _ => continue,
            },
        };
        if packet.len() < 12 {
            continue;
        }
        let seq = u16::from_be_bytes([packet[2], packet[3]]);
        if last_seq.is_some_and(|last| seq.wrapping_sub(last) == 0 || seq.wrapping_sub(last) > 0x8000) {
            continue; // duplicate or late
        }
        last_seq = Some(seq);

        let Some(decoder) = decoder.as_mut() else { continue };
        let payload = &packet[12..];
        if payload.len() <= NO_DATA_MARKER.len() {
            continue;
        }
        let mut frame = payload.to_vec();
        decrypt_cbc(&cipher, &stream.iv, &mut frame);
        match decoder.decode(&frame, None) {
            Ok(pcm) => sink.samples(&pcm.samples),
            Err(e) => {
                errors += 1;
                if errors <= 5 {
                    tracing::warn!("cannot decode audio packet {seq}: {e}");
                }
            }
        }
    }
}

struct StopGuard(Arc<dyn AudioSink>);

impl Drop for StopGuard {
    fn drop(&mut self) {
        self.0.stop();
    }
}

/// AES-128-CBC over the whole 16-byte blocks; a trailing partial block is
/// left as is. Every packet starts again from the stream IV.
fn decrypt_cbc(cipher: &Aes128, iv: &[u8; 16], data: &mut [u8]) {
    let mut previous = *iv;
    for block in data.as_chunks_mut::<16>().0 {
        let encrypted = *block;
        cipher.decrypt_block(block.into());
        for (b, p) in block.iter_mut().zip(previous) {
            *b ^= p;
        }
        previous = encrypted;
    }
}

/// AirPlay volume is in dB from -30 (quietest) to 0; -144 means muted.
pub fn volume_to_gain(db: f32) -> f32 {
    if db <= -30.0 { 0.0 } else { 10f32.powf(db.min(0.0) / 20.0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::cipher::BlockEncrypt;

    #[test]
    fn cbc_round_trip_keeps_tail() {
        let key = [5u8; 16];
        let iv = [9u8; 16];
        let cipher = Aes128::new(&key.into());
        let plain: Vec<u8> = (0..37).collect();

        // Encrypt the two whole blocks by hand, leave the 5-byte tail clear.
        let mut data = plain.clone();
        let mut previous = iv;
        for block in data[..32].as_chunks_mut::<16>().0 {
            for (b, p) in block.iter_mut().zip(previous) {
                *b ^= p;
            }
            cipher.encrypt_block(block.into());
            previous = *block;
        }
        decrypt_cbc(&cipher, &iv, &mut data);
        assert_eq!(data, plain);
    }

    #[test]
    fn volume_mapping() {
        assert_eq!(volume_to_gain(-144.0), 0.0);
        assert_eq!(volume_to_gain(0.0), 1.0);
        assert!((volume_to_gain(-20.0) - 0.1).abs() < 1e-6);
    }

    #[test]
    fn eld_config_is_accepted() {
        assert!(AacDecoder::with_config_bytes(AudioCodec::AacEld.audio_specific_config().unwrap()).is_ok());
    }
}
