//! The built-in voice engine: Supertonic 3 (speech) and Parakeet TDT (text) through sherpa-onnx, in this process.
//! Why Supertonic replaced Pocket TTS, and what was measured: `docs/engineering/VOICE.md`.
//!
//! Everything here blocks; callers run it with `spawn_blocking`.

use super::models::{self, Pin};
use crate::error::{AppError, Result};
use serde_json::json;
use sherpa_onnx::{
    GenerationConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, OfflineTts, OfflineTtsConfig,
    OfflineTtsModelConfig, OfflineTtsSupertonicModelConfig,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

/// The two shipped voices: (key, name shown in Settings, description, Supertonic speaker id).
/// The keys stay "ethan_brooks" and "priya_shah" so saved settings and panel personas keep working.
/// Speaker ids 0-4 are the five female styles and 5-9 the five male ones (pitch measured on the model's output,
/// 2026-11-02). Supertonic has no accent labels, so none are shown.
pub const VOICES: [(&str, &str, &str, i32); 2] = [("ethan_brooks", "Ethan", "Male voice", 5), ("priya_shah", "Charlotte", "Female voice", 0)];
pub const DEFAULT_VOICE: &str = "ethan_brooks";
/// Long sentences are cut into parts of about this many words. The model returns audio only when a whole
/// part is done, so a long sentence is a long wait before anything plays.
const MAX_PIECE_WORDS: usize = 14;
/// A cut never leaves a part shorter than this; very short parts sound clipped.
const MIN_PIECE_WORDS: usize = 4;
/// Flow-matching steps per part. Measured on the owner's Ryzen 7 250: 5 steps RTF 0.13, 16 steps RTF 0.4.
const STEPS: i32 = 8;

/// ONNX threads for each model: about one per physical core, at most 4 (more didn't help on an 8-core CPU).
/// Using every core starved the speaker thread on 4-thread laptops, and the audio stuttered.
fn threads() -> i32 {
    let logical = std::thread::available_parallelism().map_or(4, |n| n.get());
    (logical / 2).clamp(1, 4) as i32
}

/// Splits `text` into the parts the TTS speaks one call at a time: sentences over [`MAX_PIECE_WORDS`] are cut at a
/// comma, semicolon, colon or dash near their middle; short sentences are packed together up to that size.
pub fn pieces(text: &str) -> Vec<String> {
    let mut units = Vec::new();
    for s in sentences(text) {
        split_long(&s.split_whitespace().collect::<Vec<_>>(), &mut units);
    }
    let mut out: Vec<String> = Vec::new();
    for u in units {
        match out.last_mut() {
            Some(last) if words(last) + words(&u) <= MAX_PIECE_WORDS => {
                last.push(' ');
                last.push_str(&u);
            }
            _ => out.push(u),
        }
    }
    out
}

fn words(s: &str) -> usize {
    s.split_whitespace().count()
}

/// Sentences of `text`, each ending at `.`, `!` or `?` followed by a space (or at the end).
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let b = text.as_bytes();
    for i in 0..b.len() {
        if matches!(b[i], b'.' | b'!' | b'?') && b.get(i + 1).is_some_and(|c| c.is_ascii_whitespace()) {
            out.push(text[start..=i].trim());
            start = i + 1;
        }
    }
    out.push(text[start..].trim());
    out.retain(|s| !s.is_empty());
    out
}

fn split_long(w: &[&str], out: &mut Vec<String>) {
    if w.len() <= MAX_PIECE_WORDS {
        if !w.is_empty() {
            out.push(w.join(" "));
        }
        return;
    }
    // A cut after word i, where word i ends a clause and both sides keep MIN_PIECE_WORDS words.
    let clause_end = |s: &str| s.ends_with([',', ';', ':']) || matches!(s, "-" | "\u{2013}" | "\u{2014}") || s.ends_with(['\u{2014}', '\u{2013}']);
    let cut = (MIN_PIECE_WORDS - 1..w.len() - MIN_PIECE_WORDS)
        .filter(|&i| clause_end(w[i]))
        .min_by_key(|&i| (i + 1).abs_diff(w.len() / 2));
    match cut {
        Some(i) => {
            split_long(&w[..=i], out);
            split_long(&w[i + 1..], out);
        }
        None => out.push(w.join(" ")),
    }
}

