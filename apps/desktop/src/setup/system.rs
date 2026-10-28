//! What installing and uninstalling do on the system. Everything goes in one
//! folder, `%LOCALAPPDATA%\Riflesso X`, for the current user only (no
//! administrator rights); outside it there are only the Start menu (and
//! optional desktop) shortcut and the entry in "Installed apps".

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

pub const APP_NAME: &str = "Riflesso X";
pub const APP_EXE: &str = "Riflesso X.exe";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
const PUBLISHER: &str = "kannarozzi";
const HOMEPAGE: &str = "https://github.com/kannarozzi/Riflesso-X";
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\RiflessoX";
const LICENSE: &str = include_str!("../../../../LICENSE.md");

/// Steps reported while installing or uninstalling, for the progress bar.
#[derive(Debug, Clone, Copy)]
pub enum Step {
    ClosingApp,
    CopyingFiles,
    CreatingShortcuts,
    Registering,
    RemovingFiles,
}

/// Where Riflesso X is installed: the folder recorded in "Installed apps",
/// or the default one for a new installation.
pub fn install_dir() -> PathBuf {
    registered_dir().unwrap_or_else(default_dir)
}

pub fn default_dir() -> PathBuf {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    local.join(APP_NAME)
}

fn registered_dir() -> Option<PathBuf> {
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL_KEY).ok()?;
    let dir: String = key.get_value("InstallLocation").ok()?;
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

/// The folder to install into when the user picks `chosen`: a "Riflesso X"
/// folder inside it, unless it already is one, so that uninstalling only
/// ever concerns the app's own folder.
pub fn folder_for(chosen: &Path) -> PathBuf {
    let named = chosen.file_name().is_some_and(|name| name.eq_ignore_ascii_case(APP_NAME));
    if named { chosen.to_path_buf() } else { chosen.join(APP_NAME) }
}

/// Whether the current user can create and write `dir` (without
/// administrator rights). Leaves nothing behind.
pub fn can_write(dir: &Path) -> bool {
    let Some(existing) = dir.ancestors().find(|a| a.is_dir()) else { return false };
    let probe = existing.join(format!(".riflesso-write-test-{}", std::process::id()));
    let ok = fs::write(&probe, b"").is_ok();
    let _ = fs::remove_file(&probe);
    ok
}

/// The version recorded in "Installed apps", if Riflesso X is installed.
pub fn installed_version() -> Option<String> {
    RegKey::predef(HKEY_CURRENT_USER).open_subkey(UNINSTALL_KEY).ok()?.get_value("DisplayVersion").ok()
}

/// Copies this very executable into the installation folder: the installer
/// runs the app in setup mode (see `apps/installer`), and the installed app is
/// also its own uninstaller.
pub fn install(dir: &Path, desktop_shortcut: bool, progress: impl Fn(Step)) -> Result<()> {
    if !can_write(dir) {
        bail!("{} cannot be written without administrator rights", dir.display());
    }
    // An update stays where the app already is.
    let previous = registered_dir();
    let dir = previous.as_deref().unwrap_or(dir);

    progress(Step::ClosingApp);
    close_running(dir)?;

    progress(Step::CopyingFiles);
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let me = std::env::current_exe().context("locating the app")?;
    let target = dir.join(APP_EXE);
    if !same_file(&me, &target) {
        let app = fs::read(&me).context("reading the app")?;
        write_replacing(&target, &app)?;
    }
    fs::write(dir.join("LICENSE.md"), LICENSE).context("writing the license")?;

    progress(Step::CreatingShortcuts);
    shortcuts::create(&shortcuts::start_menu()?, &target)?;
    let desktop = shortcuts::desktop()?;
    // On an update, keep the desktop shortcut the user already has.
    if desktop_shortcut || desktop.exists() {
        shortcuts::create(&desktop, &target)?;
    }

    progress(Step::Registering);
    register(dir, folder_size_kb(dir))?;
    Ok(())
}

pub fn uninstall(progress: impl Fn(Step)) -> Result<()> {
    let dir = install_dir();

    progress(Step::ClosingApp);
    close_running(&dir)?;

    progress(Step::RemovingFiles);
    for shortcut in [shortcuts::start_menu()?, shortcuts::desktop()?] {
        let _ = fs::remove_file(shortcut);
    }
    let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(UNINSTALL_KEY);
    // Only what the app put there: the program, the license and the "data"
    // folder (settings, identity, logs). Anything else the user added stays,
    // and so does the folder then.
    for file in [APP_EXE, "LICENSE.md"] {
        let path = dir.join(file);
        if path.exists() {
            fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        }
    }
    let data = dir.join("data");
    if data.exists() {
        fs::remove_dir_all(&data).with_context(|| format!("removing {}", data.display()))?;
    }
    let _ = fs::remove_dir(&dir);
    Ok(())
}

/// Starts the installed app, detached from the setup.
pub fn launch_app() -> Result<()> {
    let dir = install_dir();
    std::process::Command::new(dir.join(APP_EXE)).current_dir(&dir).spawn().context("starting Riflesso X")?;
    Ok(())
}

/// Whether a program from `dir` (the installed app) is running.
pub fn app_running() -> bool {
    !processes::in_folder(&install_dir()).is_empty()
}

