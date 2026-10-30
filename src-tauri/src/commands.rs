//! Thin `#[tauri::command]` wrappers. Logic lives in the modules.

use crate::brain::{self, run_once, ClaudeLocation, Provider, Tools};
use crate::catalog::Catalog;
use crate::error::{AppError, Result};
use crate::interview::live::{self, RoundInfo};
use crate::interview::{LoopSetup, Workspace};
use crate::store::{HistoryItem, RoundRow, Turn};
use crate::{AppState, Settings};
use serde::Serialize;
use serde_json::Value;
use crate::voice::engine::{Engine, VoiceEngine};
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
    let backend = state.backend().await?;
    let model = state.settings().interviewer_model;
    let out = run_once(&backend, "Reply with OK", Tools::None, &model, |_| {}).await?;
    Ok(Health {
        ok: out.text.trim().trim_end_matches('.').eq_ignore_ascii_case("ok"),
        reply: out.text,
        latency_ms: out.duration_ms,
        location: backend.describe(),
        model: out.model,
    })
}

/// Which providers have a key saved. Keys themselves never go back to the UI.
#[tauri::command]
pub fn ai_keys(state: State<AppState>) -> std::collections::HashMap<String, bool> {
    let keys = state.api_keys();
    Provider::APIS.iter().map(|p| (p.id().to_string(), keys.get(p.id()).is_some_and(|k| !k.is_empty()))).collect()
}

#[tauri::command]
pub fn set_api_key(state: State<AppState>, provider: Provider, key: String) -> Result<()> {
    if provider == Provider::ClaudeCode {
        return Err(AppError::Invalid("Claude Code uses your Claude login, not a key.".into()));
    }
    state.set_api_key(provider, &key)
}

/// Switches where model calls go and resets both models to that provider's defaults.
#[tauri::command]
pub fn set_provider(state: State<AppState>, provider: Provider) -> Result<Settings> {
    let mut s = state.settings();
    if s.provider != provider {
        let (iv, rep) = provider.default_models();
        s.provider = provider;
        s.interviewer_model = iv.into();
        s.report_model = rep.into();
        state.save_settings(&s)?;
    }
    Ok(s)
}

/// Models the current provider offers (asked live from the API).
#[tauri::command]
pub async fn list_models(state: State<'_, AppState>) -> Result<Vec<String>> {
    match state.backend().await? {
        brain::Backend::Cli(_) => Ok(["sonnet", "opus", "haiku", "fable"].map(String::from).to_vec()),
        brain::Backend::Api(c) => c.models().await,
    }
}

#[tauri::command]
pub fn list_dossiers(state: State<AppState>) -> Result<Vec<Value>> {
    state.store.lock().unwrap().dossiers()
}

#[tauri::command]
pub fn delete_dossier(state: State<AppState>, id: String) -> Result<()> {
    state.store.lock().unwrap().delete_dossier(&id)?;
    let _ = std::fs::remove_file(state.data_dir.join("dossiers").join(format!("{id}.json")));
    Ok(())
}

/// A dossier file someone shared (JSON text). Returns the saved dossier.
#[tauri::command]
pub fn import_dossier(state: State<AppState>, text: String) -> Result<crate::dossier::Dossier> {
    crate::research::import(&state, &text)
}

/// Writes the dossier as pretty JSON into Downloads, for sharing. Returns the file path.
#[tauri::command]
pub fn export_dossier(app: AppHandle, id: String) -> Result<String> {
    let state = app.state::<AppState>();
    let d = state.store.lock().unwrap().dossier(&id)?.ok_or_else(|| AppError::NotFound("That dossier is gone.".into()))?;
    let dir = app.path().download_dir().map_err(|_| AppError::NotFound("No Downloads folder found.".into()))?;
    let path = dir.join(format!("{}.json", crate::dossier::slug(&id, "", "")));
    std::fs::write(&path, serde_json::to_string_pretty(&d)?)?;
    Ok(path.display().to_string())
}

#[tauri::command]
pub async fn research_start(app: AppHandle, request: crate::research::ResearchRequest) -> Result<i64> {
    crate::research::start(&app, request)
}

#[tauri::command]
pub fn research_cancel(app: AppHandle, id: i64) {
    crate::research::cancel(&app, id)
}

#[tauri::command]
pub async fn research_retry(app: AppHandle, id: i64) -> Result<i64> {
    crate::research::retry(&app, id)
}

/// Removes a finished, failed or cancelled job from the list.
#[tauri::command]
pub fn research_dismiss(state: State<AppState>, id: i64) -> Result<()> {
    if state.research.lock().unwrap().contains_key(&id) {
        return Err(AppError::Invalid("That research is still running. Cancel it first.".into()));
    }
    state.store.lock().unwrap().delete_job(id)
}

