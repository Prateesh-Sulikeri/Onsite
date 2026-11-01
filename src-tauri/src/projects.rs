//! Multi-file projects for the code comprehension round: `projects/<id>/project.json` plus the files.
//! Curated data, like `prompts/questions.csv`. Nothing in a project is ever run or executed.
//! The candidate sees the brief and the files. Issues, the extension task and the reference diff
//! go only to the interviewer and the report.

use serde::Deserialize;
use serde_json::{json, Value};
use similar::TextDiff;
use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path};

pub const SCHEMA: u32 = 1;
const MAX_FILES: usize = 8;
const MAX_FILE_BYTES: usize = 40_000;
const MAX_TOTAL_BYTES: usize = 120_000;
/// Files the candidate may add on top of the project's own.
pub const MAX_EXTRA_FILES: usize = 6;

#[derive(Debug, Clone, Deserialize)]
struct FileMeta {
    path: String,
    #[serde(default)]
    open: bool,
    #[serde(default)]
    readonly: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Issue {
    pub id: String,
    pub file: String,
    #[serde(default)]
    pub lines: Vec<u32>,
    #[serde(default)]
    pub category: String,
    pub why: String,
    #[serde(default)]
    pub good_fix: String,
}

#[derive(Debug, Clone, Deserialize)]
struct Manifest {
    schema: u32,
    id: String,
    title: String,
    language: String,
    #[serde(default)]
    difficulty: String,
    #[serde(default)]
    levels: Vec<String>,
    #[serde(default)]
    topics: Vec<String>,
    #[serde(default)]
    companies: Vec<String>,
    #[serde(default)]
    minutes: u32,
    brief: String,
    files: Vec<FileMeta>,
    #[serde(default)]
    issues: Vec<Issue>,
    #[serde(default)]
    extension: String,
}

#[derive(Debug, Clone)]
pub struct ProjectFile {
    pub path: String,
    pub text: String,
    pub open: bool,
    pub readonly: bool,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub id: String,
    pub title: String,
    pub language: String,
    pub difficulty: String,
    pub levels: Vec<String>,
    pub topics: Vec<String>,
    pub companies: Vec<String>,
    pub minutes: u32,
    pub brief: String,
    pub files: Vec<ProjectFile>,
    pub issues: Vec<Issue>,
    pub extension: String,
    /// Reference fix (`solution.diff`), hidden.
    pub solution: String,
}

/// A path the candidate or a manifest may use: relative, forward slashes, no `..`, no drive.
pub fn safe_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 120
        && !p.contains('\\')
        && !p.contains(':')
        && !p.contains('\0')
        && Path::new(p).components().all(|c| matches!(c, Component::Normal(_)))
}

fn unified(path: &str, old: &str, new: &str) -> String {
    TextDiff::from_lines(old, new)
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string()
}

impl Project {
    /// Reads one project folder. Every problem is a plain message, so a bad folder is skipped, not fatal.
    pub fn load(dir: &Path) -> Result<Project, String> {
        let raw = std::fs::read_to_string(dir.join("project.json")).map_err(|e| format!("project.json: {e}"))?;
        let m: Manifest = serde_json::from_str(&raw).map_err(|e| format!("project.json: {e}"))?;
        if m.schema != SCHEMA {
            return Err(format!("unknown schema {}", m.schema));
        }
        if m.files.is_empty() || m.files.len() > MAX_FILES {
            return Err(format!("needs 1 to {MAX_FILES} files"));
        }
        let mut files = Vec::new();
        let mut total = 0;
        for f in &m.files {
            if !safe_path(&f.path) {
                return Err(format!("unsafe path '{}'", f.path));
            }
            let text = std::fs::read_to_string(dir.join(&f.path)).map_err(|e| format!("{}: {e}", f.path))?;
            if text.len() > MAX_FILE_BYTES {
                return Err(format!("{} is too big", f.path));
            }
            total += text.len();
            files.push(ProjectFile { path: f.path.clone(), text: text.replace("\r\n", "\n"), open: f.open, readonly: f.readonly });
        }
        if total > MAX_TOTAL_BYTES {
            return Err("project is too big".into());
        }
        let names: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
        if let Some(i) = m.issues.iter().find(|i| !names.contains(i.file.as_str())) {
            return Err(format!("issue '{}' names a file that is not in the project", i.id));
        }
        let solution = std::fs::read_to_string(dir.join("solution.diff")).unwrap_or_default().replace("\r\n", "\n");
        Ok(Project {
            id: m.id,
            title: m.title,
            language: m.language,
            difficulty: m.difficulty,
            levels: m.levels,
            topics: m.topics,
            companies: m.companies,
            minutes: m.minutes,
            brief: m.brief,
            files,
            issues: m.issues,
            extension: m.extension,
            solution,
        })
    }

