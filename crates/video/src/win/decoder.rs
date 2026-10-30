//! Media Foundation video decoders: the H.264 decoder built into Windows and,
//! when the "HEVC Video Extensions" are installed, the H.265 one.

use std::mem::ManuallyDrop;

use anyhow::{Context, Result, bail};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device, ID3D11DeviceContext,
    ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance, CoTaskMemFree};
use windows::core::{GUID, Interface};

use crate::Codec;
use super::sps::{self, ColorInfo};

/// A decoded picture on the GPU. Holding it keeps the decoder from reusing
/// its surface, so only the latest frame is kept.
pub struct Frame {
    pub texture: ID3D11Texture2D,
    pub subresource: u32,
    /// Visible area inside the (padded) decoded picture.
    pub crop: RECT,
    pub color: ColorInfo,
    _sample: Option<IMFSample>,
}

impl Frame {
    pub fn size(&self) -> (u32, u32) {
        ((self.crop.right - self.crop.left) as u32, (self.crop.bottom - self.crop.top) as u32)
    }
}

pub struct Decoder {
    codec: Codec,
    transform: IMFTransform,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    hardware: bool,
    /// Whether the decoder allocates output samples itself (always true with DXVA).
    provides_samples: bool,
    output_buffer_size: u32,
    width: u32,
    height: u32,
    crop: RECT,
    color: ColorInfo,
    /// Software path only: NV12 texture the CPU-side output is copied into.
    upload: Option<ID3D11Texture2D>,
}

impl Decoder {
    pub fn new(
        codec: Codec,
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        manager: &IMFDXGIDeviceManager,
    ) -> Result<Self> {
        unsafe {
            let transform: IMFTransform = match codec {
                Codec::H264 => CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER)
                    .context("creating the Media Foundation H.264 decoder")?,
                Codec::Hevc => find_decoder(&MFVideoFormat_HEVC)?
                    .context("no H.265 decoder installed (get \"HEVC Video Extensions\" from the Microsoft Store)")?,
            };
            let attributes = transform.GetAttributes()?;
            // Output every picture as soon as it is decoded: mirroring has no B-frames.
            attributes.SetUINT32(&MF_LOW_LATENCY, 1)?;

            let mut hardware = false;
            if std::env::var_os("RIFLESSO_SOFTWARE_DECODE").is_none()
                && attributes.GetUINT32(&MF_SA_D3D11_AWARE).unwrap_or(0) != 0
            {
                match transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize) {
                    Ok(()) => hardware = true,
                    Err(e) => tracing::warn!("hardware decoding unavailable, using software: {e}"),
                }
            }

            let input = MFCreateMediaType()?;
            input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            let subtype = match codec {
                Codec::H264 => MFVideoFormat_H264,
                Codec::Hevc => MFVideoFormat_HEVC,
            };
            input.SetGUID(&MF_MT_SUBTYPE, &subtype)?;
            // Some decoders (H.265) only offer output types once they know a
            // frame size; the stream's real size arrives as a format change.
            input.SetUINT64(&MF_MT_FRAME_SIZE, (1920u64 << 32) | 1080)?;
            transform.SetInputType(0, &input, 0).context("setting decoder input type")?;

