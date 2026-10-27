//! Research jobs: one web-enabled model call that reads recent candidate reports and writes a
//! dossier. Runs in the background (10-40 min on Claude Code, a few minutes on the APIs) with
//! real progress: searches and pages counted from tool calls, stages from PROGRESS lines.

use crate::brain::prompts::{self, Vars};
use crate::brain::stream::StreamEvent;
use crate::brain::{run_once_with_timeout, Tools};
use crate::dossier::Dossier;
use crate::error::{AppError, Result};
use crate::store::JobRow;
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

const RESEARCH_LIMIT: Duration = Duration::from_secs(45 * 60);
const LOG_MAX: usize = 8_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResearchRequest {
    pub company: String,
    pub role: String,
    pub level: String,
    #[serde(default)]
    pub jd: String,
}

/// What a running job is doing, kept in memory (the stored row has status, % and log).
#[derive(Debug, Clone, Default, Serialize)]
pub struct Live {
    pub stage: u8,
    pub note: String,
    pub searches: u32,
    pub reads: u32,
}

pub struct Running {
    pub live: Live,
    abort: tokio::task::AbortHandle,
}

/// One job as the UI shows it: the stored row plus live counters.
#[derive(Debug, Clone, Serialize)]
pub struct JobView {
    #[serde(flatten)]
    pub job: JobRow,
    pub live: Live,
}

/// `PROGRESS stage=3 pct=40 note=Reading LeetCode Discuss` → (3, 40, "Reading LeetCode Discuss").
pub fn parse_progress(line: &str) -> Option<(u8, u8, String)> {
    let rest = line.trim().trim_start_matches(['*', '`', '-', ' ']).strip_prefix("PROGRESS")?;
    let mut stage = None;
    let mut pct = None;
    let mut note = String::new();
    for (i, tok) in rest.split_whitespace().enumerate() {
        if let Some(v) = tok.strip_prefix("stage=") {
            stage = v.trim_matches(|c: char| !c.is_ascii_digit()).parse::<u8>().ok();
        } else if let Some(v) = tok.strip_prefix("pct=") {
            pct = v.trim_matches(|c: char| !c.is_ascii_digit()).parse::<u8>().ok();
        } else if tok.starts_with("note=") {
            // The note runs to the end of the line.
            let words: Vec<&str> = rest.split_whitespace().skip(i).collect();
            note = words.join(" ")["note=".len()..].trim_matches(['"', '`', '*']).to_string();
            break;
        }
    }
    let stage = stage?.clamp(1, 6);
    Some((stage, pct.unwrap_or(stage * 15).min(99), note))
}

/// Collects text pieces into lines and reports PROGRESS lines.
#[derive(Default)]
struct Lines {
    partial: String,
}

impl Lines {
    fn push(&mut self, text: &str, out: &mut Vec<String>) {
        self.partial.push_str(text);
        while let Some(i) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=i).collect();
            out.push(line);
        }
    }

    /// A whole message: its last line is complete too.
    fn push_message(&mut self, text: &str, out: &mut Vec<String>) {
        self.push(text, out);
        out.push(std::mem::take(&mut self.partial));
    }
}

fn emit(app: &AppHandle, id: i64) {
    let state = app.state::<AppState>();
    let job = state.store.lock().unwrap().job(id);
    let live = state.research.lock().unwrap().get(&id).map(|r| r.live.clone()).unwrap_or_default();
    if let Ok(job) = job {
        let _ = app.emit("research", JobView { job, live });
    }
}

fn update(app: &AppHandle, id: i64, f: impl FnOnce(&mut JobRow)) {
    let state = app.state::<AppState>();
    let store = state.store.lock().unwrap();
    if let Ok(mut job) = store.job(id) {
        f(&mut job);
        if job.log.len() > LOG_MAX {
            let cut = job.log.len() - LOG_MAX;
            let cut = (cut..job.log.len()).find(|&i| job.log.is_char_boundary(i)).unwrap_or(0);
            job.log.drain(..cut);
        }
        if let Err(e) = store.update_job(&job) {
            tracing::warn!(id, "saving research job: {e}");
        }
    }
}

pub fn validate(req: &ResearchRequest, state: &AppState) -> Result<()> {
    if req.company.trim().is_empty() || req.role.trim().is_empty() {
        return Err(AppError::Invalid("Name the company and the role.".into()));
    }
    if !state.catalog.levels.contains_key(&req.level) {
        return Err(AppError::Invalid("Unknown level.".into()));
    }
    Ok(())
}

