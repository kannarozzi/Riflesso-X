//! `riflesso`: Riflesso X from the command line. The same AirPlay receiver and
//! mirroring window as the app, without its interface: settings come from the
//! arguments and what happens is printed here. Ctrl+C quits.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, mpsc};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use riflesso_airplay::{Codec, DisplayConfig, Event, ReceiverConfig, Security};
use riflesso_window::{MirrorEvent, MirrorProxy, playback};
use tokio::sync::Notify;
use tracing_subscriber::EnvFilter;

const MIN_PASSWORD_LEN: usize = 4;

/// AirPlay screen mirroring receiver for Windows (Riflesso X, command line).
///
/// Shows the screen of an iPhone, iPad or Mac in a window. On the device, open
/// Control Center, tap Screen Mirroring and choose this receiver's name.
#[derive(Parser)]
#[command(name = "riflesso", version)]
struct Args {
    /// Name shown under Screen Mirroring on the device.
    #[arg(short, long, default_value = "Riflesso X")]
    name: String,

    /// Display size to request; "auto" is the size of the main monitor.
    #[arg(short, long, value_enum, default_value_t = Resolution::Auto)]
    resolution: Resolution,

    /// Don't offer H.265 (the picture is then limited to 1080p).
    #[arg(long)]
    no_hevc: bool,

    /// Ask devices for this password.
    #[arg(long, value_name = "PASSWORD", conflicts_with_all = ["code", "code_every_time"])]
    password: Option<String>,

    /// Show a code here the first time a device connects; it is then remembered.
    #[arg(long, conflicts_with = "code_every_time")]
    code: bool,

    /// Show a new code here at every connection.
    #[arg(long)]
    code_every_time: bool,

    /// Closing the window only hides it; the device keeps mirroring.
    #[arg(long)]
    hide_on_close: bool,

    /// Where the identity, remembered devices and other data are kept
    /// [default: "data" next to the executable].
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,

    /// Also save the received video stream (Annex-B) to FILE.
    #[arg(long, value_name = "FILE")]
    dump: Option<PathBuf>,

    /// Replay a stream saved with --dump instead of receiving.
    #[arg(long, value_name = "FILE")]
    play: Option<PathBuf>,

    /// Don't ask GitHub whether a newer version is out.
    #[arg(long)]
    no_update_check: bool,

    /// Show more details: -v for the protocol log, -vv for everything.
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Clone, Copy, ValueEnum)]
enum Resolution {
    Auto,
    #[value(name = "1080p")]
    P1080,
    #[value(name = "1440p")]
    P1440,
    #[value(name = "2160p")]
    P2160,
}

impl Resolution {
    fn size(self) -> Option<(u16, u16)> {
        match self {
            Resolution::Auto => None,
            Resolution::P1080 => Some((1920, 1080)),
            Resolution::P1440 => Some((2560, 1440)),
            Resolution::P2160 => Some((3840, 2160)),
        }
    }
}

fn main() -> ExitCode {
    let args = Args::parse();
    init_logging(args.verbose);
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Our own messages go to stdout; the log (warnings by default) to stderr.
fn init_logging(verbose: u8) {
    let level = match verbose {
        0 => "warn",
        1 => "info,riflesso_airplay=debug",
        _ => "debug",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));
    tracing_subscriber::fmt().with_env_filter(filter).with_writer(std::io::stderr).init();
}

fn run(args: Args) -> Result<()> {
    let security = security(&args)?;
    let runtime = tokio::runtime::Runtime::new().context("starting the async runtime")?;
    let (closed, closed_events) = mpsc::channel();
    let (mirror, window) = riflesso_window::spawn(args.dump.clone(), closed)?;

    let result = match args.play {
        Some(path) => {
            playback::spawn(path, mirror.clone());
            println!("Playing the file. Press Ctrl+C to quit.");
            runtime.block_on(tokio::signal::ctrl_c()).context("waiting for Ctrl+C")
        }
        None => {
            let disconnect = Arc::new(Notify::new());
            on_window_closed(closed_events, disconnect.clone(), args.hide_on_close);

            let hevc = !args.no_hevc && riflesso_video::hevc_supported();
            let display = DisplayConfig::choose(args.resolution.size(), riflesso_video::primary_display_size(), hevc);
            let config = ReceiverConfig {
                name: args.name.clone(),
                port: 7000,
                display,
                hevc,
                security,
                data_dir: args.data_dir.unwrap_or_else(default_data_dir),
            };
            let (events, receiver_events) = mpsc::channel();
            report_events(receiver_events, mirror.clone());

            println!(
                "Riflesso X {} receiving as \"{}\" ({}×{}, {}). Press Ctrl+C to quit.",
                env!("CARGO_PKG_VERSION"),
                args.name,
                display.width,
                display.height,
                if hevc { "H.264/H.265" } else { "H.264" },
            );
            if !args.no_update_check {
                announce_newer_version();
            }
            let audio = Arc::new(riflesso_audio::Player::new());
            runtime.block_on(async {
                tokio::select! {
                    result = riflesso_airplay::run(config, events, audio, disconnect) => result,
                    _ = tokio::signal::ctrl_c() => Ok(()),
                }
            })
        }
    };

    let _ = mirror.send_event(MirrorEvent::Exit);
    let _ = window.join();
    result
}

