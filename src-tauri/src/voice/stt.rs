//! One spoken answer: mic -> VAD -> VoiceStudio streaming STT -> final text.
//!
//! VoiceStudio 0.5.6 facts (read from its backend source, checked live):
//! - The stream endpoint runs the *dictation* engine. With no `model=` it uses the user's
//!   dictation pref (default `sherpa-whisper-tiny`), which drops everything past ~30 s of
//!   unbroken speech and loops ("for the MQ for the MQ ...") on long audio.
//!   `model=sherpa-parakeet-tdt-v3` (sherpa-onnx, CPU) is far more accurate and does not loop.
//! - The stream takes no language or prompt. Only batch `/v1/audio/transcriptions` does, and it
//!   reloads Whisper large-v3 on every request (~8 s), which is too slow for turn-taking.
//! - Summary final = all utterance finals joined, so they carry the same text.
//!
//! So we: pick the best installed model, cut long answers into segments at short pauses
//! (one WS session each, so the tail left to decode at the end stays short), fix domain words
//! against a vocabulary, and collapse repetition loops.
//!
//! Parakeet still mangles tech terms ("Postgres Creole", "reddish" for Redis), so each finished
//! segment also goes to Whisper large-v3 on the GPU ([`super::whisper`]), with the vocabulary as
//! its prompt. Parakeet gives the live caption; Whisper's text is what gets sent, when it works.

use super::audio::{i16_to_f32, rms, Level, Mic, Vad, VadEvent, MIC_RATE};
use super::builtin::ParakeetStt;
use super::engine::Engine;
use super::vs::VoiceStudio;
use super::{whisper, VoiceEvent};
use crate::error::{AppError, Result};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::Notify;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

const FRAME: usize = (MIC_RATE / 50) as usize; // 20 ms
const PREROLL: usize = (MIC_RATE as usize * 400) / 1000;
/// A segment needs this much audio before a short pause may cut it.
const SEG_MIN: usize = MIC_RATE as usize * 3;
/// Past this length, cut at a shorter pause so one segment never grows huge.
const SEG_LONG: usize = MIC_RATE as usize * 20;
const SEG_PAUSE_MS: u32 = 700;
const SEG_PAUSE_LONG_MS: u32 = 300;
/// Wait this long for the last segment's summary after the answer ends.
const FINAL_WAIT: Duration = Duration::from_secs(15);

/// Dictation models to request, best first. Only installed ones are used.
/// v2 is English-only. v3 guesses among 25 languages and turned accented English into Spanish,
/// so it is only a stopgap while v2 downloads.
const MODEL_PREFERENCE: &[&str] = &[ENGLISH_MODEL, "sherpa-parakeet-tdt-v3"];
pub const ENGLISH_MODEL: &str = "sherpa-parakeet-tdt-v2";

/// Tech and payments words the recogniser often gets wrong. Matched case- and space-insensitively.
pub const TECH_TERMS: &[&str] = &[
    "Java", "JVM", "Kotlin", "Spring Boot", "Hibernate", "JPA", "Kafka", "Kafka Streams", "ZooKeeper",
    "RabbitMQ", "ActiveMQ", "IBM MQ", "PostgreSQL", "Postgres", "MySQL", "Redis", "MongoDB", "Cassandra",
    "Elasticsearch", "DynamoDB", "Kubernetes", "Docker", "gRPC", "GraphQL", "JSON", "OAuth", "JWT",
    "microservice", "microservices", "idempotency", "idempotent", "CQRS", "sharding", "Prometheus",
    "Grafana", "BFSI", "SWIFT", "MT103", "MT202", "MT300", "MT940", "ISO 20022", "pacs.008", "pacs.009",
    "camt.053", "UPI", "NEFT", "RTGS", "IMPS", "PCI DSS", "Razorpay",
];

/// Vocabulary for one answer: the static tech list plus caller context (company, JD, round terms).
pub fn domain_vocab(extra: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = TECH_TERMS.iter().map(|s| s.to_string()).collect();
    for e in extra.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        if !v.iter().any(|t| t.eq_ignore_ascii_case(e)) {
            v.push(e.to_string());
        }
    }
    v
}

#[derive(Debug, Clone)]
pub struct SttOptions {
    /// Dictation model id for `?model=`. `None` picks the best installed one.
    pub model: Option<String>,
    /// Words to restore in the transcript (see [`fix_vocab`]).
    pub vocab: Vec<String>,
}

impl Default for SttOptions {
    fn default() -> Self {
        Self { model: None, vocab: domain_vocab(&[]) }
    }
}

#[derive(Default)]
pub struct ListenCtl {
    pub finish: Notify,
    pub cancel: Notify,
    pub mic_level: Level,
}

#[derive(Debug, PartialEq)]
pub enum ListenEnd {
    Text { text: String, stt_ms: u64 },
    /// Cancelled (e.g. muted). `heard` is what was recognised so far, cleaned up.
    Cancelled { heard: String },
    NoSpeech,
}

