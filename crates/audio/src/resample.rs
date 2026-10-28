//! Band-limited (windowed sinc) sample-rate conversion with an adjustable
//! ratio, so playback can drift along with the sender's clock.

use std::f64::consts::PI;
use std::sync::OnceLock;

/// Kernel half-width in zero crossings: 32 taps per output sample.
const ZERO_CROSSINGS: usize = 16;
/// Kernel table resolution per zero crossing (linearly interpolated).
const PHASES: usize = 256;
/// Passband edge as a fraction of the lower Nyquist frequency.
const CUTOFF: f64 = 0.95;

fn kernel_table() -> &'static [f32] {
    static TABLE: OnceLock<Vec<f32>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let len = ZERO_CROSSINGS * PHASES + 2;
        (0..len)
            .map(|i| {
                let x = i as f64 / PHASES as f64;
                if x >= ZERO_CROSSINGS as f64 {
                    return 0.0;
                }
                let sinc = if x == 0.0 { 1.0 } else { (PI * x).sin() / (PI * x) };
                // Blackman-Harris window over [-ZERO_CROSSINGS, ZERO_CROSSINGS].
                let w = PI * (x / ZERO_CROSSINGS as f64 + 1.0);
                let window = 0.35875 - 0.48829 * w.cos() + 0.14128 * (2.0 * w).cos() - 0.01168 * (3.0 * w).cos();
                (sinc * window) as f32
            })
            .collect()
    })
}

pub struct Resampler {
    channels: usize,
    /// Input frames advanced per output frame.
    ratio: f64,
    /// Kernel scale: below 1 when downsampling, to filter out aliases.
    scale: f64,
    /// Kernel half-width in input frames.
    half_width: usize,
    /// Pending interleaved input, starting `half_width` frames before `pos`.
    input: Vec<f32>,
    /// Position of the next output frame within `input`, in frames.
    pos: f64,
}

impl Resampler {
    pub fn new(input_rate: u32, output_rate: u32, channels: usize) -> Self {
        let ratio = f64::from(input_rate) / f64::from(output_rate);
        let scale = CUTOFF * ratio.recip().min(1.0);
        let half_width = (ZERO_CROSSINGS as f64 / scale).ceil() as usize;
        Self {
            channels,
            ratio,
            scale,
            half_width,
            // Silence as history, so output starts right away.
            input: vec![0.0; half_width * channels],
            pos: half_width as f64,
        }
    }

    /// Converts interleaved `input`, appending to `out`. `adjust` slightly
    /// speeds up (> 0) or slows down (< 0) consumption of the input.
    pub fn process(&mut self, input: &[f32], adjust: f64, out: &mut Vec<f32>) {
        let table = kernel_table();
        let ch = self.channels;
        self.input.extend_from_slice(input);
        let frames = self.input.len() / ch;
        let step = self.ratio * (1.0 + adjust);
        let mut acc = vec![0f32; ch];

        while (self.pos.floor() as usize) + self.half_width < frames {
            let center = self.pos.floor() as usize;
            let first = center + 1 - self.half_width;
            acc.iter_mut().for_each(|a| *a = 0.0);
            let mut weight_sum = 0f32;
            for k in first..=center + self.half_width {
                let d = ((self.pos - k as f64) * self.scale).abs() * PHASES as f64;
                let i = d as usize;
                if i + 1 >= table.len() {
                    continue;
                }
                let frac = (d - i as f64) as f32;
                let w = table[i] + (table[i + 1] - table[i]) * frac;
                weight_sum += w;
                let frame = &self.input[k * ch..(k + 1) * ch];
                for (a, s) in acc.iter_mut().zip(frame) {
                    *a += w * s;
                }
            }
            // Normalising keeps DC gain exactly 1 despite the truncated kernel.
            let norm = if weight_sum.abs() > 1e-6 { weight_sum.recip() } else { 0.0 };
            out.extend(acc.iter().map(|a| a * norm));
            self.pos += step;
        }

        let consumed = (self.pos.floor() as usize + 1).saturating_sub(self.half_width).min(frames);
        self.input.drain(..consumed * ch);
        self.pos -= consumed as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: f64, freq: f64, n: usize) -> Vec<f32> {
        (0..n).map(|i| (2.0 * PI * freq * i as f64 / rate).sin() as f32).collect()
    }

    #[test]
    fn upsamples_sine_accurately() {
        let input = sine(44_100.0, 1_000.0, 44_100);
        let mut r = Resampler::new(44_100, 48_000, 1);
        let mut out = Vec::new();
        // Feed in packet-sized pieces, as the network does.
        for chunk in input.chunks(480) {
            r.process(chunk, 0.0, &mut out);
        }
        assert!((out.len() as i64 - 48_000).abs() < 40, "got {} samples", out.len());

        let expected = sine(48_000.0, 1_000.0, out.len());
        let max_error = out[100..out.len() - 100]
            .iter()
            .zip(&expected[100..])
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        assert!(max_error < 1e-3, "max error {max_error}");
    }

    #[test]
    fn adjust_changes_output_length() {
        let input = vec![0.5f32; 2 * 44_100];
        let (mut normal, mut faster) = (Vec::new(), Vec::new());
        Resampler::new(44_100, 48_000, 2).process(&input, 0.0, &mut normal);
        Resampler::new(44_100, 48_000, 2).process(&input, 0.002, &mut faster);
        assert!(faster.len() < normal.len());
        // Constant input stays constant (unity DC gain).
        assert!(normal[200..].iter().all(|s| (s - 0.5).abs() < 1e-4));
    }
}
