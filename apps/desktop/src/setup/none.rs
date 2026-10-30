//! Linux: the app is installed and updated by its package (AppImage, .deb,
//! .rpm), so there is no setup mode.

use tauri::{Context, Wry};

pub fn run_if_requested(_context: &mut Option<Context<Wry>>) -> Option<i32> {
    None
}

/// Updates open the release page, as for the portable copy on Windows.
pub fn is_installed_copy() -> bool {
    false
}
