//! Report and verdict prompts built from stored rounds.

use crate::brain::prompts::{render, Vars};
use crate::store::Turn;
use serde::Serialize;
use serde_json::Value;

const TRANSCRIPT_MAX: usize = 150_000;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Metrics {
    pub answers: usize,
    pub avg_words: usize,
    pub longest: usize,
    pub minutes: i64,
    pub code_shares: usize,
}

/// Below this the report would be guesswork, so we don't ask Claude for one.
const MIN_ANSWERS: usize = 2;
const MIN_WORDS: usize = 40;

/// True when the candidate said enough (at least 2 answers and 40 words) to review.
/// Workspace-only turns have no text and don't count.
pub fn enough_to_review(turns: &[Turn]) -> bool {
    let answers: Vec<usize> = turns
        .iter()
        .filter(|t| t.who == "me")
        .map(|t| t.text.trim())
        .filter(|t| !t.is_empty() && *t != "(shared workspace)")
        .map(|t| t.split_whitespace().count())
        .collect();
    answers.len() >= MIN_ANSWERS && answers.iter().sum::<usize>() >= MIN_WORDS
}

pub fn metrics(turns: &[Turn], actual_minutes: f64) -> Metrics {
    let words: Vec<usize> = turns
        .iter()
        .filter(|t| t.who == "me" && !t.text.trim().is_empty())
        .map(|t| t.text.split_whitespace().count())
        .collect();
    Metrics {
        answers: words.len(),
        avg_words: if words.is_empty() { 0 } else { (words.iter().sum::<usize>() as f64 / words.len() as f64).round() as usize },
        longest: words.iter().copied().max().unwrap_or(0),
        minutes: actual_minutes.round() as i64,
        code_shares: turns.iter().filter(|t| t.shared_workspace).count(),
    }
}

pub fn transcript_text(turns: &[Turn]) -> String {
    let text = turns
        .iter()
        .map(|t| format!("{}: {}", if t.who == "iv" { "INTERVIEWER" } else { "CANDIDATE" }, t.sent.as_deref().unwrap_or(&t.text)))
        .collect::<Vec<_>>()
        .join("\n\n");
    if text.len() <= TRANSCRIPT_MAX {
        return text;
    }
    // Keep the end: wrap-up and the candidate's questions matter for the report.
    let mut start = text.len() - TRANSCRIPT_MAX;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

pub struct ReportInput<'a> {
    pub round_name: &'a str,
    pub round_minutes: i64,
    pub company: &'a str,
    pub level_name: &'a str,
    pub level_bar: &'a str,
    pub rubric: &'a [String],
    pub lang: &'a str,
    pub turns: &'a [Turn],
    pub workspace: &'a Value,
    pub metrics: &'a Metrics,
}

pub fn report_prompt(template: &str, r: &ReportInput) -> String {
    let field = |k: &str| r.workspace[k].as_str().filter(|s| !s.trim().is_empty()).unwrap_or("(empty)").to_string();
    let final_ws = format!("Final editor ({}):\n{}\n\nFinal whiteboard:\n{}", r.lang, field("code"), field("notes"));
    let vars: Vars = [
        ("round_name", r.round_name.to_string()),
        ("round_minutes", r.round_minutes.to_string()),
        ("actual_minutes", r.metrics.minutes.to_string()),
        ("company", r.company.to_string()),
        ("level_name", r.level_name.to_string()),
        ("level_bar", r.level_bar.to_string()),
        ("rubric", r.rubric.join(", ")),
        ("answers", r.metrics.answers.to_string()),
        ("avg_words", r.metrics.avg_words.to_string()),
        ("longest", r.metrics.longest.to_string()),
        ("transcript", transcript_text(r.turns)),
        ("final_workspace", final_ws),
    ]
    .into();
    render(template, &vars)
}

