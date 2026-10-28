//! User settings, stored as JSON next to the receiver identity.

use std::fs;
use std::path::{Path, PathBuf};

use riflesso_airplay::{DisplayConfig, Security};
use serde::{Deserialize, Serialize};

const FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Name shown in the sender's AirPlay menu.
    pub name: String,
    pub resolution: Resolution,
    /// Accept H.265 when a decoder is installed (needed above 1080p).
    pub hevc: bool,
    /// Interface language, a code from `src/i18n` such as "en" or "pt-BR".
    pub language: String,
    /// What closing the main window does.
    pub close_action: CloseAction,
    /// What closing the mirroring window does.
    pub mirror_close_action: MirrorCloseAction,
    /// Who may mirror to this PC.
    pub security: SecurityMode,
    /// The password for `SecurityMode::Password`. Stored in plain text like the
    /// rest of this file; it only keeps strangers on the network out.
    pub password: String,
    /// Ask GitHub once a day whether a newer version is out.
    pub check_updates: bool,
    /// A version the user chose to skip ("Skip this version").
    pub skipped_version: String,
    /// No update notice before this time (Unix seconds, "Remind me later").
    pub remind_after: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            name: "Riflesso X".into(),
            resolution: Resolution::Auto,
            hevc: true,
            language: "en".into(),
            close_action: CloseAction::Quit,
            mirror_close_action: MirrorCloseAction::Disconnect,
            security: SecurityMode::None,
            password: String::new(),
            check_updates: true,
            skipped_version: String::new(),
            remind_after: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SecurityMode {
    /// Anyone on the network.
    None,
    Password,
    /// A code shown on this PC, the first time a device connects.
    Code,
    /// A code shown on this PC at every connection.
    CodeEveryTime,
}

/// Shortest accepted password.
pub const MIN_PASSWORD_LEN: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CloseAction {
    /// Quit Riflesso X: senders can no longer find it.
    Quit,
    /// Keep receiving in the background, from the notification area.
    Tray,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MirrorCloseAction {
    /// End the sender's mirroring session.
    Disconnect,
    /// Only hide the window; the sender keeps mirroring.
    Hide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resolution {
    /// The primary monitor's size.
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "1440p")]
    P1440,
    #[serde(rename = "2160p")]
    P2160,
}

impl Resolution {
    /// The display size, or `None` for the monitor's.
    pub fn size(self) -> Option<(u16, u16)> {
        match self {
            Resolution::Auto => None,
            Resolution::P1080 => Some((1920, 1080)),
            Resolution::P1440 => Some((2560, 1440)),
            Resolution::P2160 => Some((3840, 2160)),
        }
    }
}

impl Settings {
    /// Whether going from `self` to `other` needs the receiver restarted
    /// (what senders see changed).
    pub fn receiver_differs(&self, other: &Self) -> bool {
        self.name != other.name
            || self.resolution != other.resolution
            || self.hevc != other.hevc
            || self.security() != other.security()
    }

    /// The access control the receiver applies.
    pub fn security(&self) -> Security {
        match self.security {
            SecurityMode::None => Security::None,
            SecurityMode::Password => Security::Password(self.password.clone()),
            SecurityMode::Code => Security::Code,
            SecurityMode::CodeEveryTime => Security::CodeEveryTime,
        }
    }

    pub fn load(dir: &Path) -> Self {
        let path = dir.join(FILE_NAME);
        match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|e| {
                tracing::warn!("ignoring unreadable {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<PathBuf> {
        fs::create_dir_all(dir)?;
        let path = dir.join(FILE_NAME);
        fs::write(&path, serde_json::to_vec_pretty(self).expect("settings serialize"))?;
        Ok(path)
    }

    /// The video size to request and whether H.265 is offered. Senders pick
    /// the stream size from the height, and use H.265 above 1080 lines.
    pub fn display(&self, hevc_available: bool, monitor: Option<(u32, u32)>) -> (DisplayConfig, bool) {
        let hevc = self.hevc && hevc_available;
        (DisplayConfig::choose(self.resolution.size(), monitor, hevc), hevc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_hevc_stays_at_1080p() {
        let settings = Settings { resolution: Resolution::P2160, ..Settings::default() };
        let (display, hevc) = settings.display(false, None);
        assert_eq!((display.width, display.height, hevc), (1920, 1080, false));
    }

    #[test]
    fn auto_follows_monitor() {
        let (display, hevc) = Settings::default().display(true, Some((2560, 1440)));
        assert_eq!((display.width, display.height, hevc), (2560, 1440, true));
    }

    #[test]
    fn only_what_senders_see_restarts_the_receiver() {
        let base = Settings::default();
        let app_only = Settings { language: "it".into(), close_action: CloseAction::Tray, ..base.clone() };
        assert!(!base.receiver_differs(&app_only));
        assert!(base.receiver_differs(&Settings { name: "Studio".into(), ..base.clone() }));
    }

    #[test]
    fn missing_fields_take_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"name":"Studio"}"#).unwrap();
        assert_eq!(settings, Settings { name: "Studio".into(), ..Settings::default() });
    }
}
