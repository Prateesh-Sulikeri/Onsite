pub mod bank;
pub mod brain;
pub mod catalog;
mod commands;
pub mod error;
pub mod interview;
pub mod panels;
pub mod runner;
pub mod store;
pub mod voice;

use brain::ClaudeLocation;
use catalog::Catalog;
use error::{AppError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use store::Store;
use tauri::Manager;

const SETTINGS_KEY: &str = "app";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub claude: Option<ClaudeLocation>,
    pub interviewer_model: String,
    pub report_model: String,
    pub voice: voice::VoiceSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
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
    pub rounds: tokio::sync::Mutex<HashMap<i64, interview::live::ActiveRound>>,
    pub bank: bank::Bank,
    pub voice: voice::Voice,
}

impl AppState {
    pub fn settings(&self) -> Settings {
        self.store.lock().unwrap().get_setting(SETTINGS_KEY).ok().flatten().unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        voice::audio::set_devices(&s.voice.input_device, &s.voice.output_device);
        self.store.lock().unwrap().set_setting(SETTINGS_KEY, s)
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

/// Dev builds read `prompts/` from the repo; installed builds from bundled resources.
fn prompts_dir(app: &tauri::App) -> PathBuf {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../prompts");
    if cfg!(debug_assertions) && dev.is_dir() {
        return dev;
    }
    app.path().resource_dir().map(|d| d.join("prompts")).unwrap_or(dev)
}

/// Logs go to %LOCALAPPDATA%\Onsite\onsite.log (the app has no console window).
fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "onsite=info,onsite_lib=info".into());
    let dir = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("Onsite");
    let file = std::fs::create_dir_all(&dir).and_then(|_| {
        std::fs::OpenOptions::new().create(true).append(true).open(dir.join("onsite.log"))
    });
    match file {
        Ok(f) => tracing_subscriber::fmt().with_env_filter(filter).with_ansi(false).with_writer(Mutex::new(f)).init(),
        Err(_) => tracing_subscriber::fmt().with_env_filter(filter).init(),
    }
}

pub fn run() {
    init_logging();

    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db = data_dir.join("onsite.db");
            tracing::info!(db = %db.display(), "opening store");
            let prompts_dir = prompts_dir(app);
            voice::whisper::set_script_dir(voice::whisper::script_dir_for(app.path().resource_dir().ok().as_deref()));
            let catalog = Catalog::load(&prompts_dir)?;
            app.manage(AppState {
                store: Arc::new(Mutex::new(Store::open(&db)?)),
                catalog,
                panels: panels::Panels::load(&prompts_dir),
                bank: bank::Bank::load(&prompts_dir),
                prompts_dir,
                rounds: Default::default(),
                voice: Default::default(),
            });
            // Warm VoiceStudio up so the first spoken reply is not a 60 s cold start.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let settings = handle.state::<AppState>().settings().voice;
                voice::audio::set_devices(&settings.input_device, &settings.output_device);
                if settings.enabled {
                    // Loads Whisper onto the GPU (~10 s) while VoiceStudio warms up.
                    voice::whisper::start();
                    commands::warm_voice(&settings).await;
                    // Build panel voices now, so a round never waits on one (~5-15 s each, once).
                    let personas = handle.state::<AppState>().panels.all_personas();
                    let vs = voice::vs::VoiceStudio::new(&settings.base_url);
                    for p in personas {
                        let pv = voice::vs::PersonaVoice { name: p.name, instruct: p.voice_instruct, states: p.voice_states };
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
            commands::create_loop,
            commands::start_round,
            commands::send_answer,
            commands::end_round,
            commands::generate_report,
            commands::get_report,
            commands::get_round,
            commands::loop_verdict,
            commands::loop_feedback,
            commands::get_loop_feedback,
            commands::history,
            commands::delete_round,
            commands::run_code,
            commands::voice_status,
            commands::voice_launch,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Onsite");
}
