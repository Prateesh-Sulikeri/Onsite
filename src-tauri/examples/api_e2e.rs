//! Live check of one provider API through the app's own code paths: model list, a short typed
//! round (streamed turns, sentences, TTFT), report, verdict, job keywords, AI screen and research.
//! The key is read from stdin, never from args, env or files:
//! `echo <key> | cargo run --example api_e2e -- groq [interviewer model] [report model] [--skip-research]`

use onsite_lib::brain::{run_once, run_once_with_timeout, ApiClient, Backend, Provider, Session, SessionConfig, SessionEvent, Tools};
use onsite_lib::brain::{prompts, stream::StreamEvent};
use onsite_lib::catalog::Catalog;
use onsite_lib::interview::{self, report, LoopSetup, PromptTemplates, Workspace};
use onsite_lib::panels::Panels;
use onsite_lib::store::Turn;
use serde_json::{json, Value};
use std::path::Path;
use std::time::{Duration, Instant};

const RESUME: &str = "Backend engineer, 4 years. Java 17, Spring Boot, Kafka, PostgreSQL, Redis. Built a payments \
ledger service handling 2,000 TPS with idempotency keys and outbox-based Kafka publishing. Cut p99 latency of the \
refund API from 900 ms to 120 ms by moving hot reads to Redis. Led migration of 40M rows to a partitioned Postgres table.";

const ANSWERS: [&str; 3] = [
    "Sure. In the payments service every request carries an idempotency key. We store the key with a unique index in Postgres \
     in the same transaction as the ledger write, so a retry hits the constraint and we return the stored response.",
    "If two retries race, both try the insert; one wins on the unique index, the other gets a conflict and reads the saved \
     result. For Kafka we use the outbox table, a relay publishes it, and consumers dedupe on the event id.",
    "The trade-off is the extra write per request and keeping keys around. We expire them after 24 hours with a partition drop.",
];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let provider = match args.first().map(String::as_str) {
        Some("groq") => Provider::Groq,
        Some("openai") => Provider::Openai,
        Some("gemini") => Provider::Gemini,
        Some("anthropic") => Provider::Anthropic,
        _ => {
            eprintln!("usage: echo <key> | api_e2e <groq|openai|gemini|anthropic> [interviewer model] [report model] [--skip-research]");
            std::process::exit(2);
        }
    };
    let mut key = String::new();
    std::io::stdin().read_line(&mut key).expect("read key");
    let (iv_default, rep_default) = provider.default_models();
    let pos: Vec<&String> = args.iter().skip(1).filter(|a| !a.starts_with("--")).collect();
    let iv_model = pos.first().map_or(iv_default.to_string(), |s| s.to_string());
    let rep_model = pos.get(1).map_or(rep_default.to_string(), |s| s.to_string());
    let skip_research = args.iter().any(|a| a == "--skip-research");
    let only_research = args.iter().any(|a| a == "--research-only");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let ok = rt.block_on(run(ApiClient::new(provider, &key), &iv_model, &rep_model, skip_research, only_research));
    std::process::exit(if ok { 0 } else { 1 });
}

fn step(name: &str, r: Result<String, String>, ok: &mut bool) {
    match r {
        Ok(s) => println!("PASS {name}: {s}"),
        Err(e) => {
            *ok = false;
            println!("FAIL {name}: {e}");
        }
    }
}