    pub fn base(&self) -> BTreeMap<String, String> {
        self.files.iter().map(|f| (f.path.clone(), f.text.clone())).collect()
    }

    /// What the candidate sees. No issues, extension or reference fix.
    pub fn view(&self) -> Value {
        json!({
            "id": self.id,
            "title": self.title,
            "language": self.language,
            "difficulty": self.difficulty,
            "brief": self.brief,
            "topics": self.topics,
            "files": self.files.iter().map(|f| json!({"path": f.path, "text": f.text, "open": f.open, "readonly": f.readonly})).collect::<Vec<_>>(),
        })
    }

    /// Unified diffs for the candidate's files. `since` is what was last sent (missing path = the
    /// original). With `force`, every file that differs from the original. Read-only files and
    /// unsafe or surplus paths are ignored.
    pub fn diffs(&self, files: &BTreeMap<String, String>, since: &BTreeMap<String, String>, force: bool) -> String {
        let base = self.base();
        let mut out = String::new();
        let mut extra = 0;
        for (path, now) in files {
            if !safe_path(path) {
                continue;
            }
            let orig = base.get(path);
            if self.files.iter().any(|f| f.path == *path && f.readonly) {
                continue;
            }
            if orig.is_none() {
                extra += 1;
                if extra > MAX_EXTRA_FILES {
                    continue;
                }
            }
            let now = clip_bytes(now);
            let orig_text = orig.map_or("", String::as_str);
            let prev = since.get(path).map_or(orig_text, String::as_str);
            let changed = if force { now != orig_text } else { now != prev };
            if !changed {
                continue;
            }
            if now == orig_text {
                out.push_str(&format!("(Reverted {path} to the original.)\n"));
            } else {
                out.push_str(&unified(path, orig_text, now));
            }
        }
        out
    }

    /// The hidden notes and the whole project, for the interviewer's system prompt.
    pub fn interviewer_block(&self) -> String {
        let mut s = format!(
            "\n\nTHE PROJECT FOR THIS ROUND (this replaces the instruction to write and paste a file yourself): {title} ({lang}, {diff}).\n\
             The candidate already has these files open on their screen, in a file tree next to the chat. They can read and edit them. \
             Never paste the code and never call it a tab. Refer to files by path and to lines by number. Nothing can be run in this round: \
             they read, edit and send you their work, and you review it. Never say a test passed or failed.\n\
             Brief they were shown: {brief}\n\n\
             Run it in stages: (1) they read the code and say what it does, (2) they trace a scenario from the brief, \
             (3) they find the problem(s), (4) they fix it and say how they would test it, (5) if time remains, this extension: {ext}\n\n\
             Their later messages carry a [Workspace] block with unified diffs against the original files. A hunk is their edit. \
             Judge it against the issues below: did it fix the real cause, miss an edge case, or add a new bug? \
             No diff means they changed nothing yet.\n\n\
             HIDDEN, never read out or shown. Planted issues:\n",
            title = self.title,
            lang = self.language,
            diff = self.difficulty,
            brief = self.brief,
            ext = if self.extension.is_empty() { "ask how they would extend it for a new requirement" } else { &self.extension },
        );
        for i in &self.issues {
            let lines = match i.lines.as_slice() {
                [a, b, ..] => format!(":{a}-{b}"),
                [a] => format!(":{a}"),
                [] => String::new(),
            };
            s.push_str(&format!("- [{}] {}{} ({}): {} Good fix: {}\n", i.id, i.file, lines, i.category, i.why, i.good_fix));
        }
        if !self.solution.trim().is_empty() {
            s.push_str(&format!("\nReference fix (hidden):\n```diff\n{}\n```\n", self.solution.trim_end()));
        }
        s.push_str("\nDo not reveal an issue they miss until the end, and then name only one or two.\n\nFILES (line numbers added for you; the candidate's editor shows the same numbers):\n");
        for f in &self.files {
            s.push_str(&format!("\n--- {}{}\n", f.path, if f.readonly { " (read-only)" } else { "" }));
            for (n, line) in f.text.lines().enumerate() {
                s.push_str(&format!("{:>3}| {line}\n", n + 1));
            }
        }
        s
    }