/// Panel interviewers share the two voices: women get Charlotte, everyone else Ethan.
pub fn clip_for_gender(gender: &str) -> &'static str {
    if gender.trim().to_lowercase().starts_with("f") {
        "priya_shah"
    } else {
        DEFAULT_VOICE
    }
}

/// A saved or unknown voice key falls back to the default voice.
pub fn known_clip(id: &str) -> &'static str {
    VOICES.iter().find(|v| v.0 == id).map_or(DEFAULT_VOICE, |v| v.0)
}

/// Pace of a voice on top of the user's speed. Ethan's Supertonic voice reads slower than Charlotte's at the same setting.
pub fn tempo(id: &str) -> f32 {
    if known_clip(id) == "ethan_brooks" {
        1.2
    } else {
        1.0
    }
}

/// The Supertonic speaker id for a voice key.
pub fn speaker_id(id: &str) -> i32 {
    let key = known_clip(id);
    VOICES.iter().find(|v| v.0 == key).map_or(VOICES[0].3, |v| v.3)
}

fn file(dir: &Path, name: &str) -> Option<String> {
    Some(dir.join(name).to_string_lossy().into_owned())
}

fn voice_err(what: &str) -> AppError {
    AppError::Voice(format!("{what} Check Settings > Voice."))
}

pub struct Supertonic {
    tts: Mutex<OfflineTts>,
    pub rate: u32,
}

impl Supertonic {
    /// `models_root` = `<app data>/models`.
    pub fn load(models_root: &Path) -> Result<Self> {
        let dir = models_root.join(models::SUPERTONIC.id);
        let model = OfflineTtsModelConfig {
            supertonic: OfflineTtsSupertonicModelConfig {
                duration_predictor: file(&dir, "duration_predictor.int8.onnx"),
                text_encoder: file(&dir, "text_encoder.int8.onnx"),
                vector_estimator: file(&dir, "vector_estimator.int8.onnx"),
                vocoder: file(&dir, "vocoder.int8.onnx"),
                tts_json: file(&dir, "tts.json"),
                unicode_indexer: file(&dir, "unicode_indexer.bin"),
                voice_style: file(&dir, "voice.bin"),
            },
            num_threads: threads(),
            provider: Some("cpu".into()),
            ..Default::default()
        };
        let tts = OfflineTts::create(&OfflineTtsConfig { model, ..Default::default() }).ok_or_else(|| voice_err("The voice model didn't load."))?;
        let rate = tts.sample_rate() as u32;
        Ok(Self { tts: Mutex::new(tts), rate })
    }

    /// Speaks `text` in voice `voice` (a voice key), sending PCM16 (at `self.rate`) to `out` as each part finishes.
    /// Stops early when `out` is closed. Returns the number of samples sent.
    pub fn speak(&self, text: &str, voice: &str, speed: f32, out: UnboundedSender<Vec<i16>>) -> Result<usize> {
        // The model reads text as-is (no phonemes), so only the language is set.
        let cfg = GenerationConfig {
            sid: speaker_id(voice),
            speed: speed * tempo(voice),
            num_steps: STEPS,
            extra: Some(HashMap::from([("lang".to_string(), json!("en"))])),
            ..Default::default()
        };
        // A short line must not run on for seconds. Cap the audio at a generous per-word length.
        let cap = ((text.split_whitespace().count() as f32 * 0.7 + 2.5) * self.rate as f32) as usize;
        let sent = Arc::new(AtomicUsize::new(0));
        let (cb_sent, cb_out) = (sent.clone(), out.clone());
        let audio = self.tts.lock().unwrap().generate_with_config(
            text,
            &cfg,
            Some(move |s: &[f32], _p: f32| send(&cb_out, &cb_sent, s, cap)),
        );
        // No per-sentence callback fired (a very short line): send what came back.
        if sent.load(Ordering::SeqCst) == 0 {
            if let Some(a) = audio {
                send(&out, &sent, a.samples(), cap);
            }
        }
        Ok(sent.load(Ordering::SeqCst))
    }
}

