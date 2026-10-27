//! What the posting asks for. Claude reads the job description once (never the resume);
//! the result is cached by a hash of the text, so re-checking an edited resume is instant.

use crate::brain::{prompts, run_once, Tools};
use crate::error::{AppError, Result};
use crate::AppState;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Bump when the prompt or the shape changes, so old cache entries are ignored.
const CACHE_VERSION: &str = "2";
const MAX_TERMS: usize = 20;
const MAX_TERM_CHARS: usize = 60;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct JobKeywords {
    pub company: String,
    pub role: String,
    pub required_skills: Vec<String>,
    pub preferred_skills: Vec<String>,
    pub keywords: Vec<String>,
    pub aliases: BTreeMap<String, Vec<String>>,
    pub experience_years: Option<f64>,
    pub seniority: String,
}

/// Spellings candidates commonly use for the same thing. Claude adds posting-specific ones.
const BUILTIN_ALIASES: &[(&str, &[&str])] = &[
    ("postgresql", &["postgres", "psql"]),
    ("kubernetes", &["k8s"]),
    ("javascript", &["js", "ecmascript"]),
    ("typescript", &["ts"]),
    ("golang", &["go"]),
    ("go", &["golang"]),
    ("amazon web services", &["aws"]),
    ("aws", &["amazon web services"]),
    ("google cloud platform", &["gcp", "google cloud"]),
    ("gcp", &["google cloud platform", "google cloud"]),
    ("microsoft azure", &["azure"]),
    ("ci/cd", &["cicd", "ci cd", "continuous integration", "continuous delivery", "continuous deployment"]),
    ("node.js", &["nodejs", "node"]),
    ("react", &["react.js", "reactjs"]),
    ("react.js", &["react", "reactjs"]),
    ("vue.js", &["vue", "vuejs"]),
    ("machine learning", &["ml"]),
    ("artificial intelligence", &["ai"]),
    ("rest", &["restful", "rest api", "rest apis"]),
    ("restful apis", &["rest", "rest api", "rest apis"]),
    ("microservices", &["microservice", "micro-services", "microservice architecture"]),
    ("apache kafka", &["kafka"]),
    ("kafka", &["apache kafka"]),
    ("spring boot", &["springboot", "spring"]),
    ("c#", &["csharp", "c sharp"]),
    (".net", &["dotnet", "asp.net"]),
    ("mongodb", &["mongo"]),
    ("elasticsearch", &["elastic search", "elk", "opensearch"]),
    ("object-oriented programming", &["oop", "object oriented"]),
    ("data structures and algorithms", &["dsa", "data structures", "algorithms"]),
    ("sql", &["mysql", "postgresql", "postgres", "t-sql", "pl/sql"]),
    ("nosql", &["mongodb", "cassandra", "dynamodb"]),
    ("docker", &["containers", "containerization"]),
    ("terraform", &["iac", "infrastructure as code"]),
    ("message queues", &["kafka", "rabbitmq", "sqs", "message queue"]),
];

impl JobKeywords {
    /// Trims, dedupes (case-insensitive, first list wins) and caps every list. Model output is data.
    pub fn sanitize(mut self) -> Self {
        let clip = |s: &str| s.trim().chars().take(MAX_TERM_CHARS).collect::<String>();
        self.company = clip(&self.company);
        self.role = clip(&self.role);
        self.seniority = clip(&self.seniority).to_lowercase();
        let mut seen = std::collections::HashSet::new();
        for list in [&mut self.required_skills, &mut self.preferred_skills, &mut self.keywords] {
            let items = std::mem::take(list);
            for t in items.iter().map(|t| clip(t)).filter(|t| !t.is_empty()) {
                if list.len() < MAX_TERMS && seen.insert(t.to_lowercase()) {
                    list.push(t);
                }
            }
        }
        self.aliases = std::mem::take(&mut self.aliases)
            .into_iter()
            .take(60)
            .map(|(k, v)| (clip(&k), v.iter().map(|a| clip(a)).filter(|a| !a.is_empty()).take(6).collect::<Vec<_>>()))
            .filter(|(k, v)| !k.is_empty() && !v.is_empty())
            .collect();
        self.experience_years = self.experience_years.filter(|y| y.is_finite() && (0.0..=40.0).contains(y));
        self
    }

    /// Every term, posting order: required, preferred, then keywords. `true` = required.
    pub fn terms(&self) -> Vec<(String, bool)> {
        let req = self.required_skills.iter().map(|t| (t.clone(), true));
        let rest = self.preferred_skills.iter().chain(&self.keywords).map(|t| (t.clone(), false));
        req.chain(rest).collect()
    }