/// What we have heard so far from one STT stream.
#[derive(Debug, Default)]
pub struct SttState {
    pub partial: String,
    pub finals: Vec<String>,
    pub summary: Option<String>,
}

impl SttState {
    /// Applies one server message. Returns a new partial transcript to show, if any.
    pub fn absorb(&mut self, msg: &Value) -> Result<Option<String>> {
        let text = || msg["text"].as_str().unwrap_or_default().trim().to_string();
        match msg["type"].as_str() {
            Some("partial") => {
                self.partial = text();
                Ok(Some(self.preview()))
            }
            Some("final") => {
                if msg["final_kind"].as_str() == Some("summary") {
                    self.summary = Some(text());
                } else if !text().is_empty() {
                    self.finals.push(text());
                    self.partial.clear();
                }
                Ok(Some(self.preview()))
            }
            Some("error") => {
                let detail = msg["detail"].as_str().or(msg["message"].as_str()).unwrap_or("transcription failed");
                Err(AppError::Voice(format!("VoiceStudio STT: {detail}")))
            }
            _ => Ok(None),
        }
    }

    fn preview(&self) -> String {
        let mut parts = self.finals.clone();
        if !self.partial.is_empty() {
            parts.push(self.partial.clone());
        }
        parts.join(" ")
    }

    /// The summary final is authoritative; then utterance finals; then the last partial.
    pub fn result(&self) -> String {
        match &self.summary {
            Some(s) if !s.is_empty() => s.clone(),
            _ => self.preview(),
        }
    }
}

// ---------- text clean-up (pure) ----------

fn norm(word: &str) -> String {
    word.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

/// Removes a phrase of 1-8 words repeated 3+ times in a row, keeping one copy.
/// Whisper-style loops ("for the MQ for the MQ for the MQ") come out as one phrase.
pub fn collapse_repeats(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let keys: Vec<String> = words.iter().map(|w| norm(w)).collect();
    let same = |a: usize, b: usize, len: usize| (0..len).all(|k| !keys[a + k].is_empty() && keys[a + k] == keys[b + k]);
    let mut out: Vec<&str> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        // Pick the phrase length that removes the most words; ties go to the shorter phrase.
        let mut best: Option<(usize, usize)> = None; // (len, copies)
        for len in 1..=8 {
            let mut copies = 1;
            while i + (copies + 1) * len <= words.len() && same(i, i + copies * len, len) {
                copies += 1;
            }
            if copies >= 3 && best.is_none_or(|(bl, bc)| (copies - 1) * len > (bc - 1) * bl) {
                best = Some((len, copies));
            }
        }
        match best {
            Some((len, copies)) => {
                // Keep the last copy so trailing punctuation survives ("MQ for the MQ.").
                out.extend(&words[i + (copies - 1) * len..i + copies * len]);
                i += copies * len;
            }
            None => {
                out.push(words[i]);
                i += 1;
            }
        }
    }
    out.join(" ")
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1; b.len() + 1];
        for (j, cb) in b.iter().enumerate() {
            cur[j + 1] = (prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Edits allowed for a fuzzy match against a term of this normalised length.
fn allowed_edits(len: usize) -> usize {
    match len {
        0..=7 => 0,
        8..=9 => 1,
        _ => 2,
    }
}

/// Restores vocabulary words split, misspelt or mis-cased by the recogniser, e.g.
/// "IDEM potency" -> "idempotency", "PostgresUL" -> "PostgreSQL", "IBM, MQ" -> "IBM MQ".
/// Short terms must match exactly (ignoring case, spaces and punctuation); long terms may be a
/// couple of edits off, but must share their first and last letters.
pub fn fix_vocab(text: &str, vocab: &[String]) -> String {
    let terms: Vec<(Vec<char>, &str)> =
        vocab.iter().map(|t| (norm(t).chars().collect::<Vec<_>>(), t.as_str())).filter(|(n, _)| !n.is_empty()).collect();
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        // (edits, window len, term); exact beats fuzzy, then longer window wins.
        let mut best: Option<(usize, usize, &str)> = None;
        for len in 1..=3.min(words.len() - i) {
            let win = &words[i..i + len];
            if win.iter().any(|w| norm(w).is_empty()) {
                break;
            }
            let joined: Vec<char> = win.iter().map(|w| norm(w)).collect::<String>().chars().collect();
            for (t, canon) in &terms {
                let max = allowed_edits(t.len());
                if joined.len().abs_diff(t.len()) > max || joined.first() != t.first() || joined.last() != t.last() {
                    continue;
                }
                let d = if joined == *t { 0 } else if max == 0 { continue } else { levenshtein(&joined, t) };
                if d <= max && best.is_none_or(|(bd, bl, _)| d < bd || (d == bd && len > bl)) {
                    best = Some((d, len, canon));
                }
            }
        }
        match best {
            Some((_, len, canon)) => {
                let first = words[i];
                let last = words[i + len - 1];
                let lead: String = first.chars().take_while(|c| !c.is_alphanumeric()).collect();
                let tail: String = {
                    let t: Vec<char> = last.chars().rev().take_while(|c| !c.is_alphanumeric()).collect();
                    t.into_iter().rev().collect()
                };
                out.push(format!("{lead}{canon}{tail}"));
                i += len;
            }
            None => {
                out.push(words[i].to_string());
                i += 1;
            }
        }
    }
    out.join(" ")
}

/// Final clean-up applied to every transcript we hand out.
pub fn clean_transcript(text: &str, vocab: &[String]) -> String {
    collapse_repeats(&fix_vocab(text, vocab))
}

// ---------- model choice ----------

/// Cached model pick. `Some(None)` = probed, nothing better installed (use server default).
static MODEL: Mutex<Option<Option<String>>> = Mutex::new(None);

fn http_base(vs: &VoiceStudio) -> String {
    let ws = vs.stt_ws_url(MIC_RATE);
    let base = ws.split("/v1/audio/").next().unwrap_or(&ws).to_string();
    base.replacen("ws", "http", 1)
}

/// Where the English-only model stands.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EnglishModel {
    Ready,
    /// Downloading through VoiceStudio; `pct` 0-100.
    Installing { pct: f32 },
    /// Not installed and the download could not start.
    Missing { reason: String },
}

