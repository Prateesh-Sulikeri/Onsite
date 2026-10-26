//! Speaker and microphone via cpal, plus the pure DSP bits (resampling, VAD) they need.

use crate::error::{AppError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

/// Linear resampler that keeps its phase across chunks.
#[derive(Debug, Clone)]
pub struct Resampler {
    step: f64,
    pos: f64,
    prev: f32,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self { step: from as f64 / to as f64, pos: 0.0, prev: 0.0 }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if (self.step - 1.0).abs() < f64::EPSILON {
            return input.to_vec();
        }
        let Some(&last) = input.last() else { return Vec::new() };
        // Virtual signal: index -1 is the previous chunk's last sample, 0.. is `input`.
        let prev = self.prev;
        let at = |k: isize| if k < 0 { prev } else { input[k as usize] };
        let n = input.len() as f64;
        let mut out = Vec::with_capacity((n / self.step) as usize + 2);
        while self.pos < n - 1.0 {
            let k = self.pos.floor() as isize;
            let (a, b) = (at(k), at(k + 1));
            out.push(a + (b - a) * (self.pos - k as f64) as f32);
            self.pos += self.step;
        }
        self.pos -= n;
        self.prev = last;
        out
    }
}

pub fn i16_to_f32(s: &[i16]) -> Vec<f32> {
    s.iter().map(|&v| v as f32 / 32768.0).collect()
}

pub fn f32_to_i16(s: &[f32]) -> Vec<i16> {
    s.iter().map(|&v| (v.clamp(-1.0, 1.0) * 32767.0) as i16).collect()
}

pub fn rms(s: &[f32]) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VadEvent {
    None,
    SpeechStart,
    SpeechEnd,
}

/// Energy VAD over fixed frames with an adaptive noise floor.
#[derive(Debug, Clone)]
pub struct Vad {
    frame_ms: u32,
    pause_ms: u32,
    noise: f32,
    loud_frames: u32,
    quiet_ms: u32,
    speaking: bool,
}

impl Vad {
    const MIN_THRESHOLD: f32 = 0.012;
    const START_FRAMES: u32 = 3;

    /// `pause_ms == 0` never ends on silence (manual mode).
    pub fn new(frame_ms: u32, pause_ms: u32) -> Self {
        Self { frame_ms, pause_ms, noise: 0.004, loud_frames: 0, quiet_ms: 0, speaking: false }
    }

    pub fn threshold(&self) -> f32 {
        (self.noise * 3.0).max(Self::MIN_THRESHOLD)
    }

    pub fn push(&mut self, frame_rms: f32) -> VadEvent {
        let loud = frame_rms > self.threshold();
        if !loud {
            // Only learn the floor from quiet frames, slowly.
            self.noise = self.noise * 0.95 + frame_rms * 0.05;
        }
        if !self.speaking {
            self.loud_frames = if loud { self.loud_frames + 1 } else { 0 };
            if self.loud_frames >= Self::START_FRAMES {
                self.speaking = true;
                self.quiet_ms = 0;
                return VadEvent::SpeechStart;
            }
            return VadEvent::None;
        }
        self.quiet_ms = if loud { 0 } else { self.quiet_ms + self.frame_ms };
        if self.pause_ms > 0 && self.quiet_ms >= self.pause_ms {
            self.speaking = false;
            self.loud_frames = 0;
            return VadEvent::SpeechEnd;
        }
        VadEvent::None
    }
}

/// f32 bits in an atomic, for meters read by the UI ticker.
#[derive(Default)]
pub struct Level(AtomicU32);

impl Level {
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed)
    }
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

fn err(e: impl std::fmt::Display) -> AppError {
    AppError::Voice(format!("audio device: {e}"))
}

/// Default output device, fed from a sample queue at the device rate.
pub struct Player {
    _stream: Mutex<cpal::Stream>,
    queue: Arc<Mutex<VecDeque<f32>>>,
    rate: u32,
    pub level: Arc<Level>,
}

