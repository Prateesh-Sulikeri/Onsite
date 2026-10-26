//! Running rounds: owns the Claude session, forwards its events to the UI, saves every turn.

use super::report::{metrics, report_prompt, round_summary, verdict_prompt, ReportInput};
use super::{candidate_message, interviewer_prompt, opening_message, LoopSetup, PlannedRound, Workspace};
use crate::brain::{prompts, run_once, ClaudeSession, SessionConfig, SessionEvent, Tools};
use crate::error::{AppError, Result};
use crate::store::Turn;
use crate::AppState;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc;

pub struct ActiveRound {
    session: ClaudeSession,
    cfg_prompt: String,
    model: String,
    minutes: u32,
    lang: String,
    last_ws: Workspace,
    log: Arc<RoundLog>,
}

struct RoundLog {
    round_id: i64,
    started: Instant,
    turns: Mutex<Vec<Turn>>,
    workspace: Mutex<Workspace>,
}

impl RoundLog {
    fn elapsed_min(&self) -> f64 {
        self.started.elapsed().as_secs_f64() / 60.0
    }

    fn persist(&self, state: &AppState, session_id: Option<&str>) {
        let turns = self.turns.lock().unwrap().clone();
        let ws = serde_json::to_value(&*self.workspace.lock().unwrap()).unwrap_or_default();
        if let Err(e) = state.store.lock().unwrap().save_round_progress(self.round_id, &turns, &ws, session_id) {
            tracing::error!(round_id = self.round_id, "saving round failed: {e}");
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RoundInfo {
    pub round_id: i64,
    pub loop_id: i64,
    pub index: usize,
    pub count: usize,
    pub plan: PlannedRound,
    pub company: String,
    pub level: String,
    pub lang: String,
}

pub fn create_loop(state: &AppState, setup: &LoopSetup) -> Result<i64> {
    setup.validate(&state.catalog)?;
    let mut config = serde_json::to_value(setup)?;
    config["company_name"] = json!(setup.company_name(&state.catalog));
    state.store.lock().unwrap().create_loop(&config)
}

fn load_setup(state: &AppState, loop_id: i64) -> Result<LoopSetup> {
    let config = state.store.lock().unwrap().loop_config(loop_id)?;
    Ok(serde_json::from_value(config)?)
}

pub async fn start_round(app: &AppHandle, loop_id: i64, index: usize) -> Result<RoundInfo> {
    let state = app.state::<AppState>();
    let setup = load_setup(&state, loop_id)?;
    if index >= setup.rounds.len() {
        return Err(AppError::Invalid("No such round in this loop.".into()));
    }
    let cat = &state.catalog;
    let plan = setup.plan(cat, index);
    let template = prompts::load(&state.prompts_dir, "interviewer.md")?;
    let system_prompt = interviewer_prompt(&template, cat, &setup, index);
    let settings = state.settings();
    let location = state.claude_location().await?;

    let round_id = state.store.lock().unwrap().create_round(
        loop_id,
        index,
        &plan.key,
        &plan.name,
        &plan.persona,
        plan.minutes,
        &settings.interviewer_model,
    )?;
    let log = Arc::new(RoundLog {
        round_id,
        started: Instant::now(),
        turns: Mutex::new(Vec::new()),
        workspace: Mutex::new(Workspace::default()),
    });

    let cfg = SessionConfig {
        location,
        model: settings.interviewer_model.clone(),
        system_prompt: system_prompt.clone(),
        resume: None,
    };
    let voice_up = crate::voice::vs::VoiceStudio::new(&settings.voice.base_url).health().await.is_ok();
    if settings.voice.enabled && voice_up {
        if let Err(e) = state.voice.begin(app, round_id, settings.voice.clone()) {
            tracing::warn!("voice off for this round: {e}");
            let _ = app.emit("voice", &crate::voice::VoiceEvent::Error { message: e.to_string() });
        }
    }
    let mut session = spawn_session(app, round_id, cfg, log.clone()).await?;
    session.send(&opening_message(plan.minutes)).await?;

    state.rounds.lock().await.insert(
        round_id,
        ActiveRound {
            session,
            cfg_prompt: system_prompt,
            model: settings.interviewer_model,
            minutes: plan.minutes,
            lang: setup.lang.clone(),
            last_ws: Workspace::default(),
            log,
        },
    );
    Ok(RoundInfo {
        round_id,
        loop_id,
        index,
        count: setup.rounds.len(),
        company: setup.company_name(cat),
        level: cat.levels[&setup.level].clone(),
        lang: setup.lang,
        plan,
    })
}

async fn spawn_session(app: &AppHandle, round_id: i64, cfg: SessionConfig, log: Arc<RoundLog>) -> Result<ClaudeSession> {
    let (tx, mut rx) = mpsc::unbounded_channel::<SessionEvent>();
    let session = ClaudeSession::start(round_id, cfg, tx).await?;
    let app = app.clone();
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            let state = app.state::<AppState>();
            match &ev {
                SessionEvent::Started { session_id, .. } => log.persist(&state, Some(session_id)),
                SessionEvent::Sentence { text, .. } => state.voice.say(round_id, text),
                SessionEvent::Error { .. } => state.voice.turn_done(round_id, false),
                SessionEvent::TurnDone { text, ttft_ms, ended, .. } => {
                    state.voice.turn_done(round_id, *ended);
                    log.turns.lock().unwrap().push(Turn {
                        who: "iv".into(),
                        text: text.clone(),
                        at_min: log.elapsed_min(),
                        ttft_ms: Some(*ttft_ms),
                        shared_workspace: false,
                        sent: None,
                    });
                    log.persist(&state, None);
                }
                _ => {}
            }
            if let Err(e) = app.emit("round", &ev) {
                tracing::error!("emit failed: {e}");
            }
        }
    });
    Ok(session)
}

