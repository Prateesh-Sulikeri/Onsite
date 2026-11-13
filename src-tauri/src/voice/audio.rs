//! Speaker and microphone via cpal, plus the pure DSP bits (resampling, VAD) they need.

use crate::error::{AppError, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

/// Speaker-path rate conversion: sherpa-onnx's filtered resampler (low-pass, then interpolate), so 44.1 kHz
/// speech reaches a 48 kHz device without the roughness plain interpolation adds. Same rate = untouched.
/// Falls back to [`Linear`] (with a warning, once) if sherpa can't make one.
pub struct Resampler(Conv);

enum Conv {
    Same,
    Filtered(sherpa_onnx::LinearResampler),
    Linear(Linear),
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        static WARNED: AtomicBool = AtomicBool::new(false);
        if from == to {
            return Self(Conv::Same);
        }
        match sherpa_onnx::LinearResampler::create(from as i32, to as i32) {
            Some(r) => Self(Conv::Filtered(r)),
            None => {
                if !WARNED.swap(true, Ordering::SeqCst) {
                    tracing::warn!(from, to, "filtered resampler unavailable; using linear");
                }
                Self(Conv::Linear(Linear::new(from, to)))
            }
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        match &mut self.0 {
            Conv::Same => input.to_vec(),
            Conv::Filtered(r) => r.resample(input, false),
            Conv::Linear(l) => l.process(input),
        }
    }

    /// The samples still held inside the filter. Call once after the last chunk of a reply.
    pub fn flush(&mut self) -> Vec<f32> {
        match &mut self.0 {
            Conv::Filtered(r) => r.resample(&[], true),
            Conv::Same | Conv::Linear(_) => Vec::new(),
        }
    }
}

/// Linear resampler that keeps its phase across chunks. The mic path uses it after its own low-pass.
#[derive(Debug, Clone)]
struct Linear {
    step: f64,
    pos: f64,
    prev: f32,
}

impl Linear {
    fn new(from: u32, to: u32) -> Self {
        Self { step: from as f64 / to as f64, pos: 0.0, prev: 0.0 }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
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

    /// Filters `io` in place (delay of half the tap count).
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
    rs: Linear,
}

impl Downsampler {
    pub fn new(from: u32, to: u32) -> Self {
        let lp = (from > to).then(|| {
            let nyq = to as f32 / 2.0;
            // Pass to ~0.75 * Nyquist, stop by ~1.1 * Nyquist, so nothing folds below ~6 kHz at 16 kHz out.
            LowPass::new(from, nyq * 0.925, nyq * 0.35)
        });
        Self { lp, rs: Linear::new(from, to) }
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

/// A queue the jitter buffer can read: the device ring in the app, a `VecDeque` in tests.
pub trait Source {
    fn len(&self) -> usize;
    fn pop(&mut self) -> Option<f32>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Source for VecDeque<f32> {
    fn len(&self) -> usize {
        VecDeque::len(self)
    }
    fn pop(&mut self) -> Option<f32> {
        self.pop_front()
    }
}

impl Source for rtrb::Consumer<f32> {
    fn len(&self) -> usize {
        self.slots()
    }
    fn pop(&mut self) -> Option<f32> {
        rtrb::Consumer::pop(self).ok()
    }
}

/// Playback smoothing for one reply. While the reply is still being generated, the device waits until
/// `prebuffer` samples are queued, and waits again after any dry spell. Without this, each short part
/// plays out before the next one is generated, and the candidate hears a gap after almost every sentence.
#[derive(Default)]
pub struct Jitter {
    /// Samples to queue before (re)starting (from the tier's prebuffer, read each callback).
    pub prebuffer: usize,
    rebuffering: bool,
    /// Dry spells not yet reported to [`Shared::underruns`].
    pub underruns: u32,
}

impl Jitter {
    /// The next sample for the device, or silence while waiting.
    pub fn next(&mut self, q: &mut impl Source, streaming: bool) -> f32 {
        if self.rebuffering {
            // Once the reply is complete, play whatever is left.
            if q.len() < self.prebuffer && (streaming || q.is_empty()) {
                return 0.0;
            }
            self.rebuffering = false;
        }
        match q.pop() {
            Some(v) => v,
            None => {
                if streaming {
                    self.rebuffering = true;
                    self.underruns += 1;
                }
                0.0
            }
        }
    }
}

/// Playback state shared by the producer side and the device callback. Atomics only: the callback never locks.
struct Shared {
    /// A reply is still being generated: more samples will come.
    streaming: AtomicBool,
    /// Drop everything queued (set by `clear`, done by the callback).
    flush: AtomicBool,
    /// Wait for the prebuffer again before playing (a new reply started).
    rebuffer: AtomicBool,
    /// Samples written and not yet played.
    queued: AtomicUsize,
    prebuffer_ms: AtomicU32,
    underruns: AtomicU32,
}

impl Default for Shared {
    fn default() -> Self {
        Self {
            streaming: AtomicBool::new(false),
            flush: AtomicBool::new(false),
            rebuffer: AtomicBool::new(false),
            queued: AtomicUsize::new(0),
            prebuffer_ms: AtomicU32::new(300),
            underruns: AtomicU32::new(0),
        }
    }
}

/// Seconds of audio the ring holds.
const RING_SECS: usize = 30;
/// How long `push` waits for room in a full ring before dropping the rest.
const PUSH_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Output device fed from a lock-free ring at the device rate. Rebuilds its stream
/// when Windows invalidates it (headphones plugged in, Bluetooth connecting).
pub struct Player {
    stream: Mutex<Option<cpal::Stream>>,
    /// Only producers lock this; the device callback owns the consumer end.
    producer: Mutex<Option<rtrb::Producer<f32>>>,
    shared: Arc<Shared>,
    rate: AtomicU32,
    dead: Arc<AtomicBool>,
    /// The `OUTPUT` choice the stream was built for.
    built_for: Mutex<String>,
    /// A full ring dropped audio during this reply (warn once per reply).
    overflow_warned: AtomicBool,
    pub level: Arc<Level>,
}

impl Player {
    pub fn open() -> Result<Self> {
        let p = Self {
            stream: Mutex::new(None),
            producer: Mutex::new(None),
            shared: Default::default(),
            rate: AtomicU32::new(0),
            dead: Arc::new(AtomicBool::new(true)),
            built_for: Default::default(),
            overflow_warned: AtomicBool::new(false),
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
        let (shared, level, dead) = (self.shared.clone(), self.level.clone(), self.dead.clone());
        let chosen = want.clone();
        let (stream, rate, producer) = on_audio_thread(move || build_output(&chosen, shared, level, dead))?;
        *self.built_for.lock().unwrap() = want;
        // Samples queued for the old device were resampled for its rate; the new ring starts empty.
        *self.producer.lock().unwrap() = Some(producer);
        self.shared.queued.store(0, Ordering::SeqCst);
        self.rate.store(rate, Ordering::SeqCst);
        *self.stream.lock().unwrap() = Some(stream);
        self.dead.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Queues mono PCM, resampled to the device rate. A full ring waits up to 2 s for room, then drops the rest.
    pub fn push(&self, resampler: &mut Resampler, pcm: &[i16]) {
        self.push_f32(&resampler.process(&i16_to_f32(pcm)));
    }

    /// Queues what the resampler still holds; call after the last chunk of a reply.
    pub fn flush(&self, resampler: &mut Resampler) {
        self.push_f32(&resampler.flush());
    }

    fn push_f32(&self, out: &[f32]) {
        let mut guard = self.producer.lock().unwrap();
        let Some(prod) = guard.as_mut() else { return };
        let mut rest = out;
        let waited = std::time::Instant::now();
        while !rest.is_empty() {
            let n = prod.slots().min(rest.len());
            if n == 0 {
                if waited.elapsed() >= PUSH_WAIT || self.dead.load(Ordering::SeqCst) {
                    if !self.overflow_warned.swap(true, Ordering::SeqCst) {
                        tracing::warn!(dropped = rest.len(), "speaker queue full; audio dropped");
                    }
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
                continue;
            }
            for &v in &rest[..n] {
                let _ = prod.push(v);
            }
            self.shared.queued.fetch_add(n, Ordering::SeqCst);
            rest = &rest[n..];
        }
    }

    /// Marks the start (`true`) or end (`false`) of a reply that is still being generated. While it is on,
    /// playback prebuffers and waits out dry spells; turning it off plays the rest and stops waiting.
    pub fn set_streaming(&self, on: bool) {
        self.shared.streaming.store(on, Ordering::SeqCst);
        if on {
            self.shared.rebuffer.store(true, Ordering::SeqCst);
            self.overflow_warned.store(false, Ordering::SeqCst);
        }
    }

    /// Audio the speaker waits for before starting a reply, and after a dry spell.
    pub fn set_prebuffer_ms(&self, ms: u32) {
        self.shared.prebuffer_ms.store(ms, Ordering::SeqCst);
    }

    /// Dry spells since the last call (the log wants them per reply).
    pub fn take_underruns(&self) -> u32 {
        self.shared.underruns.swap(0, Ordering::SeqCst)
    }

    /// A resampler to the device rate. Reopens a dead stream first, so each reply after a
    /// device change plays on the new device.
    pub fn resampler_from(&self, sr: u32) -> Resampler {
        if let Err(e) = self.ensure() {
            tracing::warn!("speaker reopen: {e}");
        }
        Resampler::new(sr, self.rate.load(Ordering::SeqCst).max(1))
    }

    /// Drops everything queued and ends the reply (barge-in, or the round is over).
    pub fn clear(&self) {
        self.shared.streaming.store(false, Ordering::SeqCst);
        self.shared.flush.store(true, Ordering::SeqCst);
    }

    /// Idle when nothing is queued, or when the stream died (the queue would never drain).
    pub fn is_idle(&self) -> bool {
        self.dead.load(Ordering::SeqCst) || self.shared.queued.load(Ordering::SeqCst) == 0
    }
}

/// The device callback's own state: the consumer end of the ring and the jitter buffer.
struct Feed {
    ring: rtrb::Consumer<f32>,
    jitter: Jitter,
    shared: Arc<Shared>,
    rate: u32,
    level: Arc<Level>,
}

impl Feed {
    /// Fills `frames` frames through `out(frame, sample)`. No allocation, lock or logging.
    fn fill(&mut self, out: &mut dyn FnMut(usize, f32), frames: usize) {
        let sh = &*self.shared;
        if sh.flush.swap(false, Ordering::SeqCst) {
            let n = self.ring.slots();
            if let Ok(chunk) = self.ring.read_chunk(n) {
                chunk.commit_all();
            }
            sh.queued.store(0, Ordering::SeqCst);
            self.jitter.rebuffering = sh.streaming.load(Ordering::SeqCst);
        }
        if sh.rebuffer.swap(false, Ordering::SeqCst) {
            self.jitter.rebuffering = true;
        }
        self.jitter.prebuffer = self.rate as usize * sh.prebuffer_ms.load(Ordering::Relaxed) as usize / 1000;
        let streaming = sh.streaming.load(Ordering::SeqCst);
        let before = self.ring.slots();
        let mut sum = 0.0;
        for f in 0..frames {
            let v = self.jitter.next(&mut self.ring, streaming);
            sum += v * v;
            out(f, v);
        }
        let popped = before.saturating_sub(self.ring.slots());
        let _ = sh.queued.try_update(Ordering::SeqCst, Ordering::SeqCst, |q| Some(q.saturating_sub(popped)));
        if self.jitter.underruns > 0 {
            sh.underruns.fetch_add(std::mem::take(&mut self.jitter.underruns), Ordering::SeqCst);
        }
        self.level.set(if frames > 0 { (sum / frames as f32).sqrt() } else { 0.0 });
    }
}

fn build_output(chosen: &str, shared: Arc<Shared>, lv: Arc<Level>, dead: Arc<AtomicBool>) -> Result<(cpal::Stream, u32, rtrb::Producer<f32>)> {
    let device = pick_device(&cpal::default_host(), chosen, false).ok_or_else(|| err("no speaker found"))?;
    let supported = device.default_output_config().map_err(err)?;
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let channels = config.channels as usize;
    let rate = config.sample_rate;
    let (producer, ring) = rtrb::RingBuffer::<f32>::new(rate as usize * RING_SECS);
    let streaming = shared.streaming.load(Ordering::SeqCst);
    let mut feed = Feed { ring, jitter: Jitter { rebuffering: streaming, ..Default::default() }, shared, rate, level: lv };
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
                feed.fill(&mut |f, v| data[f * channels..(f + 1) * channels].fill(v), frames)
            },
            on_err,
            None,
        ),
        cpal::SampleFormat::I16 => device.build_output_stream(
            config,
            move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                let frames = data.len() / channels;
                feed.fill(&mut |f, v| data[f * channels..(f + 1) * channels].fill((v * 32767.0) as i16), frames)
            },
            on_err,
            None,
        ),
        other => return Err(err(format!("unsupported speaker format {other}"))),
    }
    .map_err(err)?;
    stream.play().map_err(err)?;
    tracing::info!(rate, channels, device = %device, "speaker open");
    Ok((stream, rate, producer))
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
        // The device callback only hands raw samples over. The 511-tap low-pass runs on a worker thread:
        // done inside the callback it overran the device buffer (a slow or busy CPU, and unoptimised dev
        // builds), the OS dropped those samples, and words went missing from what STT heard.
        let (raw_tx, raw_rx) = std::sync::mpsc::channel::<Vec<f32>>();
        std::thread::Builder::new()
            .name("onsite-mic-dsp".into())
            .spawn(move || {
                // Ends when the stream (and with it `raw_tx`) is dropped.
                for mut mono in raw_rx {
                    lv.set(rms(&mono));
                    let _ = tx.send(f32_to_i16(&rs.process(&mut mono)));
                }
            })
            .map_err(err)?;
        let handle = move |mono: Vec<f32>| {
            let _ = raw_tx.send(mono);
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
    fn playback_resampler_keeps_level_and_length() {
        let input = tone(44_100, 1000.0, 1.0);
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        for c in input.chunks(441) {
            out.extend(r.process(c));
        }
        out.extend(r.flush());
        assert!((47_500..=48_500).contains(&out.len()), "{}", out.len());
        let level = rms(&out[4800..43_200]);
        assert!((level / (0.5 / 2f32.sqrt()) - 1.0).abs() < 0.05, "{level}");
        // Same rate: untouched.
        assert_eq!(Resampler::new(48_000, 48_000).process(&[0.25; 10]), vec![0.25; 10]);
    }

    fn tone(rate: u32, hz: f32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n).map(|i| 0.5 * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin()).collect()
    }

    /// RMS of a 48 kHz tone after conversion to 16 kHz in 10 ms chunks, skipping the warm-up.
    fn rms_after(hz: f32, filtered: bool) -> f32 {
        let input = tone(48_000, hz, 1.0);
        let mut ds = Downsampler::new(48_000, 16_000);
        let mut plain = Linear::new(48_000, 16_000);
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
        assert!(a.taps.len() % 2 == 1 && a.taps.len() < 200, "taps {}", a.taps.len());
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

    /// Pulls `n` samples through the jitter buffer; returns (samples, silent count).
    fn pull(j: &mut Jitter, q: &mut VecDeque<f32>, streaming: bool, n: usize) -> (Vec<f32>, usize) {
        let out: Vec<f32> = (0..n).map(|_| j.next(q, streaming)).collect();
        let silent = out.iter().filter(|v| **v == 0.0).count();
        (out, silent)
    }

    #[test]
    fn jitter_waits_for_prebuffer_while_streaming() {
        let mut j = Jitter { prebuffer: 4, rebuffering: true, ..Default::default() };
        let mut q: VecDeque<f32> = [0.5, 0.5].into();
        assert_eq!(pull(&mut j, &mut q, true, 3).1, 3, "2 queued is below the prebuffer: silence");
        assert_eq!(j.underruns, 0, "waiting at the start is not an underrun");
        q.extend([0.5, 0.5, 0.5]);
        assert_eq!(pull(&mut j, &mut q, true, 3).0, vec![0.5, 0.5, 0.5]);
    }

    #[test]
    fn jitter_rebuffers_after_a_dry_spell_and_counts_it() {
        let mut j = Jitter { prebuffer: 4, ..Default::default() };
        let mut q: VecDeque<f32> = [0.5; 4].into();
        assert_eq!(pull(&mut j, &mut q, true, 4).1, 0);
        // Queue empty mid-reply: silence, one underrun, and the next part must fill the prebuffer again.
        assert_eq!(pull(&mut j, &mut q, true, 1).1, 1);
        assert_eq!(j.underruns, 1);
        q.extend([0.5; 2]);
        assert_eq!(pull(&mut j, &mut q, true, 2).1, 2, "2 of 4 prebuffered: still waiting");
        q.extend([0.5; 2]);
        assert_eq!(pull(&mut j, &mut q, true, 4).1, 0, "prebuffer reached: plays on");
        assert_eq!(j.underruns, 1);
    }

    #[test]
    fn ring_source_reports_len_and_pops_in_order() {
        let (mut p, mut c) = rtrb::RingBuffer::<f32>::new(8);
        for v in [0.1, 0.2, 0.3] {
            p.push(v).unwrap();
        }
        assert_eq!(Source::len(&c), 3);
        assert_eq!(Source::pop(&mut c), Some(0.1));
        let mut j = Jitter { prebuffer: 2, ..Default::default() };
        assert_eq!(j.next(&mut c, true), 0.2);
        assert_eq!(j.next(&mut c, true), 0.3);
        assert_eq!(j.next(&mut c, true), 0.0, "dry: silence and an underrun");
        assert_eq!(j.underruns, 1);
    }

    #[test]
    fn jitter_plays_the_end_of_a_short_reply() {
        let mut j = Jitter { prebuffer: 100, rebuffering: true, ..Default::default() };
        let mut q: VecDeque<f32> = [0.5, 0.25].into();
        // The reply ended before the prebuffer filled: play what there is, no waiting.
        assert_eq!(pull(&mut j, &mut q, false, 2).0, vec![0.5, 0.25]);
        assert_eq!(pull(&mut j, &mut q, false, 1).1, 1, "nothing left: silence, not an underrun");
        assert_eq!(j.underruns, 0);
    }
}
