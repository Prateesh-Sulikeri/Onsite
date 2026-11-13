//! The built-in voice engine: Supertonic 3 (speech) and Parakeet TDT (text) through sherpa-onnx, in this process.
//! Why Supertonic replaced Pocket TTS, and what was measured: `docs/engineering/VOICE.md`.
//!
//! Everything here blocks; callers run it with `spawn_blocking`.

use super::health::Health;
use super::models::{self, Pin};
use crate::error::{AppError, Result};
use serde_json::json;
use sherpa_onnx::{
    GenerationConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, OfflineTts, OfflineTtsConfig,
    OfflineTtsModelConfig, OfflineTtsSupertonicModelConfig,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// The two shipped voices: (key, name shown in Settings, description, Supertonic speaker id).
/// The keys stay "ethan_brooks" and "priya_shah" so saved settings and panel personas keep working.
/// Speaker ids 0-4 are the five female styles and 5-9 the five male ones (pitch measured on the model's output,
/// 2026-11-02). Supertonic has no accent labels, so none are shown.
pub const VOICES: [(&str, &str, &str, i32); 2] = [("ethan_brooks", "Ethan", "Male voice", 5), ("priya_shah", "Charlotte", "Female voice", 0)];
pub const DEFAULT_VOICE: &str = "ethan_brooks";
/// A cut never leaves a part shorter than this; very short parts sound clipped.
const MIN_PIECE_WORDS: usize = 4;
/// Audio shorter than this per word (at speed 1.0) means words were dropped.
pub const MIN_S_PER_WORD: f32 = 0.16;

/// ONNX threads for each model: about one per physical core, at most 4 (more didn't help on an 8-core CPU).
/// Using every core starved the speaker thread on 4-thread laptops, and the audio stuttered.
pub fn threads() -> i32 {
    let logical = std::thread::available_parallelism().map_or(4, |n| n.get());
    (logical / 2).clamp(1, 4) as i32
}

/// Splits `text` into the parts the TTS speaks one call at a time: sentences over `max_words` are cut at a
/// comma, semicolon, colon or dash near their middle; short sentences are packed together up to that size.
/// The model returns audio only when a whole part is done, so a long part is a long wait before anything plays.
pub fn pieces(text: &str, max_words: usize) -> Vec<String> {
    let mut units = Vec::new();
    for s in sentences(text) {
        split_long(&s.split_whitespace().collect::<Vec<_>>(), max_words, &mut units);
    }
    let mut out: Vec<String> = Vec::new();
    for u in units {
        match out.last_mut() {
            Some(last) if words(last) + words(&u) <= max_words => {
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

/// Sentences of `text`, cut by the same rule as the reply splitter (abbreviations and decimals stay whole).
fn sentences(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(n) = crate::brain::splitter::first_sentence_len(rest) {
        out.push(rest[..n].trim());
        rest = &rest[n..];
    }
    out.push(rest.trim());
    out.retain(|s| !s.is_empty());
    out
}

fn split_long(w: &[&str], max_words: usize, out: &mut Vec<String>) {
    if w.len() <= max_words {
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
            split_long(&w[..=i], max_words, out);
            split_long(&w[i + 1..], max_words, out);
        }
        None => out.push(w.join(" ")),
    }
}

/// What the length guard found for one generated piece.
#[derive(Debug, PartialEq)]
pub enum Verdict {
    Ok,
    /// Shorter than the floor per word: words were dropped.
    TooShort,
    /// Ran past the cap: keep this many samples.
    TooLong { keep: usize },
}

/// Checks `samples` of audio for `words` words at model speed `speed`. Both bounds scale with speed.
pub fn judge(samples: usize, words: usize, speed: f32, rate: u32) -> Verdict {
    let speed = speed.clamp(0.5, 2.0) as f64;
    let secs = samples as f64 / rate as f64;
    let max_s = words as f64 * 0.9 / speed + 3.0;
    let min_s = words as f64 * MIN_S_PER_WORD as f64 / speed;
    if secs > max_s {
        Verdict::TooLong { keep: (max_s * rate as f64).round() as usize }
    } else if words >= 4 && secs < min_s {
        Verdict::TooShort
    } else {
        Verdict::Ok
    }
}

fn ms_samples(ms: u32, rate: u32) -> usize {
    (rate as u64 * ms as u64 / 1000) as usize
}

/// Linear ramp up over the first `n` samples (the whole slice when shorter).
pub fn fade_in(s: &mut [f32], n: usize) {
    let n = n.min(s.len());
    for (i, v) in s[..n].iter_mut().enumerate() {
        *v *= i as f32 / n as f32;
    }
}

/// Linear ramp down over the last `n` samples (the whole slice when shorter).
pub fn fade_out(s: &mut [f32], n: usize) {
    let n = n.min(s.len());
    let tail = s.len() - n;
    for (i, v) in s[tail..].iter_mut().rev().enumerate() {
        *v *= i as f32 / n as f32;
    }
}

/// Drops leading and trailing 10 ms windows quieter than RMS 0.004, keeping 20 ms on each side.
pub fn trim_silence(s: &[f32], rate: u32) -> &[f32] {
    let win = ms_samples(10, rate).max(1);
    let loud = |w: &[f32]| (w.iter().map(|v| v * v).sum::<f32>() / w.len() as f32).sqrt() >= 0.004;
    let windows: Vec<&[f32]> = s.chunks(win).collect();
    let (Some(first), Some(last)) = (windows.iter().position(|w| loud(w)), windows.iter().rposition(|w| loud(w))) else {
        return &s[..0];
    };
    let keep = ms_samples(20, rate);
    let start = (first * win).saturating_sub(keep);
    let end = ((last + 1) * win + keep).min(s.len());
    &s[start..end]
}

/// Silence after a piece: a breath after a sentence, a short beat after a clause.
pub fn gap_ms(piece: &str) -> u32 {
    if ends_sentence(piece) {
        250
    } else {
        80
    }
}

fn ends_sentence(piece: &str) -> bool {
    piece.trim_end().ends_with(['.', '!', '?'])
}

/// One generated piece, ready for the speaker (PCM16 at the TTS rate, gap included).
#[derive(Debug, Clone)]
pub struct Piece {
    pub pcm: Vec<i16>,
    pub ends_sentence: bool,
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

/// One generation call: speaker, model speed and flow-matching steps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Synth {
    pub sid: i32,
    pub speed: f32,
    pub steps: i32,
}

pub struct Supertonic {
    tts: Mutex<OfflineTts>,
    pub rate: u32,
}

impl Supertonic {
    /// `models_root` = `<app data>/models`.
    pub fn load(models_root: &Path) -> Result<Self> {
        Self::load_with(models_root, threads())
    }

    /// Same as [`Supertonic::load`] with a chosen ONNX thread count (the bench uses it).
    pub fn load_with(models_root: &Path, threads: i32) -> Result<Self> {
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
            num_threads: threads,
            provider: Some("cpu".into()),
            ..Default::default()
        };
        let tts = OfflineTts::create(&OfflineTtsConfig { model, ..Default::default() }).ok_or_else(|| voice_err("The voice model didn't load."))?;
        let rate = tts.sample_rate() as u32;
        Ok(Self { tts: Mutex::new(tts), rate })
    }

    /// One sherpa call for `text`: every sample at `self.rate`, no cap, no splitting. `stop` aborts generation.
    pub fn synth(&self, text: &str, s: &Synth, stop: &Arc<AtomicBool>) -> Result<Vec<f32>> {
        let cfg = GenerationConfig {
            sid: s.sid,
            speed: s.speed,
            num_steps: s.steps,
            extra: Some(HashMap::from([("lang".to_string(), json!("en"))])),
            ..Default::default()
        };
        // The callback must be 'static, so it holds its own handle on the flag.
        let stop = stop.clone();
        let audio = self
            .tts
            .lock()
            .unwrap()
            .generate_with_config(text, &cfg, Some(move |_: &[f32], _: f32| !stop.load(Ordering::SeqCst)));
        Ok(audio.map(|a| a.samples().to_vec()).unwrap_or_default())
    }

    /// One piece through the guard: generate, check the length against the word count, retry once when words
    /// were dropped, cut (with a fade) when it ran on, trim edge silence, fade the edges, add the gap after it.
    pub fn speak_piece(&self, text: &str, s: &Synth, stop: &Arc<AtomicBool>, health: &Health) -> Result<Piece> {
        let words = text.split_whitespace().count();
        let mut pcm = self.synth(text, s, stop)?;
        if judge(pcm.len(), words, s.speed, self.rate) == Verdict::TooShort && !stop.load(Ordering::SeqCst) {
            health.retry();
            let again = self.synth(text, &Synth { steps: s.steps + 2, ..*s }, stop)?;
            if again.len() > pcm.len() {
                pcm = again;
            }
        }
        if let Verdict::TooLong { keep } = judge(pcm.len(), words, s.speed, self.rate) {
            pcm.truncate(keep);
            fade_out(&mut pcm, ms_samples(30, self.rate));
            health.cap_hit();
            tracing::warn!(words, "tts piece ran past its length cap and was cut");
        }
        let mut out = trim_silence(&pcm, self.rate).to_vec();
        let edge = ms_samples(10, self.rate);
        fade_in(&mut out, edge);
        fade_out(&mut out, edge);
        if !out.is_empty() {
            out.resize(out.len() + ms_samples(gap_ms(text), self.rate), 0.0);
        }
        Ok(Piece { pcm: super::audio::f32_to_i16(&out), ends_sentence: ends_sentence(text) })
    }
}

pub struct ParakeetStt {
    rec: Mutex<OfflineRecognizer>,
}

impl ParakeetStt {
    pub fn load(models_root: &Path) -> Result<Self> {
        Self::load_with(models_root, threads())
    }

    /// Same as [`ParakeetStt::load`] with a chosen ONNX thread count (the bench uses it).
    pub fn load_with(models_root: &Path, threads: i32) -> Result<Self> {
        let dir = models_root.join(models::PARAKEET.id);
        let mut c = OfflineRecognizerConfig::default();
        c.model_config.transducer = OfflineTransducerModelConfig {
            encoder: file(&dir, "encoder.int8.onnx"),
            decoder: file(&dir, "decoder.int8.onnx"),
            joiner: file(&dir, "joiner.int8.onnx"),
        };
        c.model_config.tokens = file(&dir, "tokens.txt");
        c.model_config.model_type = Some("nemo_transducer".into());
        c.model_config.num_threads = threads;
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

    /// Text of 16 kHz mono float audio (the bench uses it).
    pub fn transcribe_f32(&self, pcm16k: &[f32]) -> String {
        let rec = self.rec.lock().unwrap();
        let s = rec.create_stream();
        s.accept_waveform(super::audio::MIC_RATE as i32, pcm16k);
        rec.decode(&s);
        s.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default()
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
        let p = pieces(long, 14);
        assert_eq!(p, ["If two requests carry the same idempotency key,", "the second one should return the stored response, and never charge the card twice."]);
        assert_eq!(p.join(" "), long);
        // Short sentences are packed together; a long one without commas is left whole.
        assert_eq!(pieces("Okay. Got it. Let's move on.", 14), ["Okay. Got it. Let's move on."]);
        let plain = "word ".repeat(20);
        assert_eq!(pieces(&plain, 14), [plain.trim()]);
        assert!(pieces("  ", 14).is_empty());
        assert_eq!(pieces("It costs 1.5 ms. Fine.", 14), ["It costs 1.5 ms. Fine."]);
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
    fn judge_scales_with_speed_and_words() {
        let r = 44_100;
        assert!(matches!(judge(r as usize * 4, 10, 1.0, r), Verdict::Ok));
        // 10 words in 1 s: words were dropped.
        assert!(matches!(judge(r as usize, 10, 1.0, r), Verdict::TooShort));
        // Short lines are never "too short".
        assert!(matches!(judge(r as usize / 4, 3, 1.0, r), Verdict::Ok));
        // 10 words: cap is 12 s at speed 1.0 and 14.25 s at 0.8.
        assert!(matches!(judge(r as usize * 13, 10, 1.0, r), Verdict::TooLong { keep } if keep == r as usize * 12));
        assert!(matches!(judge(r as usize * 13, 10, 0.8, r), Verdict::Ok));
    }

    #[test]
    fn fades_and_trim() {
        let mut s = vec![1.0f32; 100];
        fade_in(&mut s, 10);
        fade_out(&mut s, 10);
        assert_eq!(s[0], 0.0);
        assert!(s[5] > 0.3 && s[5] < 0.7 && s[50] == 1.0 && s[99] < 0.15);
        let rate = 16_000;
        let mut clip = vec![0.0f32; 4800];
        clip.extend(vec![0.5f32; 1600]);
        clip.extend(vec![0.0f32; 4800]);
        let t = trim_silence(&clip, rate);
        // 100 ms of sound plus 20 ms kept on each side.
        assert!((2200..=2600).contains(&t.len()), "{}", t.len());
        assert_eq!(trim_silence(&[0.0; 3200], rate).len(), 0);
    }

    #[test]
    fn gaps_follow_punctuation() {
        assert_eq!(gap_ms("Okay, got it."), 250);
        assert_eq!(gap_ms("If the key repeats,"), 80);
        assert_eq!(gap_ms("Why?  "), 250);
    }

    #[test]
    fn pieces_use_the_splitter_rules() {
        // "e.g." does not end a piece any more.
        assert_eq!(pieces("Use a map, e.g. a hash map. It is fast.", 14), ["Use a map, e.g. a hash map. It is fast."]);
        assert_eq!(sentences("Use a map, e.g. a hash map. It is fast."), ["Use a map, e.g. a hash map.", "It is fast."]);
    }

    #[test]
    fn fixtures_parse() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice");
        let tech = std::fs::read_to_string(dir.join("tts_tech_lines.tsv")).unwrap();
        let rows: Vec<_> = tech.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).collect();
        assert_eq!(rows.len(), 30);
        assert!(rows.iter().all(|r| r.split('\t').count() == 2));
        let stt = std::fs::read_to_string(dir.join("stt_terms.tsv")).unwrap();
        assert_eq!(stt.lines().filter(|l| l.split('\t').count() == 2).count(), 12);
    }

    #[test]
    fn thread_count_is_sane() {
        assert!((1..=4).contains(&threads()));
    }
}
