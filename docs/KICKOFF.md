# First prompt to paste into Claude Code (run it from the repo root)

Read CLAUDE.md fully, then prompts/, then skim ui/onsite-artifact-reference.html.
We're starting milestone M1.

Before writing code:
1. Run `claude --help` and confirm the exact flags for: stream-json input/output, partial messages,
   restricting or disabling tools, appending vs replacing the system prompt, choosing a model.
   Update the brain/ section of CLAUDE.md with what you find.
2. Check whether the Tauri prerequisites are installed on this machine (Rust, WebView2, MSVC build tools).
   If something is missing, tell me exactly what to install. Don't install system software yourself.
3. Propose the M1 file layout and the Rust types for ClaudeSession and its events. Wait for my OK.

Then build M1 in small, tested commits.