pub fn start(app: &AppHandle, req: ResearchRequest) -> Result<i64> {
    let state = app.state::<AppState>();
    validate(&req, &state)?;
    let id = state.store.lock().unwrap().create_job(&json!(req))?;
    let handle = app.clone();
    {
        // Held while spawning, so the job is registered before it can report anything.
        let mut running = state.research.lock().unwrap();
        let task = tokio::spawn(async move { run(&handle, id, req).await });
        running.insert(id, Running { live: Live { stage: 1, note: "Reading your request".into(), ..Default::default() }, abort: task.abort_handle() });
    }
    emit(app, id);
    Ok(id)
}

pub fn cancel(app: &AppHandle, id: i64) {
    let state = app.state::<AppState>();
    if let Some(r) = state.research.lock().unwrap().remove(&id) {
        r.abort.abort();
    }
    update(app, id, |j| {
        if j.status == "running" {
            j.status = "cancelled".into();
            j.log.push_str("Cancelled.\n");
        }
    });
    emit(app, id);
}

/// Starts the same request again as a new job and drops the old one.
pub fn retry(app: &AppHandle, id: i64) -> Result<i64> {
    let state = app.state::<AppState>();
    let job = state.store.lock().unwrap().job(id)?;
    if job.status == "running" {
        return Err(AppError::Invalid("That research is still running.".into()));
    }
    let req: ResearchRequest = serde_json::from_value(job.request).map_err(|_| AppError::Invalid("That request can't be read. Start a new one.".into()))?;
    let new = start(app, req)?;
    state.store.lock().unwrap().delete_job(id)?;
    Ok(new)
}

pub fn jobs(state: &AppState) -> Result<Vec<JobView>> {
    let rows = state.store.lock().unwrap().jobs(20)?;
    let running = state.research.lock().unwrap();
    Ok(rows.into_iter().map(|job| JobView { live: running.get(&job.id).map(|r| r.live.clone()).unwrap_or_default(), job }).collect())
}

fn render_prompt(state: &AppState, req: &ResearchRequest) -> Result<String> {
    let template = prompts::load(&state.prompts_dir, "research.md")?;
    let round_types = state.catalog.round_keys().iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(" | ");
    let vars: Vars = [
        ("company", req.company.trim().to_string()),
        ("role", req.role.trim().to_string()),
        ("level_name", state.catalog.levels.get(&req.level).cloned().unwrap_or_else(|| req.level.clone())),
        ("jd", if req.jd.trim().is_empty() { "(none)".into() } else { req.jd.trim().chars().take(10_000).collect() }),
        ("round_types", round_types),
        ("today", chrono::Utc::now().format("%Y-%m-%d").to_string()),
    ]
    .into();
    Ok(prompts::render(&template, &vars))
}

async fn run(app: &AppHandle, id: i64, req: ResearchRequest) {
    let outcome = research(app, id, &req).await;
    let state = app.state::<AppState>();
    let live = state.research.lock().unwrap().remove(&id).map(|r| r.live).unwrap_or_default();
    let tally = format!("Searched {} times and read {} pages.\n", live.searches, live.reads);
    match outcome {
        Ok(dossier_id) => update(app, id, |j| {
            j.status = "done".into();
            j.progress = 100;
            j.dossier_id = Some(dossier_id);
            j.log.push_str(&tally);
            j.log.push_str("Dossier saved.\n");
        }),
        Err((msg, raw)) => {
            tracing::warn!(id, "research failed: {msg}");
            update(app, id, |j| {
                j.status = "failed".into();
                j.raw_output = raw.or(j.raw_output.take());
                j.log.push_str(&tally);
                j.log.push_str(&format!("Stopped: {msg}\n"));
            })
        }
    }
    emit(app, id);
    let _ = app.emit("dossiers_changed", ());
}

