//! One long-lived `claude -p` process per interview round, spoken to over stream-json.

use super::locate::{isolation_args, ClaudeLocation, Tools};
use super::splitter::{Piece, SentenceSplitter, END_MARKER};
use super::stream::{parse_line, StreamEvent};
use crate::error::{AppError, Result};
use serde::Serialize;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

pub struct SessionConfig {
    pub location: ClaudeLocation,
    pub model: String,
    pub system_prompt: String,
    pub resume: Option<String>,
}

/// Sent to the UI as the `round` Tauri event.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    Started { round_id: i64, session_id: String, model: String },
    Delta { round_id: i64, text: String },
    Sentence { round_id: i64, text: String },
    Code { round_id: i64, lang: String, code: String },
    TurnDone { round_id: i64, text: String, ttft_ms: u64, total_ms: u64, ended: bool },
    Error { round_id: i64, message: String, recoverable: bool },
    Exited { round_id: i64 },
}

#[derive(Default)]
struct Shared {
    session_id: Mutex<Option<String>>,
    /// One entry per sent turn still waiting for its `result`. Claude answers queued turns in order.
    pending: Mutex<VecDeque<Instant>>,
    stderr_tail: Mutex<Vec<String>>,
    alive: AtomicBool,
    closing: AtomicBool,
}

pub struct ClaudeSession {
    child: Child,
    stdin: Option<ChildStdin>,
    _workdir: tempfile::TempDir,
    shared: Arc<Shared>,
    reader: JoinHandle<()>,
}

impl ClaudeSession {
    pub async fn start(round_id: i64, cfg: SessionConfig, events: UnboundedSender<SessionEvent>) -> Result<Self> {
        // Empty dir: no CLAUDE.md, repo files or project settings can leak in.
        let workdir = tempfile::Builder::new().prefix("onsite-round-").tempdir()?;
        let prompt_path = workdir.path().join(".system-prompt.md");
        std::fs::write(&prompt_path, &cfg.system_prompt)?;

        let mut args = isolation_args(Tools::None, &cfg.model, Some(&cfg.location.path_arg(&prompt_path)));
        args.extend(
            ["--input-format", "stream-json", "--output-format", "stream-json", "--verbose", "--include-partial-messages"]
                .map(String::from),
        );
        if let Some(id) = &cfg.resume {
            args.extend(["--resume".to_string(), id.clone()]);
        }

        let mut cmd = cfg.location.command(&args, workdir.path());
        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| AppError::Claude(format!("could not start: {e}")))?;
        tracing::info!(round_id, location = %cfg.location.describe(), model = %cfg.model, resume = ?cfg.resume, "claude session started");

        let shared = Arc::new(Shared { session_id: Mutex::new(cfg.resume.clone()), ..Default::default() });
        shared.alive.store(true, Ordering::SeqCst);

        let stderr = child.stderr.take().expect("piped stderr");
        let sh = shared.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::warn!(round_id, "claude stderr: {line}");
                let mut tail = lock(&sh.stderr_tail);
                tail.push(line);
                if tail.len() > 20 {
                    tail.remove(0);
                }
            }
        });

        let stdout = child.stdout.take().expect("piped stdout");
        let reader = tokio::spawn(read_loop(round_id, stdout, shared.clone(), events));
        Ok(Self { stdin: child.stdin.take(), child, _workdir: workdir, shared, reader })
    }

    pub fn session_id(&self) -> Option<String> {
        lock(&self.shared.session_id).clone()
    }

    /// True while a sent turn has no reply yet. A turn older than `stale` no longer counts,
    /// so a hung reply can't lock the candidate out.
    pub fn busy(&self, stale: Duration) -> bool {
        self.is_alive() && lock(&self.shared.pending).front().is_some_and(|t| t.elapsed() < stale)
    }

    pub fn is_alive(&self) -> bool {
        self.shared.alive.load(Ordering::SeqCst)
    }

    /// Writes one user turn. The reply arrives as events.
    pub async fn send(&mut self, text: &str) -> Result<()> {
        let stdin = self.stdin.as_mut().ok_or_else(|| AppError::Claude("session is closed".into()))?;
        let mut line = serde_json::to_string(&user_message(text))?;
        line.push('\n');
        // Queued before the write so a fast reply always finds its start time.
        lock(&self.shared.pending).push_back(Instant::now());
        let res = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.flush().await
        }
        .await;
        if let Err(e) = res {
            lock(&self.shared.pending).pop_back();
            tracing::warn!("writing to claude failed: {e}");
            return Err(AppError::Claude("the interviewer process is gone".into()));
        }
        Ok(())
    }

    /// Kills the process without reporting it as a crash (used when replacing a dead session).
    pub fn discard(mut self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        let _ = self.child.start_kill();
    }

    /// Closes stdin so claude exits cleanly; kills it after a timeout.
    pub async fn shutdown(mut self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        drop(self.stdin.take());
        if tokio::time::timeout(Duration::from_secs(5), self.child.wait()).await.is_err() {
            let _ = self.child.kill().await;
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), &mut self.reader).await;
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Short, plain text for the UI. The full stderr is in the log.
fn crash_message(last_stderr: Option<&str>) -> String {
    let base = "The interviewer disconnected. Answer again to reconnect.";
    match last_stderr.map(str::trim).filter(|l| !l.is_empty()) {
        Some(l) => {
            let l: String = l.chars().take(160).collect();
            format!("{base} ({l})")
        }
        None => base.to_string(),
    }
}

