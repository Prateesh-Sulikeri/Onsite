//! Speaker and microphone via cpal, plus the pure DSP bits (resampling, VAD) they need.

use crate::error::{AppError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

/// Windowed-sinc (Blackman) FIR low-pass. Keeps its history across chunks.
#[derive(Debug, Clone)]
pub struct LowPass {
    taps: Vec<f32>,
    /// Last `taps.len() - 1` input samples, followed by the current chunk while filtering.
    buf: Vec<f32>,
}

impl LowPass {
    /// `cutoff_hz` is the -6 dB point; `transition_hz` is the full pass-to-stop width.
    pub fn new(rate: u32, cutoff_hz: f32, transition_hz: f32) -> Self {
        // Blackman main lobe is about 5.5 * rate / N wide.
        let n = ((5.5 * rate as f32 / transition_hz).ceil() as usize).clamp(15, 511) | 1;
        let fc = cutoff_hz / rate as f32;
        let mid = (n / 2) as f32;
        let mut taps: Vec<f32> = (0..n)
            .map(|i| {
                let x = i as f32 - mid;
                let sinc = if x == 0.0 { 2.0 * fc } else { (2.0 * std::f32::consts::PI * fc * x).sin() / (std::f32::consts::PI * x) };
                let w = 2.0 * std::f32::consts::PI * i as f32 / (n - 1) as f32;
                sinc * (0.42 - 0.5 * w.cos() + 0.08 * (2.0 * w).cos())
            })
            .collect();
        let sum: f32 = taps.iter().sum();
        taps.iter_mut().for_each(|t| *t /= sum);
        Self { buf: vec![0.0; n - 1], taps }
    }

    pub fn len(&self) -> usize {
        self.taps.len()
    }

    pub fn is_empty(&self) -> bool {
        self.taps.is_empty()
    }

    /// Filters `io` in place (delay of `len() / 2` samples).
    pub fn process(&mut self, io: &mut [f32]) {
        let keep = self.taps.len() - 1;
        self.buf.extend_from_slice(io);
        for (i, out) in io.iter_mut().enumerate() {
            let win = &self.buf[i..i + self.taps.len()];
            *out = win.iter().zip(&self.taps).map(|(a, b)| a * b).sum();
        }
        let drop = self.buf.len() - keep;
        self.buf.drain(..drop);
    }
}

/// Anti-aliased rate conversion: low-pass below the target Nyquist, then interpolate.
/// Without the filter, 48 kHz -> 16 kHz folds 8-24 kHz noise and sibilants into the speech band.
#[derive(Debug, Clone)]
pub struct Downsampler {
    lp: Option<LowPass>,
    rs: Resampler,
}

impl Downsampler {
    pub fn new(from: u32, to: u32) -> Self {
        let lp = (from > to).then(|| {
            let nyq = to as f32 / 2.0;
            // Pass to ~0.75 * Nyquist, stop by ~1.1 * Nyquist, so nothing folds below ~6 kHz at 16 kHz out.
            LowPass::new(from, nyq * 0.925, nyq * 0.35)
        });
        Self { lp, rs: Resampler::new(from, to) }
    }

    pub fn process(&mut self, input: &mut [f32]) -> Vec<f32> {
        if let Some(lp) = self.lp.as_mut() {
            lp.process(input);
        }
        self.rs.process(input)
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

/// Energy VAD over fixed frames with a noise floor learned before speech.
///
/// The floor only learns while nobody is speaking. Learning during speech let soft syllables
/// raise it until normal speech read as silence, which ended answers mid-sentence.
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
    /// A noisy room can't push the start threshold above 3x this.
    const MAX_NOISE: f32 = 0.02;
    /// While speaking, frames above this keep the turn open (quiet word endings).
    const MIN_HOLD: f32 = 0.008;
    const START_FRAMES: u32 = 3;

    /// `pause_ms == 0` never ends on silence (manual mode).
    pub fn new(frame_ms: u32, pause_ms: u32) -> Self {
        Self { frame_ms, pause_ms, noise: 0.004, loud_frames: 0, quiet_ms: 0, speaking: false }
    }

    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    /// Milliseconds of quiet since the last loud frame while speaking.
    pub fn quiet_ms(&self) -> u32 {
        self.quiet_ms
    }

    /// Level that starts speech.
    pub fn threshold(&self) -> f32 {
        (self.noise * 3.0).max(Self::MIN_THRESHOLD)
    }

    /// Level that keeps speech going once started.
    fn hold_threshold(&self) -> f32 {
        (self.noise * 2.0).max(Self::MIN_HOLD)
    }

    pub fn push(&mut self, frame_rms: f32) -> VadEvent {
        if !self.speaking {
            let loud = frame_rms > self.threshold();
            if !loud {
                self.noise = (self.noise * 0.95 + frame_rms * 0.05).min(Self::MAX_NOISE);
            }
            self.loud_frames = if loud { self.loud_frames + 1 } else { 0 };
            if self.loud_frames >= Self::START_FRAMES {
                self.speaking = true;
                self.quiet_ms = 0;
                return VadEvent::SpeechStart;
            }
            return VadEvent::None;
        }
        let voiced = frame_rms > self.hold_threshold();
        self.quiet_ms = if voiced { 0 } else { self.quiet_ms + self.frame_ms };
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

/// Runs `f` on the one long-lived audio thread. Opening WASAPI devices from tokio workers failed
/// after a device change ("Cannot change thread mode after it is set"): another library had put
/// that worker's COM apartment in a different mode. This thread only ever touches cpal.
fn on_audio_thread<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> R {
    type Job = Box<dyn FnOnce() + Send>;
    static JOBS: std::sync::OnceLock<Mutex<std::sync::mpsc::Sender<Job>>> = std::sync::OnceLock::new();
    let jobs = JOBS.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("onsite-audio".into())
            .spawn(move || rx.into_iter().for_each(|job| job()))
            .expect("audio thread");
        Mutex::new(tx)
    });
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    jobs.lock().unwrap().send(Box::new(move || drop(tx.send(f())))).expect("audio thread alive");
    rx.recv().expect("audio job finished")
}

/// Chosen devices as cpal device ids; empty = the Windows default. Read whenever a stream opens.
static INPUT: Mutex<String> = Mutex::new(String::new());
static OUTPUT: Mutex<String> = Mutex::new(String::new());

/// Sets the mic and speaker for the next stream that opens. The speaker switches on the next reply.
pub fn set_devices(input: &str, output: &str) {
    *INPUT.lock().unwrap() = input.trim().to_string();
    *OUTPUT.lock().unwrap() = output.trim().to_string();
}

#[derive(Debug, Clone, Serialize)]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct AudioDevices {
    pub inputs: Vec<AudioDevice>,
    pub outputs: Vec<AudioDevice>,
}

/// Microphones and speakers Windows offers right now.
pub fn list_devices() -> Result<AudioDevices> {
    on_audio_thread(|| {
        let host = cpal::default_host();
        let id = |d: &cpal::Device| d.id().map(|i| i.to_string()).unwrap_or_default();
        let collect = |devs: Vec<cpal::Device>, default: Option<cpal::Device>| {
            let def = default.as_ref().map(id);
            devs.iter()
                .map(|d| AudioDevice { id: id(d), name: d.to_string(), default: def.as_deref() == Some(id(d).as_str()) })
                .filter(|d| !d.id.is_empty())
                .collect()
        };
        Ok(AudioDevices {
            inputs: collect(host.input_devices().map_err(err)?.collect(), host.default_input_device()),
            outputs: collect(host.output_devices().map_err(err)?.collect(), host.default_output_device()),
        })
    })
}

/// The chosen device, else the default. A chosen device that is gone (unplugged) falls back
/// to the default rather than leaving the round silent.
fn pick_device(host: &cpal::Host, chosen: &str, input: bool) -> Option<cpal::Device> {
    if !chosen.is_empty() {
        let found = chosen.parse::<cpal::DeviceId>().ok().and_then(|id| host.device_by_id(&id));
        match found {
            Some(d) => return Some(d),
            None => tracing::warn!(chosen, input, "chosen audio device not found; using the default"),
        }
    }
    if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    }
}

