//! Model calls over a provider's HTTP API, for people without Claude Code: the Claude API,
//! OpenAI and Gemini, each with the user's own key. Same jobs as the CLI path: streamed chat
//! for the interviewer, one-shot jobs, and web research (the only call that gets tools).

use super::stream::StreamEvent;
use crate::error::{AppError, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

/// Where model calls go. `ClaudeCode` is the local CLI; the rest are HTTP APIs with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    #[default]
    ClaudeCode,
    Anthropic,
    Openai,
    Gemini,
}

impl Provider {
    pub const APIS: [Provider; 3] = [Provider::Anthropic, Provider::Openai, Provider::Gemini];

    pub fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude_code",
            Self::Anthropic => "anthropic",
            Self::Openai => "openai",
            Self::Gemini => "gemini",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Anthropic => "Claude API",
            Self::Openai => "OpenAI",
            Self::Gemini => "Gemini",
        }
    }

    /// (interviewer, feedback) models. API ids change often, so Settings also lists the live ones.
    pub fn default_models(self) -> (&'static str, &'static str) {
        match self {
            Self::ClaudeCode => ("sonnet", "opus"),
            Self::Anthropic => ("claude-sonnet-5-5", "claude-opus-5-5"),
            Self::Openai => ("gpt-5.5", "gpt-5.5"),
            // The free tier covers Flash models only.
            Self::Gemini => ("gemini-flash-latest", "gemini-flash-latest"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Msg {
    pub role: Role,
    pub text: String,
}

impl Msg {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: Role::User, text: text.into() }
    }
}

/// How hard the model thinks. Low for live replies (latency), medium for written feedback.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effort {
    Low,
    Medium,
}

impl Effort {
    fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
        }
    }
}

const ANTHROPIC: &str = "https://api.anthropic.com/v1";
const OPENAI: &str = "https://api.openai.com/v1";
const GEMINI: &str = "https://generativelanguage.googleapis.com/v1beta";
const CHAT_TIMEOUT: Duration = Duration::from_secs(5 * 60);
const RESEARCH_TIMEOUT: Duration = Duration::from_secs(25 * 60);
/// Claude's server-side tool loop pauses after 10 steps; a research job resumes it this many times.
const MAX_CONTINUATIONS: usize = 8;

#[derive(Clone)]
pub struct ApiClient {
    pub provider: Provider,
    key: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiClient").field("provider", &self.provider).finish_non_exhaustive()
    }
}

impl ApiClient {
    pub fn new(provider: Provider, key: &str) -> Self {
        let http = reqwest::Client::builder().connect_timeout(Duration::from_secs(15)).build().unwrap_or_default();
        Self { provider, key: key.trim().to_string(), http }
    }

    fn post(&self, url: &str, body: &Value, timeout: Duration) -> reqwest::RequestBuilder {
        let req = self.http.post(url).json(body).timeout(timeout);
        self.auth(req)
    }

