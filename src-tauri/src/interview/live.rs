//! Running rounds: owns the Claude session, forwards its events to the UI, saves every turn.

use super::report::{enough_to_review, metrics, report_prompt, round_summary, verdict_prompt, ReportInput};
use super::{candidate_message, interviewer_prompt, opening_excerpt, opening_message, pick_opener, session_block, LoopSetup, PlannedRound, PromptTemplates, Workspace};
use crate::bank::{Question, Test};
use crate::brain::{prompts, run_once, Session, SessionConfig, SessionEvent, Tools};
use crate::runner::{self, Kind, Lang};
use crate::error::{AppError, Result};
use crate::store::Turn;
use crate::AppState;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{mpsc, oneshot};

/// A reply older than this no longer blocks the next answer (Claude may be hung).
const REPLY_STALE: Duration = Duration::from_secs(120);

pub struct ActiveRound {
    session: Session,
    /// The DSA problem for a coding round, from the local bank.
    question: Option<Question>,
    /// Full-size tests for Submit, built once per round.
    stress: Option<Vec<Test>>,
    /// Latest Run/Submit result, sent with the candidate's next message.
    pending_run: Option<String>,
    cfg_prompt: String,
    model: String,
    minutes: u32,
    lang: String,
    last_ws: Workspace,
    log: Arc<RoundLog>,
}

struct RoundLog {
    round_id: i64,
    /// Reset when the loader finishes, so preparation time doesn't count against the round.
    started: Mutex<Instant>,
    turns: Mutex<Vec<Turn>>,
    workspace: Mutex<Workspace>,
}

impl RoundLog {
    fn elapsed_min(&self) -> f64 {
        self.started.lock().unwrap().elapsed().as_secs_f64() / 60.0
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
    /// The voice loop really started (VoiceStudio up and the speaker opened).
    pub voice: bool,
    /// Coding rounds: the problem as the candidate sees it.
    pub problem: Option<Value>,
}

/// Problem difficulty for a level, until there is a picker.
fn difficulty_for(level: &str) -> &'static str {
    match level {
        "ng" => "Easy",
        "senior" | "staff" => "Hard",
        _ => "Medium",
    }
}

/// What the candidate sees: no hidden tests, reference or interviewer hints.
fn problem_view(q: &Question, lang: &str) -> Value {
    let runner = Lang::from_name(lang);
    json!({
        "id": q.id,
        "title": q.title,
        "difficulty": q.difficulty,
        "topics": q.topics,
        "statement": q.statement,
        "constraints": q.constraints,
        "examples": q.examples().map(|t| json!({
            "input": q.input_text(&t.args),
            "output": t.expected,
            "explanation": t.explanation,
        })).collect::<Vec<_>>(),
        "hidden": q.tests.iter().filter(|t| !t.example).count(),
        "runnable": runner.is_some(),
        "starter": runner.map(|l| runner::starter(q, l)).unwrap_or_default(),
    })
}

/// Added to the interviewer's system prompt for a coding round.
fn problem_block(q: &Question) -> String {
    let examples = q
        .examples()
        .map(|t| format!("- Input: {} → Output: {}", q.input_text(&t.args), t.expected))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "\n\nTHE PROBLEM FOR THIS ROUND (this replaces choosing one yourself): {title} ({difficulty}).\n\
         The candidate already sees it on screen, above your chat, with the full statement, examples and constraints. \
         Introduce it in one or two spoken sentences and tell them it is on their screen. Never call it a tab. Do not read it out in full or paste it.\n\
         Statement:\n{statement}\nExamples:\n{examples}\nConstraints:\n{constraints}\n\
         Optimal: time {time}, space {space}.\n\
         Hints, one at a time and only if they are stuck: {hints}\nFollow-ups if time allows: {follow}\n\
         They can Run their code on the examples or Submit it against hidden and full-size tests on their own machine. \
         A [Code run] block in their message is the real result of that. React to it like an interviewer watching their screen.",
        title = q.title,
        difficulty = q.difficulty,
        statement = q.statement,
        constraints = q.constraints.join("\n"),
        time = q.time,
        space = q.space,
        hints = q.hints.join(" | "),
        follow = q.follow_ups.join(" | "),
    )
}

