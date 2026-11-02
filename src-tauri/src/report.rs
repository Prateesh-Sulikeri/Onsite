//! "Report a bug": builds a GitHub new-issue link with the form already filled in.
//! Onsite sends nothing itself. The user sees the redacted report, edits it, and submits on GitHub.
//! Design and decisions: `docs/plans/raise-a-bug.md`. Field ids, labels, dropdown options and title prefixes below
//! must equal `.github/ISSUE_TEMPLATE/*.yml`; a unit test reads the yml files and fails the build if they drift.

use crate::error::{AppError, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

pub const REPO_URL: &str = "https://github.com/Prateesh-Sulikeri/Onsite";
const REPO_PATH: &str = "/Prateesh-Sulikeri/Onsite";
/// Longest link (encoded) we open. Raised only after the Windows opener test passes at a larger size.
pub const MAX_URL_BYTES: usize = 4000;
const MAX_FIELD_CHARS: usize = 2000;
const TITLE_CHARS: usize = 80;
const ERROR_SHRINK: usize = 500;
const TEXT_SHRINK: usize = 800;
/// A line that shares this many characters in a row with private text is treated as private.
const RUN: usize = 40;
const LOG_LINES: usize = 200;
const LOG_BYTES: usize = 50 * 1024;
const PRIVATE_CAP_CHARS: usize = 400_000;

pub const OS_OPTIONS: &[&str] = &["Windows", "macOS", "Linux (AppImage)", "Linux (.deb)"];
pub const AI_OPTIONS: &[&str] = &["Claude Code", "Claude API", "OpenAI", "Gemini", "Groq"];
pub const VOICE_OPTIONS: &[&str] = &["Typed", "Spoken (built-in voice)", "Spoken (VoiceStudio)", "Not in a round"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Bug,
    Idea,
}

impl Category {
    pub fn template(self) -> &'static str {
        match self {
            Self::Bug => "bug_report.yml",
            Self::Idea => "feature_request.yml",
        }
    }
    /// Equals `title:` in the template. GitHub's `title` query value replaces it, so we add it ourselves.
    pub fn title_prefix(self) -> &'static str {
        match self {
            Self::Bug => "[Bug] ",
            Self::Idea => "[Idea] ",
        }
    }
    fn lead(self) -> &'static str {
        match self {
            Self::Bug => "what",
            Self::Idea => "problem",
        }
    }
    fn specs(self) -> &'static [Spec] {
        match self {
            Self::Bug => BUG,
            Self::Idea => IDEA,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Line,
    Drop(&'static [&'static str]),
}

struct Spec {
    id: &'static str,
    label: &'static str,
    kind: Kind,
    required: bool,
    /// Private lines are dropped (error text), instead of flagged.
    strict: bool,
    /// Cut to `TEXT_SHRINK` when the link is too long.
    shrink: bool,
}

const fn t(id: &'static str, label: &'static str, required: bool, shrink: bool) -> Spec {
    Spec { id, label, kind: Kind::Text, required, strict: false, shrink }
}

const BUG: &[Spec] = &[
    t("what", "What happened", true, false),
    t("steps", "Steps to reproduce", true, true),
    t("expected", "What you expected", true, true),
    t("actual", "What happened instead", true, true),
    Spec { id: "app_version", label: "Onsite version", kind: Kind::Line, required: true, strict: false, shrink: false },
    Spec { id: "os", label: "System", kind: Kind::Drop(OS_OPTIONS), required: true, strict: false, shrink: false },
    Spec { id: "ai", label: "AI provider", kind: Kind::Drop(AI_OPTIONS), required: true, strict: false, shrink: false },
    Spec { id: "voice", label: "How you ran the round", kind: Kind::Drop(VOICE_OPTIONS), required: true, strict: false, shrink: false },
    Spec { id: "view", label: "Screen you were on", kind: Kind::Line, required: false, strict: false, shrink: false },
    Spec { id: "error", label: "Error message", kind: Kind::Text, required: false, strict: true, shrink: false },
];

