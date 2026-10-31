//! Finds Claude Code (Windows first, then WSL) and builds isolated command lines for it.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClaudeLocation {
    Windows { exe: PathBuf },
    /// Runs `claude` through `wsl.exe -e bash -lc`.
    Wsl,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tools {
    None,
    Web,
}

impl ClaudeLocation {
    pub fn describe(&self) -> String {
        match self {
            // `Windows` is the native install on every OS; only the label says which.
            Self::Windows { exe } if cfg!(windows) => format!("Windows: {}", exe.display()),
            Self::Windows { exe } => exe.display().to_string(),
            Self::Wsl => "WSL: claude".to_string(),
        }
    }

    /// A path as `claude` itself will see it.
    pub fn path_arg(&self, p: &Path) -> String {
        match self {
            Self::Windows { .. } => p.display().to_string(),
            Self::Wsl => to_wsl_path(&p.display().to_string()),
        }
    }

    pub fn command(&self, args: &[String], cwd: &Path) -> Command {
        let mut cmd = match self {
            Self::Windows { exe } => {
                let mut c = Command::new(exe);
                c.args(args).current_dir(cwd);
                c
            }
            Self::Wsl => {
                let quoted: Vec<String> = args.iter().map(|a| sh_quote(a)).collect();
                let line = format!("cd {} && exec claude {}", sh_quote(&self.path_arg(cwd)), quoted.join(" "));
                let mut c = Command::new("wsl.exe");
                c.args(["-e", "bash", "-lc", &line]);
                c
            }
        };
        hide_console(&mut cmd);
        cmd.kill_on_drop(true);
        cmd
    }
}

/// Flags shared by every call: no settings, no MCP, no slash commands, our system prompt only.
pub fn isolation_args(tools: Tools, model: &str, system_prompt_file: Option<&str>) -> Vec<String> {
    let tools = match tools {
        Tools::None => "",
        Tools::Web => "WebSearch,WebFetch",
    };
    let mut args: Vec<String> = [
        "-p", "--tools", tools, "--setting-sources", "", "--strict-mcp-config",
        "--disable-slash-commands", "--model", model,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(f) = system_prompt_file {
        args.extend(["--system-prompt-file".to_string(), f.to_string()]);
    }
    args
}

#[cfg(windows)]
fn hide_console(cmd: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_cmd: &mut Command) {}

pub fn to_wsl_path(win: &str) -> String {
    let b = win.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        let rest = win[2..].replace('\\', "/");
        format!("/mnt/{}{}", (b[0] as char).to_ascii_lowercase(), rest)
    } else {
        win.replace('\\', "/")
    }
}

pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Windows PATH and the native installer's location, then WSL.
pub async fn detect() -> Option<ClaudeLocation> {
    if let Some(exe) = find_windows_exe() {
        return Some(ClaudeLocation::Windows { exe });
    }
    if cfg!(windows) && wsl_has_claude().await {
        return Some(ClaudeLocation::Wsl);
    }
    None
}

fn find_windows_exe() -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) { &["claude.exe", "claude.cmd"] } else { &["claude"] };
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local").join("bin"));
        if !cfg!(windows) {
            // Where the npm and older installers put it.
            dirs.extend([home.join(".claude").join("local"), home.join(".npm-global").join("bin")]);
        }
    }
    if !cfg!(windows) {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    }
    dirs.iter().flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

async fn wsl_has_claude() -> bool {
    let mut c = Command::new("wsl.exe");
    c.args(["-e", "bash", "-lc", "command -v claude"]);
    hide_console(&mut c);
    matches!(c.output().await, Ok(o) if o.status.success() && !o.stdout.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wsl_paths_and_quotes() {
        assert_eq!(to_wsl_path(r"C:\Users\me\AppData\Local\Temp\x"), "/mnt/c/Users/me/AppData/Local/Temp/x");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote(""), "''");
    }

    #[test]
    fn no_tools_means_empty_tools_flag() {
        let a = isolation_args(Tools::None, "sonnet", Some("p.md"));
        let i = a.iter().position(|x| x == "--tools").unwrap();
        assert_eq!(a[i + 1], "");
        assert!(a.ends_with(&["--system-prompt-file".into(), "p.md".into()]));
        assert!(isolation_args(Tools::Web, "opus", None).contains(&"WebSearch,WebFetch".to_string()));
    }
}
