//! FairPlay SAP v1 (type 0x03) server side, as used by AirPlay mirroring.
//!
//! The client sends two `/fp-setup` messages (16 and 164 bytes). After that,
//! the 72-byte `ekey` from the first RTSP SETUP can be decrypted into the
//! 16-byte AES session key.

mod tables;

use std::fmt;

unsafe extern "C" {
    fn playfair_decrypt(message3: *mut u8, cipher_text: *mut u8, key_out: *mut u8);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    UnsupportedVersion(u8),
    InvalidMode(u8),
    HandshakeMissing,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnsupportedVersion(v) => write!(f, "unsupported FairPlay version 0x{v:02x}"),
            Error::InvalidMode(m) => write!(f, "invalid FairPlay mode {m}"),
            Error::HandshakeMissing => write!(f, "fp-setup handshake has not completed"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Default)]
pub struct FairPlay {
    key_message: Option<[u8; 164]>,
}

impl FairPlay {
    pub fn new() -> Self {
        Self::default()
    }

    /// Handles the first 16-byte `/fp-setup` request.
    pub fn setup(&mut self, request: &[u8; 16]) -> Result<[u8; 142], Error> {
        if request[4] != 0x03 {
            return Err(Error::UnsupportedVersion(request[4]));
        }
        let mode = request[14];
        let reply = tables::REPLY_MESSAGES
            .get(mode as usize)
            .ok_or(Error::InvalidMode(mode))?;
        self.key_message = None;
        Ok(*reply)
    }

    /// Handles the second 164-byte `/fp-setup` request.
    pub fn handshake(&mut self, request: &[u8; 164]) -> Result<[u8; 32], Error> {
        if request[4] != 0x03 {
            return Err(Error::UnsupportedVersion(request[4]));
        }
        self.key_message = Some(*request);
        let mut reply = [0u8; 32];
        reply[..12].copy_from_slice(&tables::FP_HEADER);
        reply[12..].copy_from_slice(&request[144..164]);
        Ok(reply)
    }

    /// Decrypts the 72-byte `ekey` into the 16-byte AES session key.
    pub fn decrypt(&self, ekey: &[u8; 72]) -> Result<[u8; 16], Error> {
        let mut message = self.key_message.ok_or(Error::HandshakeMissing)?;
        let mut cipher = *ekey;
        let mut key = [0u8; 16];
        // SAFETY: playfair_decrypt reads 164 bytes of message3 and 72 bytes of
        // cipher_text and writes exactly 16 bytes to key_out; all buffers are
        // local copies of exactly those sizes.
        unsafe { playfair_decrypt(message.as_mut_ptr(), cipher.as_mut_ptr(), key.as_mut_ptr()) };
        Ok(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn setup_rejects_unknown_version() {
        let mut fp = FairPlay::new();
        let mut req = [0u8; 16];
        req[4] = 0x02;
        assert_eq!(fp.setup(&req), Err(Error::UnsupportedVersion(2)));
    }

    #[test]
    fn decrypt_requires_handshake() {
        let fp = FairPlay::new();
        assert_eq!(fp.decrypt(&[0u8; 72]), Err(Error::HandshakeMissing));
    }

    #[test]
    fn handshake_echoes_tail() {
        let mut fp = FairPlay::new();
        let mut req = [0u8; 164];
        req[4] = 0x03;
        for (i, b) in req[144..].iter_mut().enumerate() {
            *b = i as u8;
        }
        let reply = fp.handshake(&req).unwrap();
        assert_eq!(&reply[..4], b"FPLY");
        assert_eq!(&reply[12..], &req[144..]);
        // Decryption is deterministic and must not crash on arbitrary input.
        let a = fp.decrypt(&[7u8; 72]).unwrap();
        let b = fp.decrypt(&[7u8; 72]).unwrap();
        assert_eq!(a, b);
    }
}
