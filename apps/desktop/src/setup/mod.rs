//! Setup mode: the app installs, updates and uninstalls itself, with a window
//! in its own style (`setup.html`).
//!
//!   Riflesso X.exe --setup [--silent] [--dir FOLDER]
//!                                             install or update (started by the
//!                                             installer in `apps/installer`)
//!   Riflesso X.exe --uninstall [--silent]     what "Installed apps" runs
//!
//! A running program cannot delete itself, so uninstalling restarts from a
//! temporary copy, which is removed once it exits.

mod system;

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use tauri::{AppHandle, Context, Emitter, Manager, RunEvent, WebviewWindowBuilder, Wry};
use tauri_plugin_opener::OpenerExt;

use system::Step;

const LICENSE_URL: &str = "https://github.com/kannarozzi/Riflesso-X/blob/main/LICENSE.md";
/// Do not show a console window for helper commands.
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Args {
    uninstall: bool,
    silent: bool,
    /// `--dir FOLDER`: where to install (a "Riflesso X" folder inside it).
    dir: Option<PathBuf>,
    /// This is the temporary copy started by the installed app to uninstall.
    relaunched: bool,
}

/// Runs setup mode if the command line asks for it, returning the exit code;
/// `None` means a normal start.
pub fn run_if_requested(context: &mut Option<Context<Wry>>) -> Option<i32> {
    let words: Vec<String> = std::env::args().skip(1).map(|a| a.to_ascii_lowercase()).collect();
    let has = |flags: &[&str]| words.iter().any(|w| flags.contains(&w.as_str()));
    if !has(&["--setup", "--uninstall"]) {
        return None;
    }
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let dir = raw.iter().position(|a| a.eq_ignore_ascii_case("--dir")).and_then(|i| raw.get(i + 1)).map(PathBuf::from);
    let args = Args {
        uninstall: has(&["--uninstall"]),
        silent: has(&["--silent", "/s", "/silent"]),
        dir,
        relaunched: has(&["--relaunched"]),
    };

    if args.uninstall && !args.relaunched && running_from_install_dir() {
        return Some(match relaunch_from_temp(&args) {
            Ok(()) => 0,
            Err(e) => fail(&args, &format!("{e:#}")),
        });
    }

    if args.silent {
        let result = if args.uninstall {
            system::uninstall(|_| {})
        } else {
            let dir = args.dir.as_deref().map_or_else(system::default_dir, system::folder_for);
            system::install(&dir, false, |_| {})
        };
        if args.relaunched {
            clean_up_after_exit(None, true);
        }
        return Some(if result.is_ok() { 0 } else { 1 });
    }

    if !system::webview2_installed() {
        webview2_missing();
        return Some(1);
    }
    run_window(args, context.take().expect("context used once"));
    Some(0)
}

fn fail(args: &Args, message: &str) -> i32 {
    if !args.silent {
        message_box(message);
    }
    1
}

/// Whether this is the installed copy (which the installer can update), not
/// a portable one.
pub fn is_installed_copy() -> bool {
    system::installed_version().is_some() && running_from_install_dir()
}

fn running_from_install_dir() -> bool {
    let (Ok(me), Ok(dir)) = (std::env::current_exe(), system::install_dir().canonicalize()) else { return false };
    me.canonicalize().is_ok_and(|me| me.starts_with(dir))
}

fn relaunch_from_temp(args: &Args) -> anyhow::Result<()> {
    let me = std::env::current_exe()?;
    let copy = std::env::temp_dir().join(format!("riflesso-uninstall-{}.exe", std::process::id()));
    std::fs::copy(&me, &copy)?;
    let mut command = Command::new(&copy);
    command.args(["--uninstall", "--relaunched"]);
    if args.silent {
        command.arg("--silent");
    }
    command.spawn()?;
    Ok(())
}

/// Once this process has ended (a hidden `cmd` waits two seconds), removes
/// `folder` and, for the temporary uninstaller, this executable too.
fn clean_up_after_exit(folder: Option<&Path>, delete_self: bool) {
    use std::os::windows::process::CommandExt;

    let mut script = String::from("ping 127.0.0.1 -n 3 >NUL");
    if let Some(folder) = folder {
        script.push_str(&format!(" & rmdir /s /q \"{}\"", folder.display()));
    }
    if delete_self && let Ok(me) = std::env::current_exe() {
        script.push_str(&format!(" & del /f /q \"{}\"", me.display()));
    }
    // Passed verbatim: Rust's quoting (\") is not something cmd understands.
    let _ = Command::new("cmd").raw_arg(format!("/C {script}")).creation_flags(CREATE_NO_WINDOW).spawn();
}

/// Where the setup window's web view keeps its files: a temporary folder,
/// removed at exit, so nothing of the setup stays on the PC.
fn webview_data_dir() -> PathBuf {
    std::env::temp_dir().join(format!("riflesso-setup-{}", std::process::id()))
}

