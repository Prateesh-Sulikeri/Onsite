//! One-shot jobs (reports, verdicts, health check, research). Claude Code: prompt on stdin,
//! stream-json out. Provider APIs: one request (research: with web search).

use super::chat::{api_once, Backend};
use super::locate::{isolation_args, ClaudeLocation, Tools};
use super::parse::extract_json;
use super::stream::{parse_line, StreamEvent};
use crate::error::{AppError, Result};
use serde::Serialize;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[derive(Debug, Clone, Serialize)]
pub struct OneShot {
    pub text: String,
    pub json: Option<serde_json::Value>,
    pub duration_ms: u64,
    pub model: String,
}

const JOB_SYSTEM_PROMPT: &str = "You are a careful assistant inside a mock-interview app. Follow the user's instructions exactly. When asked for JSON, reply with only that JSON.";

/// Reports and verdicts finish in well under a minute; this only catches a hung CLI.
const JOB_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Runs one prompt to completion. `on_event` sees every parsed event (for progress).
pub async fn run_once(
    backend: &Backend,
    prompt: &str,
    tools: Tools,
    model: &str,
    on_event: impl FnMut(&StreamEvent) + Send,
) -> Result<OneShot> {
    run_once_with_timeout(backend, prompt, tools, model, JOB_TIMEOUT, on_event).await
}

/// Like `run_once`, but gives up after `limit` (research jobs need far longer).
pub async fn run_once_with_timeout(
    backend: &Backend,
    prompt: &str,
    tools: Tools,
    model: &str,
    limit: Duration,
    on_event: impl FnMut(&StreamEvent) + Send,
) -> Result<OneShot> {
    match backend {
        Backend::Cli(loc) => run_cli(loc, prompt, tools, model, limit, on_event).await,
        Backend::Api(client) => {
            let started = Instant::now();
            let text = api_once(client, JOB_SYSTEM_PROMPT, prompt, tools == Tools::Web, model, limit, on_event).await?;
            Ok(OneShot { json: extract_json(&text), text, duration_ms: started.elapsed().as_millis() as u64, model: model.to_string() })
        }
    }
}

async fn run_cli(
    loc: &ClaudeLocation,
    prompt: &str,
    tools: Tools,
    model: &str,
    limit: Duration,
    mut on_event: impl FnMut(&StreamEvent) + Send,
) -> Result<OneShot> {
    let workdir = tempfile::Builder::new().prefix("onsite-job-").tempdir()?;
    let sp = workdir.path().join(".system-prompt.md");
    std::fs::write(&sp, JOB_SYSTEM_PROMPT)?;
    let mut args = isolation_args(tools, model, Some(&loc.path_arg(&sp)));
    args.extend(["--output-format", "stream-json", "--verbose"].map(String::from));

    let started = Instant::now();
    let mut cmd = loc.command(&args, workdir.path());
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| AppError::Claude(format!("could not start: {e}")))?;

    // Prompt goes over stdin: transcripts are far longer than a Windows command line allows.
    let mut stdin = child.stdin.take().expect("piped stdin");
    stdin.write_all(prompt.as_bytes()).await?;
    drop(stdin);

    let mut stderr = child.stderr.take().expect("piped stderr");
    let stderr_task = tokio::spawn(async move {
        let mut s = String::new();
        let _ = tokio::io::AsyncReadExt::read_to_string(&mut stderr, &mut s).await;
        s
    });

    let mut lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
    let mut result: Option<(String, bool)> = None;
    let mut model_used = String::new();
    let read = async {
        while let Some(line) = lines.next_line().await? {
            let Ok(events) = parse_line(&line) else { continue };
            for ev in events {
                on_event(&ev);
                match ev {
                    StreamEvent::Init { model, .. } => model_used = model,
                    StreamEvent::Result { text, is_error, .. } => result = Some((text, is_error)),
                    _ => {}
                }
            }
        }
        Ok::<(), std::io::Error>(())
    };
    if tokio::time::timeout(limit, read).await.is_err() {
        let _ = child.kill().await;
        tracing::warn!(secs = limit.as_secs(), "one-shot job timed out");
        return Err(AppError::Claude(format!("no reply after {} minutes. Try again.", limit.as_secs() / 60)));
    }
    let status = child.wait().await?;
    let stderr = stderr_task.await.unwrap_or_default();
    let duration_ms = started.elapsed().as_millis() as u64;
    match result {
        Some((text, false)) => Ok(OneShot { json: extract_json(&text), text, duration_ms, model: model_used }),
        Some((text, true)) => {
            tracing::warn!(%status, stderr = %stderr.trim(), "one-shot job failed: {text}");
            Err(AppError::Claude(if text.trim().is_empty() { "the job failed with no message".into() } else { text }))
        }
        None => {
            let err = stderr.trim().to_string();
            Err(AppError::Claude(if err.is_empty() { format!("no result (exit {status})") } else { err }))
        }
    }
}
