//! Legacy AirPlay pairing (pair-setup / pair-verify) and session key derivation.

use aes::Aes128;
use ctr::cipher::{KeyIvInit, StreamCipher};
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use rand_core::OsRng;
use sha2::{Digest, Sha512};
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::identity::Identity;

pub type Aes128Ctr = ctr::Ctr128BE<Aes128>;

/// First 16 bytes of SHA-512 over the concatenated parts.
pub fn sha512_16(parts: &[&[u8]]) -> [u8; 16] {
    let mut hasher = Sha512::new();
    for part in parts {
        hasher.update(part);
    }
    hasher.finalize()[..16].try_into().unwrap()
}

/// The FairPlay-decrypted key is bound to the pairing's ECDH secret when
/// pairing took place.
pub fn bind_key_to_pairing(aes_key: &[u8; 16], ecdh_secret: &[u8; 32]) -> [u8; 16] {
    sha512_16(&[aes_key, ecdh_secret])
}

/// AES-128-CTR cipher for the screen-mirroring stream. The keystream runs
/// continuously across all packets of one stream.
pub fn mirror_cipher(aes_key: &[u8; 16], stream_connection_id: u64) -> Aes128Ctr {
    let key = sha512_16(&[format!("AirPlayStreamKey{stream_connection_id}").as_bytes(), aes_key]);
    let iv = sha512_16(&[format!("AirPlayStreamIV{stream_connection_id}").as_bytes(), aes_key]);
    Aes128Ctr::new(&key.into(), &iv.into())
}

#[derive(Debug)]
pub enum PairingError {
    Malformed(&'static str),
    OutOfOrder,
    BadSignature,
}

impl std::fmt::Display for PairingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PairingError::Malformed(what) => write!(f, "malformed pair-verify message: {what}"),
            PairingError::OutOfOrder => write!(f, "pair-verify step out of order"),
            PairingError::BadSignature => write!(f, "pair-verify signature check failed"),
        }
    }
}

struct Handshake {
    ours: [u8; 32],
    theirs: [u8; 32],
    their_signing_key: VerifyingKey,
}

#[derive(Default)]
pub struct Pairing {
    handshake: Option<Handshake>,
    ecdh_secret: Option<[u8; 32]>,
    verified: bool,
}

impl Pairing {
    /// `/pair-setup`: the client sends its Ed25519 key, we answer with ours.
    pub fn setup(&mut self, identity: &Identity, body: &[u8]) -> Result<[u8; 32], PairingError> {
        if body.len() != 32 {
            return Err(PairingError::Malformed("pair-setup body must be 32 bytes"));
        }
        Ok(identity.public_key())
    }

    /// `/pair-verify`: step 1 exchanges X25519 keys and our encrypted
    /// signature; step 0 carries the client's encrypted signature.
    pub fn verify(&mut self, identity: &Identity, body: &[u8]) -> Result<Vec<u8>, PairingError> {
        match body.first() {
            Some(1) => self.verify_start(identity, body),
            Some(0) => self.verify_finish(body).map(|()| Vec::new()),
            _ => Err(PairingError::Malformed("unknown pair-verify step")),
        }
    }

    fn verify_start(&mut self, identity: &Identity, body: &[u8]) -> Result<Vec<u8>, PairingError> {
        if body.len() != 4 + 32 + 32 {
            return Err(PairingError::Malformed("pair-verify step 1 must be 68 bytes"));
        }
        let theirs: [u8; 32] = body[4..36].try_into().unwrap();
        let their_signing_key = VerifyingKey::from_bytes(body[36..68].try_into().unwrap())
            .map_err(|_| PairingError::Malformed("invalid Ed25519 key"))?;

        let secret = EphemeralSecret::random_from_rng(OsRng);
        let ours = PublicKey::from(&secret).to_bytes();
        let shared = secret.diffie_hellman(&PublicKey::from(theirs)).to_bytes();

        let mut message = [0u8; 64];
        message[..32].copy_from_slice(&ours);
        message[32..].copy_from_slice(&theirs);
        let mut signature = identity.signing_key.sign(&message).to_bytes();
        verify_cipher(&shared).apply_keystream(&mut signature);

        self.ecdh_secret = Some(shared);
        self.handshake = Some(Handshake { ours, theirs, their_signing_key });
        self.verified = false;

        let mut reply = ours.to_vec();
        reply.extend_from_slice(&signature);
        Ok(reply)
    }

