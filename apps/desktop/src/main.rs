// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod receiver;
mod settings;
mod setup;
mod state;
mod updates;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

use anyhow::{Context, Result, bail};
use riflesso_airplay::Event;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, RunEvent, State, WebviewWindowBuilder};
use tauri_plugin_opener::OpenerExt;
use tracing_subscriber::EnvFilter;

use riflesso_window::{MirrorEvent, MirrorProxy, playback};
use receiver::Receiver;
use settings::{CloseAction, MIN_PASSWORD_LEN, MirrorCloseAction, SecurityMode, Settings};
use state::{AppState, Snapshot, Status};

const REPOSITORY: &str = "https://github.com/kannarozzi/Riflesso-X";

const USAGE: &str = "\
Usage: riflesso-x [--minimized] [--dump FILE]
       riflesso-x --play FILE

  --minimized   start in the notification area, without the main window
  --dump FILE   also write the received video stream to FILE
  --play FILE   replay a stream saved with --dump instead of receiving";

#[derive(Default)]
struct Args {
    minimized: bool,
    dump: Option<PathBuf>,
    play: Option<PathBuf>,
}

fn parse_args() -> Result<Args> {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        let mut value = || it.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--minimized" => args.minimized = true,
            "--dump" => args.dump = Some(value()?.into()),
            "--play" => args.play = Some(value()?.into()),
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => bail!("unknown argument {other:?}\n\n{USAGE}"),
        }
    }
    Ok(args)
}

/// Settings, identity, logs and the web view's files live in a `data` folder
/// next to the executable, so everything stays in the app's own folder: the
/// per-user installation or an unzipped portable copy. Only if that folder
/// cannot be written (e.g. a copy under Program Files) %APPDATA% is used.
fn data_dir() -> PathBuf {
    let local = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("data")));
    match local {
        Some(dir) if is_writable(&dir) => dir,
        _ => std::env::var_os("APPDATA").map_or_else(|| PathBuf::from("data"), |dir| PathBuf::from(dir).join("Riflesso X")),
    }
}

fn is_writable(dir: &std::path::Path) -> bool {
    let probe = dir.join(".write-test");
    std::fs::create_dir_all(dir).is_ok() && std::fs::write(&probe, b"").is_ok() && std::fs::remove_file(probe).is_ok()
}

/// Logs go to the terminal in debug builds and to a file in release builds,
/// which have no console.
fn init_logging(data_dir: &std::path::Path) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if cfg!(debug_assertions) {
        tracing_subscriber::fmt().with_env_filter(filter).init();
        return;
    }
    let _ = std::fs::create_dir_all(data_dir);
    match std::fs::File::create(data_dir.join("riflesso-x.log")) {
        Ok(file) => tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(Mutex::new(file)).init(),
        Err(_) => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
}

/// The mirroring window's proxy, shareable with Tauri commands.
struct Mirror(Mutex<MirrorProxy>);

impl Mirror {
    fn send(&self, event: MirrorEvent) {
        let _ = self.0.lock().unwrap().send_event(event);
    }
}

#[tauri::command]
fn get_snapshot(state: State<'_, Arc<AppState>>) -> Snapshot {
    state.snapshot()
}

/// Saves the settings. The receiver restarts only when what senders see
/// changed (name, resolution, codec, security).
#[tauri::command]
async fn save_settings(
    mut settings: Settings,
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    receiver: State<'_, Arc<Receiver>>,
) -> Result<(), String> {
    settings.name = settings.name.trim().to_string();
    if settings.name.is_empty() || settings.name.chars().count() > 50 {
        return Err("The name must be 1 to 50 characters long.".into());
    }
    if settings.security == SecurityMode::Password && settings.password.chars().count() < MIN_PASSWORD_LEN {
        return Err(format!("The password must be at least {MIN_PASSWORD_LEN} characters long."));
    }
    settings.save(&receiver.data_dir).map_err(|e| format!("Cannot save the settings: {e}"))?;
    let previous = state.snapshot().settings;
    state.update(|s| s.settings = settings.clone());
    if previous.language != settings.language {
        update_tray_menu(&app, &settings.language);
    }
    if previous.receiver_differs(&settings) {
        receiver.restart(&settings, state.inner().clone()).await;
    }
    Ok(())
}

