//! Interviewer panels from `prompts/panels.json`: fixed personas so the same people
//! interview the candidate every session.

use crate::brain::prompts::Vars;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// Role family used for focus areas until setup lets the user pick one.
pub const DEFAULT_FAMILY: &str = "software_engineering";

#[derive(Debug, Clone, Default)]
pub struct Panels {
    raw: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct PanelSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub interviewers: Vec<InterviewerSummary>,
    /// Round key → interviewer id.
    pub round_owner: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct InterviewerSummary {
    pub id: String,
    pub name: String,
    pub role: String,
}

/// One interviewer, as the round needs them.
#[derive(Debug, Clone, PartialEq)]
pub struct Persona {
    pub id: String,
    pub name: String,
    pub first_name: String,
    pub role: String,
    pub panel_name: String,
    /// VoiceStudio voice-design tags, e.g. "male, middle-aged, low pitch, indian accent".
    pub voice_instruct: String,
    pub voice_states: Value,
    /// Multiplies the user's speaking speed: brisk people talk a little faster.
    pub speed_factor: f32,
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or_default().to_string()
}

fn joined(v: &Value) -> String {
    v.as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "))
        .unwrap_or_else(|| s(v))
}

impl Panels {
    /// Missing or broken panels.json just means no panels (classic personas).
    pub fn load(prompts_dir: &Path) -> Self {
        let path = prompts_dir.join("panels.json");
        match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
            Ok(raw) => Self { raw },
            Err(e) => {
                tracing::warn!(path = %path.display(), "no interviewer panels: {e}");
                Self::default()
            }
        }
    }

    fn panel(&self, id: &str) -> Option<&Value> {
        self.raw["panels"].as_array()?.iter().find(|p| p["id"] == id)
    }

    pub fn list(&self) -> Vec<PanelSummary> {
        self.raw["panels"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| PanelSummary {
                id: s(&p["id"]),
                name: s(&p["name"]),
                description: s(&p["description"]),
                interviewers: p["interviewers"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|i| InterviewerSummary { id: s(&i["id"]), name: s(&i["name"]), role: s(&i["role"]) })
                    .collect(),
                round_owner: p["round_owner"].clone(),
            })
            .collect()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.panel(id).is_some()
    }

    /// The interviewer who owns this round type on this panel.
    fn interviewer(&self, panel_id: &str, round_key: &str) -> Option<(&Value, &Value)> {
        let panel = self.panel(panel_id)?;
        let owner = panel["round_owner"][round_key].as_str()?;
        let iv = panel["interviewers"].as_array()?.iter().find(|i| i["id"] == owner)?;
        Some((panel, iv))
    }

    pub fn persona(&self, panel_id: &str, round_key: &str) -> Option<Persona> {
        let (panel, iv) = self.interviewer(panel_id, round_key)?;
        let name = s(&iv["name"]);
        let (instruct, states) = voice_design(&iv["voice"]);
        Some(Persona {
            id: s(&iv["id"]),
            first_name: name.split_whitespace().next().unwrap_or(&name).to_string(),
            name,
            role: s(&iv["role"]),
            panel_name: s(&panel["name"]),
            voice_instruct: instruct,
            voice_states: states,
            speed_factor: match s(&iv["voice"]["pace"]).as_str() {
                "brisk" => 1.05,
                "relaxed" | "measured" => 0.96,
                _ => 1.0,
            },
        })
    }

    /// Values for `prompts/persona_block.md`.
    pub fn persona_vars(&self, panel_id: &str, round_key: &str, family: &str) -> Option<Vars<'static>> {
        let (panel, iv) = self.interviewer(panel_id, round_key)?;
        let sp = &iv["speech"];
        let specs = &iv["role_specializations"];
        // Recruiters and leaders list "all_roles" / "engineering" instead of a family.
        let focus = [family, "engineering", "all_roles"]
            .iter()
            .map(|k| &specs[*k])
            .find(|v| v.is_array())
            .map(joined)
            .unwrap_or_else(|| joined(&iv["interview_modes"]).replace('_', " "));
        let p = &iv["personality"];
        Some(
            [
                ("name", s(&iv["name"])),
                ("role", s(&iv["role"])),
                ("panel_name", s(&panel["name"])),
                ("backstory", s(&iv["backstory"])),
                ("traits", joined(&p["traits"])),
                ("communication_style", s(&p["communication_style"])),
                ("strictness", p["strictness"].as_i64().unwrap_or(3).to_string()),
                ("strictness_behavior", s(&iv["strictness_behavior"])),
                ("opening_line", s(&sp["opening_line"])),
                ("acknowledgements", joined(&sp["acknowledgements"])),
                ("probes", joined(&sp["probes"])),
                ("nudges", joined(&sp["nudges"])),
                ("transition_phrases", joined(&sp["transition_phrases"])),
                ("closing_line", s(&sp["closing_line"])),
                ("verbal_tics", joined(&sp["verbal_tics"])),
                ("never", joined(&iv["never"])),
                ("evaluates", joined(&iv["evaluates"])),
                ("focus_areas", focus),
            ]
            .into(),
        )
    }
}

