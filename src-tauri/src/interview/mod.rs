//! Loop setup, interviewer prompt, and candidate message composition. All pure.

pub mod live;
pub mod report;

use crate::brain::prompts::{render, Vars};
use crate::catalog::Catalog;
use crate::error::{AppError, Result};
use crate::panels::{Panels, DEFAULT_FAMILY};
use serde::{Deserialize, Serialize};

const RESUME_MAX: usize = 12_000;
const JD_MAX: usize = 10_000;
const EDITOR_MAX: usize = 14_000;
const WHITEBOARD_MAX: usize = 8_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoopSetup {
    /// Catalog round keys in loop order.
    pub rounds: Vec<String>,
    pub company: String,
    pub level: String,
    pub temperament: String,
    pub lang: String,
    #[serde(default)]
    pub resume: String,
    #[serde(default)]
    pub jd: String,
    #[serde(default)]
    pub notes: String,
    /// Overrides every round's length (e.g. a 10-minute practice round).
    #[serde(default)]
    pub minutes: Option<u32>,
    /// Interviewer panel id from panels.json; "" = one-off personas from the catalog.
    #[serde(default)]
    pub panel: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PlannedRound {
    pub key: String,
    pub name: String,
    pub persona: String,
    /// Panel interviewer id and role, when the loop uses a panel.
    pub persona_id: Option<String>,
    pub persona_role: Option<String>,
    pub minutes: u32,
    pub tab: String,
}

impl LoopSetup {
    pub fn validate(&self, cat: &Catalog, panels: &Panels) -> Result<()> {
        if !self.panel.is_empty() && !panels.contains(&self.panel) {
            return Err(AppError::Invalid("Unknown interviewer panel.".into()));
        }
        if self.rounds.is_empty() {
            return Err(AppError::Invalid("Pick at least one round.".into()));
        }
        for k in &self.rounds {
            if !cat.rounds.contains_key(k) {
                return Err(AppError::Invalid(format!("Unknown round: {k}")));
            }
        }
        let checks = [
            (cat.companies.contains_key(&self.company), "company style"),
            (cat.levels.contains_key(&self.level), "level"),
            (cat.temper.contains_key(&self.temperament), "temperament"),
        ];
        match checks.iter().find(|(ok, _)| !ok) {
            Some((_, what)) => Err(AppError::Invalid(format!("Unknown {what}."))),
            None => Ok(()),
        }
    }

    /// What the candidate sees: "General" for a loop with no company style.
    pub fn company_name(&self, cat: &Catalog) -> String {
        match self.company.as_str() {
            "auto" => "General".into(),
            k => cat.companies.get(k).map_or_else(|| k.to_string(), |c| c.name.clone()),
        }
    }

    /// The company as the report and verdict prompts name it.
    pub fn company_for_prompt(&self, cat: &Catalog) -> String {
        match self.company.as_str() {
            "auto" => "the company in the job description (a typical product company if there is none)".into(),
            _ => self.company_name(cat),
        }
    }

    pub fn plan(&self, cat: &Catalog, panels: &Panels, idx: usize) -> PlannedRound {
        let k = &self.rounds[idx];
        let r = &cat.rounds[k];
        let p = panels.persona(&self.panel, k);
        PlannedRound {
            key: k.clone(),
            name: r.name.clone(),
            persona: p.as_ref().map_or_else(|| cat.persona(idx).to_string(), |p| p.first_name.clone()),
            persona_id: p.as_ref().map(|p| p.id.clone()),
            persona_role: p.map(|p| p.role),
            minutes: self.minutes.unwrap_or(r.min),
            tab: r.tab.clone(),
        }
    }
}

fn clip(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn or_default(s: &str, max: usize, fallback: &str) -> String {
    let t = s.trim();
    if t.is_empty() { fallback.to_string() } else { clip(t, max).to_string() }
}

/// Templates the interviewer prompt is built from.
pub struct PromptTemplates {
    pub interviewer: String,
    pub persona_block: String,
}

pub fn interviewer_prompt(t: &PromptTemplates, cat: &Catalog, panels: &Panels, setup: &LoopSetup, idx: usize) -> String {
    let plan = setup.plan(cat, panels, idx);
    // A panel persona is a fixed person; otherwise a generic engineer with a rotating name.
    let (persona, persona_role, persona_block) = match panels.persona_vars(&setup.panel, &plan.key, DEFAULT_FAMILY) {
        Some(vars) => (
            vars["name"].clone(),
            vars["role"].clone(),
            format!("\nYOUR PERSONA (the same person every session; the questions change every session):\n{}\n", render(&t.persona_block, &vars)),
        ),
        None => (plan.persona.clone(), "a senior software engineer".to_string(), String::new()),
    };
    let round = &cat.rounds[&plan.key];
    let company = &cat.companies[&setup.company];
    let company_line = if setup.company == "auto" { String::new() } else { format!(" at {}", company.name) };
    let vars: Vars = [
        ("persona", persona),
        ("persona_role", persona_role),
        ("persona_block", persona_block),
        ("company_line", company_line),
        ("round_name", plan.name.clone()),
        ("round_minutes", plan.minutes.to_string()),
        ("round_index", (idx + 1).to_string()),
        ("round_count", setup.rounds.len().to_string()),
        ("round_guide", round.guide.clone()),
        ("company_style", company.style.clone()),
        ("level_name", cat.levels[&setup.level].clone()),
        ("level_bar", cat.level_bar[&setup.level].clone()),
        ("temperament", cat.temper[&setup.temperament].clone()),
        ("lang", setup.lang.clone()),
        ("dossier_block", String::new()),
        ("resume", or_default(&setup.resume, RESUME_MAX, "(not provided — ask them to walk you through their background)")),
        ("jd", or_default(&setup.jd, JD_MAX, "(none — use a typical role at this level)")),
        ("candidate_notes", or_default(&setup.notes, 4_000, "(none)")),
    ]
    .into();
    render(&t.interviewer, &vars)
}

/// How the last few sessions of this round opened: the first two interviewer turns, with
/// shared code cut to its first lines. Shown to the interviewer so it doesn't repeat itself.
pub fn opening_excerpt(turns: &[crate::store::Turn]) -> Option<String> {
    let mut parts = Vec::new();
    for t in turns.iter().filter(|t| t.who == "iv").take(2) {
        let mut out = String::new();
        let mut in_code = 0usize;
        for line in t.text.lines() {
            if line.trim_start().starts_with("```") {
                in_code = if in_code > 0 { 0 } else { 1 };
                if in_code == 0 {
                    out.push_str(" …code… ");
                }
                continue;
            }
            if in_code > 0 {
                in_code += 1;
                if in_code > 5 {
                    continue;
                }
            }
            out.push_str(line.trim());
            out.push(' ');
        }
        let out = out.replace("[[END]]", "");
        let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
        if !out.is_empty() {
            parts.push(out);
        }
    }
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join(" / ");
    let clipped = clip(&joined, 600);
    Some(if clipped.len() < joined.len() { format!("{clipped}…") } else { joined })
}

/// One opener, avoiding the ones used recently while others are left. `seed` picks among the rest.
pub fn pick_opener<'a>(openers: &'a [String], recent: &[String], seed: u64) -> Option<&'a String> {
    let fresh: Vec<&String> = openers.iter().filter(|o| !recent.contains(o)).collect();
    let pool: Vec<&String> = if fresh.is_empty() { openers.iter().collect() } else { fresh };
    (!pool.is_empty()).then(|| pool[(seed % pool.len() as u64) as usize])
}