/// Starts the receiver again, e.g. after the port was busy.
#[tauri::command]
async fn restart_receiver(state: State<'_, Arc<AppState>>, receiver: State<'_, Arc<Receiver>>) -> Result<(), ()> {
    let settings = state.snapshot().settings;
    receiver.restart(&settings, state.inner().clone()).await;
    Ok(())
}

/// Forgets the devices paired with an on-screen code: they will be asked
/// for a new one.
#[tauri::command]
async fn forget_paired_devices(
    state: State<'_, Arc<AppState>>,
    receiver: State<'_, Arc<Receiver>>,
) -> Result<(), String> {
    riflesso_airplay::forget_paired_devices(&receiver.data_dir).map_err(|e| e.to_string())?;
    receiver.restart(&state.snapshot().settings, state.inner().clone()).await;
    Ok(())
}

#[tauri::command]
fn open_repository(app: AppHandle) -> Result<(), String> {
    app.opener().open_url(REPOSITORY, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
fn show_mirror(mirror: State<'_, Mirror>) {
    mirror.send(MirrorEvent::Show);
}

/// Shows the main window, creating it again if it was closed: closing it
/// destroys the web view so that nothing but the receiver stays in memory.
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main") else { return };
    // WebView2 keeps its profile in our data folder instead of %LOCALAPPDATA%.
    let webview_data = app.state::<Arc<Receiver>>().data_dir.join("webview");
    match WebviewWindowBuilder::from_config(app, config).and_then(|builder| builder.data_directory(webview_data).build()) {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(e) => tracing::error!("cannot open the main window: {e}"),
    }
}

/// Labels of the tray menu, in the languages the interface offers.
fn tray_labels(language: &str) -> (&'static str, &'static str) {
    match language {
        "it" => ("Mostra Riflesso X", "Esci"),
        "es" => ("Mostrar Riflesso X", "Salir"),
        "fr" => ("Afficher Riflesso X", "Quitter"),
        "de" => ("Riflesso X anzeigen", "Beenden"),
        "pt-BR" => ("Mostrar Riflesso X", "Sair"),
        "ja" => ("Riflesso X を表示", "終了"),
        "zh-CN" => ("显示 Riflesso X", "退出"),
        "ko" => ("Riflesso X 표시", "종료"),
        "ru" => ("Показать Riflesso X", "Выход"),
        _ => ("Show Riflesso X", "Quit"),
    }
}

fn tray_menu<R: tauri::Runtime, M: Manager<R>>(manager: &M, language: &str) -> tauri::Result<Menu<R>> {
    let (show, quit) = tray_labels(language);
    let show = MenuItem::with_id(manager, "show", show, true, None::<&str>)?;
    let quit = MenuItem::with_id(manager, "quit", quit, true, None::<&str>)?;
    Menu::with_items(manager, &[&show, &quit])
}

fn update_tray_menu(app: &AppHandle, language: &str) {
    if let Some(tray) = app.tray_by_id("main")
        && let Err(e) = tray_menu(app, language).and_then(|menu| tray.set_menu(Some(menu)))
    {
        tracing::warn!("cannot update the tray menu: {e}");
    }
}

fn create_tray(app: &tauri::App, language: &str) -> tauri::Result<()> {
    let menu = tray_menu(app, language)?;
    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("bundled icon"))
        .tooltip("Riflesso X")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn main() {
    let mut context = Some(tauri::generate_context!());
    // `--setup` / `--uninstall`: the app installs or removes itself.
    if let Some(code) = setup::run_if_requested(&mut context) {
        std::process::exit(code);
    }
    let data_dir = data_dir();
    init_logging(&data_dir);
    if let Err(e) = run(data_dir, context.take().expect("context")) {
        tracing::error!("{e:#}");
        std::process::exit(1);
    }
}

fn run(data_dir: PathBuf, context: tauri::Context<tauri::Wry>) -> Result<()> {
    let args = parse_args()?;
    let settings = Settings::load(&data_dir);
    let hevc_available = riflesso_video::hevc_supported();
    let monitor = riflesso_video::primary_display_size();
    let (mirror_closed, mirror_closed_events) = mpsc::channel();
    let (mirror, _mirror_thread) = riflesso_window::spawn(args.dump.clone(), mirror_closed)?;

    let receiver = Arc::new(Receiver::new_channel(data_dir, hevc_available, monitor));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main_window(app)))
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            save_settings,
            restart_receiver,
            forget_paired_devices,
            show_mirror,
            open_repository,
            updates::check_for_updates,
            updates::install_update,
            updates::remind_update_later,
            updates::skip_update
        ])
        .setup({
            let mirror = mirror.clone();
            let receiver = receiver.clone();
            move |app| {
                let snapshot = Snapshot {
                    version: env!("CARGO_PKG_VERSION"),
                    status: if args.play.is_some() { Status::Playback } else { Status::Starting },
                    requested: receiver.requested(&settings),
                    settings: settings.clone(),
                    hevc_available,
                    devices: Vec::new(),
                    code: None,
                    update: None,
                };
                let state = Arc::new(AppState::new(app.handle().clone(), snapshot));
                app.manage(state.clone());
                app.manage(Mirror(Mutex::new(mirror.clone())));
                app.manage(receiver.clone());
                let updates = Arc::new(updates::Updates::default());
                app.manage(updates.clone());
                updates::spawn_checks(state.clone(), updates);
                create_tray(app, &settings.language)?;
                on_mirror_closed(mirror_closed_events, state.clone(), receiver.clone());

                match args.play {
                    Some(path) => playback::spawn(path, mirror),
                    None => {
                        forward_events(receiver.take_events(), state.clone(), mirror);
                        tauri::async_runtime::spawn(async move { receiver.restart(&settings, state).await });
                    }
                }
                // The main window is not created from the configuration
                // (`"create": false`), so that it can use our data folder.
                if !args.minimized {
                    show_main_window(app.handle());
                }
                Ok(())
            }
        })
        .build(context)
        .context("starting the user interface")?;

    app.run(move |app, event| match event {
        // Closing the main window quits, or keeps Riflesso X receiving from
        // the tray if the user chose so. "Quit" in the tray always exits.
        RunEvent::ExitRequested { code: None, api, .. }
            if app.state::<Arc<AppState>>().snapshot().settings.close_action == CloseAction::Tray =>
        {
            api.prevent_exit()
        }
        RunEvent::Exit => {
            tauri::async_runtime::block_on(receiver.stop());
            let _ = mirror.send_event(MirrorEvent::Exit);
        }
        _ => {}
    });
    Ok(())
}