/// Writes through a temporary file, so a failure never leaves half a file.
fn write_replacing(path: &Path, data: &[u8]) -> Result<()> {
    let temp = path.with_extension("new");
    fs::write(&temp, data).with_context(|| format!("writing {}", path.display()))?;
    fs::rename(&temp, path).with_context(|| format!("replacing {}", path.display()))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn folder_size_kb(dir: &Path) -> u32 {
    let bytes: u64 = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .map(|meta| meta.len())
        .sum();
    (bytes / 1024) as u32
}

/// The entry in Settings → Apps → Installed apps.
fn register(dir: &Path, size_kb: u32) -> Result<()> {
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(UNINSTALL_KEY).context("registering the app")?;
    let uninstaller = dir.join(APP_EXE).display().to_string();
    let values: [(&str, String); 8] = [
        ("DisplayName", APP_NAME.into()),
        ("DisplayVersion", VERSION.into()),
        ("Publisher", PUBLISHER.into()),
        ("DisplayIcon", format!("{},0", dir.join(APP_EXE).display())),
        ("InstallLocation", dir.display().to_string()),
        ("UninstallString", format!("\"{uninstaller}\" --uninstall")),
        ("QuietUninstallString", format!("\"{uninstaller}\" --uninstall --silent")),
        ("URLInfoAbout", HOMEPAGE.into()),
    ];
    for (name, value) in values {
        key.set_value(name, &value).context("registering the app")?;
    }
    for (name, value) in [("NoModify", 1u32), ("NoRepair", 1), ("EstimatedSize", size_kb)] {
        key.set_value(name, &value).context("registering the app")?;
    }
    Ok(())
}

/// Closes the installed app if it is running, so its files can be replaced.
fn close_running(dir: &Path) -> Result<()> {
    let running = processes::in_folder(dir);
    if running.is_empty() {
        return Ok(());
    }
    for pid in &running {
        processes::terminate(*pid);
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    while !processes::in_folder(dir).is_empty() {
        if Instant::now() > deadline {
            bail!("Riflesso X is still running: close it and try again");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Microsoft Edge WebView2, which the app and this setup display their
/// interface with. Part of Windows 11; most Windows 10 PCs have it too.
pub fn webview2_installed() -> bool {
    const CLIENT: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
    let version = |root, path: &str| -> Option<String> { RegKey::predef(root).open_subkey(path).ok()?.get_value("pv").ok() };
    [
        (winreg::enums::HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\WOW6432Node\{CLIENT}")),
        (winreg::enums::HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\{CLIENT}")),
        (HKEY_CURRENT_USER, format!(r"Software\{CLIENT}")),
    ]
    .into_iter()
    .any(|(root, path)| version(root, &path).is_some_and(|v| !v.is_empty() && v != "0.0.0.0"))
}

mod shortcuts {
    use std::path::{Path, PathBuf};

    use anyhow::{Context, Result};
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, IPersistFile,
    };
    use windows::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Programs, IShellLinkW, KF_FLAG_DEFAULT, SHGetKnownFolderPath, ShellLink};
    use windows::core::{GUID, HSTRING, Interface};

    use super::APP_NAME;

    pub fn start_menu() -> Result<PathBuf> {
        Ok(known_folder(&FOLDERID_Programs)?.join(format!("{APP_NAME}.lnk")))
    }

    pub fn desktop() -> Result<PathBuf> {
        Ok(known_folder(&FOLDERID_Desktop)?.join(format!("{APP_NAME}.lnk")))
    }

    /// A shortcut at `link` that starts `target`, with its icon.
    pub fn create(link: &Path, target: &Path) -> Result<()> {
        let folder = target.parent().context("shortcut target has no folder")?;
        // SAFETY: plain COM calls on this thread; strings outlive the calls.
        unsafe {
            // Already initialised on this thread is fine too.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let shell_link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            shell_link.SetPath(&HSTRING::from(target.as_os_str()))?;
            shell_link.SetWorkingDirectory(&HSTRING::from(folder.as_os_str()))?;
            shell_link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;
            shell_link.SetDescription(&HSTRING::from("Mirror your iPhone, iPad or Mac over AirPlay"))?;
            shell_link.cast::<IPersistFile>()?.Save(&HSTRING::from(link.as_os_str()), true)?;
        }
        Ok(())
    }

    fn known_folder(id: &GUID) -> Result<PathBuf> {
        // SAFETY: the returned string is copied, then freed with CoTaskMemFree.
        unsafe {
            let path = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).context("locating a system folder")?;
            let result = path.to_string().map(PathBuf::from).context("reading a system folder path");
            CoTaskMemFree(Some(path.0 as *const _));
            result
        }
    }
}

mod processes {
    use std::path::Path;

    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, QueryFullProcessImageNameW,
        TerminateProcess,
    };
    use windows::core::PWSTR;

    /// Processes whose executable is in `dir` (other than this setup).
    pub fn in_folder(dir: &Path) -> Vec<u32> {
        let prefix = format!("{}\\", dir.display()).to_lowercase();
        let own = std::process::id();
        let mut found = Vec::new();
        // SAFETY: standard Toolhelp enumeration; handles are closed.
        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
            let mut entry = PROCESSENTRY32W { dwSize: size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
            while more {
                let pid = entry.th32ProcessID;
                if pid != own && image_path(pid).is_some_and(|path| path.to_lowercase().starts_with(&prefix)) {
                    found.push(pid);
                }
                more = Process32NextW(snapshot, &mut entry).is_ok();
            }
            let _ = CloseHandle(snapshot);
        }
        found
    }

    pub fn terminate(pid: u32) {
        // SAFETY: the handle is opened for termination only and closed.
        unsafe {
            if let Ok(process) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(process, 0);
                let _ = CloseHandle(process);
            }
        }
    }

    fn image_path(pid: u32) -> Option<String> {
        // SAFETY: the buffer is large enough for any path; the handle is closed.
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buffer = [0u16; 1024];
            let mut len = buffer.len() as u32;
            let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buffer.as_mut_ptr()), &mut len).is_ok();
            let _ = CloseHandle(process);
            ok.then(|| String::from_utf16_lossy(&buffer[..len as usize]))
        }
    }
}
