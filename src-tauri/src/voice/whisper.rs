//! Whisper large-v3 on the GPU, in one long-lived Python process (`sidecar/whisper_server.py`).
//!
//! Measured 2026-10-26 on 8 accented answers full of tech terms (RTX 5050, VoiceStudio 0.5.6):
//! - Parakeet v2 (VoiceStudio dictation stream, CPU): ~1.2 s, but "Postgres Creole",
//!   "identity case" for idempotency key, "reddish" for Redis, "Kugikabka" for Kafka.
//! - VoiceStudio's `/v1/audio/transcriptions` (large-v3): word-perfect, but ~38 s per call:
//!   its voice model fills the GPU, so Whisper is swapped in and out every time.
//! - large-v3 int8 kept loaded here: word-perfect, 1.1-1.5 s per answer. CPU int8: ~12 s.
//!
//! The worker runs with VoiceStudio's bundled Python (faster-whisper + CUDA already there) and
//! the large-v3 weights VoiceStudio downloaded. If any of that is missing, or the GPU has no
//! room, `transcribe` fails fast and STT keeps Parakeet's text, so speech never dead-ends.

use crate::error::{AppError, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::oneshot;

/// Model folders tried in order, under VoiceStudio's Hugging Face cache.
const MODELS: &[&str] = &[
    "models--Systran--faster-whisper-large-v3",
    "models--deepdml--faster-whisper-large-v3-turbo-ct2",
    "models--Systran--faster-distil-whisper-large-v3",
];
/// Long answers are cut into segments of up to ~20 s; one decodes in ~1-2 s once loaded.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WhisperStatus {
    Off { reason: String },
    Loading,
    Ready { model: String },
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<String>>>>>;

struct Server {
    stdin: tokio::sync::Mutex<ChildStdin>,
    pending: Pending,
    next: AtomicU64,
    _child: Child,
}

static STATUS: Mutex<Option<WhisperStatus>> = Mutex::new(None);
static SERVER: Mutex<Option<Arc<Server>>> = Mutex::new(None);
/// Folder holding `whisper_server.py`; set once at app start.
static SCRIPT_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_script_dir(dir: PathBuf) {
    *SCRIPT_DIR.lock().unwrap() = Some(dir);
}

pub fn status() -> WhisperStatus {
    STATUS.lock().unwrap().clone().unwrap_or(WhisperStatus::Off { reason: "not started".into() })
}

fn set_status(s: WhisperStatus) {
    *STATUS.lock().unwrap() = Some(s);
}

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).map(PathBuf::from)
}

pub(crate) fn voicestudio_python() -> Option<PathBuf> {
    let p = env_dir("APPDATA")?.join("VoiceStudio/runtime/project/.venv/Scripts/python.exe");
    p.is_file().then_some(p)
}

/// The first snapshot of the first model VoiceStudio has downloaded.
fn model_dir() -> Option<PathBuf> {
    let cache = env_dir("LOCALAPPDATA")?.join("OmniVoice/hf_cache");
    MODELS.iter().find_map(|m| {
        let snaps = std::fs::read_dir(cache.join(m).join("snapshots")).ok()?;
        snaps.filter_map(|e| e.ok()).map(|e| e.path()).find(|p| p.join("model.bin").is_file())
    })
}

/// Starts the worker in the background unless it is running or loading. Cheap to call often.
pub fn start() {
    {
        let st = STATUS.lock().unwrap();
        if matches!(*st, Some(WhisperStatus::Loading | WhisperStatus::Ready { .. })) {
            return;
        }
    }
    set_status(WhisperStatus::Loading);
    tokio::spawn(async {
        if let Err(e) = launch().await {
            tracing::warn!("Whisper worker unavailable, using Parakeet: {e}");
            set_status(WhisperStatus::Off { reason: e.to_string() });
        }
    });
}

async fn launch() -> Result<()> {
    let off = |m: &str| AppError::Voice(m.to_string());
    let python = voicestudio_python().ok_or_else(|| off("VoiceStudio's Python runtime not found"))?;
    let model = model_dir().ok_or_else(|| off("no Whisper large-v3 model in VoiceStudio's cache"))?;
    let script = SCRIPT_DIR.lock().unwrap().clone().map(|d| d.join("whisper_server.py")).filter(|p| p.is_file());
    let script = script.ok_or_else(|| off("sidecar/whisper_server.py not found"))?;

    let mut cmd = Command::new(&python);
    // -I: ignore PYTHON* env vars and the user's site dir; only VoiceStudio's venv is used.
    cmd.arg("-I").arg(&script).arg(&model);
    cmd.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    cmd.kill_on_drop(true);
    if let Some(tmp) = scratch_dir() {
        cmd.current_dir(tmp);
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| off(&format!("start Whisper worker: {e}")))?;
    let stdin = child.stdin.take().ok_or_else(|| off("no stdin"))?;
    let mut lines = BufReader::new(child.stdout.take().ok_or_else(|| off("no stdout"))?).lines();

    // Loading large-v3 onto the GPU takes ~10 s, longer on a cold disk.
    let first = tokio::time::timeout(Duration::from_secs(180), lines.next_line())
        .await
        .map_err(|_| off("Whisper worker took too long to load"))?
        .map_err(|e| off(&e.to_string()))?
        .ok_or_else(|| off("Whisper worker exited while loading"))?;
    let hello: Value = serde_json::from_str(&first).unwrap_or_default();
    if hello["ready"] != true {
        return Err(off(hello["error"].as_str().unwrap_or("Whisper worker failed to load")));
    }
    let name = hello["model"].as_str().unwrap_or("whisper").to_string();
    tracing::info!(model = %name, load_ms = hello["load_ms"].as_u64(), "Whisper worker ready");

    let pending: Pending = Default::default();
    let server = Arc::new(Server { stdin: tokio::sync::Mutex::new(stdin), pending: pending.clone(), next: AtomicU64::new(1), _child: child });
    *SERVER.lock().unwrap() = Some(server);
    set_status(WhisperStatus::Ready { model: name });

    tokio::spawn(async move {
        while let Ok(Some(line)) = lines.next_line().await {
            let v: Value = serde_json::from_str(&line).unwrap_or_default();
            let Some(id) = v["id"].as_u64() else { continue };
            let Some(tx) = pending.lock().unwrap().remove(&id) else { continue };
            let res = match v["error"].as_str() {
                Some(e) => Err(AppError::Voice(format!("Whisper: {e}"))),
                None => {
                    tracing::debug!(ms = v["ms"].as_u64(), "whisper segment");
                    Ok(v["text"].as_str().unwrap_or_default().to_string())
                }
            };
            let _ = tx.send(res);
        }
        tracing::warn!("Whisper worker exited");
        *SERVER.lock().unwrap() = None;
        set_status(WhisperStatus::Off { reason: "worker exited".into() });
        // Dropping the senders fails every request still waiting.
        pending.lock().unwrap().clear();
    });
    Ok(())
}