            let mut decoder = Self {
                codec,
                transform,
                device: device.clone(),
                context: context.clone(),
                hardware,
                provides_samples: false,
                output_buffer_size: 0,
                width: 0,
                height: 0,
                crop: RECT::default(),
                color: ColorInfo::default(),
                upload: None,
            };
            decoder.negotiate_output()?;
            decoder.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
            decoder.transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
            Ok(decoder)
        }
    }

    pub fn is_hardware(&self) -> bool {
        self.hardware
    }

    pub fn codec(&self) -> Codec {
        self.codec
    }

    /// Feeds one access unit and returns the newest picture it produced, if any.
    pub fn decode(&mut self, data: &[u8], timestamp: i64) -> Result<Option<Frame>> {
        if self.codec == Codec::H264
            && let Some(color) = sps::find_h264_color(data) {
                if color != self.color {
                    tracing::info!("video colour: {color:?}");
                }
                self.color = color;
            }
        let sample = unsafe { input_sample(data, timestamp)? };
        let mut latest = None;
        // A full decoder only accepts input again after its output is drained.
        for _ in 0..2 {
            match unsafe { self.transform.ProcessInput(0, &sample, 0) } {
                Ok(()) => {
                    self.drain(&mut latest)?;
                    return Ok(latest);
                }
                Err(e) if e.code() == MF_E_NOTACCEPTING => self.drain(&mut latest)?,
                Err(e) => return Err(e).context("decoder rejected input"),
            }
        }
        bail!("decoder does not accept input")
    }

    fn drain(&mut self, latest: &mut Option<Frame>) -> Result<()> {
        loop {
            let provided = if self.provides_samples { None } else { Some(unsafe { self.output_sample()? }) };
            let mut buffer = MFT_OUTPUT_DATA_BUFFER {
                dwStreamID: 0,
                pSample: ManuallyDrop::new(provided),
                dwStatus: 0,
                pEvents: ManuallyDrop::new(None),
            };
            let mut status = 0;
            let result = unsafe { self.transform.ProcessOutput(0, std::slice::from_mut(&mut buffer), &mut status) };
            let sample = ManuallyDrop::into_inner(buffer.pSample);
            drop(ManuallyDrop::into_inner(buffer.pEvents));

            match result {
                Ok(()) => {
                    if let Some(sample) = sample {
                        *latest = Some(unsafe { self.frame_from_sample(sample)? });
                    }
                }
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => return Ok(()),
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => self.negotiate_output()?,
                Err(e) => return Err(e).context("decoding failed"),
            }
        }
    }

    /// Selects NV12 output and records the picture geometry. Called at start
    /// and whenever the stream format changes (e.g. the phone rotates).
    fn negotiate_output(&mut self) -> Result<()> {
        unsafe {
            let mut index = 0;
            let media_type = loop {
                let candidate = self
                    .transform
                    .GetOutputAvailableType(0, index)
                    .context("decoder offers no NV12 output")?;
                if candidate.GetGUID(&MF_MT_SUBTYPE)? == MFVideoFormat_NV12 {
                    break candidate;
                }
                index += 1;
            };
            self.transform.SetOutputType(0, &media_type, 0).context("setting decoder output type")?;

            let size = media_type.GetUINT64(&MF_MT_FRAME_SIZE)?;
            self.width = (size >> 32) as u32;
            self.height = size as u32;
            self.crop = display_aperture(&media_type)
                .unwrap_or(RECT { left: 0, top: 0, right: self.width as i32, bottom: self.height as i32 });

            let info = self.transform.GetOutputStreamInfo(0)?;
            let provides = (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0) as u32;
            self.provides_samples = info.dwFlags & provides != 0;
            self.output_buffer_size = info.cbSize;
            self.upload = None;
            tracing::debug!(
                "decoder output {}x{}, visible {:?}, provides samples: {}",
                self.width,
                self.height,
                self.crop,
                self.provides_samples
            );
            Ok(())
        }
    }

    unsafe fn output_sample(&self) -> Result<IMFSample> {
        unsafe {
            let sample = MFCreateSample()?;
            sample.AddBuffer(&MFCreateMemoryBuffer(self.output_buffer_size)?)?;
            Ok(sample)
        }
    }

    unsafe fn frame_from_sample(&mut self, sample: IMFSample) -> Result<Frame> {
        unsafe {
            let buffer = sample.GetBufferByIndex(0)?;
            if let Ok(dxgi) = buffer.cast::<IMFDXGIBuffer>() {
                let mut texture: Option<ID3D11Texture2D> = None;
                dxgi.GetResource(&ID3D11Texture2D::IID, &mut texture as *mut _ as *mut _)?;
                return Ok(Frame {
                    texture: texture.context("decoder output has no texture")?,
                    subresource: dxgi.GetSubresourceIndex()?,
                    crop: self.crop,
                    color: self.color,
                    _sample: Some(sample),
                });
            }

            // Software decoding: NV12 in system memory, Y plane then UV plane.
            let texture = match &self.upload {
                Some(texture) => texture.clone(),
                None => {
                    let desc = D3D11_TEXTURE2D_DESC {
                        Width: self.width,
                        Height: self.height,
                        MipLevels: 1,
                        ArraySize: 1,
                        Format: DXGI_FORMAT_NV12,
                        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                        Usage: D3D11_USAGE_DEFAULT,
                        // No bind flags: the video processor reads it directly.
                        BindFlags: 0,
                        ..Default::default()
                    };
                    let mut texture = None;
                    self.device.CreateTexture2D(&desc, None, Some(&mut texture))?;
                    let texture = texture.context("no upload texture")?;
                    self.upload = Some(texture.clone());
                    texture
                }
            };
            let mut data = std::ptr::null_mut();
            let mut length = 0;
            buffer.Lock(&mut data, None, Some(&mut length))?;
            let needed = self.width * self.height * 3 / 2;
            if length >= needed {
                self.context.UpdateSubresource(&texture, 0, None, data as *const _, self.width, 0);
            }
            buffer.Unlock()?;
            if length < needed {
                bail!("decoded picture is {length} bytes, expected {needed}");
            }
            Ok(Frame { texture, subresource: 0, crop: self.crop, color: self.color, _sample: None })
        }
    }
}