    /// The hidden issues as plain text, for the report.
    pub fn issues_text(&self) -> String {
        self.issues
            .iter()
            .map(|i| format!("- [{}] {} ({}): {} Good fix: {}", i.id, i.file, i.category, i.why, i.good_fix))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// File extensions a project in this language may use.
pub fn lang_exts(lang: &str) -> &'static [&'static str] {
    match lang.to_ascii_lowercase().as_str() {
        "java" => &["java"],
        "python" => &["py"],
        "c++" | "cpp" => &["cpp", "cc", "h", "hpp"],
        "javascript" | "js" => &["js", "mjs"],
        "typescript" | "ts" => &["ts"],
        "go" => &["go"],
        _ => &[],
    }
}

/// What the model returns when it writes a project (see `prompts/project.md`).
#[derive(Debug, Deserialize)]
struct Generated {
    title: String,
    #[serde(default)]
    difficulty: String,
    #[serde(default)]
    topics: Vec<String>,
    brief: String,
    files: Vec<GeneratedFile>,
    issues: Vec<Issue>,
    #[serde(default)]
    extension: String,
    #[serde(default)]
    solution_diff: String,
}

#[derive(Debug, Deserialize)]
struct GeneratedFile {
    path: String,
    #[serde(default)]
    open: bool,
    #[serde(default)]
    readonly: bool,
    text: String,
}

/// Words a comment must not contain: they point at a planted bug.
fn giveaway() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| {
        regex::Regex::new(r"(?i)\b(bugs?|buggy|fixme|todo|xxx|hack|race condition|deadlock|memory leak|vulnerab\w*|off-by-one|incorrect|wrong|broken|intentional\w*|planted)\b").unwrap()
    })
}