    fn auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match self.provider {
            Provider::Anthropic => req.header("x-api-key", &self.key).header("anthropic-version", "2023-06-01"),
            Provider::Openai => req.bearer_auth(&self.key),
            Provider::Gemini => req.header("x-goog-api-key", &self.key),
            Provider::ClaudeCode => req,
        }
    }

    /// Streams one reply. `on_delta` sees each text piece; returns the whole text.
    pub async fn chat(&self, model: &str, system: &str, msgs: &[Msg], effort: Effort, on_delta: impl FnMut(&str) + Send) -> Result<String> {
        match self.provider {
            Provider::Anthropic => self.anthropic_chat(model, system, msgs, effort, on_delta).await,
            Provider::Openai => self.openai_stream(model, system, msgs, Some(effort), false, on_delta, |_| {}).await,
            Provider::Gemini => self.gemini_stream(model, system, msgs, false, on_delta, |_| {}).await,
            Provider::ClaudeCode => Err(AppError::Ai("Claude Code runs through the CLI, not the API.".into())),
        }
    }

    /// One prompt with web search (and fetch, where the provider has it). `on_event` gets
    /// `ToolUse` per search or page read and the model's text, for progress.
    pub async fn research(&self, model: &str, system: &str, prompt: &str, mut on_event: impl FnMut(&StreamEvent) + Send) -> Result<String> {
        let msgs = [Msg::user(prompt)];
        match self.provider {
            Provider::Anthropic => self.anthropic_research(model, system, prompt, on_event).await,
            Provider::Openai => {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let tx2 = tx.clone();
                let run = self.openai_stream(
                    model,
                    system,
                    &msgs,
                    Some(Effort::Medium),
                    true,
                    move |d| {
                        let _ = tx.send(StreamEvent::TextDelta(d.to_string()));
                    },
                    move |ev| {
                        let _ = tx2.send(ev);
                    },
                );
                drain_while(run, &mut rx, &mut on_event).await
            }
            Provider::Gemini => {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
                let tx2 = tx.clone();
                let run = self.gemini_stream(
                    model,
                    system,
                    &msgs,
                    true,
                    move |d| {
                        let _ = tx.send(StreamEvent::TextDelta(d.to_string()));
                    },
                    move |ev| {
                        let _ = tx2.send(ev);
                    },
                );
                drain_while(run, &mut rx, &mut on_event).await
            }
            Provider::ClaudeCode => Err(AppError::Ai("Claude Code runs through the CLI, not the API.".into())),
        }
    }

    /// Chat models this key can use, for the model pickers in Settings.
    pub async fn models(&self) -> Result<Vec<String>> {
        let url = match self.provider {
            Provider::Anthropic => format!("{ANTHROPIC}/models?limit=100"),
            Provider::Openai => format!("{OPENAI}/models"),
            Provider::Gemini => format!("{GEMINI}/models?pageSize=1000"),
            Provider::ClaudeCode => return Ok(["sonnet", "opus", "haiku", "fable"].map(String::from).to_vec()),
        };
        let resp = self.auth(self.http.get(url).timeout(Duration::from_secs(20))).send().await.map_err(|e| self.net(e))?;
        let v = self.check(resp).await?.json::<Value>().await.map_err(|e| self.net(e))?;
        Ok(chat_models(self.provider, &v))
    }

    fn net(&self, e: reqwest::Error) -> AppError {
        let what = if e.is_timeout() { "timed out" } else if e.is_connect() { "could not connect" } else { "connection failed" };
        tracing::warn!(provider = self.provider.id(), "{what}: {e}");
        AppError::Ai(format!("{}: {what}. Check your internet connection.", self.provider.label()))
    }

    /// Turns an HTTP error into a short message; passes a good response through.
    async fn check(&self, resp: reqwest::Response) -> Result<reqwest::Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let body = resp.text().await.unwrap_or_default();
        tracing::warn!(provider = self.provider.id(), %status, "API error: {body}");
        Err(AppError::Ai(http_error(self.provider, status.as_u16(), &body)))
    }

    // ---------- Claude API ----------

    async fn anthropic_chat(&self, model: &str, system: &str, msgs: &[Msg], effort: Effort, mut on_delta: impl FnMut(&str) + Send) -> Result<String> {
        let messages: Vec<Value> = merged(msgs).iter().map(|m| json!({"role": role_name(m.role, "assistant"), "content": m.text})).collect();
        let mut body = json!({"model": model, "max_tokens": 16000, "system": system, "messages": messages, "stream": true});
        if claude_has_effort(model) {
            body["output_config"] = json!({"effort": effort.as_str()});
        }
        let resp = self.anthropic_send(model, &mut body, CHAT_TIMEOUT).await?;
        let mut text = String::new();
        let mut stop = String::new();
        let mut failed: Option<String> = None;
        sse(resp, |_event, data| {
            let Ok(v) = serde_json::from_str::<Value>(data) else { return };
            match v["type"].as_str() {
                Some("content_block_delta") if v["delta"]["type"] == "text_delta" => {
                    let d = v["delta"]["text"].as_str().unwrap_or_default();
                    text.push_str(d);
                    on_delta(d);
                }
                Some("message_delta") => {
                    if let Some(s) = v["delta"]["stop_reason"].as_str() {
                        stop = s.to_string();
                    }
                }
                Some("error") => failed = Some(v["error"]["message"].as_str().unwrap_or("stream error").to_string()),
                _ => {}
            }
        })
        .await
        .map_err(|e| self.net(e))?;
        if let Some(m) = failed {
            return Err(AppError::Ai(format!("Claude API: {m}")));
        }
        if text.trim().is_empty() && stop == "refusal" {
            return Err(AppError::Ai("Claude declined to answer that. Try rephrasing.".into()));
        }
        Ok(text)
    }

    /// Sends a Messages request. Current models get server-side refusal fallbacks; if the
    /// API rejects that option, the request goes again without it.
    async fn anthropic_send(&self, model: &str, body: &mut Value, timeout: Duration) -> Result<reqwest::Response> {
        let url = format!("{ANTHROPIC}/messages");
        if claude_has_fallbacks(model) {
            body["fallbacks"] = json!("default");
            let resp = self
                .post(&url, body, timeout)
                .header("anthropic-beta", "server-side-fallback-2026-07-01")
                .send()
                .await
                .map_err(|e| self.net(e))?;
            if resp.status().as_u16() != 400 {
                return self.check(resp).await;
            }
            let text = resp.text().await.unwrap_or_default();
            if !text.contains("fallback") {
                return Err(AppError::Ai(http_error(self.provider, 400, &text)));
            }
            tracing::warn!("Claude API refused refusal fallbacks; sending without them: {text}");
            body.as_object_mut().map(|o| o.remove("fallbacks"));
        }
        let resp = self.post(&url, body, timeout).send().await.map_err(|e| self.net(e))?;
        self.check(resp).await
    }

    /// Not streamed: the server runs searches between turns, and a paused turn has to be sent
    /// back whole, which the full response gives directly.
    async fn anthropic_research(&self, model: &str, system: &str, prompt: &str, mut on_event: impl FnMut(&StreamEvent) + Send) -> Result<String> {
        let tools = if claude_has_dynamic_web(model) {
            json!([
                {"type": "web_search_20260209", "name": "web_search", "max_uses": 30},
                {"type": "web_fetch_20260209", "name": "web_fetch", "max_uses": 30}
            ])
        } else {
            json!([
                {"type": "web_search_20250305", "name": "web_search", "max_uses": 30},
                {"type": "web_fetch_20250910", "name": "web_fetch", "max_uses": 30}
            ])
        };
        let mut messages = vec![json!({"role": "user", "content": prompt})];
        for _ in 0..=MAX_CONTINUATIONS {
            let mut body = json!({"model": model, "max_tokens": 16000, "system": system, "messages": messages, "tools": tools});
            if claude_has_effort(model) {
                body["output_config"] = json!({"effort": "medium"});
            }
            let resp = self.anthropic_send(model, &mut body, RESEARCH_TIMEOUT).await?;
            let v: Value = resp.json().await.map_err(|e| self.net(e))?;
            let mut text = String::new();
            for block in v["content"].as_array().into_iter().flatten() {
                match block["type"].as_str() {
                    Some("server_tool_use") => match block["name"].as_str() {
                        Some("web_search") => on_event(&StreamEvent::ToolUse { name: "WebSearch".into() }),
                        Some("web_fetch") => on_event(&StreamEvent::ToolUse { name: "WebFetch".into() }),
                        _ => {}
                    },
                    Some("text") => text.push_str(block["text"].as_str().unwrap_or_default()),
                    _ => {}
                }
            }
            if !text.is_empty() {
                on_event(&StreamEvent::AssistantText(text.clone()));
            }
            match v["stop_reason"].as_str() {
                // Resume: send the paused turn back as is; the server picks up where it stopped.
                Some("pause_turn") => messages.push(json!({"role": "assistant", "content": v["content"]})),
                Some("refusal") if text.trim().is_empty() => return Err(AppError::Ai("Claude declined this research request.".into())),
                _ => return Ok(text),
            }
        }
        Err(AppError::Ai("The research ran too many search steps without finishing. Try again.".into()))
    }

    // ---------- OpenAI (Responses API) ----------

    #[allow(clippy::too_many_arguments)]
    async fn openai_stream(
        &self,
        model: &str,
        system: &str,
        msgs: &[Msg],
        effort: Option<Effort>,
        web: bool,
        mut on_delta: impl FnMut(&str) + Send,
        mut on_tool: impl FnMut(StreamEvent) + Send,
    ) -> Result<String> {
        let input: Vec<Value> = merged(msgs).iter().map(|m| json!({"role": role_name(m.role, "assistant"), "content": m.text})).collect();
        let mut body = json!({"model": model, "instructions": system, "input": input, "stream": true, "store": false});
        if let (Some(e), true) = (effort, openai_reasons(model)) {
            body["reasoning"] = json!({"effort": e.as_str()});
        }
        if web {
            body["tools"] = json!([{"type": "web_search"}]);
        }
        let timeout = if web { RESEARCH_TIMEOUT } else { CHAT_TIMEOUT };
        let resp = self.post(&format!("{OPENAI}/responses"), &body, timeout).send().await.map_err(|e| self.net(e))?;
        let resp = self.check(resp).await?;
        let mut text = String::new();
        let mut failed: Option<String> = None;
        sse(resp, |_event, data| {
            let Ok(v) = serde_json::from_str::<Value>(data) else { return };
            match v["type"].as_str() {
                Some("response.output_text.delta") => {
                    let d = v["delta"].as_str().unwrap_or_default();
                    text.push_str(d);
                    on_delta(d);
                }
                Some("response.output_item.done") if v["item"]["type"] == "web_search_call" => {
                    let name = if v["item"]["action"]["type"] == "search" { "WebSearch" } else { "WebFetch" };
                    on_tool(StreamEvent::ToolUse { name: name.into() });
                }
                Some("response.failed") => {
                    failed = Some(v["response"]["error"]["message"].as_str().unwrap_or("the reply failed").to_string())
                }
                Some("response.incomplete") => {
                    let why = v["response"]["incomplete_details"]["reason"].as_str().unwrap_or("unknown");
                    failed.get_or_insert_with(|| format!("the reply stopped early ({why})"));
                }
                Some("error") => failed = Some(v["message"].as_str().or(v["error"]["message"].as_str()).unwrap_or("stream error").to_string()),
                _ => {}
            }
        })
        .await
        .map_err(|e| self.net(e))?;
        match failed {
            // A reply cut short by the token cap is still usable.
            Some(m) if text.trim().is_empty() || !m.contains("max_output_tokens") => Err(AppError::Ai(format!("OpenAI: {m}"))),
            _ => Ok(text),
        }
    }

    // ---------- Gemini ----------

    async fn gemini_stream(
        &self,
        model: &str,
        system: &str,
        msgs: &[Msg],
        web: bool,
        mut on_delta: impl FnMut(&str) + Send,
        mut on_tool: impl FnMut(StreamEvent) + Send,
    ) -> Result<String> {
        let contents: Vec<Value> = merged(msgs).iter().map(|m| json!({"role": role_name(m.role, "model"), "parts": [{"text": m.text}]})).collect();
        let mut body = json!({"systemInstruction": {"parts": [{"text": system}]}, "contents": contents});
        if web {
            body["tools"] = json!([{"google_search": {}}]);
        }
        let model = model.trim_start_matches("models/");
        let url = format!("{GEMINI}/models/{model}:streamGenerateContent?alt=sse");
        let timeout = if web { RESEARCH_TIMEOUT } else { CHAT_TIMEOUT };
        let resp = self.post(&url, &body, timeout).send().await.map_err(|e| self.net(e))?;
        let resp = self.check(resp).await?;
        let mut text = String::new();
        let mut blocked: Option<String> = None;
        let mut searches = 0usize;
        sse(resp, |_event, data| {
            let Ok(v) = serde_json::from_str::<Value>(data) else { return };
            if let Some(m) = v["error"]["message"].as_str() {
                blocked = Some(m.to_string());
            }
            if let Some(r) = v["promptFeedback"]["blockReason"].as_str() {
                blocked = Some(format!("blocked ({r})"));
            }
            let cand = &v["candidates"][0];
            for part in cand["content"]["parts"].as_array().into_iter().flatten() {
                if part["thought"].as_bool() == Some(true) {
                    continue;
                }
                if let Some(d) = part["text"].as_str() {
                    text.push_str(d);
                    on_delta(d);
                }
            }
            // Grounding metadata repeats the queries so far; report only the new ones.
            let n = cand["groundingMetadata"]["webSearchQueries"].as_array().map_or(0, Vec::len);
            for _ in searches..n {
                on_tool(StreamEvent::ToolUse { name: "WebSearch".into() });
            }
            searches = searches.max(n);
        })
        .await
        .map_err(|e| self.net(e))?;
        match blocked {
            Some(m) if text.trim().is_empty() => Err(AppError::Ai(format!("Gemini: {m}"))),
            _ => Ok(text),
        }
    }
}