/// `repo_id` of the English model in `GET /dictation/models`, when it is listed but not installed.
pub fn english_repo_to_install(models: &Value) -> Option<String> {
    let m = models["models"].as_array()?.iter().find(|m| m["id"] == ENGLISH_MODEL)?;
    (m["installed"].as_bool() != Some(true)).then(|| m["repo_id"].as_str().map(str::to_string)).flatten()
}

fn get(vs: &VoiceStudio, path: &str) -> reqwest::RequestBuilder {
    let mut req = reqwest::Client::new().get(format!("{}{path}", http_base(vs))).timeout(Duration::from_secs(3));
    if let Some(k) = std::env::var("OMNIVOICE_API_KEY").ok().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    req
}

/// Checks the English model and starts its download in VoiceStudio (0.66 GB, once) when missing.
pub async fn english_model(vs: &VoiceStudio) -> EnglishModel {
    let models: Value = match get(vs, "/dictation/models").send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => return EnglishModel::Missing { reason: e.to_string() },
    };
    let Some(repo) = english_repo_to_install(&models) else {
        return if pick_model(&models).as_deref() == Some(ENGLISH_MODEL) {
            EnglishModel::Ready
        } else {
            EnglishModel::Missing { reason: "VoiceStudio doesn't offer Parakeet TDT v2".into() }
        };
    };
    let jobs: Value = match get(vs, "/models/install/status").send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => return EnglishModel::Missing { reason: e.to_string() },
    };
    let job = jobs["jobs"].as_array().into_iter().flatten().find(|j| j["repo_id"] == repo.as_str());
    if let Some(j) = job.filter(|j| !matches!(j["state"].as_str(), Some("error" | "failed" | "cancelled"))) {
        let (done, total) = (j["bytes_done"].as_f64().unwrap_or(0.0), j["total_bytes"].as_f64().unwrap_or(0.0));
        return EnglishModel::Installing { pct: if total > 0.0 { (done / total * 100.0) as f32 } else { 0.0 } };
    }
    let mut req = reqwest::Client::new().post(format!("{}/models/install", http_base(vs))).timeout(Duration::from_secs(10));
    if let Some(k) = std::env::var("OMNIVOICE_API_KEY").ok().filter(|k| !k.is_empty()) {
        req = req.bearer_auth(k);
    }
    match req.json(&serde_json::json!({ "repo_id": repo })).send().await.and_then(|r| r.error_for_status()) {
        Ok(_) => {
            tracing::info!(%repo, "installing the English STT model in VoiceStudio");
            EnglishModel::Installing { pct: 0.0 }
        }
        Err(e) => {
            tracing::warn!(%repo, "English STT model install: {e}");
            EnglishModel::Missing { reason: e.to_string() }
        }
    }
}

/// Best installed dictation model from `GET /dictation/models`.
pub fn pick_model(models: &Value) -> Option<String> {
    let installed: Vec<&str> = models["models"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["installed"].as_bool() == Some(true))
        .filter_map(|m| m["id"].as_str())
        .collect();
    MODEL_PREFERENCE.iter().find(|p| installed.contains(p)).map(|s| s.to_string())
}

/// Picks (and un-demotes) the dictation model before the round, so the first answer doesn't
/// pay for it. None = VoiceStudio's own default (often Whisper Tiny).
pub async fn prepare(vs: &VoiceStudio) -> Option<String> {
    choose_model(vs).await
}