impl Project {
    /// Checks a project the model wrote. Model output is data: every rule here is a plain message that
    /// is sent back to the model for one retry. Nothing in it is compiled or run.
    pub fn from_generated(v: &Value, lang: &str, minutes: u32) -> Result<Project, String> {
        let g: Generated = serde_json::from_value(v.clone()).map_err(|e| format!("the JSON does not match the shape asked for ({e})"))?;
        let exts = lang_exts(lang);
        if exts.is_empty() {
            return Err(format!("no projects in {lang}"));
        }
        if !(2..=5).contains(&g.files.len()) {
            return Err("it needs 3 to 5 files".into());
        }
        let mut files: Vec<ProjectFile> = Vec::new();
        let mut total = 0;
        for f in &g.files {
            if !safe_path(&f.path) {
                return Err(format!("unsafe file path '{}'", f.path));
            }
            if files.iter().any(|x| x.path == f.path) {
                return Err(format!("{} appears twice", f.path));
            }
            let ext = f.path.rsplit('.').next().unwrap_or("");
            if !exts.contains(&ext) {
                return Err(format!("{} is not a {lang} file", f.path));
            }
            let text = f.text.replace("\r\n", "\n");
            if text.trim().is_empty() || text.len() > MAX_FILE_BYTES {
                return Err(format!("{} is empty or too big", f.path));
            }
            total += text.len();
            files.push(ProjectFile { path: f.path.clone(), text, open: f.open, readonly: f.readonly });
        }
        if total > MAX_TOTAL_BYTES {
            return Err("the project is too big; keep it to about 300 lines".into());
        }
        if files.iter().all(|f| f.readonly) {
            return Err("every file is read-only; the bugs must be in an editable file".into());
        }
        // Exactly one file opens first, and it is an editable one.
        let first = files.iter().position(|f| f.open && !f.readonly).or_else(|| files.iter().position(|f| !f.readonly)).unwrap_or(0);
        for (i, f) in files.iter_mut().enumerate() {
            f.open = i == first;
        }
        if !(2..=4).contains(&g.issues.len()) {
            return Err("it needs 2 or 3 planted issues".into());
        }
        for i in &g.issues {
            let Some(f) = files.iter().find(|f| f.path == i.file) else {
                return Err(format!("issue '{}' names {} which is not a file", i.id, i.file));
            };
            if f.readonly {
                return Err(format!("issue '{}' is in a read-only file", i.id));
            }
            let n = f.text.lines().count() as u32;
            if i.lines.len() != 2 || i.lines[0] < 1 || i.lines[0] > i.lines[1] || i.lines[1] > n {
                return Err(format!("issue '{}' has line numbers outside {} ({n} lines)", i.id, i.file));
            }
            if i.why.trim().len() < 20 || i.good_fix.trim().len() < 10 {
                return Err(format!("issue '{}' needs a real explanation and good_fix", i.id));
            }
        }
        for f in &files {
            for (n, line) in f.text.lines().enumerate() {
                let t = line.trim_start();
                let comment = t.starts_with("//") || t.starts_with('#') || t.starts_with("/*") || t.starts_with('*') || line.contains(" // ");
                if comment && giveaway().is_match(line) {
                    return Err(format!("the comment on line {} of {} hints at a bug: remove or reword it", n + 1, f.path));
                }
            }
        }
        if !(30..=700).contains(&g.brief.trim().len()) {
            return Err("the brief should be 2 to 4 short sentences".into());
        }
        let difficulty = match g.difficulty.to_ascii_lowercase().as_str() {
            "easy" => "Easy",
            "hard" => "Hard",
            _ => "Medium",
        };
        let mut id_hash = 0xcbf2_9ce4_8422_2325_u64;
        for b in g.title.bytes().chain(files.iter().flat_map(|f| f.text.bytes())) {
            id_hash = (id_hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
        }
        Ok(Project {
            id: format!("gen-{:08x}", id_hash as u32),
            title: g.title.trim().chars().take(80).collect(),
            language: lang.to_string(),
            difficulty: difficulty.into(),
            levels: Vec::new(),
            topics: g.topics.iter().take(4).map(|t| t.trim().chars().take(40).collect()).collect(),
            companies: Vec::new(),
            minutes,
            brief: g.brief.trim().to_string(),
            files,
            issues: g.issues,
            extension: g.extension.trim().to_string(),
            solution: g.solution_diff.replace("\r\n", "\n"),
        })
    }
}

fn clip_bytes(s: &str) -> &str {
    if s.len() <= MAX_FILE_BYTES {
        return s;
    }
    let mut end = MAX_FILE_BYTES;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[derive(Default)]
pub struct Projects {
    pub list: Vec<Project>,
    pub errors: Vec<String>,
}

fn same_language(a: &str, b: &str) -> bool {
    let norm = |s: &str| match s.to_ascii_lowercase().as_str() {
        "c++" | "cpp" => "cpp".to_string(),
        "js" | "javascript" => "javascript".to_string(),
        "ts" | "typescript" => "typescript".to_string(),
        o => o.to_string(),
    };
    norm(a) == norm(b)
}

fn mix(seed: u64, id: &str) -> u64 {
    id.bytes().fold(seed ^ 0x9E37_79B9_7F4A_7C15, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3))
}

impl Projects {
    /// Loads every folder under `<dir>/projects`. A missing folder is an empty list.
    pub fn load(dir: &Path) -> Projects {
        let mut out = Projects::default();
        let Ok(rd) = std::fs::read_dir(dir) else { return out };
        let mut dirs: Vec<_> = rd.filter_map(|e| e.ok()).filter(|e| e.path().join("project.json").is_file()).collect();
        dirs.sort_by_key(|e| e.file_name());
        for e in dirs {
            match Project::load(&e.path()) {
                Ok(p) if out.list.iter().any(|q| q.id == p.id) => out.errors.push(format!("{}: duplicate id", p.id)),
                Ok(p) => out.list.push(p),
                Err(m) => out.errors.push(format!("{}: {m}", e.file_name().to_string_lossy())),
            }
        }
        out
    }

