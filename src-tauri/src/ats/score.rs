//! Job match and per-ATS scores. All deterministic: same file and posting, same numbers.
//!
//! The match score is Resume-Matcher's `services/ats.py` (Apache-2.0): keyword match 40%,
//! skills coverage 20%, job title 15%, sections 15%, date consistency 10%, with weights of
//! unscorable parts spread over the rest. The six ATS profiles follow ats-screener
//! (`src/lib/engine/scorer/profiles`, MIT): each product reads and matches differently.
//! They are approximations from public docs, not the vendors' own code.

use super::checks::{Category, Check, Status};
use super::extract::{Document, Kind};
use super::job::JobKeywords;
use super::text::{self, keyword_in_text};
use serde::Serialize;

// ---------------------------------------------------------------- facts about the resume

/// Everything the scorers need, computed once from the extracted text.
pub struct Facts {
    pub lower: String,
    pub stems: Vec<String>,
    pub sections: Vec<(&'static str, String)>,
    pub found: Vec<&'static str>,
    pub has_contact: bool,
    pub experience_months: Option<u32>,
    pub bullets: usize,
    pub quantified: usize,
    pub words: usize,
}

impl Facts {
    pub fn new(doc: &Document, today: (i32, u32)) -> Self {
        let lower = text::normalize(&doc.text);
        let sections = text::sections(&lower);
        let mut found: Vec<&'static str> = sections.iter().map(|(k, _)| *k).filter(|k| *k != "_top").collect();
        found.dedup();
        let exp = text::section_text(&sections, "experience");
        // Experience dates from the Experience section; with no such heading, from the whole text.
        let experience_months = text::experience_months(if exp.trim().is_empty() { &lower } else { &exp }, today);
        let bullet_lines: Vec<&str> = exp.lines().map(str::trim).filter(|l| l.split_whitespace().count() >= 6).collect();
        let quantified = bullet_lines.iter().filter(|l| l.chars().any(|c| c.is_ascii_digit()) || l.contains('%') || l.contains('$') || l.contains('₹')).count();
        Self {
            stems: text::stems(&lower),
            has_contact: text::EMAIL.is_match(&doc.text) || text::has_phone(&doc.text),
            words: doc.text.split_whitespace().count(),
            bullets: bullet_lines.len(),
            quantified,
            experience_months,
            found,
            sections,
            lower,
        }
    }

    fn section(&self, kind: &str) -> String {
        text::section_text(&self.sections, kind)
    }
}

// ---------------------------------------------------------------- terms

#[derive(Debug, Clone, Serialize)]
pub struct TermHit {
    pub term: String,
    pub required: bool,
    /// The posting's exact words are in the resume.
    pub exact: bool,
    /// Same word stems in a row ("microservice" for "microservices").
    pub stem: bool,
    /// The alias the resume uses instead ("Postgres" for "PostgreSQL").
    pub alias: Option<String>,
}

impl TermHit {
    pub fn found_any(&self) -> bool {
        self.exact || self.stem || self.alias.is_some()
    }
}

pub fn term_hits(job: &JobKeywords, f: &Facts) -> Vec<TermHit> {
    job.terms()
        .into_iter()
        .map(|(term, required)| {
            let exact = keyword_in_text(&term, &f.lower);
            let stem = exact || text::stem_match(&term, &f.stems);
            let alias = if exact { None } else { job.aliases_for(&term).into_iter().find(|a| keyword_in_text(a, &f.lower)) };
            TermHit { term, required, exact, stem, alias }
        })
        .collect()
}

// ---------------------------------------------------------------- Resume-Matcher match score

const TITLE_IN_HEADLINE: f64 = 100.0;
const TITLE_IN_SUMMARY: f64 = 75.0;
const TITLE_OVERLAP_MAX: f64 = 50.0;
const TITLE_STOPWORDS: [&str; 10] = ["a", "an", "and", "the", "of", "for", "in", "to", "at", "&"];

#[derive(Debug, Clone, Serialize)]
pub struct SubScores {
    pub keyword_match: Option<f64>,
    pub skills_coverage: Option<f64>,
    pub title_match: Option<f64>,
    pub section_completeness: f64,
    pub date_consistency: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MatchScore {
    pub overall: u32,
    pub sub_scores: SubScores,
}

fn title_tokens(s: &str) -> std::collections::HashSet<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty() && !TITLE_STOPWORDS.contains(t)).map(String::from).collect()
}

