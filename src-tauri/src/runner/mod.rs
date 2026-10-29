//! Runs the candidate's solution against a question's tests with their own local toolchain.
//!
//! Only the candidate's code and our generated drivers are executed; tests are data read from
//! a JSON file. Each driver prints one `\x1e{json}` line per event:
//! `{"begin":i}`, `{"i":i,"out":...,"ms":...}`, `{"i":i,"err":"...","ms":...}` or `{"compile":"..."}`.
//! Anything else on stdout is the candidate's own printing, attributed to the current test.

mod gen;

use crate::bank::{Question, Test};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

pub use gen::starter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Lang {
    Java,
    Python,
    Cpp,
    Js,
}

impl Lang {
    pub const ALL: [Lang; 4] = [Lang::Java, Lang::Python, Lang::Cpp, Lang::Js];

    /// From the setup screen's language name. TypeScript and Go have no runner.
    pub fn from_name(s: &str) -> Option<Lang> {
        match s.trim().to_lowercase().as_str() {
            "java" => Some(Lang::Java),
            "python" | "python3" => Some(Lang::Python),
            "c++" | "cpp" => Some(Lang::Cpp),
            "javascript" | "js" => Some(Lang::Js),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Lang::Java => "Java",
            Lang::Python => "Python",
            Lang::Cpp => "C++",
            Lang::Js => "JavaScript",
        }
    }

    /// Per-test time limit. Interpreted languages get more, as on LeetCode.
    fn limit_ms(self) -> f64 {
        match self {
            Lang::Java | Lang::Cpp => 2_000.0,
            Lang::Js => 3_000.0,
            Lang::Python => 6_000.0,
        }
    }
}

/// Which test a case came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Example,
    Hidden,
    /// Generated at full constraint size.
    Stress,
}

#[derive(Debug, Clone, Serialize)]
pub struct Case {
    pub kind: Kind,
    /// `nums = [2,7,11,15], target = 9`, clipped for huge stress inputs.
    pub input: String,
    pub expected: Value,
    pub got: Option<Value>,
    pub passed: bool,
    pub ms: Option<f64>,
    pub error: Option<String>,
    pub stdout: String,
    /// Not reached: an earlier test crashed or the run timed out.
    pub skipped: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Accepted,
    WrongAnswer,
    RuntimeError,
    TimeLimit,
    CompileError,
}

#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub status: Status,
    pub lang: Lang,
    pub compile_error: Option<String>,
    pub cases: Vec<Case>,
    pub passed: usize,
    pub total: usize,
    /// Slowest test, for the runtime line.
    pub max_ms: f64,
}

impl Report {
    /// One line for the interviewer, e.g. "Submit: 12/15 passed. First failure (hidden): ...".
    pub fn summary(&self, what: &str) -> String {
        let head = format!("{what} in {}: {}/{} tests passed", self.lang.label(), self.passed, self.total);
        match self.status {
            Status::Accepted => format!("{head}. All accepted, slowest test {:.0} ms.", self.max_ms),
            Status::CompileError => format!("{what} in {}: compile error:\n{}", self.lang.label(), clip(self.compile_error.as_deref().unwrap_or(""), 1_500)),
            _ => {
                let Some(c) = self.cases.iter().find(|c| !c.passed && !c.skipped) else { return head };
                let kind = match c.kind {
                    Kind::Example => "example",
                    Kind::Hidden => "hidden test",
                    Kind::Stress => "large test",
                };
                let why = match (self.status, &c.error) {
                    (Status::TimeLimit, _) => "time limit exceeded".to_string(),
                    (_, Some(e)) => format!("error: {}", clip(e, 600)),
                    _ => format!("expected {}, got {}", clip(&c.expected.to_string(), 300), clip(&c.got.as_ref().map_or("nothing".into(), Value::to_string), 300)),
                };
                format!("{head}. First failure ({kind}): input {} → {why}", clip(&c.input, 400))
            }
        }
    }
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max).collect::<String>())
}

/// Hides console windows for child processes on Windows.
fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut c = Command::new(program);
    c.kill_on_drop(true).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

struct Output {
    stdout: String,
    stderr: String,
    code: Option<i32>,
    timed_out: bool,
}

/// Runs to completion or `limit`, keeping at most 4 MB of each stream.
async fn exec(mut cmd: Command, dir: &Path, limit: Duration) -> std::io::Result<Output> {
    let mut child = cmd.current_dir(dir).spawn()?;
    let ro = tokio::spawn(read_capped(child.stdout.take()));
    let re = tokio::spawn(read_capped(child.stderr.take()));
    let (code, timed_out) = match tokio::time::timeout(limit, child.wait()).await {
        Ok(status) => (status?.code(), false),
        Err(_) => {
            kill_tree(&mut child).await;
            (None, true)
        }
    };
    // A grandchild could still hold the pipes open; never wait on them forever.
    let grace = Duration::from_secs(3);
    let stdout = tokio::time::timeout(grace, ro).await.ok().and_then(Result::ok).unwrap_or_default();
    let stderr = tokio::time::timeout(grace, re).await.ok().and_then(Result::ok).unwrap_or_default();
    Ok(Output { stdout, stderr, code, timed_out })
}