/// Forwards `s` up to `cap` samples in total. False tells sherpa to stop generating.
fn send(out: &UnboundedSender<Vec<i16>>, sent: &AtomicUsize, s: &[f32], cap: usize) -> bool {
    let room = cap.saturating_sub(sent.load(Ordering::SeqCst));
    let s = &s[..s.len().min(room)];
    if !s.is_empty() {
        sent.fetch_add(s.len(), Ordering::SeqCst);
        if out.send(super::audio::f32_to_i16(s)).is_err() {
            return false;
        }
    }
    room > s.len()
}

pub struct ParakeetStt {
    rec: Mutex<OfflineRecognizer>,
}

impl ParakeetStt {
    pub fn load(models_root: &Path) -> Result<Self> {
        let dir = models_root.join(models::PARAKEET.id);
        let mut c = OfflineRecognizerConfig::default();
        c.model_config.transducer = OfflineTransducerModelConfig {
            encoder: file(&dir, "encoder.int8.onnx"),
            decoder: file(&dir, "decoder.int8.onnx"),
            joiner: file(&dir, "joiner.int8.onnx"),
        };
        c.model_config.tokens = file(&dir, "tokens.txt");
        c.model_config.model_type = Some("nemo_transducer".into());
        c.model_config.num_threads = threads();
        c.model_config.provider = Some("cpu".into());
        let rec = OfflineRecognizer::create(&c).ok_or_else(|| voice_err("The speech model didn't load."))?;
        Ok(Self { rec: Mutex::new(rec) })
    }

    fn decode(rec: &OfflineRecognizer, pcm: &[i16]) -> String {
        let s = rec.create_stream();
        s.accept_waveform(super::audio::MIC_RATE as i32, &super::audio::i16_to_f32(pcm));
        rec.decode(&s);
        s.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default()
    }

    /// Text of one finished segment (16 kHz mono). Waits for a caption decode in progress.
    pub fn transcribe(&self, pcm: &[i16]) -> String {
        Self::decode(&self.rec.lock().unwrap(), pcm)
    }

    /// Caption for the open segment. None when a decode is already running: that tick is skipped.
    pub fn caption(&self, pcm: &[i16]) -> Option<String> {
        self.rec.try_lock().ok().map(|rec| Self::decode(&rec, pcm))
    }
}

/// Both halves of the engine, loaded once and kept for later rounds.
#[derive(Default)]
pub struct Cache {
    tts: Mutex<Option<Arc<Supertonic>>>,
    stt: Mutex<Option<Arc<ParakeetStt>>>,
}

fn not_installed(p: &Pin) -> AppError {
    AppError::Voice(format!("The {} model isn't installed. Download it in Settings > Voice.", p.label))
}

impl Cache {
    pub async fn tts(&self, data_dir: &Path) -> Result<Arc<Supertonic>> {
        if let Some(t) = self.tts.lock().unwrap().clone() {
            return Ok(t);
        }
        if !models::installed(data_dir, &models::SUPERTONIC) {
            return Err(not_installed(&models::SUPERTONIC));
        }
        let root = models::root(data_dir);
        let t0 = std::time::Instant::now();
        let t = Arc::new(tokio::task::spawn_blocking(move || Supertonic::load(&root)).await.map_err(|e| AppError::Voice(e.to_string()))??);
        let cpus = std::thread::available_parallelism().map_or(0, |n| n.get());
        tracing::info!(ms = t0.elapsed().as_millis() as u64, cpus, threads = threads(), "Supertonic TTS loaded");
        *self.tts.lock().unwrap() = Some(t.clone());
        Ok(t)
    }

