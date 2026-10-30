//! Linux: GStreamer decodes (with whichever hardware decoder the system has,
//! see [`decoder`]) and OpenGL draws (see [`renderer`]).
//!
//! Unlike on Windows, decoding runs on GStreamer's own thread, and the window
//! can come and go: on Wayland a window cannot be hidden, so the mirroring
//! window is created for the first picture and destroyed when mirroring
//! stops. Hence the separate [`attach`](VideoOutput::attach) and the `wake`
//! callback, which tells the window thread to [`receive`](VideoOutput::receive).

mod decoder;
mod renderer;

use std::sync::Arc;

use anyhow::{Context, Result};
use gstreamer as gst;
use gstreamer_video as gst_video;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

use crate::Codec;
use decoder::Decoder;
use renderer::Renderer;

fn init() -> Result<()> {
    gst::init().context("starting GStreamer")
}

/// Whether an H.265 decoder is installed (GStreamer's nvcodec, va, libav or
/// libde265 plugins).
pub fn hevc_supported() -> bool {
    init().is_ok() && decoder::find_decoder(Codec::Hevc, &[]).is_some()
}

/// Size of the largest connected display, in pixels, from the kernel's list
/// of screens (Wayland does not tell applications which one is primary).
pub fn primary_display_size() -> Option<(u32, u32)> {
    std::fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .filter(|entry| std::fs::read_to_string(entry.path().join("status")).is_ok_and(|s| s.trim() == "connected"))
        .filter_map(|entry| {
            let modes = std::fs::read_to_string(entry.path().join("modes")).ok()?;
            let (w, h) = modes.lines().next()?.split_once('x')?;
            // Interlaced modes end in "i".
            Some((w.parse().ok()?, h.trim_end_matches(|c: char| !c.is_ascii_digit()).parse().ok()?))
        })
        .max_by_key(|&(w, h): &(u32, u32)| w * h)
}

pub struct VideoOutput {
    wake: Arc<dyn Fn() + Send + Sync>,
    decoder: Option<Decoder>,
    /// Decoders that failed on this machine, not to be tried again.
    failed: Vec<String>,
    renderer: Option<Renderer>,
    /// The newest picture, kept while no window is attached.
    sample: Option<gst::Sample>,
    uploaded: bool,
    frame_size: Option<(u32, u32)>,
}

impl VideoOutput {
    /// `wake` is called from another thread whenever a decoded picture is
    /// waiting for [`receive`](Self::receive).
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self> {
        init()?;
        Ok(Self {
            wake: Arc::new(wake),
            decoder: None,
            failed: Vec::new(),
            renderer: None,
            sample: None,
            uploaded: false,
            frame_size: None,
        })
    }

    /// Starts drawing into `window`, which must outlive the next [`detach`](Self::detach).
    pub fn attach(&mut self, window: &(impl HasWindowHandle + HasDisplayHandle), width: u32, height: u32) -> Result<()> {
        self.renderer = None;
        self.renderer = Some(Renderer::new(window, width, height)?);
        self.uploaded = false;
        Ok(())
    }

    /// Stops drawing; call it before the window goes away.
    pub fn detach(&mut self) {
        self.renderer = None;
    }

    /// Queues one access unit (Annex-B) for decoding.
    pub fn decode(&mut self, codec: Codec, annex_b: &[u8]) -> Result<()> {
        if self.decoder.as_ref().is_none_or(|d| d.codec() != codec) {
            self.decoder = None;
            let factory = decoder::find_decoder(codec, &self.failed).with_context(|| {
                format!("no {codec:?} decoder: install GStreamer's nvcodec or va plugins (gst-plugins-bad), or gst-libav")
            })?;
            let decoder = Decoder::new(codec, &factory, self.wake.clone())?;
            tracing::info!(
                "{codec:?} decoder ready: {} ({})",
                decoder.name(),
                if decoder.is_hardware() { "hardware accelerated" } else { "software" }
            );
            self.decoder = Some(decoder);
        }
        let decoder = self.decoder.as_mut().expect("just created");
        if let Err(e) = decoder.push(annex_b) {
            // Next time, the next decoder on the list.
            self.failed.push(decoder.name().to_owned());
            self.decoder = None;
            return Err(e);
        }
        Ok(())
    }

    /// Takes the newest decoded picture, if a new one came. Returns whether
    /// one did.
    pub fn receive(&mut self) -> bool {
        let Some(sample) = self.decoder.as_ref().and_then(Decoder::latest) else { return false };
        let size = sample.caps().and_then(|caps| gst_video::VideoInfo::from_caps(caps).ok()).map(|i| (i.width(), i.height()));
        if size.is_none() {
            return false;
        }
        self.frame_size = size;
        self.sample = Some(sample);
        self.uploaded = false;
        true
    }

    /// Forgets the current stream; the window goes back to the idle background.
    pub fn reset(&mut self) {
        self.decoder = None;
        self.sample = None;
        self.frame_size = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.clear();
        }
    }

    /// Visible size of the latest decoded picture.
    pub fn frame_size(&self) -> Option<(u32, u32)> {
        self.frame_size
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        if let Some(renderer) = &mut self.renderer {
            renderer.resize(width, height);
        }
        Ok(())
    }

    /// Draws the latest picture (or the idle background) and presents it.
    pub fn present(&mut self) -> Result<()> {
        let Some(renderer) = &mut self.renderer else { return Ok(()) };
        if !self.uploaded
            && let Some(sample) = &self.sample
        {
            let buffer = sample.buffer().context("decoded sample without a buffer")?;
            let caps = sample.caps().context("decoded sample without caps")?;
            let info = gst_video::VideoInfo::from_caps(caps).context("reading the picture format")?;
            let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
                .map_err(|_| anyhow::anyhow!("cannot read the decoded picture"))?;
            renderer.upload(&frame);
            self.uploaded = true;
        }
        renderer.draw()
    }
}

impl Drop for VideoOutput {
    fn drop(&mut self) {
        // The pipeline first: its thread may still call `wake`.
        self.decoder = None;
    }
}
