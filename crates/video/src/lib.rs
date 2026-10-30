//! Video output for Riflesso X: H.264/H.265 access units in, pixels on a
//! window out. Each system has its own implementation behind the same
//! [`VideoOutput`]:
//!
//! - Windows (`win`): Media Foundation decoding and the Direct3D 11 video
//!   processor;
//! - Linux (`linux`): GStreamer decoding (NVDEC, VA-API or software, whichever
//!   is installed) and OpenGL drawing.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    H264,
    Hevc,
}

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::{VideoOutput, hevc_supported, primary_display_size};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{VideoOutput, hevc_supported, primary_display_size};