/// True for stream errors after which the stream never recovers (device unplugged or switched).
fn fatal(e: &cpal::Error) -> bool {
    matches!(e.kind(), cpal::ErrorKind::StreamInvalidated | cpal::ErrorKind::DeviceNotAvailable)
}

/// Default output device, fed from a sample queue at the device rate. Rebuilds its stream
/// when Windows invalidates it (headphones plugged in, Bluetooth connecting).
pub struct Player {
    stream: Mutex<Option<cpal::Stream>>,
    queue: Arc<Mutex<VecDeque<f32>>>,
    rate: AtomicU32,
    dead: Arc<AtomicBool>,
    /// The `OUTPUT` choice the stream was built for.
    built_for: Mutex<String>,
    pub level: Arc<Level>,
}

impl Player {
    pub fn open() -> Result<Self> {
        let p = Self {
            stream: Mutex::new(None),
            queue: Default::default(),
            rate: AtomicU32::new(0),
            dead: Arc::new(AtomicBool::new(true)),
            built_for: Default::default(),
            level: Default::default(),
        };
        p.ensure()?;
        Ok(p)
    }

    /// Opens the stream if it is missing, was invalidated, or another speaker was chosen.
    fn ensure(&self) -> Result<()> {
        let want = OUTPUT.lock().unwrap().clone();
        if !self.dead.load(Ordering::SeqCst) && *self.built_for.lock().unwrap() == want {
            return Ok(());
        }
        let (queue, level, dead) = (self.queue.clone(), self.level.clone(), self.dead.clone());
        let chosen = want.clone();
        let (stream, rate) = on_audio_thread(move || build_output(&chosen, queue, level, dead))?;
        *self.built_for.lock().unwrap() = want;
        // Samples queued for the old device were resampled for its rate.
        self.queue.lock().unwrap().clear();
        self.rate.store(rate, Ordering::SeqCst);
        *self.stream.lock().unwrap() = Some(stream);
        self.dead.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Queues mono PCM recorded at `sr`.
    pub fn push(&self, resampler: &mut Resampler, pcm: &[i16]) {
        let out = resampler.process(&i16_to_f32(pcm));
        self.queue.lock().unwrap().extend(out);
    }

    /// A resampler to the device rate. Reopens a dead stream first, so each reply after a
    /// device change plays on the new device.
    pub fn resampler_from(&self, sr: u32) -> Resampler {
        if let Err(e) = self.ensure() {
            tracing::warn!("speaker reopen: {e}");
        }
        Resampler::new(sr, self.rate.load(Ordering::SeqCst).max(1))
    }

    pub fn clear(&self) {
        self.queue.lock().unwrap().clear();
    }

    /// Idle when nothing is queued, or when the stream died (the queue would never drain).
    pub fn is_idle(&self) -> bool {
        self.dead.load(Ordering::SeqCst) || self.queue.lock().unwrap().is_empty()
    }
}

fn build_output(chosen: &str, queue: Arc<Mutex<VecDeque<f32>>>, lv: Arc<Level>, dead: Arc<AtomicBool>) -> Result<(cpal::Stream, u32)> {
    let device = pick_device(&cpal::default_host(), chosen, false).ok_or_else(|| err("no speaker found"))?;
    let supported = device.default_output_config().map_err(err)?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;
    let fill = move |out: &mut dyn FnMut(usize, f32), frames: usize| {
        let mut q = queue.lock().unwrap();
        let mut sum = 0.0;
        for f in 0..frames {
            let v = q.pop_front().unwrap_or(0.0);
            sum += v * v;
            out(f, v);
        }
        lv.set(if frames > 0 { (sum / frames as f32).sqrt() } else { 0.0 });
    };
    let on_err = move |e: cpal::Error| {
        tracing::warn!("speaker stream: {e}");
        if fatal(&e) {
            dead.store(true, Ordering::SeqCst);
        }
    };
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
    tracing::info!(rate = config.sample_rate, channels, device = %device, "speaker open");
    Ok((stream, config.sample_rate))
}

pub const MIC_RATE: u32 = 16_000;

/// An open microphone. Sends 16 kHz mono i16 chunks until dropped.
pub struct Mic {
    _stream: cpal::Stream,
    pub level: Arc<Level>,
    dead: Arc<AtomicBool>,
}

impl Mic {
    pub fn open(tx: UnboundedSender<Vec<i16>>) -> Result<Self> {
        on_audio_thread(move || Self::build(tx))
    }

