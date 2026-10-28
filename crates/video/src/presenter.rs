//! Swap chain plus the Direct3D 11 video processor that converts and scales
//! decoded NV12 pictures into it, letterboxed to keep the aspect ratio.

use std::mem::ManuallyDrop;

use anyhow::{Context, Result};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_ALPHA_MODE_IGNORE, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709, DXGI_COLOR_SPACE_TYPE,
    DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P601, DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709,
    DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601, DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_UNKNOWN, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::*;
use windows::core::{GUID, Interface};

use crate::decoder::Frame;
use crate::sps::ColorInfo;

/// Background around the video and while waiting for a sender.
const BACKGROUND: [f32; 4] = [0.07, 0.07, 0.08, 1.0];

/// NVIDIA's video processor extension that turns on RTX Video Super
/// Resolution when upscaling. Ignored by other GPUs and drivers.
const NVIDIA_PPE_INTERFACE: GUID = GUID::from_u128(0xd43ce1b3_1f4b_48ac_baee_c3c25375e6f7);

#[repr(C)]
struct NvidiaStreamExtension {
    version: u32,
    method: u32,
    enable: u32,
}

fn input_color_space(color: ColorInfo) -> DXGI_COLOR_SPACE_TYPE {
    match (color.full_range, color.bt601) {
        (true, false) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P709,
        (true, true) => DXGI_COLOR_SPACE_YCBCR_FULL_G22_LEFT_P601,
        (false, false) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709,
        (false, true) => DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P601,
    }
}

struct Processor {
    key: (u32, u32, u32, u32),
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
}

pub struct Presenter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext1,
    swapchain: IDXGISwapChain1,
    width: u32,
    height: u32,
    processor: Option<Processor>,
}

impl Presenter {
    pub fn new(device: &ID3D11Device, context: &ID3D11DeviceContext, hwnd: HWND, width: u32, height: u32) -> Result<Self> {
        let (width, height) = (width.max(1), height.max(1));
        unsafe {
            let factory: IDXGIFactory2 = device.cast::<IDXGIDevice>()?.GetAdapter()?.GetParent()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: width,
                Height: height,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_NONE,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swapchain = factory
                .CreateSwapChainForHwnd(device, hwnd, &desc, None, None)
                .context("creating swap chain")?;
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            Ok(Self {
                device: device.clone(),
                context: context.clone(),
                video_device: device.cast().context("GPU has no Direct3D video support")?,
                video_context: context.cast()?,
                swapchain,
                width,
                height,
                processor: None,
            })
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        let (width, height) = (width.max(1), height.max(1));
        if (width, height) == (self.width, self.height) {
            return Ok(());
        }
        unsafe {
            self.swapchain
                .ResizeBuffers(0, width, height, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))
                .context("resizing swap chain")?;
        }
        self.width = width;
        self.height = height;
        Ok(())
    }

    pub fn present(&mut self, frame: Option<&Frame>) -> Result<()> {
        unsafe {
            let target: ID3D11Texture2D = self.swapchain.GetBuffer(0)?;
            match frame {
                Some(frame) => self.blit(frame, &target)?,
                None => {
                    let mut view = None;
                    self.device.CreateRenderTargetView(&target, None, Some(&mut view))?;
                    self.context.ClearRenderTargetView(&view.context("no render target view")?, &BACKGROUND);
                }
            }
            drop(target);
            self.swapchain.Present(1, DXGI_PRESENT(0)).ok().context("presenting")?;
        }
        Ok(())
    }

    unsafe fn blit(&mut self, frame: &Frame, target: &ID3D11Texture2D) -> Result<()> {
        unsafe {
            let mut input_desc = D3D11_TEXTURE2D_DESC::default();
            frame.texture.GetDesc(&mut input_desc);
            self.ensure_processor(input_desc.Width, input_desc.Height)?;
            let Processor { enumerator, processor, .. } = self.processor.as_ref().unwrap();

            let input_view_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                FourCC: 0,
                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPIV { MipSlice: 0, ArraySlice: frame.subresource },
                },
            };
            let mut input_view = None;
            self.video_device
                .CreateVideoProcessorInputView(&frame.texture, enumerator, &input_view_desc, Some(&mut input_view))
                .context("creating video input view")?;