/// A device asks for the code: bring the main window up (it may be closed or
/// minimised) and hide the code again once it has expired.
fn show_code(state: &Arc<AppState>, code: String) {
    let app = state.app().clone();
    let _ = app.clone().run_on_main_thread(move || show_main_window(&app));
    let state = state.clone();
    std::thread::spawn(move || {
        std::thread::sleep(riflesso_airplay::CODE_LIFETIME);
        state.update(|s| {
            if s.code.as_ref() == Some(&code) {
                s.code = None;
            }
        });
    });
}

/// Closing the mirroring window also ends the session, unless the user
/// chose to only hide it.
fn on_mirror_closed(closed: mpsc::Receiver<()>, state: Arc<AppState>, receiver: Arc<Receiver>) {
    std::thread::Builder::new()
        .name("mirror-closed".into())
        .spawn(move || {
            for () in closed {
                if state.snapshot().settings.mirror_close_action == MirrorCloseAction::Disconnect {
                    receiver.disconnect_all();
                }
            }
        })
        .expect("spawning mirror-closed thread");
}

/// Receiver events go to the mirroring window; all but video frames also
/// update what the user interface shows.
fn forward_events(events: mpsc::Receiver<Event>, state: Arc<AppState>, mirror: MirrorProxy) {
    std::thread::Builder::new()
        .name("receiver-events".into())
        .spawn(move || {
            for event in events {
                if let Event::CodeShown { code } = &event {
                    show_code(&state, code.clone());
                }
                if !matches!(event, Event::VideoFrame(_)) {
                    state.apply(&event);
                }
                if mirror.send_event(MirrorEvent::AirPlay(event)).is_err() {
                    break;
                }
            }
        })
        .expect("spawning event thread");
}