    /// True once the device was unplugged or switched; no more audio will come.
    pub fn is_dead(&self) -> bool {
        self.dead.load(Ordering::SeqCst)
    }

    fn build(tx: UnboundedSender<Vec<i16>>) -> Result<Self> {
        let chosen = INPUT.lock().unwrap().clone();
        let device = pick_device(&cpal::default_host(), &chosen, true).ok_or_else(|| err("no microphone found"))?;
        let supported = device.default_input_config().map_err(err)?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let channels = config.channels as usize;
        let mut rs = Downsampler::new(config.sample_rate, MIC_RATE);
        let level: Arc<Level> = Default::default();
        let dead: Arc<AtomicBool> = Default::default();
        let (lv, d) = (level.clone(), dead.clone());
        let mut handle = move |mut mono: Vec<f32>| {
            lv.set(rms(&mono));
            let _ = tx.send(f32_to_i16(&rs.process(&mut mono)));
        };
        let on_err = move |e: cpal::Error| {
            tracing::warn!("mic stream: {e}");
            if fatal(&e) {
                d.store(true, Ordering::SeqCst);
            }
        };
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
        tracing::info!(rate = config.sample_rate, channels, device = %device, "mic open");
        Ok(Self { _stream: stream, level, dead })
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

    fn tone(rate: u32, hz: f32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n).map(|i| 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin()).collect()
    }

