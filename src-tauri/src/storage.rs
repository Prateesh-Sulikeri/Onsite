//! Settings → Storage: what Onsite keeps on disk, limits on it, and cleanup. Also the
//! custom background image, which lives in the app data folder.

use crate::error::{AppError, Result};
use crate::store::StoreCounts;
use crate::AppState;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const LIMITS_KEY: &str = "storage_limits";
const BACKDROP_EXTS: [&str; 3] = ["jpg", "png", "webp"];
const BACKDROP_MAX: usize = 12 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Limits {
    /// Keep only the newest N finished rounds (with their feedback). 0 = keep all.
    pub keep_rounds: u32,
    /// Delete finished rounds older than N days. 0 = keep forever.
    pub keep_days: u32,
    /// The log file is cleared when it grows past this.
    pub log_max_mb: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self { keep_rounds: 0, keep_days: 0, log_max_mb: 20 }
    }
}

/// Windows: %LOCALAPPDATA%\Onsite\onsite.log (the app has no console window).
/// macOS: ~/Library/Logs/Onsite. Linux: $XDG_STATE_HOME/Onsite, else ~/.local/state/Onsite.
pub fn log_path() -> PathBuf {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    let base = if cfg!(windows) {
        var("LOCALAPPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|h| h.join("Library").join("Logs"))
    } else {
        var("XDG_STATE_HOME").or_else(|| var("HOME").map(|h| h.join(".local").join("state")))
    };
    base.unwrap_or_else(std::env::temp_dir).join("Onsite").join("onsite.log")
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Total bytes under a folder (0 if it doesn't exist). Symlinks aren't followed.
pub fn dir_size(p: &Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.filter_map(|e| e.ok())
        .map(|e| match e.file_type() {
            Ok(t) if t.is_dir() => dir_size(&e.path()),
            Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
            _ => 0,
        })
        .sum()
}

#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub label: String,
    pub path: String,
    pub bytes: u64,
    /// Onsite's own data; the rest belongs to VoiceStudio or the system.
    pub ours: bool,
    pub note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageInfo {
    pub data_dir: String,
    pub items: Vec<Item>,
    pub counts: StoreCounts,
    pub limits: Limits,
    /// Bytes in Onsite's own items.
    pub total: u64,
}

pub fn limits(state: &AppState) -> Limits {
    state.store.lock().unwrap().get_setting(LIMITS_KEY).ok().flatten().unwrap_or_default()
}

pub fn info(state: &AppState) -> Result<StorageInfo> {
    let d = &state.data_dir;
    let db = d.join("onsite.db");
    let db_bytes = file_size(&db) + file_size(&d.join("onsite.db-wal")) + file_size(&d.join("onsite.db-shm"));
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let tmp = std::env::temp_dir();
    let backdrop = backdrop_file(d);
    let item = |label: &str, path: &Path, bytes: u64, ours: bool, note: &str| Item {
        label: label.into(),
        path: path.display().to_string(),
        bytes,
        ours,
        note: note.into(),
    };
    let mut items = vec![
        item("Database", &db, db_bytes, true, "Interviews, transcripts, feedback, dossiers, settings, API keys."),
        item("Dossier files", &d.join("dossiers"), dir_size(&d.join("dossiers")), true, "A JSON copy of each dossier."),
        item("Background image", backdrop.as_deref().unwrap_or(&d.join("backdrop.jpg")), backdrop.as_deref().map_or(0, file_size), true, "Your custom background, if set."),
        item("Log", &log_path(), file_size(&log_path()), true, "Errors and timings, for debugging. No transcripts."),
        item("Voice models", &crate::voice::models::root(d), dir_size(&crate::voice::models::root(d)), true, "Built-in voice engine models (Pocket TTS, Parakeet). Delete them in Settings > Voice."),
        item("Temporary files", &tmp.join("onsite-update"), dir_size(&tmp.join("onsite-update")) + dir_size(&tmp.join("onsite-stt")), true, "Downloaded installers and speech scratch files."),
    ];
    if let Some(l) = &local {
        let cache = l.join("OmniVoice").join("hf_cache");
        items.push(item("VoiceStudio models", &cache, dir_size(&cache), false, "Voice and speech models. Managed by VoiceStudio."));
    }
    let total = items.iter().filter(|i| i.ours).map(|i| i.bytes).sum();
    let counts = state.store.lock().unwrap().counts()?;
    Ok(StorageInfo { data_dir: d.display().to_string(), items, counts, limits: limits(state), total })
}

pub fn set_limits(state: &AppState, l: Limits) -> Result<usize> {
    let l = Limits { keep_rounds: l.keep_rounds.min(100_000), keep_days: l.keep_days.min(36_500), log_max_mb: l.log_max_mb.clamp(1, 1024) };
    state.store.lock().unwrap().set_setting(LIMITS_KEY, &l)?;
    enforce(state)
}

/// Deletes rounds past the limits and clears an oversized log. Returns rounds deleted.
pub fn enforce(state: &AppState) -> Result<usize> {
    let l = limits(state);
    let before = (l.keep_days > 0).then(|| (chrono::Utc::now() - chrono::Duration::days(l.keep_days as i64)).to_rfc3339());
    let store = state.store.lock().unwrap();
    let ids = store.rounds_over_limit(l.keep_rounds, before.as_deref())?;
    for &id in &ids {
        store.delete_round(id)?;
    }
    if !ids.is_empty() {
        let day_ago = (chrono::Utc::now() - chrono::Duration::days(1)).to_rfc3339();
        store.delete_empty_loops(&day_ago)?;
        tracing::info!(n = ids.len(), "storage limits: deleted old rounds");
    }
    drop(store);
    if file_size(&log_path()) > l.log_max_mb as u64 * 1024 * 1024 {
        clear_log()?;
    }
    Ok(ids.len())
}

/// Empties the log. The writer keeps appending to the same file.
pub fn clear_log() -> Result<()> {
    match std::fs::OpenOptions::new().write(true).open(log_path()) {
        Ok(f) => Ok(f.set_len(0)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub fn clear_temp() -> Result<u64> {
    let tmp = std::env::temp_dir();
    let mut freed = 0;
    for d in [tmp.join("onsite-update"), tmp.join("onsite-stt")] {
        freed += dir_size(&d);
        // A file still in use (e.g. the STT scratch while listening) is just left.
        let _ = std::fs::remove_dir_all(&d);
    }
    Ok(freed)
}

pub fn open_folder(path: &Path) -> Result<()> {
    if !path.exists() {
        return Err(AppError::NotFound("That folder doesn't exist yet.".into()));
    }
    let opener = if cfg!(windows) {
        "explorer.exe"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener).arg(path).spawn()?;
    Ok(())
}

// ---------- background image ----------

fn backdrop_file(dir: &Path) -> Option<PathBuf> {
    BACKDROP_EXTS.iter().map(|e| dir.join(format!("backdrop.{e}"))).find(|p| p.is_file())
}

/// Image type from the first bytes; anything else is refused.
pub fn image_ext(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => Some("webp"),
        _ => None,
    }
}

/// Saves a background image sent as base64 (the UI shrinks it first).
pub fn set_backdrop(state: &AppState, data: &str) -> Result<()> {
    let b64 = data.split_once(',').map_or(data, |(_, b)| b);
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64.trim()).map_err(|_| AppError::Invalid("That image couldn't be read.".into()))?;
    if bytes.len() > BACKDROP_MAX {
        return Err(AppError::Invalid("That image is too large. Pick one under 12 MB.".into()));
    }
    let ext = image_ext(&bytes).ok_or_else(|| AppError::Invalid("Use a JPG, PNG or WebP image.".into()))?;
    clear_backdrop(state)?;
    std::fs::write(state.data_dir.join(format!("backdrop.{ext}")), bytes)?;
    Ok(())
}

/// The saved image as a data URL, if there is one.
pub fn backdrop(state: &AppState) -> Result<Option<String>> {
    let Some(p) = backdrop_file(&state.data_dir) else { return Ok(None) };
    let bytes = std::fs::read(&p)?;
    let mime = match image_ext(&bytes) {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        _ => "image/jpeg",
    };
    Ok(Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))))
}

pub fn clear_backdrop(state: &AppState) -> Result<()> {
    for e in BACKDROP_EXTS {
        let p = state.data_dir.join(format!("backdrop.{e}"));
        if p.is_file() {
            std::fs::remove_file(p)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_types_from_bytes() {
        assert_eq!(image_ext(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(image_ext(b"\x89PNG\r\n"), Some("png"));
        assert_eq!(image_ext(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(image_ext(b"<svg"), None);
        assert_eq!(image_ext(b""), None);
    }

    #[test]
    fn dir_size_counts_nested_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir(d.path().join("a")).unwrap();
        std::fs::write(d.path().join("a/x"), [0u8; 10]).unwrap();
        std::fs::write(d.path().join("y"), [0u8; 5]).unwrap();
        assert_eq!(dir_size(d.path()), 15);
        assert_eq!(dir_size(&d.path().join("missing")), 0);
    }
}
