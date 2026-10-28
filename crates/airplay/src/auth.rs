//! Access control: who may mirror to this receiver. Every mode uses HTTP
//! Digest (RFC 2617, MD5) on the first SETUP; the iPhone shows a password
//! prompt and remembers what was typed.
//!
//! - Password: a fixed password.
//! - Code: a 4-digit code shown on screen is the password. The first time
//!   only: devices that typed it are remembered in [`Registry`] and their
//!   iPhone answers with the remembered code by itself.
//! - Code every time: a fresh code at every connection.
//!
//! Apple's own on-screen code ("pair-setup-pin", SRP-6a) needs the legacy
//! pairing feature, with which iPhones wait ~5 s before connecting; see
//! docs/PROTOCOL.md.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use md5::{Digest, Md5};
use rand_core::{OsRng, RngCore};
use subtle::ConstantTimeEq;

pub const DIGEST_REALM: &str = "raop";
/// A code shown on screen stays on screen at most this long.
pub const CODE_LIFETIME: Duration = Duration::from_secs(120);
/// Times a code is shown before a new one is drawn.
const MAX_CODE_SHOWS: u8 = 3;
const PAIRED_DEVICES_FILE: &str = "paired_devices.txt";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Security {
    #[default]
    None,
    Password(String),
    /// On-screen code the first time; afterwards the device is remembered.
    Code,
    /// On-screen code at every connection.
    CodeEveryTime,
}

impl Security {
    /// Value of the `sf` TXT flags: bit 7 = a password is required.
    pub fn status_flags(&self) -> &'static str {
        if self.required() { "0x84" } else { "0x4" }
    }

    pub fn required(&self) -> bool {
        *self != Security::None
    }
}

/// A random code from 0001 to 9999.
pub fn random_code() -> String {
    loop {
        let mut bytes = [0u8; 2];
        OsRng.fill_bytes(&mut bytes);
        // Rejection sampling keeps the digits uniform.
        let n = u16::from_le_bytes(bytes);
        if n < 60_000 && n % 10_000 != 0 {
            return format!("{:04}", n % 10_000);
        }
    }
}

fn random_nonce() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Receiver-wide authentication state, shared by all connections (a sender
/// retries on a new connection after prompting).
#[derive(Default)]
pub struct AuthState {
    /// Nonce of the last Digest challenge; each one serves a single attempt.
    nonce: Option<String>,
    /// Code on screen: for which device, how often it was shown.
    shown: Option<ShownCode>,
}

struct ShownCode {
    code: String,
    device: String,
    shows: u8,
}

impl AuthState {
    /// A new Digest challenge (`WWW-Authenticate` value).
    pub fn challenge(&mut self) -> String {
        let nonce = random_nonce();
        let header = format!("Digest realm=\"{DIGEST_REALM}\", nonce=\"{nonce}\"");
        self.nonce = Some(nonce);
        header
    }

    /// Checks an `Authorization` header against `password`. The nonce is used
    /// up either way.
    pub fn check(&mut self, authorization: &str, method: &str, password: &str) -> bool {
        let Some(nonce) = self.nonce.take() else { return false };
        DigestAnswer::parse(authorization).is_some_and(|answer| answer.nonce == nonce && answer.verify(method, password))
    }

    /// The code for `device`. When `showing` (the sender is about to prompt),
    /// counts a display; a new code is drawn after a few, or for another device.
    pub fn code_for(&mut self, device: &str, showing: bool) -> String {
        let stale = match &self.shown {
            Some(s) => s.device != device || (showing && s.shows >= MAX_CODE_SHOWS),
            None => true,
        };
        if stale {
            self.shown = Some(ShownCode { code: random_code(), device: device.to_owned(), shows: 0 });
        }
        let shown = self.shown.as_mut().unwrap();
        if showing {
            shown.shows += 1;
        }
        shown.code.clone()
    }

    pub fn clear_code(&mut self) {
        self.shown = None;
    }
}

/// The fields of a Digest `Authorization` header we need.
#[derive(Debug, PartialEq, Eq)]
struct DigestAnswer {
    username: String,
    realm: String,
    nonce: String,
    uri: String,
    response: String,
    qop: Option<(String, String, String)>,
}

impl DigestAnswer {
    fn parse(header: &str) -> Option<Self> {
        let params = header.trim().strip_prefix("Digest")?;
        let field = |name: &str| param(params, name);
        let qop = match (field("qop"), field("nc"), field("cnonce")) {
            (Some(qop), Some(nc), Some(cnonce)) => Some((qop, nc, cnonce)),
            _ => None,
        };
        Some(Self {
            username: field("username")?,
            realm: field("realm")?,
            nonce: field("nonce")?,
            uri: field("uri")?,
            response: field("response")?.to_ascii_lowercase(),
            qop,
        })
    }

    fn verify(&self, method: &str, password: &str) -> bool {
        let ha1 = md5_hex(&[&self.username, &self.realm, password]);
        let ha2 = md5_hex(&[method, &self.uri]);
        let expected = match &self.qop {
            Some((qop, nc, cnonce)) => md5_hex(&[&ha1, &self.nonce, nc, cnonce, qop, &ha2]),
            None => md5_hex(&[&ha1, &self.nonce, &ha2]),
        };
        expected.as_bytes().ct_eq(self.response.as_bytes()).into()
    }
}

/// Value of `name=value` or `name="value"` in a comma-separated list.
fn param(params: &str, name: &str) -> Option<String> {
    params.split(',').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key.trim().eq_ignore_ascii_case(name)).then(|| value.trim().trim_matches('"').to_owned())
    })
}