/// 100 when the exact title is in the headline (text above the first heading), 75 in the
/// summary, up to 50 for word overlap. Recruiters filter by title literally.
pub fn title_match(role: &str, f: &Facts) -> Option<f64> {
    let role = role.trim();
    if role.is_empty() {
        return None;
    }
    let top = f.section("_top");
    if keyword_in_text(role, &top) {
        return Some(TITLE_IN_HEADLINE);
    }
    if keyword_in_text(role, &f.section("summary")) {
        return Some(TITLE_IN_SUMMARY);
    }
    let want = title_tokens(role);
    if want.is_empty() {
        return Some(0.0);
    }
    let have = title_tokens(&top);
    Some((want.intersection(&have).count() as f64 / want.len() as f64 * TITLE_OVERLAP_MAX * 10.0).round() / 10.0)
}

/// Contact details plus Summary, Experience, Education and Skills, each one fifth.
pub fn section_completeness(f: &Facts) -> f64 {
    let n = ["summary", "experience", "education", "skills"].iter().filter(|k| f.found.contains(k)).count() + usize::from(f.has_contact);
    n as f64 / 5.0 * 100.0
}

fn pct(hits: usize, total: usize) -> Option<f64> {
    (total > 0).then(|| (hits as f64 / total as f64 * 1000.0).round() / 10.0)
}

pub fn match_score(job: Option<&JobKeywords>, hits: &[TermHit], f: &Facts) -> MatchScore {
    let dated = ["experience", "education", "projects"].iter().map(|k| f.section(k)).collect::<Vec<_>>().join("\n");
    let sub = SubScores {
        keyword_match: job.and_then(|_| pct(hits.iter().filter(|h| h.exact).count(), hits.len())),
        skills_coverage: job.and_then(|_| {
            let skills: Vec<&TermHit> = hits.iter().filter(|h| h.required || job.is_some_and(|j| j.preferred_skills.contains(&h.term))).collect();
            pct(skills.iter().filter(|h| h.exact).count(), skills.len())
        }),
        title_match: job.and_then(|j| title_match(&j.role, f)),
        section_completeness: section_completeness(f),
        date_consistency: text::date_consistency(if dated.trim().is_empty() { &f.lower } else { &dated }),
    };
    let parts = [
        (sub.keyword_match, 0.40),
        (sub.skills_coverage, 0.20),
        (sub.title_match, 0.15),
        (Some(sub.section_completeness), 0.15),
        (sub.date_consistency, 0.10),
    ];
    MatchScore { overall: weighted(&parts), sub_scores: sub }
}

/// Weighted mean of the parts that could be scored.
fn weighted(parts: &[(Option<f64>, f64)]) -> u32 {
    let (sum, w) = parts.iter().filter_map(|(v, w)| v.map(|v| (v * w, *w))).fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1));
    if w == 0.0 { 0 } else { (sum / w).clamp(0.0, 100.0).round() as u32 }
}

// ---------------------------------------------------------------- ATS profiles

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Strategy {
    /// Literal terms only (Workday, Taleo).
    Exact,
    /// Word stems, but no abbreviations (Lever).
    Stem,
    /// Normalises skills to a taxonomy, so aliases count (SuccessFactors / Textkernel).
    Taxonomy,
    /// ML matching: stems and aliases count, at slightly less than an exact hit (iCIMS, Greenhouse).
    Semantic,
}

impl Strategy {
    fn credit(self, h: &TermHit) -> f64 {
        if h.exact {
            return 1.0;
        }
        match self {
            Strategy::Exact => 0.0,
            Strategy::Stem => if h.stem { 1.0 } else { 0.0 },
            Strategy::Taxonomy => if h.alias.is_some() { 1.0 } else { 0.0 },
            Strategy::Semantic => if h.stem || h.alias.is_some() { 0.9 } else { 0.0 },
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Strategy::Exact => "Matches the posting's exact words",
            Strategy::Stem => "Matches word forms, not abbreviations",
            Strategy::Taxonomy => "Maps skills to a standard list, so common aliases count",
            Strategy::Semantic => "Matches by meaning, so close wording counts",
        }
    }
}