async fn run(client: ApiClient, iv_model: &str, rep_model: &str, skip_research: bool, only_research: bool) -> bool {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts");
    let cat = Catalog::load(&dir).unwrap();
    let panels = Panels::load(&dir);
    let backend = Backend::Api(client.clone());
    let mut ok = true;
    println!("provider {} | interviewer {iv_model} | report {rep_model}", client.provider.label());

    if only_research && !std::env::args().any(|a| a == "--print-prompt") {
        return research(&backend, &cat, &dir, rep_model).await;
    }

    // 1. Model list (Settings pickers).
    let t = Instant::now();
    let models = client.models().await;
    step("models", models.as_ref().map(|m| format!("{} in {:?}: {}", m.len(), t.elapsed(), m.join(", "))).map_err(|e| e.to_string()), &mut ok);

    // 2. A short typed round, through the same Session the app uses.
    let setup = LoopSetup {
        rounds: vec!["fundamentals".into()],
        company: "auto".into(),
        level: "sde2".into(),
        temperament: "realistic".into(),
        lang: "Java".into(),
        resume: RESUME.into(),
        jd: String::new(),
        notes: String::new(),
        minutes: Some(10),
        panel: String::new(),
        dossier_id: String::new(),
        dossier_rounds: vec![],
        dossier: None,
    };
    let templates = PromptTemplates {
        interviewer: prompts::load(&dir, "interviewer.md").unwrap(),
        persona_block: prompts::load(&dir, "persona_block.md").unwrap_or_default(),
        dossier_block: prompts::load(&dir, "dossier_block.md").unwrap_or_default(),
    };
    let plan = setup.plan(&cat, &panels, 0);
    let mut system = interview::interviewer_prompt(&templates, &cat, &panels, &setup, 0);
    system.push_str(&interview::session_block(cat.rounds[&plan.key].openers.first().map(String::as_str), &[]));
    if std::env::args().any(|a| a == "--print-prompt") {
        println!("{system}");
        return true;
    }
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let cfg = SessionConfig { backend: backend.clone(), model: iv_model.to_string(), system_prompt: system, resume: None };
    let mut session = Session::start(1, cfg, tx).await.expect("session start");
    let mut turns: Vec<Turn> = Vec::new();
    let mut last = Workspace::default();
    let started = Instant::now();
    let mut ttfts = Vec::new();
    let mut to_send = vec![interview::opening_message(plan.minutes)];
    for (i, a) in ANSWERS.iter().enumerate() {
        let ws = if i == 1 { Workspace { code: String::new(), notes: "client -> API -> [idempotency table] -> ledger\nledger -> outbox -> relay -> Kafka".into() } } else { Workspace::default() };
        let (msg, _) = interview::candidate_message(a, &ws, &mut last, false, &setup.lang, started.elapsed().as_secs_f64() / 60.0, plan.minutes).unwrap();
        to_send.push(msg);
    }
    for (i, msg) in to_send.iter().enumerate() {
        if i > 0 {
            turns.push(Turn { who: "me".into(), text: ANSWERS[i - 1].into(), at_min: 1.0 + i as f64, ttft_ms: None, shared_workspace: i == 2, sent: Some(msg.clone()) });
        }
        session.send(msg).await.expect("send");
        let mut sentences = 0;
        let mut deltas = 0;
        let outcome = loop {
            match tokio::time::timeout(Duration::from_secs(120), rx.recv()).await {
                Ok(Some(SessionEvent::Delta { .. })) => deltas += 1,
                Ok(Some(SessionEvent::Sentence { .. })) => sentences += 1,
                Ok(Some(SessionEvent::TurnDone { text, ttft_ms, total_ms, ended, .. })) => break Ok((text, ttft_ms, total_ms, ended)),
                Ok(Some(SessionEvent::Error { message, .. })) => break Err(message),
                Ok(Some(_)) => {}
                Ok(None) => break Err("event channel closed".into()),
                Err(_) => break Err("no reply in 120 s".into()),
            }
        };
        let name = format!("turn {}", i + 1);
        match outcome {
            Ok((text, ttft, total, ended)) => {
                let leak = ["<think>", "analysis", "We need to", "The user"].iter().find(|s| text.trim_start().starts_with(**s));
                let r = if text.trim().is_empty() {
                    Err("empty reply".into())
                } else if let Some(s) = leak {
                    Err(format!("reply looks like reasoning (starts with {s:?}): {}", text.chars().take(160).collect::<String>()))
                } else {
                    ttfts.push(ttft);
                    Ok(format!("ttft {ttft} ms, total {total} ms, {deltas} deltas, {sentences} sentences, ended={ended}\n    > {}", text.replace('\n', " ").chars().take(300).collect::<String>()))
                };
                step(&name, r, &mut ok);
                turns.push(Turn { who: "iv".into(), text, at_min: 1.0 + i as f64, ttft_ms: Some(ttft), shared_workspace: false, sent: None });
            }
            Err(e) => step(&name, Err(e), &mut ok),
        }
    }
    session.shutdown().await;
    if !ttfts.is_empty() {
        println!("     TTFT ms per turn: {ttfts:?}");
    }

    // 3. Report (report model, no tools).
    let metrics = report::metrics(&turns, 4.0);
    let ws = json!({"code": "", "notes": "client -> API -> [idempotency table] -> ledger"});
    let rubric = &cat.rounds[&plan.key].rubric;
    let prompt = report::report_prompt(
        &prompts::load(&dir, "report.md").unwrap(),
        &report::ReportInput {
            round_name: &plan.name,
            round_minutes: plan.minutes as i64,
            company: "a top tech company",
            level_name: &cat.levels["sde2"],
            level_bar: &cat.level_bar["sde2"],
            rubric,
            lang: "Java",
            turns: &turns,
            workspace: &ws,
            metrics: &metrics,
        },
    );
    let rep = run_once(&backend, &prompt, Tools::None, rep_model, |_| {}).await;
    let rep_json = match rep {
        Ok(o) => match o.json.filter(|j| j["overall"].is_number() && j["dimensions"].is_array() && j["decision"].is_string()) {
            Some(j) => {
                step("report", Ok(format!("{} ms, decision {}, overall {}, {} dimensions", o.duration_ms, j["decision"], j["overall"], j["dimensions"].as_array().unwrap().len())), &mut ok);
                Some(j)
            }
            None => {
                step("report", Err(format!("no usable JSON in {} chars: {}", o.text.len(), o.text.chars().take(300).collect::<String>())), &mut ok);
                None
            }
        },
        Err(e) => {
            step("report", Err(e.to_string()), &mut ok);
            None
        }
    };

    // 4. Loop verdict.
    if let Some(j) = &rep_json {
        let sums = vec![report::round_summary(&plan.name, j), report::round_summary("System design", j)];
        let prompt = report::verdict_prompt(&prompts::load(&dir, "verdict.md").unwrap(), "a top tech company", &cat.levels["sde2"], &sums);
        let r = run_once(&backend, &prompt, Tools::None, rep_model, |_| {}).await;
        step(
            "verdict",
            r.map_err(|e| e.to_string()).and_then(|o| match o.json.filter(Value::is_object) {
                Some(j) => Ok(format!("{} ms, keys {:?}", o.duration_ms, j.as_object().unwrap().keys().collect::<Vec<_>>())),
                None => Err(format!("no JSON: {}", o.text.chars().take(300).collect::<String>())),
            }),
            &mut ok,
        );
    }

    // 5. Resume check: job keywords (interviewer model) and AI screen (report model).
    let jd = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../dossiers/razorpay-sde-2-backend.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["jd"].as_str().map(String::from))
        .unwrap_or_default();
    let prompt = prompts::render(&prompts::load(&dir, "ats_job.md").unwrap(), &[("jd", jd.clone())].into());
    let r = run_once(&backend, &prompt, Tools::None, iv_model, |_| {}).await;
    step(
        "ats job keywords",
        r.map_err(|e| e.to_string()).and_then(|o| {
            let j = o.json.ok_or_else(|| format!("no JSON: {}", o.text.chars().take(300).collect::<String>()))?;
            let k = serde_json::from_value::<onsite_lib::ats::job::JobKeywords>(j).map_err(|e| e.to_string())?.sanitize();
            if k.is_empty() {
                return Err("no keywords".into());
            }
            Ok(format!("{} ms, required {:?}", o.duration_ms, k.required_skills))
        }),
        &mut ok,
    );
    let prompt = onsite_lib::ats::screen::render_prompt(&prompts::load(&dir, "ats_screen.md").unwrap(), RESUME, &jd);
    let r = run_once(&backend, &prompt, Tools::None, rep_model, |_| {}).await;
    step(
        "ats AI screen",
        r.map_err(|e| e.to_string()).and_then(|o| {
            let j = o.json.ok_or_else(|| format!("no JSON: {}", o.text.chars().take(300).collect::<String>()))?;
            let s = serde_json::from_value::<onsite_lib::ats::screen::AiScreen>(j).map_err(|e| e.to_string())?.sanitize();
            Ok(format!("{} ms, grade {} fit {}", o.duration_ms, s.grade, s.fit))
        }),
        &mut ok,
    );

    if !skip_research && !research(&backend, &cat, &dir, rep_model).await {
        ok = false;
    }
    println!("{}", if ok { "ALL PASSED" } else { "SOME STEPS FAILED" });
    ok
}