/// Lowercase hex MD5 of the parts joined with ':'.
fn md5_hex(parts: &[&str]) -> String {
    hex::encode(Md5::digest(parts.join(":").as_bytes()))
}

/// Devices that typed the on-screen code, and the code each one remembers:
/// `device ID<TAB>code` per line, in the data folder.
pub struct Registry {
    path: PathBuf,
    codes: HashMap<String, String>,
}

impl Registry {
    pub fn load(data_dir: &Path) -> Self {
        let path = data_dir.join(PAIRED_DEVICES_FILE);
        let codes = fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(device, code)| (device.to_owned(), code.to_owned()))
            .collect();
        Self { path, codes }
    }

    pub fn code(&self, device: &str) -> Option<&str> {
        self.codes.get(device).map(String::as_str)
    }

    pub fn add(&mut self, device: &str, code: &str) -> io::Result<()> {
        self.codes.insert(device.replace(['\t', '\n'], " "), code.to_owned());
        self.save()
    }

    pub fn remove(&mut self, device: &str) -> io::Result<()> {
        if self.codes.remove(device).is_some() { self.save() } else { Ok(()) }
    }

    fn save(&self) -> io::Result<()> {
        let text: String = self.codes.iter().map(|(device, code)| format!("{device}\t{code}\n")).collect();
        // Write a temporary file and rename it, so the list is never half-written.
        let temp = self.path.with_extension("tmp");
        fs::write(&temp, text)?;
        fs::rename(temp, &self.path)
    }
}

/// Forgets every device that typed the on-screen code; they will see a new one.
pub fn forget_paired_devices(data_dir: &Path) -> io::Result<()> {
    match fs::remove_file(data_dir.join(PAIRED_DEVICES_FILE)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_have_four_digits_and_are_never_zero() {
        for _ in 0..1000 {
            let code = random_code();
            assert_eq!(code.len(), 4);
            assert_ne!(code, "0000");
        }
    }

    /// RFC 2617 section 3.5 example, with qop.
    #[test]
    fn digest_with_qop_matches_rfc() {
        let header = r#"Digest username="Mufasa", realm="testrealm@host.com", nonce="dcd98b7102dd2f0e8b11d0f600bfb0c093", uri="/dir/index.html", qop=auth, nc=00000001, cnonce="0a4f113b", response="6629fae49393a05397450978507c4ef1", opaque="5ccc069c403ebaf9f0171e9517f40e41""#;
        let answer = DigestAnswer::parse(header).unwrap();
        assert!(answer.verify("GET", "Circle Of Life"));
        assert!(!answer.verify("GET", "circle of life"));
    }

    #[test]
    fn nonce_is_single_use() {
        let mut auth = AuthState::default();
        let challenge = auth.challenge();
        let nonce = param(challenge.strip_prefix("Digest").unwrap(), "nonce").unwrap();
        let uri = "rtsp://192.168.1.2/123";
        let ha1 = md5_hex(&["AirPlay", DIGEST_REALM, "1234"]);
        let response = md5_hex(&[&ha1, &nonce, &md5_hex(&["SETUP", uri])]);
        let header = format!(
            r#"Digest username="AirPlay", realm="{DIGEST_REALM}", nonce="{nonce}", uri="{uri}", response="{response}""#
        );
        assert!(auth.check(&header, "SETUP", "1234"));
        assert!(!auth.check(&header, "SETUP", "1234"));
    }

    #[test]
    fn code_changes_after_three_prompts_or_another_device() {
        let mut auth = AuthState::default();
        let shows = |auth: &AuthState| auth.shown.as_ref().map(|s| (s.device.clone(), s.shows));
        let first = auth.code_for("A", true);
        assert_eq!(auth.code_for("A", false), first);
        auth.code_for("A", true);
        auth.code_for("A", true);
        assert_eq!(shows(&auth), Some(("A".into(), 3)));
        // Codes are random and may repeat by chance: check the counter restarts.
        auth.code_for("A", true);
        assert_eq!(shows(&auth), Some(("A".into(), 1)));
        auth.code_for("B", false);
        assert_eq!(shows(&auth), Some(("B".into(), 0)));
    }

    #[test]
    fn registry_round_trip() {
        let dir = std::env::temp_dir().join(format!("riflesso-registry-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let mut registry = Registry::load(&dir);
        registry.add("AA:BB", "0427").unwrap();
        assert_eq!(Registry::load(&dir).code("AA:BB"), Some("0427"));
        registry.remove("AA:BB").unwrap();
        assert_eq!(Registry::load(&dir).code("AA:BB"), None);
        registry.add("AA:BB", "0427").unwrap();
        forget_paired_devices(&dir).unwrap();
        assert_eq!(Registry::load(&dir).code("AA:BB"), None);
        fs::remove_dir_all(dir).unwrap();
    }
}