impl Player {
    pub fn open() -> Result<Self> {
        let device = cpal::default_host().default_output_device().ok_or_else(|| err("no speaker found"))?;
        let supported = device.default_output_config().map_err(err)?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let queue: Arc<Mutex<VecDeque<f32>>> = Default::default();
        let level: Arc<Level> = Default::default();
        let (q, lv) = (queue.clone(), level.clone());
        let fill = move |out: &mut dyn FnMut(usize, f32), frames: usize| {
            let mut q = q.lock().unwrap();
            let mut sum = 0.0;
            for f in 0..frames {
                let v = q.pop_front().unwrap_or(0.0);
                sum += v * v;
                out(f, v);
            }
            lv.set(if frames > 0 { (sum / frames as f32).sqrt() } else { 0.0 });
        };
        let on_err = |e: cpal::Error| tracing::warn!("speaker stream: {e}");
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let frames = data.len() / channels;
                    fill(&mut |f, v| data[f * channels..(f + 1) * channels].fill(v), frames)
                },
                on_err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let frames = data.len() / channels;
                    fill(&mut |f, v| data[f * channels..(f + 1) * channels].fill((v * 32767.0) as i16), frames)
                },
                on_err,
                None,
            ),
            other => return Err(err(format!("unsupported speaker format {other}"))),
        }
        .map_err(err)?;
        stream.play().map_err(err)?;
        tracing::info!(rate = config.sample_rate, channels, "speaker open");
        Ok(Self { _stream: Mutex::new(stream), queue, rate: config.sample_rate, level })
    }

    /// Queues mono PCM recorded at `sr`.
    pub fn push(&self, resampler: &mut Resampler, pcm: &[i16]) {
        let out = resampler.process(&i16_to_f32(pcm));
        self.queue.lock().unwrap().extend(out);
    }

    pub fn resampler_from(&self, sr: u32) -> Resampler {
        Resampler::new(sr, self.rate)
    }

    pub fn clear(&self) {
        self.queue.lock().unwrap().clear();
    }

    pub fn is_idle(&self) -> bool {
        self.queue.lock().unwrap().is_empty()
    }
}

pub const MIC_RATE: u32 = 16_000;

/// An open microphone. Sends 16 kHz mono i16 chunks until dropped.
pub struct Mic {
    _stream: cpal::Stream,
    pub level: Arc<Level>,
}

impl Mic {
    pub fn open(tx: UnboundedSender<Vec<i16>>) -> Result<Self> {
        let device = cpal::default_host().default_input_device().ok_or_else(|| err("no microphone found"))?;
        let supported = device.default_input_config().map_err(err)?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let mut rs = Resampler::new(config.sample_rate, MIC_RATE);
        let level: Arc<Level> = Default::default();
        let lv = level.clone();
        let mut handle = move |mono: Vec<f32>| {
            lv.set(rms(&mono));
            let _ = tx.send(f32_to_i16(&rs.process(&mono)));
        };
        let on_err = |e: cpal::Error| tracing::warn!("mic stream: {e}");
        let stream = match format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    handle(data.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect())
                },
                on_err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_input_stream(
                config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    handle(
                        data.chunks(channels)
                            .map(|f| f.iter().map(|&v| v as f32 / 32768.0).sum::<f32>() / channels as f32)
                            .collect(),
                    )
                },
                on_err,
                None,
            ),
            other => return Err(err(format!("unsupported mic format {other}"))),
        }
        .map_err(err)?;
        stream.play().map_err(err)?;
        tracing::info!(rate = config.sample_rate, channels, "mic open");
        Ok(Self { _stream: stream, level })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_length_and_dc() {
        let mut r = Resampler::new(48_000, 16_000);
        let mut total = 0;
        for _ in 0..10 {
            let out = r.process(&[0.5; 480]);
            assert!(out.iter().all(|v| (v - 0.5).abs() < 1e-6));
            total += out.len();
        }
        assert!((1599..=1601).contains(&total), "got {total}");
        let mut up = Resampler::new(24_000, 48_000);
        let n: usize = (0..10).map(|_| up.process(&[0.0; 240]).len()).sum();
        assert!((4798..=4800).contains(&n), "got {n}");
    }

    #[test]
    fn resample_is_continuous_across_chunks() {
        let ramp: Vec<f32> = (0..300).map(|i| i as f32 / 300.0).collect();
        let mut one = Resampler::new(44_100, 16_000);
        let whole = one.process(&ramp);
        let mut two = Resampler::new(44_100, 16_000);
        let mut parts = two.process(&ramp[..137]);
        parts.extend(two.process(&ramp[137..]));
        assert_eq!(whole.len(), parts.len());
        assert!(whole.iter().zip(&parts).all(|(a, b)| (a - b).abs() < 1e-5));
        assert!(parts.windows(2).all(|w| w[1] >= w[0] - 1e-6), "not monotonic");
    }

    #[test]
    fn vad_start_and_end() {
        let mut v = Vad::new(20, 100);
        for _ in 0..20 {
            assert_eq!(v.push(0.002), VadEvent::None);
        }
        assert_eq!(v.push(0.2), VadEvent::None);
        assert_eq!(v.push(0.2), VadEvent::None);
        assert_eq!(v.push(0.2), VadEvent::SpeechStart);
        for _ in 0..4 {
            assert_eq!(v.push(0.001), VadEvent::None);
        }
        assert_eq!(v.push(0.2), VadEvent::None, "short gap keeps speaking");
        let ends: Vec<_> = (0..5).map(|_| v.push(0.001)).collect();
        assert_eq!(ends.last(), Some(&VadEvent::SpeechEnd));
    }

    #[test]
    fn vad_manual_mode_never_ends() {
        let mut v = Vad::new(20, 0);
        (0..3).for_each(|_| {
            v.push(0.3);
        });
        assert!((0..500).all(|_| v.push(0.0) == VadEvent::None));
    }
}