const ACCENTS: [&str; 10] =
    ["american", "british", "australian", "chinese", "canadian", "indian", "korean", "portuguese", "russian", "japanese"];

/// Maps a persona's free-text voice onto VoiceStudio's design whitelist
/// (one tag per category: gender, age, pitch, English accent).
pub fn voice_design(voice: &Value) -> (String, Value) {
    let low = |k: &str| s(&voice[k]).to_lowercase();
    let gender = match low("gender").as_str() {
        g @ ("male" | "female") => g.to_string(),
        _ => String::new(),
    };
    let age = match low("age").as_str() {
        a @ ("child" | "teenager" | "young adult" | "middle-aged" | "elderly") => a.to_string(),
        _ => String::new(),
    };
    let pitch = match low("pitch").as_str() {
        p @ ("very low" | "low" | "moderate" | "high" | "very high") => format!("{p} pitch"),
        _ => String::new(),
    };
    let accent_text = low("accent");
    let accent = ACCENTS.iter().find(|a| accent_text.contains(*a)).map(|a| format!("{a} accent")).unwrap_or_default();
    let instruct = [&gender, &age, &pitch, &accent].iter().filter(|t| !t.is_empty()).map(|t| t.as_str()).collect::<Vec<_>>().join(", ");
    let auto = |t: &str| if t.is_empty() { "Auto".to_string() } else { t.to_string() };
    let states = serde_json::json!({
        "Gender": auto(&gender), "Age": auto(&age), "Pitch": auto(&pitch),
        "Style": "Auto", "EnglishAccent": auto(&accent), "ChineseDialect": "Auto",
    });
    (instruct, states)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn repo() -> Panels {
        Panels::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts"))
    }

    #[test]
    fn every_round_has_an_owner_with_full_persona() {
        let p = repo();
        let cat = crate::catalog::Catalog::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts")).unwrap();
        assert_eq!(p.list().len(), 2);
        for panel in p.list() {
            for key in cat.rounds.keys() {
                let persona = p.persona(&panel.id, key).unwrap_or_else(|| panel_missing(&panel.id, key));
                assert!(!persona.voice_instruct.is_empty(), "{} has no voice", persona.id);
                let vars = p.persona_vars(&panel.id, key, DEFAULT_FAMILY).unwrap();
                for (k, v) in &vars {
                    assert!(!v.is_empty(), "{}.{k} empty", persona.id);
                }
            }
        }
    }

    fn panel_missing(panel: &str, key: &str) -> Persona {
        panic!("{panel} has no owner for {key}")
    }

    #[test]
    fn persona_block_renders_without_placeholders() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts");
        let tpl = crate::brain::prompts::load(&dir, "persona_block.md").unwrap();
        let vars = repo().persona_vars("indian_techies", "coding", DEFAULT_FAMILY).unwrap();
        let out = crate::brain::prompts::render(&tpl, &vars);
        assert!(!out.contains("{{"), "{out}");
        assert!(out.contains("Karthik Reddy"));
    }

    #[test]
    fn voice_maps_to_design_whitelist() {
        let (i, st) = voice_design(&json!({"gender":"Male","age":"young adult","accent":"indian accent (south indian English)","pitch":"moderate","pace":"brisk"}));
        assert_eq!(i, "male, young adult, moderate pitch, indian accent");
        assert_eq!(st["EnglishAccent"], "indian accent");
        let (i, st) = voice_design(&json!({"gender":"female","accent":"Mumbai English"}));
        assert_eq!(i, "female");
        assert_eq!(st["Age"], "Auto");
    }

    #[test]
    fn missing_file_is_empty() {
        let p = Panels::load(Path::new("/definitely/not/here"));
        assert!(p.list().is_empty());
        assert!(p.persona("indian_techies", "coding").is_none());
    }
}