#[tauri::command]
pub fn research_jobs(state: State<AppState>) -> Result<Vec<crate::research::JobView>> {
    crate::research::jobs(&state)
}

#[tauri::command]
pub fn create_loop(state: State<AppState>, setup: LoopSetup) -> Result<i64> {
    live::create_loop(&state, &setup)
}

#[tauri::command]
pub async fn start_round(app: AppHandle, loop_id: i64, index: usize) -> Result<RoundInfo> {
    live::start_round(&app, loop_id, index).await
}

/// Stops a round that is still loading; its `start_round` returns an error.
#[tauri::command]
pub fn cancel_start(state: State<AppState>) {
    state.prep_gen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

#[tauri::command]
pub async fn send_answer(app: AppHandle, round_id: i64, text: String, workspace: Workspace, force: bool) -> Result<Option<Turn>> {
    live::send_answer(&app, round_id, text, workspace, force).await
}

#[tauri::command]
pub async fn end_round(app: AppHandle, round_id: i64, workspace: Workspace) -> Result<bool> {
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

/// The editor language picker in the call room.
#[tauri::command]
pub async fn set_round_lang(app: AppHandle, round_id: i64, lang: String) -> Result<Value> {
    live::set_round_lang(&app, round_id, lang).await
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

/// Rounds per week by round key, e.g. {"coding": 2, "sysdesign": 1}. Shown on the dashboard.
#[tauri::command]
pub fn get_goal(state: State<AppState>) -> Result<Value> {
    let goal = state.store.lock().unwrap().get_setting::<Value>("weekly_goal")?;
    Ok(goal.unwrap_or_else(|| serde_json::json!({"coding": 2, "sysdesign": 1, "behavioral": 1})))
}

#[tauri::command]
pub fn set_goal(state: State<AppState>, goal: std::collections::BTreeMap<String, u32>) -> Result<()> {
    let goal: std::collections::BTreeMap<_, _> = goal.into_iter().filter(|(k, n)| *n > 0 && state.catalog.rounds.contains_key(k)).collect();
    state.store.lock().unwrap().set_setting("weekly_goal", &goal)
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
    /// Whisper on the GPU, which writes the text sent to the interviewer.
    pub whisper: crate::voice::whisper::WhisperStatus,
    /// Installed VoiceStudio version (from the backend, else its install folder). No network.
    pub version: Option<String>,
    /// The engine in settings. For the built-in engine `up` means "all models installed".
    pub engine: VoiceEngine,
    /// Built-in engine models and whether each is installed.
    pub models: Vec<crate::voice::models::ModelInfo>,
    /// The shipped interviewer voices.
    pub voices: Vec<BuiltinVoice>,
}

#[derive(Serialize)]
pub struct BuiltinVoice {
    pub id: &'static str,
    pub name: &'static str,
    pub accent: &'static str,
}

fn voicestudio_exe() -> Option<std::path::PathBuf> {
    let p = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("Programs").join("VoiceStudio").join("VoiceStudio.exe");
    p.is_file().then_some(p)
}

#[tauri::command]
pub async fn voice_status(state: State<'_, AppState>) -> Result<VoiceStatus> {
    let settings = state.settings().voice;
    let models = crate::voice::models::status(&state.data_dir);
    let voices = crate::voice::builtin::VOICES.iter().map(|v| BuiltinVoice { id: v.0, name: v.1, accent: v.2 }).collect();
    if settings.engine == VoiceEngine::Builtin {
        let ready = models.iter().all(|m| m.installed);
        return Ok(VoiceStatus {
            up: ready,
            loaded: ready,
            device: "Built-in".into(),
            profiles: vec![],
            installed: voicestudio_exe().is_some(),
            english: None,
            whisper: crate::voice::whisper::status(),
            version: None,
            engine: settings.engine,
            models,
            voices,
        });
    }
    let vs = VoiceStudio::new(&settings.base_url);
    let installed = voicestudio_exe().is_some();
    let engine = settings.engine;
    let Ok(health) = vs.health().await else {
        return Ok(VoiceStatus { up: false, loaded: false, device: String::new(), profiles: vec![], installed, english: None, whisper: crate::voice::whisper::status(), version: crate::updates::installed_voicestudio(), engine, models, voices });
    };
    Ok(VoiceStatus {
        up: true,
        loaded: vs.ready().await.unwrap_or(false),
        device: health["device"].as_str().unwrap_or_default().to_string(),
        profiles: vs.profiles().await.unwrap_or_default(),
        installed,
        english: Some(crate::voice::stt::english_model(&vs).await),
        whisper: crate::voice::whisper::status(),
        version: health["version"].as_str().map(String::from).or_else(crate::updates::installed_voicestudio),
        engine,
        models,
        voices,
    })
}

/// Downloads one built-in voice model (progress arrives as `voice_model` events). Returns when it is installed.
#[tauri::command]
pub async fn voice_model_download(app: AppHandle, state: State<'_, AppState>, id: String) -> Result<()> {
    let pin = crate::voice::models::pin(&id).ok_or_else(|| AppError::Invalid("Unknown voice model.".into()))?;
    crate::voice::models::install(&state.data_dir, pin, move |p| {
        let _ = app.emit("voice_model", &p);
    })
    .await?;
    state.builtin.unload();
    Ok(())
}

#[tauri::command]
pub fn voice_model_delete(state: State<AppState>, id: String) -> Result<()> {
    let pin = crate::voice::models::pin(&id).ok_or_else(|| AppError::Invalid("Unknown voice model.".into()))?;
    state.builtin.unload();
    crate::voice::models::delete(&state.data_dir, &pin)
}

/// Drops the loaded built-in models; the next round loads them again.
#[tauri::command]
pub fn voice_models_reload(state: State<AppState>) {
    state.builtin.unload();
}

/// Speaks one line in a built-in voice ("ethan_brooks" or "priya_shah"). Returns ms to first audio.
#[tauri::command]
pub async fn voice_builtin_preview(state: State<'_, AppState>, voice: String) -> Result<u64> {
    let mut settings = state.settings().voice;
    settings.engine = VoiceEngine::Builtin;
    settings.persona = None;
    settings.builtin_voice = crate::voice::builtin::known_clip(&voice).into();
    crate::voice::audio::set_devices(&settings.input_device, &settings.output_device);
    let engine = Engine::from_settings(&state, &settings).await?;
    let name = crate::voice::builtin::VOICES.iter().find(|v| v.0 == settings.builtin_voice).map_or("your interviewer", |v| v.1);
    let line = format!("Hi, I'm {name}. I'll be one of the people interviewing you today.");
    state.voice.test_speak(&engine, &settings, &line).await
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
    // VoiceStudio only: the built-in engine warms up when a round starts.
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
    let engine = Engine::from_settings(&state, &settings).await?;
    state.voice.test_speak(&engine, &settings, &text).await
}

#[tauri::command]
pub async fn voice_test_listen(app: AppHandle, state: State<'_, AppState>, settings: VoiceSettings) -> Result<String> {
    crate::voice::audio::set_devices(&settings.input_device, &settings.output_device);
    let ctl = crate::voice::stt::ListenCtl::default();
    let engine = Engine::from_settings(&state, &settings).await?;
    let pause = if settings.pause_ms == 0 { 1800 } else { settings.pause_ms };
    let emit = move |ev: crate::voice::VoiceEvent| {
        let _ = app.emit("voice_test", &ev);
    };
    use crate::voice::stt::ListenEnd;
    let listen = crate::voice::stt::listen(&engine, pause, &ctl, emit);
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

#[derive(Serialize)]
pub struct ResumeText {
    pub kind: crate::ats::extract::Kind,
    pub kind_label: &'static str,
    pub text: String,
    pub pages: Option<usize>,
    pub words: usize,
}

/// Text out of a resume or job description file (PDF, .docx, .rtf, .txt, .md), for the interview setup.
#[tauri::command]
pub async fn read_document(name: String, data: String) -> Result<ResumeText> {
    let doc = crate::ats::read_file(name, &data).await?;
    if doc.text.split_whitespace().count() < 5 {
        return Err(AppError::Invalid("No text came out of that file. If it's a scan, export it as a text PDF or .docx.".into()));
    }
    Ok(ResumeText { kind: doc.kind, kind_label: doc.kind.label(), words: doc.text.split_whitespace().count(), pages: doc.page_count, text: doc.text })
}

/// Resume check. Send the file (`name` + base64 `data`) when there is one; else `text`.
#[tauri::command]
pub async fn ats_check(state: State<'_, AppState>, name: Option<String>, data: Option<String>, text: Option<String>, jd: String) -> Result<crate::ats::Report> {
    crate::ats::run(&state, name, data, text, &jd).await
}

/// The AI screen for the same resume and posting as `ats_check`. Takes Claude 10-30 s.
#[tauri::command]
pub async fn ats_screen(state: State<'_, AppState>, name: Option<String>, data: Option<String>, text: Option<String>, jd: String) -> Result<crate::ats::screen::AiScreen> {
    crate::ats::run_screen(&state, name, data, text, &jd).await
}

/// The last resume check, if any.
#[tauri::command]
pub fn ats_last(state: State<AppState>) -> Result<Option<Value>> {
    state.store.lock().unwrap().get_setting(crate::ats::LAST_KEY)
}

#[tauri::command]
pub fn get_update_channel(app: AppHandle, state: State<AppState>) -> String {
    crate::updates::channel(&state, &app.package_info().version.to_string()).as_str().into()
}

#[tauri::command]
pub fn set_update_channel(state: State<AppState>, channel: String) -> Result<()> {
    crate::updates::set_channel(&state, crate::updates::Channel::parse(&channel))
}

#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub async fn check_updates(app: AppHandle) -> Result<crate::updates::UpdateInfo> {
    crate::updates::check(&app).await
}

/// Downloads that version's installer, starts it and quits. Works for older versions too.
#[tauri::command]
pub async fn install_update(app: AppHandle, tag: String) -> Result<String> {
    crate::updates::install(&app, &tag).await
}

#[tauri::command]
pub async fn voicestudio_version(state: State<'_, AppState>) -> Result<crate::updates::VoiceStudioVersion> {
    Ok(crate::updates::voicestudio(&state).await)
}

#[tauri::command]
pub async fn storage_info(app: AppHandle) -> Result<crate::storage::StorageInfo> {
    // Walking the model cache can take a moment; keep it off the async workers.
    tauri::async_runtime::spawn_blocking(move || crate::storage::info(&app.state::<AppState>()))
        .await
        .map_err(|e| AppError::Invalid(e.to_string()))?
}

/// Saves the limits and applies them now. Returns how many rounds were deleted.
#[tauri::command]
pub fn set_storage_limits(state: State<AppState>, limits: crate::storage::Limits) -> Result<usize> {
    crate::storage::set_limits(&state, limits)
}

#[tauri::command]
pub fn clear_log() -> Result<()> {
    crate::storage::clear_log()
}

/// Returns bytes freed.
#[tauri::command]
pub fn clear_temp() -> Result<u64> {
    crate::storage::clear_temp()
}

#[tauri::command]
pub fn vacuum_db(state: State<AppState>) -> Result<()> {
    state.store.lock().unwrap().vacuum()
}

/// Opens one of Onsite's folders in Explorer: "data", "log", "models" or "voicestudio".
#[tauri::command]
pub fn open_folder(state: State<AppState>, which: String) -> Result<()> {
    let path = match which.as_str() {
        "data" => state.data_dir.clone(),
        "log" => crate::storage::log_path().parent().map(std::path::Path::to_path_buf).unwrap_or_default(),
        "models" => {
            let p = crate::voice::models::root(&state.data_dir);
            std::fs::create_dir_all(&p)?;
            p
        }
        "voicestudio" => std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("OmniVoice").join("hf_cache"),
        _ => return Err(AppError::Invalid("Unknown folder.".into())),
    };
    crate::storage::open_folder(&path)
}

#[tauri::command]
pub fn get_backdrop(state: State<AppState>) -> Result<Option<String>> {
    crate::storage::backdrop(&state)
}

/// `data`: the image as base64 or a data URL.
#[tauri::command]
pub fn set_backdrop(state: State<AppState>, data: String) -> Result<()> {
    crate::storage::set_backdrop(&state, &data)
}

#[tauri::command]
pub fn clear_backdrop(state: State<AppState>) -> Result<()> {
    crate::storage::clear_backdrop(&state)
}

/// Says a short line in a panel interviewer's own voice. Builds their VoiceStudio profile
/// first if it doesn't exist yet (5-15 s, once). Returns ms to first audio.
#[tauri::command]
pub async fn voice_persona_preview(state: State<'_, AppState>, id: String) -> Result<u64> {
    let p = state.panels.persona_by_id(&id).ok_or_else(|| AppError::NotFound("No interviewer with that id.".into()))?;
    let mut settings = state.settings().voice;
    settings.speed *= p.speed_factor;
    if settings.engine == VoiceEngine::Voicestudio {
        VoiceStudio::new(&settings.base_url).health().await.map_err(|_| AppError::Voice("VoiceStudio isn't running. Start it to hear the interviewers.".into()))?;
    }
    settings.persona = Some(crate::voice::vs::PersonaVoice { name: p.name.clone(), instruct: p.voice_instruct, states: p.voice_states, clip: p.clip });
    let engine = Engine::from_settings(&state, &settings).await?;
    let voice = engine.voice(&settings).await?;
    let line = format!("Hi, I'm {}, {}. I'll be one of the people interviewing you today.", p.first_name, p.role.split(" / ").next().unwrap_or(&p.role));
    crate::voice::audio::set_devices(&settings.input_device, &settings.output_device);
    state.voice.speak_as(&voice, &settings, &line).await
}