async fn choose_model(vs: &VoiceStudio) -> Option<String> {
    if let Some(m) = MODEL.lock().unwrap().clone() {
        return m;
    }
    let models: Value = match get(vs, "/dictation/models").send().await {
        Ok(r) => match r.json().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("dictation models: {e}");
                return None;
            }
        },
        Err(e) => {
            tracing::warn!("dictation models: {e}");
            return None;
        }
    };
    let pick = pick_model(&models);
    if pick.as_deref() != Some(ENGLISH_MODEL) {
        // Starts the download for next time; this answer uses what is installed.
        let vs = vs.clone();
        tokio::spawn(async move { english_model(&vs).await });
    }
    if let Some(m) = &pick {
        undemote(vs, m).await;
    }
    match &pick {
        Some(m) => tracing::info!(model = %m, "STT model"),
        None => tracing::warn!("no Parakeet dictation model installed; using VoiceStudio's default (often Whisper Tiny)"),
    }
    *MODEL.lock().unwrap() = Some(pick.clone());
    pick
}

/// Forget the cached pick, so the next answer probes (and un-demotes) again. Called per round.
pub fn reset_model_choice() {
    *MODEL.lock().unwrap() = None;
}

/// VoiceStudio demotes a sherpa model for good after one session where it decoded nothing
/// (a known Parakeet glitch), and then silently serves Whisper Tiny instead of the model we
/// ask for. Choosing a model in its dictation prefs clears the demotion, so we choose ours,
/// then put the owner's own choice back.
async fn undemote(vs: &VoiceStudio, model: &str) {
    let url = format!("{}/dictation/prefs", http_base(vs));
    let key = std::env::var("OMNIVOICE_API_KEY").ok().filter(|k| !k.is_empty());
    let client = reqwest::Client::new();
    let req = |m: reqwest::Method| {
        let r = client.request(m, &url).timeout(Duration::from_secs(3));
        match &key {
            Some(k) => r.bearer_auth(k),
            None => r,
        }
    };
    let own = match req(reqwest::Method::GET).send().await {
        Ok(r) => r.json::<Value>().await.ok().and_then(|v| v["model_id"].as_str().map(str::to_string)),
        Err(e) => {
            tracing::warn!("dictation prefs: {e}");
            return;
        }
    };
    for m in std::iter::once(model.to_string()).chain(own.filter(|o| o != model)) {
        if let Err(e) = req(reqwest::Method::POST).json(&serde_json::json!({ "model_id": m })).send().await.and_then(|r| r.error_for_status()) {
            tracing::warn!(model = %m, "dictation prefs: {e}");
        }
    }
}

// ---------- streaming ----------

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

enum SegMsg {
    Msg(usize, Value),
    Closed(usize),
    Failed(String),
}

/// Reader tasks for each segment's socket; aborted when the answer ends.
#[derive(Default)]
struct Readers(Vec<JoinHandle<()>>);

impl Drop for Readers {
    fn drop(&mut self) {
        self.0.iter().for_each(JoinHandle::abort);
    }
}

fn spawn_reader(idx: usize, mut stream: SplitStream<Ws>, tx: UnboundedSender<SegMsg>) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Some(m) = stream.next().await {
            match m {
                Ok(Message::Text(t)) => {
                    let v: Value = serde_json::from_str(t.as_str()).unwrap_or_default();
                    let _ = tx.send(SegMsg::Msg(idx, v));
                }
                Ok(Message::Close(_)) => break,
                Ok(_) => {}
                Err(e) => {
                    let _ = tx.send(SegMsg::Failed(e.to_string()));
                    return;
                }
            }
        }
        let _ = tx.send(SegMsg::Closed(idx));
    })
}

struct Seg {
    st: SttState,
    sink: Option<SplitSink<Ws, Message>>,
    done: bool,
    /// This segment's audio, handed to Whisper when the segment ends.
    pcm: Vec<i16>,
    whisper: Option<JoinHandle<Result<String>>>,
    /// Whisper's text; preferred over Parakeet's when present.
    fixed: Option<String>,
}

impl Seg {
    fn new(sink: Option<SplitSink<Ws, Message>>, pcm: Vec<i16>) -> Self {
        Self { st: SttState::default(), sink, done: false, pcm, whisper: None, fixed: None }
    }

    fn text(&self) -> String {
        self.fixed.clone().unwrap_or_else(|| self.st.result())
    }
}

/// Every segment of one answer, in order.
#[derive(Default)]
struct Answer {
    segs: Vec<Seg>,
}

impl Answer {
    /// Best text so far: each segment's summary when it has one, else its finals and partial.
    fn text(&self) -> String {
        self.segs.iter().map(Seg::text).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
    }

    fn all_done(&self) -> bool {
        self.segs.iter().all(|s| s.done)
    }

