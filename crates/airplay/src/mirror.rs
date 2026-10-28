//! Screen-mirroring data stream (SETUP stream type 110).
//!
//! The sender opens a TCP connection and sends packets made of a 128-byte
//! header followed by a payload. Header bytes 0..4 hold the payload size
//! (little endian), byte 4 the packet type:
//! - 0: AES-CTR encrypted access unit, NAL units with 4-byte length prefixes
//! - 1: unencrypted codec configuration: an avcC record (H.264) or an `hvc1`
//!   sample entry holding an hvcC record (H.265); the header carries the size
//! - 2, 5: keep-alive and streaming reports, ignored

use std::net::SocketAddr;
use std::sync::mpsc::Sender;

use ctr::cipher::StreamCipher;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

use crate::crypto::Aes128Ctr;
use crate::{Codec, Event, VideoFormat, VideoFrame};

const HEADER_LEN: usize = 128;
const MAX_PAYLOAD: usize = 32 * 1024 * 1024;
const START_CODE: [u8; 4] = [0, 0, 0, 1];

pub async fn run(listener: TcpListener, cipher: Aes128Ctr, events: Sender<Event>) {
    let (mut stream, addr) = match listener.accept().await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::error!("mirror stream accept failed: {e}");
            return;
        }
    };
    drop(listener);
    let _ = stream.set_nodelay(true);
    tracing::info!("mirror stream connected from {addr}");
    let _ = events.send(Event::MirrorStarted { peer: addr });
    let _stopped = StoppedGuard { peer: addr, events: events.clone() };

    let mut parser = StreamParser::new(cipher);
    let mut header = [0u8; HEADER_LEN];
    let mut out = Vec::new();
    loop {
        if let Err(e) = stream.read_exact(&mut header).await {
            tracing::info!("mirror stream closed: {e}");
            return;
        }
        let size = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        if size > MAX_PAYLOAD {
            tracing::error!("mirror packet too large ({size} bytes), dropping stream");
            return;
        }
        let mut payload = vec![0u8; size];
        if let Err(e) = stream.read_exact(&mut payload).await {
            tracing::info!("mirror stream closed mid-packet: {e}");
            return;
        }
        parser.process(&header, payload, &mut out);
        for event in out.drain(..) {
            if events.send(event).is_err() {
                return;
            }
        }
    }
}

/// Reports the end of the stream however the task ends, including abort.
struct StoppedGuard {
    peer: SocketAddr,
    events: Sender<Event>,
}

impl Drop for StoppedGuard {
    fn drop(&mut self) {
        let _ = self.events.send(Event::MirrorStopped { peer: self.peer });
    }
}

pub struct StreamParser {
    cipher: Aes128Ctr,
    codec: Codec,
    /// Parameter sets in Annex-B form, prepended to the next access unit.
    pending_config: Option<Vec<u8>>,
    format: Option<VideoFormat>,
}

impl StreamParser {
    pub fn new(cipher: Aes128Ctr) -> Self {
        Self { cipher, codec: Codec::H264, pending_config: None, format: None }
    }

    pub fn process(&mut self, header: &[u8; HEADER_LEN], mut payload: Vec<u8>, out: &mut Vec<Event>) {
        let timestamp = u64::from_le_bytes(header[8..16].try_into().unwrap());
        match header[4] {
            0 => {
                // Decrypt first: the keystream must advance even for bad packets.
                self.cipher.apply_keystream(&mut payload);
                let Some(keyframe) = length_prefixed_to_annex_b(&mut payload, self.codec) else {
                    tracing::warn!("dropping undecodable video packet ({} bytes)", payload.len());
                    return;
                };
                let annex_b = match self.pending_config.take() {
                    Some(mut config) => {
                        config.extend_from_slice(&payload);
                        config
                    }
                    None => payload,
                };
                out.push(Event::VideoFrame(VideoFrame { codec: self.codec, annex_b, keyframe, timestamp }));
            }
            1 => {
                let Some((codec, config)) = parse_avcc(&payload)
                    .map(|c| (Codec::H264, c))
                    .or_else(|| parse_hvc1(&payload).map(|c| (Codec::Hevc, c)))
                else {
                    tracing::warn!("unsupported codec configuration ({} bytes)", payload.len());
                    return;
                };
                self.codec = codec;
                let float = |at: usize| f32::from_le_bytes(header[at..at + 4].try_into().unwrap()) as u32;
                let format = VideoFormat {
                    codec,
                    width: float(56),
                    height: float(60),
                    source_width: float(40),
                    source_height: float(44),
                };
                if self.format != Some(format) {
                    tracing::info!("video format {codec:?} {}x{}", format.width, format.height);
                    self.format = Some(format);
                    out.push(Event::VideoFormat(format));
                }
                self.pending_config = Some(config);
            }
            other => tracing::trace!("ignoring mirror packet type {other}"),
        }
    }
}

/// Rewrites length-prefixed NAL units in place to Annex-B start codes.
/// Returns whether the access unit contains a random access picture, or
/// `None` if the data is not valid (typically a decryption failure).
fn length_prefixed_to_annex_b(data: &mut [u8], codec: Codec) -> Option<bool> {
    let mut pos = 0;
    let mut keyframe = false;
    while pos < data.len() {
        let len = u32::from_be_bytes(data.get(pos..pos + 4)?.try_into().unwrap()) as usize;
        let nal = pos + 4;
        if len == 0 || len > data.len() - nal {
            return None;
        }
        let nal_header = data[nal];
        if nal_header & 0x80 != 0 {
            return None; // forbidden_zero_bit
        }
        keyframe |= match codec {
            Codec::H264 => nal_header & 0x1F == 5,
            Codec::Hevc => (16..=21).contains(&((nal_header >> 1) & 0x3F)),
        };
        data[pos..nal].copy_from_slice(&START_CODE);
        pos = nal + len;
    }
    Some(keyframe)
}