/// One line of the loader, sent as the `prep` event. `state`: run, ok, warn, fail or off.
#[derive(Debug, Clone, Serialize)]
struct Prep<'a> {
    step: &'a str,
    state: &'a str,
    detail: &'a str,
}

fn prep(app: &AppHandle, step: &str, state: &str, detail: &str) {
    let _ = app.emit("prep", Prep { step, state, detail });
}

/// Loader steps that need VoiceStudio.
const VOICE_STEPS: [&str; 5] = ["voice", "stt", "whisper", "speaker", "mic"];

/// Opens the speaker and starts the voice loop, if spoken interviews are on and VoiceStudio is up.
async fn start_voice(app: &AppHandle, round_id: i64, settings: &crate::voice::VoiceSettings) -> Option<crate::voice::VoiceStart> {
    let state = app.state::<AppState>();
    let skip = |why: &str| VOICE_STEPS.iter().for_each(|s| prep(app, s, "off", why));
    if !settings.enabled {
        prep(app, "voicestudio", "off", "Spoken interviews are off");
        skip("Typed round");
        return None;
    }
    prep(app, "voicestudio", "run", "Connecting");
    let health = match crate::voice::vs::VoiceStudio::new(&settings.base_url).health().await {
        Ok(h) => h,
        Err(_) => {
            prep(app, "voicestudio", "fail", "Not running. This round will be typed.");
            skip("Needs VoiceStudio");
            return None;
        }
    };
    let device = health["device"].as_str().unwrap_or("Running");
    prep(app, "voicestudio", "ok", device);
    match state.voice.begin(app, round_id, settings.clone()) {
        Ok(v) => {
            prep(app, "speaker", "ok", "Open");
            Some(v)
        }
        Err(e) => {
            tracing::warn!("voice off for this round: {e}");
            prep(app, "speaker", "fail", &e.to_string());
            ["voice", "stt", "whisper", "mic"].iter().for_each(|s| prep(app, s, "off", "Needs the speaker"));
            None
        }
    }
}

/// Waits for Claude to start and write the first sentence of its reply.
async fn wait_claude(
    app: &AppHandle,
    started: oneshot::Receiver<std::result::Result<String, String>>,
    reply: oneshot::Receiver<std::result::Result<(), String>>,
) -> Result<()> {
    let t = Instant::now();
    let fail = |m: &str| {
        prep(app, "claude", "fail", m);
        prep(app, "question", "off", "");
        AppError::Ai(m.to_string())
    };
    prep(app, "claude", "run", "Starting the interviewer");
    match tokio::time::timeout(Duration::from_secs(60), started).await {
        Ok(Ok(Ok(model))) => prep(app, "claude", "ok", &model),
        Ok(Ok(Err(m))) if !m.is_empty() => return Err(fail(&m)),
        _ => return Err(fail("The interviewer didn't start. Run the health check in Settings.")),
    }
    prep(app, "question", "run", "Writing the first question");
    let m = match tokio::time::timeout(Duration::from_secs(150), reply).await {
        Ok(Ok(Ok(()))) => {
            prep(app, "question", "ok", &format!("First sentence in {:.1} s", t.elapsed().as_secs_f64()));
            return Ok(());
        }
        Ok(Ok(Err(m))) => m,
        Ok(Err(_)) => "The interviewer stopped.".to_string(),
        Err(_) => "No reply after two and a half minutes.".to_string(),
    };
    prep(app, "question", "fail", &m);
    Err(AppError::Ai(m))
}