/// One line per round report, as the committee reads them.
pub fn round_summary(name: &str, rep: &Value) -> String {
    let list = |k: &str| {
        rep[k].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; ")).unwrap_or_default()
    };
    let dims = rep["dimensions"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|d| {
                    let score = d["score"].as_i64().map_or_else(|| "not assessed".to_string(), |n| format!("{n}/4"));
                    format!("{} {score}", d["name"].as_str().unwrap_or("?"))
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    format!(
        "{name}: {}, {}/100. {} Dimensions: {dims}. Gaps: {}",
        rep["decision"].as_str().unwrap_or("?"),
        rep["overall"].as_i64().unwrap_or(0),
        rep["headline"].as_str().unwrap_or(""),
        list("held_back"),
    )
}

pub fn verdict_prompt(template: &str, company: &str, level_name: &str, summaries: &[String]) -> String {
    let vars: Vars = [
        ("company", company.to_string()),
        ("level_name", level_name.to_string()),
        ("round_summaries", summaries.join("\n")),
    ]
    .into();
    render(template, &vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn turn(who: &str, text: &str, shared: bool) -> Turn {
        Turn { who: who.into(), text: text.into(), at_min: 0.0, ttft_ms: None, shared_workspace: shared, sent: None }
    }

    #[test]
    fn metrics_and_transcript() {
        let t = vec![turn("iv", "Hi. Tell me about X.", false), turn("me", "one two three", false), turn("me", "a b", true)];
        assert_eq!(metrics(&t, 9.6), Metrics { answers: 2, avg_words: 3, longest: 3, minutes: 10, code_shares: 1 });
        assert_eq!(transcript_text(&t[..2]), "INTERVIEWER: Hi. Tell me about X.\n\nCANDIDATE: one two three");
    }

    #[test]
    fn report_prompt_renders() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts");
        let tpl = crate::brain::prompts::load(&dir, "report.md").unwrap();
        let t = vec![turn("iv", "Q?", false)];
        let m = metrics(&t, 1.0);
        let rubric = vec!["Problem solving".to_string()];
        let p = report_prompt(
            &tpl,
            &ReportInput {
                round_name: "DSA coding",
                round_minutes: 10,
                company: "Google",
                level_name: "SDE-2",
                level_bar: "bar",
                rubric: &rubric,
                lang: "Java",
                turns: &t,
                workspace: &json!({"code": "int x;"}),
                metrics: &m,
            },
        );
        assert!(!p.contains("{{"));
        assert!(p.contains("Final editor (Java):\nint x;\n\nFinal whiteboard:\n(empty)"));
    }

    #[test]
    fn review_needs_two_answers_and_forty_words() {
        let words = |n: usize| vec!["w"; n].join(" ");
        assert!(!enough_to_review(&[]));
        assert!(!enough_to_review(&[turn("iv", &words(100), false), turn("me", "Hi", false)]));
        // One long answer is still one answer.
        assert!(!enough_to_review(&[turn("me", &words(80), false)]));
        // Two answers, 39 words.
        assert!(!enough_to_review(&[turn("me", &words(20), false), turn("me", &words(19), false)]));
        // Workspace-only turns and the UI placeholder don't count.
        assert!(!enough_to_review(&[turn("me", &words(40), false), turn("me", "", true), turn("me", "(shared workspace)", true)]));
        assert!(enough_to_review(&[turn("me", &words(20), false), turn("me", &words(20), true)]));
    }

    #[test]
    fn summary_line_with_unassessed_dimension() {
        let rep = json!({"decision":"Lean hire","overall":55,"headline":"Short.","dimensions":[{"name":"Depth","score":null},{"name":"Clarity","score":3}],"held_back":[]});
        assert_eq!(round_summary("Resume", &rep), "Resume: Lean hire, 55/100. Short. Dimensions: Depth not assessed, Clarity 3/4. Gaps: ");
    }

    #[test]
    fn summary_line() {
        let rep = json!({"decision":"Hire","overall":72,"headline":"Solid.","dimensions":[{"name":"Code quality","score":3}],"held_back":["slow"]});
        assert_eq!(round_summary("DSA", &rep), "DSA: Hire, 72/100. Solid. Dimensions: Code quality 3/4. Gaps: slow");
    }
}
