//! Windows: the Media Foundation decoders that ship with Windows, with DXVA
//! hardware acceleration when the GPU supports it. Decoded frames stay on the
//! GPU: the Direct3D 11 video processor converts them from NV12 to RGB and
//! scales them straight into the window's swap chain.

mod decoder;
mod sps;
mod presenter;

use anyhow::{Context, Result};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D10::ID3D10Multithread;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
    ID3D11Device, ID3D11DeviceContext,
};
use windows::Win32::Media::MediaFoundation::{
    IMFDXGIDeviceManager, MF_VERSION, MFCreateDXGIDeviceManager, MFSTARTUP_NOSOCKET, MFStartup,
};
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx};
use windows::core::Interface;

use decoder::{Decoder, Frame};
use presenter::Presenter;

use crate::Codec;

/// Whether an H.265 decoder is installed ("HEVC Video Extensions").
pub fn hevc_supported() -> bool {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        if MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_err() {
            return false;
        }
    }
    matches!(decoder::find_decoder(&windows::Win32::Media::MediaFoundation::MFVideoFormat_HEVC), Ok(Some(_)))
}

/// Resolution of the primary monitor, in physical pixels.
pub fn primary_display_size() -> Option<(u32, u32)> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1().ok()?;
        let output = factory.EnumAdapters1(0).ok()?.EnumOutputs(0).ok()?;
        let rect = output.GetDesc().ok()?.DesktopCoordinates;
        Some(((rect.right - rect.left) as u32, (rect.bottom - rect.top) as u32))
    }
}

pub struct VideoOutput {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    manager: IMFDXGIDeviceManager,
    presenter: Presenter,
    decoder: Option<Decoder>,
    frame: Option<Frame>,
    timestamp: i64,
}

impl VideoOutput {
    /// Creates the GPU device and a swap chain for the given window.
    pub fn new(hwnd: isize, width: u32, height: u32) -> Result<Self> {
        unsafe {
            // Media Foundation needs COM; winit may already have initialised it.
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).context("starting Media Foundation")?;

            let mut device = None;
            let mut context = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .context("creating Direct3D 11 device")?;
            let device: ID3D11Device = device.context("no Direct3D device")?;
            let context = context.context("no Direct3D context")?;
            // The decoder uses the device from Media Foundation worker threads.
            let _ = device.cast::<ID3D10Multithread>()?.SetMultithreadProtected(true);

            let mut token = 0;
            let mut manager = None;
            MFCreateDXGIDeviceManager(&mut token, &mut manager).context("creating DXGI device manager")?;
            let manager = manager.context("no DXGI device manager")?;
            manager.ResetDevice(&device, token)?;

            let presenter = Presenter::new(&device, &context, HWND(hwnd as *mut _), width, height)?;
            Ok(Self { device, context, manager, presenter, decoder: None, frame: None, timestamp: 0 })
        }
    }

    /// Decodes one access unit (Annex-B). The newest decoded picture is
    /// shown by the next [`present`](Self::present).
    pub fn decode(&mut self, codec: Codec, annex_b: &[u8]) -> Result<()> {
        if self.decoder.as_ref().is_none_or(|d| d.codec() != codec) {
            self.decoder = None;
            let decoder = Decoder::new(codec, &self.device, &self.context, &self.manager)?;
            tracing::info!(
                "{codec:?} decoder ready ({})",
                if decoder.is_hardware() { "hardware accelerated" } else { "software" }
            );
            self.decoder = Some(decoder);
        }
        let decoder = self.decoder.as_mut().unwrap();
        // 100 ns units; only the ordering matters to the decoder.
        self.timestamp += 166_666;
        if let Some(frame) = decoder.decode(annex_b, self.timestamp)? {
            self.frame = Some(frame);
        }
        Ok(())
    }

    /// Forgets the current stream; the window goes back to the idle background.
    pub fn reset(&mut self) {
        self.frame = None;
        self.decoder = None;
    }

    /// Visible size of the latest decoded picture.
    pub fn frame_size(&self) -> Option<(u32, u32)> {
        self.frame.as_ref().map(Frame::size)
    }

    pub fn resize(&mut self, width: u32, height: u32) -> Result<()> {
        self.presenter.resize(width, height)
    }

    /// Draws the latest picture (or the idle background) and presents it.
    pub fn present(&mut self) -> Result<()> {
        self.presenter.present(self.frame.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> (ID3D11Device, ID3D11DeviceContext, IMFDXGIDeviceManager) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).unwrap();
            let (mut device, mut context) = (None, None);
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                Default::default(),
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .unwrap();
            let device: ID3D11Device = device.unwrap();
            let _ = device.cast::<ID3D10Multithread>().unwrap().SetMultithreadProtected(true);
            let (mut token, mut manager) = (0, None);
            MFCreateDXGIDeviceManager(&mut token, &mut manager).unwrap();
            let manager = manager.unwrap();
            manager.ResetDevice(&device, token).unwrap();
            (device, context.unwrap(), manager)
        }
    }

    /// Needs a GPU; the H.265 part also needs "HEVC Video Extensions".
    #[test]
    #[ignore]
    fn decoders_initialise() {
        let (device, context, manager) = device();
        let h264 = Decoder::new(Codec::H264, &device, &context, &manager).unwrap();
        println!("H.264 hardware: {}", h264.is_hardware());
        if hevc_supported() {
            let hevc = Decoder::new(Codec::Hevc, &device, &context, &manager).unwrap();
            println!("H.265 hardware: {}", hevc.is_hardware());
        } else {
            println!("no H.265 decoder installed");
        }
    }
}
