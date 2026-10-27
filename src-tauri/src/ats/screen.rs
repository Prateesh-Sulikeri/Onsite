//! The AI screen: what HiredScore, Role Fit or Eightfold add on top of parsing and keywords.
//! Claude grades the resume text exactly as the parser extracted it, so a scrambled layout hurts
//! here too. Cached per (resume text, posting), so the grade doesn't wobble between re-checks.

use crate::brain::{prompts, run_once, Tools};
use crate::error::{AppError, Result};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bump when the prompt or the shape changes.
const CACHE_VERSION: &str = "1";
const MAX_INPUT_CHARS: usize = 12_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MustHave {
    pub skill: String,
    /// "proven", "claimed" or "missing".
    pub verdict: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AiScreen {
    /// A to D, like HiredScore.
    pub grade: String,
    pub fit: u32,
    pub advance: bool,
    /// "below", "at" or "above" the posting's level.
    pub level: String,
    pub level_note: String,
    pub summary: String,
    pub must_haves: Vec<MustHave>,
    pub strengths: Vec<String>,
    pub red_flags: Vec<String>,
    pub recruiter_take: String,
    /// Set by Onsite, not the model.
    pub model: String,
    pub duration_ms: u64,
}

fn clip(s: &str, n: usize) -> String {
    s.trim().chars().take(n).collect()
}

impl AiScreen {
    /// Model output is data: clamp every field to what the UI expects.
    pub fn sanitize(mut self) -> Self {
        let g = self.grade.trim().to_uppercase();
        self.grade = match g.chars().next() {
            Some(c @ 'A'..='D') => c.to_string(),
            _ => match self.fit {
                85.. => "A",
                70..=84 => "B",
                50..=69 => "C",
                _ => "D",
            }
            .to_string(),
        };
        self.fit = self.fit.min(100);
        self.level = match self.level.trim().to_lowercase().as_str() {
            l @ ("below" | "at" | "above") => l.to_string(),
            _ => String::new(),
        };
        self.level_note = clip(&self.level_note, 200);
        self.summary = clip(&self.summary, 400);
        self.recruiter_take = clip(&self.recruiter_take, 300);
        self.must_haves = std::mem::take(&mut self.must_haves)
            .into_iter()
            .filter(|m| !m.skill.trim().is_empty())
            .take(20)
            .map(|m| MustHave {
                skill: clip(&m.skill, 60),
                verdict: match m.verdict.trim().to_lowercase().as_str() {
                    v @ ("proven" | "claimed") => v.to_string(),
                    _ => "missing".to_string(),
                },
                evidence: clip(&m.evidence, 240),
            })
            .collect();
        for list in [&mut self.strengths, &mut self.red_flags] {
            *list = list.iter().map(|s| clip(s, 240)).filter(|s| !s.is_empty()).take(4).collect();
        }
        self
    }
}

fn cache_key(resume: &str, jd: &str) -> String {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let hash = Sha256::digest(format!("{CACHE_VERSION}\n{}\n---\n{}", norm(resume), norm(jd)).as_bytes());
    let hex: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("ats_screen:{hex}")
}

pub fn render_prompt(template: &str, resume: &str, jd: &str) -> String {
    let take = |s: &str| s.chars().take(MAX_INPUT_CHARS).collect::<String>();
    prompts::render(template, &[("jd", take(jd)), ("resume", take(resume))].into())
}

/// Grades `resume` (the extracted text) against `jd`. One Claude call with no tools, then cached.
pub async fn screen(state: &AppState, resume: &str, jd: &str) -> Result<AiScreen> {
    if resume.split_whitespace().count() < 20 {
        return Err(AppError::Invalid("Too little text came out of the resume for an AI screen.".into()));
    }
    if jd.trim().is_empty() {
        return Err(AppError::Invalid("The AI screen needs a job description.".into()));
    }
    let key = cache_key(resume, jd);
    if let Ok(Some(hit)) = state.store.lock().unwrap().get_setting::<AiScreen>(&key) {
        return Ok(hit);
    }
    let template = prompts::load(&state.prompts_dir, "ats_screen.md")?;
    let prompt = render_prompt(&template, resume, jd);
    let backend = state.backend().await?;
    // Judging fit is the report model's kind of job.
    let model = state.settings().report_model;
    let out = run_once(&backend, &prompt, Tools::None, &model, |_| {}).await?;
    let json = out.json.ok_or_else(|| AppError::Ai("The AI screen came back unreadable. Try again.".into()))?;
    let mut s = serde_json::from_value::<AiScreen>(json)?.sanitize();
    s.model = out.model;
    s.duration_ms = out.duration_ms;
    tracing::info!(ms = s.duration_ms, grade = %s.grade, fit = s.fit, "AI screen done");
    state.store.lock().unwrap().set_setting(&key, &s)?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitize_clamps_model_output() {
        let raw = json!({
            "grade": "b+", "fit": 140, "advance": true, "level": "Above",
            "must_haves": [
                {"skill": "Kafka", "verdict": "Proven", "evidence": "Ran Kafka consumers"},
                {"skill": "K8s", "verdict": "maybe"},
                {"skill": " "}
            ],
            "strengths": ["a", "", "b", "c", "d", "e"],
            "unknown": 1
        });
        let s = serde_json::from_value::<AiScreen>(raw).unwrap().sanitize();
        assert_eq!(s.grade, "B");
        assert_eq!(s.fit, 100);
        assert_eq!(s.level, "above");
        assert_eq!(s.must_haves.len(), 2);
        assert_eq!(s.must_haves[0].verdict, "proven");
        assert_eq!(s.must_haves[1].verdict, "missing");
        assert_eq!(s.strengths, ["a", "b", "c", "d"]);
    }

    #[test]
    fn grade_falls_back_to_fit() {
        let s = AiScreen { grade: "?".into(), fit: 72, ..Default::default() }.sanitize();
        assert_eq!(s.grade, "B");
    }

    #[test]
    fn cache_key_tracks_both_inputs() {
        assert_eq!(cache_key("a  b", "x"), cache_key("a b", "x"));
        assert_ne!(cache_key("a b", "x"), cache_key("a b", "y"));
        assert_ne!(cache_key("a b", "x"), cache_key("a c", "x"));
    }

    #[test]
    fn prompt_renders_both_blocks() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts");
        let t = prompts::load(&dir, "ats_screen.md").unwrap();
        let p = render_prompt(&t, "RESUME TEXT", "JD TEXT");
        assert!(p.contains("<<<\nJD TEXT\n>>>") && p.contains("<<<\nRESUME TEXT\n>>>") && !p.contains("{{"));
    }
}
