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
use tauri::{AppHandle, State};

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
