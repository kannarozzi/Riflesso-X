//! H.264/H.265 decoding with GStreamer:
//! `appsrc ! h264parse ! <decoder> ! videoconvert ! appsink`.
//!
//! The decoder is picked by hand rather than by decodebin, so that the log
//! says which one runs and a broken one can be skipped: NVDEC on NVIDIA,
//! VA-API on Intel and AMD, V4L2 on ARM boards, then software (libav,
//! OpenH264, libde265), whichever plugins the system has.

use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

use crate::Codec;

/// Decoders to try, best first.
fn candidates(codec: Codec) -> &'static [&'static str] {
    match codec {
        Codec::H264 => &["nvh264dec", "vah264dec", "v4l2slh264dec", "v4l2h264dec", "avdec_h264", "openh264dec"],
        Codec::Hevc => &["nvh265dec", "vah265dec", "v4l2slh265dec", "v4l2h265dec", "avdec_h265", "libde265dec"],
    }
}

fn parser(codec: Codec) -> &'static str {
    match codec {
        Codec::H264 => "h264parse",
        Codec::Hevc => "h265parse",
    }
}

fn is_hardware(factory: &gst::ElementFactory) -> bool {
    factory.metadata(gst::ELEMENT_METADATA_KLASS).is_some_and(|klass| klass.contains("Hardware"))
}

/// The first installed decoder for `codec` that is not in `skip`.
/// `RIFLESSO_SOFTWARE_DECODE=1` leaves out the hardware ones.
pub fn find_decoder(codec: Codec, skip: &[String]) -> Option<gst::ElementFactory> {
    let software_only = std::env::var_os("RIFLESSO_SOFTWARE_DECODE").is_some_and(|v| v == "1");
    gst::ElementFactory::find(parser(codec))?;
    candidates(codec)
        .iter()
        .filter(|name| !skip.iter().any(|s| s == *name))
        .filter_map(|name| gst::ElementFactory::find(name))
        .find(|factory| !(software_only && is_hardware(factory)))
}

pub struct Decoder {
    codec: Codec,
    name: String,
    hardware: bool,
    pipeline: gst::Pipeline,
    src: gst_app::AppSrc,
    sink: gst_app::AppSink,
    frames: u64,
}

impl Decoder {
    /// Builds and starts a pipeline with `factory`. `wake` is called from
    /// GStreamer's thread each time a picture is ready to be taken with
    /// [`latest`](Self::latest).
    pub fn new(codec: Codec, factory: &gst::ElementFactory, wake: Arc<dyn Fn() + Send + Sync>) -> Result<Self> {
        let caps = gst::Caps::builder(match codec {
            Codec::H264 => "video/x-h264",
            Codec::Hevc => "video/x-h265",
        })
        .field("stream-format", "byte-stream")
        .field("alignment", "au")
        .build();
        let src = gst_app::AppSrc::builder()
            .caps(&caps)
            .is_live(true)
            .format(gst::Format::Time)
            .stream_type(gst_app::AppStreamType::Stream)
            .build();
        let parse = gst::ElementFactory::make(parser(codec)).build().context("creating the stream parser")?;
        let decode = factory.create().build().with_context(|| format!("creating {}", factory.name()))?;
        // Hand pictures over as soon as they are decoded, not a few frames later.
        if decode.find_property("max-display-delay").is_some() {
            decode.set_property("max-display-delay", 0i32);
        }
        // Passes NV12 and I420 through untouched; converts anything else.
        let convert = gst::ElementFactory::make("videoconvert").build().context("creating videoconvert")?;
        let sink = gst_app::AppSink::builder()
            .caps(&gst::Caps::builder("video/x-raw").field("format", gst::List::new(["NV12", "I420"])).build())
            .sync(false)
            .max_buffers(2)
            .drop(true)
            .callbacks(
                gst_app::AppSinkCallbacks::builder()
                    .new_sample(move |_| {
                        wake();
                        Ok(gst::FlowSuccess::Ok)
                    })
                    .build(),
            )
            .build();

        let pipeline = gst::Pipeline::new();
        let elements = [src.upcast_ref(), &parse, &decode, &convert, sink.upcast_ref()];
        pipeline.add_many(elements)?;
        gst::Element::link_many(elements)?;
        pipeline.set_state(gst::State::Playing).with_context(|| format!("starting {}", factory.name()))?;
        Ok(Self {
            codec,
            name: factory.name().to_string(),
            hardware: is_hardware(factory),
            pipeline,
            src,
            sink,
            frames: 0,
        })
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_hardware(&self) -> bool {
        self.hardware
    }

    /// Queues one access unit (Annex-B). Fails if the pipeline reported an error.
    pub fn push(&mut self, annex_b: &[u8]) -> Result<()> {
        self.check_errors()?;
        let mut buffer = gst::Buffer::from_slice(annex_b.to_vec());
        // Only the order matters: the sink shows pictures as soon as they come.
        let pts = gst::ClockTime::from_nseconds(self.frames * 16_666_667);
        buffer.get_mut().expect("new buffer").set_pts(pts);
        self.frames += 1;
        self.src.push_buffer(buffer).map_err(|e| anyhow!("{} refused the frame: {e:?}", self.name))?;
        Ok(())
    }

    /// The newest decoded picture since the last call, if any.
    pub fn latest(&self) -> Option<gst::Sample> {
        std::iter::from_fn(|| self.sink.try_pull_sample(gst::ClockTime::ZERO)).last()
    }

    fn check_errors(&self) -> Result<()> {
        let Some(bus) = self.pipeline.bus() else { return Ok(()) };
        if let Some(message) = bus.pop_filtered(&[gst::MessageType::Error])
            && let gst::MessageView::Error(error) = message.view()
        {
            bail!("{} failed: {}", self.name, error.error());
        }
        Ok(())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}
