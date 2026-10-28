//! Runs the AirPlay receiver in the background and restarts it when the
//! settings change (a new name or resolution must be re-advertised).

use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use riflesso_airplay::{Event, ReceiverConfig};
use riflesso_audio::Player;
use tauri::async_runtime::JoinHandle;
use tokio::sync::{Mutex as AsyncMutex, Notify};

use crate::settings::Settings;
use crate::state::{AppState, Requested, Status};

pub struct Receiver {
    task: Mutex<Option<JoinHandle<()>>>,
    /// Held while restarting: two overlapping restarts would both start a receiver.
    restarting: AsyncMutex<()>,
    events: mpsc::Sender<Event>,
    /// The receiving end, until [`take_events`](Self::take_events) is called.
    receiver_events: Mutex<Option<mpsc::Receiver<Event>>>,
    audio: Arc<Player>,
    disconnect: Arc<Notify>,
    pub data_dir: PathBuf,
    hevc_available: bool,
    monitor: Option<(u32, u32)>,
}

impl Receiver {
    pub fn new_channel(data_dir: PathBuf, hevc_available: bool, monitor: Option<(u32, u32)>) -> Self {
        let (events, receiver_events) = mpsc::channel();
        Self {
            task: Mutex::new(None),
            restarting: AsyncMutex::new(()),
            events,
            receiver_events: Mutex::new(Some(receiver_events)),
            audio: Arc::new(Player::new()),
            disconnect: Arc::new(Notify::new()),
            data_dir,
            hevc_available,
            monitor,
        }
    }

    /// Events from every receiver run; can be taken once.
    pub fn take_events(&self) -> mpsc::Receiver<Event> {
        self.receiver_events.lock().unwrap().take().expect("receiver events already taken")
    }

    /// What `settings` ask senders for on this machine.
    pub fn requested(&self, settings: &Settings) -> Requested {
        let (display, hevc) = settings.display(self.hevc_available, self.monitor);
        Requested { width: display.width, height: display.height, hevc }
    }

    pub async fn restart(&self, settings: &Settings, state: Arc<AppState>) {
        let _restarting = self.restarting.lock().await;
        self.stop().await;

        let (display, hevc) = settings.display(self.hevc_available, self.monitor);
        let requested = self.requested(settings);
        state.update(|s| {
            s.status = Status::Starting;
            s.requested = requested;
            s.devices.clear();
        });
        let config = ReceiverConfig {
            name: settings.name.clone(),
            port: 7000,
            display,
            hevc,
            security: settings.security(),
            data_dir: self.data_dir.clone(),
        };
        let (events, audio, disconnect) = (self.events.clone(), self.audio.clone(), self.disconnect.clone());
        let task = tauri::async_runtime::spawn(async move {
            state.update(|s| s.status = Status::Running);
            if let Err(e) = riflesso_airplay::run(config, events, audio, disconnect).await {
                tracing::error!("receiver stopped: {e:#}");
                state.update(|s| s.status = Status::Error { message: format!("{e:#}") });
            }
        });
        *self.task.lock().unwrap() = Some(task);
    }

    /// Ends every sender's session; the receiver stays available.
    pub fn disconnect_all(&self) {
        self.disconnect.notify_waiters();
    }

    /// Stops the receiver, withdrawing its Bonjour advertisement.
    pub async fn stop(&self) {
        let task = self.task.lock().unwrap().take();
        if let Some(task) = task {
            task.abort();
            let _ = task.await;
        }
    }
}
