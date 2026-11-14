//! One spoken answer: mic -> VAD -> Parakeet (in this process) -> final text.
//!
//! Long answers are cut into segments at short pauses, so each finished segment decodes while the
//! candidate keeps talking and the tail left to decode at the end stays short. The open segment is
//! re-decoded every [`SttOptions::caption_every`] for the live caption. Domain words are fixed against a
//! vocabulary and repetition loops are collapsed.

use super::audio::{f32_to_i16, i16_to_f32, rms, Agc, Level, Mic, SpeechGate, Vad, VadEvent, MIC_RATE};
use super::builtin::ParakeetStt;
use super::engine::Engine;
use super::VoiceEvent;
use crate::error::{AppError, Result};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::Notify;

const FRAME: usize = (MIC_RATE / 50) as usize; // 20 ms
const PREROLL: usize = (MIC_RATE as usize * 400) / 1000;
/// A segment needs this much audio before a short pause may cut it.
const SEG_MIN: usize = MIC_RATE as usize * 3;
/// Past this length, cut at a shorter pause so one segment never grows huge.
const SEG_LONG: usize = MIC_RATE as usize * 20;
const SEG_PAUSE_MS: u32 = 700;
const SEG_PAUSE_LONG_MS: u32 = 300;
/// Wait this long for the last segment's final text after the answer ends.
const FINAL_WAIT: Duration = Duration::from_secs(15);

/// Tech and payments words the recogniser often gets wrong. Matched case- and space-insensitively.
pub const TECH_TERMS: &[&str] = &[
    "Java", "JVM", "Kotlin", "Spring Boot", "Hibernate", "JPA", "Kafka", "Kafka Streams", "ZooKeeper",
    "RabbitMQ", "ActiveMQ", "IBM MQ", "PostgreSQL", "Postgres", "MySQL", "Redis", "MongoDB", "Cassandra",
    "Elasticsearch", "DynamoDB", "Kubernetes", "Docker", "gRPC", "GraphQL", "JSON", "OAuth", "JWT",
    "microservice", "microservices", "idempotency", "idempotent", "CQRS", "sharding", "Prometheus",
    "Grafana", "BFSI", "SWIFT", "MT103", "MT202", "MT300", "MT940", "ISO 20022", "pacs.008", "pacs.009",
    "camt.053", "UPI", "NEFT", "RTGS", "IMPS", "PCI DSS", "Razorpay",
];

/// Words to restore in a transcript: the static tech list, and this round's names and terms
/// (resume, JD, dossier), which match a little more loosely.
#[derive(Debug, Clone, Default)]
pub struct Vocab {
    pub base: Vec<String>,
    pub round: Vec<String>,
}

/// Vocabulary for one answer: the static tech list plus caller context (company, JD, round terms).
pub fn domain_vocab(extra: &[&str]) -> Vocab {
    let base: Vec<String> = TECH_TERMS.iter().map(|s| s.to_string()).collect();
    let mut round: Vec<String> = Vec::new();
    for e in extra.iter().map(|e| e.trim()).filter(|e| !e.is_empty()) {
        if !base.iter().chain(&round).any(|t| t.eq_ignore_ascii_case(e)) {
            round.push(e.to_string());
        }
    }
    Vocab { base, round }
}

/// Known mishearings (`prompts/stt_aliases.json`): what the recognizer wrote -> the term.
#[derive(Debug, Clone, Default)]
pub struct Aliases(Vec<Alias>);

#[derive(Debug, Clone, serde::Deserialize)]
struct Alias {
    heard: Vec<String>,
    term: String,
    /// Words that must appear somewhere in the answer, for mishearings that are also plain English.
    #[serde(default)]
    needs: Vec<String>,
}

impl Aliases {
    pub fn from_json(text: &str) -> std::result::Result<Self, String> {
        #[derive(serde::Deserialize)]
        struct File {
            aliases: Vec<Alias>,
        }
        let f: File = serde_json::from_str(text).map_err(|e| e.to_string())?;
        // Stored as normalised words, so matching is a plain comparison.
        let aliases = f
            .aliases
            .into_iter()
            .map(|a| Alias {
                heard: a.heard.iter().map(|h| h.split_whitespace().map(norm).collect::<Vec<_>>().join(" ")).collect(),
                needs: a.needs.iter().map(|n| norm(n)).collect(),
                term: a.term,
            })
            .collect();
        Ok(Self(aliases))
    }