            let output_view_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 { Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 } },
            };
            let mut output_view = None;
            self.video_device
                .CreateVideoProcessorOutputView(target, enumerator, &output_view_desc, Some(&mut output_view))
                .context("creating video output view")?;
            let output_view = output_view.context("no video output view")?;

            let full = RECT { left: 0, top: 0, right: self.width as i32, bottom: self.height as i32 };
            let dest = letterbox(frame.size(), (self.width, self.height));
            let vc = &self.video_context;
            vc.VideoProcessorSetStreamColorSpace1(processor, 0, input_color_space(frame.color));
            vc.VideoProcessorSetStreamSourceRect(processor, 0, true, Some(&frame.crop));
            vc.VideoProcessorSetStreamDestRect(processor, 0, true, Some(&dest));
            vc.VideoProcessorSetOutputTargetRect(processor, true, Some(&full));

            let stream = D3D11_VIDEO_PROCESSOR_STREAM {
                Enable: true.into(),
                pInputSurface: ManuallyDrop::new(input_view),
                ..Default::default()
            };
            let mut streams = [stream];
            let result = vc.VideoProcessorBlt(processor, &output_view, 0, &streams);
            ManuallyDrop::drop(&mut streams[0].pInputSurface);
            result.context("video processor blit")
        }
    }

    /// The video processor is tied to input and output sizes; recreate it
    /// when either changes.
    unsafe fn ensure_processor(&mut self, input_width: u32, input_height: u32) -> Result<()> {
        let key = (input_width, input_height, self.width, self.height);
        if self.processor.as_ref().is_none_or(|p| p.key != key) {
            unsafe {
                let rate = DXGI_RATIONAL { Numerator: 60, Denominator: 1 };
                let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                    InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                    InputFrameRate: rate,
                    InputWidth: input_width,
                    InputHeight: input_height,
                    OutputFrameRate: rate,
                    OutputWidth: self.width,
                    OutputHeight: self.height,
                    Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
                };
                let enumerator = self.video_device.CreateVideoProcessorEnumerator(&desc)?;
                let processor = self.video_device.CreateVideoProcessor(&enumerator, 0)?;

                let vc = &self.video_context;
                vc.VideoProcessorSetStreamFrameFormat(&processor, 0, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
                vc.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);
                vc.VideoProcessorSetOutputColorSpace1(&processor, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
                let vsr = NvidiaStreamExtension { version: 1, method: 2, enable: 1 };
                let vsr_enabled = vc
                    .VideoProcessorSetStreamExtension(
                        &processor,
                        0,
                        &NVIDIA_PPE_INTERFACE,
                        size_of::<NvidiaStreamExtension>() as u32,
                        &vsr as *const _ as *mut _,
                    )
                    >= 0; // HRESULT success
                tracing::debug!("video processor {key:?}, RTX Video Super Resolution requested: {vsr_enabled}");
                let [r, g, b, a] = BACKGROUND;
                let background = D3D11_VIDEO_COLOR {
                    Anonymous: D3D11_VIDEO_COLOR_0 { RGBA: D3D11_VIDEO_COLOR_RGBA { R: r, G: g, B: b, A: a } },
                };
                vc.VideoProcessorSetOutputBackgroundColor(&processor, false, &background);

                self.processor = Some(Processor { key, enumerator, processor });
            }
        }
        Ok(())
    }
}

/// Largest rectangle with the source aspect ratio, centred in the output.
fn letterbox((src_w, src_h): (u32, u32), (out_w, out_h): (u32, u32)) -> RECT {
    if src_w == 0 || src_h == 0 {
        return RECT { left: 0, top: 0, right: out_w as i32, bottom: out_h as i32 };
    }
    let scale = (out_w as f64 / src_w as f64).min(out_h as f64 / src_h as f64);
    let w = (src_w as f64 * scale).round() as i32;
    let h = (src_h as f64 * scale).round() as i32;
    let left = (out_w as i32 - w) / 2;
    let top = (out_h as i32 - h) / 2;
    RECT { left, top, right: left + w, bottom: top + h }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterbox_portrait_in_landscape() {
        let r = letterbox((500, 1000), (1000, 500));
        assert_eq!((r.left, r.top, r.right, r.bottom), (375, 0, 625, 500));
    }

    #[test]
    fn letterbox_same_aspect_fills() {
        let r = letterbox((1920, 1080), (960, 540));
        assert_eq!((r.left, r.top, r.right, r.bottom), (0, 0, 960, 540));
    }
}