unsafe fn input_sample(data: &[u8], timestamp: i64) -> Result<IMFSample> {
    unsafe {
        let buffer = MFCreateMemoryBuffer(data.len() as u32)?;
        let mut ptr = std::ptr::null_mut();
        buffer.Lock(&mut ptr, None, None)?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len());
        buffer.Unlock()?;
        buffer.SetCurrentLength(data.len() as u32)?;

        let sample = MFCreateSample()?;
        sample.AddBuffer(&buffer)?;
        sample.SetSampleTime(timestamp)?;
        Ok(sample)
    }
}

/// Reads `MF_MT_MINIMUM_DISPLAY_APERTURE` (an `MFVideoArea`): the picture
/// without the padding added to reach whole macroblocks.
unsafe fn display_aperture(media_type: &IMFMediaType) -> Option<RECT> {
    let mut blob = [0u8; 16];
    unsafe { media_type.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut blob, None).ok()? };
    let x = i16::from_le_bytes([blob[2], blob[3]]) as i32;
    let y = i16::from_le_bytes([blob[6], blob[7]]) as i32;
    let w = i32::from_le_bytes(blob[8..12].try_into().unwrap());
    let h = i32::from_le_bytes(blob[12..16].try_into().unwrap());
    (w > 0 && h > 0).then_some(RECT { left: x, top: y, right: x + w, bottom: y + h })
}

/// First synchronous decoder registered for `subtype`, if any.
pub(crate) fn find_decoder(subtype: &GUID) -> Result<Option<IMFTransform>> {
    unsafe {
        let input = MFT_REGISTER_TYPE_INFO { guidMajorType: MFMediaType_Video, guidSubtype: *subtype };
        let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0;
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_DECODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            None,
            &mut activates,
            &mut count,
        )?;
        if activates.is_null() {
            return Ok(None);
        }
        let list = std::slice::from_raw_parts_mut(activates, count as usize);
        let first = list.first().and_then(|a| a.as_ref()).map(|a| a.ActivateObject::<IMFTransform>());
        for activate in list.iter_mut() {
            drop(activate.take());
        }
        CoTaskMemFree(Some(activates as *const _));
        first.transpose().map_err(Into::into)
    }
}