/// Kills the process and everything it started. `java.exe` and `python.exe` on PATH are often
/// launcher shims whose real program is a child; killing only the shim leaves it running.
async fn kill_tree(child: &mut tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        let mut k = command("taskkill");
        k.args(["/F", "/T", "/PID", &pid.to_string()]);
        if let Ok(mut c) = k.spawn() {
            let _ = tokio::time::timeout(Duration::from_secs(5), c.wait()).await;
        }
    }
    let _ = child.kill().await;
}

async fn read_capped(s: Option<impl tokio::io::AsyncRead + Unpin>) -> String {
    let Some(mut s) = s else { return String::new() };
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match s.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) if buf.len() < 4 << 20 => buf.extend_from_slice(&chunk[..n]),
            Ok(_) => {}
        }
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// Installed toolchain for a language, e.g. "javac 21.0.2".
#[derive(Debug, Clone, Serialize)]
pub struct Toolchain {
    pub lang: Lang,
    pub label: &'static str,
    pub version: Option<String>,
}

pub async fn toolchains() -> Vec<Toolchain> {
    let mut out = Vec::new();
    for lang in Lang::ALL {
        let (prog, args): (&std::ffi::OsStr, &[&str]) = match lang {
            Lang::Java => ("javac".as_ref(), &["-version"]),
            Lang::Python => (python().as_os_str(), &["--version"]),
            Lang::Cpp => ("g++".as_ref(), &["--version"]),
            Lang::Js => ("node".as_ref(), &["--version"]),
        };
        let mut c = command(prog);
        c.args(args);
        let version = match exec(c, &std::env::temp_dir(), Duration::from_secs(10)).await {
            Ok(o) if o.code == Some(0) => {
                let text = format!("{}\n{}", o.stdout, o.stderr);
                text.lines().map(str::trim).find(|l| !l.is_empty() && !l.starts_with("Picked up")).map(String::from)
            }
            _ => None,
        };
        out.push(Toolchain { lang, label: lang.label(), version });
    }
    out
}

#[cfg(windows)]
const PYTHON: &str = "python";
#[cfg(not(windows))]
const PYTHON: &str = "python3";

/// Python for running code and building Submit tests: one on PATH, else the copy VoiceStudio
/// ships, so a machine with only VoiceStudio can still Run and Submit. On Windows, PATH's
/// `WindowsApps\python.exe` is a Store shortcut, not Python, and is skipped.
fn python() -> &'static std::path::PathBuf {
    static PY: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
    PY.get_or_init(|| {
        if cfg!(windows) {
            let on_path = std::env::var_os("PATH")
                .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter()
                .filter(|d| !d.to_string_lossy().to_lowercase().contains("windowsapps"))
                .map(|d| d.join("python.exe"))
                .find(|p| p.is_file());
            if let Some(p) = on_path.or_else(crate::voice::whisper::voicestudio_python) {
                return p;
            }
        }
        PYTHON.into()
    })
}

/// Files and commands for one language, built in `dir`.
struct Plan {
    compile: Option<Command>,
    run: Command,
}

fn plan(lang: Lang, q: &Question, code: &str, dir: &Path) -> std::io::Result<Plan> {
    let w = |name: &str, body: &str| std::fs::write(dir.join(name), body);
    Ok(match lang {
        Lang::Python => {
            w("Solution.py", code)?;
            w("driver.py", &gen::python_driver())?;
            w("sig.json", &gen::sig_json(q))?;
            let mut run = command(python());
            run.args(["-X", "utf8", "driver.py", "Solution.py", "sig.json", "tests.json"]);
            Plan { compile: None, run }
        }
        Lang::Js => {
            w("solution.js", code)?;
            w("driver.js", gen::JS_DRIVER)?;
            w("sig.json", &gen::sig_json(q))?;
            let mut run = command("node");
            run.args(["driver.js", "solution.js", "sig.json", "tests.json"]);
            Plan { compile: None, run }
        }
        Lang::Java => {
            // Imports go on line 1 so compiler line numbers match the editor.
            w("Solution.java", &format!("{}{}", gen::JAVA_IMPORTS, java_unpublic(code)))?;
            w("Main.java", &gen::java_main(q))?;
            let mut compile = command("javac");
            compile.args(["-encoding", "UTF-8", "-nowarn", "-d", "out", "Solution.java", "Main.java"]);
            let mut run = command("java");
            run.args(["-Xss512m", "-XX:+UseSerialGC", "-cp", "out", "Main", "tests.json"]);
            Plan { compile: Some(compile), run }
        }
        Lang::Cpp => {
            w("solution.cpp", code)?;
            w("main.cpp", &gen::cpp_main(q))?;
            let mut compile = command("g++");
            compile.args(["-std=c++17", "-O2", "-o", "main.exe", "main.cpp"]);
            // MinGW: a static exe with a 256 MB stack for deep recursion. macOS can't link
            // statically and Linux sets the stack with ulimit, so they build a plain binary.
            if cfg!(windows) {
                compile.args(["-static", "-Wl,--stack,268435456"]);
            }
            let mut run = command(if cfg!(windows) { "main.exe" } else { "./main.exe" });
            // Windows resolves a bare program name against PATH, not the working directory.
            if cfg!(windows) {
                run = command(dir.join("main.exe"));
            }
            run.arg("tests.json");
            Plan { compile: Some(compile), run }
        }
    })
}

