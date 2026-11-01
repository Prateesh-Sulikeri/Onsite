//! Writes a code base for the comprehension round with the AI, from the job description, the company
//! dossier and the resume. A one-shot job with no tools. The result is checked by
//! `Project::from_generated`; a rejected answer gets one retry with the reason. It is never run.

use crate::brain::prompts::{render, Vars};
use crate::brain::{run_once, Tools};
use crate::dossier::Dossier;
use crate::error::{AppError, Result};
use crate::projects::{lang_exts, Project};
use crate::AppState;
use std::time::Duration;

const JD_MAX: usize = 6_000;
const RESUME_MAX: usize = 3_000;
const ATTEMPT_LIMIT: Duration = Duration::from_secs(180);
/// How many recent titles are kept, so the next project is a different scenario.
pub const RECENT: usize = 6;
pub const RECENT_KEY: &str = "projgen:recent";

/// Everything the prompt is built from.
pub struct GenInput<'a> {
    pub lang: &'a str,
    pub level_name: &'a str,
    pub level_bar: &'a str,
    pub minutes: u32,
    pub company: &'a str,
    /// The loop's dossier and the index of this round in it, for a company-specific loop.
    pub dossier: Option<(&'a Dossier, usize)>,
    pub jd: &'a str,
    pub resume: &'a str,
    pub scenario_hint: &'a str,
    pub avoid: &'a [String],
}

/// Cuts at a char boundary.
fn clip(s: &str, max: usize) -> &str {
    let s = s.trim();
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn company_context(d: Option<(&Dossier, usize)>) -> String {
    let Some((d, idx)) = d else {
        return "(a general practice interview, no particular company)".into();
    };
    let mut s = format!("{}: {}\nHow they interview: {}\n", d.company, d.summary.trim(), d.style.trim());
    if !d.values.trim().is_empty() {
        s.push_str(&format!("Values: {}\n", d.values.trim()));
    }
    if let Some(r) = d.process.get(idx) {
        s.push_str(&format!("This round ({}): {}\n", r.name, r.what.trim()));
        if !r.questions.is_empty() {
            s.push_str(&format!("Questions candidates reported: {}\n", r.questions.join(" | ")));
        }
    }
    if !d.pitfalls.is_empty() {
        s.push_str(&format!("Common pitfalls: {}\n", d.pitfalls.join("; ")));
    }
    s
}

pub fn prompt(template: &str, i: &GenInput) -> String {
    let or = |s: &str, none: &str| if s.trim().is_empty() { none.to_string() } else { clip(s, usize::MAX).to_string() };
    let vars: Vars = [
        ("lang", i.lang.to_string()),
        ("lang_ext", lang_exts(i.lang).first().copied().unwrap_or("txt").to_string()),
        ("level_name", i.level_name.to_string()),
        ("level_bar", i.level_bar.to_string()),
        ("minutes", i.minutes.to_string()),
        ("company", i.company.to_string()),
        ("company_context", company_context(i.dossier)),
        ("jd", if i.jd.trim().is_empty() { "(none given)".into() } else { clip(i.jd, JD_MAX).to_string() }),
        ("resume", if i.resume.trim().is_empty() { "(none given)".into() } else { clip(i.resume, RESUME_MAX).to_string() }),
        ("scenario_hint", or(i.scenario_hint, "(none)")),
        ("avoid", if i.avoid.is_empty() { "(none)".into() } else { i.avoid.join("; ") }),
    ]
    .into();
    render(template, &vars)
}

/// Asks the model for a project and checks it. One retry with the reason when the first answer is rejected.
pub async fn generate(state: &AppState, template: &str, input: &GenInput<'_>) -> Result<Project> {
    let backend = state.backend().await?;
    let model = state.settings().report_model;
    let base = prompt(template, input);
    let mut ask = base.clone();
    let mut last = String::new();
    for attempt in 1..=2 {
        let started = std::time::Instant::now();
        let out = tokio::time::timeout(ATTEMPT_LIMIT, run_once(&backend, &ask, Tools::None, &model, |_| {}))
            .await
            .map_err(|_| AppError::Ai("Writing the project took too long.".into()))??;
        let checked = match &out.json {
            Some(v) => Project::from_generated(v, input.lang, input.minutes),
            None => Err("the reply was not a JSON object".into()),
        };
        match checked {
            Ok(p) => {
                tracing::info!(id = %p.id, files = p.files.len(), issues = p.issues.len(), attempt, ms = started.elapsed().as_millis() as u64, "project written");
                return Ok(p);
            }
            Err(why) => {
                tracing::warn!(attempt, "written project rejected: {why}");
                ask = format!("{base}\n\nYour last answer was rejected: {why}. Fix that and reply with the whole JSON object again.");
                last = why;
            }
        }
    }
    Err(AppError::Ai(format!("The project didn't pass the checks ({last}).")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_fills_every_placeholder() {
        let avoid = vec!["Order totals".to_string()];
        let i = GenInput {
            lang: "Java",
            level_name: "SDE-2",
            level_bar: "Owns a service.",
            minutes: 45,
            company: "Razorpay",
            dossier: None,
            jd: "Backend engineer, payments, Kafka",
            resume: "",
            scenario_hint: "a rate limiter",
            avoid: &avoid,
        };
        let t = crate::brain::prompts::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts"), "project.md").unwrap();
        let p = prompt(&t, &i);
        assert!(!p.contains("{{"), "{p}");
        assert!(p.contains("Backend engineer, payments, Kafka") && p.contains("src/Name.java") && p.contains("Order totals") && p.contains("(none given)"));
    }
}
