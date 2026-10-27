//! Company dossiers: how one company interviews for one role, researched from candidate reports.
//! Research output is model text, so it is checked and normalised here before it is saved
//! (rules from `docs/dossier.schema.json`). Dossiers are data: only shown and put in prompts.

use crate::brain::prompts::{render, Vars};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Dossier {
    #[serde(default)]
    pub id: String,
    pub company: String,
    pub role: String,
    #[serde(default)]
    pub level: String,
    #[serde(default)]
    pub jd: String,
    #[serde(default, rename = "preparedAt")]
    pub prepared_at: String,
    pub summary: String,
    pub style: String,
    #[serde(default)]
    pub values: String,
    #[serde(default)]
    pub pitfalls: Vec<String>,
    pub process: Vec<DossierRound>,
    #[serde(default)]
    pub sources: Vec<Source>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DossierRound {
    /// A catalog round key.
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub minutes: u32,
    #[serde(default)]
    pub what: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub questions: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Source {
    #[serde(default)]
    pub title: String,
    pub url: String,
}

/// "Razorpay", "SDE 2, backend", "sde2" → "razorpay-sde-2-backend-sde2" (level dropped when the role already says it).
pub fn slug(company: &str, role: &str, level: &str) -> String {
    let words = |s: &str| s.to_lowercase().split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect::<Vec<_>>();
    let mut parts = words(company);
    parts.extend(words(role));
    let lvl = words(level).concat();
    if !lvl.is_empty() && !parts.concat().contains(&lvl) {
        parts.push(lvl);
    }
    let s = parts.join("-");
    if s.is_empty() { "dossier".into() } else { s.chars().take(80).collect() }
}

/// Maps a round type the model wrote to a catalog key; unknown kinds fall back by keyword.
fn round_kind(raw: &str, name: &str, known: &[String]) -> String {
    let t = raw.trim().to_lowercase();
    if known.contains(&t) {
        return t;
    }
    let hay = format!("{t} {}", name.to_lowercase());
    let has = |ws: &[&str]| ws.iter().any(|w| hay.contains(w));
    let k = if has(&["machine coding", "low level", "low-level", "lld", "object oriented", "ood"]) {
        "lld"
    } else if has(&["system design", "high level", "high-level", "hld", "architecture"]) {
        "sysdesign"
    } else if has(&["dsa", "coding", "algorithm", "problem solving", "data structure", "leetcode"]) {
        "coding"
    } else if has(&["code review", "debug", "comprehension", "read code"]) {
        "comprehension"
    } else if has(&["behaviour", "behavior", "culture", "values", "leadership principle", "bar raiser"]) {
        "behavioral"
    } else if has(&["resume", "screen", "recruiter", "introduct"]) {
        "resume"
    } else if has(&["project", "deep dive", "past work"]) {
        "project"
    } else if has(&["fundamental", "core cs", "technical round", "java", "database", "os "]) {
        "fundamentals"
    } else {
        "hm"
    };
    if known.iter().any(|x| x == k) { k.to_string() } else { known.first().cloned().unwrap_or_else(|| k.to_string()) }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Array(a) => a.iter().map(text).filter(|s| !s.is_empty()).collect::<Vec<_>>().join("; "),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn strings(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().map(text).filter(|s| !s.is_empty()).collect(),
        Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => Vec::new(),
    }
}

fn minutes(v: &Value) -> u32 {
    let n = v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().split(|c: char| !c.is_ascii_digit() && c != '.').next()?.parse().ok()));
    n.map_or(45, |m| m.round().clamp(10.0, 180.0) as u32)
}

impl Dossier {
    /// Checks and tidies research output or an imported file. `known` = catalog round keys.
    /// Errors are short, plain sentences.
    pub fn from_value(v: &Value, known: &[String]) -> Result<Self, String> {
        if !v.is_object() {
            return Err("It isn't a JSON object.".into());
        }
        let mut missing = Vec::new();
        let field = |k: &str, missing: &mut Vec<&'static str>, name: &'static str| {
            let s = text(&v[k]);
            if s.is_empty() {
                missing.push(name);
            }
            s
        };
        let company = field("company", &mut missing, "company");
        let role = field("role", &mut missing, "role");
        let summary = field("summary", &mut missing, "summary");
        let style = field("style", &mut missing, "style");
        let process: Vec<DossierRound> = v["process"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| r.is_object())
            .map(|r| {
                let name = text(&r["name"]);
                let kind = round_kind(&text(&r["type"]), &name, known);
                DossierRound {
                    name: if name.is_empty() { kind.clone() } else { name },
                    kind,
                    minutes: minutes(&r["minutes"]),
                    what: text(&r["what"]),
                    style: text(&r["style"]),
                    questions: strings(&r["questions"]),
                }
            })
            .collect();
        if process.is_empty() {
            missing.push("rounds");
        }
        if !missing.is_empty() {
            return Err(format!("It has no {}.", missing.join(", ")));
        }
        let sources = v["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| match s {
                Value::String(u) => Some(Source { title: String::new(), url: u.clone() }),
                _ => Some(Source { title: text(&s["title"]), url: text(&s["url"]) }).filter(|s| !s.url.is_empty()),
            })
            .collect();
        let level = text(&v["level"]);
        let mut d = Dossier {
            id: text(&v["id"]),
            level,
            jd: text(&v["jd"]),
            prepared_at: text(&v["preparedAt"]),
            values: text(&v["values"]),
            pitfalls: strings(&v["pitfalls"]),
            company,
            role,
            summary,
            style,
            process,
            sources,
        };
        if d.id.is_empty() {
            d.id = slug(&d.company, &d.role, &d.level);
        }
        if d.prepared_at.is_empty() {
            d.prepared_at = chrono::Utc::now().to_rfc3339();
        }
        Ok(d)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// The `{{dossier_block}}` for round `idx`, from `prompts/dossier_block.md`.
    pub fn prompt_block(&self, template: &str, idx: usize) -> String {
        let Some(r) = self.process.get(idx) else { return String::new() };
        let questions = if r.questions.is_empty() {
            "- (none reported; use the round description)".to_string()
        } else {
            r.questions.iter().map(|q| format!("- {q}")).collect::<Vec<_>>().join("\n")
        };
        let or_none = |s: &str| if s.trim().is_empty() { "(not reported)".to_string() } else { s.to_string() };
        let vars: Vars = [
            ("summary", self.summary.clone()),
            ("style", self.style.clone()),
            ("values", or_none(&self.values)),
            ("round_name", r.name.clone()),
            ("round_what", r.what.clone()),
            ("round_style", or_none(&r.style)),
            ("round_questions_bulleted", questions),
            ("pitfalls_joined", if self.pitfalls.is_empty() { "(none reported)".into() } else { self.pitfalls.join("; ") }),
        ]
        .into();
        format!("\n{}\n", render(template, &vars).trim())
    }

