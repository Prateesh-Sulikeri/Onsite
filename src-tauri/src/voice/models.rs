//! Built-in voice engine models: pinned GitHub release assets, downloaded once, verified, installed atomically.
//!
//! Layout: `<app data>/models/<id>/` is a finished install (only this counts). Work happens in
//! `<app data>/models/.partial/<id>/` and ends with one rename. TODO: resume (HTTP Range) and cancel.

use crate::error::{AppError, Result};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// One pinned model: a release asset of k2-fsa/sherpa-onnx, checked by size and SHA-256.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Pin {
    /// Folder name under `models/` (the tarball's own top folder).
    pub id: &'static str,
    pub label: &'static str,
    pub purpose: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// Size on disk once unpacked; used for the free-space check.
    pub unpacked: u64,
    /// Files that must exist in the install.
    #[serde(skip)]
    pub files: &'static [&'static str],
}

pub const POCKET: Pin = Pin {
    id: "sherpa-onnx-pocket-tts-int8-2026-01-26",
    label: "Pocket TTS",
    purpose: "The interviewer's voice",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/sherpa-onnx-pocket-tts-int8-2026-01-26.tar.bz2",
    bytes: 98_336_520,
    sha256: "2f3b88823cbbb9bf0b2477ec8ae7b3fec417b3a87b6bb5f256dba66f2ad967cb",
    unpacked: 195_000_000,
    files: &[
        "lm_flow.int8.onnx", "lm_main.int8.onnx", "encoder.onnx", "decoder.int8.onnx", "text_conditioner.onnx", "vocab.json",
        "token_scores.json",
    ],
};

pub const PARAKEET: Pin = Pin {
    id: "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8",
    label: "Parakeet TDT 0.6B v2",
    purpose: "Live captions and your answers",
    url: "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8.tar.bz2",
    bytes: 482_468_385,
    sha256: "157c157bc51155e03e37d2466522a3a737dd9c72bb25f36eb18912964161e1ad",
    unpacked: 631_000_000,
    files: &["encoder.int8.onnx", "decoder.int8.onnx", "joiner.int8.onnx", "tokens.txt"],
};

pub const PINS: [Pin; 2] = [POCKET, PARAKEET];

pub fn pin(id: &str) -> Option<Pin> {
    PINS.iter().copied().find(|p| p.id == id)
}

pub fn root(data_dir: &Path) -> PathBuf {
    data_dir.join("models")
}

pub fn path(data_dir: &Path, p: &Pin) -> PathBuf {
    root(data_dir).join(p.id)
}

fn partial(data_dir: &Path, p: &Pin) -> PathBuf {
    root(data_dir).join(".partial").join(p.id)
}

/// A folder only appears under its final name after the whole install, but check the files too
/// (a user may have deleted some).
pub fn installed(data_dir: &Path, p: &Pin) -> bool {
    let dir = path(data_dir, p);
    p.files.iter().all(|f| dir.join(f).is_file())
}

pub fn all_installed(data_dir: &Path) -> bool {
    PINS.iter().all(|p| installed(data_dir, p))
}

/// Ids being downloaded right now. One download per model.
static ACTIVE: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);

pub fn downloading(id: &str) -> bool {
    ACTIVE.lock().unwrap().as_ref().is_some_and(|s| s.contains(id))
}

struct Claim(&'static str);

impl Claim {
    fn take(id: &'static str) -> Option<Self> {
        // Insert first and release the lock: dropping an unused `Claim` locks it again.
        let fresh = ACTIVE.lock().unwrap().get_or_insert_with(Default::default).insert(id);
        if fresh {
            Some(Claim(id))
        } else {
            None
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(s) = ACTIVE.lock().unwrap().as_mut() {
            s.remove(self.0);
        }
    }
}

/// Sent to the UI as the `voice_model` event.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Progress {
    pub id: String,
    /// downloading, verifying, unpacking, done, error
    pub state: &'static str,
    pub done: u64,
    pub total: u64,
    pub message: String,
}

fn progress(id: &str, state: &'static str, done: u64, total: u64, message: &str) -> Progress {
    Progress { id: id.into(), state, done, total, message: message.into() }
}

fn gb(n: u64) -> String {
    format!("{:.1} GB", n as f64 / 1e9)
}

/// github.com, and the CDN GitHub redirects release downloads to. Nothing else, and only over https.
pub fn allowed_host(url: &reqwest::Url) -> bool {
    url.scheme() == "https" && url.host_str().is_some_and(|h| h == "github.com" || h.ends_with(".githubusercontent.com"))
}

fn client() -> Result<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(|a| {
        if allowed_host(a.url()) && a.previous().len() < 5 {
            a.follow()
        } else {
            a.error("The download was redirected to a site Onsite doesn't allow.")
        }
    });
    reqwest::Client::builder()
        .redirect(policy)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .map_err(|e| AppError::Voice(e.to_string()))
}

