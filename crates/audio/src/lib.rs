//! Plays the sender's audio on the default output device.
//!
//! Network audio arrives in bursts and its clock never quite matches the
//! sound card's. Samples are resampled to the device rate into a queue that
//! is kept around [`TARGET_LATENCY`]: when it fills up or drains, the
//! resampling ratio is nudged by a fraction of a percent (inaudible), and
//! after an underrun playback waits until the queue is full enough again.

mod resample;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SizedSample, StreamConfig};
use riflesso_airplay::AudioSink;

use resample::Resampler;

/// Queue length to aim for: enough to absorb network jitter.
const TARGET_LATENCY: Duration = Duration::from_millis(60);
/// Beyond this the queue is cut back to the target at once.
const MAX_LATENCY: Duration = Duration::from_millis(300);
/// Largest speed correction for clock drift (0.3%).
const MAX_ADJUST: f64 = 0.003;

pub struct Player {
    shared: Arc<Shared>,
    output: Mutex<Option<Output>>,
}

struct Output {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

struct Shared {
    state: Mutex<State>,
    /// `f32` bits of the linear gain.
    gain: AtomicU32,
}

#[derive(Default)]
struct State {
    input_rate: u32,
    input_channels: usize,
    output_rate: u32,
    output_channels: usize,
    resampler: Option<Resampler>,
    /// Interleaved samples at the output rate and channel count.
    queue: VecDeque<f32>,
    /// Smoothed queue length in frames, for drift correction.
    average_fill: f64,
    /// Waiting for the queue to reach the target before playing.
    buffering: bool,
    scratch: Vec<f32>,
}

impl State {
    fn target_frames(&self) -> f64 {
        f64::from(self.output_rate) * TARGET_LATENCY.as_secs_f64()
    }

    fn reset_resampler(&mut self) {
        self.queue.clear();
        self.buffering = true;
        self.average_fill = self.target_frames();
        self.resampler = (self.input_rate > 0 && self.output_rate > 0)
            .then(|| Resampler::new(self.input_rate, self.output_rate, self.input_channels));
    }
}

impl Default for Player {
    fn default() -> Self {
        Self::new()
    }
}

impl Player {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(Shared { state: Mutex::default(), gain: AtomicU32::new(1f32.to_bits()) }),
            output: Mutex::new(None),
        }
    }

    fn open_output(&self) {
        let mut output = self.output.lock().unwrap();
        if output.is_some() {
            return;
        }
        let (stop, stopped) = mpsc::channel();
        let shared = self.shared.clone();
        // cpal streams cannot move between threads: this one owns it.
        let thread = std::thread::Builder::new()
            .name("audio-output".into())
            .spawn(move || {
                let stream = match open_stream(&shared) {
                    Ok(stream) => stream,
                    Err(e) => {
                        tracing::error!("cannot open the audio output: {e}");
                        return;
                    }
                };
                // Play until stop() is called (or the player is dropped).
                let _ = stopped.recv();
                drop(stream);
            })
            .expect("spawning audio thread");
        *output = Some(Output { stop, thread });
    }
}

fn open_stream(shared: &Arc<Shared>) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
    let supported = device.default_output_config()?;
    let config: StreamConfig = supported.config();
    {
        let mut state = shared.state.lock().unwrap();
        state.output_rate = config.sample_rate;
        state.output_channels = usize::from(config.channels);
        state.reset_resampler();
    }
    tracing::info!(
        "audio output: {} Hz, {} channels, {:?}",
        config.sample_rate,
        config.channels,
        supported.sample_format()
    );
    let stream = match supported.sample_format() {
        SampleFormat::F32 => build::<f32>(&device, &config, shared.clone())?,
        SampleFormat::I16 => build::<i16>(&device, &config, shared.clone())?,
        SampleFormat::I32 => build::<i32>(&device, &config, shared.clone())?,
        other => return Err(format!("unsupported output sample format {other:?}").into()),
    };
    stream.play()?;
    Ok(stream)
}

fn build<T>(device: &cpal::Device, config: &StreamConfig, shared: Arc<Shared>) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample + cpal::FromSample<f32>,
{
    device.build_output_stream(
        *config,
        move |data: &mut [T], _| {
            let gain = f32::from_bits(shared.gain.load(Ordering::Relaxed));
            let mut state = shared.state.lock().unwrap();
            let channels = state.output_channels.max(1);
            if state.buffering && state.queue.len() / channels >= state.target_frames() as usize {
                state.buffering = false;
            }
            for sample in data.iter_mut() {
                let value = if state.buffering { None } else { state.queue.pop_front() };
                if value.is_none() && !state.buffering {
                    state.buffering = true; // underrun: refill before resuming
                }
                *sample = T::from_sample(value.unwrap_or(0.0) * gain);
            }
        },
        |e| tracing::warn!("audio output error: {e}"),
        None,
    )
}

impl AudioSink for Player {
    fn start(&self, sample_rate: u32, channels: u16) {
        {
            let mut state = self.shared.state.lock().unwrap();
            state.input_rate = sample_rate;
            state.input_channels = usize::from(channels);
            state.reset_resampler();
        }
        self.open_output();
    }

    fn samples(&self, interleaved: &[f32]) {
        let mut state = self.shared.state.lock().unwrap();
        let state = &mut *state;
        let (in_ch, out_ch) = (state.input_channels, state.output_channels);
        if in_ch == 0 || out_ch == 0 {
            return;
        }

        // Drift correction from the smoothed queue length.
        let fill = (state.queue.len() / out_ch) as f64;
        state.average_fill += (fill - state.average_fill) * 0.01;
        let target = state.target_frames();
        let adjust = ((state.average_fill - target) / target * 0.01).clamp(-MAX_ADJUST, MAX_ADJUST);

        let Some(resampler) = state.resampler.as_mut() else { return };
        state.scratch.clear();
        resampler.process(interleaved, adjust, &mut state.scratch);

        for frame in state.scratch.chunks_exact(in_ch) {
            for c in 0..out_ch {
                // Mono is copied to every channel; stereo fills front left/right.
                let sample = match in_ch {
                    1 => frame[0],
                    _ if c < in_ch => frame[c],
                    _ => 0.0,
                };
                state.queue.push_back(sample);
            }
        }

        let max = (f64::from(state.output_rate) * MAX_LATENCY.as_secs_f64()) as usize * out_ch;
        if state.queue.len() > max {
            let excess = state.queue.len() - target as usize * out_ch;
            state.queue.drain(..excess);
            state.average_fill = target;
            tracing::debug!("audio queue too long, dropped {} ms", excess / out_ch * 1000 / state.output_rate as usize);
        }
    }

    fn set_volume(&self, gain: f32) {
        self.shared.gain.store(gain.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    fn stop(&self) {
        if let Some(output) = self.output.lock().unwrap().take() {
            let _ = output.stop.send(());
            let _ = output.thread.join();
        }
        let mut state = self.shared.state.lock().unwrap();
        state.queue.clear();
        state.resampler = None;
    }
}