    pub async fn stt(&self, data_dir: &Path) -> Result<Arc<ParakeetStt>> {
        if let Some(s) = self.stt.lock().unwrap().clone() {
            return Ok(s);
        }
        if !models::installed(data_dir, &models::PARAKEET) {
            return Err(not_installed(&models::PARAKEET));
        }
        let root = models::root(data_dir);
        let t0 = std::time::Instant::now();
        let s = Arc::new(tokio::task::spawn_blocking(move || ParakeetStt::load(&root)).await.map_err(|e| AppError::Voice(e.to_string()))??);
        tracing::info!(ms = t0.elapsed().as_millis() as u64, "Parakeet loaded");
        *self.stt.lock().unwrap() = Some(s.clone());
        Ok(s)
    }

    /// Drops the loaded models (after a delete, or "Reload models").
    pub fn unload(&self) {
        *self.tts.lock().unwrap() = None;
        *self.stt.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc::unbounded_channel;

    #[test]
    fn women_get_priya() {
        assert_eq!(clip_for_gender("female"), "priya_shah");
        assert_eq!(clip_for_gender("Female"), "priya_shah");
        assert_eq!(clip_for_gender("male"), "ethan_brooks");
        assert_eq!(clip_for_gender(""), "ethan_brooks");
        assert_eq!(known_clip("nobody"), "ethan_brooks");
        assert_eq!(known_clip("priya_shah"), "priya_shah");
    }

    #[test]
    fn long_sentences_are_cut_at_clauses() {
        let long = "If two requests carry the same idempotency key, the second one should return the stored response, and never charge the card twice.";
        let p = pieces(long);
        assert_eq!(p, ["If two requests carry the same idempotency key,", "the second one should return the stored response, and never charge the card twice."]);
        assert_eq!(p.join(" "), long);
        // Short sentences are packed together; a long one without commas is left whole.
        assert_eq!(pieces("Okay. Got it. Let's move on."), ["Okay. Got it. Let's move on."]);
        let plain = "word ".repeat(20);
        assert_eq!(pieces(&plain), [plain.trim()]);
        assert!(pieces("  ").is_empty());
        assert_eq!(pieces("It costs 1.5 ms. Fine."), ["It costs 1.5 ms. Fine."]);
    }

    #[test]
    fn voices_map_to_supertonic_speakers() {
        // Speakers 0-4 are female, 5-9 male (pitch check, 2026-11-02). Saved keys keep working.
        assert_eq!(speaker_id("priya_shah"), 0);
        assert_eq!(speaker_id("ethan_brooks"), 5);
        assert_eq!(speaker_id("nobody"), 5, "unknown keys fall back to the default voice");
        assert!(VOICES.iter().all(|v| (0..10).contains(&v.3)));
        assert!(tempo("ethan_brooks") > tempo("priya_shah"), "Ethan is sped up, Charlotte is not");
    }

    #[test]
    fn thread_count_is_sane() {
        assert!((1..=4).contains(&threads()));
    }

    #[test]
    fn audio_is_capped_and_stops_when_closed() {
        let (tx, mut rx) = unbounded_channel();
        let sent = AtomicUsize::new(0);
        assert!(send(&tx, &sent, &[0.1; 100], 150));
        assert!(!send(&tx, &sent, &[0.1; 100], 150), "cap reached: stop generating");
        assert_eq!(sent.load(Ordering::SeqCst), 150);
        assert_eq!(rx.try_recv().unwrap().len(), 100);
        assert_eq!(rx.try_recv().unwrap().len(), 50);
        drop(rx);
        assert!(!send(&tx, &AtomicUsize::new(0), &[0.1; 10], 100), "listener gone");
    }
}