/// LeetCode's own starters say `public class Codec`, which javac only accepts in Codec.java.
/// Everything lives in Solution.java, so `public` is dropped from type declarations.
/// Line numbers don't change.
fn java_unpublic(code: &str) -> String {
    let mut out = code.to_string();
    for kw in ["class ", "final class ", "abstract class ", "interface ", "enum ", "record "] {
        out = out.replace(&format!("public {kw}"), kw);
    }
    out
}

/// Compiler output without our temp paths and the JVM's banner.
fn tidy_compile(s: &str, dir: &Path) -> String {
    let d = dir.to_string_lossy();
    s.lines()
        .filter(|l| !l.starts_with("Picked up"))
        .map(|l| l.replace(&*d, "").replace(['\\', '/'], "/").trim_start_matches('/').to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .chars()
        .take(6_000)
        .collect()
}

/// Runs `code` against `tests` (each with its kind). Never fails on the candidate's mistakes:
/// those come back in the report. Errors are only for a missing toolchain or I/O.
pub async fn run(q: &Question, lang: Lang, code: &str, tests: &[(&Test, Kind)]) -> crate::error::Result<Report> {
    let tmp = tempfile::tempdir()?;
    let dir = tmp.path();
    let args: Vec<&Vec<Value>> = tests.iter().map(|(t, _)| &t.args).collect();
    std::fs::write(dir.join("tests.json"), serde_json::to_vec(&args)?)?;
    let Plan { compile, run } = plan(lang, q, code, dir)?;
    let missing = |e: std::io::Error| crate::error::AppError::Invalid(format!("{} isn't installed or isn't on PATH ({e}).", lang.label()));

    let report = |status, compile_error, cases: Vec<Case>| {
        let passed = cases.iter().filter(|c| c.passed).count();
        let max_ms = cases.iter().filter_map(|c| c.ms).fold(0.0, f64::max);
        Report { status, lang, compile_error, total: tests.len(), passed, cases, max_ms }
    };

    if let Some(c) = compile {
        let o = exec(c, dir, Duration::from_secs(90)).await.map_err(missing)?;
        if o.code != Some(0) {
            let msg = if o.timed_out { "Compiling took longer than 90 s.".to_string() } else { tidy_compile(&format!("{}{}", o.stdout, o.stderr), dir) };
            return Ok(report(Status::CompileError, Some(msg), vec![]));
        }
    }
    let limit_ms = lang.limit_ms();
    // Startup plus every test at its limit, capped: one slow test shouldn't hang the UI.
    let budget = Duration::from_millis((3_000.0 + limit_ms * tests.len() as f64).min(60_000.0) as u64);
    let o = exec(run, dir, budget).await.map_err(missing)?;
    Ok(parse_output(q, tests, &o, limit_ms, report))
}

fn parse_output(
    q: &Question,
    tests: &[(&Test, Kind)],
    o: &Output,
    limit_ms: f64,
    report: impl Fn(Status, Option<String>, Vec<Case>) -> Report,
) -> Report {
    let mut cases: Vec<Case> = tests
        .iter()
        .map(|(t, kind)| Case {
            kind: *kind,
            input: clip(&q.input_text(&t.args), 600),
            expected: if *kind == Kind::Stress { clip_value(&t.expected) } else { t.expected.clone() },
            got: None,
            passed: false,
            ms: None,
            error: None,
            stdout: String::new(),
            skipped: true,
        })
        .collect();
    let mut current: Option<usize> = None;
    for line in o.stdout.lines() {
        let Some(json) = line.strip_prefix('\u{1e}') else {
            if let Some(c) = current.and_then(|i| cases.get_mut(i)) {
                if c.stdout.len() < 8_000 {
                    c.stdout.push_str(line);
                    c.stdout.push('\n');
                }
            }
            continue;
        };
        let Ok(ev) = serde_json::from_str::<Value>(json) else { continue };
        if let Some(msg) = ev["compile"].as_str() {
            return report(Status::CompileError, Some(msg.trim().to_string()), vec![]);
        }
        if let Some(i) = ev["begin"].as_u64() {
            current = Some(i as usize);
            if let Some(c) = cases.get_mut(i as usize) {
                c.skipped = false;
            }
            continue;
        }
        let Some(c) = ev["i"].as_u64().and_then(|i| cases.get_mut(i as usize)) else { continue };
        let i = ev["i"].as_u64().unwrap() as usize;
        c.ms = ev["ms"].as_f64();
        if let Some(e) = ev["err"].as_str() {
            c.error = Some(e.trim().to_string());
        } else {
            let got = ev.get("out").cloned().unwrap_or(Value::Null);
            c.passed = q.compare.equal(&tests[i].0.expected, &got) && c.ms.is_none_or(|ms| ms <= limit_ms);
            c.got = Some(if c.kind == Kind::Stress { clip_value(&got) } else { got });
        }
    }
    // A test that began but never reported: the process crashed or ran out of time on it.
    let crashed_on = current.filter(|&i| cases.get(i).is_some_and(|c| c.got.is_none() && c.error.is_none()));
    if let Some(i) = crashed_on {
        cases[i].error = Some(if o.timed_out {
            "Time limit exceeded.".into()
        } else {
            let tail: String = o.stderr.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
            format!("The program crashed (exit code {}).\n{}", o.code.map_or("none".into(), |c| c.to_string()), tail.trim())
        });
    }
    let first_bad = cases.iter().find(|c| !c.passed && !c.skipped);
    let status = match first_bad {
        None if cases.iter().all(|c| c.passed) => Status::Accepted,
        None => {
            // Nothing ran at all: the driver died before the first test.
            if let Some(c) = cases.first_mut() {
                c.skipped = false;
                c.error = Some(format!("The program exited before running any test.\n{}", clip(o.stderr.trim(), 2_000)));
            }
            Status::RuntimeError
        }
        Some(c) if o.timed_out && crashed_on.is_some() || c.ms.is_some_and(|ms| ms > limit_ms) => Status::TimeLimit,
        Some(c) if c.error.is_some() => Status::RuntimeError,
        Some(_) => Status::WrongAnswer,
    };
    if status == Status::TimeLimit {
        if let Some(c) = cases.iter_mut().find(|c| !c.passed && !c.skipped) {
            c.error.get_or_insert_with(|| format!("Time limit exceeded ({:.0} ms).", c.ms.unwrap_or(0.0)));
        }
    }
    report(status, None, cases)
}

/// Huge stress values are shown shortened.
fn clip_value(v: &Value) -> Value {
    let s = v.to_string();
    if s.len() <= 600 { v.clone() } else { Value::String(clip(&s, 600)) }
}

/// Builds full-size tests from the question's Python generator and reference solution.
/// Returns none when the question has no generator or Python is missing.
pub async fn stress_tests(q: &Question, count: u32) -> Vec<Test> {
    if q.stress.trim().is_empty() || q.reference.trim().is_empty() {
        return vec![];
    }
    let Ok(tmp) = tempfile::tempdir() else { return vec![] };
    let dir = tmp.path();
    let (maker, sig) = (gen::stress_maker(), gen::sig_json(q));
    let files = [("reference.py", q.reference.as_str()), ("stress.py", q.stress.as_str()), ("make.py", maker.as_str()), ("sig.json", sig.as_str())];
    if files.iter().any(|(n, b)| std::fs::write(dir.join(n), b).is_err()) {
        return vec![];
    }
    let mut c = command(python());
    c.args(["-X", "utf8", "make.py", &count.to_string()]);
    match exec(c, dir, Duration::from_secs(120)).await {
        Ok(o) if o.code == Some(0) => o
            .stdout
            .lines()
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .filter(|v| v["args"].as_array().is_some_and(|a| q.check(a, &v["expected"]).is_ok()))
            .map(|v| Test { args: v["args"].as_array().unwrap().clone(), expected: v["expected"].clone(), example: false, explanation: None })
            .collect(),
        Ok(o) => {
            tracing::warn!(id = %q.id, "stress generator failed: {}", clip(&o.stderr, 500));
            vec![]
        }
        Err(e) => {
            tracing::warn!(id = %q.id, "stress generator: {e}");
            vec![]
        }
    }
}

#[test]
fn java_unpublic_keeps_lines() {
    let src = "public class Codec {\n    public String f() { return \"public class \"; }\n    public static final class X {}\n}\n";
    let out = java_unpublic(src);
    assert!(out.starts_with("class Codec {") && out.contains("public String f()") && out.contains("public static final class X"));
    assert_eq!(out.lines().count(), src.lines().count());
}

#[cfg(test)]
mod tests;