fn scratch_dir() -> Option<PathBuf> {
    let d = std::env::temp_dir().join("onsite-stt");
    std::fs::create_dir_all(&d).ok().map(|_| d)
}

/// Prompt that steers Whisper to the candidate's vocabulary. Whisper reads at most ~224 tokens.
pub fn prompt_for(vocab: &[String]) -> String {
    let mut p = String::from("Technical interview in English.");
    for (i, term) in vocab.iter().enumerate() {
        if p.len() + term.len() + 2 > 600 {
            break;
        }
        p.push_str(if i == 0 { " " } else { ", " });
        p.push_str(term);
    }
    p.push('.');
    p
}

/// Transcribes 16 kHz mono PCM. Fails fast when the worker isn't ready.
pub async fn transcribe(pcm: Vec<i16>, prompt: String) -> Result<String> {
    let server = SERVER.lock().unwrap().clone().ok_or_else(|| AppError::Voice("Whisper worker not ready".into()))?;
    let id = server.next.fetch_add(1, Ordering::SeqCst);
    let dir = scratch_dir().ok_or_else(|| AppError::Voice("no temp dir".into()))?;
    let wav = dir.join(format!("seg-{}-{id}.wav", std::process::id()));
    std::fs::write(&wav, super::vs::wav_bytes(&pcm, super::audio::MIC_RATE))?;
    let _cleanup = Remove(wav.clone());
    let (tx, rx) = oneshot::channel();
    server.pending.lock().unwrap().insert(id, tx);
    let req = serde_json::json!({ "id": id, "wav": wav.to_string_lossy(), "prompt": prompt });
    {
        let mut stdin = server.stdin.lock().await;
        stdin.write_all(format!("{req}\n").as_bytes()).await?;
        stdin.flush().await?;
    }
    match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
        Ok(Ok(res)) => res,
        Ok(Err(_)) => Err(AppError::Voice("Whisper worker exited".into())),
        Err(_) => {
            server.pending.lock().unwrap().remove(&id);
            Err(AppError::Voice("Whisper took too long".into()))
        }
    }
}

struct Remove(PathBuf);

impl Drop for Remove {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Dev builds read `sidecar/` from the repo; installed builds from bundled resources.
pub fn script_dir_for(resource_dir: Option<&Path>) -> PathBuf {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sidecar");
    if cfg!(debug_assertions) && dev.is_dir() {
        return dev;
    }
    resource_dir.map(|d| d.join("sidecar")).unwrap_or(dev)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_lists_terms_within_budget() {
        let p = prompt_for(&["Kafka".into(), "PostgreSQL".into()]);
        assert_eq!(p, "Technical interview in English. Kafka, PostgreSQL.");
        let many: Vec<String> = (0..500).map(|i| format!("Term{i}")).collect();
        assert!(prompt_for(&many).len() <= 602);
    }

    /// Live: needs VoiceStudio running. `ONSITE_TEST_WAV` = a 16 kHz mono WAV.
    /// `cargo test whisper_live -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn whisper_live_next_to_tts() {
        let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        rt.block_on(async {
            set_script_dir(script_dir_for(None));
            let t = std::time::Instant::now();
            start();
            while matches!(status(), WhisperStatus::Loading) {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            println!("status {:?} after {:?}", status(), t.elapsed());
            let wav = std::fs::read(std::env::var("ONSITE_TEST_WAV").unwrap()).unwrap();
            let pcm: Vec<i16> = wav[44..].as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect();
            let prompt = prompt_for(&crate::voice::stt::domain_vocab(&[]));
            for round in 0..2 {
                let t = std::time::Instant::now();
                let text = transcribe(pcm.clone(), prompt.clone()).await.unwrap();
                println!("whisper #{round} {:?}: {text}", t.elapsed());
                let vs = crate::voice::vs::VoiceStudio::new("http://127.0.0.1:3900");
                let voice = vs.resolve_voice("").await.unwrap();
                let t = std::time::Instant::now();
                let mut n = 0;
                vs.speak("Okay, tell me how you would make that retry idempotent.", &voice, 8, 1.0, |p| n += p.len()).await.unwrap();
                println!("tts with whisper loaded {:?}, {} samples", t.elapsed(), n);
            }
        });
    }

    #[test]
    fn transcribe_fails_fast_when_not_started() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        assert!(rt.block_on(transcribe(vec![0; 160], String::new())).is_err());
    }
}