/// Asks GitHub in the background and prints a line if a newer version is out.
fn announce_newer_version() {
    std::thread::spawn(|| {
        let current = env!("CARGO_PKG_VERSION");
        if let Ok(release) = riflesso_updates::latest()
            && riflesso_updates::is_newer(&release.version, current)
        {
            println!(
                "\n  Riflesso X {} is available (you have {current}). To update, run in PowerShell:\n  \
                 irm https://raw.githubusercontent.com/kannarozzi/Riflesso-X/main/scripts/install.ps1 | iex\n",
                release.version
            );
        }
    });
}

fn security(args: &Args) -> Result<Security> {
    Ok(match (&args.password, args.code, args.code_every_time) {
        (Some(password), _, _) => {
            if password.chars().count() < MIN_PASSWORD_LEN {
                bail!("the password must be at least {MIN_PASSWORD_LEN} characters long");
            }
            Security::Password(password.clone())
        }
        (None, true, _) => Security::Code,
        (None, false, true) => Security::CodeEveryTime,
        (None, false, false) => Security::None,
    })
}

/// "data" next to the executable, like the app; %APPDATA% if that folder
/// cannot be written.
fn default_data_dir() -> PathBuf {
    let local = std::env::current_exe().ok().and_then(|exe| Some(exe.parent()?.join("data")));
    match local {
        Some(dir) if is_writable(&dir) => dir,
        _ => std::env::var_os("APPDATA").map_or_else(|| PathBuf::from("data"), |dir| PathBuf::from(dir).join("Riflesso X")),
    }
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".write-test");
    std::fs::create_dir_all(dir).is_ok() && std::fs::write(&probe, b"").is_ok() && std::fs::remove_file(probe).is_ok()
}

/// Closing the mirroring window ends the session, unless asked to only hide it.
fn on_window_closed(closed: mpsc::Receiver<()>, disconnect: Arc<Notify>, hide_only: bool) {
    std::thread::spawn(move || {
        for () in closed {
            if !hide_only {
                disconnect.notify_waiters();
            }
        }
    });
}

/// Prints what happens and passes every event on to the mirroring window.
fn report_events(events: mpsc::Receiver<Event>, mirror: MirrorProxy) {
    std::thread::spawn(move || {
        let mut names: HashMap<SocketAddr, String> = HashMap::new();
        for event in events {
            match &event {
                Event::ClientConnected { peer, name, model } => {
                    let name = name.clone().or_else(|| model.clone()).unwrap_or_else(|| peer.ip().to_string());
                    println!("● {name} connected");
                    names.insert(*peer, name);
                }
                Event::VideoFormat(format) => {
                    let codec = match format.codec {
                        Codec::H264 => "H.264",
                        Codec::Hevc => "H.265",
                    };
                    println!("  mirroring in {codec}, {}×{}", format.width, format.height);
                }
                Event::MirrorStopped { .. } => println!("  mirroring stopped"),
                Event::ClientDisconnected { peer } => {
                    let name = names.remove(peer).unwrap_or_else(|| peer.ip().to_string());
                    println!("○ {name} disconnected");
                }
                Event::CodeShown { code } => {
                    let spaced: Vec<String> = code.chars().map(String::from).collect();
                    println!("\n  AirPlay code:  {}\n  Enter it on the device when asked for a password.\n", spaced.join(" "));
                }
                Event::MirrorStarted { .. } | Event::VideoFrame(_) | Event::CodeHidden => {}
            }
            if mirror.send_event(MirrorEvent::AirPlay(event)).is_err() {
                break;
            }
        }
    });
}
