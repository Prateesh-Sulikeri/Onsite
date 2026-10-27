//! Updates: Onsite's versions come from its GitHub releases. Any listed version can be
//! installed (newer or older) by downloading that release's installer and running it.
//! VoiceStudio's installed and latest versions are only shown; it updates itself.
//! Nothing here runs unless the user opens Updates.

use crate::error::{AppError, Result};
use crate::AppState;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::AsyncWriteExt;

const ONSITE_RELEASES: &str = "https://api.github.com/repos/Prateesh-Sulikeri/Onsite/releases?per_page=30";
const ONSITE_TAG: &str = "https://api.github.com/repos/Prateesh-Sulikeri/Onsite/releases/tags/";
const VOICESTUDIO_LATEST: &str = "https://api.github.com/repos/debpalash/VoiceStudio/releases/latest";

/// (major, minor, patch, is_release). A pre-release sorts below its release.
pub fn version_key(v: &str) -> Option<(u64, u64, u64, bool)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match v.split_once('-') {
        Some((c, _)) => (c, true),
        None => (v, false),
    };
    let mut n = core.split('.').map(|x| x.parse::<u64>().ok());
    Some((n.next()??, n.next().flatten().unwrap_or(0), n.next().flatten().unwrap_or(0), !pre))
}

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub published_at: String,
    pub prerelease: bool,
    pub notes: String,
    /// The Windows installer, when the release has one.
    pub installer: Option<String>,
    pub size: u64,
    /// -1 older than this app, 0 this version, 1 newer.
    pub compared: i8,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    /// Newest first; drafts are skipped.
    pub releases: Vec<Release>,
    /// The newest stable release above `current`, if any.
    pub latest: Option<String>,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Onsite/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .unwrap_or_default()
}

async fn get_json(url: &str) -> Result<Value> {
    let resp = client().get(url).header("Accept", "application/vnd.github+json").timeout(Duration::from_secs(20)).send().await.map_err(|e| {
        tracing::warn!("update check: {e}");
        AppError::Invalid("Couldn't reach GitHub. Check your internet connection.".into())
    })?;
    match resp.status().as_u16() {
        200 => resp.json().await.map_err(|_| AppError::Invalid("GitHub sent something unreadable.".into())),
        403 | 429 => Err(AppError::Invalid("GitHub's rate limit was hit. Try again in an hour.".into())),
        404 => Err(AppError::NotFound("No releases found.".into())),
        s => Err(AppError::Invalid(format!("GitHub returned HTTP {s}."))),
    }
}

fn installer_asset(r: &Value) -> Option<&Value> {
    r["assets"].as_array()?.iter().find(|a| a["name"].as_str().is_some_and(|n| n.to_lowercase().ends_with("-setup.exe")))
}

pub fn parse_releases(v: &Value, current: &str) -> UpdateInfo {
    let cur = version_key(current);
    let mut releases: Vec<Release> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["draft"].as_bool() != Some(true))
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?.to_string();
            let key = version_key(&tag)?;
            let asset = installer_asset(r);
            Some(Release {
                name: r["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(&tag).to_string(),
                published_at: r["published_at"].as_str().unwrap_or_default().to_string(),
                prerelease: r["prerelease"].as_bool().unwrap_or(false),
                notes: r["body"].as_str().unwrap_or_default().chars().take(4000).collect(),
                installer: asset.and_then(|a| a["name"].as_str()).map(String::from),
                size: asset.and_then(|a| a["size"].as_u64()).unwrap_or(0),
                compared: cur.map_or(1, |c| key.cmp(&c) as i8),
                tag,
            })
        })
        .collect();
    releases.sort_by_key(|r| std::cmp::Reverse(version_key(&r.tag)));
    let latest = releases.iter().find(|r| r.compared > 0 && !r.prerelease && r.installer.is_some()).map(|r| r.tag.clone());
    UpdateInfo { current: current.to_string(), releases, latest }
}

pub async fn check(app: &AppHandle) -> Result<UpdateInfo> {
    let current = app.package_info().version.to_string();
    let v = match get_json(ONSITE_RELEASES).await {
        Err(AppError::NotFound(_)) => Value::Array(vec![]),
        other => other?,
    };
    Ok(parse_releases(&v, &current))
}

#[derive(Clone, Serialize)]
struct Progress {
    tag: String,
    received: u64,
    total: u64,
}

