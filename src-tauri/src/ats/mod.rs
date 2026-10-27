//! Resume check: how an ATS reads your resume, and how well it matches a posting.
//!
//! 1. `extract`: the file's text and layout, as a simple ATS parser would see it.
//! 2. `checks`: can it be read at all (columns, tables, text boxes, images, contact, headings).
//! 3. `job`: what the posting asks for (Claude reads the posting only; cached).
//! 4. `score`: Resume-Matcher's match score plus six simulated ATS products.

pub mod checks;
pub mod extract;
pub mod job;
pub mod score;
pub mod text;

use crate::error::{AppError, Result};
use crate::AppState;
use base64::prelude::*;
use checks::{Category, Check, Status};
use extract::{Document, Kind};
use job::JobKeywords;
use score::{AtsScore, Facts, Inputs, MatchScore, TermHit};
use serde::Serialize;

/// Settings key for the last report, so the page shows it again after a restart.
pub const LAST_KEY: &str = "ats_last";
const PREVIEW_CHARS: usize = 4000;

#[derive(Debug, Clone, Serialize)]
pub struct FileInfo {
    pub name: String,
    pub kind: Kind,
    pub kind_label: &'static str,
    pub pages: Option<usize>,
    pub words: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub file: FileInfo,
    /// "full", "partial" or "none": how much text an ATS gets out of the file.
    pub extractability: &'static str,
    /// Can an ATS read the file (extraction and layout checks).
    pub parse_score: u32,
    /// Contact details, headings and dates.
    pub content_score: u32,
    pub checks: Vec<Check>,
    pub job: Option<JobKeywords>,
    /// Resume-Matcher's score. Without a posting it covers sections and dates only.
    pub matched: MatchScore,
    pub keywords: Vec<TermHit>,
    pub ats: Vec<AtsScore>,
    pub tips: Vec<String>,
    pub sections: Vec<&'static str>,
    pub experience_years: Option<f64>,
    /// The start of the text exactly as extracted: what the ATS sees.
    pub preview: String,
    pub checked_at: String,
}

/// Pure scoring: no I/O, so it is tested directly.
pub fn analyze(name: &str, doc: &Document, job: Option<&JobKeywords>, today: (i32, u32)) -> Report {
    let facts = Facts::new(doc, today);
    let checks = checks::run(doc, &facts.found);
    let status = |id: &str| checks.iter().find(|c| c.id == id).map(|c| c.status);
    let extractability = if status("text_layer") == Some(Status::Fail) {
        "none"
    } else if status("garbled_glyphs") == Some(Status::Fail) || status("text_coverage").is_some_and(|s| s != Status::Pass) || doc.truncated {
        "partial"
    } else {
        "full"
    };
    // An empty read is a zero, whatever the other checks say.
    let parse_score = if extractability == "none" { 0 } else { checks::score(&checks, &[Category::Extraction, Category::Layout]) };
    let content_score = checks::score(&checks, &[Category::Content]);
    let hits = job.map(|j| score::term_hits(j, &facts)).unwrap_or_default();
    let matched = score::match_score(job, &hits, &facts);
    let inputs = Inputs { doc, facts: &facts, checks: &checks, parse_score, job, hits: &hits, matched: &matched };
    let ats = score::ats_scores(&inputs);
    let tips = score::tips(&inputs);
    Report {
        file: FileInfo { name: name.to_string(), kind: doc.kind, kind_label: doc.kind.label(), pages: doc.page_count, words: facts.words },
        extractability,
        parse_score,
        content_score,
        job: job.cloned(),
        keywords: hits,
        ats,
        tips,
        sections: facts.found.clone(),
        experience_years: facts.experience_months.map(|m| (m as f64 / 12.0 * 10.0).round() / 10.0),
        preview: doc.text.chars().take(PREVIEW_CHARS).collect(),
        checked_at: chrono::Utc::now().to_rfc3339(),
        matched,
        checks,
    }
}

fn decode(data_b64: &str) -> Result<Vec<u8>> {
    BASE64_STANDARD.decode(data_b64.trim()).map_err(|_| AppError::Invalid("Couldn't read that file.".into()))
}

/// Extraction can take a second on a big PDF; keep it off the async runtime.
pub async fn read_file(name: String, data_b64: &str) -> Result<Document> {
    let bytes = decode(data_b64)?;
    tokio::task::spawn_blocking(move || extract::extract(&name, &bytes))
        .await
        .map_err(|e| AppError::Invalid(format!("Reading the file failed: {e}")))?
}

pub fn today() -> (i32, u32) {
    use chrono::Datelike;
    let d = chrono::Utc::now().date_naive();
    (d.year(), d.month())
}

/// The full check. A file wins over pasted text, since only a file shows layout problems.
pub async fn run(state: &AppState, name: Option<String>, data_b64: Option<String>, text: Option<String>, jd: &str) -> Result<Report> {
    let (name, doc) = match (name, data_b64) {
        (Some(n), Some(d)) => {
            let doc = read_file(n.clone(), &d).await?;
            (n, doc)
        }
        _ => {
            let t = text.unwrap_or_default();
            if t.trim().is_empty() {
                return Err(AppError::Invalid("Add your resume first.".into()));
            }
            ("Pasted text".to_string(), Document::from_text(&t))
        }
    };
    let job = if jd.trim().is_empty() { None } else { Some(job::keywords(state, jd).await?) };
    let report = analyze(&name, &doc, job.as_ref(), today());
    if let Err(e) = state.store.lock().unwrap().set_setting(LAST_KEY, &report) {
        tracing::warn!("saving resume check: {e}");
    }
    tracing::info!(kind = ?doc.kind, parse = report.parse_score, matched = report.matched.overall, "resume checked");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_from_pasted_text() {
        let doc = Document::from_text("Jane Doe\njane@x.io +1 415 555 0101\nExperience\nAcme Jan 2021 - Present\nEducation\nMIT 2016 - 2020\nSkills\nRust");
        let r = analyze("Pasted text", &doc, None, (2026, 10));
        assert_eq!(r.extractability, "full");
        assert_eq!(r.sections, ["experience", "education", "skills"]);
        assert_eq!(r.experience_years, Some(5.8));
        assert_eq!(r.parse_score, 100);
        assert!(r.keywords.is_empty() && r.job.is_none());
        assert_eq!(r.ats.len(), 6);
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["file"]["kind"], "text");
        assert_eq!(v["ats"][0]["strategy"], "exact");
    }

    #[test]
    fn base64_round_trip() {
        let enc = BASE64_STANDARD.encode(b"hello");
        assert_eq!(decode(&enc).unwrap(), b"hello");
        assert!(decode("***").is_err());
    }
}