/// Waits for the interviewer's voice, speech recognition and the mic. Returns `go` when the
/// round can be spoken; None makes it a typed round.
async fn prepare_voice(app: &AppHandle, settings: &crate::voice::VoiceSettings, start: Option<crate::voice::VoiceStart>) -> Option<oneshot::Sender<()>> {
    use crate::voice::{audio, stt, vs::VoiceStudio, whisper::{self, WhisperStatus}};
    let crate::voice::VoiceStart { ready, go } = start?;
    let vs = VoiceStudio::new(&settings.base_url);
    let voice = async {
        prep(app, "voice", "run", "Loading the voice model. Up to a minute when VoiceStudio is cold.");
        let (state, detail) = match tokio::time::timeout(Duration::from_secs(240), ready).await {
            Ok(Ok(Ok(name))) => ("ok", name),
            Ok(Ok(Err(e))) => ("fail", e),
            Ok(Err(_)) => ("fail", "The voice loop stopped.".to_string()),
            Err(_) => ("fail", "VoiceStudio took over four minutes.".to_string()),
        };
        prep(app, "voice", state, &detail);
        state == "ok"
    };
    let recognizer = async {
        prep(app, "stt", "run", "Choosing the speech model");
        match stt::prepare(&vs).await {
            Some(m) if m == stt::ENGLISH_MODEL => prep(app, "stt", "ok", "Parakeet TDT v2, English"),
            Some(m) => prep(app, "stt", "warn", &format!("{m} for now. The English model is downloading.")),
            None => prep(app, "stt", "warn", "VoiceStudio's default model. Captions may be rough."),
        }
    };
    let accuracy = async {
        prep(app, "whisper", "run", "Loading Whisper large-v3 on the GPU");
        let until = Instant::now() + Duration::from_secs(60);
        loop {
            match whisper::status() {
                WhisperStatus::Ready { model } => break prep(app, "whisper", "ok", &model),
                WhisperStatus::Off { reason } => break prep(app, "whisper", "warn", &format!("Off ({reason}). Parakeet's text is used.")),
                WhisperStatus::Loading if Instant::now() > until => break prep(app, "whisper", "warn", "Still loading. Parakeet's text is used until it's ready."),
                WhisperStatus::Loading => tokio::time::sleep(Duration::from_millis(300)).await,
            }
        }
    };
    let mic = async {
        prep(app, "mic", "run", "Opening the microphone");
        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<i16>>();
        let heard = match audio::Mic::open(tx) {
            Ok(m) => {
                let got = tokio::time::timeout(Duration::from_secs(3), rx.recv()).await;
                drop(m);
                if matches!(got, Ok(Some(_))) { Ok(()) } else { Err("No audio came from the microphone.".to_string()) }
            }
            Err(e) => Err(e.to_string()),
        };
        match &heard {
            Ok(()) => prep(app, "mic", "ok", "Receiving audio"),
            Err(e) => prep(app, "mic", "fail", e),
        }
        heard.is_ok()
    };
    let (voice_ok, (), (), mic_ok) = tokio::join!(voice, recognizer, accuracy, mic);
    (voice_ok && mic_ok).then_some(go)
}

