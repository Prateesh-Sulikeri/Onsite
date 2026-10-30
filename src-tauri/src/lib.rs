pub mod ats;
pub mod bank;
pub mod brain;
pub mod catalog;
mod commands;
pub mod dossier;
pub mod error;
pub mod interview;
pub mod panels;
pub mod research;
pub mod runner;
pub mod storage;
pub mod store;
pub mod updates;
pub mod voice;

use brain::{ApiClient, Backend, ClaudeLocation, Provider};
use catalog::Catalog;
use error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use store::Store;
use tauri::Manager;

const SETTINGS_KEY: &str = "app";
/// Provider id → API key. Kept apart from `Settings`, which the UI reads in full.
const KEYS_KEY: &str = "api_keys";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where model calls go. Older installs have no value and keep Claude Code.
    pub provider: Provider,
    pub claude: Option<ClaudeLocation>,
    pub interviewer_model: String,
    pub report_model: String,
    pub voice: voice::VoiceSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: Provider::ClaudeCode,
            claude: None,
            interviewer_model: "sonnet".into(),
            report_model: "opus".into(),
            voice: Default::default(),
        }
    }
}

pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub catalog: Catalog,
    pub panels: panels::Panels,
    pub prompts_dir: PathBuf,
    /// The app's data folder; dossier files are written under it.
    pub data_dir: PathBuf,
    /// Research jobs running now, by job id.
    pub research: Mutex<HashMap<i64, research::Running>>,
    pub rounds: tokio::sync::Mutex<HashMap<i64, interview::live::ActiveRound>>,
    pub bank: bank::Bank,
    pub voice: voice::Voice,
    /// Built-in voice engine models, loaded on first use and kept.
    pub builtin: voice::builtin::Cache,
    /// Bumped by every round start and by `cancel_start`; a start whose number is stale stops loading.
    pub prep_gen: std::sync::atomic::AtomicU64,
}

impl AppState {
    pub fn settings(&self) -> Settings {
        self.store.lock().unwrap().get_setting(SETTINGS_KEY).ok().flatten().unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        voice::audio::set_devices(&s.voice.input_device, &s.voice.output_device);
        self.store.lock().unwrap().set_setting(SETTINGS_KEY, s)
    }

    pub fn api_keys(&self) -> HashMap<String, String> {
        self.store.lock().unwrap().get_setting(KEYS_KEY).ok().flatten().unwrap_or_default()
    }

    /// An empty key removes it.
    pub fn set_api_key(&self, p: Provider, key: &str) -> Result<()> {
        let mut keys = self.api_keys();
        match key.trim() {
            "" => keys.remove(p.id()),
            k => keys.insert(p.id().to_string(), k.to_string()),
        };
        self.store.lock().unwrap().set_setting(KEYS_KEY, &keys)
    }

    /// Where this call should go: Claude Code, or the chosen API with its key.
    pub async fn backend(&self) -> Result<Backend> {
        let p = self.settings().provider;
        if p == Provider::ClaudeCode {
            return Ok(Backend::Cli(self.claude_location().await?));
        }
        match self.api_keys().get(p.id()) {
            Some(k) if !k.trim().is_empty() => Ok(Backend::Api(ApiClient::new(p, k))),
            _ => Err(AppError::Ai(format!("Add your {} key in Settings first.", p.label()))),
        }
    }

    /// The saved location, or auto-detect once and remember it.
    pub async fn claude_location(&self) -> Result<ClaudeLocation> {
        let mut s = self.settings();
        match s.claude.clone() {
            // A saved .exe that has since moved or been uninstalled: detect again.
            Some(ClaudeLocation::Windows { exe }) if !exe.is_file() => {
                tracing::warn!(exe = %exe.display(), "saved Claude Code path is gone; detecting again");
            }
            Some(loc) => return Ok(loc),
            None => {}
        }
        let loc = brain::locate::detect().await.ok_or(AppError::ClaudeNotFound)?;
        tracing::info!(location = %loc.describe(), "detected Claude Code");
        s.claude = Some(loc.clone());
        self.save_settings(&s)?;
        Ok(loc)
    }
}

/// Dev builds read `prompts/` (and `dossiers/`) from the repo; installed builds from bundled resources.
fn resource(app: &tauri::App, name: &str) -> PathBuf {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(name);
    if cfg!(debug_assertions) && dev.is_dir() {
        return dev;
    }
    app.path().resource_dir().map(|d| d.join(name)).unwrap_or(dev)
}

/// Logs go to %LOCALAPPDATA%\Onsite\onsite.log (the app has no console window).
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "onsite=info,onsite_lib=info".into());
    let path = storage::log_path();
    let file = std::fs::create_dir_all(path.parent().unwrap_or(&path)).and_then(|_| std::fs::OpenOptions::new().create(true).append(true).open(&path));
    match file {
        Ok(f) => tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(Mutex::new(f)).init(),
        Err(_) => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
}

