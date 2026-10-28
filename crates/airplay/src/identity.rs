//! Persistent receiver identity: the AirPlay device ID and the Ed25519 key
//! used for pairing. Both must stay stable so senders recognise us.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use ed25519_dalek::SigningKey;
use rand_core::{OsRng, RngCore};

const FILE_NAME: &str = "identity.bin";

pub struct Identity {
    pub device_id: [u8; 6],
    pub signing_key: SigningKey,
}

impl Identity {
    pub fn load_or_create(dir: &Path) -> Result<Self> {
        let path = dir.join(FILE_NAME);
        if let Ok(bytes) = fs::read(&path) {
            if bytes.len() == 38 {
                let seed: [u8; 32] = bytes[..32].try_into().unwrap();
                let device_id: [u8; 6] = bytes[32..].try_into().unwrap();
                return Ok(Self { device_id, signing_key: SigningKey::from_bytes(&seed) });
            }
            tracing::warn!("ignoring malformed identity file {}", path.display());
        }

        let signing_key = SigningKey::generate(&mut OsRng);
        let device_id = match mac_address::get_mac_address() {
            Ok(Some(mac)) => mac.bytes(),
            _ => {
                let mut id = [0u8; 6];
                OsRng.fill_bytes(&mut id);
                // Locally administered, unicast.
                id[0] = (id[0] | 0x02) & 0xFE;
                id
            }
        };

        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        let mut bytes = signing_key.to_bytes().to_vec();
        bytes.extend_from_slice(&device_id);
        fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        Ok(Self { device_id, signing_key })
    }

    /// `AA:BB:CC:DD:EE:FF`, as used in TXT records and /info.
    pub fn device_id_string(&self) -> String {
        self.device_id.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
    }

    /// `AABBCCDDEEFF`, as used in the RAOP service instance name.
    pub fn device_id_compact(&self) -> String {
        hex::encode_upper(self.device_id)
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.signing_key.verifying_key().to_bytes()
    }
}
