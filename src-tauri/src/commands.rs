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
use tauri::{AppHandle, Emitter, Manager, State};

/// UI errors land in the same log file as the backend's.
#[tauri::command]
pub fn ui_log(level: String, message: String) {
    match level.as_str() {
        "error" => tracing::error!(target: "ui", "{message}"),
        _ => tracing::info!(target: "ui", "{message}"),
    }
}

#[tauri::command]
pub fn get_panels(state: State<AppState>) -> Vec<crate::panels::PanelSummary> {
    state.panels.list()
}

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

/// Feedback for the whole interview: reviews every round still missing feedback, then the verdict.
#[tauri::command]
pub async fn loop_feedback(app: AppHandle, loop_id: i64) -> Result<live::LoopFeedback> {
    live::loop_feedback(&app, loop_id).await
}

/// The saved interview feedback, without writing anything.
#[tauri::command]
pub fn get_loop_feedback(state: State<AppState>, loop_id: i64) -> Result<live::LoopFeedback> {
    live::loop_feedback_saved(&state, loop_id)
}

/// Run (examples) or Submit (all tests) for the current coding round.
#[tauri::command]
pub async fn run_code(app: AppHandle, round_id: i64, code: String, submit: bool) -> Result<crate::runner::Report> {
    live::run_code(&app, round_id, code, submit).await
}

/// Deletes a finished round with its transcript and feedback.
#[tauri::command]
pub async fn delete_round(state: State<'_, AppState>, round_id: i64) -> Result<()> {
    if state.rounds.lock().await.contains_key(&round_id) {
        return Err(AppError::Invalid("This round is still running. End it first.".into()));
    }
    state.store.lock().unwrap().delete_round(round_id)
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
    /// The English-only speech-to-text model (installed on demand).
    pub english: Option<crate::voice::stt::EnglishModel>,
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
        return Ok(VoiceStatus { up: false, loaded: false, device: String::new(), profiles: vec![], installed, english: None });
    };
    Ok(VoiceStatus {
        up: true,
        loaded: vs.ready().await.unwrap_or(false),
        device: health["device"].as_str().unwrap_or_default().to_string(),
        profiles: vs.profiles().await.unwrap_or_default(),
        installed,
        english: Some(crate::voice::stt::english_model(&vs).await),
    })
}

/// Opens the VoiceStudio desktop app; its backend takes about a minute to come up.
#[tauri::command]
pub fn voice_launch() -> Result<()> {
    let exe = voicestudio_exe().ok_or_else(|| AppError::Voice("VoiceStudio is not installed.".into()))?;
    std::process::Command::new(exe).spawn()?;
    Ok(())
}

/// Speaks a short line once, in the interviewer's voice, so the first real reply skips
/// VoiceStudio's warm-up (model load plus encoding the profile's reference sample).
pub async fn warm_voice(settings: &VoiceSettings) {
    let vs = VoiceStudio::new(&settings.base_url);
    if vs.health().await.is_err() {
        return;
    }
    let voice = match vs.resolve_voice(&settings.voice).await {
        Ok(v) => v,
        Err(e) => return tracing::warn!("VoiceStudio warm-up skipped: {e}"),
    };
    let started = std::time::Instant::now();
    match vs.speak("Okay, let's begin.", &voice, settings.steps, settings.speed, |_| {}).await {
        Ok(()) => tracing::info!(ms = started.elapsed().as_millis() as u64, "VoiceStudio warmed up"),
        Err(e) => tracing::warn!("VoiceStudio warm-up failed: {e}"),
    }
}

#[tauri::command]
pub fn audio_devices() -> Result<crate::voice::audio::AudioDevices> {
    crate::voice::audio::list_devices()
}

/// Bumped by every start and stop; a monitor runs only while the counter still holds its own value.
static MIC_MONITOR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[derive(Clone, Serialize)]
struct MicLevel {
    mic: f32,
}

/// Opens the chosen mic for the "Get ready" meter and emits `mic_level` every 80 ms. No STT.
/// Stops on `mic_monitor_stop`, on the next start, or after 10 minutes.
#[tauri::command]
pub async fn mic_monitor_start(app: AppHandle, input: String) -> Result<()> {
    use std::sync::atomic::Ordering;
    let me = MIC_MONITOR.fetch_add(1, Ordering::SeqCst) + 1;
    let s = app.state::<AppState>().settings();
    crate::voice::audio::set_devices(&input, &s.voice.output_device);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<i16>>();
    let mic = crate::voice::audio::Mic::open(tx)?;
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let mut tick = tokio::time::interval(std::time::Duration::from_millis(80));
        while MIC_MONITOR.load(Ordering::SeqCst) == me && !mic.is_dead() && started.elapsed().as_secs() < 600 {
            tokio::select! {
                _ = tick.tick() => { let _ = app.emit("mic_level", MicLevel { mic: mic.level.get() }); }
                c = rx.recv() => if c.is_none() { break },
            }
        }
        if mic.is_dead() {
            let _ = app.emit("mic_level", MicLevel { mic: -1.0 });
        }
    });
    Ok(())
}

#[tauri::command]
pub fn mic_monitor_stop() {
    MIC_MONITOR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

/// Returns ms to first audio.
#[tauri::command]
pub async fn voice_test_speak(state: State<'_, AppState>, settings: VoiceSettings, text: String) -> Result<u64> {
    crate::voice::audio::set_devices(&settings.input_device, &settings.output_device);
    state.voice.test_speak(&settings, &text).await
}

#[tauri::command]
pub async fn voice_test_listen(app: AppHandle, settings: VoiceSettings) -> Result<String> {
    crate::voice::audio::set_devices(&settings.input_device, &settings.output_device);
    let ctl = crate::voice::stt::ListenCtl::default();
    let vs = VoiceStudio::new(&settings.base_url);
    let pause = if settings.pause_ms == 0 { 1800 } else { settings.pause_ms };
    let emit = move |ev: crate::voice::VoiceEvent| {
        let _ = app.emit("voice_test", &ev);
    };
    use crate::voice::stt::ListenEnd;
    let listen = crate::voice::stt::listen(&vs, pause, &ctl, emit);
    match tokio::time::timeout(std::time::Duration::from_secs(30), listen).await {
        Ok(Ok(ListenEnd::Text { text, .. })) => Ok(text),
        Ok(Ok(ListenEnd::Cancelled { heard })) if !heard.is_empty() => Ok(heard),
        Ok(Ok(ListenEnd::Cancelled { .. } | ListenEnd::NoSpeech)) | Err(_) => {
            Err(AppError::Voice("I didn't hear anything. Check the mic and try again.".into()))
        }
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

/// Tapped the mic while the interviewer was talking.
#[tauri::command]
pub fn voice_interrupt(state: State<AppState>) {
    state.voice.interrupt();
}

/// Switches the mic (and speaker) mid-round. The speaker changes on the next reply.
#[tauri::command]
pub fn voice_set_devices(state: State<AppState>, input: String, output: String, relisten: bool) -> Result<()> {
    crate::voice::audio::set_devices(&input, &output);
    let mut s = state.settings();
    s.voice.input_device = input;
    s.voice.output_device = output;
    state.save_settings(&s)?;
    if relisten {
        state.voice.relisten();
    }
    Ok(())
}