    /// RMS of a 48 kHz tone after conversion to 16 kHz in 10 ms chunks, skipping the warm-up.
    fn rms_after(hz: f32, filtered: bool) -> f32 {
        let input = tone(48_000, hz, 1.0);
        let mut ds = Downsampler::new(48_000, 16_000);
        let mut plain = Resampler::new(48_000, 16_000);
        let mut out = Vec::new();
        for c in input.chunks(480) {
            let mut c = c.to_vec();
            out.extend(if filtered { ds.process(&mut c) } else { plain.process(&c) });
        }
        rms(&out[1600..])
    }

    #[test]
    fn downsampler_passes_speech_and_blocks_aliases() {
        let full = 0.5 / 2f32.sqrt();
        for hz in [300.0, 1000.0, 3000.0, 5000.0] {
            let r = rms_after(hz, true);
            assert!((r / full - 1.0).abs() < 0.05, "{hz} Hz passband: {r}");
        }
        for hz in [9_000.0, 12_000.0, 15_000.0, 20_000.0] {
            let r = rms_after(hz, true);
            assert!(r < full * 0.01, "{hz} Hz should be < -40 dB, got {r}");
        }
        // Without the filter a 12 kHz tone folds to 4 kHz at a large fraction of its level.
        assert!(rms_after(12_000.0, false) > full * 0.2);
    }

    #[test]
    fn lowpass_is_continuous_across_chunks() {
        let x = tone(48_000, 2500.0, 0.05);
        let mut a = LowPass::new(48_000, 7_400.0, 2_800.0);
        let mut whole = x.clone();
        a.process(&mut whole);
        let mut b = LowPass::new(48_000, 7_400.0, 2_800.0);
        let (mut p1, mut p2) = (x[..333].to_vec(), x[333..].to_vec());
        b.process(&mut p1);
        b.process(&mut p2);
        p1.extend(p2);
        assert!(whole.iter().zip(&p1).all(|(u, v)| (u - v).abs() < 1e-6));
        assert!(a.len() % 2 == 1 && a.len() < 200, "taps {}", a.len());
    }

    #[test]
    fn downsampler_is_identity_at_same_rate() {
        let mut d = Downsampler::new(16_000, 16_000);
        let mut x = vec![0.25; 160];
        assert_eq!(d.process(&mut x), vec![0.25; 160]);
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
    fn vad_floor_does_not_climb_during_speech() {
        // Speech with soft syllables between loud ones, for 30 s. The old VAD learned the
        // soft frames as noise, its threshold rose past the speech, and it ended the turn.
        let mut v = Vad::new(20, 1800);
        (0..50).for_each(|_| {
            v.push(0.003);
        });
        let levels = [0.01, 0.02, 0.04, 0.08, 0.12, 0.06, 0.03, 0.015];
        let events: Vec<_> = (0..1500).map(|i| v.push(levels[i % levels.len()])).collect();
        assert_eq!(events.iter().filter(|e| **e == VadEvent::SpeechStart).count(), 1);
        assert!(!events.contains(&VadEvent::SpeechEnd));
        assert!(v.threshold() <= 0.015, "threshold {}", v.threshold());
    }

    #[test]
    fn vad_soft_word_endings_keep_turn_open() {
        let mut v = Vad::new(20, 200);
        (0..50).for_each(|_| {
            v.push(0.003);
        });
        (0..5).for_each(|_| {
            v.push(0.1);
        });
        // Trailing off: below the start threshold but well above the room.
        assert!((0..50).all(|_| v.push(0.01) == VadEvent::None));
        let ends: Vec<_> = (0..10).map(|_| v.push(0.003)).collect();
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