    /// Empty (with a warning) when the file is missing or broken: repair still works without it.
    pub fn load(prompts_dir: &std::path::Path) -> Self {
        let path = prompts_dir.join("stt_aliases.json");
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| Self::from_json(&t)) {
            Ok(a) => a,
            Err(e) => {
                tracing::warn!(path = %path.display(), "stt aliases not loaded: {e}");
                Self::default()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct SttOptions {
    /// Words to restore in the transcript (see [`fix_vocab`]).
    pub vocab: Vocab,
    /// Known mishearings (see [`Aliases`]).
    pub aliases: Arc<Aliases>,
    /// How often the open segment is re-decoded for the live caption. None = no live caption.
    pub caption_every: Option<Duration>,
    /// Silero VAD model. None, or a model that won't load = the energy VAD and no gain control.
    pub vad_model: Option<std::path::PathBuf>,
}

impl Default for SttOptions {
    fn default() -> Self {
        Self { vocab: domain_vocab(&[]), aliases: Default::default(), caption_every: Some(Duration::from_millis(500)), vad_model: None }
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

/// What we have heard so far in one segment: captions (`partial`) and the decoded text (`final`).
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
                Err(AppError::Voice(format!("Speech recognition: {detail}")))
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
/// Recogniser loops ("for the MQ for the MQ for the MQ") come out as one phrase.
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

/// This round's names and terms get one more edit than the static list.
fn allowed_edits_round(len: usize) -> usize {
    match len {
        0..=4 => 0,
        5..=7 => 1,
        _ => 3,
    }
}

/// Common English words. A window made only of these is never turned into a term by spelling or
/// sound ("just pay" stays, even when it was "Juspay"): rewriting ordinary speech is worse than
/// missing a name, and the interviewer prompt lists the terms anyway.
const EVERYDAY: &[&str] = &[
    "the", "be", "to", "of", "and", "a", "in", "that", "have", "i", "it", "for", "not", "on", "with", "he", "as", "you",
    "do", "at", "this", "but", "his", "by", "from", "they", "we", "say", "her", "she", "or", "an", "will", "my", "one",
    "all", "would", "there", "their", "what", "so", "up", "out", "if", "about", "who", "get", "which", "go", "me",
    "when", "make", "can", "like", "time", "no", "just", "him", "know", "take", "people", "into", "year", "your",
    "good", "some", "could", "them", "see", "other", "than", "then", "now", "look", "only", "come", "its", "over",
    "think", "also", "back", "after", "use", "two", "how", "our", "work", "first", "well", "way", "even", "new",
    "want", "because", "any", "these", "give", "day", "most", "us", "is", "was", "are", "were", "been", "has", "had",
    "did", "does", "said", "made", "went", "got", "here", "where", "why", "very", "much", "more", "many", "such",
    "own", "same", "each", "few", "both", "those", "every", "still", "through", "down", "off", "before", "between",
    "under", "again", "never", "always", "often", "while", "should", "may", "might", "must", "need", "let", "put",
    "set", "run", "ran", "pay", "read", "reads", "write", "wrote", "post", "press", "rate", "rates", "trade", "cost",
    "speed", "data", "team", "part", "next", "last", "long", "great", "little", "right", "big", "high", "small",
    "large", "old", "young", "late", "early", "important", "public", "bad", "able", "thing", "things", "man", "world",
    "life", "hand", "week", "case", "point", "number", "group", "problem", "fact", "end", "start", "call", "try",
    "ask", "feel", "become", "leave", "keep", "begin", "seem", "help", "talk", "turn", "show", "hear", "play", "move",
    "live", "believe", "hold", "bring", "happen", "provide", "sit", "stand", "lose", "add", "change", "lead",
    "understand", "watch", "follow", "stop", "create", "speak", "allow", "spend", "grow", "open", "walk", "win",
    "offer", "remember", "love", "consider", "appear", "buy", "wait", "serve", "send", "expect", "build", "stay",
    "fall", "cut", "reach", "remain", "attention", "service", "hour", "result", "results", "red", "yeah", "okay",
    "um", "uh", "really", "actually", "basically", "something", "anything", "nothing", "everything", "someone",
    "system", "level", "side", "kind", "sort", "lot", "too", "yes", "since", "without", "around", "against", "during",
    "though", "until", "already", "maybe", "sure", "story", "money", "account", "order", "place", "table", "store",
    "line", "key", "value", "list", "file", "code", "test", "tests", "rest", "sky", "dusk", "im", "dont", "thats",
];

/// A rough sound-alike key: similar spellings of the same sounds ("cubernetties", "Kubernetes") share it.
pub fn sound_key(word: &str) -> String {
    let s: Vec<char> = word.chars().filter(char::is_ascii_alphabetic).map(|c| c.to_ascii_lowercase()).collect();
    let is_vowel = |c: char| matches!(c, 'a' | 'e' | 'i' | 'o' | 'u');
    let mut out = String::new();
    let mut i = 0;
    while i < s.len() {
        let (c, next) = (s[i], s.get(i + 1).copied());
        let pair = match (c, next) {
            ('p', Some('h')) => Some("f"),
            ('c', Some('k')) | ('c', Some('h')) => Some("k"),
            ('s', Some('h')) => Some("s"),
            _ => None,
        };
        if let Some(p) = pair {
            out.push_str(p);
            i += 2;
            continue;
        }
        match c {
            'c' if matches!(next, Some('e' | 'i' | 'y')) => out.push('s'),
            'c' | 'q' | 'g' => out.push('k'),
            'x' => out.push_str("ks"),
            'z' => out.push('s'),
            'd' => out.push('t'),
            'b' => out.push('p'),
            'v' => out.push('f'),
            'h' | 'w' | 'y' => {}
            v if is_vowel(v) => {
                if i == 0 {
                    out.push(v);
                }
            }
            other => out.push(other),
        }
        i += 1;
    }
    let mut key = String::new();
    for c in out.chars() {
        if !key.ends_with(c) {
            key.push(c);
        }
    }
    key
}

/// One vocabulary term, prepared for matching.
struct Term<'a> {
    chars: Vec<char>,
    key: String,
    canon: &'a str,
    round: bool,
}

/// (rule, edits, window len, term). Lower rule wins, then fewer edits, then the longer window.
type Pick<'a> = Option<(u8, usize, usize, &'a str)>;

fn offer<'a>(best: &mut Pick<'a>, rule: u8, d: usize, len: usize, canon: &'a str) {
    if best.is_none_or(|(br, bd, bl, _)| (rule, d) < (br, bd) || ((rule, d) == (br, bd) && len > bl)) {
        *best = Some((rule, d, len, canon));
    }
}

/// Restores vocabulary words split, misspelt, mis-cased or misheard by the recogniser, e.g.
/// "IDEM potency" -> "idempotency", "PostgresUL" -> "PostgreSQL", "IBM, MQ" -> "IBM MQ",
/// "reddish" -> "Redis" (in a caching answer). Per window of 1-3 words, in order of preference:
/// exact (ignoring case, spaces, punctuation) or a known mishearing; a few spelling edits with the
/// first and last letters kept; the same sound key (long terms, or this round's names).
pub fn fix_vocab(text: &str, vocab: &Vocab, aliases: &Aliases) -> String {
    let terms: Vec<Term> = vocab
        .base
        .iter()
        .map(|t| (t, false))
        .chain(vocab.round.iter().map(|t| (t, true)))
        .map(|(t, round)| Term { chars: norm(t).chars().collect(), key: sound_key(t), canon: t.as_str(), round })
        .filter(|t| !t.chars.is_empty())
        .collect();
    let words: Vec<&str> = text.split_whitespace().collect();
    let normed: Vec<String> = words.iter().map(|w| norm(w)).collect();
    let in_text = |w: &str| normed.iter().any(|n| n == w);
    let mut out: Vec<String> = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let mut best: Pick = None;
        for len in 1..=3.min(words.len() - i) {
            let win = &normed[i..i + len];
            if win.iter().any(String::is_empty) {
                break;
            }
            let joined: Vec<char> = win.concat().chars().collect();
            let spaced = win.join(" ");
            let everyday = win.iter().all(|w| EVERYDAY.contains(&w.as_str()));
            for a in &aliases.0 {
                let context = a.needs.is_empty() || a.needs.iter().any(|n| in_text(n)) || vocab.round.iter().any(|r| r.eq_ignore_ascii_case(&a.term));
                if context && a.heard.contains(&spaced) {
                    offer(&mut best, 0, 0, len, a.term.as_str());
                }
            }
            for t in &terms {
                if joined == t.chars {
                    offer(&mut best, 0, 0, len, t.canon);
                    continue;
                }
                if everyday {
                    continue;
                }
                let max = if t.round { allowed_edits_round(t.chars.len()) } else { allowed_edits(t.chars.len()) };
                let ends_match = joined.first() == t.chars.first() && joined.last() == t.chars.last();
                if max > 0 && ends_match && joined.len().abs_diff(t.chars.len()) <= max {
                    let d = levenshtein(&joined, &t.chars);
                    if d <= max {
                        offer(&mut best, 1, d, len, t.canon);
                        continue;
                    }
                }
                let long_enough = t.chars.len() >= 8 || (t.round && t.chars.len() >= 5);
                if long_enough && t.key.len() >= 3 && sound_key(&spaced) == t.key {
                    offer(&mut best, 2, 0, len, t.canon);
                }
            }
        }
        match best {
            Some((_, _, len, canon)) => {
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
pub fn clean_transcript(text: &str, vocab: &Vocab, aliases: &Aliases) -> String {
    collapse_repeats(&fix_vocab(text, vocab, aliases))
}

// ---------- listening ----------

enum SegMsg {
    Msg(usize, Value),
    Closed(usize),
}

struct Seg {
    st: SttState,
    done: bool,
    /// This segment's audio so far.
    pcm: Vec<i16>,
}

impl Seg {
    fn new(pcm: Vec<i16>) -> Self {
        Self { st: SttState::default(), done: false, pcm }
    }
}

/// Every segment of one answer, in order.
#[derive(Default)]
struct Answer {
    segs: Vec<Seg>,
}

impl Answer {
    /// Best text so far: each segment's final text when it has one, else its caption.
    fn text(&self) -> String {
        self.segs.iter().map(|s| s.st.result()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")
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
                }
                Ok(changed)
            }
            SegMsg::Closed(i) => {
                self.segs[i].done = true;
                Ok(false)
            }
        }
    }
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
    listen_with(engine, pause_ms, ctl, &SttOptions { vad_model: Some(engine.vad_model.clone()), aliases: engine.aliases.clone(), ..Default::default() }, emit).await
}

/// Decodes a finished segment off the async runtime and reports it as the segment's final text.
fn spawn_decode(stt: Arc<ParakeetStt>, idx: usize, pcm: Vec<i16>, tx: UnboundedSender<SegMsg>) {
    tokio::task::spawn_blocking(move || {
        let text = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stt.transcribe(&pcm)));
        let _ = tx.send(match text {
            Ok(text) => SegMsg::Msg(idx, serde_json::json!({"type": "final", "final_kind": "summary", "text": text})),
            Err(_) => SegMsg::Closed(idx),
        });
    });
}

/// When the next caption may start. A caption decode is followed by a rest as long as the decode took,
/// so captions use at most half the CPU. On slow laptops back-to-back decodes starved the mic thread
/// ("buffer underrun or overrun") and audio was lost.
#[derive(Default)]
struct CaptionGate {
    busy: AtomicBool,
    next: Mutex<Option<Instant>>,
}

impl CaptionGate {
    fn ready(&self) -> bool {
        !self.busy.load(Ordering::SeqCst) && self.next.lock().unwrap().is_none_or(|t| Instant::now() >= t)
    }
}

/// Decodes the open segment for a live caption. Never more than one at a time; skipped while a
/// finished segment is being decoded.
fn spawn_caption(stt: Arc<ParakeetStt>, idx: usize, pcm: Vec<i16>, gate: Arc<CaptionGate>, tx: UnboundedSender<SegMsg>) {
    gate.busy.store(true, Ordering::SeqCst);
    tokio::task::spawn_blocking(move || {
        let t = Instant::now();
        let text = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stt.caption(&pcm))).ok().flatten();
        *gate.next.lock().unwrap() = Some(Instant::now() + t.elapsed());
        gate.busy.store(false, Ordering::SeqCst);
        if let Some(text) = text.filter(|t| !t.is_empty()) {
            let _ = tx.send(SegMsg::Msg(idx, serde_json::json!({"type": "partial", "text": text})));
        }
    });
}

