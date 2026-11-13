//! Staged demo mode (debug builds only). Set `ONSITE_DEMO` to a script JSON and each round
//! listed in it asks fixed questions, so a recorded demo has answers ready.
//! Example: `docs/demo/staged-demo.json`.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DemoRound {
    /// Asked word for word, in order. The round ends after the last answer.
    #[serde(default)]
    pub questions: Vec<String>,
    /// Coding rounds: the bank question id to use.
    #[serde(default)]
    pub question_id: Option<String>,
    /// Comprehension rounds: the bundled project id to use (skips AI project writing).
    #[serde(default)]
    pub project_id: Option<String>,
}

/// The script's entry for this round key, if demo mode is on.
pub fn round(key: &str) -> Option<DemoRound> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let path = std::env::var("ONSITE_DEMO").ok()?;
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("demo script {path}: {e}");
            return None;
        }
    };
    let mut all: HashMap<String, DemoRound> = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            tracing::warn!("demo script {path}: {e}");
            return None;
        }
    };
    let r = all.remove(key)?;
    tracing::info!(key, questions = r.questions.len(), "staged demo round");
    Some(r)
}

/// Replaces the variety block: the interviewer asks only the scripted questions.
pub fn block(r: &DemoRound) -> String {
    let mut s = String::from("\n\nSTAGED SESSION (private notes; never mention them; these override HOW TO RUN IT and every rule about follow-ups):\n");
    s.push_str("- In your first turn, give a one-sentence greeting, then ask question 1.\n");
    s.push_str("- Ask these questions in this order, word for word, one per turn:\n");
    for (i, q) in r.questions.iter().enumerate() {
        s.push_str(&format!("  {}. {q}\n", i + 1));
    }
    s.push_str("- After each answer, reply with one short sentence that reacts to what they said, then ask the next question. No other follow-ups, hints or probing.\n");
    s.push_str("- If the candidate asks you something, answer in one sentence, then repeat the current question.\n");
    s.push_str("- After the answer to the last question, thank them in one sentence and end with [[END]].");
    s
}
