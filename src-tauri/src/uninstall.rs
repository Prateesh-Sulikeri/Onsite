//! Settings → Storage → Uninstall Onsite: removes the app and everything it left behind.
//! Windows hands off to the NSIS uninstaller; its hook (`windows/hooks.nsh`) deletes the files
//! once the app has closed. macOS and Linux have no uninstaller, so a shell waits for Onsite to
//! quit, then deletes the files and the app itself. VoiceStudio is never touched, only the
//! interviewer voices Onsite made in it.

use crate::error::{AppError, Result};
use crate::updates::{self, Package};
use crate::AppState;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Manager};

/// What a real uninstall always removes: downloads and scratch, never the user's own data.
#[cfg(unix)]
/// Windows has the same list in `windows/hooks.nsh`.
fn residue(data_dir: &Path) -> Vec<PathBuf> {
    let tmp = std::env::temp_dir();
    let mut v = vec![crate::voice::models::root(data_dir), tmp.join("onsite-update"), tmp.join("onsite-stt")];
    if let Some(log_dir) = crate::storage::log_path().parent() {
        v.push(log_dir.to_path_buf());
    }
    if let Ok(rd) = std::fs::read_dir(&tmp) {
        let scratch = |n: &str| n.starts_with("onsite-round-") || n.starts_with("onsite-job-");
        v.extend(rd.filter_map(|e| e.ok()).filter(|e| scratch(&e.file_name().to_string_lossy())).map(|e| e.path()));
    }
    v
}

/// Interviews, settings, keys, and the WebView's storage (theme, last setup, a copy of the resume).
#[cfg(unix)]
fn user_data(app: &AppHandle, data_dir: &Path) -> Vec<PathBuf> {
    let p = app.path();
    let mut v = vec![data_dir.to_path_buf()];
    v.extend([p.app_local_data_dir(), p.app_config_dir(), p.app_cache_dir()].into_iter().flatten());
    if cfg!(target_os = "macos") {
        if let Some(lib) = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library")) {
            let id = &app.config().identifier;
            v.push(lib.join("WebKit").join(id));
            v.push(lib.join("HTTPStorages").join(id));
            v.push(lib.join("Saved Application State").join(format!("{id}.savedState")));
            v.push(lib.join("Preferences").join(format!("{id}.plist")));
        }
    }
    v
}

/// The .app bundle Onsite runs from (macOS).
fn app_bundle() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(Path::to_path_buf)
}

/// The Debian package that owns the running binary, from `dpkg -S`.
async fn deb_package() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let out = tokio::process::Command::new("dpkg").arg("-S").arg(&exe).output().await.ok()?;
    let line = String::from_utf8_lossy(&out.stdout);
    let name = line.lines().next()?.split_once(':')?.0.trim().to_string();
    (out.status.success() && !name.is_empty() && !name.contains(char::is_whitespace)).then_some(name)
}

/// Deletes `paths` after this process exits. Detached, so it outlives the app.
#[cfg(unix)]
fn delete_after_exit(paths: &[PathBuf]) -> Result<()> {
    use std::os::unix::process::CommandExt;
    const SCRIPT: &str = r#"pid=$1; shift; while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done; rm -rf -- "$@""#;
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(SCRIPT)
        .arg("onsite-uninstall")
        .arg(std::process::id().to_string())
        .args(paths)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| AppError::Invalid(format!("Couldn't start the clean-up: {e}")))?;
    Ok(())
}

/// Removes the interviewer voices Onsite made in VoiceStudio, if it's running. Best effort.
async fn remove_voicestudio_voices(state: &AppState) {
    let vs = crate::voice::vs::VoiceStudio::new(&state.settings().voice.base_url);
    match tokio::time::timeout(Duration::from_secs(20), vs.delete_persona_profiles()).await {
        Ok(Ok(n)) => tracing::info!(n, "uninstall: removed interviewer voices from VoiceStudio"),
        Ok(Err(e)) => tracing::info!("uninstall: VoiceStudio voices left: {e}"),
        Err(_) => tracing::info!("uninstall: VoiceStudio didn't answer; its voices are left"),
    }
}

/// Uninstalls Onsite. `keep_data` keeps interviews, feedback and settings for a later reinstall;
/// models, the log and temp files go either way. Returns what happens next; Onsite then quits.
pub async fn run(app: &AppHandle, keep_data: bool) -> Result<String> {
    let state = app.state::<AppState>();
    updates::ensure_idle(&state).await?;
    let pkg = Package::current();
    // Settle how the app itself goes before deleting anything.
    let app_file = match pkg {
        Package::Nsis => {
            let dir = std::env::current_exe()?.parent().map(Path::to_path_buf).unwrap_or_default();
            let u = dir.join("uninstall.exe");
            if !u.is_file() {
                return Err(AppError::Invalid("This copy of Onsite wasn't installed by its installer, so there's nothing to uninstall.".into()));
            }
            u
        }
        Package::Dmg => app_bundle().ok_or_else(|| AppError::Invalid("Couldn't find Onsite.app.".into()))?,
        Package::AppImage => std::env::var_os("APPIMAGE").map(PathBuf::from).filter(|p| p.is_file()).ok_or_else(|| AppError::Invalid("Couldn't find the AppImage.".into()))?,
        Package::Deb => PathBuf::new(),
    };
    if pkg == Package::Deb {
        let name = deb_package().await.ok_or_else(|| AppError::Invalid("This copy of Onsite wasn't installed from a .deb package.".into()))?;
        // pkexec shows the system's password prompt. Removing the package leaves this process running.
        let ok = tokio::process::Command::new("pkexec").args(["apt-get", "remove", "-y", &name]).status().await.is_ok_and(|s| s.success());
        if !ok {
            return Err(AppError::Invalid(format!("Couldn't remove the package. Try again, or run: sudo apt remove {name}")));
        }
    }
    remove_voicestudio_voices(&state).await;

    let msg = match pkg {
        Package::Nsis => {
            let mut cmd = std::process::Command::new(&app_file);
            cmd.arg("/P");
            if !keep_data {
                cmd.arg("/PURGE");
            }
            cmd.spawn().map_err(|e| AppError::Invalid(format!("Couldn't start the uninstaller: {e}")))?;
            "Uninstalling. Onsite will close."
        }
        #[cfg(unix)]
        _ => {
            let mut paths = residue(&state.data_dir);
            if !keep_data {
                paths.extend(user_data(app, &state.data_dir));
            }
            if pkg != Package::Deb {
                paths.push(app_file);
            }
            delete_after_exit(&paths)?;
            "Onsite is removing itself and will close."
        }
        #[cfg(not(unix))]
        _ => unreachable!("Package::current() is Nsis on Windows"),
    };
    tracing::info!(?pkg, keep_data, "uninstalling");
    let handle = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        handle.exit(0);
    });
    Ok(msg.into())
}
