//! Thin `#[tauri::command]` wrappers. Logic lives in the modules.

use crate::brain::{self, run_once, ClaudeLocation, Tools};
use crate::catalog::Catalog;
use crate::error::{AppError, Result};
use crate::interview::live::{self, RoundInfo};
use crate::interview::{LoopSetup, Workspace};
use crate::store::{HistoryItem, RoundRow, Turn};
use crate::{AppState, Settings};
use serde::Serialize;
use serde_json::Value;
use crate::voice::vs::VoiceStudio;
use crate::voice::VoiceSettings;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub fn get_catalog(state: State<AppState>) -> Catalog {
    state.catalog.clone()
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn save_settings(state: State<AppState>, settings: Settings) -> Result<()> {
    state.save_settings(&settings)
}

#[tauri::command]
pub async fn detect_claude(state: State<'_, AppState>) -> Result<ClaudeLocation> {
    let loc = brain::locate::detect().await.ok_or(AppError::ClaudeNotFound)?;
    let mut s = state.settings();
    s.claude = Some(loc.clone());
    state.save_settings(&s)?;
    Ok(loc)
}

#[derive(Serialize)]
pub struct Health {
    pub ok: bool,
    pub reply: String,
    pub latency_ms: u64,
    pub location: String,
    pub model: String,
}

#[tauri::command]
pub async fn health_check(state: State<'_, AppState>) -> Result<Health> {
    let loc = state.claude_location().await?;
    let model = state.settings().interviewer_model;
    let out = run_once(&loc, "Reply with OK", Tools::None, &model, |_| {}).await?;
    Ok(Health {
        ok: out.text.trim().trim_end_matches('.').eq_ignore_ascii_case("ok"),
        reply: out.text,
        latency_ms: out.duration_ms,
        location: loc.describe(),
        model: out.model,
    })
}

#[tauri::command]
pub fn create_loop(state: State<AppState>, setup: LoopSetup) -> Result<i64> {
    live::create_loop(&state, &setup)
}

#[tauri::command]
pub async fn start_round(app: AppHandle, loop_id: i64, index: usize) -> Result<RoundInfo> {
    live::start_round(&app, loop_id, index).await
}

#[tauri::command]
pub async fn send_answer(app: AppHandle, round_id: i64, text: String, workspace: Workspace, force: bool) -> Result<Option<Turn>> {
    live::send_answer(&app, round_id, text, workspace, force).await
}

#[tauri::command]
pub async fn end_round(app: AppHandle, round_id: i64, workspace: Workspace) -> Result<()> {
    live::end_round(&app, round_id, workspace).await
}

#[tauri::command]
pub async fn generate_report(app: AppHandle, round_id: i64) -> Result<Value> {
    live::generate_report(&app, round_id).await
}

#[tauri::command]
pub fn get_report(state: State<AppState>, round_id: i64) -> Result<Option<Value>> {
    Ok(state.store.lock().unwrap().report(round_id)?.and_then(|(data, _)| data))
}

#[tauri::command]
pub fn get_round(state: State<AppState>, round_id: i64) -> Result<RoundRow> {
    state.store.lock().unwrap().round(round_id)
}

#[tauri::command]
pub async fn loop_verdict(app: AppHandle, loop_id: i64) -> Result<Value> {
    live::loop_verdict(&app, loop_id).await
}

#[tauri::command]
pub fn history(state: State<AppState>) -> Result<Vec<HistoryItem>> {
    state.store.lock().unwrap().history(100)
}

#[derive(Serialize)]
pub struct VoiceStatus {
    pub up: bool,
    pub loaded: bool,
    pub device: String,
    pub profiles: Vec<crate::voice::vs::Profile>,
    pub installed: bool,
}

fn voicestudio_exe() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join("VoiceStudio").join("VoiceStudio.exe");
    p.is_file().then_some(p)
}

#[tauri::command]
pub async fn voice_status(state: State<'_, AppState>) -> Result<VoiceStatus> {
    let vs = VoiceStudio::new(&state.settings().voice.base_url);
    let installed = voicestudio_exe().is_some();
    let Ok(health) = vs.health().await else {
        return Ok(VoiceStatus { up: false, loaded: false, device: String::new(), profiles: vec![], installed });
    };
    Ok(VoiceStatus {
        up: true,
        loaded: vs.model_loaded().await.unwrap_or(false),
        device: health["device"].as_str().unwrap_or_default().to_string(),
        profiles: vs.profiles().await.unwrap_or_default(),
        installed,
    })
}

/// Opens the VoiceStudio desktop app; its backend takes about a minute to come up.
#[tauri::command]
pub fn voice_launch() -> Result<()> {
    let exe = voicestudio_exe().ok_or_else(|| AppError::Voice("VoiceStudio is not installed.".into()))?;
    std::process::Command::new(exe).spawn()?;
    Ok(())
}

/// Speaks a short line once so the first real reply skips VoiceStudio's warm-up.
pub async fn warm_voice(settings: &VoiceSettings) {
    let vs = VoiceStudio::new(&settings.base_url);
    if vs.health().await.is_err() {
        return;
    }
    let started = std::time::Instant::now();
    match vs.speak("Okay.", &settings.voice, settings.steps, settings.speed, |_| {}).await {
        Ok(()) => tracing::info!(ms = started.elapsed().as_millis() as u64, "VoiceStudio warmed up"),
        Err(e) => tracing::warn!("VoiceStudio warm-up failed: {e}"),
    }
}

/// Returns ms to first audio.
#[tauri::command]
pub async fn voice_test_speak(state: State<'_, AppState>, settings: VoiceSettings, text: String) -> Result<u64> {
    state.voice.test_speak(&settings, &text).await
}

#[tauri::command]
pub async fn voice_test_listen(app: AppHandle, settings: VoiceSettings) -> Result<String> {
    let ctl = crate::voice::stt::ListenCtl::default();
    let vs = VoiceStudio::new(&settings.base_url);
    let pause = if settings.pause_ms == 0 { 1800 } else { settings.pause_ms };
    let emit = move |ev: crate::voice::VoiceEvent| {
        let _ = app.emit("voice_test", &ev);
    };
    let listen = crate::voice::stt::listen(&vs, pause, &ctl, emit);
    match tokio::time::timeout(std::time::Duration::from_secs(30), listen).await {
        Ok(Ok(crate::voice::stt::ListenEnd::Text { text, .. })) => Ok(text),
        Ok(Ok(_)) | Err(_) => Err(AppError::Voice("I didn't hear anything. Check the mic and try again.".into())),
        Ok(Err(e)) => Err(e),
    }
}

#[tauri::command]
pub fn voice_finish(state: State<AppState>) {
    state.voice.finish_answer();
}

#[tauri::command]
pub fn voice_mute(state: State<AppState>, muted: bool) {
    state.voice.set_muted(muted);
}

#[tauri::command]
pub fn voice_cancel_listen(state: State<AppState>) {
    state.voice.cancel_listen();
}