struct Profile {
    id: &'static str,
    name: &'static str,
    vendor: &'static str,
    strategy: Strategy,
    /// How much layout problems hurt: 1.0 = the full parse penalty.
    strictness: f64,
    /// parse, keywords, required skills, title, sections, dates.
    weights: [f64; 6],
    passing: u32,
    note: &'static str,
}

const PROFILES: [Profile; 6] = [
    Profile { id: "workday", name: "Workday", vendor: "Workday", strategy: Strategy::Exact, strictness: 1.0, weights: [0.25, 0.30, 0.15, 0.10, 0.12, 0.08], passing: 70,
        note: "Strict parser. Skips page headers and footers. Recruiters search with exact terms." },
    Profile { id: "taleo", name: "Taleo", vendor: "Oracle", strategy: Strategy::Exact, strictness: 0.95, weights: [0.20, 0.30, 0.20, 0.10, 0.12, 0.08], passing: 65,
        note: "Oldest and most literal. Boolean keyword filters and knock-out questions." },
    Profile { id: "icims", name: "iCIMS", vendor: "iCIMS", strategy: Strategy::Semantic, strictness: 0.6, weights: [0.15, 0.35, 0.15, 0.15, 0.12, 0.08], passing: 60,
        note: "Most forgiving parser. Its Role Fit model matches by meaning." },
    Profile { id: "greenhouse", name: "Greenhouse", vendor: "Greenhouse", strategy: Strategy::Semantic, strictness: 0.6, weights: [0.20, 0.30, 0.15, 0.15, 0.12, 0.08], passing: 60,
        note: "Doesn't auto-reject. A person reviews, often after a keyword search." },
    Profile { id: "lever", name: "Lever", vendor: "Employ", strategy: Strategy::Stem, strictness: 0.75, weights: [0.20, 0.35, 0.15, 0.10, 0.12, 0.08], passing: 60,
        note: "No ranking; recruiters search. Search matches word forms but not abbreviations." },
    Profile { id: "successfactors", name: "SuccessFactors", vendor: "SAP", strategy: Strategy::Taxonomy, strictness: 0.85, weights: [0.20, 0.30, 0.20, 0.10, 0.12, 0.08], passing: 65,
        note: "Textkernel parser. Normalises skills to a taxonomy." },
];

#[derive(Debug, Clone, Serialize)]
pub struct AtsScore {
    pub id: &'static str,
    pub name: &'static str,
    pub vendor: &'static str,
    pub score: u32,
    pub passing: u32,
    pub passes: bool,
    pub strategy: Strategy,
    pub matching: &'static str,
    pub note: &'static str,
    /// Problems specific to this product, worst first.
    pub issues: Vec<String>,
}

fn credit_pct(hits: &[&TermHit], s: Strategy) -> Option<f64> {
    (!hits.is_empty()).then(|| hits.iter().map(|h| s.credit(h)).sum::<f64>() / hits.len() as f64 * 100.0)
}

pub struct Inputs<'a> {
    pub doc: &'a Document,
    pub facts: &'a Facts,
    pub checks: &'a [Check],
    pub parse_score: u32,
    pub job: Option<&'a JobKeywords>,
    pub hits: &'a [TermHit],
    pub matched: &'a MatchScore,
}

fn failed(checks: &[Check], id: &str) -> bool {
    checks.iter().any(|c| c.id == id && c.status == Status::Fail)
}

fn not_pass(checks: &[Check], id: &str) -> bool {
    checks.iter().any(|c| c.id == id && c.status != Status::Pass)
}