fn net_error(e: impl std::fmt::Display) -> AppError {
    tracing::warn!("model download: {e}");
    AppError::Voice("Couldn't reach GitHub. Check your connection and press Retry.".into())
}

/// Free bytes on the drive holding `dir` (nearest existing parent). None when unknown.
fn free_space(dir: &Path) -> Option<u64> {
    let mut d = dir;
    while !d.exists() {
        d = d.parent()?;
    }
    fs4::available_space(d).ok()
}

/// Checks free space for the download plus the unpacked copy (+ 10 %).
fn check_space(data_dir: &Path, p: &Pin) -> Result<()> {
    let need = (p.bytes + p.unpacked) / 10 * 11;
    match free_space(&root(data_dir)) {
        Some(free) if free < need => {
            Err(AppError::Voice(format!("Needs {} free to install {}. You have {}. Free some space and press Retry.", gb(need), p.label, gb(free))))
        }
        _ => Ok(()),
    }
}

/// Downloads, verifies and installs `p`. Progress goes to `on`. Quietly does nothing if this
/// model is already downloading (the caller's UI follows the same events).
pub async fn install(data_dir: &Path, p: Pin, on: impl Fn(Progress) + Send + Sync + 'static) -> Result<()> {
    if installed(data_dir, &p) {
        on(progress(p.id, "done", p.bytes, p.bytes, ""));
        return Ok(());
    }
    let Some(_claim) = Claim::take(p.id) else { return Ok(()) };
    let res = install_inner(data_dir, p, &on).await;
    match &res {
        Ok(()) => on(progress(p.id, "done", p.bytes, p.bytes, "")),
        Err(e) => on(progress(p.id, "error", 0, p.bytes, &e.to_string())),
    }
    res
}

async fn install_inner(data_dir: &Path, p: Pin, on: &(impl Fn(Progress) + Send + Sync)) -> Result<()> {
    check_space(data_dir, &p)?;
    let work = partial(data_dir, &p);
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work)?;
    let archive = work.join("archive.tar.bz2");

    let url = reqwest::Url::parse(p.url).map_err(|e| AppError::Voice(e.to_string()))?;
    if !allowed_host(&url) {
        return Err(AppError::Voice("Model downloads only come from github.com.".into()));
    }
    let resp = client()?.get(url).send().await.map_err(net_error)?;
    if !resp.status().is_success() {
        return Err(net_error(format!("HTTP {}", resp.status())));
    }
    let mut out = std::fs::File::create(&archive)?;
    let (mut hasher, mut done, mut last) = (Sha256::new(), 0u64, Instant::now() - Duration::from_secs(1));
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(net_error)?;
        hasher.update(&chunk);
        out.write_all(&chunk)?;
        done += chunk.len() as u64;
        if last.elapsed() >= Duration::from_millis(200) {
            last = Instant::now();
            on(progress(p.id, "downloading", done, p.bytes, ""));
        }
    }
    out.flush()?;
    drop(out);

    on(progress(p.id, "verifying", done, p.bytes, ""));
    let sum = format!("{:x}", hasher.finalize());
    if done != p.bytes || sum != p.sha256 {
        let _ = std::fs::remove_dir_all(&work);
        tracing::warn!(id = p.id, done, expected = p.bytes, %sum, "model download failed its check");
        return Err(AppError::Voice("The download was damaged. Press Retry.".into()));
    }

    on(progress(p.id, "unpacking", done, p.bytes, ""));
    let (work2, archive2) = (work.clone(), archive.clone());
    tokio::task::spawn_blocking(move || unpack(&archive2, &work2.join("unpacked")))
        .await
        .map_err(|e| AppError::Voice(e.to_string()))??;
    let _ = std::fs::remove_file(&archive);

    // The tarball holds one folder; that folder is the install.
    let inner = std::fs::read_dir(work.join("unpacked"))?.filter_map(|e| e.ok()).map(|e| e.path()).find(|e| e.is_dir()).ok_or_else(|| AppError::Voice("The model file was empty. Press Retry.".into()))?;
    if let Some(missing) = p.files.iter().find(|f| !inner.join(f).is_file()) {
        let _ = std::fs::remove_dir_all(&work);
        return Err(AppError::Voice(format!("The model file is missing {missing}. Press Retry.")));
    }
    let dest = path(data_dir, &p);
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&inner, &dest)?;
    let _ = std::fs::remove_dir_all(&work);
    tracing::info!(id = p.id, "voice model installed");
    Ok(())
}

