//! The mirroring window: a native window that shows the sender's screen.
//!
//! Shared by the app and the command line. It runs its own event loop on a
//! dedicated thread, leaving the main thread to the caller. The window stays hidden until a mirrored picture arrives and
//! hides again when mirroring stops. On Linux it is created and destroyed
//! instead, because Wayland has no hidden windows.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub mod playback;

use anyhow::{Context, Result};
use riflesso_airplay::{Codec, Event};
use riflesso_video::VideoOutput;
use winit::application::ApplicationHandler;
#[cfg(windows)]
use winit::dpi::LogicalSize;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
#[cfg(target_os = "linux")]
use winit::platform::wayland::{EventLoopBuilderExtWayland, WindowAttributesExtWayland};
#[cfg(windows)]
use winit::platform::windows::{EventLoopBuilderExtWindows, IconExtWindows, WindowAttributesExtWindows};
#[cfg(windows)]
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::{Fullscreen, Icon, Window, WindowAttributes, WindowId};

const DOUBLE_CLICK: Duration = Duration::from_millis(400);
const TITLE: &str = "Riflesso X";
/// The app icon, embedded in the executable under this id (by tauri-build for
/// the app, by `cli/build.rs` for the command line).
#[cfg(windows)]
const APP_ICON: u16 = 32512;
/// Wayland's app id: desktops show the icon of the .desktop file with this
/// name. The same as the main window's, which GTK names after the executable
/// (and so do the packages' .desktop files).
#[cfg(target_os = "linux")]
const APP_ID: &str = "riflesso-x";

pub enum MirrorEvent {
    AirPlay(Event),
    /// Bring the window to the front, if something is being mirrored.
    Show,
    /// Like `Show`, with a token the desktop issued for the user's latest
    /// click (Wayland's xdg-activation, X11's startup id). Wayland lets a
    /// window come to the front only with such a token.
    ShowActivated(String),
    Exit,
    /// A decoded picture is waiting. Sent by the Linux decoder's thread;
    /// callers have no reason to send it.
    Decoded,
}

pub type MirrorProxy = EventLoopProxy<MirrorEvent>;

/// Starts the mirroring window thread and returns a handle to talk to it.
/// `closed` receives a message each time the user closes the window.
pub fn spawn(dump: Option<PathBuf>, closed: mpsc::Sender<()>) -> Result<(MirrorProxy, std::thread::JoinHandle<()>)> {
    let (proxy_tx, proxy_rx) = mpsc::channel();
    let thread = std::thread::Builder::new().name("mirror-window".into()).spawn(move || {
        let event_loop = match EventLoop::<MirrorEvent>::with_user_event().with_any_thread(true).build() {
            Ok(event_loop) => event_loop,
            Err(e) => {
                let _ = proxy_tx.send(Err(anyhow::Error::from(e)));
                return;
            }
        };
        let mut window = match MirrorWindow::new(dump, closed, event_loop.create_proxy()) {
            Ok(window) => window,
            Err(e) => {
                let _ = proxy_tx.send(Err(e));
                return;
            }
        };
        let _ = proxy_tx.send(Ok(event_loop.create_proxy()));
        if let Err(e) = event_loop.run_app(&mut window) {
            tracing::error!("mirror window stopped: {e}");
        }
    })?;
    let proxy = proxy_rx.recv().context("mirror window thread exited")??;
    Ok((proxy, thread))
}

struct MirrorWindow {
    window: Option<Window>,
    #[cfg(windows)]
    hwnd: isize,
    #[cfg(target_os = "linux")]
    proxy: MirrorProxy,
    closed: mpsc::Sender<()>,
    video: Option<VideoOutput>,
    dump: Option<BufWriter<File>>,
    /// A new picture or a resize is waiting to be presented.
    dirty: bool,
    frame_size: Option<(u32, u32)>,
    sender: Option<String>,
    /// When the current mirror stream began, to measure the time to the first picture.
    stream_started: Option<Instant>,
    last_click: Option<Instant>,
    decode_errors: u32,
    present_errors: u32,
    stats: Stats,
}

