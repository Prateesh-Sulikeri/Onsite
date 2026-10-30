//! Updates: Onsite's versions come from its GitHub releases. Any listed version can be
//! installed (newer or older) by downloading that release's package for this system: the NSIS
//! installer on Windows (run, then quit), the disk image on macOS (opened for the user to drag),
//! the AppImage on Linux (swapped in place, then restarted) or else the .deb (saved to Downloads).
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

/// (major, minor, patch, is_release, pre-release number). A pre-release sorts below its release and
/// pre-releases of one version sort by their last number: `0.1.3-exp.2` > `0.1.3-exp.1`, `0.1.3` > `0.1.3-exp.9`.
pub fn version_key(v: &str) -> Option<(u64, u64, u64, bool, u64)> {
    let v = v.trim().trim_start_matches(['v', 'V']);
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let mut n = core.split('.').map(|x| x.parse::<u64>().ok());
    let num = pre.map_or(0, |p| {
        let tail = p.trim_end_matches(|c: char| !c.is_ascii_digit());
        let digits: String = tail.chars().rev().take_while(char::is_ascii_digit).collect::<Vec<_>>().into_iter().rev().collect();
        digits.parse().unwrap_or(0)
    });
    Some((n.next()??, n.next().flatten().unwrap_or(0), n.next().flatten().unwrap_or(0), pre.is_none(), num))
}

/// Which releases `latest` may name. Stable skips pre-releases; Experimental takes the newest of all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Experimental,
}

impl Channel {
    pub fn parse(s: &str) -> Self {
        if s == "experimental" { Self::Experimental } else { Self::Stable }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Experimental => "experimental",
        }
    }
    /// An `-exp` build starts on Experimental; every other install on Stable.
    pub fn default_for(version: &str) -> Self {
        if version.contains("-exp") { Self::Experimental } else { Self::Stable }
    }
}

const CHANNEL_KEY: &str = "update_channel";

pub fn channel(state: &AppState, current: &str) -> Channel {
    match state.store.lock().unwrap().get_setting::<String>(CHANNEL_KEY) {
        Ok(Some(s)) => Channel::parse(&s),
        _ => Channel::default_for(current),
    }
}

pub fn set_channel(state: &AppState, c: Channel) -> Result<()> {
    state.store.lock().unwrap().set_setting(CHANNEL_KEY, &c.as_str())
}

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub tag: String,
    pub name: String,
    pub published_at: String,
    pub prerelease: bool,
    pub notes: String,
    /// The package for this system, when the release has one.
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
    /// The newest release above `current` on this channel, if any.
    pub latest: Option<String>,
    /// "stable" or "experimental".
    pub channel: String,
    /// What this system downloads, e.g. "Windows installer".
    pub package: String,
    /// What installing does here, for the confirm dialog.
    pub how: String,
}

/// The kind of release file that installs on this machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Package {
    Nsis,
    Dmg,
    /// Running from an AppImage ($APPIMAGE is set): the new one replaces it.
    AppImage,
    Deb,
}

impl Package {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Nsis
        } else if cfg!(target_os = "macos") {
            Self::Dmg
        } else if std::env::var_os("APPIMAGE").is_some() {
            Self::AppImage
        } else {
            Self::Deb
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Nsis => "Windows installer",
            Self::Dmg => "Mac disk image",
            Self::AppImage => "AppImage",
            Self::Deb => ".deb package",
        }
    }

    fn how(self) -> &'static str {
        match self {
            Self::Nsis => "downloads the installer from GitHub, then closes so it can run",
            Self::Dmg => "downloads the disk image from GitHub and opens it; drag Onsite into Applications to replace this one",
            Self::AppImage => "downloads the new AppImage from GitHub, swaps it in for this one and restarts",
            Self::Deb => "downloads the .deb package from GitHub to your Downloads folder for you to install",
        }
    }

    /// Release files are named like `Onsite_0.2.0_x64-setup.exe`, `Onsite_0.2.0_universal.dmg`,
    /// `Onsite_0.2.0_amd64.AppImage`, `Onsite_0.2.0_amd64.deb`.
    pub fn matches(self, name: &str, arch: &str) -> bool {
        let n = name.to_lowercase();
        let linux_arch = || match arch {
            "x86_64" => n.contains("amd64") || n.contains("x86_64"),
            "aarch64" => n.contains("arm64") || n.contains("aarch64"),
            _ => false,
        };
        match self {
            Self::Nsis => n.ends_with("-setup.exe"),
            Self::Dmg => n.ends_with(".dmg") && (n.contains("universal") || n.contains(if arch == "aarch64" { "aarch64" } else { "x64" })),
            Self::AppImage => n.ends_with(".appimage") && linux_arch(),
            Self::Deb => n.ends_with(".deb") && linux_arch(),
        }
    }
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