pub fn create_loop(state: &AppState, setup: &LoopSetup) -> Result<i64> {
    let mut setup = setup.clone();
    if !setup.dossier_id.is_empty() {
        let raw = state.store.lock().unwrap().dossier(&setup.dossier_id)?.ok_or_else(|| AppError::NotFound("That dossier is gone. Pick another one.".into()))?;
        let full = crate::dossier::Dossier::from_value(&raw, &state.catalog.round_keys()).map_err(AppError::Invalid)?;
        let picks: Vec<usize> = if setup.dossier_rounds.is_empty() { (0..full.process.len()).collect() } else { setup.dossier_rounds.clone() };
        let d = full.with_rounds(&picks);
        setup.rounds = d.process.iter().map(|r| r.kind.clone()).collect();
        // The dossier sets the style; level and the rest stay as picked.
        setup.company = "auto".into();
        setup.dossier = Some(d);
    }
    let setup = &setup;
    setup.validate(&state.catalog, &state.panels)?;
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
    // The catalog may have changed since the loop was saved; fail cleanly instead of panicking.
    setup.validate(&state.catalog, &state.panels)?;
    let cat = &state.catalog;
    let plan = setup.plan(cat, &state.panels, index);
    let templates = PromptTemplates {
        interviewer: prompts::load(&state.prompts_dir, "interviewer.md")?,
        persona_block: prompts::load(&state.prompts_dir, "persona_block.md").unwrap_or_default(),
        dossier_block: prompts::load(&state.prompts_dir, "dossier_block.md").unwrap_or_default(),
    };
    let mut system_prompt = interviewer_prompt(&templates, cat, &state.panels, &setup, index);
    let question = (plan.key == "coding")
        .then(|| {
            let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
            // A company loop asks a problem its candidates reported, when the bank has one.
            let reported = setup.dossier.as_ref().and_then(|d| d.process.get(index)).map(|r| r.questions.as_slice()).unwrap_or_default();
            state.bank.match_reported(reported, seed).or_else(|| state.bank.pick(difficulty_for(&setup.level), &[], &setup.company, &Default::default(), seed)).cloned()
        })
        .flatten();
    if let Some(q) = &question {
        tracing::info!(id = %q.id, difficulty = %q.difficulty, "coding question");
        system_prompt.push_str(&problem_block(q));
    }
    // Claude Code has no temperature flag (and newer API models drop it), so the same prompt
    // opens the same way every time.
    // So each session gets a starting point not used lately, and sees how recent ones opened.
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    let opener = {
        let openers = &cat.rounds[&plan.key].openers;
        let key = format!("openers:{}", plan.key);
        let store = state.store.lock().unwrap();
        let mut recent: Vec<String> = store.get_setting(&key).ok().flatten().unwrap_or_default();
        let pick = pick_opener(openers, &recent, seed).cloned();
        if let Some(o) = &pick {
            recent.retain(|r| r != o);
            recent.insert(0, o.clone());
            recent.truncate((openers.len() / 2).max(1));
            if let Err(e) = store.set_setting(&key, &recent) {
                tracing::warn!("saving recent openers: {e}");
            }
        }
        pick
    };
    // A coding round's problem comes from the bank, so only its wording needs to vary.
    let earlier: Vec<String> = match question {
        Some(_) => Vec::new(),
        None => state.store.lock().unwrap().recent_transcripts(&plan.key, 4).unwrap_or_default().iter().filter_map(|t| opening_excerpt(t)).collect(),
    };
    tracing::info!(?opener, earlier = earlier.len(), "round variety");
    system_prompt.push_str(&session_block(opener.as_deref(), &earlier));
    let mut settings = state.settings();
    // A panel interviewer always speaks in their own voice, at their own pace.
    if let Some(p) = state.panels.persona(&setup.panel, &plan.key) {
        settings.voice.speed *= p.speed_factor;
        settings.voice.persona = Some(crate::voice::vs::PersonaVoice { name: p.name, instruct: p.voice_instruct, states: p.voice_states });
    }
    // A newer start (or cancel_start) bumps this; the loader below then gives up.
    let me = state.prep_gen.fetch_add(1, Ordering::SeqCst) + 1;
    prep(app, "claude", "run", "Connecting to the AI model");
    let backend = match state.backend().await {
        Ok(l) => l,
        Err(e) => {
            prep(app, "claude", "fail", &e.to_string());
            return Err(e);
        }
    };
    // One round at a time: a round left open (window closed, crash) would keep its Claude process.
    close_other_rounds(&state, None).await;

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
        started: Mutex::new(Instant::now()),
        turns: Mutex::new(Vec::new()),
        workspace: Mutex::new(Workspace::default()),
    });

    let cfg = SessionConfig {
        backend,
        model: settings.interviewer_model.clone(),
        system_prompt: system_prompt.clone(),
        resume: None,
    };
    let voice_start = start_voice(app, round_id, &settings.voice).await;
    let (started_tx, started_rx) = oneshot::channel();
    let (reply_tx, reply_rx) = oneshot::channel();
    let first = FirstReply { started: started_tx, reply: reply_tx };
    let started = async {
        let mut session = spawn_session(app, round_id, cfg, log.clone(), Some(first)).await?;
        session.send(&opening_message(plan.minutes)).await?;
        Ok::<_, AppError>(session)
    }
    .await;
    let session = match started {
        Ok(s) => s,
        Err(e) => {
            prep(app, "claude", "fail", &e.to_string());
            state.voice.end();
            let _ = state.store.lock().unwrap().delete_round(round_id);
            return Err(e);
        }
    };

    // The loader: Claude's first question and every voice part, side by side.
    let ready = async {
        let (claude, go) = tokio::join!(wait_claude(app, started_rx, reply_rx), prepare_voice(app, &settings.voice, voice_start));
        claude.map(|()| go)
    };
    let cancelled = async {
        while state.prep_gen.load(Ordering::SeqCst) == me {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    };
    let outcome = tokio::select! {
        r = ready => r,
        () = cancelled => Err(AppError::Invalid("Cancelled.".into())),
    };
    let voice = match outcome {
        Ok(Some(go)) => {
            let _ = go.send(());
            true
        }
        Ok(None) => {
            state.voice.end();
            false
        }
        Err(e) => {
            state.voice.end();
            session.shutdown().await;
            // Nothing was answered, so the round isn't kept.
            let _ = state.store.lock().unwrap().delete_round(round_id);
            return Err(e);
        }
    };
    // The round starts now, not when the loader opened.
    *log.started.lock().unwrap() = Instant::now();

    state.rounds.lock().await.insert(
        round_id,
        ActiveRound {
            session,
            question: question.clone(),
            stress: None,
            pending_run: None,
            cfg_prompt: system_prompt,
            model: settings.interviewer_model,
            minutes: plan.minutes,
            lang: setup.lang.clone(),
            // The editor opens with the starter code; it is not worth sending until they change it.
            last_ws: Workspace {
                code: question.as_ref().zip(Lang::from_name(&setup.lang)).map(|(q, l)| runner::starter(q, l)).unwrap_or_default(),
                notes: String::new(),
            },
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
        problem: question.as_ref().map(|q| problem_view(q, &setup.lang)),
        lang: setup.lang,
        plan,
        voice,
    })
}

