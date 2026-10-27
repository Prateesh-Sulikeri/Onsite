//! An interview round over a provider API. Mirrors `ClaudeSession`: same events, same
//! sentence splitting, but the conversation lives here and each turn is one streamed request.

use super::api::{ApiClient, Effort, Msg, Role};
use super::locate::ClaudeLocation;
use super::session::{ClaudeSession, SessionConfig, SessionEvent, TurnState};
use super::stream::StreamEvent;
use crate::error::{AppError, Result};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

/// Where model calls go: the Claude Code CLI, or a provider API with the user's key.
#[derive(Debug, Clone)]
pub enum Backend {
    Cli(ClaudeLocation),
    Api(ApiClient),
}

impl Backend {
    pub fn describe(&self) -> String {
        match self {
            Self::Cli(loc) => loc.describe(),
            Self::Api(c) => format!("{} API", c.provider.label()),
        }
    }
}

/// One live round, whichever backend runs it.
pub enum Session {
    Cli(Box<ClaudeSession>),
    Api(ApiSession),
}

impl Session {
    pub async fn start(round_id: i64, cfg: SessionConfig, events: UnboundedSender<SessionEvent>) -> Result<Self> {
        match cfg.backend.clone() {
            Backend::Cli(loc) => Ok(Self::Cli(Box::new(ClaudeSession::start(round_id, loc, cfg, events).await?))),
            Backend::Api(client) => Ok(Self::Api(ApiSession::start(round_id, client, cfg, events))),
        }
    }

    pub fn session_id(&self) -> Option<String> {
        match self {
            Self::Cli(s) => s.session_id(),
            Self::Api(_) => None,
        }
    }

    pub fn busy(&self, stale: Duration) -> bool {
        match self {
            Self::Cli(s) => s.busy(stale),
            Self::Api(s) => s.busy(stale),
        }
    }

    pub fn is_alive(&self) -> bool {
        match self {
            Self::Cli(s) => s.is_alive(),
            Self::Api(_) => true,
        }
    }

    pub async fn send(&mut self, text: &str) -> Result<()> {
        match self {
            Self::Cli(s) => s.send(text).await,
            Self::Api(s) => s.send(text),
        }
    }

    pub fn discard(self) {
        match self {
            Self::Cli(s) => (*s).discard(),
            Self::Api(s) => s.stop(),
        }
    }

    pub async fn shutdown(self) {
        match self {
            Self::Cli(s) => (*s).shutdown().await,
            Self::Api(s) => s.stop(),
        }
    }
}

pub struct ApiSession {
    round_id: i64,
    client: ApiClient,
    model: String,
    system: Arc<str>,
    history: Arc<Mutex<Vec<Msg>>>,
    /// Start of the turn still waiting for its reply.
    pending: Arc<Mutex<Option<Instant>>>,
    events: UnboundedSender<SessionEvent>,
    task: Option<JoinHandle<()>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl ApiSession {
    fn start(round_id: i64, client: ApiClient, cfg: SessionConfig, events: UnboundedSender<SessionEvent>) -> Self {
        tracing::info!(round_id, provider = client.provider.id(), model = %cfg.model, "api session started");
        let _ = events.send(SessionEvent::Started { round_id, session_id: String::new(), model: cfg.model.clone() });
        Self {
            round_id,
            client,
            model: cfg.model,
            system: cfg.system_prompt.into(),
            history: Default::default(),
            pending: Default::default(),
            events,
            task: None,
        }
    }

    fn busy(&self, stale: Duration) -> bool {
        lock(&self.pending).is_some_and(|t| t.elapsed() < stale)
    }

    /// Adds the candidate's turn and streams the reply as events.
    fn send(&mut self, text: &str) -> Result<()> {
        if let Some(old) = self.task.take() {
            // Only reached when the last reply went stale; it won't be waited for.
            old.abort();
        }
        let msgs = {
            let mut h = lock(&self.history);
            h.push(Msg::user(text));
            h.clone()
        };
        let started = Instant::now();
        *lock(&self.pending) = Some(started);
        let (round_id, client, model, system) = (self.round_id, self.client.clone(), self.model.clone(), self.system.clone());
        let (history, pending, tx) = (self.history.clone(), self.pending.clone(), self.events.clone());
        self.task = Some(tokio::spawn(async move {
            let mut turn = TurnState::default();
            let res = client
                .chat(&model, &system, &msgs, Effort::Low, |d| {
                    for out in turn.apply(round_id, StreamEvent::TextDelta(d.to_string()), Some(started)) {
                        let _ = tx.send(out);
                    }
                })
                .await;
            let result = match res {
                Ok(text) => {
                    lock(&history).push(Msg { role: Role::Assistant, text: text.clone() });
                    StreamEvent::Result { text, session_id: String::new(), is_error: false, duration_ms: 0 }
                }
                Err(e) => {
                    tracing::warn!(round_id, "interviewer reply failed: {e}");
                    // The candidate's turn stays in the history; their next answer is joined to it.
                    StreamEvent::Result { text: e.to_string(), session_id: String::new(), is_error: true, duration_ms: 0 }
                }
            };
            *lock(&pending) = None;
            for out in turn.apply(round_id, result, Some(started)) {
                let _ = tx.send(out);
            }
        }));
        Ok(())
    }

    fn stop(mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
        let _ = self.events.send(SessionEvent::Exited { round_id: self.round_id });
    }
}

/// One-shot jobs over the API: `Tools::Web` turns on web search; anything else gets no tools.
pub async fn api_once(
    client: &ApiClient,
    system: &str,
    prompt: &str,
    web: bool,
    model: &str,
    limit: Duration,
    on_event: impl FnMut(&StreamEvent) + Send,
) -> Result<String> {
    let run = async {
        if web {
            client.research(model, system, prompt, on_event).await
        } else {
            client.chat(model, system, &[Msg::user(prompt)], Effort::Medium, |_| {}).await
        }
    };
    match tokio::time::timeout(limit, run).await {
        Ok(r) => r,
        Err(_) => Err(AppError::Ai(format!("No reply after {} minutes. Try again.", limit.as_secs() / 60))),
    }
}
