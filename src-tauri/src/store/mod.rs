//! SQLite storage. Every write is immediate, so reports and transcripts survive restarts.

use crate::error::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

const SCHEMA_VERSION: i32 = 1;

pub struct Store {
    conn: Connection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Turn {
    /// "iv" (interviewer) or "me" (candidate).
    pub who: String,
    pub text: String,
    /// Minutes since round start.
    pub at_min: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttft_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shared_workspace: bool,
    /// Exact message sent to the interviewer (with workspace and elapsed tags), when it differs from `text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoundRow {
    pub id: i64,
    pub loop_id: i64,
    pub idx: i64,
    pub round_key: String,
    pub name: String,
    pub persona: String,
    pub minutes: i64,
    pub model: String,
    pub claude_session_id: Option<String>,
    pub transcript: Vec<Turn>,
    pub workspace: Value,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryItem {
    pub round_id: i64,
    pub loop_id: i64,
    pub name: String,
    pub company: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub overall: Option<i64>,
    pub decision: Option<String>,
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let v: i32 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if v < SCHEMA_VERSION {
            conn.execute_batch(include_str!("schema.sql"))?;
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(Self { conn })
    }

    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        let raw: Option<String> =
            self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        Ok(match raw {
            Some(s) => Some(serde_json::from_str(&s)?),
            None => None,
        })
    }

    pub fn set_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, serde_json::to_string(value)?],
        )?;
        Ok(())
    }

    pub fn create_loop(&self, config: &Value) -> Result<i64> {
        self.conn.execute("INSERT INTO loops (config, created_at) VALUES (?1, ?2)", params![config.to_string(), now()])?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn loop_config(&self, loop_id: i64) -> Result<Value> {
        let raw: String = self.conn.query_row("SELECT config FROM loops WHERE id = ?1", [loop_id], |r| r.get(0))?;
        Ok(serde_json::from_str(&raw)?)
    }

    pub fn set_loop_verdict(&self, loop_id: i64, verdict: &Value) -> Result<()> {
        self.conn.execute("UPDATE loops SET verdict = ?1 WHERE id = ?2", params![verdict.to_string(), loop_id])?;
        Ok(())
    }

    pub fn loop_verdict(&self, loop_id: i64) -> Result<Option<Value>> {
        let raw: Option<String> = self.conn.query_row("SELECT verdict FROM loops WHERE id = ?1", [loop_id], |r| r.get(0))?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_round(
        &self,
        loop_id: i64,
        idx: usize,
        round_key: &str,
        name: &str,
        persona: &str,
        minutes: u32,
        model: &str,
    ) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO rounds (loop_id, idx, round_key, name, persona, minutes, model, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![loop_id, idx as i64, round_key, name, persona, minutes, model, now()],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn save_round_progress(
        &self,
        round_id: i64,
        transcript: &[Turn],
        workspace: &Value,
        session_id: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE rounds SET transcript = ?1, workspace = ?2, claude_session_id = COALESCE(?3, claude_session_id)
             WHERE id = ?4",
            params![serde_json::to_string(transcript)?, workspace.to_string(), session_id, round_id],
        )?;
        Ok(())
    }

    pub fn finish_round(&self, round_id: i64) -> Result<()> {
        self.conn.execute("UPDATE rounds SET ended_at = COALESCE(ended_at, ?1) WHERE id = ?2", params![now(), round_id])?;
        Ok(())
    }

    pub fn round(&self, round_id: i64) -> Result<RoundRow> {
        let row = self.conn.query_row(
            "SELECT id, loop_id, idx, round_key, name, persona, minutes, model, claude_session_id,
                    transcript, workspace, started_at, ended_at FROM rounds WHERE id = ?1",
            [round_id],
            |r| {
                Ok((
                    RoundRow {
                        id: r.get(0)?,
                        loop_id: r.get(1)?,
                        idx: r.get(2)?,
                        round_key: r.get(3)?,
                        name: r.get(4)?,
                        persona: r.get(5)?,
                        minutes: r.get(6)?,
                        model: r.get(7)?,
                        claude_session_id: r.get(8)?,
                        transcript: Vec::new(),
                        workspace: Value::Null,
                        started_at: r.get(11)?,
                        ended_at: r.get(12)?,
                    },
                    r.get::<_, String>(9)?,
                    r.get::<_, String>(10)?,
                ))
            },
        )?;
        let (mut round, transcript, workspace) = row;
        round.transcript = serde_json::from_str(&transcript)?;
        round.workspace = serde_json::from_str(&workspace)?;
        Ok(round)
    }

    pub fn loop_round_ids(&self, loop_id: i64) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT id FROM rounds WHERE loop_id = ?1 ORDER BY idx, id")?;
        let ids = st.query_map([loop_id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    pub fn save_report(&self, round_id: i64, data: Option<&Value>, raw: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO reports (round_id, data, raw, created_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(round_id) DO UPDATE SET data = excluded.data, raw = excluded.raw, created_at = excluded.created_at",
            params![round_id, data.map(Value::to_string), raw, now()],
        )?;
        Ok(())
    }

    /// Returns (parsed report, raw text).
    pub fn report(&self, round_id: i64) -> Result<Option<(Option<Value>, String)>> {
        let row: Option<(Option<String>, String)> = self
            .conn
            .query_row("SELECT data, raw FROM reports WHERE round_id = ?1", [round_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(match row {
            Some((data, raw)) => Some((data.map(|d| serde_json::from_str(&d)).transpose()?, raw)),
            None => None,
        })
    }

    pub fn history(&self, limit: i64) -> Result<Vec<HistoryItem>> {
        let mut st = self.conn.prepare(
            "SELECT r.id, r.loop_id, r.name, l.config, r.started_at, r.ended_at, rep.data
             FROM rounds r JOIN loops l ON l.id = r.loop_id LEFT JOIN reports rep ON rep.round_id = r.id
             ORDER BY r.id DESC LIMIT ?1",
        )?;
        let rows = st.query_map([limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<String>>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (round_id, loop_id, name, config, started_at, ended_at, data) = row?;
            let config: Value = serde_json::from_str(&config).unwrap_or_default();
            let data: Option<Value> = data.and_then(|d| serde_json::from_str(&d).ok());
            out.push(HistoryItem {
                round_id,
                loop_id,
                name,
                company: config["company_name"].as_str().unwrap_or_default().to_string(),
                started_at,
                ended_at,
                overall: data.as_ref().and_then(|d| d["overall"].as_i64()),
                decision: data.as_ref().and_then(|d| d["decision"].as_str().map(String::from)),
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_lifecycle_and_history() {
        let s = Store::open_in_memory().unwrap();
        let loop_id = s.create_loop(&json!({"company_name": "Google"})).unwrap();
        let id = s.create_round(loop_id, 0, "coding", "DSA coding", "Rahul", 45, "sonnet").unwrap();
        let turns = vec![Turn { who: "iv".into(), text: "Hi.".into(), at_min: 0.0, ttft_ms: Some(900), shared_workspace: false, sent: None }];
        s.save_round_progress(id, &turns, &json!({"code": "x"}), Some("sess-1")).unwrap();
        s.save_round_progress(id, &turns, &json!({"code": "y"}), None).unwrap();
        let r = s.round(id).unwrap();
        assert_eq!(r.transcript, turns);
        assert_eq!(r.claude_session_id.as_deref(), Some("sess-1"));
        assert_eq!(r.workspace["code"], "y");

        s.finish_round(id).unwrap();
        s.save_report(id, Some(&json!({"overall": 61, "decision": "Lean hire"})), "raw").unwrap();
        s.save_report(id, Some(&json!({"overall": 64, "decision": "Hire"})), "raw2").unwrap();
        let h = s.history(10).unwrap();
        assert_eq!((h[0].overall, h[0].decision.as_deref(), h[0].company.as_str()), (Some(64), Some("Hire"), "Google"));
        assert_eq!(s.report(id).unwrap().unwrap().1, "raw2");
        assert_eq!(s.loop_round_ids(loop_id).unwrap(), vec![id]);
    }

    #[test]
    fn settings_roundtrip_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        Store::open(&path).unwrap().set_setting("model", &"opus").unwrap();
        let s = Store::open(&path).unwrap();
        assert_eq!(s.get_setting::<String>("model").unwrap().as_deref(), Some("opus"));
        assert_eq!(s.get_setting::<String>("missing").unwrap(), None);
    }
}