pub fn user_message(text: &str) -> serde_json::Value {
    serde_json::json!({"type": "user", "message": {"role": "user", "content": [{"type": "text", "text": text}]}})
}

/// Strips `[[END]]` from the final text. Returns (clean text, ended).
pub fn clean_final(text: &str) -> (String, bool) {
    let ended = text.contains(END_MARKER);
    (text.replace(END_MARKER, "").trim().to_string(), ended)
}

async fn read_loop(
    round_id: i64,
    stdout: impl tokio::io::AsyncRead + Unpin,
    shared: Arc<Shared>,
    tx: UnboundedSender<SessionEvent>,
) {
    let mut lines = BufReader::new(stdout).lines();
    let mut turn = TurnState::default();
    while let Ok(Some(line)) = lines.next_line().await {
        let events = match parse_line(&line) {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!(round_id, "non-JSON stdout line ({e}): {line}");
                continue;
            }
        };
        for ev in events {
            if let StreamEvent::Init { session_id, .. } | StreamEvent::Result { session_id, .. } = &ev {
                if !session_id.is_empty() {
                    *lock(&shared.session_id) = Some(session_id.clone());
                }
            }
            let is_result = matches!(ev, StreamEvent::Result { .. });
            let started = lock(&shared.pending).front().copied();
            if is_result {
                lock(&shared.pending).pop_front();
            }
            for out in turn.apply(round_id, ev, started) {
                let _ = tx.send(out);
            }
        }
    }
    shared.alive.store(false, Ordering::SeqCst);
    lock(&shared.pending).clear();
    if !shared.closing.load(Ordering::SeqCst) {
        // Give the stderr reader a moment to catch the last line (e.g. a login error).
        tokio::time::sleep(Duration::from_millis(200)).await;
        let last = lock(&shared.stderr_tail).last().cloned();
        tracing::warn!(round_id, ?last, "claude exited mid-round");
        let _ = tx.send(SessionEvent::Error { round_id, message: crash_message(last.as_deref()), recoverable: true });
    }
    let _ = tx.send(SessionEvent::Exited { round_id });
}

/// Pure per-turn state machine, so it can be tested on recorded streams.
#[derive(Default)]
struct TurnState {
    splitter: SentenceSplitter,
    ttft_ms: Option<u64>,
    got_delta: bool,
}

impl TurnState {
    fn apply(&mut self, round_id: i64, ev: StreamEvent, started: Option<Instant>) -> Vec<SessionEvent> {
        let since = |s: Option<Instant>| s.map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
        let mut out = Vec::new();
        match ev {
            StreamEvent::Init { session_id, model, .. } => out.push(SessionEvent::Started { round_id, session_id, model }),
            StreamEvent::TextDelta(text) => {
                self.ttft_ms.get_or_insert_with(|| since(started));
                self.got_delta = true;
                let pieces = self.splitter.push(&text);
                out.push(SessionEvent::Delta { round_id, text });
                out.extend(pieces.into_iter().map(|p| piece_event(round_id, p)));
            }
            StreamEvent::Result { text, is_error, .. } => {
                let mut pieces = if self.got_delta { Vec::new() } else { self.splitter.push(&text) };
                pieces.extend(self.splitter.finish().0);
                out.extend(pieces.into_iter().map(|p| piece_event(round_id, p)));
                if is_error {
                    let message = if text.is_empty() { "The interviewer's reply failed.".into() } else { text };
                    out.push(SessionEvent::Error { round_id, message, recoverable: true });
                } else {
                    let (text, ended) = clean_final(&text);
                    let total_ms = since(started);
                    let ttft_ms = self.ttft_ms.unwrap_or(total_ms);
                    tracing::info!(round_id, ttft_ms, total_ms, "interviewer turn done");
                    out.push(SessionEvent::TurnDone { round_id, text, ttft_ms, total_ms, ended });
                }
                *self = Self::default();
            }
            _ => {}
        }
        out
    }
}

