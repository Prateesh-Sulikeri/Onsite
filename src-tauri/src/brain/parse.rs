//! Tolerant JSON extraction from model output: whole text, else a fenced block, else `{` .. `}`.

use serde_json::Value;

pub fn extract_json(text: &str) -> Option<Value> {
    let t = text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return Some(v);
    }
    for body in fenced_bodies(t) {
        if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
            return Some(v);
        }
    }
    let (start, end) = (t.find('{')?, t.rfind('}')?);
    if end <= start {
        return None;
    }
    serde_json::from_str(&t[start..=end]).ok()
}

fn fenced_bodies(t: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(open) = rest.find("```") {
        let after = &rest[open + 3..];
        let Some(nl) = after.find('\n') else { break };
        let body = &after[nl + 1..];
        let Some(close) = body.find("```") else { break };
        out.push(&body[..close]);
        rest = &body[close + 3..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whole_text() {
        assert_eq!(extract_json(" {\"a\":1} "), Some(json!({"a":1})));
    }

    #[test]
    fn fenced_block_wins_over_braces_in_prose() {
        let t = "Here {is} the report:\n```json\n{\"overall\": 70}\n```\nDone.";
        assert_eq!(extract_json(t), Some(json!({"overall": 70})));
    }

    #[test]
    fn first_to_last_brace() {
        let t = "Sure. {\"decision\":\"Hire\",\"plan\":[\"x\"]} hope this helps";
        assert_eq!(extract_json(t), Some(json!({"decision":"Hire","plan":["x"]})));
    }

    #[test]
    fn garbage_is_none() {
        assert_eq!(extract_json("no json here"), None);
        assert_eq!(extract_json("} backwards {"), None);
    }
}