/// Runs `fut` while forwarding the events it queues, so `on_event` needn't be shared with it.
async fn drain_while(
    fut: impl std::future::Future<Output = Result<String>>,
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<StreamEvent>,
    on_event: &mut (impl FnMut(&StreamEvent) + Send),
) -> Result<String> {
    tokio::pin!(fut);
    let out = loop {
        tokio::select! {
            r = &mut fut => break r,
            Some(ev) = rx.recv() => on_event(&ev),
        }
    };
    while let Ok(ev) = rx.try_recv() {
        on_event(&ev);
    }
    out
}

fn role_name(r: Role, assistant: &'static str) -> &'static str {
    match r {
        Role::User => "user",
        Role::Assistant => assistant,
    }
}

/// Joins back-to-back turns from the same side (a failed reply leaves two user turns in a row).
pub fn merged(msgs: &[Msg]) -> Vec<Msg> {
    let mut out: Vec<Msg> = Vec::with_capacity(msgs.len());
    for m in msgs {
        match out.last_mut() {
            Some(last) if last.role == m.role => {
                last.text.push_str("\n\n");
                last.text.push_str(&m.text);
            }
            _ => out.push(m.clone()),
        }
    }
    out
}

/// Claude 5 models take `output_config.effort`; older ones may reject it.
fn claude_has_effort(model: &str) -> bool {
    ["claude-opus-5", "claude-sonnet-5", "claude-haiku-5", "claude-fable-5", "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6"]
        .iter()
        .any(|p| model.starts_with(p))
}

fn claude_has_fallbacks(model: &str) -> bool {
    ["claude-fable-5-1", "claude-opus-5-5", "claude-opus-5", "claude-sonnet-5-5"].contains(&model)
}

/// The newer web tools (with dynamic filtering) need Opus 4.6+ or Sonnet 4.6+.
fn claude_has_dynamic_web(model: &str) -> bool {
    ["claude-opus-5", "claude-sonnet-5", "claude-fable-5", "claude-opus-4-8", "claude-opus-4-7", "claude-opus-4-6", "claude-sonnet-4-6"]
        .iter()
        .any(|p| model.starts_with(p))
}

/// Reasoning models take `reasoning.effort`; older chat models reject it.
fn openai_reasons(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("gpt-5") || m.starts_with("gpt-6") || (m.starts_with('o') && m[1..].starts_with(|c: char| c.is_ascii_digit()))
}

/// Text-chat models from a provider's model list, best guesses first where it matters.
pub fn chat_models(p: Provider, v: &Value) -> Vec<String> {
    const SKIP: [&str; 14] =
        ["audio", "realtime", "tts", "transcribe", "image", "embedding", "moderation", "search", "instruct", "dall-e", "whisper", "live", "robotics", "lyria"];
    let ok = |id: &str| !SKIP.iter().any(|s| id.contains(s));
    let mut ids: Vec<String> = match p {
        Provider::Anthropic => v["data"].as_array().into_iter().flatten().filter_map(|m| m["id"].as_str()).map(String::from).collect(),
        Provider::Openai => v["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| m["id"].as_str())
            .filter(|id| (id.starts_with("gpt-") || openai_reasons(id)) && ok(id))
            .map(String::from)
            .collect(),
        Provider::Gemini => v["models"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["supportedGenerationMethods"].as_array().is_some_and(|a| a.iter().any(|x| x == "generateContent")))
            .filter_map(|m| m["name"].as_str())
            .map(|n| n.trim_start_matches("models/"))
            .filter(|id| id.starts_with("gemini") && ok(id))
            .map(String::from)
            .collect(),
        Provider::ClaudeCode => Vec::new(),
    };
    if p != Provider::Anthropic {
        ids.sort();
        ids.reverse();
    }
    ids.dedup();
    // The defaults are aliases that may not appear in the list; keep them pickable.
    let (iv, rep) = p.default_models();
    for d in [rep, iv] {
        if !ids.iter().any(|x| x == d) {
            ids.insert(0, d.to_string());
        }
    }
    ids
}

/// Short, plain error text for an HTTP failure.
pub fn http_error(p: Provider, status: u16, body: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let v = if v.is_array() { v[0].clone() } else { v };
    let msg = v["error"]["message"].as_str().or(v["message"].as_str()).unwrap_or("").trim().to_string();
    let msg: String = msg.chars().take(240).collect();
    let name = p.label();
    match status {
        401 | 403 => format!("{name} rejected the API key. Check it in Settings."),
        404 => format!("{name} doesn't know that model. Pick another one in Settings.{}", if msg.is_empty() { String::new() } else { format!(" ({msg})") }),
        429 => format!("{name} rate limit or quota reached. Wait a minute, or check your plan.{}", if msg.is_empty() { String::new() } else { format!(" ({msg})") }),
        _ if msg.is_empty() => format!("{name} returned HTTP {status}."),
        _ => format!("{name}: {msg}"),
    }
}

/// Server-sent events: calls `on(event, data)` for each complete event.
async fn sse(resp: reqwest::Response, mut on: impl FnMut(Option<&str>, &str)) -> std::result::Result<(), reqwest::Error> {
    let mut parser = SseParser::default();
    let mut body = resp.bytes_stream();
    while let Some(chunk) = body.next().await {
        for (event, data) in parser.push(&chunk?) {
            on(event.as_deref(), &data);
        }
    }
    for (event, data) in parser.finish() {
        on(event.as_deref(), &data);
    }
    Ok(())
}

#[derive(Default)]
pub struct SseParser {
    buf: Vec<u8>,
    event: Option<String>,
    data: String,
}

impl SseParser {
    pub fn push(&mut self, bytes: &[u8]) -> Vec<(Option<String>, String)> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    out.push((self.event.take(), std::mem::take(&mut self.data)));
                }
                self.event = None;
            } else if let Some(v) = line.strip_prefix("data:") {
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(v.strip_prefix(' ').unwrap_or(v));
            } else if let Some(v) = line.strip_prefix("event:") {
                self.event = Some(v.trim().to_string());
            }
        }
        out
    }

    pub fn finish(&mut self) -> Vec<(Option<String>, String)> {
        let mut out = self.push(b"\n\n");
        if !self.data.is_empty() {
            out.push((self.event.take(), std::mem::take(&mut self.data)));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_events_split_across_chunks() {
        let mut p = SseParser::default();
        assert!(p.push(b"event: message_start\ndata: {\"a\"").is_empty());
        let out = p.push(b":1}\r\n\r\ndata: x\ndata: y\n\n: comment\n");
        assert_eq!(out, vec![(Some("message_start".into()), "{\"a\":1}".into()), (None, "x\ny".into())]);
        assert!(p.push(b"data: tail").is_empty());
        assert_eq!(p.finish(), vec![(None, "tail".into())]);
    }

    #[test]
    fn same_side_turns_are_merged() {
        let m = merged(&[Msg::user("a"), Msg::user("b"), Msg { role: Role::Assistant, text: "c".into() }]);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].text, "a\n\nb");
    }

    #[test]
    fn model_lists_keep_chat_models_and_defaults() {
        let oa = json!({"data": [{"id": "gpt-5.5"}, {"id": "gpt-realtime"}, {"id": "o4-mini"}, {"id": "text-embedding-3"}, {"id": "dall-e-3"}]});
        assert_eq!(chat_models(Provider::Openai, &oa), ["o4-mini", "gpt-5.5"]);
        let g = json!({"models": [
            {"name": "models/gemini-3.8-flash", "supportedGenerationMethods": ["generateContent"]},
            {"name": "models/gemini-embedding-2", "supportedGenerationMethods": ["embedContent"]},
            {"name": "models/gemini-3.8-flash-tts", "supportedGenerationMethods": ["generateContent"]}
        ]});
        assert_eq!(chat_models(Provider::Gemini, &g), ["gemini-flash-latest", "gemini-3.8-flash"]);
        let a = json!({"data": [{"id": "claude-opus-5-5"}, {"id": "claude-sonnet-5-5"}, {"id": "claude-haiku-5-5"}]});
        assert_eq!(chat_models(Provider::Anthropic, &a), ["claude-opus-5-5", "claude-sonnet-5-5", "claude-haiku-5-5"]);
    }

    #[test]
    fn errors_are_short_and_plain() {
        let body = r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#;
        assert_eq!(http_error(Provider::Anthropic, 401, body), "Claude API rejected the API key. Check it in Settings.");
        let g = r#"[{"error":{"code":429,"message":"Quota exceeded"}}]"#;
        assert!(http_error(Provider::Gemini, 429, g).ends_with("(Quota exceeded)"));
        assert_eq!(http_error(Provider::Openai, 500, ""), "OpenAI returned HTTP 500.");
    }

    #[test]
    fn model_capabilities() {
        assert!(claude_has_effort("claude-sonnet-5-5") && !claude_has_effort("claude-haiku-4-5"));
        assert!(claude_has_dynamic_web("claude-opus-5-5") && !claude_has_dynamic_web("claude-haiku-5-5"));
        assert!(openai_reasons("gpt-5.5") && openai_reasons("o3") && !openai_reasons("gpt-4.1") && !openai_reasons("omni"));
    }
}
