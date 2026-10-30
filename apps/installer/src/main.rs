//! The downloadable installer of Riflesso X: a small program carrying the
//! app, compressed. It unpacks the app into a temporary folder and starts it
//! in setup mode (`--setup`, see apps/desktop/src/setup), which shows the
//! installer window and copies itself into place. Other arguments, such as
//! `--silent`, are passed on.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
mod launcher;

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    launcher::main()
}

/// On Linux the app ships as a package (AppImage, .deb, .rpm) instead.
#[cfg(not(windows))]
fn main() {
    eprintln!("The Riflesso X installer is for Windows. On Linux, use the AppImage, .deb or .rpm package.");
    std::process::exit(1);
}