/// Tells the loader when Claude is up (its model) and when the first sentence of its reply is in.
struct FirstReply {
    started: oneshot::Sender<std::result::Result<String, String>>,
    reply: oneshot::Sender<std::result::Result<(), String>>,
}

fn fail_first(
    started: &mut Option<oneshot::Sender<std::result::Result<String, String>>>,
    reply: &mut Option<oneshot::Sender<std::result::Result<(), String>>>,
    message: &str,
) {
    if let Some(tx) = started.take() {
        let _ = tx.send(Err(message.to_string()));
    }
    if let Some(tx) = reply.take() {
        let _ = tx.send(Err(message.to_string()));
    }
}

async fn spawn_session(app: &AppHandle, round_id: i64, cfg: SessionConfig, log: Arc<RoundLog>, first: Option<FirstReply>) -> Result<Session> {
    let (tx, mut rx) = mpsc::unbounded_channel::<SessionEvent>();
    let session = Session::start(round_id, cfg, tx).await?;
    let app = app.clone();
    tokio::spawn(async move {
        let (mut started, mut reply) = match first {
            Some(f) => (Some(f.started), Some(f.reply)),
            None => (None, None),
        };
        while let Some(ev) = rx.recv().await {
            let state = app.state::<AppState>();
            match &ev {
                SessionEvent::Started { session_id, model, .. } => {
                    log.persist(&state, Some(session_id));
                    if let Some(tx) = started.take() {
                        let _ = tx.send(Ok(model.clone()));
                    }
                }
                SessionEvent::Sentence { text, .. } => {
                    if let Some(tx) = reply.take() {
                        let _ = tx.send(Ok(()));
                    }
                    state.voice.say(round_id, text)
                }
                SessionEvent::Error { message, .. } => {
                    state.voice.turn_done(round_id, false);
                    fail_first(&mut started, &mut reply, message);
                }
                SessionEvent::Exited { .. } => fail_first(&mut started, &mut reply, "The interviewer stopped before its first reply."),
                SessionEvent::TurnDone { text, ttft_ms, ended, .. } => {
                    if let Some(tx) = reply.take() {
                        let _ = tx.send(Ok(()));
                    }
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
/// Rejected while the interviewer is still replying, so turns never pile up.
pub async fn send_answer(app: &AppHandle, round_id: i64, text: String, ws: Workspace, force: bool) -> Result<Option<Turn>> {
    let state = app.state::<AppState>();
    let mut rounds = state.rounds.lock().await;
    let r = rounds.get_mut(&round_id).ok_or_else(|| AppError::NotFound("This round is not running.".into()))?;
    if r.session.busy(REPLY_STALE) {
        return Err(AppError::Invalid(BUSY.into()));
    }
    *r.log.workspace.lock().unwrap() = ws.clone();
    let elapsed = r.log.elapsed_min();
    let prev_ws = r.last_ws.clone();
    let Some((mut content, shared)) = candidate_message(&text, &ws, &mut r.last_ws, force, &r.lang, elapsed, r.minutes) else {
        return Ok(None);
    };
    if let Some(run) = r.pending_run.take() {
        content.push_str(&format!("\n[Code run] {run}"));
    }

    let mut sent = if r.session.is_alive() { r.session.send(&content).await } else { Err(AppError::Claude("not running".into())) };
    if sent.is_err() {
        // Dead or broken pipe: reconnect to the same conversation and try once more.
        let resume = r.session.session_id();
        tracing::warn!(round_id, ?resume, "claude session died; restarting");
        let restarted = async {
            let cfg = SessionConfig {
                backend: state.backend().await?,
                model: r.model.clone(),
                system_prompt: r.cfg_prompt.clone(),
                resume,
            };
            spawn_session(app, round_id, cfg, r.log.clone(), None).await
        }
        .await;
        sent = match restarted {
            Ok(new) => {
                std::mem::replace(&mut r.session, new).discard();
                r.session.send(&content).await
            }
            Err(e) => Err(e),
        };
    }
    if let Err(e) = sent {
        // Not delivered: attach the workspace again next time.
        r.last_ws = prev_ws;
        tracing::error!(round_id, "could not reach the interviewer: {e}");
        return Err(AppError::Ai("Could not reach the interviewer. Try again.".into()));
    }

    let turn = Turn {
        who: "me".into(),
        text: text.trim().to_string(),
        at_min: elapsed,
        ttft_ms: None,
        shared_workspace: shared,
        sent: Some(content),
    };
    r.log.turns.lock().unwrap().push(turn.clone());
    r.log.persist(&state, None);
    state.voice.answer_sent(round_id);
    Ok(Some(turn))
}

/// Runs the candidate's code: `submit` = every test plus a full-size one, else the examples.
pub async fn run_code(app: &AppHandle, round_id: i64, code: String, submit: bool) -> Result<runner::Report> {
    let state = app.state::<AppState>();
    let (q, lang, stress) = {
        let rounds = state.rounds.lock().await;
        let r = rounds.get(&round_id).ok_or_else(|| AppError::NotFound("This round is not running.".into()))?;
        let q = r.question.clone().ok_or_else(|| AppError::Invalid("This round has no problem to run.".into()))?;
        (q, r.lang.clone(), r.stress.clone())
    };
    let lang = Lang::from_name(&lang).ok_or_else(|| AppError::Invalid(format!("Running {lang} isn't supported yet. Use Java, Python, C++ or JavaScript.")))?;
    // Built outside the lock: the generator can take a few seconds.
    let stress = match (submit, stress) {
        (true, None) => {
            let s = runner::stress_tests(&q, 1).await;
            if let Some(r) = state.rounds.lock().await.get_mut(&round_id) {
                r.stress = Some(s.clone());
            }
            s
        }
        (_, s) => s.unwrap_or_default(),
    };
    let mut tests: Vec<(&Test, Kind)> = q.tests.iter().filter(|t| submit || t.example).map(|t| (t, if t.example { Kind::Example } else { Kind::Hidden })).collect();
    if submit {
        tests.extend(stress.iter().map(|t| (t, Kind::Stress)));
    }
    let report = runner::run(&q, lang, &code, &tests).await?;
    let summary = report.summary(if submit { "Submit" } else { "Run" });
    tracing::info!(round_id, submit, status = ?report.status, passed = report.passed, total = report.total, "code run");
    if let Some(r) = state.rounds.lock().await.get_mut(&round_id) {
        r.pending_run = Some(summary);
    }
    Ok(report)
}

/// The candidate switched editor language mid-round. Returns that language's starter code
/// ("" when the round has no problem) and whether Run works in it.
pub async fn set_round_lang(app: &AppHandle, round_id: i64, lang: String) -> Result<Value> {
    let state = app.state::<AppState>();
    let mut rounds = state.rounds.lock().await;
    let r = rounds.get_mut(&round_id).ok_or_else(|| AppError::NotFound("This round is not running.".into()))?;
    let runner = Lang::from_name(&lang);
    let starter = r.question.as_ref().zip(runner).map(|(q, l)| runner::starter(q, l)).unwrap_or_default();
    tracing::info!(round_id, from = %r.lang, to = %lang, "editor language changed");
    r.lang = lang;
    // A fresh starter is not worth sending until they edit it.
    if !starter.is_empty() {
        r.last_ws.code = starter.clone();
    }
    Ok(json!({ "starter": starter, "runnable": runner.is_some() }))
}

/// Returned by `send_answer` while the interviewer is still replying.
pub const BUSY: &str = "The interviewer is still talking. Send again when they finish.";

/// Saves and stops every running round except `keep`.
async fn close_other_rounds(state: &AppState, keep: Option<i64>) {
    let others: Vec<ActiveRound> = {
        let mut rounds = state.rounds.lock().await;
        let ids: Vec<i64> = rounds.keys().copied().filter(|id| Some(*id) != keep).collect();
        ids.into_iter().filter_map(|id| rounds.remove(&id)).collect()
    };
    for r in others {
        tracing::info!(round_id = r.log.round_id, "closing a round that was left open");
        r.log.persist(state, r.session.session_id().as_deref());
        r.session.shutdown().await;
        let _ = state.store.lock().unwrap().finish_round(r.log.round_id);
    }
}

/// Stops the round and saves it. A round the candidate never answered in is deleted
/// instead, so history only lists rounds that happened. True when the round was kept.
pub async fn end_round(app: &AppHandle, round_id: i64, ws: Workspace) -> Result<bool> {
    let state = app.state::<AppState>();
    state.voice.end();
    let active = state.rounds.lock().await.remove(&round_id);
    if let Some(r) = active {
        *r.log.workspace.lock().unwrap() = ws;
        r.log.persist(&state, r.session.session_id().as_deref());
        r.session.shutdown().await;
    }
    let store = state.store.lock().unwrap();
    store.finish_round(round_id)?;
    let answered = store.round(round_id).is_ok_and(|r| r.transcript.iter().any(|t| t.who == "me"));
    if !answered {
        tracing::info!(round_id, "no answers; deleting the round");
        store.delete_round(round_id)?;
    }
    Ok(answered)
}

/// Returned by `generate_report` when the candidate said too little to judge.
pub const NOT_ENOUGH: &str = "Not enough to review yet. Answer at least a couple of questions, then end the round.";

pub async fn generate_report(app: &AppHandle, round_id: i64) -> Result<Value> {
    let state = app.state::<AppState>();
    let round = state.store.lock().unwrap().round(round_id)?;
    if !enough_to_review(&round.transcript) {
        return Err(AppError::Invalid(NOT_ENOUGH.into()));
    }
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
            company: &setup.company_for_prompt(cat),
            level_name: cat.levels.get(&setup.level).unwrap_or(&setup.level),
            level_bar: cat.level_bar.get(&setup.level).map_or("", String::as_str),
            rubric: &rubric,
            lang: &setup.lang,
            turns: &round.transcript,
            workspace: &round.workspace,
            metrics: &m,
        },
    );
    let backend = state.backend().await?;
    let out = run_once(&backend, &prompt, Tools::None, &state.settings().report_model, |_| {}).await?;
    tracing::info!(round_id, ms = out.duration_ms, model = %out.model, "report generated");
    let mut report = out.json.clone().filter(Value::is_object).ok_or_else(|| {
        // Keep the raw text for debugging, but never overwrite a good earlier report.
        let store = state.store.lock().unwrap();
        if !matches!(store.report(round_id), Ok(Some((Some(_), _)))) {
            let _ = store.save_report(round_id, None, &out.text);
        }
        tracing::warn!(round_id, raw = %out.text.chars().take(500).collect::<String>(), "report JSON did not parse");
        AppError::Ai("The report came back in the wrong format. Try again.".into())
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
    let prompt = verdict_prompt(&template, &setup.company_for_prompt(cat), cat.levels.get(&setup.level).unwrap_or(&setup.level), &summaries);
    let backend = state.backend().await?;
    let out = run_once(&backend, &prompt, Tools::None, &state.settings().report_model, |_| {}).await?;
    let verdict = out.json.filter(Value::is_object).ok_or_else(|| AppError::Ai("The verdict came back in the wrong format.".into()))?;
    state.store.lock().unwrap().set_loop_verdict(loop_id, &verdict)?;
    Ok(verdict)
}

/// One round on the interview feedback page.
#[derive(Debug, Clone, Serialize)]
pub struct RoundFeedback {
    pub round: crate::store::RoundRow,
    pub report: Option<Value>,
    /// Why there is no report: too little said, or the model call failed.
    pub missing: Option<String>,
}

/// The whole interview: committee verdict plus each round's feedback.
#[derive(Debug, Clone, Serialize)]
pub struct LoopFeedback {
    pub loop_id: i64,
    pub company: String,
    pub level: String,
    pub verdict: Option<Value>,
    pub rounds: Vec<RoundFeedback>,
}

/// Progress for the UI while the whole interview is reviewed.
#[derive(Debug, Clone, Serialize)]
struct LoopProgress {
    loop_id: i64,
    done: usize,
    total: usize,
    stage: &'static str,
}

/// Rounds reviewed at once. Each is one headless Claude call (20-90 s).
const PARALLEL_REPORTS: usize = 3;

/// What is saved for a loop. Writes nothing.
pub fn loop_feedback_saved(state: &AppState, loop_id: i64) -> Result<LoopFeedback> {
    let setup = load_setup(state, loop_id)?;
    let cat = &state.catalog;
    let store = state.store.lock().unwrap();
    let mut rounds = Vec::new();
    for id in store.loop_round_ids(loop_id)? {
        let round = store.round(id)?;
        let report = store.report(id)?.and_then(|(data, _)| data);
        let missing = match (&report, enough_to_review(&round.transcript)) {
            (Some(_), _) => None,
            (None, false) => Some(NOT_ENOUGH.to_string()),
            (None, true) => Some("No feedback yet.".to_string()),
        };
        rounds.push(RoundFeedback { round, report, missing });
    }
    Ok(LoopFeedback {
        loop_id,
        company: setup.company_name(cat),
        level: cat.levels.get(&setup.level).cloned().unwrap_or(setup.level.clone()),
        verdict: store.loop_verdict(loop_id)?,
        rounds,
    })
}

/// Reviews every round that has enough to judge and no feedback yet, then writes the
/// committee verdict once two or more rounds have feedback. One call for the whole interview.
pub async fn loop_feedback(app: &AppHandle, loop_id: i64) -> Result<LoopFeedback> {
    use futures_util::StreamExt;
    let state = app.state::<AppState>();
    let todo: Vec<i64> = loop_feedback_saved(&state, loop_id)?
        .rounds
        .iter()
        .filter(|r| r.report.is_none() && r.missing.as_deref() != Some(NOT_ENOUGH))
        .map(|r| r.round.id)
        .collect();
    let total = todo.len();
    let progress = |done, stage| {
        let _ = app.emit("loop_feedback", LoopProgress { loop_id, done, total, stage });
    };
    progress(0, "rounds");
    let mut failed: Vec<(i64, String)> = Vec::new();
    let mut done = 0;
    let mut jobs = futures_util::stream::iter(todo.into_iter().map(|id| async move { (id, generate_report(app, id).await) }))
        .buffer_unordered(PARALLEL_REPORTS);
    while let Some((id, res)) = jobs.next().await {
        done += 1;
        progress(done, "rounds");
        if let Err(e) = res {
            tracing::warn!(round_id = id, "interview feedback: round report failed: {e}");
            failed.push((id, e.to_string()));
        }
    }
    let saved = loop_feedback_saved(&state, loop_id)?;
    let reviewed = saved.rounds.iter().filter(|r| r.report.is_some()).count();
    // A new round report makes the old verdict stale.
    if reviewed >= 2 && (done > failed.len() || saved.verdict.is_none()) {
        progress(done, "verdict");
        if let Err(e) = loop_verdict(app, loop_id).await {
            tracing::warn!(loop_id, "interview feedback: verdict failed: {e}");
        }
    }
    let mut out = loop_feedback_saved(&state, loop_id)?;
    for r in out.rounds.iter_mut() {
        if let Some((_, e)) = failed.iter().find(|(id, _)| *id == r.round.id) {
            r.missing = Some(e.clone());
        }
    }
    Ok(out)
}
