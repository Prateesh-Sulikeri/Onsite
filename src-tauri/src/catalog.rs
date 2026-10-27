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
    pub temper: IndexMap<String, String>,
}

impl Catalog {
    pub fn load(prompts_dir: &Path) -> crate::error::Result<Self> {
        let text = std::fs::read_to_string(prompts_dir.join("catalog.json"))?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Same rotation as the artifact, so personas stay stable per round index.
    pub fn persona(&self, round_index: usize) -> &str {
        &self.names[(round_index * 5 + 3) % self.names.len()]
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
        assert!(!c.persona(0).is_empty());
        assert!(c.rounds["comprehension"].openers.len() > 10);
    }
}