fn unpack(archive: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    let file = std::fs::File::open(archive)?;
    // `unpack` skips entries that would land outside `to`.
    tar::Archive::new(bzip2::read::BzDecoder::new(file)).unpack(to).map_err(|e| AppError::Voice(format!("Couldn't unpack the model: {e}. Press Retry.")))
}

/// Removes a model and any half-done download.
pub fn delete(data_dir: &Path, p: &Pin) -> Result<()> {
    if downloading(p.id) {
        return Err(AppError::Voice("Wait for the download to finish first.".into()));
    }
    let _ = std::fs::remove_dir_all(partial(data_dir, p));
    match std::fs::remove_dir_all(path(data_dir, p)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// What the UI shows for one model.
#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    #[serde(flatten)]
    pub pin: Pin,
    pub installed: bool,
    pub downloading: bool,
}

pub fn status(data_dir: &Path) -> Vec<ModelInfo> {
    PINS.iter().map(|p| ModelInfo { pin: *p, installed: installed(data_dir, p), downloading: downloading(p.id) }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_well_formed() {
        for p in PINS {
            assert!(p.url.starts_with("https://github.com/k2-fsa/sherpa-onnx/releases/download/"), "{}", p.id);
            assert!(p.url.ends_with(&format!("{}.tar.bz2", p.id)));
            assert_eq!(p.sha256.len(), 64, "{}", p.id);
            assert!(p.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(p.unpacked > p.bytes / 2);
        }
    }

    #[test]
    fn only_github_hosts() {
        let ok = |u: &str| allowed_host(&reqwest::Url::parse(u).unwrap());
        assert!(ok("https://github.com/k2-fsa/x"));
        assert!(ok("https://release-assets.githubusercontent.com/a/b"));
        assert!(!ok("http://github.com/k2-fsa/x"));
        assert!(!ok("https://evilgithub.com/x"));
        assert!(!ok("https://github.com.evil.io/x"));
        assert!(!ok("https://huggingface.co/x"));
    }

    #[test]
    fn install_state_follows_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!installed(dir.path(), &PARAKEET));
        let d = path(dir.path(), &PARAKEET);
        std::fs::create_dir_all(&d).unwrap();
        for f in PARAKEET.files {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        assert!(installed(dir.path(), &PARAKEET));
        assert!(!all_installed(dir.path()));
        delete(dir.path(), &PARAKEET).unwrap();
        assert!(!installed(dir.path(), &PARAKEET));
    }

    #[test]
    fn one_claim_per_model() {
        let a = Claim::take("x-test-model").unwrap();
        assert!(Claim::take("x-test-model").is_none());
        drop(a);
        assert!(Claim::take("x-test-model").is_some());
    }

    #[test]
    fn unpacks_a_tarball() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("a.tar.bz2");
        let enc = bzip2::write::BzEncoder::new(std::fs::File::create(&src).unwrap(), bzip2::Compression::fast());
        let mut b = tar::Builder::new(enc);
        let mut h = tar::Header::new_gnu();
        h.set_size(2);
        h.set_mode(0o644);
        h.set_cksum();
        b.append_data(&mut h, "top/f.txt", &b"hi"[..]).unwrap();
        b.into_inner().unwrap().finish().unwrap();
        unpack(&src, &dir.path().join("out")).unwrap();
        assert_eq!(std::fs::read(dir.path().join("out/top/f.txt")).unwrap(), b"hi");
    }
}
