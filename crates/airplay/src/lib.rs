//! AirPlay screen-mirroring receiver.
//!
//! [`run`] advertises the receiver over Bonjour, accepts sender connections
//! and reports what happens through [`Event`]s: connection changes, the
//! video format, and decrypted H.264/H.265 access units in Annex-B form.
//! Decoded audio goes straight to an [`AudioSink`].

mod audio;
mod auth;
mod consts;
mod crypto;
mod identity;
mod mdns;
mod mirror;
mod ntp;
mod rtsp;
mod session;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tokio::net::TcpListener;
use tokio::sync::Notify;

use auth::{AuthState, Registry};
use identity::Identity;
use mdns::{Advertiser, TxtRecords};

pub use auth::{CODE_LIFETIME, Security, forget_paired_devices};

#[derive(Debug, Clone)]
pub struct ReceiverConfig {
    /// Name shown in the sender's AirPlay menu.
    pub name: String,
    /// RTSP control port (7000 is what Apple TVs use).
    pub port: u16,
    pub display: DisplayConfig,
    /// Offer H.265 mirroring (needs an H.265 decoder). Senders only use it
    /// for displays taller than 1080 pixels.
    pub hevc: bool,
    /// Who may mirror: anyone, with a password, or with a code shown on screen.
    pub security: Security,
    /// Where the persistent identity (device ID, pairing key) and the devices
    /// paired with a code are stored.
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Copy)]
pub struct DisplayConfig {
    pub width: u16,
    pub height: u16,
    pub refresh_rate: u16,
    pub max_fps: u16,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self { width: 1920, height: 1080, refresh_rate: 60, max_fps: 60 }
    }
}

impl DisplayConfig {
    /// The display to advertise: `size`, or the monitor's size when `None`
    /// (between 1080p and 4K). Senders pick the stream size from the height
    /// and only send more than 1080 lines in H.265, so without `hevc` the
    /// display is capped at 1080p.
    pub fn choose(size: Option<(u16, u16)>, monitor: Option<(u32, u32)>, hevc: bool) -> Self {
        let (width, height) = size
            .or_else(|| monitor.map(|(w, h)| (w.min(3840) as u16, h.min(2160) as u16)).filter(|&(_, h)| h >= 1080))
            .unwrap_or((1920, 1080));
        let (width, height) = if hevc || height <= 1080 { (width, height) } else { (1920, 1080) };
        Self { width, height, ..Self::default() }
    }
}

#[derive(Debug)]
pub enum Event {
    ClientConnected { peer: SocketAddr, name: Option<String>, model: Option<String> },
    MirrorStarted { peer: SocketAddr },
    VideoFormat(VideoFormat),
    VideoFrame(VideoFrame),
    MirrorStopped { peer: SocketAddr },
    ClientDisconnected { peer: SocketAddr },
    /// Show this code on screen: the sender asks the user to type it.
    CodeShown { code: String },
    /// The code is no longer needed (entered correctly, or replaced).
    CodeHidden,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoFormat {
    pub codec: Codec,
    pub width: u32,
    pub height: u32,
    pub source_width: u32,
    pub source_height: u32,
}

#[derive(Debug)]
pub struct VideoFrame {
    pub codec: Codec,
    /// H.264 or H.265 access unit with Annex-B start codes; carries SPS/PPS when they changed.
    pub annex_b: Vec<u8>,
    pub keyframe: bool,
    /// Sender clock, NTP format.
    pub timestamp: u64,
}

/// Receives decoded audio. Called from network tasks, so implementations
/// must hand samples over to the playback thread without blocking for long.
pub trait AudioSink: Send + Sync {
    /// A stream begins; samples will be interleaved at this rate.
    fn start(&self, sample_rate: u32, channels: u16);
    fn samples(&self, interleaved: &[f32]);
    /// Linear gain from the sender's volume control (0 = muted).
    fn set_volume(&self, gain: f32);
    fn stop(&self);
}

pub(crate) struct Shared {
    name: String,
    identity: Identity,
    security: Security,
    auth: Mutex<AuthState>,
    registry: Mutex<Registry>,
    txt: TxtRecords,
    features: u64,
    display: DisplayConfig,
    events: Sender<Event>,
    audio: Arc<dyn AudioSink>,
}

/// Runs the receiver until the listener fails. Drop the future to stop it
/// and close all connections. Notifying `disconnect` closes the current
/// connections but keeps the receiver running.
pub async fn run(
    config: ReceiverConfig,
    events: Sender<Event>,
    audio: Arc<dyn AudioSink>,
    disconnect: Arc<Notify>,
) -> Result<()> {
    let identity = Identity::load_or_create(&config.data_dir)?;
    let listener = TcpListener::bind(("0.0.0.0", config.port))
        .await
        .with_context(|| format!("cannot listen on port {} (is another AirPlay receiver running?)", config.port))?;
    let port = listener.local_addr()?.port();

    let features = consts::features(config.hevc);
    let txt = TxtRecords::new(&identity, features, &config.security);
    let _advertiser = Advertiser::start(&config.name, &identity, &txt, port)?;
    tracing::info!("receiver \"{}\" ready, device ID {}", config.name, identity.device_id_string());

    tracing::info!(
        "requesting {}x{} video{}",
        config.display.width,
        config.display.height,
        if config.hevc { ", H.265 allowed" } else { "" }
    );
    let shared = Arc::new(Shared {
        name: config.name,
        identity,
        registry: Mutex::new(Registry::load(&config.data_dir)),
        security: config.security,
        auth: Mutex::default(),
        txt,
        features,
        display: config.display,
        events,
        audio,
    });
    // Sessions live in this set: dropping the future (to stop or restart the
    // receiver) also ends every connection.
    let mut sessions = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, peer) = accepted.context("accepting connection")?;
                tracing::info!("connection from {peer}");
                sessions.spawn(session::serve(stream, peer, shared.clone()));
            }
            Some(_) = sessions.join_next() => {}
            () = disconnect.notified() => {
                tracing::info!("disconnecting {} connection(s)", sessions.len());
                sessions.abort_all();
            }
        }
    }
}