impl MirrorWindow {
    fn new(dump: Option<PathBuf>, closed: mpsc::Sender<()>, proxy: MirrorProxy) -> Result<Self> {
        #[cfg(windows)]
        let _ = proxy;
        let dump = match dump {
            Some(path) => {
                let file = File::create(&path).with_context(|| format!("creating {}", path.display()))?;
                let full = std::path::absolute(&path).unwrap_or(path);
                tracing::info!("saving video to {}", full.display());
                Some(BufWriter::new(file))
            }
            None => None,
        };
        Ok(Self {
            window: None,
            #[cfg(windows)]
            hwnd: 0,
            #[cfg(target_os = "linux")]
            proxy,
            closed,
            video: None,
            dump,
            dirty: false,
            frame_size: None,
            sender: None,
            stream_started: None,
            last_click: None,
            decode_errors: 0,
            present_errors: 0,
            stats: Stats::new(),
        })
    }

    fn update_title(&self) {
        if let Some(window) = &self.window {
            match &self.sender {
                Some(sender) => window.set_title(&format!("{TITLE} — {sender}")),
                None => window.set_title(TITLE),
            }
        }
    }

    fn handle_airplay(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::ClientConnected { name, model, .. } => {
                self.sender = name.or(model);
                self.update_title();
            }
            Event::VideoFrame(frame) => self.show_frame(event_loop, frame.codec, &frame.annex_b),
            Event::MirrorStopped { .. } => self.stop_mirroring(),
            Event::ClientDisconnected { .. } => self.sender = None,
            Event::MirrorStarted { .. } => self.stream_started = Some(Instant::now()),
            Event::VideoFormat(_) | Event::CodeShown { .. } | Event::CodeHidden => {}
        }
    }

    fn stop_mirroring(&mut self) {
        if let Some(video) = &mut self.video {
            video.reset();
        }
        self.hide_window();
        self.frame_size = None;
        self.decode_errors = 0;
        self.present_errors = 0;
        if let Some(dump) = &mut self.dump {
            let _ = dump.flush();
        }
    }

    fn show_frame(&mut self, event_loop: &ActiveEventLoop, codec: Codec, annex_b: &[u8]) {
        if let Some(dump) = &mut self.dump
            && let Err(e) = dump.write_all(annex_b)
        {
            tracing::error!("writing video dump failed, stopping it: {e}");
            self.dump = None;
        }
        self.stats.add(annex_b.len());
        let Some(video) = &mut self.video else { return };

        let codec = match codec {
            Codec::H264 => riflesso_video::Codec::H264,
            Codec::Hevc => riflesso_video::Codec::Hevc,
        };
        if let Err(e) = video.decode(codec, annex_b) {
            self.decode_errors += 1;
            if self.decode_errors <= 5 {
                tracing::warn!("cannot decode frame: {e:#}");
            }
            return;
        }
        // On Linux the picture comes later, with `MirrorEvent::Decoded`.
        if cfg!(windows) {
            self.picture_ready(event_loop);
        }
    }

    /// A new picture was decoded: show the window for the first one and fit
    /// it to the picture when the sender rotates.
    fn picture_ready(&mut self, event_loop: &ActiveEventLoop) {
        let Some(video) = &self.video else { return };
        self.dirty = true;
        let size = video.frame_size();
        if size.is_some() && size != self.frame_size {
            let first = self.frame_size.is_none();
            let rotated = match (self.frame_size, size) {
                (Some((w0, h0)), Some((w1, h1))) => (w0 > h0) != (w1 > h1),
                _ => true,
            };
            self.frame_size = size;
            let reopen = rotated && self.must_reopen(event_loop);
            if reopen {
                self.hide_window();
            }
            if rotated {
                self.fit_window();
            }
            if first && let Some(started) = self.stream_started.take() {
                tracing::info!("first picture shown {} ms after the stream started", started.elapsed().as_millis());
            }
            if first || reopen {
                self.show_window(event_loop, None);
                self.update_title();
            }
        }
    }

    /// Whether to replace the window rather than resize it. Wayland windows
    /// cannot move themselves: resized in place, a window turning landscape
    /// would grow past the screen's edge, while a new one is placed on screen
    /// by the desktop.
    #[cfg(target_os = "linux")]
    fn must_reopen(&self, event_loop: &ActiveEventLoop) -> bool {
        winit::platform::wayland::ActiveEventLoopExtWayland::is_wayland(event_loop)
            && self.window.as_ref().is_some_and(|w| w.fullscreen().is_none() && !w.is_maximized())
    }

    #[cfg(windows)]
    fn must_reopen(&self, _event_loop: &ActiveEventLoop) -> bool {
        false
    }

    /// The window's drawing area changed size.
    fn resized(&mut self, size: PhysicalSize<u32>) {
        if let Some(video) = &mut self.video
            && let Err(e) = video.resize(size.width, size.height)
        {
            tracing::warn!("{e:#}");
        }
        self.dirty = true;
    }

    /// Resizes the window to the picture's aspect ratio, as large as the
    /// screen's work area allows, and anchors it to the top of the screen so
    /// that no part of the picture ends up under the taskbar.
    fn fit_window(&mut self) {
        let (Some(window), Some((w, h))) = (&self.window, self.frame_size) else { return };
        if window.fullscreen().is_some() || window.is_maximized() {
            return;
        }
        #[cfg(windows)]
        let work = work_area(self.hwnd);
        #[cfg(target_os = "linux")]
        let work = window.current_monitor().map(monitor_work_area);
        let Some(work) = work else { return };
        // Title bar and borders around the picture.
        let (outer, inner) = (window.outer_size(), window.inner_size());
        let frame_w = outer.width.saturating_sub(inner.width) as f64;
        let frame_h = outer.height.saturating_sub(inner.height) as f64;
        let size = fitted_size(&work, (frame_w, frame_h), (w, h));
        let applied = window.request_inner_size(size);
        let x = work.left + ((work.width as f64 - size.width as f64 - frame_w) / 2.0) as i32;
        window.set_outer_position(PhysicalPosition::new(x, work.top));
        // Some systems (X11, Wayland) apply the size at once and send no
        // `Resized` event for it.
        if let Some(size) = applied {
            self.resized(size);
        }
    }

    /// Shows the window for the current picture.
    #[cfg(windows)]
    fn show_window(&mut self, _event_loop: &ActiveEventLoop, _token: Option<String>) {
        if let Some(window) = &self.window {
            window.set_visible(true);
            window.set_minimized(false);
            window.focus_window();
        }
    }

    #[cfg(windows)]
    fn hide_window(&mut self) {
        if let Some(window) = &self.window {
            window.set_fullscreen(None);
            window.set_visible(false);
        }
    }

    /// Creates the window, already sized for the current picture, and starts
    /// drawing into it. With an activation token the desktop brings it to
    /// the front; an existing window is then replaced, since Wayland takes
    /// the token only when a window is created.
    #[cfg(target_os = "linux")]
    fn show_window(&mut self, event_loop: &ActiveEventLoop, token: Option<String>) {
        use winit::platform::startup_notify::WindowAttributesExtStartupNotify;
        use winit::window::ActivationToken;

        if let Some(window) = &self.window {
            if token.is_none() || window.fullscreen().is_some() {
                window.set_minimized(false);
                // Works on X11; Wayland ignores it.
                window.focus_window();
                return;
            }
            self.hide_window();
        }
        let (Some(video), Some(picture)) = (&mut self.video, self.frame_size) else { return };
        let mut attributes = window_attributes();
        if let Some(token) = token {
            attributes = attributes.with_activation_token(ActivationToken::from_raw(token));
        }
        let monitor = event_loop.primary_monitor().or_else(|| event_loop.available_monitors().next());
        if let Some(work) = monitor.map(monitor_work_area) {
            attributes = attributes.with_inner_size(fitted_size(&work, (0.0, 0.0), picture));
        }
        let result = event_loop.create_window(attributes).map_err(anyhow::Error::from).and_then(|window| {
            let size = window.inner_size();
            video.attach(&window, size.width, size.height)?;
            Ok(window)
        });
        match result {
            Ok(window) => {
                window.focus_window();
                self.window = Some(window);
                self.dirty = true;
            }
            Err(e) => tracing::error!("cannot create the video window: {e:#}"),
        }
    }

    #[cfg(target_os = "linux")]
    fn hide_window(&mut self) {
        if let Some(video) = &mut self.video {
            video.detach();
        }
        self.window = None;
    }

    fn toggle_fullscreen(&self) {
        if let Some(window) = &self.window {
            let next = match window.fullscreen() {
                Some(_) => None,
                None => Some(Fullscreen::Borderless(None)),
            };
            window.set_fullscreen(next);
        }
    }
}