/// Extracts SPS and PPS from an avcC record as Annex-B NAL units.
fn parse_avcc(data: &[u8]) -> Option<Vec<u8>> {
    if data.first() != Some(&1) {
        return None; // not an avcC record (e.g. H.265 "hvc1")
    }
    let sps_len = u16::from_be_bytes(data.get(6..8)?.try_into().unwrap()) as usize;
    let sps = data.get(8..8 + sps_len)?;
    let pps_at = 8 + sps_len + 1;
    let pps_len = u16::from_be_bytes(data.get(pps_at..pps_at + 2)?.try_into().unwrap()) as usize;
    let pps = data.get(pps_at + 2..pps_at + 2 + pps_len)?;

    let mut out = Vec::with_capacity(8 + sps_len + pps_len);
    out.extend_from_slice(&START_CODE);
    out.extend_from_slice(sps);
    out.extend_from_slice(&START_CODE);
    out.extend_from_slice(pps);
    Some(out)
}

/// Extracts VPS, SPS and PPS from the hvcC record inside an `hvc1` sample
/// entry, as Annex-B NAL units.
fn parse_hvc1(data: &[u8]) -> Option<Vec<u8>> {
    if data.get(4..8)? != b"hvc1" {
        return None;
    }
    let record = data.windows(4).position(|w| w == b"hvcC")? + 4;
    // 22 bytes of profile/level fields, then the NAL unit arrays.
    let mut pos = record + 22;
    let arrays = *data.get(pos)?;
    pos += 1;
    let mut out = Vec::new();
    for _ in 0..arrays {
        let count = u16::from_be_bytes(data.get(pos + 1..pos + 3)?.try_into().unwrap());
        pos += 3;
        for _ in 0..count {
            let len = u16::from_be_bytes(data.get(pos..pos + 2)?.try_into().unwrap()) as usize;
            out.extend_from_slice(&START_CODE);
            out.extend_from_slice(data.get(pos + 2..pos + 2 + len)?);
            pos += 2 + len;
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::mirror_cipher;

    fn header(kind: u8, size: usize) -> [u8; HEADER_LEN] {
        let mut h = [0u8; HEADER_LEN];
        h[..4].copy_from_slice(&(size as u32).to_le_bytes());
        h[4] = kind;
        h[56..60].copy_from_slice(&1920f32.to_le_bytes());
        h[60..64].copy_from_slice(&1080f32.to_le_bytes());
        h
    }

    #[test]
    fn config_then_encrypted_frame() {
        let sps = [0x67, 0x64, 0x00, 0x28];
        let pps = [0x68, 0xEE, 0x3C];
        let mut avcc = vec![1, 0x64, 0, 0x28, 0xFF, 0xE1, 0, sps.len() as u8];
        avcc.extend_from_slice(&sps);
        avcc.extend_from_slice(&[1, 0, pps.len() as u8]);
        avcc.extend_from_slice(&pps);

        let idr = [0x65, 0x88, 0x84, 0x00, 0x33];
        let mut frame = (idr.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&idr);
        let mut encrypted = frame.clone();
        mirror_cipher(&[3; 16], 7).apply_keystream(&mut encrypted);

        let mut parser = StreamParser::new(mirror_cipher(&[3; 16], 7));
        let mut out = Vec::new();
        parser.process(&header(1, avcc.len()), avcc, &mut out);
        parser.process(&header(0, encrypted.len()), encrypted, &mut out);

        assert!(matches!(out[0], Event::VideoFormat(VideoFormat { width: 1920, height: 1080, .. })));
        let Event::VideoFrame(f) = &out[1] else { panic!("expected frame") };
        assert!(f.keyframe);
        let mut expected = vec![0, 0, 0, 1];
        expected.extend_from_slice(&sps);
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(&pps);
        expected.extend_from_slice(&[0, 0, 0, 1]);
        expected.extend_from_slice(&idr);
        assert_eq!(f.annex_b, expected);
    }

    #[test]
    fn parses_hvc1_sample_entry() {
        // hvc1 sample entry: size, fourcc, 78 bytes of visual fields, then the hvcC box.
        let mut entry = vec![0, 0, 0, 0];
        entry.extend_from_slice(b"hvc1");
        entry.extend_from_slice(&[0; 78]);
        entry.extend_from_slice(&[0, 0, 0, 0]);
        entry.extend_from_slice(b"hvcC");
        entry.extend_from_slice(&[1; 22]);
        entry.push(3); // arrays: VPS, SPS, PPS
        for (kind, nal) in [(0xA0u8, &[0x40u8, 0x01][..]), (0xA1, &[0x42, 0x01, 0x07]), (0xA2, &[0x44, 0x01])] {
            entry.extend_from_slice(&[kind, 0, 1, 0, nal.len() as u8]);
            entry.extend_from_slice(nal);
        }
        let annex_b = parse_hvc1(&entry).unwrap();
        assert_eq!(annex_b, [0, 0, 0, 1, 0x40, 0x01, 0, 0, 0, 1, 0x42, 0x01, 0x07, 0, 0, 0, 1, 0x44, 0x01]);
        assert!(parse_avcc(&entry).is_none());
    }

    #[test]
    fn rejects_garbage() {
        let mut data = vec![0xFF; 16];
        assert_eq!(length_prefixed_to_annex_b(&mut data, Codec::H264), None);
    }
}