fn piece_event(round_id: i64, p: Piece) -> SessionEvent {
    match p {
        Piece::Sentence(text) => SessionEvent::Sentence { round_id, text },
        Piece::Code { lang, code } => SessionEvent::Code { round_id, lang, code },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/turn_code_end.jsonl");

    fn replay(lines: &str) -> Vec<SessionEvent> {
        let mut t = TurnState::default();
        lines
            .lines()
            .flat_map(|l| parse_line(l).unwrap())
            .flat_map(|ev| t.apply(1, ev, Some(Instant::now())))
            .collect()
    }

    #[test]
    fn fixture_turn_yields_speech_code_and_end() {
        let evs = replay(FIXTURE);
        assert!(matches!(&evs[0], SessionEvent::Started { model, .. } if model == "claude-sonnet-5-5"));
        let sentences: Vec<&str> = evs
            .iter()
            .filter_map(|e| if let SessionEvent::Sentence { text, .. } = e { Some(text.as_str()) } else { None })
            .collect();
        assert_eq!(sentences, ["Here is a quick example.", "It prints a greeting.", "That code outputs the word hi when run."]);
        assert!(evs.iter().any(|e| matches!(e, SessionEvent::Code { lang, .. } if lang == "python")));
        let SessionEvent::TurnDone { text, ended, .. } = evs.last().unwrap() else { panic!("no TurnDone") };
        assert!(*ended);
        assert!(!text.contains("[[END]]") && text.ends_with("when run."));
    }

    #[test]
    fn result_without_deltas_still_speaks() {
        let line = r#"{"type":"result","subtype":"success","result":"Hello there. Ready?","session_id":"s"}"#;
        let evs = replay(line);
        assert_eq!(
            evs[..2],
            [
                SessionEvent::Sentence { round_id: 1, text: "Hello there.".into() },
                SessionEvent::Sentence { round_id: 1, text: "Ready?".into() }
            ]
        );
    }

    #[tokio::test]
    async fn queued_turns_each_get_their_own_start_and_crash_is_reported() {
        let r = |t: &str| format!(r#"{{"type":"result","subtype":"success","result":"{t}","session_id":"s"}}"#);
        let input = format!("{}\n{}\n", r("Apple."), r("Banana."));
        let shared = Arc::new(Shared::default());
        shared.alive.store(true, Ordering::SeqCst);
        let t0 = Instant::now() - Duration::from_secs(5);
        lock(&shared.pending).extend([t0, Instant::now()]);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        read_loop(1, input.as_bytes(), shared.clone(), tx).await;
        let mut done = Vec::new();
        let mut tail = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            match ev {
                SessionEvent::TurnDone { text, total_ms, .. } => done.push((text, total_ms)),
                SessionEvent::Error { .. } | SessionEvent::Exited { .. } => tail.push(ev),
                _ => {}
            }
        }
        assert_eq!(done.len(), 2);
        assert!(done[0].1 >= 5000, "first turn timed from its own send");
        assert!(done[1].1 < 5000, "second turn timed from its own send");
        assert!(lock(&shared.pending).is_empty());
        assert!(!shared.alive.load(Ordering::SeqCst));
        assert!(matches!(tail[..], [SessionEvent::Error { recoverable: true, .. }, SessionEvent::Exited { .. }]));
    }

    #[test]
    fn crash_message_is_short_and_plain() {
        assert_eq!(crash_message(None), "The interviewer disconnected. Answer again to reconnect.");
        assert_eq!(crash_message(Some("  ")), "The interviewer disconnected. Answer again to reconnect.");
        let long = "x".repeat(500);
        let m = crash_message(Some(&long));
        assert!(m.starts_with("The interviewer disconnected.") && m.len() < 240);
    }

    #[test]
    fn user_message_shape() {
        let m = user_message("hi");
        assert_eq!(m["type"], "user");
        assert_eq!(m["message"]["role"], "user");
        assert_eq!(m["message"]["content"][0], serde_json::json!({"type": "text", "text": "hi"}));
    }
}
