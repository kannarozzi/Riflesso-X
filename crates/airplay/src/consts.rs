//! Values advertised to AirPlay senders.
//!
//! We present ourselves as an Apple TV 3 running AirPlay 220.68, the
//! combination used by the open-source receivers that work with current iOS.

pub const MODEL: &str = "AppleTV3,2";
pub const SOURCE_VERSION: &str = "220.68";
pub const SERVER_HEADER: &str = "AirTunes/220.68";

/// Feature bits: video, photo, screen mirroring, audio, FairPlay (12, 14),
/// RAOP (30). Legacy pairing (bit 27) stays off: with it, iPhones wait 4–6 s
/// on "Connecting…" before even opening the connection (measured with iOS 26,
/// AirPlay 980.77). UxPlay leaves it off too.
const BASE_FEATURES: u64 = 0x527F_FEE6;
/// "ScreenMultiCodec": the sender may mirror in H.265, which it does when the
/// requested display is taller than 1080 pixels.
const FEATURE_SCREEN_MULTI_CODEC: u64 = 1 << 42;

pub fn features(hevc: bool) -> u64 {
    if let Some(features) = features_override() {
        return features;
    }
    if hevc { BASE_FEATURES | FEATURE_SCREEN_MULTI_CODEC } else { BASE_FEATURES }
}

/// Debugging aid: `RIFLESSO_FEATURES=0x527FFFF7,0x1E` advertises exactly these
/// bits (low word, high word), to compare how senders react to other receivers.
fn features_override() -> Option<u64> {
    let value = std::env::var("RIFLESSO_FEATURES").ok()?;
    let parse = |word: &str| u64::from_str_radix(word.trim().trim_start_matches("0x").trim_start_matches("0X"), 16).ok();
    let (low, high) = value.split_once(',').unwrap_or((&value, "0"));
    match (parse(low), parse(high)) {
        (Some(low), Some(high)) if low <= 0xFFFF_FFFF && high <= 0xFFFF_FFFF => {
            tracing::warn!("advertising features {} from RIFLESSO_FEATURES", features_txt(high << 32 | low));
            Some(high << 32 | low)
        }
        _ => {
            tracing::warn!("ignoring malformed RIFLESSO_FEATURES={value:?}");
            None
        }
    }
}

pub const PAIRING_IDENTITY: &str = "b3a7c2f0-5d1e-4c8a-9f6b-2e7d4a1c8b90";
pub const DISPLAY_UUID: &str = "6f0d7c9e-2a41-4b8e-9d3c-5e1a7b2f4c60";

pub fn features_txt(features: u64) -> String {
    format!("0x{:X},0x{:X}", features & 0xFFFF_FFFF, features >> 32)
}
