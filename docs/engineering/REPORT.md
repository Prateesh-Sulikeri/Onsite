# Report a bug (`src/report.rs`)

Purpose: a friend reports a bug or idea in two clicks. Onsite builds a GitHub new-issue link with the form filled in
and opens it in the browser. The user presses Submit on GitHub. Onsite sends nothing. Design: `docs/plans/raise-a-bug.md`.

## Map
| Piece | Where |
|---|---|
| Field ids, labels, dropdown options, title prefixes | `report.rs` (`BUG`, `IDEA`, `OS_OPTIONS`, `AI_OPTIONS`, `VOICE_OPTIONS`, `Category::title_prefix`) |
| Redaction | `report::redact`, `drop_private_lines`, `flag_lines`, `PrivateIndex` |
| Link | `report::prepare` then `build_url` (`MAX_URL_BYTES` 4000), `assert_safe` |
| Commands | `report_context`, `report_preview`, `open_issue`, `report_log`, `report_save` (`commands.rs`) |
| Link keys | `open_link`: `issues_search` (`/issues`), `discussions` |
| UI | `#view-issue`, `openIssue` in `ui/app.js` |
| Templates | `.github/ISSUE_TEMPLATE/bug_report.yml`, `feature_request.yml`, `config.yml` (Discussions link) |

## Rules
- The version field id is `app_version`, not `version`: GitHub ignores a `version` query key when prefilling (found live, 2026-11-02).
- Categories are Bug and Idea. Questions go to GitHub Discussions (no question template).
- `report_preview` and `open_issue` run the same `prepare`, so the link never holds text the preview did not show.
  The UI never sends a URL, host or labels. Fields must be in the per-category allow-list; dropdown values must be a listed label.
- Redaction (safety net, the user still sees and edits everything): API keys (stored values and key shapes), home folder
  to `~`, other absolute paths to the file name, user document names to `[file]` (error text only). Error text and log
  lines sharing a run of 40+ characters with the current round's transcript or the last resume check are dropped.
  Typed fields are flagged in the preview, never dropped.
- Too long: error text is cut to 500 chars, then steps/expected/actual (or the idea fields) to 800, else the blank
  template opens and the UI copies the full report. The log never goes in a link: "Copy recent log lines" copies the last
  200 lines (max 50 KB), redacted, to the clipboard.
- `report.rs` tests read the yml files with `include_str!`. Editing a template (id, label, dropdown option, title) without
  editing the Rust constants fails `cargo test`.
- The reply target is one week, as in `SECURITY.md`.

## The log must stay clean
The log holds errors and timings only: never speech text, transcripts, resume, job description, dossier text or keys.
Any `tracing::` call that logs text we do not control (provider error bodies, CLI output, UI messages) goes through
`error::log_clip` (one line, 300 chars). Log lengths, not content (`stt.rs` logs `raw_chars`, not the words). When you add a
log line, check what the value could hold. Audit of 2026-11-02 and its fixes: `docs/plans/raise-a-bug.md`.

## Opening the link
Windows: `storage::open_url` calls `ShellExecuteW` (`shell_open`), falling back to rundll32 when it returns <= 32. Only
`assert_safe` links, fixed `open_link` keys and `https://` URLs reach it.

## Not verified (needs a person)
Windows opener with 2, 4 and 6 KB links; live GitHub prefill (field id as key, dropdown labels, `title` replacing the
template prefix, labels, 414 limit, login redirect); macOS and Linux openers.