    /// Alternative spellings for a term: the posting-specific ones first, then the built-ins.
    pub fn aliases_for(&self, term: &str) -> Vec<String> {
        let low = term.to_lowercase();
        let mut out: Vec<String> = self.aliases.iter().filter(|(k, _)| k.to_lowercase() == low).flat_map(|(_, v)| v.clone()).collect();
        if let Some((_, v)) = BUILTIN_ALIASES.iter().find(|(k, _)| *k == low) {
            out.extend(v.iter().map(|s| s.to_string()));
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|a| a.to_lowercase() != low && seen.insert(a.to_lowercase()));
        out
    }

    pub fn is_empty(&self) -> bool {
        self.required_skills.is_empty() && self.preferred_skills.is_empty() && self.keywords.is_empty() && self.role.is_empty()
    }
}

fn cache_key(jd: &str) -> String {
    let norm = jd.split_whitespace().collect::<Vec<_>>().join(" ");
    let hash = Sha256::digest(format!("{CACHE_VERSION}\n{norm}").as_bytes());
    let hex: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    format!("ats_job:{hex}")
}

/// Keywords for this posting, from the cache or from one Claude call (no tools).
pub async fn keywords(state: &AppState, jd: &str) -> Result<JobKeywords> {
    let key = cache_key(jd);
    if let Ok(Some(hit)) = state.store.lock().unwrap().get_setting::<JobKeywords>(&key) {
        return Ok(hit);
    }
    let template = prompts::load(&state.prompts_dir, "ats_job.md")?;
    // Long postings carry boilerplate (benefits, EEO); the first 12k chars hold the requirements.
    let jd_clip: String = jd.chars().take(12_000).collect();
    let prompt = prompts::render(&template, &[("jd", jd_clip)].into());
    let backend = state.backend().await?;
    let model = state.settings().interviewer_model;
    let out = run_once(&backend, &prompt, Tools::None, &model, |_| {}).await?;
    let json = out.json.ok_or_else(|| AppError::Ai("Couldn't read the job keywords. Try again.".into()))?;
    let job: JobKeywords = serde_json::from_value::<JobKeywords>(json)?.sanitize();
    if job.is_empty() {
        return Err(AppError::Invalid("No skills or keywords found in that job description.".into()));
    }
    tracing::info!(ms = out.duration_ms, terms = job.terms().len(), "job keywords extracted");
    state.store.lock().unwrap().set_setting(&key, &job)?;
    Ok(job)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitize_dedupes_and_caps() {
        let raw = json!({
            "role": "  Senior Backend Engineer ",
            "required_skills": ["Java", "Kafka", " ", "java"],
            "preferred_skills": ["Kafka", "Redis"],
            "keywords": ["payments"],
            "aliases": {"Kafka": ["Apache Kafka"], "Java": []},
            "experience_years": 99,
            "extra": "ignored"
        });
        let j: JobKeywords = serde_json::from_value::<JobKeywords>(raw).unwrap().sanitize();
        assert_eq!(j.role, "Senior Backend Engineer");
        assert_eq!(j.required_skills, ["Java", "Kafka"]);
        assert_eq!(j.preferred_skills, ["Redis"]);
        assert_eq!(j.aliases.len(), 1);
        assert_eq!(j.experience_years, None);
        assert_eq!(j.terms().first(), Some(&("Java".to_string(), true)));
    }

    #[test]
    fn aliases_merge_builtin() {
        let j = JobKeywords { aliases: [("PostgreSQL".into(), vec!["PG".into(), "postgres".into()])].into(), ..Default::default() };
        assert_eq!(j.aliases_for("PostgreSQL"), ["PG", "postgres", "psql"]);
        assert!(j.aliases_for("Rust").is_empty());
    }

    #[test]
    fn cache_key_ignores_whitespace() {
        assert_eq!(cache_key("Java  dev\n\nKafka"), cache_key("Java dev Kafka"));
        assert_ne!(cache_key("Java dev"), cache_key("Go dev"));
    }

    #[test]
    fn prompt_renders() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts");
        let t = prompts::load(&dir, "ats_job.md").unwrap();
        let p = prompts::render(&t, &[("jd", "Backend role".to_string())].into());
        assert!(p.contains("<<<\nBackend role\n>>>") && !p.contains("{{"));
    }
}
