//! SQLite storage. Every write is immediate, so reports and transcripts survive restarts.

use crate::error::Result;
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

const SCHEMA_VERSION: i32 = 1;
/// Settings key holding the last round id handed out.
const ROUND_SEQ: &str = "round_seq";

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
    pub round_key: String,
    pub name: String,
    pub company: String,
    pub minutes: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub overall: Option<i64>,
    pub decision: Option<String>,
    /// The parsed feedback, for the dashboard's skill and weakness tracking.
    pub report: Option<Value>,
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
        // Ids are never reused, even after a delete: the UI ignores late events from rounds it
        // left by id, so a new round that got an old id would never show its replies.
        let last: i64 = self.get_setting(ROUND_SEQ)?.unwrap_or(0);
        let max: i64 = self.conn.query_row("SELECT COALESCE(MAX(id), 0) FROM rounds", [], |r| r.get(0))?;
        let id = last.max(max) + 1;
        self.conn.execute(
            "INSERT INTO rounds (id, loop_id, idx, round_key, name, persona, minutes, model, started_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![id, loop_id, idx as i64, round_key, name, persona, minutes, model, now()],
        )?;
        self.set_setting(ROUND_SEQ, &id)?;
        Ok(id)
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

    /// Transcripts of the latest rounds of one kind, newest first.
    pub fn recent_transcripts(&self, round_key: &str, limit: i64) -> Result<Vec<Vec<Turn>>> {
        let mut st = self.conn.prepare("SELECT transcript FROM rounds WHERE round_key = ?1 ORDER BY id DESC LIMIT ?2")?;
        let rows = st.query_map(params![round_key, limit], |r| r.get::<_, String>(0))?;
        Ok(rows.filter_map(|r| r.ok()).filter_map(|t| serde_json::from_str(&t).ok()).collect())
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

    /// Deletes a round and its feedback. The loop stays (a live loop may start its next round),
    /// but its verdict is cleared because it was judged on the deleted round too.
    /// Deletes rounds the candidate never answered in: ones a crash or force-close left behind.
    /// Call only when no round is running. Returns how many went.
    pub fn delete_unanswered_rounds(&self) -> Result<usize> {
        let mut stmt = self.conn.prepare("SELECT id, transcript FROM rounds")?;
        let empty: Vec<i64> = stmt
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .filter(|(_, t)| !serde_json::from_str::<Vec<Turn>>(t).is_ok_and(|t| t.iter().any(|x| x.who == "me")))
            .map(|(id, _)| id)
            .collect();
        for &id in &empty {
            self.delete_round(id)?;
        }
        Ok(empty.len())
    }

    pub fn delete_round(&self, round_id: i64) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        let loop_id: Option<i64> = tx.query_row("SELECT loop_id FROM rounds WHERE id = ?1", [round_id], |r| r.get(0)).optional()?;
        tx.execute("DELETE FROM reports WHERE round_id = ?1", [round_id])?;
        tx.execute("DELETE FROM rounds WHERE id = ?1", [round_id])?;
        if let Some(l) = loop_id {
            tx.execute("UPDATE loops SET verdict = NULL WHERE id = ?1", [l])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Row counts and transcript bytes, for Settings → Storage.
    pub fn counts(&self) -> Result<StoreCounts> {
        let n = |sql: &str| self.conn.query_row(sql, [], |r| r.get::<_, i64>(0));
        Ok(StoreCounts {
            loops: n("SELECT COUNT(*) FROM loops")?,
            rounds: n("SELECT COUNT(*) FROM rounds")?,
            reports: n("SELECT COUNT(*) FROM reports")?,
            dossiers: n("SELECT COUNT(*) FROM dossiers")?,
            research_jobs: n("SELECT COUNT(*) FROM research_jobs")?,
            transcript_bytes: n("SELECT COALESCE(SUM(LENGTH(transcript) + LENGTH(workspace)), 0) FROM rounds")?,
            oldest_round: self.conn.query_row("SELECT MIN(started_at) FROM rounds", [], |r| r.get(0))?,
        })
    }

    /// Finished rounds past the limits: beyond the newest `keep` (0 = no cap), or started
    /// before `before` (RFC 3339). Rounds still running are never picked.
    pub fn rounds_over_limit(&self, keep: u32, before: Option<&str>) -> Result<Vec<i64>> {
        let mut st = self.conn.prepare("SELECT id, started_at FROM rounds WHERE ended_at IS NOT NULL ORDER BY started_at DESC, id DESC")?;
        let rows: Vec<(i64, String)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<std::result::Result<_, _>>()?;
        Ok(rows
            .into_iter()
            .enumerate()
            .filter(|(i, (_, at))| (keep > 0 && *i >= keep as usize) || before.is_some_and(|b| at.as_str() < b))
            .map(|(_, (id, _))| id)
            .collect())
    }

    /// Loops with no rounds left, created before `before`.
    pub fn delete_empty_loops(&self, before: &str) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM loops WHERE created_at < ?1 AND id NOT IN (SELECT loop_id FROM rounds)", [before])?)
    }

    pub fn vacuum(&self) -> Result<()> {
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
        Ok(())
    }

    pub fn history(&self, limit: i64) -> Result<Vec<HistoryItem>> {
        let mut st = self.conn.prepare(
            "SELECT r.id, r.loop_id, r.name, l.config, r.started_at, r.ended_at, rep.data, r.round_key, r.minutes
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
                r.get::<_, String>(7)?,
                r.get::<_, i64>(8)?,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (round_id, loop_id, name, config, started_at, ended_at, data, round_key, minutes) = row?;
            let config: Value = serde_json::from_str(&config).unwrap_or_default();
            let data: Option<Value> = data.and_then(|d| serde_json::from_str(&d).ok());
            out.push(HistoryItem {
                round_id,
                loop_id,
                round_key,
                name,
                company: config["company_name"].as_str().unwrap_or_default().to_string(),
                minutes,
                started_at,
                ended_at,
                overall: data.as_ref().and_then(|d| d["overall"].as_i64()),
                decision: data.as_ref().and_then(|d| d["decision"].as_str().map(String::from)),
                report: data,
            });
        }
        Ok(out)
    }

    /// Inserts or replaces a dossier (researching the same company and role again refreshes it).
    pub fn save_dossier(&self, id: &str, company: &str, role: &str, level: &str, data: &Value) -> Result<()> {
        self.conn.execute(
            "INSERT INTO dossiers (id, company, role, level, data, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET company = excluded.company, role = excluded.role, level = excluded.level,
             data = excluded.data, created_at = excluded.created_at",
            params![id, company, role, level, data.to_string(), now()],
        )?;
        Ok(())
    }

    /// Newest first.
    pub fn dossiers(&self) -> Result<Vec<Value>> {
        let mut st = self.conn.prepare("SELECT data FROM dossiers ORDER BY created_at DESC")?;
        let rows = st.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for raw in rows {
            out.push(serde_json::from_str(&raw?)?);
        }
        Ok(out)
    }

    pub fn dossier(&self, id: &str) -> Result<Option<Value>> {
        let raw: Option<String> = self.conn.query_row("SELECT data FROM dossiers WHERE id = ?1", [id], |r| r.get(0)).optional()?;
        Ok(raw.map(|s| serde_json::from_str(&s)).transpose()?)
    }

    pub fn delete_dossier(&self, id: &str) -> Result<()> {
        self.conn.execute("UPDATE research_jobs SET dossier_id = NULL WHERE dossier_id = ?1", [id])?;
        self.conn.execute("DELETE FROM dossiers WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn create_job(&self, request: &Value) -> Result<i64> {
        let t = now();
        self.conn.execute(
            "INSERT INTO research_jobs (request, status, created_at, updated_at) VALUES (?1, 'running', ?2, ?2)",
            params![request.to_string(), t],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_job(&self, job: &JobRow) -> Result<()> {
        self.conn.execute(
            "UPDATE research_jobs SET status = ?1, progress = ?2, log = ?3, raw_output = ?4, dossier_id = ?5, updated_at = ?6
             WHERE id = ?7",
            params![job.status, job.progress, job.log, job.raw_output, job.dossier_id, now(), job.id],
        )?;
        Ok(())
    }

    pub fn job(&self, id: i64) -> Result<JobRow> {
        Ok(self.conn.query_row(&format!("{JOB_COLS} WHERE id = ?1"), [id], job_row)?)
    }

    /// Newest first.
    pub fn jobs(&self, limit: i64) -> Result<Vec<JobRow>> {
        let mut st = self.conn.prepare(&format!("{JOB_COLS} ORDER BY id DESC LIMIT ?1"))?;
        let rows = st.query_map([limit], job_row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn delete_job(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM research_jobs WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Jobs left running when the app closed can't resume; they become failed, with a retry.
    pub fn fail_running_jobs(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE research_jobs SET status = 'failed', log = log || 'Onsite closed while this was running.\n', updated_at = ?1
             WHERE status = 'running'",
            [now()],
        )?)
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StoreCounts {
    pub loops: i64,
    pub rounds: i64,
    pub reports: i64,
    pub dossiers: i64,
    pub research_jobs: i64,
    pub transcript_bytes: i64,
    pub oldest_round: Option<String>,
}

const JOB_COLS: &str = "SELECT id, request, status, progress, log, raw_output, dossier_id, created_at, updated_at FROM research_jobs";

/// One research job. `status`: running, done, failed or cancelled.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct JobRow {
    pub id: i64,
    pub request: Value,
    pub status: String,
    pub progress: i64,
    pub log: String,
    pub raw_output: Option<String>,
    pub dossier_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

fn job_row(r: &rusqlite::Row) -> rusqlite::Result<JobRow> {
    Ok(JobRow {
        id: r.get(0)?,
        request: serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or(Value::Null),
        status: r.get(2)?,
        progress: r.get(3)?,
        log: r.get(4)?,
        raw_output: r.get(5)?,
        dossier_id: r.get(6)?,
        created_at: r.get(7)?,
        updated_at: r.get(8)?,
    })
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
        assert_eq!((h[0].round_key.as_str(), h[0].minutes), ("coding", 45));
        assert_eq!(h[0].report.as_ref().unwrap()["overall"], 64);
        assert_eq!(s.report(id).unwrap().unwrap().1, "raw2");
        assert_eq!(s.loop_round_ids(loop_id).unwrap(), vec![id]);

        s.set_loop_verdict(loop_id, &json!({"decision": "Hire"})).unwrap();
        s.delete_round(id).unwrap();
        assert!(s.history(10).unwrap().is_empty());
        assert!(s.report(id).unwrap().is_none());
        assert!(s.round(id).is_err());
        // Rounds with no answers are swept; answered ones stay.
        let kept = s.create_round(loop_id, 0, "coding", "DSA coding", "Rahul", 45, "sonnet").unwrap();
        let me = vec![Turn { who: "me".into(), text: "Hello.".into(), at_min: 0.0, ttft_ms: None, shared_workspace: false, sent: None }];
        s.save_round_progress(kept, &me, &json!({}), None).unwrap();
        s.create_round(loop_id, 0, "coding", "DSA coding", "Rahul", 45, "sonnet").unwrap();
        assert_eq!(s.delete_unanswered_rounds().unwrap(), 1);
        assert_eq!(s.loop_round_ids(loop_id).unwrap(), vec![kept]);
        // A deleted round's id is never handed out again.
        assert!(s.create_round(loop_id, 0, "coding", "DSA coding", "Rahul", 45, "sonnet").unwrap() > id);
        assert_eq!(s.loop_verdict(loop_id).unwrap(), None);
        s.delete_round(id).unwrap(); // already gone: no error
    }

    #[test]
    fn limits_pick_old_finished_rounds() {
        let s = Store::open_in_memory().unwrap();
        let l = s.create_loop(&json!({})).unwrap();
        let ids: Vec<i64> = (0..4).map(|i| s.create_round(l, i, "coding", "DSA", "R", 45, "m").unwrap()).collect();
        for (i, id) in ids.iter().enumerate() {
            s.conn.execute("UPDATE rounds SET started_at = ?1 WHERE id = ?2", params![format!("2026-0{}-01T00:00:00Z", i + 1), id]).unwrap();
        }
        for id in &ids[..3] {
            s.finish_round(*id).unwrap();
        }
        // ids[3] is still running: never picked.
        assert_eq!(s.rounds_over_limit(2, None).unwrap(), vec![ids[0]]);
        assert_eq!(s.rounds_over_limit(0, Some("2026-02-15T00:00:00Z")).unwrap(), vec![ids[1], ids[0]]);
        assert!(s.rounds_over_limit(0, None).unwrap().is_empty());
        let c = s.counts().unwrap();
        assert_eq!((c.loops, c.rounds), (1, 4));
        assert_eq!(c.oldest_round.as_deref(), Some("2026-01-01T00:00:00Z"));
        s.vacuum().unwrap();
    }

    #[test]
    fn dossiers_and_jobs() {
        let s = Store::open_in_memory().unwrap();
        s.save_dossier("acme-sde", "Acme", "SDE", "sde2", &json!({"id": "acme-sde", "company": "Acme"})).unwrap();
        s.save_dossier("acme-sde", "Acme", "SDE", "sde2", &json!({"id": "acme-sde", "company": "Acme", "v": 2})).unwrap();
        assert_eq!(s.dossiers().unwrap().len(), 1);
        assert_eq!(s.dossier("acme-sde").unwrap().unwrap()["v"], 2);

        let id = s.create_job(&json!({"company": "Acme"})).unwrap();
        let mut job = s.job(id).unwrap();
        assert_eq!((job.status.as_str(), job.request["company"].as_str()), ("running", Some("Acme")));
        job.progress = 40;
        job.log.push_str("searching\n");
        s.update_job(&job).unwrap();
        assert_eq!(s.fail_running_jobs().unwrap(), 1);
        let job = s.job(id).unwrap();
        assert_eq!((job.status.as_str(), job.progress), ("failed", 40));
        assert!(job.log.ends_with("Onsite closed while this was running.\n"));
        assert_eq!(s.jobs(5).unwrap().len(), 1);

        let mut done = s.job(s.create_job(&json!({})).unwrap()).unwrap();
        done.dossier_id = Some("acme-sde".into());
        s.update_job(&done).unwrap();
        s.delete_dossier("acme-sde").unwrap();
        assert!(s.dossier("acme-sde").unwrap().is_none());
        assert_eq!(s.job(done.id).unwrap().dossier_id, None);
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