    /// The same dossier with only the chosen rounds, in their order. Bad indexes are skipped.
    pub fn with_rounds(&self, picks: &[usize]) -> Dossier {
        let mut d = self.clone();
        d.process = picks.iter().filter_map(|&i| self.process.get(i).cloned()).collect();
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::Path;

    fn known() -> Vec<String> {
        ["resume", "project", "fundamentals", "coding", "comprehension", "lld", "sysdesign", "behavioral", "hm"].map(String::from).to_vec()
    }

    #[test]
    fn seed_dossier_loads_as_is() {
        let raw = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../dossiers/razorpay-sde-2-backend.json")).unwrap();
        let v: Value = serde_json::from_str(&raw).unwrap();
        let d = Dossier::from_value(&v, &known()).unwrap();
        assert_eq!(d.id, "razorpay-sde-2-backend");
        assert_eq!(d.process.iter().map(|r| r.kind.as_str()).collect::<Vec<_>>(), ["coding", "lld", "sysdesign", "hm"]);
        assert_eq!(d.process[1].minutes, 90);
        assert!(!d.sources.is_empty());
    }

    #[test]
    fn model_output_is_tidied() {
        let v = json!({
            "company": "Acme", "role": "Backend SDE", "level": "sde2", "summary": "Three rounds.", "style": ["Friendly", "deep"],
            "process": [
                {"type": "Machine Coding", "name": "MC round", "minutes": "90 min", "questions": "Build a parking lot"},
                {"type": "dsa", "minutes": 400},
                {"type": "weird", "name": "Chat with the director", "minutes": 30.4}
            ],
            "sources": ["https://a.example", {"title": "B", "url": "https://b.example"}, {"title": "no url"}]
        });
        let d = Dossier::from_value(&v, &known()).unwrap();
        assert_eq!(d.id, "acme-backend-sde-sde2");
        assert_eq!(d.style, "Friendly; deep");
        let kinds: Vec<_> = d.process.iter().map(|r| (r.kind.as_str(), r.minutes)).collect();
        assert_eq!(kinds, [("lld", 90), ("coding", 180), ("hm", 30)]);
        assert_eq!(d.process[0].questions, ["Build a parking lot"]);
        assert_eq!(d.process[1].name, "coding");
        assert_eq!(d.sources.len(), 2);
    }

    #[test]
    fn missing_fields_are_named() {
        let e = Dossier::from_value(&json!({"company": "Acme", "process": []}), &known()).unwrap_err();
        assert_eq!(e, "It has no role, summary, style, rounds.");
        assert!(Dossier::from_value(&json!([1]), &known()).is_err());
    }

    #[test]
    fn slug_skips_level_already_in_role() {
        assert_eq!(slug("Razorpay", "SDE 2, backend", "sde2"), "razorpay-sde-2-backend");
        assert_eq!(slug("Stripe", "Backend Engineer", "senior"), "stripe-backend-engineer-senior");
    }

    #[test]
    fn prompt_block_for_a_round() {
        let tpl = crate::brain::prompts::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts"), "dossier_block.md").unwrap();
        let d = Dossier {
            company: "Acme".into(),
            summary: "S".into(),
            style: "St".into(),
            process: vec![DossierRound { kind: "lld".into(), name: "Machine coding".into(), minutes: 90, questions: vec!["Parking lot".into()], ..Default::default() }],
            ..Default::default()
        };
        let b = d.prompt_block(&tpl, 0);
        assert!(b.contains("THIS ROUND as reported by candidates: Machine coding.") && b.contains("- Parking lot"), "{b}");
        assert!(!b.contains("{{"));
        assert_eq!(d.prompt_block(&tpl, 5), "");
        assert_eq!(d.with_rounds(&[0, 7]).process.len(), 1);
    }
}