    fn verify_finish(&mut self, body: &[u8]) -> Result<(), PairingError> {
        if body.len() != 4 + 64 {
            return Err(PairingError::Malformed("pair-verify step 2 must be 68 bytes"));
        }
        let (Some(handshake), Some(shared)) = (&self.handshake, &self.ecdh_secret) else {
            return Err(PairingError::OutOfOrder);
        };

        // The client's signature follows our 64-byte one in the same keystream.
        let mut cipher = verify_cipher(shared);
        cipher.apply_keystream(&mut [0u8; 64]);
        let mut signature: [u8; 64] = body[4..].try_into().unwrap();
        cipher.apply_keystream(&mut signature);

        let mut message = [0u8; 64];
        message[..32].copy_from_slice(&handshake.theirs);
        message[32..].copy_from_slice(&handshake.ours);
        handshake
            .their_signing_key
            .verify(&message, &Signature::from_bytes(&signature))
            .map_err(|_| PairingError::BadSignature)?;
        self.verified = true;
        Ok(())
    }

    pub fn ecdh_secret(&self) -> Option<&[u8; 32]> {
        self.ecdh_secret.as_ref()
    }

    pub fn is_verified(&self) -> bool {
        self.verified
    }
}

fn verify_cipher(shared: &[u8; 32]) -> Aes128Ctr {
    let key = sha512_16(&[b"Pair-Verify-AES-Key", shared]);
    let iv = sha512_16(&[b"Pair-Verify-AES-IV", shared]);
    Aes128Ctr::new(&key.into(), &iv.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;

    fn identity() -> Identity {
        Identity { device_id: [2, 0, 0, 0, 0, 1], signing_key: SigningKey::generate(&mut OsRng) }
    }

    /// Plays the sender side of pair-verify against our implementation.
    #[test]
    fn pair_verify_round_trip() {
        let server = identity();
        let client_key = SigningKey::generate(&mut OsRng);
        let client_secret = EphemeralSecret::random_from_rng(OsRng);
        let client_pub = PublicKey::from(&client_secret).to_bytes();

        let mut step1 = vec![1, 0, 0, 0];
        step1.extend_from_slice(&client_pub);
        step1.extend_from_slice(&client_key.verifying_key().to_bytes());

        let mut pairing = Pairing::default();
        let reply = pairing.verify(&server, &step1).unwrap();
        let server_pub: [u8; 32] = reply[..32].try_into().unwrap();
        let shared = client_secret.diffie_hellman(&PublicKey::from(server_pub)).to_bytes();
        assert_eq!(pairing.ecdh_secret(), Some(&shared));

        // Client checks our signature...
        let mut cipher = verify_cipher(&shared);
        let mut server_sig: [u8; 64] = reply[32..].try_into().unwrap();
        cipher.apply_keystream(&mut server_sig);
        let mut msg = server_pub.to_vec();
        msg.extend_from_slice(&client_pub);
        server.signing_key.verifying_key().verify(&msg, &Signature::from_bytes(&server_sig)).unwrap();

        // ...then sends its own, continuing the same keystream.
        let mut msg = client_pub.to_vec();
        msg.extend_from_slice(&server_pub);
        let mut client_sig = client_key.sign(&msg).to_bytes();
        cipher.apply_keystream(&mut client_sig);
        let mut step2 = vec![0, 0, 0, 0];
        step2.extend_from_slice(&client_sig);

        pairing.verify(&server, &step2).unwrap();
        assert!(pairing.is_verified());

        step2[10] ^= 1;
        assert!(matches!(pairing.verify(&server, &step2), Err(PairingError::BadSignature)));
    }

    #[test]
    fn mirror_keystream_is_continuous() {
        let key = [9u8; 16];
        let mut whole = vec![0u8; 100];
        mirror_cipher(&key, 42).apply_keystream(&mut whole);

        let mut split = vec![0u8; 100];
        let mut cipher = mirror_cipher(&key, 42);
        let (a, b) = split.split_at_mut(37);
        cipher.apply_keystream(a);
        cipher.apply_keystream(b);
        assert_eq!(whole, split);
    }
}
