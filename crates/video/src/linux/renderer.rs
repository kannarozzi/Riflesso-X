//! Draws decoded pictures into the window with OpenGL ES 3 (EGL, on Wayland
//! and X11): the Y and chroma planes are uploaded as textures, a shader turns
//! them into RGB with the stream's own matrix and range, and the picture is
//! letterboxed to keep its aspect ratio.

use std::num::NonZeroU32;

use anyhow::{Context, Result, anyhow};
use glow::HasContext;
use glutin::config::{ConfigTemplateBuilder, GlConfig};
use glutin::context::{ContextApi, ContextAttributesBuilder, NotCurrentGlContext, PossiblyCurrentContext, Version};
use glutin::display::{Display, DisplayApiPreference, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use gstreamer as gst;
use gstreamer_video::{self as gst_video, VideoFrameExt};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawWindowHandle};

/// Background around the video and while waiting for a sender.
const BACKGROUND: [f32; 4] = [0.07, 0.07, 0.08, 1.0];

const VERTEX_SHADER: &str = r"#version 300 es
out vec2 uv;
void main() {
    // A quad covering the viewport, as a triangle strip of 4 vertices.
    vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
    uv = vec2(corner.x, 1.0 - corner.y);
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}";

const FRAGMENT_SHADER: &str = r"#version 300 es
precision mediump float;
in vec2 uv;
out vec4 color;
uniform sampler2D plane_y;
uniform sampler2D plane_u;
uniform sampler2D plane_v;
uniform bool semi_planar;
uniform mat3 yuv_to_rgb;
uniform vec3 offset;
void main() {
    float y = texture(plane_y, uv).r;
    vec2 cbcr = semi_planar ? texture(plane_u, uv).rg : vec2(texture(plane_u, uv).r, texture(plane_v, uv).r);
    color = vec4(clamp(yuv_to_rgb * (vec3(y, cbcr) - offset), 0.0, 1.0), 1.0);
}";

/// How YUV becomes RGB for a stream: `rgb = matrix * (yuv - offset)`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Conversion {
    /// Column-major, as OpenGL expects.
    matrix: [f32; 9],
    offset: [f32; 3],
}

impl Conversion {
    fn new(colorimetry: gst_video::VideoColorimetry) -> Self {
        use gst_video::{VideoColorMatrix, VideoColorRange};
        let (kr, kb) = match colorimetry.matrix() {
            VideoColorMatrix::Bt601 => (0.299, 0.114),
            VideoColorMatrix::Bt2020 => (0.2627, 0.0593),
            VideoColorMatrix::Smpte240m => (0.212, 0.087),
            _ => (0.2126, 0.0722),
        };
        // iPhones send full range: unknown means trust that, as on Windows.
        let full = colorimetry.range() != VideoColorRange::Range16_235;
        let (y_scale, c_scale, y_offset) =
            if full { (1.0, 1.0, 0.0) } else { (255.0 / 219.0, 255.0 / 224.0, 16.0 / 255.0) };
        let kg = 1.0 - kr - kb;
        let r_cr = 2.0 * (1.0 - kr) * c_scale;
        let b_cb = 2.0 * (1.0 - kb) * c_scale;
        let g_cb = -2.0 * kb * (1.0 - kb) / kg * c_scale;
        let g_cr = -2.0 * kr * (1.0 - kr) / kg * c_scale;
        Self {
            // Columns: Y, Cb, Cr.
            matrix: [y_scale, y_scale, y_scale, 0.0, g_cb, b_cb, r_cr, g_cr, 0.0],
            offset: [y_offset, 128.0 / 255.0, 128.0 / 255.0],
        }
    }
}

/// Where a `src` picture goes in an `out` area: as large as fits, centred.
/// Returns x, y (from the bottom, as OpenGL counts), width and height.
fn letterbox((src_w, src_h): (u32, u32), (out_w, out_h): (u32, u32)) -> (i32, i32, i32, i32) {
    let scale = (f64::from(out_w) / f64::from(src_w)).min(f64::from(out_h) / f64::from(src_h));
    let w = (f64::from(src_w) * scale).round() as i32;
    let h = (f64::from(src_h) * scale).round() as i32;
    ((out_w as i32 - w) / 2, (out_h as i32 - h) / 2, w, h)
}

/// One plane of the picture as a texture, reallocated when its size changes.
struct Plane {
    texture: glow::Texture,
    size: (i32, i32),
}

pub struct Renderer {
    gl: glow::Context,
    program: glow::Program,
    vertex_array: glow::VertexArray,
    planes: [Plane; 3],
    semi_planar: bool,
    conversion: Option<Conversion>,
    picture: Option<(u32, u32)>,
    width: u32,
    height: u32,
    // Dropped after the GL objects above, which belong to this context.
    surface: Surface<WindowSurface>,
    context: PossiblyCurrentContext,
    _display: Display,
}

fn size(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value.max(1)).expect("at least 1")
}

