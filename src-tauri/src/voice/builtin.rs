//! The built-in voice engine: Pocket TTS (speech) and Parakeet TDT (text) through sherpa-onnx, in this process.
//! Settings that worked in the Phase 0 spike are in `docs/plans/voice-spike-results.md`.
//!
//! Everything here blocks; callers run it with `spawn_blocking`.

use super::models::{self, Pin};
use crate::error::{AppError, Result};
use serde_json::json;
use sherpa_onnx::{
    GenerationConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, OfflineTts, OfflineTtsConfig,
    OfflineTtsModelConfig, OfflineTtsPocketModelConfig, Wave,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc::UnboundedSender;

/// The two shipped voices: (clip id = file in `prompts/voices/`, name shown in Settings, accent).
pub const VOICES: [(&str, &str, &str); 2] = [("ethan_brooks", "Ethan", "American, male"), ("priya_shah", "Priya", "British, female")];
pub const DEFAULT_VOICE: &str = "ethan_brooks";
const THREADS: i32 = 4;
/// Pocket reads at most this much of the reference clip; the clips are ~21 s. Longer references held the voice steady.
const MAX_REFERENCE_S: f64 = 25.0;
/// Fixed seed: the same line sounds the same on every run.
const SEED: i64 = 42;

/// Panel interviewers share the two voices: women get Priya, everyone else Ethan.
pub fn clip_for_gender(gender: &str) -> &'static str {
    if gender.trim().to_lowercase().starts_with("f") {
        "priya_shah"
    } else {
        DEFAULT_VOICE
    }
}

/// A saved or unknown clip id falls back to the default voice.
pub fn known_clip(id: &str) -> &'static str {
    VOICES.iter().find(|v| v.0 == id).map_or(DEFAULT_VOICE, |v| v.0)
}

struct Clip {
    samples: Vec<f32>,
    rate: i32,
}

pub struct PocketTts {
    tts: Mutex<OfflineTts>,
    clips: HashMap<&'static str, Clip>,
    pub rate: u32,
}

fn file(dir: &Path, name: &str) -> Option<String> {
    Some(dir.join(name).to_string_lossy().into_owned())
}

fn voice_err(what: &str) -> AppError {
    AppError::Voice(format!("{what} Check Settings > Voice."))
}

impl PocketTts {
    /// `models_root` = `<app data>/models`; `voices_dir` = `prompts/voices`.
    pub fn load(models_root: &Path, voices_dir: &Path) -> Result<Self> {
        let dir = models_root.join(models::POCKET.id);
        let mut model = OfflineTtsModelConfig { num_threads: THREADS, provider: Some("cpu".into()), ..Default::default() };
        model.pocket = OfflineTtsPocketModelConfig {
            lm_flow: file(&dir, "lm_flow.int8.onnx"),
            lm_main: file(&dir, "lm_main.int8.onnx"),
            encoder: file(&dir, "encoder.onnx"),
            decoder: file(&dir, "decoder.int8.onnx"),
            text_conditioner: file(&dir, "text_conditioner.onnx"),
            vocab_json: file(&dir, "vocab.json"),
            token_scores_json: file(&dir, "token_scores.json"),
            voice_embedding_cache_capacity: 16,
        };
        let tts = OfflineTts::create(&OfflineTtsConfig { model, ..Default::default() }).ok_or_else(|| voice_err("The voice model didn't load."))?;
        let mut clips = HashMap::new();
        for (id, ..) in VOICES {
            let w = Wave::read(&voices_dir.join(format!("{id}.wav")).to_string_lossy()).ok_or_else(|| voice_err(&format!("The voice clip {id}.wav is missing.")))?;
            clips.insert(id, Clip { samples: w.samples().to_vec(), rate: w.sample_rate() });
        }
        let rate = tts.sample_rate() as u32;
        Ok(Self { tts: Mutex::new(tts), clips, rate })
    }

    /// Speaks `text` in voice `clip`, sending PCM16 (at `self.rate`) to `out` as each sentence finishes.
    /// Stops early when `out` is closed. Returns the number of samples sent.
    pub fn speak(&self, text: &str, clip: &str, speed: f32, out: UnboundedSender<Vec<i16>>) -> Result<usize> {
        let c = &self.clips[known_clip(clip)];
        let mut extra = HashMap::new();
        extra.insert("max_reference_audio_len".to_string(), json!(MAX_REFERENCE_S));
        extra.insert("seed".to_string(), json!(SEED));
        let cfg = GenerationConfig {
            speed,
            reference_audio: Some(c.samples.clone()),
            reference_sample_rate: c.rate,
            extra: Some(extra),
            ..Default::default()
        };
        // Pocket sometimes runs on for tens of seconds after a one-word line ("Okay."). Cap the audio.
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
        c.model_config.num_threads = THREADS;
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
    tts: Mutex<Option<Arc<PocketTts>>>,
    stt: Mutex<Option<Arc<ParakeetStt>>>,
}

fn not_installed(p: &Pin) -> AppError {
    AppError::Voice(format!("The {} model isn't installed. Download it in Settings > Voice.", p.label))
}

impl Cache {
    pub async fn tts(&self, data_dir: &Path, voices_dir: &Path) -> Result<Arc<PocketTts>> {
        if let Some(t) = self.tts.lock().unwrap().clone() {
            return Ok(t);
        }
        if !models::installed(data_dir, &models::POCKET) {
            return Err(not_installed(&models::POCKET));
        }
        let (root, voices) = (models::root(data_dir), voices_dir.to_path_buf());
        let t0 = std::time::Instant::now();
        let t = Arc::new(tokio::task::spawn_blocking(move || PocketTts::load(&root, &voices)).await.map_err(|e| AppError::Voice(e.to_string()))??);
        tracing::info!(ms = t0.elapsed().as_millis() as u64, "Pocket TTS loaded");
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
