//! Typed view of `prompts/catalog.json`. Key order is kept for the UI pickers.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoundKind {
    pub name: String,
    pub min: u32,
    pub tab: String,
    pub blurb: String,
    pub guide: String,
    pub rubric: Vec<String>,
    /// Starting points; one is drawn per session so the round doesn't open the same way each time.
    #[serde(default)]
    pub openers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyStyle {
    pub name: String,
    pub style: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub struct Catalog {
    pub rounds: IndexMap<String, RoundKind>,
    pub companies: IndexMap<String, CompanyStyle>,
    pub levels: IndexMap<String, String>,
    pub level_bar: IndexMap<String, String>,
    pub names: Vec<String>,
    /// The women's names in `names`, so a round's name can match its voice.
    #[serde(default)]
    pub female_names: Vec<String>,
    pub temper: IndexMap<String, String>,
}

impl Catalog {
    pub fn load(prompts_dir: &Path) -> crate::error::Result<Self> {
        let text = std::fs::read_to_string(prompts_dir.join("catalog.json"))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Same rotation as the artifact, so personas stay stable per round index.
    pub fn round_keys(&self) -> Vec<String> {
        self.rounds.keys().cloned().collect()
    }

    /// The interviewer's name for a round without a panel. `female` = the voice's gender, when known:
    /// the name is then picked from that gender's names, so "Neha" never speaks in a man's voice.
    pub fn persona(&self, round_index: usize, female: Option<bool>) -> &str {
        let pool: Vec<&String> = self.names.iter().filter(|n| female.is_none_or(|f| self.female_names.contains(n) == f)).collect();
        let pool = if pool.is_empty() { self.names.iter().collect() } else { pool };
        pool[(round_index * 5 + 3) % pool.len()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_repo_catalog() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts");
        let c = Catalog::load(&dir).unwrap();
        assert_eq!(c.rounds.get_index(0).unwrap().0, "resume");
        assert_eq!(c.rounds["coding"].tab, "editor");
        assert!(c.levels.keys().eq(c.level_bar.keys()));
        assert!(!c.persona(0, None).is_empty());
        assert!(c.female_names.iter().all(|n| c.names.contains(n)), "FEMALE_NAMES must be in NAMES");
        assert!(c.rounds["comprehension"].openers.len() > 10);
    }
}
