//! Loop setup, interviewer prompt, and candidate message composition. All pure.

pub mod live;
pub mod report;

use crate::brain::prompts::{render, Vars};
use crate::catalog::Catalog;
use crate::error::{AppError, Result};
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
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PlannedRound {
    pub key: String,
    pub name: String,
    pub persona: String,
    pub minutes: u32,
    pub tab: String,
}

impl LoopSetup {
    pub fn validate(&self, cat: &Catalog) -> Result<()> {
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

    pub fn company_name(&self, cat: &Catalog) -> String {
        match self.company.as_str() {
            "auto" => "Inferred from JD".into(),
            k => cat.companies[k].name.clone(),
        }
    }

    pub fn plan(&self, cat: &Catalog, idx: usize) -> PlannedRound {
        let k = &self.rounds[idx];
        let r = &cat.rounds[k];
        PlannedRound {
            key: k.clone(),
            name: r.name.clone(),
            persona: cat.persona(idx).to_string(),
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

pub fn interviewer_prompt(template: &str, cat: &Catalog, setup: &LoopSetup, idx: usize) -> String {
    let plan = setup.plan(cat, idx);
    let round = &cat.rounds[&plan.key];
    let company = &cat.companies[&setup.company];
    let company_line = if setup.company == "auto" { String::new() } else { format!(" at {}", company.name) };
    let vars: Vars = [
        ("persona", plan.persona.clone()),
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
    render(template, &vars)
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

    fn fixtures() -> (Catalog, String) {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts");
        (Catalog::load(&dir).unwrap(), crate::brain::prompts::load(&dir, "interviewer.md").unwrap())
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
        }
    }

    #[test]
    fn renders_full_prompt_without_placeholders() {
        let (cat, tpl) = fixtures();
        let s = setup();
        s.validate(&cat).unwrap();
        let p = interviewer_prompt(&tpl, &cat, &s, 1);
        assert!(!p.contains("{{"), "unrendered placeholder in:\n{p}");
        assert!(p.contains("a senior software engineer at Google, conducting"));
        assert!(p.contains("ROUND: DSA coding (10 minutes). Round 2 of 2"));
        assert!(p.contains("COMPANY STYLE: Google: structured"));
        assert!(p.contains("(not provided"));
        assert!(p.contains("Backend, payments"));
    }

    #[test]
    fn auto_company_has_no_at_line_and_bad_setup_fails() {
        let (cat, tpl) = fixtures();
        let mut s = setup();
        s.company = "auto".into();
        assert!(interviewer_prompt(&tpl, &cat, &s, 0).contains("a senior software engineer, conducting"));
        s.rounds.push("nope".into());
        assert!(s.validate(&cat).is_err());
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

    #[test]
    fn clip_respects_char_boundaries() {
        assert_eq!(clip("héllo", 2), "h");
    }
}