pub fn ats_scores(i: &Inputs) -> Vec<AtsScore> {
    let unreadable = failed(i.checks, "text_layer");
    PROFILES
        .iter()
        .map(|p| {
            let mut issues: Vec<(f64, String)> = vec![];
            let all: Vec<&TermHit> = i.hits.iter().collect();
            let req: Vec<&TermHit> = i.hits.iter().filter(|h| h.required).collect();
            let parse = 100.0 - (100.0 - i.parse_score as f64) * p.strictness;
            let parts = [
                (Some(parse), p.weights[0]),
                (credit_pct(&all, p.strategy), p.weights[1]),
                (credit_pct(&req, p.strategy), p.weights[2]),
                (i.matched.sub_scores.title_match, p.weights[3]),
                (Some(i.matched.sub_scores.section_completeness), p.weights[4]),
                (i.matched.sub_scores.date_consistency, p.weights[5]),
            ];
            let mut score = weighted(&parts) as f64;

            // Product quirks. Penalties are points off the weighted score.
            let pages = i.doc.page_count.unwrap_or(1);
            match p.id {
                "workday" => {
                    if pages > 2 {
                        issues.push((8.0, format!("{pages} pages: Workday may cut text after page 2.")));
                    }
                    if failed(i.checks, "header_footer") {
                        issues.push((10.0, "Your contact details are only in the page header, which Workday drops.".into()));
                    }
                    if failed(i.checks, "multi_column") {
                        issues.push((5.0, "Workday reads two columns as one, mixing lines.".into()));
                    }
                }
                "taleo" => {
                    if i.doc.tables > 0 {
                        issues.push((8.0, "Taleo often skips table cells.".into()));
                    }
                    let missing: Vec<&str> = text::REQUIRED_SECTIONS.into_iter().filter(|k| !i.facts.found.contains(k)).collect();
                    if missing.len() > 1 || (!i.facts.has_contact && !missing.is_empty()) {
                        issues.push((8.0, format!("Taleo needs standard headings. Missing: {}.", missing.join(", "))));
                    }
                    let missing_req: Vec<&str> = req.iter().filter(|h| !h.exact).map(|h| h.term.as_str()).collect();
                    if !missing_req.is_empty() && i.job.is_some() {
                        issues.push((0.0, format!("A Taleo AND-filter on must-haves would drop you for: {}.", missing_req.iter().take(4).copied().collect::<Vec<_>>().join(", "))));
                    }
                }
                "lever" => {
                    let abbrev: Vec<String> = all.iter().filter(|h| !h.stem && h.alias.is_some()).map(|h| format!("\"{}\" (you wrote \"{}\")", h.term, h.alias.as_deref().unwrap_or(""))).collect();
                    if !abbrev.is_empty() {
                        issues.push((0.0, format!("Lever search won't link other names or abbreviations: {}.", abbrev.into_iter().take(3).collect::<Vec<_>>().join(", "))));
                    }
                }
                "greenhouse" => {
                    if i.facts.bullets > 0 && (i.facts.quantified as f64) < i.facts.bullets as f64 * 0.3 {
                        issues.push((0.0, "A person reads Greenhouse applications. Few bullets have numbers.".into()));
                    }
                }
                "successfactors" if not_pass(i.checks, "text_boxes") => {
                    issues.push((5.0, "Textkernel skips text boxes.".into()));
                }
                _ => {}
            }
            if let (Some(need), Some(m), Some(_)) = (i.job.and_then(|j| j.experience_years), i.facts.experience_months, i.job) {
                let have = m as f64 / 12.0;
                if have + 0.5 < need && matches!(p.id, "workday" | "taleo" | "successfactors") {
                    issues.push((10.0, format!("The posting asks for {need} years; your dates add up to about {have:.1}. A knock-out question would screen you out.")));
                }
            }
            if p.strategy == Strategy::Exact {
                let alias_only: Vec<String> = all.iter().filter(|h| !h.exact && h.alias.is_some()).map(|h| format!("\"{}\"", h.term)).collect();
                if !alias_only.is_empty() {
                    issues.push((0.0, format!("Uses exact words. Write {} as the posting does.", alias_only.into_iter().take(4).collect::<Vec<_>>().join(", "))));
                }
            }

            score -= issues.iter().map(|(p, _)| p).sum::<f64>();
            let score = if unreadable { 0 } else { score.clamp(0.0, 100.0).round() as u32 };
            if unreadable {
                issues.insert(0, (100.0, "No readable text: this ATS sees an empty resume.".into()));
            }
            issues.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            AtsScore {
                id: p.id,
                name: p.name,
                vendor: p.vendor,
                score,
                passing: p.passing,
                passes: score >= p.passing,
                strategy: p.strategy,
                matching: p.strategy.describe(),
                note: p.note,
                issues: issues.into_iter().map(|(_, s)| s).collect(),
            }
        })
        .collect()
}

// ---------------------------------------------------------------- tips