fn run_window(args: Args, context: Context<Wry>) {
    let relaunched = args.relaunched;
    let webview_data = webview_data_dir();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(Mode { uninstall: args.uninstall })
        .invoke_handler(tauri::generate_handler![
            setup_info,
            choose_folder,
            install,
            uninstall_app,
            launch_app,
            open_license
        ])
        .setup({
            let webview_data = webview_data.clone();
            move |app| {
                let config = app.config().app.windows.iter().find(|w| w.label == "setup").expect("setup window").clone();
                WebviewWindowBuilder::from_config(app.handle(), &config)?.data_directory(webview_data).build()?;
                Ok(())
            }
        })
        .build(context)
        .expect("starting the setup window");

    app.run(move |_app, event| {
        if let RunEvent::Exit = event {
            // The web view's temporary files always; the program itself only
            // when it is the temporary uninstaller.
            clean_up_after_exit(Some(&webview_data), relaunched);
        }
    });
}

struct Mode {
    uninstall: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetupInfo {
    /// "install", "update", "reinstall" or "uninstall".
    mode: &'static str,
    version: &'static str,
    installed_version: Option<String>,
    install_dir: String,
    /// Only a new installation can choose its folder; updates stay put.
    can_choose_dir: bool,
    app_running: bool,
}

#[tauri::command]
fn setup_info(mode: tauri::State<'_, Mode>) -> SetupInfo {
    let installed = system::installed_version();
    let kind = if mode.uninstall {
        "uninstall"
    } else {
        match installed.as_deref() {
            None => "install",
            Some(v) if v == system::VERSION => "reinstall",
            Some(_) => "update",
        }
    };
    SetupInfo {
        mode: kind,
        version: system::VERSION,
        installed_version: installed,
        install_dir: system::install_dir().display().to_string(),
        can_choose_dir: kind == "install",
        app_running: system::app_running(),
    }
}

/// Progress events for the interface: a step name and how far along it is.
fn report(app: &AppHandle) -> impl Fn(Step) + use<> {
    let app = app.clone();
    move |step| {
        let (name, fraction) = match step {
            Step::ClosingApp => ("closing", 0.1),
            Step::CopyingFiles => ("copying", 0.3),
            Step::RemovingFiles => ("removing", 0.5),
            Step::CreatingShortcuts => ("shortcuts", 0.75),
            Step::Registering => ("registering", 0.9),
        };
        let _ = app.emit("progress", (name, fraction));
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChosenFolder {
    dir: String,
    writable: bool,
}

/// Asks for a folder with the system dialog; `None` if the user cancelled.
#[tauri::command]
async fn choose_folder(app: AppHandle, current: String) -> Option<ChosenFolder> {
    use tauri_plugin_dialog::DialogExt;

    let start = Path::new(&current).parent().map(Path::to_path_buf).unwrap_or_default();
    let window = app.get_webview_window("setup")?;
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_parent(&window).set_directory(start).blocking_pick_folder()
    })
    .await
    .ok()??;
    let dir = system::folder_for(&picked.into_path().ok()?);
    Some(ChosenFolder { writable: system::can_write(&dir), dir: dir.display().to_string() })
}

#[tauri::command]
async fn install(app: AppHandle, dir: String, desktop_shortcut: bool) -> Result<(), String> {
    let progress = report(&app);
    let dir = PathBuf::from(dir);
    tauri::async_runtime::spawn_blocking(move || system::install(&dir, desktop_shortcut, progress))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn uninstall_app(app: AppHandle) -> Result<(), String> {
    let progress = report(&app);
    tauri::async_runtime::spawn_blocking(move || system::uninstall(progress))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn launch_app(app: AppHandle) -> Result<(), String> {
    system::launch_app().map_err(|e| format!("{e:#}"))?;
    if let Some(window) = app.get_webview_window("setup") {
        let _ = window.close();
    }
    Ok(())
}

#[tauri::command]
fn open_license(app: AppHandle) -> Result<(), String> {
    app.opener().open_url(LICENSE_URL, None::<&str>).map_err(|e| e.to_string())
}

fn webview2_missing() {
    use windows::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONINFORMATION, MB_YESNO, MessageBoxW};
    use windows::core::{HSTRING, w};

    let text = HSTRING::from(
        "Riflesso X needs the Microsoft Edge WebView2 Runtime, which is missing on this PC.\n\n\
         Open the download page? Install the \"Evergreen Bootstrapper\", then run this setup again.",
    );
    // SAFETY: a modal message box with valid strings.
    let answer = unsafe { MessageBoxW(None, &text, w!("Riflesso X Setup"), MB_YESNO | MB_ICONINFORMATION) };
    if answer == IDYES {
        let _ = Command::new("explorer").arg("https://developer.microsoft.com/microsoft-edge/webview2/").spawn();
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