fn installer_asset(r: &Value, pkg: Package) -> Option<&Value> {
    let arch = std::env::consts::ARCH;
    r["assets"].as_array()?.iter().find(|a| a["name"].as_str().is_some_and(|n| pkg.matches(n, arch)))
}

pub fn parse_releases(v: &Value, current: &str, pkg: Package, channel: Channel) -> UpdateInfo {
    let cur = version_key(current);
    let mut releases: Vec<Release> = v
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["draft"].as_bool() != Some(true))
        .filter_map(|r| {
            let tag = r["tag_name"].as_str()?.to_string();
            let key = version_key(&tag)?;
            let asset = installer_asset(r, pkg);
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
    let latest = releases.iter().find(|r| r.compared > 0 && (channel == Channel::Experimental || !r.prerelease) && r.installer.is_some()).map(|r| r.tag.clone());
    UpdateInfo { current: current.to_string(), releases, latest, channel: channel.as_str().into(), package: pkg.label().into(), how: pkg.how().into() }
}

pub async fn check(app: &AppHandle) -> Result<UpdateInfo> {
    let current = app.package_info().version.to_string();
    let channel = channel(&app.state::<AppState>(), &current);
    let v = match get_json(ONSITE_RELEASES).await {
        Err(AppError::NotFound(_)) => Value::Array(vec![]),
        other => other?,
    };
    Ok(parse_releases(&v, &current, Package::current(), channel))
}

#[derive(Clone, Serialize)]
struct Progress {
    tag: String,
    received: u64,
    total: u64,
}

/// Downloads `tag`'s package for this system and installs it the way the system expects.
/// The release is looked up again here; the UI only names the tag. Returns what happens next.
pub async fn install(app: &AppHandle, tag: &str) -> Result<String> {
    let state = app.state::<AppState>();
    if !state.rounds.lock().await.is_empty() {
        return Err(AppError::Invalid("Finish or leave the interview first.".into()));
    }
    if !state.research.lock().unwrap().is_empty() {
        return Err(AppError::Invalid("A research job is running. Wait for it or cancel it first.".into()));
    }
    let pkg = Package::current();
    let r = get_json(&format!("{ONSITE_TAG}{tag}")).await?;
    let asset = installer_asset(&r, pkg).ok_or_else(|| AppError::NotFound(format!("That version has no {}.", pkg.label())))?;
    let url = asset["browser_download_url"].as_str().unwrap_or_default();
    let name = asset["name"].as_str().unwrap_or("Onsite-setup.exe");
    // Only GitHub's own download host; the name must be a plain file name.
    if !url.starts_with("https://github.com/") || name.contains(['/', '\\']) || name.starts_with('.') {
        return Err(AppError::Invalid("That download link doesn't look like a GitHub release.".into()));
    }
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let dir: PathBuf = match pkg {
        Package::Nsis => std::env::temp_dir().join("onsite-update"),
        // Same folder as the running AppImage, so the swap is a rename on one disk.
        Package::AppImage => appimage.as_ref().and_then(|p| p.parent()).map(PathBuf::from).unwrap_or_else(std::env::temp_dir),
        Package::Dmg | Package::Deb => app.path().download_dir().unwrap_or_else(|_| std::env::temp_dir()),
    };
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(if pkg == Package::AppImage { format!(".{name}.part") } else { name.to_string() });
    download(app, tag, url, &path).await?;
    tracing::info!(%tag, path = %path.display(), ?pkg, "update downloaded");
    let quit = |app: &AppHandle| {
        // Give the installer (or the new copy) a moment to open, then get out of its way.
        let handle = app.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            handle.exit(0);
        });
    };
    match pkg {
        Package::Nsis => {
            std::process::Command::new(&path).spawn().map_err(|e| AppError::Invalid(format!("Couldn't start the installer: {e}")))?;
            quit(app);
            Ok("Starting the installer. Onsite will close.".into())
        }
        Package::Dmg => {
            std::process::Command::new("open").arg(&path).spawn().map_err(|e| AppError::Invalid(format!("Couldn't open the disk image: {e}")))?;
            Ok(format!("Opened {name} from Downloads. Quit Onsite, drag the new Onsite into Applications and replace the old one, then open it again."))
        }
        Package::AppImage => {
            let target = appimage.ok_or_else(|| AppError::Invalid("Couldn't find the running AppImage.".into()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
            }
            // A running AppImage can be replaced: the old file stays open until this process ends.
            std::fs::rename(&path, &target).map_err(|e| AppError::Invalid(format!("Couldn't replace {}: {e}", target.display())))?;
            std::process::Command::new(&target).spawn().map_err(|e| AppError::Invalid(format!("Updated, but couldn't restart: {e}. Open Onsite again.")))?;
            quit(app);
            Ok("Updated. Onsite is restarting.".into())
        }
        Package::Deb => {
            let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
            Ok(format!("Saved {name} to {}. Install it (double-click it, or run: sudo apt install ./{name}), then open Onsite again.", dir.display()))
        }
    }
}