    /// Returns true when the visible transcript changed.
    fn on_msg(&mut self, m: SegMsg) -> Result<bool> {
        match m {
            SegMsg::Msg(i, v) => {
                let seg = &mut self.segs[i];
                let changed = seg.st.absorb(&v)?.is_some();
                if v["final_kind"] == "summary" {
                    seg.done = true;
                    seg.sink = None;
                }
                Ok(changed)
            }
            SegMsg::Closed(i) => {
                self.segs[i].done = true;
                self.segs[i].sink = None;
                Ok(false)
            }
            SegMsg::Failed(e) => Err(AppError::Voice(format!("VoiceStudio STT: {e}"))),
        }
    }

    /// Ends segment `i`: Parakeet gets end-of-audio, Whisper gets the segment's audio.
    async fn end(&mut self, i: usize, prompt: &str) -> Result<()> {
        let seg = &mut self.segs[i];
        if seg.whisper.is_none() && matches!(whisper::status(), whisper::WhisperStatus::Ready { .. }) {
            seg.whisper = Some(tokio::spawn(whisper::transcribe(std::mem::take(&mut seg.pcm), prompt.to_string())));
        }
        if let Some(sink) = self.segs[i].sink.as_mut() {
            sink.send(Message::Text(r#"{"type":"input_audio.end"}"#.into())).await.map_err(|e| AppError::Voice(e.to_string()))?;
        }
        Ok(())
    }
}

fn pcm_bytes(s: &[i16]) -> Vec<u8> {
    s.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Pause that ends the current segment (not the answer), given its length in samples.
fn seg_pause_ms(seg_samples: usize) -> Option<u32> {
    match seg_samples {
        n if n < SEG_MIN => None,
        n if n < SEG_LONG => Some(SEG_PAUSE_MS),
        _ => Some(SEG_PAUSE_LONG_MS),
    }
}

/// Listens until the pause after speech (or `finish`), then returns the transcript.
pub async fn listen(engine: &Engine, pause_ms: u32, ctl: &ListenCtl, emit: impl FnMut(VoiceEvent)) -> Result<ListenEnd> {
    listen_with(engine, pause_ms, ctl, &SttOptions::default(), emit).await
}

/// Where a segment's text comes from.
enum Source {
    /// VoiceStudio: one WebSocket per segment (URL).
    Ws(String),
    /// Built-in: Parakeet decodes the segment in this process.
    Local(Arc<ParakeetStt>),
}

/// The built-in engine re-decodes the open segment this often to show a caption.
const CAPTION_EVERY: Duration = Duration::from_millis(500);

/// Decodes a finished segment off the async runtime and reports it like VoiceStudio's summary message.
fn spawn_decode(stt: Arc<ParakeetStt>, idx: usize, pcm: Vec<i16>, tx: UnboundedSender<SegMsg>) {
    tokio::task::spawn_blocking(move || {
        let text = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stt.transcribe(&pcm)));
        let _ = tx.send(match text {
            Ok(text) => SegMsg::Msg(idx, serde_json::json!({"type": "final", "final_kind": "summary", "text": text})),
            Err(_) => SegMsg::Closed(idx),
        });
    });
}

/// Decodes the open segment for a live caption. Never more than one at a time (`busy`); skipped
/// while a finished segment is being decoded.
fn spawn_caption(stt: Arc<ParakeetStt>, idx: usize, pcm: Vec<i16>, busy: Arc<AtomicBool>, tx: UnboundedSender<SegMsg>) {
    busy.store(true, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || {
        let text = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stt.caption(&pcm))).ok().flatten();
        busy.store(false, Ordering::SeqCst);
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            let _ = tx.send(SegMsg::Msg(idx, serde_json::json!({"type": "partial", "text": text})));
        }
    });
}

/// [`listen`] with a model and vocabulary of the caller's choice.
pub async fn listen_with(
    engine: &Engine,
    pause_ms: u32,
    ctl: &ListenCtl,
    opts: &SttOptions,
    mut emit: impl FnMut(VoiceEvent),
) -> Result<ListenEnd> {
    let src = match engine {
        Engine::Studio(vs) => {
            let model = match &opts.model {
                Some(m) => Some(m.clone()).filter(|m| !m.is_empty()),
                None => choose_model(vs).await,
            };
            Source::Ws(match &model {
                Some(m) => format!("{}&model={m}", vs.stt_ws_url(MIC_RATE)),
                None => vs.stt_ws_url(MIC_RATE),
            })
        }
        Engine::Builtin(b) => Source::Local(b.stt.clone()),
    };
    let r = run(&src, pause_ms, ctl, opts, &mut emit).await;
    if r.is_err() {
        *MODEL.lock().unwrap() = None; // re-probe next time
    }
    r
}

/// Ends segment `i`: the built-in engine starts decoding it, then Whisper gets the audio too.
async fn end_segment(ans: &mut Answer, i: usize, prompt: &str, src: &Source, tx: &UnboundedSender<SegMsg>) -> Result<()> {
    if let Source::Local(stt) = src {
        spawn_decode(stt.clone(), i, ans.segs[i].pcm.clone(), tx.clone());
    }
    ans.end(i, prompt).await
}