/// Returns the saved dossier's id, or (message, raw model text) on failure.
async fn research(app: &AppHandle, id: i64, req: &ResearchRequest) -> std::result::Result<String, (String, Option<String>)> {
    let state = app.state::<AppState>();
    let plain = |e: AppError| (e.to_string(), None);
    let prompt = render_prompt(&state, req).map_err(plain)?;
    let backend = state.backend().await.map_err(plain)?;
    let model = state.settings().report_model;
    update(app, id, |j| j.log.push_str(&format!("Using {} ({model}).\n", backend.describe())));
    emit(app, id);

    let mut lines = Lines::default();
    let on_event = |ev: &StreamEvent| {
        let mut done = Vec::new();
        let mut counted = false;
        match ev {
            StreamEvent::ToolUse { name } if name == "WebSearch" || name == "WebFetch" => {
                if let Some(r) = state.research.lock().unwrap().get_mut(&id) {
                    if name == "WebSearch" { r.live.searches += 1 } else { r.live.reads += 1 }
                }
                counted = true;
            }
            StreamEvent::TextDelta(t) => lines.push(t, &mut done),
            StreamEvent::AssistantText(t) => lines.push_message(t, &mut done),
            _ => {}
        }
        let progress: Vec<_> = done.iter().filter_map(|l| parse_progress(l)).collect();
        for (stage, pct, note) in &progress {
            if let Some(r) = state.research.lock().unwrap().get_mut(&id) {
                r.live.stage = r.live.stage.max(*stage);
                if !note.is_empty() {
                    r.live.note = note.clone();
                }
            }
            update(app, id, |j| {
                j.progress = j.progress.max(*pct as i64);
                if !note.is_empty() {
                    j.log.push_str(&format!("{note}\n"));
                }
            });
        }
        if counted || !progress.is_empty() {
            emit(app, id);
        }
    };
    let out = run_once_with_timeout(&backend, &prompt, Tools::Web, &model, RESEARCH_LIMIT, on_event).await.map_err(plain)?;
    let raw = Some(out.text.clone());
    let Some(json) = out.json.filter(Value::is_object) else {
        return Err(("The result had no dossier in it.".into(), raw));
    };
    let mut d = Dossier::from_value(&json, &state.catalog.round_keys()).map_err(|e| (format!("The dossier isn't complete. {e}"), raw.clone()))?;
    // Ours, not the model's: the level picked and the posting given.
    d.level = req.level.clone();
    d.id = crate::dossier::slug(&d.company, &d.role, &d.level);
    if d.jd.is_empty() {
        d.jd = req.jd.trim().to_string();
    }
    d.prepared_at = chrono::Utc::now().to_rfc3339();
    save(&state, &d).map_err(plain)?;
    Ok(d.id)
}

/// Saves to SQLite and writes `<app data>/dossiers/<id>.json` as a plain copy.
pub fn save(state: &AppState, d: &Dossier) -> Result<()> {
    let v = d.to_value();
    state.store.lock().unwrap().save_dossier(&d.id, &d.company, &d.role, &d.level, &v)?;
    let dir = state.data_dir.join("dossiers");
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(format!("{}.json", d.id)), serde_json::to_string_pretty(&v).unwrap_or_default())) {
        tracing::warn!(id = %d.id, "writing dossier file: {e}");
    }
    Ok(())
}

pub fn import(state: &AppState, text: &str) -> Result<Dossier> {
    let v: Value = serde_json::from_str(text).map_err(|_| AppError::Invalid("That file isn't JSON.".into()))?;
    let d = Dossier::from_value(&v, &state.catalog.round_keys()).map_err(|e| AppError::Invalid(format!("That isn't an Onsite dossier. {e}")))?;
    save(state, &d)?;
    Ok(d)
}

/// First run: the dossiers shipped with the app go into the store once.
pub fn seed(state: &AppState, dir: &std::path::Path) {
    const KEY: &str = "dossiers_seeded";
    if state.store.lock().unwrap().get_setting::<bool>(KEY).ok().flatten().is_some() {
        return;
    }
    let files = std::fs::read_dir(dir).map(|r| r.filter_map(|e| e.ok()).map(|e| e.path()).collect::<Vec<_>>()).unwrap_or_default();
    for f in files.iter().filter(|f| f.extension().is_some_and(|x| x == "json")) {
        match std::fs::read_to_string(f).map_err(AppError::from).and_then(|t| import(state, &t)) {
            Ok(d) => tracing::info!(id = %d.id, "imported shipped dossier"),
            Err(e) => tracing::warn!(file = %f.display(), "shipped dossier: {e}"),
        }
    }
    let _ = state.store.lock().unwrap().set_setting(KEY, &true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_lines() {
        assert_eq!(parse_progress("PROGRESS stage=3 pct=40 note=Reading LeetCode Discuss threads"), Some((3, 40, "Reading LeetCode Discuss threads".into())));
        assert_eq!(parse_progress("  **PROGRESS stage=6 pct=100 note=Done**"), Some((6, 99, "Done".into())));
        assert_eq!(parse_progress("PROGRESS stage=2"), Some((2, 30, String::new())));
        assert_eq!(parse_progress("Progress is good"), None);
        assert_eq!(parse_progress("PROGRESS pct=10"), None);
    }

    #[test]
    fn lines_join_pieces() {
        let mut l = Lines::default();
        let mut out = Vec::new();
        l.push("PROGRESS stage=1 ", &mut out);
        l.push("note=hi\nnext", &mut out);
        assert_eq!(out, ["PROGRESS stage=1 note=hi\n"]);
        l.push_message(" line", &mut out);
        assert_eq!(out.last().unwrap(), "next line");
    }
}