async fn download(app: &AppHandle, tag: &str, url: &str, path: &std::path::Path) -> Result<()> {
    let resp = client().get(url).timeout(Duration::from_secs(30 * 60)).send().await.map_err(|e| AppError::Invalid(format!("Download failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Invalid(format!("Download failed: HTTP {}.", resp.status().as_u16())));
    }
    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(path).await?;
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
        let _ = std::fs::remove_file(path);
        return Err(AppError::Invalid("The download was cut short. Try again.".into()));
    }
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
        assert_eq!(version_key("v2"), Some((2, 0, 0, true, 0)));
        assert!(version_key("0.1.3-exp.2") > version_key("0.1.3-exp.1"));
        assert!(version_key("0.1.3-exp.10") > version_key("0.1.3-exp.9"));
        assert!(version_key("0.1.3") > version_key("0.1.3-exp.9"));
        assert!(version_key("0.1.4-exp.1") > version_key("0.1.3"));
        assert!(version_key("0.1.3-exp.1") > version_key("0.1.2"));
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
        let info = parse_releases(&v, "0.1.0", Package::Nsis, Channel::Stable);
        let tags: Vec<_> = info.releases.iter().map(|r| (r.tag.as_str(), r.compared)).collect();
        assert_eq!(tags, [("v0.3.0-rc1", 1), ("v0.2.5", 1), ("v0.2.0", 1), ("v0.1.0", 0)]);
        // Pre-releases and releases without an installer are not offered as "the update".
        assert_eq!(info.latest.as_deref(), Some("v0.2.0"));
        // Experimental takes the newest of all.
        assert_eq!(parse_releases(&v, "0.1.0", Package::Nsis, Channel::Experimental).latest.as_deref(), Some("v0.3.0-rc1"));
        assert_eq!(parse_releases(&v, "0.2.0", Package::Nsis, Channel::Stable).releases.last().unwrap().compared, -1);
        // A Mac sees no installer in Windows-only releases.
        assert!(parse_releases(&v, "0.1.0", Package::Dmg, Channel::Stable).latest.is_none());
    }

    #[test]
    fn channel_defaults() {
        assert_eq!(Channel::default_for("0.1.3-exp.1"), Channel::Experimental);
        assert_eq!(Channel::default_for("0.1.3"), Channel::Stable);
        assert_eq!(Channel::parse("experimental"), Channel::Experimental);
        assert_eq!(Channel::parse("anything"), Channel::Stable);
    }

    #[test]
    fn packages_pick_their_file() {
        let names = ["Onsite_0.2.0_x64-setup.exe", "Onsite_0.2.0_universal.dmg", "Onsite_0.2.0_amd64.AppImage", "Onsite_0.2.0_amd64.deb", "Onsite_universal.app.tar.gz"];
        let pick = |p: Package, arch: &str| names.iter().filter(|n| p.matches(n, arch)).copied().collect::<Vec<_>>();
        assert_eq!(pick(Package::Nsis, "x86_64"), ["Onsite_0.2.0_x64-setup.exe"]);
        assert_eq!(pick(Package::Dmg, "aarch64"), ["Onsite_0.2.0_universal.dmg"]);
        assert_eq!(pick(Package::AppImage, "x86_64"), ["Onsite_0.2.0_amd64.AppImage"]);
        assert_eq!(pick(Package::Deb, "x86_64"), ["Onsite_0.2.0_amd64.deb"]);
        assert!(pick(Package::Deb, "aarch64").is_empty());
    }
}
