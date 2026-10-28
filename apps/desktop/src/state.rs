//! What the user interface shows. The whole snapshot is sent to the web view
//! (event `snapshot`) whenever something changes; it is small.

use std::sync::Mutex;

use riflesso_airplay::{Codec, Event};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::settings::Settings;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub version: &'static str,
    pub status: Status,
    pub settings: Settings,
    /// An H.265 decoder is installed.
    pub hevc_available: bool,
    /// What we currently ask senders for.
    pub requested: Requested,
    pub devices: Vec<Device>,
    /// Code to show while a device asks for it.
    pub code: Option<String>,
    /// A newer version to tell the user about.
    pub update: Option<crate::updates::UpdateInfo>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "camelCase")]
pub enum Status {
    Starting,
    Running,
    Error { message: String },
    /// Replaying a file (`--play`); no receiver.
    Playback,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Requested {
    pub width: u16,
    pub height: u16,
    pub hevc: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    /// Address of the control connection.
    pub id: String,
    #[serde(skip)]
    ip: std::net::IpAddr,
    pub name: String,
    pub model: String,
    pub kind: DeviceKind,
    pub mirroring: bool,
    pub video: Option<VideoInfo>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DeviceKind {
    Iphone,
    Ipad,
    Mac,
    Other,
}

impl DeviceKind {
    fn from_model(model: &str) -> Self {
        if model.starts_with("iPhone") {
            Self::Iphone
        } else if model.starts_with("iPad") {
            Self::Ipad
        } else if model.contains("Mac") {
            Self::Mac
        } else {
            Self::Other
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoInfo {
    pub codec: &'static str,
    pub width: u32,
    pub height: u32,
}

pub struct AppState {
    app: AppHandle,
    snapshot: Mutex<Snapshot>,
}

impl AppState {
    pub fn new(app: AppHandle, snapshot: Snapshot) -> Self {
        Self { app, snapshot: Mutex::new(snapshot) }
    }

    pub fn app(&self) -> &AppHandle {
        &self.app
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().unwrap().clone()
    }

    /// Changes the snapshot and pushes it to the user interface.
    pub fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        let snapshot = {
            let mut snapshot = self.snapshot.lock().unwrap();
            change(&mut snapshot);
            snapshot.clone()
        };
        let _ = self.app.emit("snapshot", snapshot);
    }

    /// Tracks connected devices from receiver events (not video frames).
    pub fn apply(&self, event: &Event) {
        self.update(|s| match event {
            Event::ClientConnected { peer, name, model } => {
                let model = model.clone().unwrap_or_default();
                let device = Device {
                    id: peer.to_string(),
                    ip: peer.ip(),
                    name: name.clone().unwrap_or_else(|| model.clone()),
                    kind: DeviceKind::from_model(&model),
                    model,
                    mirroring: false,
                    video: None,
                };
                s.devices.retain(|d| d.id != device.id);
                s.devices.push(device);
            }
            // The mirror stream uses its own connection: match by address.
            Event::MirrorStarted { peer } => {
                for d in s.devices.iter_mut().filter(|d| d.ip == peer.ip()) {
                    d.mirroring = true;
                }
            }
            Event::VideoFormat(format) => {
                if let Some(d) = s.devices.iter_mut().find(|d| d.mirroring) {
                    d.video = Some(VideoInfo {
                        codec: match format.codec {
                            Codec::H264 => "H.264",
                            Codec::Hevc => "H.265",
                        },
                        width: format.width,
                        height: format.height,
                    });
                }
            }
            Event::MirrorStopped { peer } => {
                for d in s.devices.iter_mut().filter(|d| d.ip == peer.ip()) {
                    d.mirroring = false;
                    d.video = None;
                }
            }
            Event::ClientDisconnected { peer } => s.devices.retain(|d| d.id != peer.to_string()),
            Event::CodeShown { code } => s.code = Some(code.clone()),
            Event::CodeHidden => s.code = None,
            Event::VideoFrame(_) => {}
        });
    }
}