/// Downloads `tag`'s installer, starts it, and quits so it can replace the app.
/// The release is looked up again here; the UI only names the tag.
pub async fn install(app: &AppHandle, tag: &str) -> Result<()> {
    let state = app.state::<AppState>();
    if !state.rounds.lock().await.is_empty() {
        return Err(AppError::Invalid("Finish or leave the interview first.".into()));
    }
    if !state.research.lock().unwrap().is_empty() {
        return Err(AppError::Invalid("A research job is running. Wait for it or cancel it first.".into()));
    }
    let r = get_json(&format!("{ONSITE_TAG}{tag}")).await?;
    let asset = installer_asset(&r).ok_or_else(|| AppError::NotFound("That version has no Windows installer.".into()))?;
    let url = asset["browser_download_url"].as_str().unwrap_or_default();
    let name = asset["name"].as_str().unwrap_or("Onsite-setup.exe");
    // Only GitHub's own download host; the name must be a plain file name.
    if !url.starts_with("https://github.com/") || name.contains(['/', '\\']) {
        return Err(AppError::Invalid("That download link doesn't look like a GitHub release.".into()));
    }
    let dir: PathBuf = std::env::temp_dir().join("onsite-update");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    let resp = client().get(url).timeout(Duration::from_secs(30 * 60)).send().await.map_err(|e| AppError::Invalid(format!("Download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Invalid(format!("Download failed: HTTP {}.", resp.status().as_u16())));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&path).await?;
    let mut body = resp.bytes_stream();
    let mut received = 0u64;
    let mut last = 0u64;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| AppError::Invalid(format!("Download stopped: {e}")))?;
        file.write_all(&chunk).await?;
        received += chunk.len() as u64;
        if received - last > 256 * 1024 || received == total {
            last = received;
            let _ = app.emit("update_progress", Progress { tag: tag.to_string(), received, total });
        }
    }
    file.flush().await?;
    drop(file);
    if total > 0 && received != total {
        return Err(AppError::Invalid("The download was cut short. Try again.".into()));
    }
    tracing::info!(%tag, path = %path.display(), "starting installer");
    std::process::Command::new(&path).spawn().map_err(|e| AppError::Invalid(format!("Couldn't start the installer: {e}")))?;
    // Give the installer a moment to open, then get out of its way.
    let handle = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        handle.exit(0);
    });
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct VoiceStudioVersion {
    /// From the running backend, else from its install folder.
    pub installed: Option<String>,
    pub running: bool,
    pub latest: Option<String>,
    pub latest_date: String,
    pub update_available: bool,
    /// Why `latest` is missing, if it is.
    pub note: String,
}

pub fn installed_voicestudio() -> Option<String> {
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs/VoiceStudio/resources/pyproject.toml");
    let text = std::fs::read_to_string(base).ok()?;
    text.lines().find_map(|l| l.trim().strip_prefix("version")?.trim().strip_prefix('=').map(|v| v.trim().trim_matches('"').to_string()))
}

pub async fn voicestudio(state: &AppState) -> VoiceStudioVersion {
    let vs = crate::voice::vs::VoiceStudio::new(&state.settings().voice.base_url);
    let health = vs.health().await.ok();
    let running = health.is_some();
    let installed = health.as_ref().and_then(|h| h["version"].as_str().map(String::from)).or_else(installed_voicestudio);
    let (latest, latest_date, note) = match get_json(VOICESTUDIO_LATEST).await {
        Ok(v) => (v["tag_name"].as_str().map(String::from), v["published_at"].as_str().unwrap_or_default().to_string(), String::new()),
        Err(e) => (None, String::new(), e.to_string()),
    };
    let update_available = matches!((installed.as_deref().and_then(version_key), latest.as_deref().and_then(version_key)), (Some(i), Some(l)) if l > i);
    VoiceStudioVersion { installed, running, latest, latest_date, update_available, note }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions_compare() {
        assert!(version_key("v0.2.0") > version_key("0.1.9"));
        assert!(version_key("v1.0.0-beta.1") < version_key("1.0.0"));
        assert_eq!(version_key("v2"), Some((2, 0, 0, true)));
        assert_eq!(version_key("latest"), None);
    }

    #[test]
    fn releases_sorted_and_marked() {
        let asset = |n: &str| json!([{"name": n, "size": 5, "browser_download_url": "https://github.com/x"}]);
        let v = json!([
            {"tag_name": "v0.1.0", "assets": asset("Onsite_0.1.0_x64-setup.exe")},
            {"tag_name": "v0.3.0-rc1", "prerelease": true, "assets": asset("Onsite_0.3.0_x64-setup.exe")},
            {"tag_name": "v0.2.0", "body": "Fixes", "assets": asset("Onsite_0.2.0_x64-setup.exe")},
            {"tag_name": "v0.2.5", "assets": []},
            {"tag_name": "v9.0.0", "draft": true, "assets": asset("x-setup.exe")}
        ]);
        let info = parse_releases(&v, "0.1.0");
        let tags: Vec<_> = info.releases.iter().map(|r| (r.tag.as_str(), r.compared)).collect();
        assert_eq!(tags, [("v0.3.0-rc1", 1), ("v0.2.5", 1), ("v0.2.0", 1), ("v0.1.0", 0)]);
        // Pre-releases and releases without an installer are not offered as "the update".
        assert_eq!(info.latest.as_deref(), Some("v0.2.0"));
        assert_eq!(parse_releases(&v, "0.2.0").releases.last().unwrap().compared, -1);
    }
}