/// Added to the interviewer's system prompt: today's starting point and what recent sessions asked.
pub fn session_block(opener: Option<&str>, earlier: &[String]) -> String {
    let mut s = String::from("\n\nTHIS SESSION (private notes; never mention them):\n");
    if let Some(o) = opener {
        s.push_str(&format!("- Starting point for today: {o}. Build the round around it. It overrides the examples in HOW TO RUN IT.\n"));
    }
    if !earlier.is_empty() {
        s.push_str("- Earlier sessions of this round opened like this. Do not reuse their questions, scenarios, code or wording:\n");
        for (i, e) in earlier.iter().enumerate() {
            s.push_str(&format!("  {}. {e}\n", i + 1));
        }
    }
    s.push_str("- Word your greeting and first question freshly. A real interviewer never sounds recited.");
    s
}

pub fn opening_message(minutes: u32) -> String {
    format!("[The candidate has joined the call.] [Elapsed: 0 of {minutes} min] Begin the interview now with your greeting and first question.")
}

/// What the candidate has in the shared workspace right now.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Workspace {
    #[serde(default)]
    pub code: String,
    /// Whiteboard text; the UI sends "" while it still equals the starter template.
    #[serde(default)]
    pub notes: String,
}

/// Builds the user turn. Only parts that changed since `last` are attached, unless `force`.
pub fn candidate_message(
    text: &str,
    ws: &Workspace,
    last: &mut Workspace,
    force: bool,
    lang: &str,
    elapsed_min: f64,
    minutes: u32,
) -> Option<(String, bool)> {
    let mut parts = Vec::new();
    if !ws.code.trim().is_empty() && (force || ws.code != last.code) {
        parts.push(format!("Editor ({lang}):\n```\n{}\n```", clip(&ws.code, EDITOR_MAX)));
    }
    if !ws.notes.trim().is_empty() && (force || ws.notes != last.notes) {
        parts.push(format!("Whiteboard:\n{}", clip(&ws.notes, WHITEBOARD_MAX)));
    }
    *last = ws.clone();
    let text = text.trim();
    if text.is_empty() && parts.is_empty() {
        return None;
    }
    let said = if text.is_empty() { "(The candidate shares their workspace without saying anything.)" } else { text };
    let shared = !parts.is_empty();
    let block = if shared { format!("\n\n[Workspace]\n{}", parts.join("\n\n")) } else { String::new() };
    Some((format!("{said}{block}\n[Elapsed: {} of {minutes} min]", elapsed_min.round() as i64), shared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn fixtures() -> (Catalog, PromptTemplates, Panels) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts");
        let load = |n| crate::brain::prompts::load(&dir, n).unwrap();
        let t = PromptTemplates { interviewer: load("interviewer.md"), persona_block: load("persona_block.md") };
        (Catalog::load(&dir).unwrap(), t, Panels::load(&dir))
    }

    fn setup() -> LoopSetup {
        LoopSetup {
            rounds: vec!["resume".into(), "coding".into()],
            company: "google".into(),
            level: "sde2".into(),
            temperament: "realistic".into(),
            lang: "Java".into(),
            resume: String::new(),
            jd: "Backend, payments".into(),
            notes: String::new(),
            minutes: Some(10),
            panel: String::new(),
        }
    }

    #[test]
    fn renders_full_prompt_without_placeholders() {
        let (cat, tpl, panels) = fixtures();
        let s = setup();
        s.validate(&cat, &panels).unwrap();
        let p = interviewer_prompt(&tpl, &cat, &panels, &s, 1);
        assert!(!p.contains("{{"), "unrendered placeholder in:\n{p}");
        assert!(p.contains("a senior software engineer at Google, conducting"));
        assert!(p.contains("ROUND: DSA coding (10 minutes). Round 2 of 2"));
        assert!(p.contains("COMPANY STYLE: Google: structured"));
        assert!(p.contains("(not provided"));
        assert!(p.contains("Backend, payments"));
    }

    #[test]
    fn auto_company_has_no_at_line_and_bad_setup_fails() {
        let (cat, tpl, panels) = fixtures();
        let mut s = setup();
        s.company = "auto".into();
        assert!(interviewer_prompt(&tpl, &cat, &panels, &s, 0).contains("a senior software engineer, conducting"));
        s.rounds.push("nope".into());
        assert!(s.validate(&cat, &panels).is_err());
    }

    #[test]
    fn panel_round_uses_its_owner_persona() {
        let (cat, tpl, panels) = fixtures();
        let mut s = setup();
        s.panel = "indian_techies".into();
        s.validate(&cat, &panels).unwrap();
        let plan = s.plan(&cat, &panels, 1);
        assert_eq!((plan.persona.as_str(), plan.persona_id.as_deref()), ("Karthik", Some("karthik_reddy")));
        let p = interviewer_prompt(&tpl, &cat, &panels, &s, 1);
        assert!(!p.contains("{{"), "unrendered placeholder in:\n{p}");
        assert!(p.contains("You are Karthik Reddy, Senior Technical Interviewer at Google"));
        assert!(p.contains("YOUR PERSONA"));
        // Same inputs, same prompt: the persona is stable across sessions.
        assert_eq!(p, interviewer_prompt(&tpl, &cat, &panels, &s, 1));
        s.panel = "nope".into();
        assert!(s.validate(&cat, &panels).is_err());
    }

    #[test]
    fn workspace_attached_only_when_changed_or_forced() {
        let mut last = Workspace::default();
        let ws = Workspace { code: "int f() {}".into(), notes: String::new() };
        let (m, shared) = candidate_message("here", &ws, &mut last, false, "Java", 3.4, 45).unwrap();
        assert!(shared);
        assert_eq!(m, "here\n\n[Workspace]\nEditor (Java):\n```\nint f() {}\n```\n[Elapsed: 3 of 45 min]");
        let (m, shared) = candidate_message("again", &ws, &mut last, false, "Java", 4.0, 45).unwrap();
        assert!(!shared);
        assert_eq!(m, "again\n[Elapsed: 4 of 45 min]");
        assert!(candidate_message("", &ws, &mut last, false, "Java", 4.0, 45).is_none());
        let (m, _) = candidate_message("", &ws, &mut last, true, "Java", 4.0, 45).unwrap();
        assert!(m.starts_with("(The candidate shares their workspace"));
    }

    fn turn(who: &str, text: &str) -> crate::store::Turn {
        crate::store::Turn { who: who.into(), text: text.into(), at_min: 0.0, ttft_ms: None, shared_workspace: false, sent: None }
    }

    #[test]
    fn opening_excerpt_keeps_first_iv_turns_and_cuts_code() {
        let code = (1..=20).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n");
        let turns = vec![
            turn("iv", "Hi, I'm Vikram.\nTell me about yourself."),
            turn("me", "I build payments."),
            turn("iv", &format!("Here is some code.\n```java\n{code}\n```\nWhat does it do?")),
            turn("iv", "Third turn is ignored."),
        ];
        let e = opening_excerpt(&turns).unwrap();
        assert!(e.starts_with("Hi, I'm Vikram. Tell me about yourself. / Here is some code. line1"), "{e}");
        assert!(e.contains("line4 …code… What does it do?") && !e.contains("line5") && !e.contains("Third"), "{e}");
        assert!(opening_excerpt(&[turn("me", "hello")]).is_none());
    }

    #[test]
    fn pick_opener_skips_recent_until_all_used() {
        let all: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        for seed in 0..10 {
            assert_eq!(pick_opener(&all, &["a".into(), "b".into()], seed).unwrap(), "c");
        }
        assert!(pick_opener(&all, &all, 7).is_some());
        assert!(pick_opener(&[], &[], 1).is_none());
    }

    #[test]
    fn session_block_lists_opener_and_earlier_openings() {
        let b = session_block(Some("a rate limiter"), &["Hi, here is a PaymentService".into()]);
        assert!(b.contains("Starting point for today: a rate limiter."));
        assert!(b.contains("1. Hi, here is a PaymentService"));
        assert!(!session_block(None, &[]).contains("Earlier sessions"));
    }

    #[test]
    fn clip_respects_char_boundaries() {
        assert_eq!(clip("héllo", 2), "h");
    }
}