const IDEA: &[Spec] = &[
    t("problem", "What would help", true, true),
    t("idea", "Your idea", false, true),
    t("alternatives", "What you do today instead", false, true),
];

// ---------- redaction ----------

/// Hashes of every `RUN`-character window of the private texts (resume, current round, ...), whitespace collapsed.
#[derive(Default)]
pub struct PrivateIndex(HashSet<u64>);

fn norm(s: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    let mut space = true;
    for c in s.chars() {
        if c.is_whitespace() {
            if !space {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    if out.last() == Some(&' ') {
        out.pop();
    }
    out
}

fn window_hash(w: &[char]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    w.hash(&mut h);
    h.finish()
}

impl PrivateIndex {
    pub fn new(texts: &[String]) -> Self {
        let mut set = HashSet::new();
        let mut budget = PRIVATE_CAP_CHARS;
        for text in texts {
            let n = norm(text);
            if n.len() < RUN || budget == 0 {
                continue;
            }
            let n = &n[..n.len().min(budget)];
            budget = budget.saturating_sub(n.len());
            for w in n.windows(RUN) {
                set.insert(window_hash(w));
            }
        }
        Self(set)
    }

    pub fn shares_run(&self, line: &str) -> bool {
        if self.0.is_empty() {
            return false;
        }
        let n = norm(line);
        n.len() >= RUN && n.windows(RUN).any(|w| self.0.contains(&window_hash(w)))
    }
}

/// What redaction needs to know. Built from the app state; tests build their own.
#[derive(Default)]
pub struct Ctx {
    pub home: Vec<String>,
    pub keys: Vec<String>,
    pub private: PrivateIndex,
}

struct Patterns {
    key: Regex,
    home: Regex,
    win_path: Regex,
    unix_path: Regex,
    tilde_path: Regex,
    file: Regex,
}

fn patterns() -> &'static Patterns {
    static P: std::sync::OnceLock<Patterns> = std::sync::OnceLock::new();
    P.get_or_init(|| Patterns {
        key: Regex::new(r"\bgsk_[A-Za-z0-9]+|\bsk-[A-Za-z0-9_-]+|AIza[0-9A-Za-z_-]{20,}|(?i:bearer)\s+\S+").unwrap(),
        home: Regex::new(r"(?i)[A-Z]:[\\/]Users[\\/][^\\/\s]+|/home/[^/\s]+|/Users/[^/\s]+").unwrap(),
        win_path: Regex::new(r#"(?i)\b[A-Z]:[\\/][^\s"'<>|*?]*"#).unwrap(),
        unix_path: Regex::new(r#"/(?:home|Users|tmp|var|opt|usr|mnt|etc|root|private|run|srv|Volumes)/[^\s"'<>|]*"#).unwrap(),
        tilde_path: Regex::new(r#"~[\\/][^\s"'<>|]*"#).unwrap(),
        file: Regex::new(r#"(?i)[^\s\\/"'<>|]+\.(?:pdf|docx?|rtf|txt)\b"#).unwrap(),
    })
}

fn last_segment(path: &str) -> &str {
    path.rsplit(['\\', '/']).find(|s| !s.is_empty()).unwrap_or("")
}

fn is_user_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".pdf", ".docx", ".doc", ".rtf", ".txt"].iter().any(|e| lower.ends_with(e))
}

/// Cuts trailing punctuation off a regex match so "see C:\a\b.txt." keeps its full stop.
fn split_tail(m: &str) -> (&str, &str) {
    let body = m.trim_end_matches(['.', ',', ';', ':', ')', ']', '}']);
    (body, &m[body.len()..])
}

/// Keys, home folder and absolute paths. Applied to every field. `files`: also hide the names of files the user
/// chose (resume.pdf) as `[file]`, used for error and log text.
pub fn redact(text: &str, ctx: &Ctx, files: bool) -> String {
    let p = patterns();
    let mut s = text.to_string();
    for k in ctx.keys.iter().filter(|k| k.trim().len() >= 8) {
        s = s.replace(k.trim(), "[key removed]");
    }
    s = p.key.replace_all(&s, "[key removed]").into_owned();
    for h in ctx.home.iter().filter(|h| h.len() > 3) {
        for variant in [h.clone(), h.replace('\\', "/"), h.replace('/', "\\")] {
            s = replace_ci(&s, &variant, "~");
        }
    }
    s = p.home.replace_all(&s, "~").into_owned();
    // Other absolute paths shrink to the file name; a path to a user's own document hides even that.
    let shrink = |c: &regex::Captures| {
        let (body, tail) = split_tail(&c[0]);
        let name = last_segment(body);
        let name = if files && is_user_file(name) { "[file]" } else { name };
        format!("{name}{tail}")
    };
    s = p.win_path.replace_all(&s, shrink).into_owned();
    s = p.unix_path.replace_all(&s, shrink).into_owned();
    if files {
        s = p.tilde_path.replace_all(&s, shrink).into_owned();
        s = p.file.replace_all(&s, "[file]").into_owned();
    }
    s
}

fn replace_ci(haystack: &str, needle: &str, with: &str) -> String {
    if needle.is_empty() {
        return haystack.to_string();
    }
    match Regex::new(&format!("(?i){}", regex::escape(needle))) {
        Ok(re) => re.replace_all(haystack, with).into_owned(),
        Err(_) => haystack.to_string(),
    }
}

/// Error and log text: a line that shares a long run with the resume, JD or transcript is dropped.
pub fn drop_private_lines(text: &str, idx: &PrivateIndex) -> String {
    text.lines()
        .map(|l| if idx.shares_run(l) { "[line removed: looks like private text]" } else { l })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Typed text: lines that look like private text. Never removed, only shown to the user.
pub fn flag_lines(text: &str, idx: &PrivateIndex) -> Vec<String> {
    text.lines().filter(|l| idx.shares_run(l)).map(|l| l.trim().chars().take(80).collect()).collect()
}

fn cap(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let mut out: String = s.chars().take(n).collect();
    out.push_str("...");
    out
}

// ---------- the report ----------

#[derive(Debug, Clone, Deserialize)]
pub struct Field {
    pub id: String,
    pub value: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReportInput {
    pub category: Category,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Flag {
    pub id: String,
    pub line: String,
}

/// Checked, redacted and ordered like the template.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub category: Category,
    pub title: Option<String>,
    pub fields: Vec<(&'static str, String)>,
    pub flags: Vec<Flag>,
}

pub fn prepare(input: &ReportInput, ctx: &Ctx) -> Result<Prepared> {
    let specs = input.category.specs();
    for f in &input.fields {
        if !specs.iter().any(|s| s.id == f.id) {
            return Err(AppError::Invalid("Unknown report field.".into()));
        }
    }
    let mut fields = Vec::new();
    let mut flags = Vec::new();
    for s in specs {
        let raw = input.fields.iter().find(|f| f.id == s.id).map(|f| f.value.trim()).unwrap_or("");
        let value = match s.kind {
            Kind::Drop(options) => options.iter().find(|o| **o == raw).map(|o| o.to_string()).unwrap_or_default(),
            Kind::Line => {
                let one = raw.lines().next().unwrap_or("");
                cap(redact(one, ctx, s.strict).trim(), 200)
            }
            Kind::Text => {
                let cleaned = redact(raw, ctx, s.strict);
                let cleaned = if s.strict { drop_private_lines(&cleaned, &ctx.private) } else { cleaned };
                if !s.strict {
                    flags.extend(flag_lines(&cleaned, &ctx.private).into_iter().map(|line| Flag { id: s.id.to_string(), line }));
                }
                cap(cleaned.trim(), MAX_FIELD_CHARS)
            }
        };
        if !value.is_empty() {
            fields.push((s.id, value));
        }
    }
    let title = fields
        .iter()
        .find(|(id, _)| *id == input.category.lead())
        .map(|(_, v)| v.lines().next().unwrap_or("").trim().chars().take(TITLE_CHARS).collect::<String>())
        .filter(|t| !t.is_empty())
        .map(|t| format!("{}{}", input.category.title_prefix(), t));
    Ok(Prepared { category: input.category, title, fields, flags })
}

/// Names the first required field that is empty, as the user sees it.
pub fn missing_required(p: &Prepared) -> Option<&'static str> {
    p.category.specs().iter().find(|s| s.required && !p.fields.iter().any(|(id, _)| *id == s.id)).map(|s| s.label)
}

/// Plain text of the whole report: what the user sees in the preview and what "Copy report" copies.
pub fn report_text(p: &Prepared) -> String {
    let mut out = p.title.clone().unwrap_or_else(|| p.category.title_prefix().trim().to_string());
    for s in p.category.specs() {
        if let Some((_, v)) = p.fields.iter().find(|(id, _)| *id == s.id) {
            out.push_str(&format!("\n\n### {}\n\n{}", s.label, v));
        }
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Everything is in the link.
    Full,
    /// Error text and long answers were cut so the link fits.
    Shortened,
    /// Too long even then: the blank form opens and the user pastes the copied report.
    Blank,
}

#[derive(Debug, Clone)]
pub struct Built {
    pub url: String,
    pub level: Level,
}

fn url_for(p: &Prepared, fields: &[(&str, String)], with_title: bool) -> Result<reqwest::Url> {
    let mut u = reqwest::Url::parse(&format!("{REPO_URL}/issues/new")).map_err(|e| AppError::Invalid(e.to_string()))?;
    {
        let mut q = u.query_pairs_mut();
        q.append_pair("template", p.category.template());
        if with_title {
            if let Some(t) = &p.title {
                q.append_pair("title", t);
            }
        }
        for (k, v) in fields {
            q.append_pair(k, v);
        }
    }
    assert_safe(&u)?;
    Ok(u)
}

/// The scheme, host and path never come from the UI; check them once more before anything opens.
pub fn assert_safe(u: &reqwest::Url) -> Result<()> {
    let ok = u.scheme() == "https"
        && u.host_str() == Some("github.com")
        && u.port().is_none()
        && u.username().is_empty()
        && u.path() == format!("{REPO_PATH}/issues/new");
    if ok {
        Ok(())
    } else {
        Err(AppError::Invalid("That link is not allowed.".into()))
    }
}

pub fn build_url(p: &Prepared) -> Result<Built> {
    let full = url_for(p, &p.fields, true)?;
    if full.as_str().len() <= MAX_URL_BYTES {
        return Ok(Built { url: full.into(), level: Level::Full });
    }
    let specs = p.category.specs();
    let mut fields = p.fields.clone();
    for (_, v) in fields.iter_mut().filter(|(id, _)| *id == "error") {
        *v = cap(v, ERROR_SHRINK);
    }
    let short = url_for(p, &fields, true)?;
    if short.as_str().len() <= MAX_URL_BYTES {
        return Ok(Built { url: short.into(), level: Level::Shortened });
    }
    for (id, v) in fields.iter_mut() {
        if specs.iter().any(|s| s.id == *id && s.shrink) {
            *v = cap(v, TEXT_SHRINK);
        }
    }
    let shorter = url_for(p, &fields, true)?;
    if shorter.as_str().len() <= MAX_URL_BYTES {
        return Ok(Built { url: shorter.into(), level: Level::Shortened });
    }
    let blank = url_for(p, &[], false)?;
    Ok(Built { url: blank.into(), level: Level::Blank })
}

// ---------- log ----------

/// The last lines of the log file, redacted, ready to paste. Never goes in a link.
pub fn log_tail(raw: &[u8], ctx: &Ctx) -> String {
    let start = raw.len().saturating_sub(LOG_BYTES);
    let text = String::from_utf8_lossy(&raw[start..]);
    let mut lines: Vec<&str> = text.lines().collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0); // the first line is probably cut in half
    }
    let from = lines.len().saturating_sub(LOG_LINES);
    let kept: Vec<String> = lines[from..].iter().map(|l| redact(l, ctx, true)).collect();
    drop_private_lines(&kept.join("\n"), &ctx.private)
}

// ---------- context ----------

#[derive(Debug, Serialize)]
pub struct ReportContext {
    pub version: String,
    pub os: &'static str,
    pub ai: &'static str,
    pub voice: &'static str,
}

pub fn os_label() -> &'static str {
    if cfg!(windows) {
        "Windows"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if std::env::var_os("APPIMAGE").is_some() {
        "Linux (AppImage)"
    } else {
        "Linux (.deb)"
    }
}

fn home_dirs() -> Vec<String> {
    ["USERPROFILE", "HOME"].iter().filter_map(|k| std::env::var(k).ok()).filter(|v| !v.is_empty()).collect()
}

pub async fn context(state: &crate::AppState, version: String) -> ReportContext {
    let s = state.settings();
    let in_round = !state.rounds.lock().await.is_empty();
    let voice = if !in_round {
        "Not in a round"
    } else if !s.voice.enabled {
        "Typed"
    } else if s.voice.engine == crate::voice::engine::VoiceEngine::Voicestudio {
        "Spoken (VoiceStudio)"
    } else {
        "Spoken (built-in voice)"
    };
    ReportContext { version, os: os_label(), ai: s.provider.label(), voice }
}

/// Keys, the home folder and the private texts the app holds right now (current round, last resume check).
pub async fn redaction_ctx(state: &crate::AppState) -> Ctx {
    let mut texts: Vec<String> = Vec::new();
    for r in state.rounds.lock().await.values() {
        texts.extend(r.private_texts());
    }
    let last: Option<serde_json::Value> = state.store.lock().unwrap().get_setting(crate::ats::LAST_KEY).ok().flatten();
    if let Some(preview) = last.as_ref().and_then(|l| l["preview"].as_str()) {
        texts.push(preview.to_string());
    }
    Ctx { home: home_dirs(), keys: state.api_keys().into_values().collect(), private: PrivateIndex::new(&texts) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        Ctx { home: vec![r"C:\Users\jane".into()], keys: vec!["my-secret-key-12345".into()], private: PrivateIndex::default() }
    }

    fn input(cat: Category, f: &[(&str, &str)]) -> ReportInput {
        ReportInput { category: cat, fields: f.iter().map(|(a, b)| Field { id: a.to_string(), value: b.to_string() }).collect() }
    }

    fn bug(extra: &[(&str, &str)]) -> ReportInput {
        let mut f = vec![("what", "Voice stops"), ("steps", "1. start"), ("expected", "talk"), ("actual", "silence"), ("app_version", "0.1.4"), ("os", "Windows"), ("ai", "Groq"), ("voice", "Typed")];
        f.retain(|(id, _)| !extra.iter().any(|(e, _)| e == id));
        f.extend_from_slice(extra);
        input(Category::Bug, &f)
    }

    #[test]
    fn redacts_every_key_shape() {
        let c = ctx();
        for k in ["gsk_AbC123xyz", "sk-ant-api03-AbC_def-123", "AIzaSyA1234567890abcdefghijk", "Bearer abc.def.ghi", "my-secret-key-12345"] {
            let out = redact(&format!("request failed with {k} in header"), &c, false);
            assert!(!out.contains(k.trim_start_matches("Bearer ")), "{k} leaked: {out}");
            assert!(out.contains("[key removed]"));
        }
    }

    #[test]
    fn words_ending_in_sk_are_kept() {
        let c = ctx();
        let line = "task-queue stalled, disk-full on risk-check";
        assert_eq!(redact(line, &c, false), line);
        assert!(redact("key=sk-abc123", &c, false).contains("[key removed]"));
    }

    #[test]
    fn home_folder_becomes_tilde() {
        let c = ctx();
        assert_eq!(redact(r"open C:\Users\jane\AppData\x failed", &c, false), r"open ~\AppData\x failed");
        let c2 = Ctx::default();
        assert!(redact("at /home/bob/.config/onsite", &c2, false).contains('~'));
        assert!(redact("at /Users/amy/Library/Logs", &c2, false).contains('~'));
        assert!(!redact(r"C:\Users\Someone\Desktop", &c2, false).contains("Someone"));
        assert!(!redact(r"c:/users/someone/x", &c2, false).contains("someone"));
    }

    #[test]
    fn other_paths_shrink_to_the_file_name() {
        let c = Ctx::default();
        assert_eq!(redact(r"cannot read D:\Work\secret\notes.log now", &c, false), "cannot read notes.log now");
        assert_eq!(redact("cannot read /var/data/db/onsite.sqlite.", &c, false), "cannot read onsite.sqlite.");
        assert!(redact("GET https://api.groq.com/openai/v1/models", &c, false).contains("https://api.groq.com/openai/v1/models"));
    }

    #[test]
    fn user_document_names_are_hidden_in_error_text() {
        let c = Ctx::default();
        let out = redact(r"could not open D:\cv\Jane Doe Resume.pdf and my_resume.docx", &c, true);
        assert!(!out.contains("Resume.pdf") && !out.contains("my_resume"), "{out}");
        assert!(out.contains("[file]"));
        assert!(redact("see resume.pdf", &c, false).contains("resume.pdf"));
    }

    #[test]
    fn private_lines_are_dropped_in_logs_but_only_flagged_in_typed_fields() {
        let resume = "Led migration of the payments ledger from MySQL to PostgreSQL, cutting p99 latency by 40 percent across three regions.";
        let c = Ctx { private: PrivateIndex::new(&[resume.to_string()]), ..Default::default() };
        let log = format!("INFO started\nWARN reply failed: {resume}\nINFO done");
        let out = drop_private_lines(&log, &c.private);
        assert!(!out.contains("PostgreSQL") && out.contains("INFO started") && out.contains("INFO done"));
        let r = prepare(&bug(&[("steps", resume), ("error", &format!("boom\n{resume}"))]), &c).unwrap();
        let steps = &r.fields.iter().find(|(id, _)| *id == "steps").unwrap().1;
        assert!(steps.contains("PostgreSQL"), "typed text must stay");
        assert!(r.flags.iter().any(|f| f.id == "steps"));
        let err = &r.fields.iter().find(|(id, _)| *id == "error").unwrap().1;
        assert!(!err.contains("PostgreSQL") && err.contains("boom"));
        // the same run with different spacing and a short unrelated line
        assert!(c.private.shares_run("  Led migration of the payments   ledger from MySQL to PostgreSQL, cutting p99 "));
        assert!(!c.private.shares_run("Led migration of the payments"));
    }

    #[test]
    fn allow_list_rejects_unknown_ids_and_dropdown_values() {
        let c = ctx();
        assert!(prepare(&bug(&[("labels", "bug")]), &c).is_err());
        assert!(prepare(&bug(&[("log", "x")]), &c).is_err(), "the log never goes through the link");
        assert!(prepare(&input(Category::Idea, &[("steps", "x")]), &c).is_err());
        let p = prepare(&bug(&[("os", "TempleOS")]), &c).unwrap();
        assert!(!p.fields.iter().any(|(id, _)| *id == "os"));
        assert_eq!(missing_required(&p), Some("System"));
    }

    #[test]
    fn host_and_path_are_checked() {
        let p = prepare(&bug(&[]), &ctx()).unwrap();
        let b = build_url(&p).unwrap();
        let u = reqwest::Url::parse(&b.url).unwrap();
        assert_eq!(u.host_str(), Some("github.com"));
        assert_eq!(u.path(), "/Prateesh-Sulikeri/Onsite/issues/new");
        assert!(assert_safe(&reqwest::Url::parse("https://evil.example/Prateesh-Sulikeri/Onsite/issues/new").unwrap()).is_err());
        assert!(assert_safe(&reqwest::Url::parse("http://github.com/Prateesh-Sulikeri/Onsite/issues/new").unwrap()).is_err());
        assert!(assert_safe(&reqwest::Url::parse("https://github.com/other/repo/issues/new").unwrap()).is_err());
        assert!(assert_safe(&reqwest::Url::parse("https://github.com@evil.example/Prateesh-Sulikeri/Onsite/issues/new").unwrap()).is_err());
    }

    #[test]
    fn values_are_percent_encoded() {
        let nasty = "a&b=c #hash +plus\nline2 \u{e9}\u{4e2d} 100%";
        let p = prepare(&bug(&[("what", nasty)]), &ctx()).unwrap();
        let b = build_url(&p).unwrap();
        assert!(!b.url.contains(' ') && !b.url.contains('\n') && !b.url.contains('#'));
        let u = reqwest::Url::parse(&b.url).unwrap();
        let what = u.query_pairs().find(|(k, _)| k == "what").unwrap().1.into_owned();
        assert_eq!(what, nasty);
        assert_eq!(u.query_pairs().filter(|(k, _)| k == "template").count(), 1);
        let title = u.query_pairs().find(|(k, _)| k == "title").unwrap().1.into_owned();
        assert!(title.starts_with("[Bug] a&b=c #hash"));
        assert!(!title.contains('\n'));
    }

    #[test]
    fn long_reports_shrink_in_order_then_fall_back_to_the_blank_form() {
        let long = "word ".repeat(400); // 2000 chars
        let c = ctx();
        // error alone pushes it over: cut the error first, steps stay whole
        let p = prepare(&bug(&[("error", &"&".repeat(1500)), ("steps", &long[..300])]), &c).unwrap();
        let b = build_url(&p).unwrap();
        assert_eq!(b.level, Level::Shortened);
        assert!(b.url.len() <= MAX_URL_BYTES);
        let u = reqwest::Url::parse(&b.url).unwrap();
        assert_eq!(u.query_pairs().find(|(k, _)| k == "error").unwrap().1.chars().count(), ERROR_SHRINK + 3);
        assert_eq!(u.query_pairs().find(|(k, _)| k == "steps").unwrap().1.chars().count(), 299, "steps untouched at this step");
        // text fields in that size need the second cut
        let p = prepare(&bug(&[("steps", &long), ("expected", &long), ("actual", &long)]), &c).unwrap();
        let b = build_url(&p).unwrap();
        assert_eq!(b.level, Level::Shortened);
        let u = reqwest::Url::parse(&b.url).unwrap();
        assert!(u.query_pairs().find(|(k, _)| k == "steps").unwrap().1.chars().count() <= TEXT_SHRINK + 3);
        // symbols that encode to 3 bytes each cannot fit even when cut
        let heavy = "&".repeat(2000);
        let p = prepare(&bug(&[("steps", &heavy), ("expected", &heavy), ("actual", &heavy), ("what", &heavy)]), &c).unwrap();
        let b = build_url(&p).unwrap();
        assert_eq!(b.level, Level::Blank);
        assert_eq!(b.url, format!("{REPO_URL}/issues/new?template=bug_report.yml"));
        assert!(report_text(&p).contains("### Steps to reproduce"), "the full report is still available to copy");
    }

    #[test]
    fn title_uses_the_prefix_and_first_line() {
        let p = prepare(&bug(&[]), &ctx()).unwrap();
        assert_eq!(p.title.as_deref(), Some("[Bug] Voice stops"));
        let idea = prepare(&input(Category::Idea, &[("problem", &format!("{}\nsecond", "x".repeat(200)))]), &ctx()).unwrap();
        assert_eq!(idea.title.unwrap().chars().count(), "[Idea] ".len() + TITLE_CHARS);
        let blank = prepare(&input(Category::Idea, &[]), &ctx()).unwrap();
        assert!(blank.title.is_none());
        assert_eq!(missing_required(&blank), Some("What would help"));
    }

    #[test]
    fn log_tail_is_limited_and_redacted() {
        let mut raw = String::new();
        for i in 0..500 {
            raw.push_str(&format!("INFO line {i} at C:\\Users\\jane\\x key gsk_ABCDEF{i}\n"));
        }
        let out = log_tail(raw.as_bytes(), &ctx());
        assert_eq!(out.lines().count(), LOG_LINES);
        assert!(out.contains("line 499") && !out.contains("line 299"));
        assert!(!out.contains("gsk_ABCDEF") && !out.contains("jane"));
        let big = "x".repeat(100) + "\n";
        assert!(log_tail(big.repeat(2000).as_bytes(), &ctx()).len() <= LOG_BYTES);
    }

    #[test]
    fn no_em_or_en_dashes_in_the_text_we_produce() {
        let p = prepare(&bug(&[]), &ctx()).unwrap();
        let all = format!("{}{:?}{:?}{:?}", report_text(&p), OS_OPTIONS, AI_OPTIONS, VOICE_OPTIONS);
        assert!(!all.contains('\u{2014}') && !all.contains('\u{2013}'));
        assert!(!include_str!("report.rs").contains(['\u{2014}', '\u{2013}']));
    }

    // ----- the code and the yml templates must agree -----

    /// Line endings differ by checkout (CRLF on Windows), so normalise.
    fn template(c: Category) -> String {
        match c {
            Category::Bug => include_str!("../../.github/ISSUE_TEMPLATE/bug_report.yml"),
            Category::Idea => include_str!("../../.github/ISSUE_TEMPLATE/feature_request.yml"),
        }
        .replace("\r\n", "\n")
    }

    /// The text of the form element with this `id:`, up to the next element.
    fn block<'a>(yml: &'a str, id: &str) -> &'a str {
        let marker = format!("    id: {id}\n");
        let start = yml.find(&marker).unwrap_or_else(|| panic!("template has no id {id}"));
        let rest = &yml[start..];
        &rest[..rest.find("\n  - type:").unwrap_or(rest.len())]
    }

    #[test]
    fn templates_match_the_code() {
        for cat in [Category::Bug, Category::Idea] {
            let yml = template(cat);
            let yml = yml.as_str();
            assert!(yml.contains(&format!("title: \"{}\"", cat.title_prefix())), "{:?} title prefix", cat);
            for s in cat.specs() {
                let b = block(yml, s.id);
                assert!(b.contains(&format!("label: {}\n", s.label)), "{}: label {}", s.id, s.label);
                assert_eq!(b.contains("required: true"), s.required, "{}: required", s.id);
                if let Kind::Drop(options) = s.kind {
                    // GitHub does not prefill dropdowns from a link, so these are text inputs listing the allowed values.
                    assert!(yml.contains(&format!("  - type: input\n    id: {}\n", s.id)), "{}: must be an input", s.id);
                    let line = b.lines().find(|l| l.contains("One of: [")).expect("One of: line");
                    let listed: Vec<&str> = line.split_once("One of: [").unwrap().1.trim_end_matches("]\"").split(", ").collect();
                    assert_eq!(listed, options, "{}: allowed values", s.id);
                }
            }
        }
    }

    #[test]
    fn config_has_the_discussions_link_and_no_question_template() {
        let cfg = include_str!("../../.github/ISSUE_TEMPLATE/config.yml");
        assert!(cfg.contains("url: https://github.com/Prateesh-Sulikeri/Onsite/discussions"));
        assert!(!std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.github/ISSUE_TEMPLATE/question.yml")).exists());
    }

    #[test]
    fn provider_labels_are_the_dropdown_options() {
        use crate::brain::api::Provider::*;
        let labels: Vec<&str> = [ClaudeCode, Anthropic, Openai, Gemini, Groq].iter().map(|p| p.label()).collect();
        assert_eq!(labels, AI_OPTIONS);
    }
}