/// Apps started from the Dock, Finder or a desktop launcher get a bare PATH (/usr/bin:/bin…),
/// so `claude`, `node`, `java` and `g++` from Homebrew, nvm or ~/.local/bin aren't found.
/// Take PATH from the user's login shell once at startup, as a terminal would have it.
#[cfg(unix)]
fn path_from_login_shell() {
    use std::io::Read;
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/sh".into());
    let child = std::process::Command::new(&shell)
        .args(["-ilc", "printf '__PATH__%s__PATH__' \"$PATH\""])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();
    let Ok(mut child) = child else { return };
    // A slow or prompting shell profile must not hold up the app.
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(3) {
        if let Ok(Some(_)) = child.try_wait() {
            let mut out = String::new();
            let _ = child.stdout.take().map(|mut o| o.read_to_string(&mut out));
            if let Some(path) = out.split("__PATH__").nth(1).filter(|p| !p.is_empty()) {
                tracing::info!("PATH from {shell}");
                std::env::set_var("PATH", path);
            }
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let _ = child.kill();
    tracing::warn!("{shell} took too long to report PATH; keeping the app's own");
}

pub fn run() {
    init_logging();
    #[cfg(unix)]
    path_from_login_shell();

    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db = data_dir.join("onsite.db");
            tracing::info!(db = %db.display(), "opening store");
            let prompts_dir = resource(app, "prompts");
            voice::whisper::set_script_dir(voice::whisper::script_dir_for(app.path().resource_dir().ok().as_deref()));
            let catalog = Catalog::load(&prompts_dir)?;
            let store = Store::open(&db)?;
            match store.delete_unanswered_rounds() {
                Ok(0) => {}
                Ok(n) => tracing::info!(n, "deleted rounds with no answers"),
                Err(e) => tracing::warn!("sweeping empty rounds: {e}"),
            }
            if let Err(e) = store.fail_running_jobs() {
                tracing::warn!("marking interrupted research jobs: {e}");
            }
            app.manage(AppState {
                store: Arc::new(Mutex::new(store)),
                catalog,
                panels: panels::Panels::load(&prompts_dir),
                bank: bank::Bank::load(&prompts_dir),
                prompts_dir,
                data_dir: data_dir.clone(),
                research: Default::default(),
                rounds: Default::default(),
                voice: Default::default(),
                builtin: Default::default(),
                prep_gen: Default::default(),
            });
            research::seed(&app.state::<AppState>(), &resource(app, "dossiers"));
            if let Err(e) = storage::enforce(&app.state::<AppState>()) {
                tracing::warn!("storage limits: {e}");
            }
            // Warm VoiceStudio up so the first spoken reply is not a 60 s cold start.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let settings = handle.state::<AppState>().settings().voice;
                voice::audio::set_devices(&settings.input_device, &settings.output_device);
                if settings.enabled {
                    // Loads Whisper onto the GPU (~10 s) while VoiceStudio warms up.
                    voice::whisper::start();
                }
                // The built-in engine loads its models when a round starts. Nothing to warm up here.
                if settings.enabled && settings.engine == voice::engine::VoiceEngine::Voicestudio {
                    commands::warm_voice(&settings).await;
                    // Build panel voices now, so a round never waits on one (~5-15 s each, once).
                    let personas = handle.state::<AppState>().panels.all_personas();
                    let vs = voice::vs::VoiceStudio::new(&settings.base_url);
                    for p in personas {
                        let pv = voice::vs::PersonaVoice { name: p.name, instruct: p.voice_instruct, states: p.voice_states, clip: p.clip };
                        if let Err(e) = vs.persona_voice(&pv).await {
                            tracing::warn!(name = %pv.name, "interviewer voice prebuild: {e}");
                        }
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_catalog,
            commands::get_panels,
            commands::ui_log,
            commands::get_settings,
            commands::save_settings,
            commands::detect_claude,
            commands::health_check,
            commands::ai_keys,
            commands::set_api_key,
            commands::set_provider,
            commands::list_models,
            commands::list_dossiers,
            commands::delete_dossier,
            commands::import_dossier,
            commands::export_dossier,
            commands::research_start,
            commands::research_cancel,
            commands::research_retry,
            commands::research_dismiss,
            commands::research_jobs,
            commands::app_version,
            commands::check_updates,
            commands::install_update,
            commands::voicestudio_version,
            commands::storage_info,
            commands::set_storage_limits,
            commands::clear_log,
            commands::clear_temp,
            commands::vacuum_db,
            commands::open_folder,
            commands::get_backdrop,
            commands::set_backdrop,
            commands::clear_backdrop,
            commands::voice_persona_preview,
            commands::create_loop,
            commands::start_round,
            commands::cancel_start,
            commands::send_answer,
            commands::end_round,
            commands::generate_report,
            commands::get_report,
            commands::get_round,
            commands::loop_verdict,
            commands::loop_feedback,
            commands::get_loop_feedback,
            commands::history,
            commands::get_goal,
            commands::set_goal,
            commands::delete_round,
            commands::run_code,
            commands::set_round_lang,
            commands::voice_status,
            commands::voice_launch,
            commands::voice_model_download,
            commands::voice_model_delete,
            commands::voice_models_reload,
            commands::voice_builtin_preview,
            commands::voice_test_speak,
            commands::voice_test_listen,
            commands::voice_finish,
            commands::voice_mute,
            commands::voice_cancel_listen,
            commands::voice_interrupt,
            commands::voice_set_devices,
            commands::audio_devices,
            commands::mic_monitor_start,
            commands::mic_monitor_stop,
            commands::read_document,
            commands::ats_check,
            commands::ats_screen,
            commands::ats_last,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Onsite");
}