impl ApplicationHandler<MirrorEvent> for MirrorWindow {
    /// Linux: only the decoder; the window comes with the first picture.
    #[cfg(target_os = "linux")]
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        if self.video.is_some() {
            return;
        }
        let proxy = self.proxy.clone();
        match VideoOutput::new(move || {
            let _ = proxy.send_event(MirrorEvent::Decoded);
        }) {
            Ok(video) => self.video = Some(video),
            Err(e) => tracing::error!("cannot start the video decoder: {e:#}"),
        }
    }

    #[cfg(windows)]
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attributes = window_attributes().with_visible(false).with_inner_size(LogicalSize::new(405.0, 720.0));
        let result = event_loop
            .create_window(attributes)
            .map_err(anyhow::Error::from)
            .and_then(|window| {
                let hwnd = match window.window_handle()?.as_raw() {
                    RawWindowHandle::Win32(handle) => handle.hwnd.get(),
                    other => anyhow::bail!("unsupported window handle {other:?}"),
                };
                style_title_bar(hwnd);
                self.hwnd = hwnd;
                let size = window.inner_size();
                let video = VideoOutput::new(hwnd, size.width, size.height)?;
                Ok((window, video))
            });
        match result {
            Ok((window, video)) => {
                self.window = Some(window);
                self.video = Some(video);
            }
            Err(e) => tracing::error!("cannot create the video window: {e:#}"),
        }
    }

    fn window_event(&mut self, _event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            // The window hides; the app decides whether the sender is also
            // disconnected (a user setting).
            WindowEvent::CloseRequested => {
                self.hide_window();
                let _ = self.closed.send(());
            }
            WindowEvent::Resized(size) => self.resized(size),
            WindowEvent::RedrawRequested => self.dirty = true,
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed && !event.repeat => {
                match event.logical_key {
                    Key::Named(NamedKey::F11) => self.toggle_fullscreen(),
                    Key::Named(NamedKey::Escape) => {
                        if let Some(window) = &self.window {
                            window.set_fullscreen(None);
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let now = Instant::now();
                if self.last_click.is_some_and(|t| now - t < DOUBLE_CLICK) {
                    self.toggle_fullscreen();
                    self.last_click = None;
                } else {
                    self.last_click = Some(now);
                }
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: MirrorEvent) {
        match event {
            MirrorEvent::AirPlay(event) => self.handle_airplay(event_loop, event),
            MirrorEvent::Show => {
                if self.frame_size.is_some() {
                    self.show_window(event_loop, None);
                }
            }
            MirrorEvent::ShowActivated(token) => {
                if self.frame_size.is_some() {
                    self.show_window(event_loop, Some(token));
                }
            }
            MirrorEvent::Decoded => {
                #[cfg(target_os = "linux")]
                if self.video.as_mut().is_some_and(VideoOutput::receive) {
                    self.picture_ready(event_loop);
                }
            }
            MirrorEvent::Exit => event_loop.exit(),
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        // Several frames may have been decoded since the last pass; only the
        // newest is shown.
        if std::mem::take(&mut self.dirty)
            && let Some(video) = &mut self.video
            && let Err(e) = video.present()
        {
            self.present_errors += 1;
            if self.present_errors <= 5 {
                tracing::warn!("cannot show frame: {e:#}");
            }
        }
        self.stats.report();
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(dump) = &mut self.dump {
            let _ = dump.flush();
        }
        // The drawing surface goes before its window.
        self.video = None;
        self.window = None;
    }
}

fn window_attributes() -> WindowAttributes {
    let icon = app_icon().inspect_err(|e| tracing::warn!("cannot load the app icon: {e:#}")).ok();
    let attributes = Window::default_attributes().with_title(TITLE).with_window_icon(icon.clone());
    #[cfg(windows)]
    let attributes = attributes.with_taskbar_icon(icon);
    #[cfg(target_os = "linux")]
    let attributes = attributes.with_name(APP_ID, APP_ID);
    attributes
}

#[cfg(windows)]
fn app_icon() -> Result<Icon> {
    Ok(Icon::from_resource(APP_ICON, None)?)
}

/// X11 shows this one; Wayland takes the icon from the .desktop file.
#[cfg(target_os = "linux")]
fn app_icon() -> Result<Icon> {
    let png = include_bytes!("../../../apps/desktop/icons/128x128.png");
    let mut decoder = png::Decoder::new(std::io::Cursor::new(&png[..]));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA);
    let mut reader = decoder.read_info()?;
    let mut rgba = vec![0; reader.output_buffer_size().context("icon too large")?];
    let frame = reader.next_frame(&mut rgba)?;
    anyhow::ensure!(frame.color_type == png::ColorType::Rgba && frame.bit_depth == png::BitDepth::Eight, "unexpected icon format");
    rgba.truncate(frame.buffer_size());
    Ok(Icon::from_rgba(rgba, frame.width, frame.height)?)
}

/// The largest size with the picture's proportions that fits the work area,
/// leaving `frame` (title bar and borders) around it.
fn fitted_size(work: &WorkArea, (frame_w, frame_h): (f64, f64), (w, h): (u32, u32)) -> PhysicalSize<u32> {
    let max_w = work.width as f64 * 0.9 - frame_w;
    let max_h = work.height as f64 - frame_h;
    let scale = (max_w / w as f64).min(max_h / h as f64);
    PhysicalSize::new((w as f64 * scale) as u32, (h as f64 * scale) as u32)
}

/// Paints the title bar in the app's ink colour, with light text and buttons.
/// Windows 10 does not support these attributes and keeps its own colours.
#[cfg(windows)]
fn style_title_bar(hwnd: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWINDOWATTRIBUTE,
        DwmSetWindowAttribute,
    };

    // Colours are COLORREF values: 0x00BBGGRR.
    let attributes: [(DWMWINDOWATTRIBUTE, u32); 4] = [
        (DWMWA_USE_IMMERSIVE_DARK_MODE, 1),
        (DWMWA_CAPTION_COLOR, 0x0018_120e), // #0e1218, ink
        (DWMWA_TEXT_COLOR, 0x00f5_f1ee),    // #eef1f5
        (DWMWA_BORDER_COLOR, 0x0030_241c),  // #1c2430, top of the icon's plate
    ];
    let hwnd = HWND(hwnd as *mut _);
    for (attribute, value) in attributes {
        let value = std::ptr::from_ref(&value).cast();
        // SAFETY: `hwnd` is our live window and `value` points to a u32 of the size given.
        let _ = unsafe { DwmSetWindowAttribute(hwnd, attribute, value, size_of::<u32>() as u32) };
    }
}

/// The part of a monitor not covered by the taskbar, in physical pixels.
struct WorkArea {
    left: i32,
    top: i32,
    width: i32,
    height: i32,
}

/// Work area of the monitor that holds most of the window.
#[cfg(windows)]
fn work_area(hwnd: isize) -> Option<WorkArea> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};

    if hwnd == 0 {
        return None;
    }
    let mut info = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
    // SAFETY: `hwnd` is our live window and `info` is a properly sized MONITORINFO.
    let found = unsafe {
        let monitor = MonitorFromWindow(HWND(hwnd as *mut _), MONITOR_DEFAULTTONEAREST);
        GetMonitorInfoW(monitor, &mut info).as_bool()
    };
    let r = info.rcWork;
    found.then_some(WorkArea { left: r.left, top: r.top, width: r.right - r.left, height: r.bottom - r.top })
}

/// Linux has no common way to ask for the space panels leave free, so a
/// slice of the monitor is kept for them and for the title bar, which Wayland
/// does not count in the window's outer size.
#[cfg(target_os = "linux")]
fn monitor_work_area(monitor: winit::monitor::MonitorHandle) -> WorkArea {
    let (position, size) = (monitor.position(), monitor.size());
    let height = (size.height as f64 * 0.88) as i32;
    WorkArea { left: position.x, top: position.y, width: size.width as i32, height }
}

struct Stats {
    frames: u32,
    bytes: usize,
    since: Instant,
}

impl Stats {
    fn new() -> Self {
        Self { frames: 0, bytes: 0, since: Instant::now() }
    }

    fn add(&mut self, bytes: usize) {
        self.frames += 1;
        self.bytes += bytes;
    }

    fn report(&mut self) {
        let elapsed = self.since.elapsed();
        if elapsed < Duration::from_secs(5) {
            return;
        }
        if self.frames > 0 {
            let secs = elapsed.as_secs_f64();
            tracing::info!(
                "video: {:.1} fps, {:.0} kbit/s",
                f64::from(self.frames) / secs,
                self.bytes as f64 * 8.0 / 1000.0 / secs
            );
        }
        *self = Self::new();
    }
}