async fn run(src: &Source, pause_ms: u32, ctl: &ListenCtl, opts: &SttOptions, emit: &mut impl FnMut(VoiceEvent)) -> Result<ListenEnd> {
    let (tx, mut rx) = unbounded_channel::<Vec<i16>>();
    let (seg_tx, mut seg_rx) = unbounded_channel::<SegMsg>();
    let mic = Mic::open(tx)?;
    let mut vad = Vad::new(20, pause_ms);
    let mut frame: Vec<i16> = Vec::with_capacity(FRAME);
    let mut preroll: VecDeque<i16> = VecDeque::with_capacity(PREROLL + FRAME);
    let mut ans = Answer::default();
    let mut readers = Readers::default();
    let mut active: Option<usize> = None;
    let mut seg_samples = 0usize;
    let mut last_level = Instant::now();
    let mut shown = String::new();
    let heard = |ans: &Answer| clean_transcript(&ans.text(), &opts.vocab);
    let prompt = whisper::prompt_for(&opts.vocab);
    let mut health = tokio::time::interval(Duration::from_millis(500));
    let mut mic_lost = false;
    let mut caption = tokio::time::interval(CAPTION_EVERY);
    caption.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let caption_busy = Arc::new(AtomicBool::new(false));
    // (segment, samples) of the last caption started: no re-decode of audio that hasn't grown.
    let mut captioned = (usize::MAX, 0usize);
    emit(VoiceEvent::Listening { active: true });

    loop {
        tokio::select! {
            _ = ctl.cancel.notified() => {
                emit(VoiceEvent::Listening { active: false });
                return Ok(ListenEnd::Cancelled { heard: heard(&ans) });
            }
            _ = ctl.finish.notified() => break,
            // An unplugged or switched mic sends nothing more; end with what we have.
            _ = health.tick() => if mic.is_dead() {
                tracing::warn!("microphone stream lost");
                mic_lost = true;
                break;
            },
            _ = caption.tick(), if matches!(src, Source::Local(_)) => {
                if let (Source::Local(stt), Some(i)) = (src, active) {
                    let n = ans.segs[i].pcm.len();
                    if n >= MIC_RATE as usize / 2 && captioned != (i, n) && !caption_busy.load(Ordering::SeqCst) {
                        captioned = (i, n);
                        spawn_caption(stt.clone(), i, ans.segs[i].pcm.clone(), caption_busy.clone(), seg_tx.clone());
                    }
                }
            }
            m = seg_rx.recv() => {
                let Some(m) = m else { continue };
                if ans.on_msg(m)? {
                    let p = heard(&ans);
                    if p != shown {
                        shown = p.clone();
                        emit(VoiceEvent::Partial { text: p });
                    }
                }
            }
            chunk = rx.recv() => {
                let Some(chunk) = chunk else { break };
                ctl.mic_level.set(mic.level.get());
                if last_level.elapsed() >= Duration::from_millis(80) {
                    last_level = Instant::now();
                    emit(VoiceEvent::Level { mic: mic.level.get(), out: 0.0 });
                }
                let (mut ended, mut started, mut loud) = (false, false, false);
                for &s in &chunk {
                    frame.push(s);
                    if frame.len() == FRAME {
                        let level = rms(&i16_to_f32(&frame));
                        loud |= level > vad.threshold();
                        match vad.push(level) {
                            VadEvent::SpeechStart => started = true,
                            VadEvent::SpeechEnd => ended = true,
                            VadEvent::None => {}
                        }
                        frame.clear();
                    }
                }
                // Open a segment on speech start, or when speech resumes after a segment cut.
                if active.is_none() && (started || (loud && vad.is_speaking())) {
                    if ans.segs.is_empty() {
                        emit(VoiceEvent::SpeechStart);
                    }
                    let head: Vec<i16> = preroll.drain(..).collect();
                    let idx = ans.segs.len();
                    let sink = match src {
                        Source::Ws(url) => {
                            let (w, _) = tokio_tungstenite::connect_async(url.as_str())
                                .await
                                .map_err(|e| AppError::Voice(format!("VoiceStudio STT connect: {e}")))?;
                            let (mut sink, stream) = w.split();
                            sink.send(Message::Binary(pcm_bytes(&head).into())).await.map_err(|e| AppError::Voice(e.to_string()))?;
                            readers.0.push(spawn_reader(idx, stream, seg_tx.clone()));
                            Some(sink)
                        }
                        Source::Local(_) => None,
                    };
                    ans.segs.push(Seg::new(sink, head.clone()));
                    active = Some(idx);
                    seg_samples = head.len();
                }
                match active {
                    Some(i) => {
                        if let Some(sink) = ans.segs[i].sink.as_mut() {
                            sink.send(Message::Binary(pcm_bytes(&chunk).into())).await.map_err(|e| AppError::Voice(e.to_string()))?;
                        }
                        ans.segs[i].pcm.extend_from_slice(&chunk);
                        seg_samples += chunk.len();
                        // Short pause inside a long answer: finish this segment now, so its
                        // decode runs while the candidate keeps talking.
                        let cut = seg_pause_ms(seg_samples).is_some_and(|p| vad.quiet_ms() >= p);
                        if !ended && vad.is_speaking() && cut {
                            end_segment(&mut ans, i, &prompt, src, &seg_tx).await?;
                            active = None;
                            preroll.clear();
                        }
                    }
                    None => {
                        preroll.extend(&chunk);
                        let extra = preroll.len().saturating_sub(PREROLL);
                        preroll.drain(..extra);
                    }
                }
                if ended {
                    break;
                }
            }
        }
    }

    drop(mic);
    ctl.mic_level.set(0.0);
    emit(VoiceEvent::Listening { active: false });
    if ans.segs.is_empty() {
        if mic_lost {
            return Err(AppError::Voice("The microphone changed. Tap the mic to listen again.".into()));
        }
        return Ok(ListenEnd::NoSpeech);
    }
    let ended_at = Instant::now();
    if let Some(i) = active {
        end_segment(&mut ans, i, &prompt, src, &seg_tx).await?;
    }
    // Whisper first. When every segment has Whisper's text, Parakeet's summaries aren't needed.
    for seg in ans.segs.iter_mut() {
        let Some(job) = seg.whisper.take() else { continue };
        tokio::select! {
            r = job => match r {
                Ok(Ok(t)) if !t.trim().is_empty() => seg.fixed = Some(t),
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!("Whisper segment failed, using Parakeet: {e}"),
                Err(e) => tracing::warn!("Whisper segment task: {e}"),
            },
            _ = ctl.cancel.notified() => return Ok(ListenEnd::Cancelled { heard: heard(&ans) }),
        }
    }
    let by_whisper = ans.segs.iter().filter(|s| s.fixed.is_some()).count();
    let deadline = tokio::time::sleep(FINAL_WAIT);
    tokio::pin!(deadline);
    while by_whisper < ans.segs.len() && !ans.all_done() {
        tokio::select! {
            _ = &mut deadline => { tracing::warn!("STT summary timed out; using what we have"); break; }
            _ = ctl.cancel.notified() => return Ok(ListenEnd::Cancelled { heard: heard(&ans) }),
            m = seg_rx.recv() => match m {
                Some(m) => if ans.on_msg(m)? {
                    let p = heard(&ans);
                    if p != shown {
                        shown = p.clone();
                        emit(VoiceEvent::Partial { text: p });
                    }
                },
                None => break,
            }
        }
    }
    drop(readers);
    let raw = ans.text();
    let text = clean_transcript(&raw, &opts.vocab);
    let stt_ms = ended_at.elapsed().as_millis() as u64;
    tracing::info!(stt_ms, segments = ans.segs.len(), by_whisper, words = text.split_whitespace().count(), "utterance transcribed");
    if raw != text {
        tracing::debug!(%raw, "transcript cleaned");
    }
    Ok(if text.is_empty() { ListenEnd::NoSpeech } else { ListenEnd::Text { text, stt_ms } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stt_state_prefers_summary() {
        let mut s = SttState::default();
        assert_eq!(s.absorb(&json!({"type":"session.started"})).unwrap(), None);
        assert_eq!(s.absorb(&json!({"type":"partial","text":"Walk me"})).unwrap().as_deref(), Some("Walk me"));
        s.absorb(&json!({"type":"final","final_kind":"utterance","text":"Walk me through it."})).unwrap();
        assert_eq!(s.absorb(&json!({"type":"partial","text":"Then"})).unwrap().as_deref(), Some("Walk me through it. Then"));
        assert_eq!(s.result(), "Walk me through it. Then");
        s.absorb(&json!({"type":"final","final_kind":"summary","text":"Walk me through it. Then stop."})).unwrap();
        assert_eq!(s.result(), "Walk me through it. Then stop.");
        assert!(s.absorb(&json!({"type":"error","detail":"boom"})).is_err());
    }

    #[test]
    fn collapse_repeats_kills_loops() {
        let looped = format!("I have used IBM MQ{} and Kafka.", " for the MQ".repeat(40));
        assert_eq!(collapse_repeats(&looped), "I have used IBM MQ for the MQ and Kafka.");
        assert_eq!(collapse_repeats("no no no no wait"), "no wait");
        assert_eq!(collapse_repeats("Thank you. Thank you. Thank you."), "Thank you.");
        assert_eq!(collapse_repeats("the cache the cache the cache layer"), "the cache layer");
        // Two copies are normal speech; leave them.
        assert_eq!(collapse_repeats("very very good, we retry and retry"), "very very good, we retry and retry");
        assert_eq!(collapse_repeats(""), "");
        // A nine-word phrase is longer than we look for.
        let nine = "one two three four five six seven eight nine ";
        assert_eq!(collapse_repeats(&nine.repeat(3)).split_whitespace().count(), 27);
    }

    #[test]
    fn fix_vocab_restores_domain_words() {
        let v = domain_vocab(&["Razorpay", "Hazelcast"]);
        let cases = [
            ("One of the hardest problems was IDEM potency.", "One of the hardest problems was idempotency."),
            ("we keep an IDEMP potency table in PostgresUL with", "we keep an idempotency table in PostgreSQL with"),
            ("we store the state in POSGRISQL, and", "we store the state in PostgreSQL, and"),
            ("over IBM, MQ and Kafka.", "over IBM MQ and Kafka."),
            ("migrating to ISO 2022 MX messages", "migrating to ISO 20022 MX messages"),
            ("we send swift mt103 messages", "we send SWIFT MT103 messages"),
            ("written in java with spring boot", "written in Java with Spring Boot"),
            ("a hazel cast cluster", "a Hazelcast cluster"),
        ];
        for (input, want) in cases {
            assert_eq!(fix_vocab(input, &v), want, "{input}");
        }
        // Must not eat neighbouring words or touch ordinary English.
        assert_eq!(fix_vocab("a PostgreSQL table", &v), "a PostgreSQL table");
        assert_eq!(fix_vocab("the microservice and the rest of it", &v), "the microservice and the rest of it");
        assert_eq!(fix_vocab("I ran the tests again.", &v), "I ran the tests again.");
    }

    #[test]
    fn cleans_real_parakeet_output() {
        // Live VoiceStudio Parakeet v3 output for a TTS-read answer (2026-10-26).
        let raw = "we send Swift MT 300 and MT103 messages to counterparties. We are also migrating to \
                   ISO 2022 MX messages. They talk to each other over IBM, MQ and Kafka. One of the hardest \
                   problems was idem potency. The acknowledgments from Swift.";
        assert_eq!(
            clean_transcript(raw, &domain_vocab(&[])),
            "we send SWIFT MT300 and MT103 messages to counterparties. We are also migrating to \
             ISO 20022 MX messages. They talk to each other over IBM MQ and Kafka. One of the hardest \
             problems was idempotency. The acknowledgments from SWIFT."
        );
    }

    #[test]
    fn domain_vocab_dedupes_extras() {
        let v = domain_vocab(&["kafka", "  ", "Juspay"]);
        assert_eq!(v.len(), TECH_TERMS.len() + 1);
        assert_eq!(v.last().map(String::as_str), Some("Juspay"));
    }

    #[test]
    fn picks_best_installed_model() {
        let models = json!({"models":[
            {"id":"sherpa-parakeet-tdt-v3","installed":true},
            {"id":"sherpa-parakeet-tdt-v2","installed":false},
            {"id":"sherpa-whisper-tiny","installed":true}]});
        assert_eq!(pick_model(&models).as_deref(), Some("sherpa-parakeet-tdt-v3"));
        assert_eq!(pick_model(&json!({"models":[{"id":"sherpa-whisper-tiny","installed":true}]})), None);
        assert_eq!(pick_model(&json!({})), None);
    }

    #[test]
    fn english_model_install_target() {
        let models = json!({"models":[
            {"id":"sherpa-parakeet-tdt-v3","repo_id":"x/v3","installed":true},
            {"id":"sherpa-parakeet-tdt-v2","repo_id":"x/v2","installed":false}]});
        assert_eq!(english_repo_to_install(&models).as_deref(), Some("x/v2"));
        let ready = json!({"models":[{"id":"sherpa-parakeet-tdt-v2","repo_id":"x/v2","installed":true}]});
        assert_eq!(english_repo_to_install(&ready), None);
        assert_eq!(pick_model(&ready).as_deref(), Some(ENGLISH_MODEL));
        assert_eq!(english_repo_to_install(&json!({})), None);
    }

    #[test]
    fn segments_cut_only_after_enough_audio() {
        assert_eq!(seg_pause_ms(SEG_MIN - 1), None);
        assert_eq!(seg_pause_ms(SEG_MIN), Some(SEG_PAUSE_MS));
        assert_eq!(seg_pause_ms(SEG_LONG + 1), Some(SEG_PAUSE_LONG_MS));
    }

    #[test]
    fn answer_joins_segments_in_order() {
        let mut a = Answer::default();
        for _ in 0..2 {
            a.segs.push(Seg::new(None, Vec::new()));
        }
        a.on_msg(SegMsg::Msg(1, json!({"type":"partial","text":"second"}))).unwrap();
        a.on_msg(SegMsg::Msg(0, json!({"type":"final","final_kind":"summary","text":"First."}))).unwrap();
        assert_eq!(a.text(), "First. second");
        assert!(!a.all_done());
        a.on_msg(SegMsg::Closed(1)).unwrap();
        assert!(a.all_done());
        assert!(a.on_msg(SegMsg::Failed("x".into())).is_err());
        a.segs[1].fixed = Some("Second, from Whisper.".into());
        assert_eq!(a.text(), "First. Second, from Whisper.", "Whisper's text wins over Parakeet's");
    }
}