impl Renderer {
    pub fn new(window: &(impl HasWindowHandle + HasDisplayHandle), width: u32, height: u32) -> Result<Self> {
        let raw_display = window.display_handle().map_err(|e| anyhow!("display handle: {e}"))?.as_raw();
        let raw_window = window.window_handle().map_err(|e| anyhow!("window handle: {e}"))?.as_raw();
        let wayland = matches!(raw_window, RawWindowHandle::Wayland(_));
        // SAFETY: the handles belong to a live window, which the caller keeps
        // until this renderer is dropped.
        unsafe {
            let display = Display::new(raw_display, DisplayApiPreference::Egl).context("opening EGL")?;
            let template = ConfigTemplateBuilder::new().compatible_with_native_window(raw_window).build();
            let config = display
                .find_configs(template)
                .context("listing EGL configurations")?
                .min_by_key(|c| (c.alpha_size(), c.num_samples()))
                .context("no EGL configuration for the window")?;
            let attributes = ContextAttributesBuilder::new()
                .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
                .build(Some(raw_window));
            let context = display.create_context(&config, &attributes).context("creating the OpenGL ES 3 context")?;
            let surface_attributes =
                SurfaceAttributesBuilder::<WindowSurface>::new().build(raw_window, size(width), size(height));
            let surface =
                display.create_window_surface(&config, &surface_attributes).context("creating the window surface")?;
            let context = context.make_current(&surface).context("activating the OpenGL context")?;
            // On Wayland the compositor already avoids tearing, and waiting for
            // vsync would block while the window is not on screen.
            let interval = if wayland { SwapInterval::DontWait } else { SwapInterval::Wait(size(1)) };
            if let Err(e) = surface.set_swap_interval(&context, interval) {
                tracing::debug!("cannot set the swap interval: {e}");
            }
            let gl = glow::Context::from_loader_function_cstr(|name| display.get_proc_address(name));
            tracing::info!("OpenGL: {}", gl.get_parameter_string(glow::RENDERER));

            let program = link_program(&gl)?;
            let vertex_array = gl.create_vertex_array().map_err(|e| anyhow!("creating a vertex array: {e}"))?;
            let plane = || -> Result<Plane> {
                let texture = gl.create_texture().map_err(|e| anyhow!("creating a texture: {e}"))?;
                gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                for (parameter, value) in [
                    (glow::TEXTURE_MIN_FILTER, glow::LINEAR),
                    (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
                    (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                    (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
                ] {
                    gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
                }
                Ok(Plane { texture, size: (0, 0) })
            };
            let planes = [plane()?, plane()?, plane()?];
            gl.use_program(Some(program));
            for (n, name) in ["plane_y", "plane_u", "plane_v"].into_iter().enumerate() {
                gl.uniform_1_i32(gl.get_uniform_location(program, name).as_ref(), n as i32);
            }
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);

            Ok(Self {
                gl,
                program,
                vertex_array,
                planes,
                semi_planar: true,
                conversion: None,
                picture: None,
                width: width.max(1),
                height: height.max(1),
                surface,
                context,
                _display: display,
            })
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        (self.width, self.height) = (width.max(1), height.max(1));
        self.surface.resize(&self.context, size(width), size(height));
    }

    /// Uploads a decoded picture (NV12 or I420); the next draw shows it.
    pub fn upload(&mut self, frame: &gst_video::VideoFrameRef<&gst::BufferRef>) {
        let info = frame.info();
        let semi_planar = info.format() == gst_video::VideoFormat::Nv12;
        let conversion = Conversion::new(info.colorimetry());
        let gl = &self.gl;
        // SAFETY: our context is current on this thread, and each plane's data
        // holds `stride * rows` bytes, as GStreamer maps it.
        unsafe {
            for n in 0..frame.n_planes() as usize {
                let (Ok(data), Some(&stride)) = (frame.plane_data(n as u32), frame.plane_stride().get(n)) else {
                    continue;
                };
                let rows = info.comp_height(n as u8) as i32;
                let (format, bytes_per_pixel) = if semi_planar && n == 1 { (glow::RG, 2) } else { (glow::RED, 1) };
                let internal = if format == glow::RG { glow::RG8 } else { glow::R8 };
                let width = if semi_planar && n == 1 { info.width().div_ceil(2) as i32 } else { info.comp_width(n as u8) as i32 };
                let plane = &mut self.planes[n];
                gl.active_texture(glow::TEXTURE0 + n as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(plane.texture));
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, stride / bytes_per_pixel);
                let pixels = glow::PixelUnpackData::Slice(Some(data));
                if plane.size == (width, rows) {
                    gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, width, rows, format, glow::UNSIGNED_BYTE, pixels);
                } else {
                    gl.tex_image_2d(glow::TEXTURE_2D, 0, internal as i32, width, rows, 0, format, glow::UNSIGNED_BYTE, pixels);
                    plane.size = (width, rows);
                }
            }
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
            if self.conversion != Some(conversion) || self.semi_planar != semi_planar {
                gl.use_program(Some(self.program));
                let uniform = |name| gl.get_uniform_location(self.program, name);
                gl.uniform_matrix_3_f32_slice(uniform("yuv_to_rgb").as_ref(), false, &conversion.matrix);
                gl.uniform_3_f32_slice(uniform("offset").as_ref(), &conversion.offset);
                gl.uniform_1_i32(uniform("semi_planar").as_ref(), i32::from(semi_planar));
                tracing::debug!("colours: {:?}", info.colorimetry());
            }
        }
        self.conversion = Some(conversion);
        self.semi_planar = semi_planar;
        self.picture = Some((info.width(), info.height()));
    }

    /// Forgets the picture: the next draw shows only the background.
    pub fn clear(&mut self) {
        self.picture = None;
    }

    pub fn draw(&mut self) -> Result<()> {
        let gl = &self.gl;
        // SAFETY: our context is current on this thread.
        unsafe {
            gl.viewport(0, 0, self.width as i32, self.height as i32);
            let [r, g, b, a] = BACKGROUND;
            gl.clear_color(r, g, b, a);
            gl.clear(glow::COLOR_BUFFER_BIT);
            if let Some(picture) = self.picture {
                let (x, y, w, h) = letterbox(picture, (self.width, self.height));
                gl.viewport(x, y, w, h);
                gl.use_program(Some(self.program));
                gl.bind_vertex_array(Some(self.vertex_array));
                for (n, plane) in self.planes.iter().enumerate() {
                    gl.active_texture(glow::TEXTURE0 + n as u32);
                    gl.bind_texture(glow::TEXTURE_2D, Some(plane.texture));
                }
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            }
        }
        self.surface.swap_buffers(&self.context).context("presenting")
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // SAFETY: the objects belong to our context, still current here.
        unsafe {
            for plane in &self.planes {
                self.gl.delete_texture(plane.texture);
            }
            self.gl.delete_vertex_array(self.vertex_array);
            self.gl.delete_program(self.program);
        }
    }
}

/// SAFETY: needs a current context.
unsafe fn link_program(gl: &glow::Context) -> Result<glow::Program> {
    unsafe {
        let program = gl.create_program().map_err(|e| anyhow!("creating the shader program: {e}"))?;
        let mut shaders = Vec::new();
        for (kind, source) in [(glow::VERTEX_SHADER, VERTEX_SHADER), (glow::FRAGMENT_SHADER, FRAGMENT_SHADER)] {
            let shader = gl.create_shader(kind).map_err(|e| anyhow!("creating a shader: {e}"))?;
            gl.shader_source(shader, source);
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                return Err(anyhow!("compiling a shader: {}", gl.get_shader_info_log(shader)));
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            return Err(anyhow!("linking the shader program: {}", gl.get_program_info_log(program)));
        }
        Ok(program)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gst_video::{VideoColorMatrix, VideoColorPrimaries, VideoColorRange, VideoColorimetry, VideoTransferFunction};

    fn rgb(conversion: Conversion, yuv: [f32; 3]) -> [f32; 3] {
        let m = conversion.matrix;
        let v = [yuv[0] - conversion.offset[0], yuv[1] - conversion.offset[1], yuv[2] - conversion.offset[2]];
        [0, 1, 2].map(|row| m[row] * v[0] + m[3 + row] * v[1] + m[6 + row] * v[2])
    }

    fn bt709(range: VideoColorRange) -> Conversion {
        Conversion::new(VideoColorimetry::new(
            range,
            VideoColorMatrix::Bt709,
            VideoTransferFunction::Bt709,
            VideoColorPrimaries::Bt709,
        ))
    }

    #[test]
    fn full_range_white_and_black() {
        let c = bt709(VideoColorRange::Range0_255);
        let mid = 128.0 / 255.0;
        for (out, want) in rgb(c, [1.0, mid, mid]).into_iter().zip([1.0; 3]) {
            assert!((out - want).abs() < 1e-3, "{out}");
        }
        assert!(rgb(c, [0.0, mid, mid]).iter().all(|v| v.abs() < 1e-3));
    }

    #[test]
    fn limited_range_stretches_to_full() {
        let c = bt709(VideoColorRange::Range16_235);
        let mid = 128.0 / 255.0;
        assert!(rgb(c, [235.0 / 255.0, mid, mid]).iter().all(|v| (v - 1.0).abs() < 1e-3));
        assert!(rgb(c, [16.0 / 255.0, mid, mid]).iter().all(|v| v.abs() < 1e-3));
    }

    #[test]
    fn bt709_red() {
        // Full-range BT.709 red: Y 0.2126, Cb -0.1146, Cr 0.5 (around 0.5).
        let c = bt709(VideoColorRange::Range0_255);
        let [r, g, b] = rgb(c, [0.2126, 0.5 - 0.1146 + (128.0 / 255.0 - 0.5), 1.0 + (128.0 / 255.0 - 0.5)]);
        assert!((r - 1.0).abs() < 2e-3 && g.abs() < 2e-3 && b.abs() < 2e-3, "{r} {g} {b}");
    }

    #[test]
    fn letterbox_portrait_in_landscape() {
        assert_eq!(letterbox((500, 1000), (1000, 500)), (375, 0, 250, 500));
        assert_eq!(letterbox((1920, 1080), (960, 540)), (0, 0, 960, 540));
    }
}