pub fn tips(i: &Inputs) -> Vec<String> {
    let mut out = vec![];
    let sub = &i.matched.sub_scores;
    if failed(i.checks, "text_layer") {
        out.push("Export the resume again as a text PDF or .docx. Right now an ATS reads nothing.".to_string());
        return out;
    }
    for c in i.checks.iter().filter(|c| c.status == Status::Fail && c.category != Category::Content) {
        out.push(c.detail.clone());
    }
    if let Some(job) = i.job {
        if sub.title_match.is_some_and(|t| t < TITLE_IN_HEADLINE) {
            out.push(format!("Put the exact job title \"{}\" in the line under your name. Recruiters filter by title literally.", job.role));
        }
        let missing_req: Vec<&str> = i.hits.iter().filter(|h| h.required && !h.found_any()).map(|h| h.term.as_str()).collect();
        if !missing_req.is_empty() {
            out.push(format!("Must-have skills missing: {}. Add the ones you really have, in the posting's words.", missing_req.iter().take(6).copied().collect::<Vec<_>>().join(", ")));
        }
        let missing_other: Vec<&str> = i.hits.iter().filter(|h| !h.required && !h.found_any()).map(|h| h.term.as_str()).collect();
        if sub.keyword_match.is_some_and(|k| k < 60.0) && !missing_other.is_empty() {
            out.push(format!("Other terms from the posting you don't use: {}.", missing_other.iter().take(6).copied().collect::<Vec<_>>().join(", ")));
        }
        let alias: Vec<String> = i.hits.iter().filter(|h| !h.exact && h.alias.is_some()).map(|h| format!("\"{}\" (you have \"{}\")", h.term, h.alias.as_deref().unwrap_or(""))).collect();
        if !alias.is_empty() {
            out.push(format!("Also write the posting's words: {}. Workday and Taleo match exact words.", alias.into_iter().take(4).collect::<Vec<_>>().join(", ")));
        }
        if let (Some(need), Some(m)) = (job.experience_years, i.facts.experience_months) {
            if (m as f64 / 12.0) + 0.5 < need {
                out.push(format!("The posting asks for {need}+ years. Your dated roles add up to {:.1}. Check every role has start and end dates.", m as f64 / 12.0));
            }
        }
    }
    if sub.section_completeness < 100.0 {
        let missing: Vec<&str> = ["summary", "experience", "education", "skills"].into_iter().filter(|k| !i.facts.found.contains(k)).collect();
        if !missing.is_empty() {
            out.push(format!("Add clearly named sections for: {}.", missing.join(", ")));
        }
    }
    if sub.date_consistency.is_some_and(|d| d < 100.0) {
        out.push("Write every date the same way, e.g. \"Jan 2020 – Mar 2023\". Mixed formats make ATSs miscount your experience.".to_string());
    }
    if i.facts.bullets >= 3 && (i.facts.quantified as f64) < i.facts.bullets as f64 * 0.3 {
        out.push(format!("Only {} of {} experience lines have a number. Add scale, latency, money or percentages.", i.facts.quantified, i.facts.bullets));
    }
    if i.facts.words < 150 && i.doc.kind != Kind::Text {
        out.push(format!("Only {} words came out of the file. Check nothing is hidden in images or text boxes.", i.facts.words));
    }
    if out.is_empty() {
        out.push("Nothing major to fix. Tailor the summary and top bullets to each posting.".to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ats::checks;

    const RESUME: &str = "Jane Doe\nSenior Backend Engineer\njane@example.com | +91 98765 43210\n\nSummary\nBackend engineer building payment systems.\n\nExperience\nAcme Payments  Jan 2020 – Present\nBuilt event-driven microservice in Java on Postgres handling 2M payments a day\nRan Kafka consumers with idempotency keys and retries\nBeta Corp  Jun 2017 – Dec 2019\nWrote REST services and Redis caches for checkout flows\n\nEducation\nBITS Pilani  2013 – 2017\n\nSkills\nJava, Spring Boot, Kafka, Redis, Postgres, Docker";

    fn job() -> JobKeywords {
        JobKeywords {
            role: "Senior Backend Engineer".into(),
            required_skills: vec!["Java".into(), "PostgreSQL".into(), "Kafka".into(), "Kubernetes".into()],
            preferred_skills: vec!["Redis".into()],
            keywords: vec!["microservices".into(), "payments".into()],
            experience_years: Some(5.0),
            ..Default::default()
        }
    }

    fn run(text: &str, job: Option<&JobKeywords>) -> (MatchScore, Vec<AtsScore>, Vec<TermHit>) {
        let doc = Document::from_text(text);
        let facts = Facts::new(&doc, (2026, 10));
        let checks = checks::run(&doc, &facts.found);
        let hits = job.map(|j| term_hits(j, &facts)).unwrap_or_default();
        let m = match_score(job, &hits, &facts);
        let parse_score = checks::score(&checks, &[Category::Extraction, Category::Layout]);
        let inputs = Inputs { doc: &doc, facts: &facts, checks: &checks, parse_score, job, hits: &hits, matched: &m };
        (m.clone(), ats_scores(&inputs), hits)
    }

    #[test]
    fn match_score_follows_resume_matcher() {
        let j = job();
        let (m, _, hits) = run(RESUME, Some(&j));
        let by = |t: &str| hits.iter().find(|h| h.term == t).unwrap().clone();
        assert!(by("Java").exact);
        assert!(!by("PostgreSQL").exact && by("PostgreSQL").alias.as_deref() == Some("postgres"));
        assert!(!by("microservices").exact && by("microservices").stem);
        assert!(!by("Kubernetes").found_any());
        let s = &m.sub_scores;
        // Exact: Java, Kafka, Redis, payments = 4 of 7.
        assert_eq!(s.keyword_match, Some(57.1));
        // Skills (required + preferred): Java, Kafka, Redis = 3 of 5.
        assert_eq!(s.skills_coverage, Some(60.0));
        assert_eq!(s.title_match, Some(100.0));
        assert_eq!(s.section_completeness, 100.0);
        // "Jan 2020", "Jun 2017", "Dec 2019" vs bare years "2013", "2017".
        assert_eq!(s.date_consistency, Some(60.0));
        let expected = (57.1 * 0.4 + 60.0 * 0.2 + 100.0 * 0.15 + 100.0 * 0.15 + 60.0 * 0.1_f64).round() as u32;
        assert_eq!(m.overall, expected);
    }

    #[test]
    fn exact_profiles_punish_aliases_more_than_semantic_ones() {
        let j = job();
        let (_, ats, _) = run(RESUME, Some(&j));
        let get = |id: &str| ats.iter().find(|a| a.id == id).unwrap();
        assert!(get("workday").score < get("icims").score, "{:?}", ats.iter().map(|a| (a.id, a.score)).collect::<Vec<_>>());
        assert!(get("successfactors").score > get("taleo").score);
        assert!(get("workday").issues.iter().any(|i| i.contains("\"PostgreSQL\"")));
        assert!(get("lever").issues.iter().any(|i| i.contains("abbreviations") && i.contains("you wrote \"postgres\"")));
    }

    #[test]
    fn no_job_scores_layout_and_sections_only() {
        let (m, ats, hits) = run(RESUME, None);
        assert!(hits.is_empty());
        assert_eq!(m.sub_scores.keyword_match, None);
        assert_eq!(m.sub_scores.title_match, None);
        assert!(ats.iter().all(|a| a.score > 80), "{:?}", ats.iter().map(|a| a.score).collect::<Vec<_>>());
    }

    #[test]
    fn empty_resume_fails_everywhere() {
        let (_, ats, _) = run("Jane", Some(&job()));
        assert!(ats.iter().all(|a| a.score == 0 && !a.passes));
    }

    #[test]
    fn short_experience_is_flagged_for_knockout_products() {
        let mut j = job();
        j.experience_years = Some(12.0);
        let (_, ats, _) = run(RESUME, Some(&j));
        let knock = |id: &str| ats.iter().find(|a| a.id == id).unwrap().issues.iter().any(|i| i.contains("knock-out"));
        assert!(knock("taleo") && knock("workday") && !knock("greenhouse"));
    }

    #[test]
    fn title_overlap_is_partial() {
        let doc = Document::from_text("Jane\nBackend Developer\nExperience\nx");
        let f = Facts::new(&doc, (2026, 10));
        assert_eq!(title_match("Senior Backend Engineer", &f), Some(16.7));
        assert_eq!(title_match("", &f), None);
    }
}
