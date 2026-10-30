//! Windows: unpacks the app and starts it in setup mode.

use std::path::Path;
use std::process::{Command, ExitCode};
use std::time::Duration;

static APP: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/app.zst"));

pub fn main() -> ExitCode {
    let temp = std::env::temp_dir().join(format!("riflesso-setup-launcher-{}", std::process::id()));
    let result = run(&temp);
    remove_folder(&temp);
    match result {
        Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        Err(message) => {
            message_box(&message);
            ExitCode::FAILURE
        }
    }
}

fn run(temp: &Path) -> Result<i32, String> {
    if APP.is_empty() {
        return Err("This installer was built without Riflesso X.".into());
    }
    let app = zstd::decode_all(APP).map_err(|e| format!("Cannot unpack Riflesso X: {e}"))?;
    std::fs::create_dir_all(temp).map_err(|e| format!("Cannot create {}: {e}", temp.display()))?;
    let exe = temp.join("Riflesso X.exe");
    std::fs::write(&exe, app).map_err(|e| format!("Cannot write {}: {e}", exe.display()))?;
    let status = Command::new(&exe)
        .arg("--setup")
        .args(std::env::args_os().skip(1))
        .status()
        .map_err(|e| format!("Cannot start the installer: {e}"))?;
    Ok(status.code().unwrap_or(1))
}

/// The unpacked app stays locked for a moment after it exits.
fn remove_folder(folder: &Path) {
    for _ in 0..20 {
        if std::fs::remove_dir_all(folder).is_ok() || !folder.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

fn message_box(message: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
    use windows::core::{HSTRING, w};

    // SAFETY: a modal message box with valid strings.
    unsafe {
        MessageBoxW(None, &HSTRING::from(message), w!("Riflesso X Setup"), MB_OK | MB_ICONERROR);
    }
}