/// Sends the candidate's turn. Restarts Claude with `--resume` if it died.
pub async fn send_answer(app: &AppHandle, round_id: i64, text: String, ws: Workspace, force: bool) -> Result<Option<Turn>> {
    let state = app.state::<AppState>();
    let mut rounds = state.rounds.lock().await;
    let r = rounds.get_mut(&round_id).ok_or_else(|| AppError::NotFound("This round is not running.".into()))?;
    *r.log.workspace.lock().unwrap() = ws.clone();
    let elapsed = r.log.elapsed_min();
    let Some((content, shared)) = candidate_message(&text, &ws, &mut r.last_ws, force, &r.lang, elapsed, r.minutes) else {
        return Ok(None);
    };

    if !r.session.is_alive() {
        let resume = r.session.session_id();
        tracing::warn!(round_id, ?resume, "claude session died; restarting");
        let cfg = SessionConfig {
            location: state.claude_location().await?,
            model: r.model.clone(),
            system_prompt: r.cfg_prompt.clone(),
            resume,
        };
        r.session = spawn_session(app, round_id, cfg, r.log.clone()).await?;
    }

    let turn = Turn {
        who: "me".into(),
        text: text.trim().to_string(),
        at_min: elapsed,
        ttft_ms: None,
        shared_workspace: shared,
        sent: Some(content.clone()),
    };
    r.log.turns.lock().unwrap().push(turn.clone());
    r.log.persist(&state, None);
    r.session.send(&content).await?;
    Ok(Some(turn))
}

pub async fn end_round(app: &AppHandle, round_id: i64, ws: Workspace) -> Result<()> {
    let state = app.state::<AppState>();
    state.voice.end();
    let active = state.rounds.lock().await.remove(&round_id);
    if let Some(r) = active {
        *r.log.workspace.lock().unwrap() = ws;
        r.log.persist(&state, r.session.session_id().as_deref());
        r.session.shutdown().await;
    }
    state.store.lock().unwrap().finish_round(round_id)?;
    Ok(())
}

pub async fn generate_report(app: &AppHandle, round_id: i64) -> Result<Value> {
    let state = app.state::<AppState>();
    let round = state.store.lock().unwrap().round(round_id)?;
    let setup = load_setup(&state, round.loop_id)?;
    let cat = &state.catalog;
    let actual = match (
        chrono::DateTime::parse_from_rfc3339(&round.started_at),
        round.ended_at.as_deref().map(chrono::DateTime::parse_from_rfc3339),
    ) {
        (Ok(s), Some(Ok(e))) => (e - s).num_seconds() as f64 / 60.0,
        _ => round.transcript.last().map(|t| t.at_min).unwrap_or(0.0),
    };
    let m = metrics(&round.transcript, actual);
    let rubric = cat.rounds.get(&round.round_key).map(|r| r.rubric.clone()).unwrap_or_default();
    let template = prompts::load(&state.prompts_dir, "report.md")?;
    let prompt = report_prompt(
        &template,
        &ReportInput {
            round_name: &round.name,
            round_minutes: round.minutes,
            company: &setup.company_name(cat),
            level_name: &cat.levels[&setup.level],
            level_bar: &cat.level_bar[&setup.level],
            rubric: &rubric,
            lang: &setup.lang,
            turns: &round.transcript,
            workspace: &round.workspace,
            metrics: &m,
        },
    );
    let location = state.claude_location().await?;
    let out = run_once(&location, &prompt, Tools::None, &state.settings().report_model, |_| {}).await?;
    tracing::info!(round_id, ms = out.duration_ms, model = %out.model, "report generated");
    let mut report = out.json.clone().filter(Value::is_object).ok_or_else(|| {
        let _ = state.store.lock().unwrap().save_report(round_id, None, &out.text);
        AppError::Claude("The report came back in the wrong format.".into())
    })?;
    report["metrics"] = serde_json::to_value(&m)?;
    state.store.lock().unwrap().save_report(round_id, Some(&report), &out.text)?;
    Ok(report)
}

pub async fn loop_verdict(app: &AppHandle, loop_id: i64) -> Result<Value> {
    let state = app.state::<AppState>();
    let setup = load_setup(&state, loop_id)?;
    let summaries: Vec<String> = {
        let store = state.store.lock().unwrap();
        let mut out = Vec::new();
        for id in store.loop_round_ids(loop_id)? {
            if let Some((Some(rep), _)) = store.report(id)? {
                out.push(round_summary(&store.round(id)?.name, &rep));
            }
        }
        out
    };
    if summaries.is_empty() {
        return Err(AppError::Invalid("Finish at least one round first.".into()));
    }
    let cat = &state.catalog;
    let template = prompts::load(&state.prompts_dir, "verdict.md")?;
    let prompt = verdict_prompt(&template, &setup.company_name(cat), &cat.levels[&setup.level], &summaries);
    let location = state.claude_location().await?;
    let out = run_once(&location, &prompt, Tools::None, &state.settings().report_model, |_| {}).await?;
    let verdict = out.json.filter(Value::is_object).ok_or_else(|| AppError::Claude("The verdict came back in the wrong format.".into()))?;
    state.store.lock().unwrap().set_loop_verdict(loop_id, &verdict)?;
    Ok(verdict)
}