/// Research: the only call with tools.
async fn research(backend: &Backend, cat: &Catalog, dir: &Path, rep_model: &str) -> bool {
    let mut ok = true;
    {
        let template = prompts::load(dir, "research.md").unwrap();
        let round_types = cat.round_keys().iter().map(|k| format!("\"{k}\"")).collect::<Vec<_>>().join(" | ");
        let vars: prompts::Vars = [
            ("company", "Zerodha".to_string()),
            ("role", "Backend Engineer".to_string()),
            ("level_name", cat.levels["sde2"].clone()),
            ("jd", "(none)".to_string()),
            ("round_types", round_types),
            ("today", "2026-10-29".to_string()),
        ]
        .into();
        let prompt = prompts::render(&template, &vars);
        let (mut searches, mut progress) = (0, 0);
        let t = Instant::now();
        let r = run_once_with_timeout(backend, &prompt, Tools::Web, rep_model, Duration::from_secs(20 * 60), |ev| match ev {
            StreamEvent::ToolUse { .. } => searches += 1,
            StreamEvent::AssistantText(t) | StreamEvent::TextDelta(t) => progress += t.lines().filter_map(onsite_lib::research::parse_progress).count(),
            _ => {}
        })
        .await;
        step(
            "research",
            r.map_err(|e| e.to_string()).and_then(|o| {
                let j = o.json.filter(Value::is_object).ok_or_else(|| format!("no dossier JSON in {} chars: {}", o.text.len(), o.text.chars().take(400).collect::<String>()))?;
                let d = onsite_lib::dossier::Dossier::from_value(&j, &cat.round_keys())?;
                Ok(format!(
                    "{:?}, {searches} searches, {progress} PROGRESS lines, {} rounds ({}), {} sources",
                    t.elapsed(),
                    d.process.len(),
                    d.process.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>().join(", "),
                    d.sources.len()
                ))
            }),
            &mut ok,
        );
    }
    ok
}