    /// A project in the candidate's language, not used lately. Prefers the level, then the company.
    /// None when no project is in that language (the round then falls back to a model-written file).
    pub fn pick(&self, lang: &str, level: &str, company: &str, recent: &[String], seed: u64) -> Option<&Project> {
        let in_lang: Vec<&Project> = self.list.iter().filter(|p| same_language(&p.language, lang)).collect();
        let fresh: Vec<&Project> = in_lang.iter().copied().filter(|p| !recent.contains(&p.id)).collect();
        let pool = if fresh.is_empty() { in_lang } else { fresh };
        pool.into_iter().max_by_key(|p| {
            let level_hit = p.levels.iter().any(|l| l.eq_ignore_ascii_case(level));
            let company_hit = p.companies.iter().any(|c| c.eq_ignore_ascii_case(company));
            (u8::from(level_hit) * 2 + u8::from(company_hit), mix(seed, &p.id))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Project {
        let f = |p: &str, t: &str, ro: bool| ProjectFile { path: p.into(), text: t.into(), open: false, readonly: ro };
        Project {
            id: "x".into(),
            title: "X".into(),
            language: "java".into(),
            difficulty: "Medium".into(),
            levels: vec!["sde2".into()],
            topics: vec![],
            companies: vec![],
            minutes: 45,
            brief: "b".into(),
            files: vec![f("src/A.java", "a\nb\nc\n", false), f("test/T.java", "t\n", true)],
            issues: vec![Issue { id: "i1".into(), file: "src/A.java".into(), lines: vec![2, 3], category: "c".into(), why: "w".into(), good_fix: "g".into() }],
            extension: String::new(),
            solution: String::new(),
        }
    }

    fn files(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect()
    }

    #[test]
    fn paths() {
        assert!(safe_path("src/A.java"));
        for bad in ["", "../x", "/etc/x", "a/../b", "C:/x", "a\\b", "./x"] {
            assert!(!safe_path(bad), "{bad}");
        }
    }

    #[test]
    fn diff_only_changed_files() {
        let p = project();
        let none = BTreeMap::new();
        assert_eq!(p.diffs(&files(&[("src/A.java", "a\nb\nc\n")]), &none, false), "");
        let d = p.diffs(&files(&[("src/A.java", "a\nB\nc\n")]), &none, false);
        assert!(d.contains("--- a/src/A.java") && d.contains("-b") && d.contains("+B"), "{d}");
        // Already sent: nothing new, unless forced.
        let sent = files(&[("src/A.java", "a\nB\nc\n")]);
        assert_eq!(p.diffs(&sent, &sent, false), "");
        assert!(p.diffs(&sent, &sent, true).contains("+B"));
        // Reverted after a send: say so.
        let back = files(&[("src/A.java", "a\nb\nc\n")]);
        assert!(p.diffs(&back, &sent, false).contains("Reverted"));
    }

    #[test]
    fn diff_is_against_the_original() {
        let p = project();
        let sent = files(&[("src/A.java", "a\nB\nc\n")]);
        let d = p.diffs(&files(&[("src/A.java", "a\nB\nC\n")]), &sent, false);
        assert!(d.contains("-b") && d.contains("+B") && d.contains("+C"), "{d}");
    }

    #[test]
    fn readonly_and_new_files() {
        let p = project();
        let none = BTreeMap::new();
        assert_eq!(p.diffs(&files(&[("test/T.java", "changed\n")]), &none, true), "");
        let d = p.diffs(&files(&[("src/B.java", "x\n"), ("../evil", "y\n")]), &none, false);
        assert!(d.contains("+++ b/src/B.java") && !d.contains("evil"), "{d}");
        assert_eq!(p.diffs(&files(&[("src/B.java", "")]), &none, true), "");
    }

    #[test]
    fn block_has_hidden_notes_and_numbered_files() {
        let b = project().interviewer_block();
        assert!(b.contains("[i1] src/A.java:2-3") && b.contains("  2| b") && b.contains("(read-only)"));
    }

    #[test]
    fn pick_prefers_language_and_fresh() {
        let mut p2 = project();
        p2.id = "y".into();
        let ps = Projects { list: vec![project(), p2], errors: vec![] };
        assert!(ps.pick("Python", "sde2", "", &[], 1).is_none());
        assert_eq!(ps.pick("Java", "sde2", "", &["x".into()], 1).unwrap().id, "y");
        assert!(ps.pick("Java", "sde2", "", &["x".into(), "y".into()], 1).is_some());
    }

    fn generated() -> Value {
        json!({
            "title": "Order totals",
            "difficulty": "medium",
            "topics": ["money"],
            "brief": "Customers see the wrong total on some orders. Find out why and fix it.",
            "files": [
                {"path": "src/Orders.java", "open": true, "text": "class Orders {\n  long total(long a, long b) {\n    return a + b;\n  }\n  int count() {\n    return 1;\n  }\n}\n"},
                {"path": "test/OrdersTest.java", "readonly": true, "text": "class OrdersTest {\n}\n"}
            ],
            "issues": [{"id": "sum", "file": "src/Orders.java", "lines": [2, 4], "category": "correctness", "why": "The total ignores the discount that applies.", "good_fix": "Subtract the discount."},
                       {"id": "cnt", "file": "src/Orders.java", "lines": [5, 7], "category": "correctness", "why": "The count is always one, never the real number.", "good_fix": "Count the lines."}],
            "extension": "Add tax.",
            "solution_diff": "--- a\n+++ b\n"
        })
    }

    #[test]
    fn generated_project_is_checked() {
        let p = Project::from_generated(&generated(), "Java", 45).unwrap();
        assert!(p.id.starts_with("gen-") && p.difficulty == "Medium" && p.files[0].open && !p.files[1].open);
        let bad = |f: &dyn Fn(&mut Value)| {
            let mut v = generated();
            f(&mut v);
            Project::from_generated(&v, "Java", 45).unwrap_err()
        };
        assert!(bad(&|v| v["issues"][0]["lines"] = json!([2, 99])).contains("line numbers"));
        assert!(bad(&|v| v["issues"][0]["file"] = json!("test/OrdersTest.java")).contains("read-only"));
        assert!(bad(&|v| v["files"][0]["path"] = json!("../Orders.java")).contains("unsafe"));
        assert!(bad(&|v| v["files"][0]["path"] = json!("src/orders.py")).contains("not a Java"));
        assert!(bad(&|v| v["files"][0]["text"] = json!("class A {\n  // TODO: this sum is wrong\n  int x;\n  int y;\n  int z;\n  int w;\n  int v;\n}\n")).contains("hints at a bug"));
        assert!(Project::from_generated(&generated(), "Cobol", 45).is_err());
    }

    #[test]
    fn shipped_projects_load() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../projects");
        let ps = Projects::load(&dir);
        assert!(ps.errors.is_empty(), "{:?}", ps.errors);
        assert!(ps.list.len() >= 2);
        for p in &ps.list {
            assert!(p.files.iter().any(|f| f.open), "{} has no file open", p.id);
            for i in &p.issues {
                let f = p.files.iter().find(|f| f.path == i.file).unwrap();
                let n = f.text.lines().count() as u32;
                assert!(i.lines.iter().all(|l| *l >= 1 && *l <= n), "{} issue {} lines outside {}", p.id, i.id, i.file);
            }
        }
    }
}
