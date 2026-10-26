//! Parses one stdout line of `claude -p --output-format stream-json` into events we care about.
//! Unknown shapes become `Other` so a CLI upgrade never kills a round.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Init { session_id: String, model: String, tools: Vec<String> },
    TextDelta(String),
    /// From full `assistant` messages only, so each tool call is counted once.
    ToolUse { name: String },
    AssistantText(String),
    Result { text: String, session_id: String, is_error: bool, duration_ms: u64 },
    Other,
}

fn s(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

pub fn parse_line(line: &str) -> Result<Vec<StreamEvent>, serde_json::Error> {
    let v: Value = serde_json::from_str(line)?;
    Ok(from_value(&v))
}

pub fn from_value(v: &Value) -> Vec<StreamEvent> {
    match v.get("type").and_then(Value::as_str) {
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            let tools = v
                .get("tools")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|t| t.as_str().map(String::from)).collect())
                .unwrap_or_default();
            vec![StreamEvent::Init { session_id: s(v, "session_id"), model: s(v, "model"), tools }]
        }
        Some("stream_event") => {
            let ev = &v["event"];
            if ev["type"] == "content_block_delta" && ev["delta"]["type"] == "text_delta" {
                vec![StreamEvent::TextDelta(s(&ev["delta"], "text"))]
            } else {
                vec![StreamEvent::Other]
            }
        }
        Some("assistant") => {
            let mut out = Vec::new();
            let mut text = String::new();
            for block in v["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str() {
                    Some("text") => text.push_str(block["text"].as_str().unwrap_or_default()),
                    Some("tool_use") | Some("server_tool_use") => {
                        out.push(StreamEvent::ToolUse { name: s(block, "name") })
                    }
                    _ => {}
                }
            }
            if !text.is_empty() {
                out.insert(0, StreamEvent::AssistantText(text));
            }
            if out.is_empty() {
                out.push(StreamEvent::Other);
            }
            out
        }
        Some("result") => {
            let is_error = v["is_error"].as_bool().unwrap_or(false)
                || v.get("subtype").and_then(Value::as_str).is_some_and(|st| st != "success");
            vec![StreamEvent::Result {
                text: s(v, "result"),
                session_id: s(v, "session_id"),
                is_error,
                duration_ms: v["duration_ms"].as_u64().unwrap_or(0),
            }]
        }
        _ => vec![StreamEvent::Other],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/turn_code_end.jsonl");

    fn all() -> Vec<StreamEvent> {
        FIXTURE.lines().flat_map(|l| parse_line(l).unwrap()).collect()
    }

    #[test]
    fn init_reports_no_tools() {
        let evs = all();
        let StreamEvent::Init { tools, model, session_id } = &evs[0] else { panic!("first event not init") };
        assert!(tools.is_empty());
        assert_eq!(model, "claude-sonnet-5-5");
        assert!(!session_id.is_empty());
    }

    #[test]
    fn deltas_join_to_result_text() {
        let evs = all();
        let joined: String = evs
            .iter()
            .filter_map(|e| if let StreamEvent::TextDelta(t) = e { Some(t.as_str()) } else { None })
            .collect();
        let result = evs.iter().find_map(|e| match e {
            StreamEvent::Result { text, is_error, .. } => Some((text.clone(), *is_error)),
            _ => None,
        });
        assert_eq!(result, Some((joined, false)));
    }

    #[test]
    fn counts_tool_use_in_assistant_message() {
        let line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"PROGRESS stage=2"},{"type":"server_tool_use","name":"WebSearch"}]}}"#;
        assert_eq!(
            parse_line(line).unwrap(),
            vec![
                StreamEvent::AssistantText("PROGRESS stage=2".into()),
                StreamEvent::ToolUse { name: "WebSearch".into() }
            ]
        );
    }

    #[test]
    fn error_result_and_unknown_types() {
        let line = r#"{"type":"result","subtype":"error_during_execution","session_id":"x"}"#;
        assert!(matches!(&parse_line(line).unwrap()[0], StreamEvent::Result { is_error: true, .. }));
        assert_eq!(parse_line(r#"{"type":"rate_limit_event"}"#).unwrap(), vec![StreamEvent::Other]);
        assert!(parse_line("not json").is_err());
    }
}