/// [`listen`] with a vocabulary of the caller's choice.
pub async fn listen_with(engine: &Engine, pause_ms: u32, ctl: &ListenCtl, opts: &SttOptions, mut emit: impl FnMut(VoiceEvent)) -> Result<ListenEnd> {
    run(&engine.stt, pause_ms, ctl, opts, &mut emit).await
}

async fn run(stt: &Arc<ParakeetStt>, pause_ms: u32, ctl: &ListenCtl, opts: &SttOptions, emit: &mut impl FnMut(VoiceEvent)) -> Result<ListenEnd> {
    let (tx, mut rx) = unbounded_channel::<Vec<i16>>();
    let (seg_tx, mut seg_rx) = unbounded_channel::<SegMsg>();
    let mic = Mic::open(tx)?;
    let mut vad = Vad::new(20, pause_ms);
    // Silero decides speech when its model loads; gain control runs only with it, because the
    // energy VAD's thresholds are tuned to the raw mic level.
    let mut gate = opts.vad_model.as_deref().and_then(SpeechGate::open);
    if opts.vad_model.is_some() && gate.is_none() {
        tracing::warn!("silero vad did not load; using the energy vad");
    }
    let mut agc = Agc::new();
    let mut gate_voiced = false;
    let mut frame: Vec<i16> = Vec::with_capacity(FRAME);
    let mut preroll: VecDeque<i16> = VecDeque::with_capacity(PREROLL + FRAME);
    let mut ans = Answer::default();
    let mut active: Option<usize> = None;
    let mut seg_samples = 0usize;
    let mut last_level = Instant::now();
    let mut shown = String::new();
    let heard = |ans: &Answer| clean_transcript(&ans.text(), &opts.vocab, &opts.aliases);
    let mut health = tokio::time::interval(Duration::from_millis(500));
    let mut mic_lost = false;
    // Without a live caption the timer still ticks (hourly) so the select stays simple; it never decodes.
    let mut caption = tokio::time::interval(opts.caption_every.unwrap_or(Duration::from_secs(3600)));
    caption.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let caption_gate = Arc::new(CaptionGate::default());
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
            _ = caption.tick() => {
                if let (Some(i), Some(_)) = (active, opts.caption_every) {
                    let n = ans.segs[i].pcm.len();
                    if n >= MIC_RATE as usize / 2 && captioned != (i, n) && caption_gate.ready() {
                        captioned = (i, n);
                        spawn_caption(stt.clone(), i, ans.segs[i].pcm.clone(), caption_gate.clone(), seg_tx.clone());
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
                let Some(mut chunk) = chunk else { break };
                if let Some(g) = gate.as_mut() {
                    // Gain follows the previous chunk's verdict; the gate then judges the scaled chunk.
                    let mut f = i16_to_f32(&chunk);
                    agc.process(&mut f, gate_voiced);
                    gate_voiced = g.voiced(&f);
                    chunk = f32_to_i16(&f);
                }
                ctl.mic_level.set(mic.level.get());
                if last_level.elapsed() >= Duration::from_millis(80) {
                    last_level = Instant::now();
                    emit(VoiceEvent::Level { mic: mic.level.get(), out: 0.0 });
                }
                let (mut ended, mut started, mut loud) = (false, false, false);
                for &s in &chunk {
                    frame.push(s);
                    if frame.len() == FRAME {
                        let event = if gate.is_some() {
                            loud |= gate_voiced;
                            vad.push_flag(gate_voiced, gate_voiced)
                        } else {
                            let level = rms(&i16_to_f32(&frame));
                            loud |= level > vad.threshold();
                            vad.push(level)
                        };
                        match event {
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
                    active = Some(ans.segs.len());
                    seg_samples = head.len();
                    ans.segs.push(Seg::new(head));
                }
                match active {
                    Some(i) => {
                        ans.segs[i].pcm.extend_from_slice(&chunk);
                        seg_samples += chunk.len();
                        // Short pause inside a long answer: finish this segment now, so its
                        // decode runs while the candidate keeps talking.
                        let cut = seg_pause_ms(seg_samples).is_some_and(|p| vad.quiet_ms() >= p);
                        if !ended && vad.is_speaking() && cut {
                            spawn_decode(stt.clone(), i, ans.segs[i].pcm.clone(), seg_tx.clone());
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
        spawn_decode(stt.clone(), i, ans.segs[i].pcm.clone(), seg_tx.clone());
    }
    let deadline = tokio::time::sleep(FINAL_WAIT);
    tokio::pin!(deadline);
    while !ans.all_done() {
        tokio::select! {
            _ = &mut deadline => { tracing::warn!("STT final text timed out; using what we have"); break; }
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
    let raw = ans.text();
    let text = clean_transcript(&raw, &opts.vocab, &opts.aliases);
    let stt_ms = ended_at.elapsed().as_millis() as u64;
    tracing::info!(stt_ms, segments = ans.segs.len(), words = text.split_whitespace().count(), "utterance transcribed");
    if raw != text {
        // Lengths only: the words themselves are the candidate's speech and never go in the log.
        tracing::debug!(raw_chars = raw.chars().count(), chars = text.chars().count(), "transcript cleaned");
    }
    Ok(if text.is_empty() { ListenEnd::NoSpeech } else { ListenEnd::Text { text, stt_ms } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn options_default_to_half_second_captions() {
        assert_eq!(SttOptions::default().caption_every, Some(Duration::from_millis(500)));
    }

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
            assert_eq!(fix_vocab(input, &v, &Aliases::default()), want, "{input}");
        }
        // Must not eat neighbouring words or touch ordinary English.
        assert_eq!(fix_vocab("a PostgreSQL table", &v, &Aliases::default()), "a PostgreSQL table");
        assert_eq!(fix_vocab("the microservice and the rest of it", &v, &Aliases::default()), "the microservice and the rest of it");
        assert_eq!(fix_vocab("I ran the tests again.", &v, &Aliases::default()), "I ran the tests again.");
    }

    #[test]
    fn cleans_real_parakeet_output() {
        // Real Parakeet output for a TTS-read answer (2026-10-26).
        let raw = "we send Swift MT 300 and MT103 messages to counterparties. We are also migrating to \
                   ISO 2022 MX messages. They talk to each other over IBM, MQ and Kafka. One of the hardest \
                   problems was idem potency. The acknowledgments from Swift.";
        assert_eq!(
            clean_transcript(raw, &domain_vocab(&[]), &Aliases::default()),
            "we send SWIFT MT300 and MT103 messages to counterparties. We are also migrating to \
             ISO 20022 MX messages. They talk to each other over IBM MQ and Kafka. One of the hardest \
             problems was idempotency. The acknowledgments from SWIFT."
        );
    }

    #[test]
    fn domain_vocab_dedupes_extras() {
        let v = domain_vocab(&["kafka", "  ", "Juspay"]);
        assert_eq!(v.base.len(), TECH_TERMS.len());
        assert_eq!(v.round, ["Juspay"]);
    }

    fn aliases() -> Aliases {
        Aliases::from_json(include_str!("../../../prompts/stt_aliases.json")).unwrap()
    }

    #[test]
    fn sound_keys() {
        assert_eq!(sound_key("Kubernetes"), sound_key("cubernetties"));
        assert_eq!(sound_key("Elasticsearch"), sound_key("elastic surch"));
        assert_ne!(sound_key("Cassandra"), sound_key("Kubernetes"));
    }

    #[test]
    fn aliases_fix_short_terms_only_in_context() {
        let v = domain_vocab(&[]);
        let a = aliases();
        assert_eq!(fix_vocab("we cache it in reddish with a ttl", &v, &a), "we cache it in Redis with a ttl");
        assert_eq!(fix_vocab("the sky was reddish at dusk", &v, &a), "the sky was reddish at dusk");
        assert_eq!(fix_vocab("stored in postgres creole tables", &v, &a), "stored in PostgreSQL tables");
    }

    #[test]
    fn round_terms_match_more_loosely() {
        let v = domain_vocab(&["Juspay", "Prateesh", "Hazelcast"]);
        let a = Aliases::default();
        assert_eq!(fix_vocab("I worked at juspe for two years", &v, &a), "I worked at Juspay for two years");
        assert_eq!(fix_vocab("my name is pratish", &v, &a), "my name is Prateesh");
        assert_eq!(fix_vocab("a hazel cast cluster", &v, &a), "a Hazelcast cluster");
    }

    #[test]
    fn ordinary_english_is_left_alone() {
        let v = domain_vocab(&["Juspay", "Prateesh", "Karthik", "Razorpay"]);
        let a = aliases();
        for s in [
            "I just pay attention to the reads and the rates",
            "we read this data from the other team and wrote it back",
            "there is a trade off between speed and the cost of the read",
            "I would post the results and press on with the next part",
            "the service was down for an hour and we were late",
        ] {
            assert_eq!(fix_vocab(s, &v, &a), s);
        }
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
            a.segs.push(Seg::new(Vec::new()));
        }
        a.on_msg(SegMsg::Msg(1, json!({"type":"partial","text":"second"}))).unwrap();
        a.on_msg(SegMsg::Msg(0, json!({"type":"final","final_kind":"summary","text":"First."}))).unwrap();
        assert_eq!(a.text(), "First. second");
        assert!(!a.all_done());
        a.on_msg(SegMsg::Closed(1)).unwrap();
        assert!(a.all_done());
    }
}
