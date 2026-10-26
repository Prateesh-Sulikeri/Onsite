pub mod brain;
pub mod catalog;
mod commands;
pub mod error;
pub mod interview;
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
    pub prompts_dir: PathBuf,
    pub rounds: tokio::sync::Mutex<HashMap<i64, interview::live::ActiveRound>>,
    pub voice: voice::Voice,
}

impl AppState {
    pub fn settings(&self) -> Settings {
        self.store.lock().unwrap().get_setting(SETTINGS_KEY).ok().flatten().unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> Result<()> {
        self.store.lock().unwrap().set_setting(SETTINGS_KEY, s)
    }

    /// The saved location, or auto-detect once and remember it.
    pub async fn claude_location(&self) -> Result<ClaudeLocation> {
        let mut s = self.settings();
        if let Some(loc) = s.claude.clone() {
            return Ok(loc);
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

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "onsite=info,onsite_lib=info".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db = data_dir.join("onsite.db");
            tracing::info!(db = %db.display(), "opening store");
            let prompts_dir = prompts_dir(app);
            let catalog = Catalog::load(&prompts_dir)?;
            app.manage(AppState {
                store: Arc::new(Mutex::new(Store::open(&db)?)),
                catalog,
                prompts_dir,
                rounds: Default::default(),
                voice: Default::default(),
            });
            // Warm VoiceStudio up so the first spoken reply is not a 60 s cold start.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let settings = handle.state::<AppState>().settings().voice;
                if settings.enabled {
                    commands::warm_voice(&settings).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_catalog,
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
            commands::history,
            commands::voice_status,
            commands::voice_launch,
            commands::voice_test_speak,
            commands::voice_test_listen,
            commands::voice_finish,
            commands::voice_mute,
            commands::voice_cancel_listen,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Onsite");
}
