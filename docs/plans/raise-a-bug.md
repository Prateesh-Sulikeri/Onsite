# Raise a bug: design (reports go to GitHub issues)

Status: built (2026-11-02). Code in `src-tauri/src/report.rs`; engineering notes in `docs/engineering/REPORT.md`.
Manual checks (Windows opener, live GitHub prefill) are not done yet, see Test plan.

## Decisions (owner, 2026-11-02)

1. GitHub only. The Google Form stays as the general survey; it has no bug mode.
2. Questions go to GitHub Discussions. The app has two categories, Bug and Idea. `question.yml` is deleted;
   `config.yml` has an "Ask a question" contact link. `open_link` keys `discussions` and `issues_search` exist, and the
   report view links to Discussions.
3. "Copy recent log lines" ships in v1, after an audit of every log call site (below).
4. The sidebar keeps "Send feedback" (the survey) and gets its own entry "Report a bug" next to it (owner's call: nobody would look for it under feedback). It opens the report view with Bug preset; Idea is selectable inside.
5. Reply target: within a week, as `SECURITY.md` says.

## Log audit (2026-11-02)

Every `tracing::` call in `src-tauri/src` was read. Fixed because they could log content:
- `voice/stt.rs`: debug line logged the raw speech transcript (`%raw`). Now logs character counts only.
- `interview/live.rs`: "report JSON did not parse" logged the first 500 chars of the report text, which quotes the
  transcript. Now logs the length.
- `brain/session.rs`: debug line logged non-JSON stdout lines (model output). Now logs the length. Claude stderr lines and
  the "exited mid-round" line are clipped.
- `brain/oneshot.rs`: a failed job logged the full result text and stderr. Clipped.
- `brain/api.rs`: provider error bodies (several sites) and network errors. Clipped.
- `research.rs`: research failure message. Clipped. `commands.rs::ui_log`: UI error text (4000 chars). Clipped.
Clipping is `error::log_clip` (one line, 300 chars). Everything else logs ids, counts, timings, device and model names, or
our own fixed messages. Residual risk: a provider that echoes request text inside a short error body would show up to 300
chars of it. The report's redaction (keys, paths, private-run line drop) runs over the log again before it is copied.
Not logged by design: speech, transcripts, resume, JD, dossier text, keys.

## Goal

A friend hits a problem. In two clicks they see a full report with the facts, edit any line, and land on the
Onsite new-issue page with the form already filled. They press Submit on GitHub. Onsite sends nothing itself.

## Entry points

1. Sidebar `.side-foot`: new "Report a bug" entry (bug icon) beside "Send feedback". "Send feedback" still opens the survey.
2. Settings > About: "Report a bug" button.
3. Error toasts (`toast(msg, "err")`): a "Report this" action. Opens the form as category Bug with the error text filled in.
4. Top of the survey: one small line pointing to the sidebar entry.

All open one view, `#view-issue`, with the category preset. The survey form (`#fbForm`) keeps its questions; its
"Add app details" box is retired, since bugs now go through the report.

## The form (category first)

Category: Bug (default) | Idea. Fields match the issue templates by id so the URL prefill lines up.

| Category | Field ids (template) | Notes |
|---|---|---|
| Bug | what, steps, expected, actual, app_version, os, ai, voice, view, error, log | app_version, os, ai, voice, view, error are auto-filled |
| Idea | problem, idea, alternatives | no auto context |

The "searched existing issues" and "no private text" checkboxes live on GitHub (checkboxes cannot be prefilled by URL).
The app shows a line before opening: "Search existing issues first" with an "Open issues" link (`open_link issues_search`, fixed key).

## Auto context (non-private only)

- version: `app_version`.
- os: Windows, macOS, Linux (AppImage) or Linux (.deb). Same labels as the template dropdown.
- ai: provider name only (Claude Code, Claude API, OpenAI, Gemini, Groq). Not the model id, not the key.
- voice: Typed, Spoken (built-in voice), Spoken (VoiceStudio), Not in a round.
- view: the name of the screen the user was on (nav key such as `setup`, `call`, `report`), never its content.
  Recorded by the router in `app.js` into a variable `lastView` (the view before the report view opened).
- error: last error toast text, kept in `lastErr` by `toast()`. Redacted in Rust (below).

## Preview

Single screen. Every field is a plain editable box or select, nothing hidden. A read-only "Exactly what will be sent
to GitHub" panel shows the final title and body. Each auto field has a small "remove" control that blanks it.
Nothing leaves the app until the user presses "Open on GitHub". Every edit re-runs `report_preview` (debounced, ~300 ms) so the box always shows the redacted text. The user then reviews it again on GitHub and presses Submit.

```
+------------------------------------------------------------+
| Feedback or bug                                            |
|  ( Bug )  ( Idea )                                         |
|                                                            |
| What happened *   [ The voice stopped after the 2nd turn ] |
| Steps *           [ 1. Start a round  2. Speak ...       ] |
| What you expected [ ...                                  ] |
| What happened instead [ ...                              ] |
|                                                            |
| Added by Onsite (edit or remove any line)                  |
|  Version  [ 0.1.4 ] x    System [ Windows v ] x            |
|  AI       [ Groq v ] x   Round  [ Spoken (built-in) v ] x  |
|  Screen   [ call ] x                                       |
|  Error    [ Voice engine failed to start (~/models) ] x    |
|                                                            |
| [ ] Copy recent log lines (off), paste them on GitHub      |
|                                                            |
| This is what opens on GitHub:                              |
| +--------------------------------------------------------+ |
| | [Bug] The voice stopped after the 2nd turn             | |
| | ### What happened ...                                  | |
| +--------------------------------------------------------+ |
| You need a free GitHub account. Nothing is sent from Onsite|
|                                                            |
| [ Open on GitHub ]  [ Copy report ]  [ Cancel ]            |
+------------------------------------------------------------+
```

## Redaction (Rust, `report.rs`)

`redact()` runs in `report_preview` (every edit) and again inside `open_issue` on the final input. The URL never uses text
the preview did not show redacted.

1. Keys: values in `api_keys` and patterns `gsk_[A-Za-z0-9]+`, `sk-[A-Za-z0-9_-]+`, `AIza[0-9A-Za-z_-]{20,}`, `Bearer \S+` become `[key removed]`.
2. Home folder: the home path (`%USERPROFILE%`, `/home/<u>`, `/Users/<u>`, `C:\Users\<u>`) becomes `~`.
3. Other absolute paths shrink to the file name. File names the user chose (`*.pdf|docx|rtf|txt`) become `[file]` in error and log text.
4. Held private strings (resume, JD, dossier, transcript of the current round):
   - Error and log text: a line sharing a run of 40+ characters with them is dropped.
   - Fields the user typed: never dropped silently. The line is flagged in the preview ("This looks like text from your resume")
     and the user decides. A flagged line is still sent if the user leaves it, and `open_issue` accepts a per-line "keep" list from the UI.
5. Log: last 200 lines, max 50 KB, off by default, only goes to the clipboard (see below). Call sites were audited first (above).

Redaction is a safety net. The user still sees and edits everything.

## URL building

What GitHub's docs say (fetched 2026-11-02, "Creating an issue from a URL query"):
- `template` works with templates in an `ISSUE_TEMPLATE` folder.
- "You can also use URL query parameters to fill custom text fields that you have defined in issue form templates." Keys must match
  the form schema; the page links to it and does not state that the key is the field `id`.
- `title` and `labels` work. Adding labels needs permission to label issues (a normal reporter may lack it, so labels come from the template).
- A URL over the server limit returns `414 URI Too Long`. No number is given.
- The page says nothing about dropdowns or checkboxes.

**Not verified yet** (treat as best effort, test live): that field `id` is the key (this is how GitHub's issue-form schema reads), that
dropdowns accept the exact option label, that checkboxes cannot be prefilled, that unknown or unpermitted labels are ignored, the real
length limit, and that a logged-out redirect keeps the params.

Title: the template sets `title: "[Bug] "`. Assume the URL `title` param replaces it, not appends. So the app adds the prefix itself, from one
Rust constant `TITLE_PREFIX` per category (`[Bug] `, `[Idea] `), which must equal the `title:` in each yml (unit test below).
Title = prefix + first line of "what" (max 80 chars).

Safe construction in Rust (new command `open_issue`, not `open_link`):
- Constants: `REPO`, the template map (`bug -> bug_report.yml`, `idea -> feature_request.yml`), `MAX_URL_BYTES = 4000`.
- Input is `{ category, fields: Vec<(id, value)> }`. Each `id` must be in the per-template allow-list. Dropdown values must be one of the
  allowed labels or the field is dropped. Values are redacted, trimmed and capped (2000 chars).
- Build with `reqwest::Url::parse(REPO + "/issues/new")` then `query_pairs_mut().append_pair(k, v)` only. Scheme and host never come from the UI.
  Assert `host_str() == Some("github.com")` and path `/Prateesh-Sulikeri/Onsite/issues/new` before `storage::open_url`.
- `labels` is not sent. The templates set them.
- `open_link` stays fixed-keys only. Keys added: `issues_search` (`/issues`), `discussions`.

Windows opener: `storage::open_url` calls `ShellExecuteW` in process (rundll32 opened no new tab when an issue tab was already open); if it refuses (return <= 32), it falls back to `rundll32.exe url.dll,FileProtocolHandler <url>`. Long URLs through it are still a known risk.
So `MAX_URL_BYTES` starts at 4000 (encoded) and is raised only after the Windows test below passes.

## Single source of truth for dropdown labels

The Rust allow-lists (os, ai, voice) and `TITLE_PREFIX` must match the yml. A unit test uses `include_str!` on
`.github/ISSUE_TEMPLATE/bug_report.yml` (and `feature_request.yml`) and checks that every Rust label is an `options` entry and every
prefix equals the `title:`. A template edit then fails the build instead of silently breaking prefill.

## The log and the fallback

The log never goes in the URL. If "Copy recent log lines" is ticked, the redacted log is copied to the clipboard when the user presses
"Open on GitHub", and the UI says "Paste the log into the Log box on GitHub".

Shrink order when the URL is over `MAX_URL_BYTES`: truncate `error` to 500 chars, then `steps`, `actual`, `expected` to 800 each.
If still over: copy the full report to the clipboard and open the blank template (`issues/new?template=bug_report.yml`, no fields).
UI: "Your report was too long for the link. It is copied. Paste it into the form."
Offline or browser fails to open: the report stays on screen, with Copy report and "Save as a text file" (Downloads).

Clipboard is new code: `ui/app.js` has no `navigator.clipboard` use today. It needs a user gesture in WebView2 (call it inside the
button handler, before any `await` that loses the gesture). Fallback: if it throws, show the text in a selected box with the hint
"Copy failed. Select the text and copy it." No new crate unless this proves unreliable on Linux or macOS.

## Privacy text changes (same commit as the code)

- `ui/index.html` privacy card and Settings > Privacy: "Bug reports, ideas and questions: Onsite shows you the full report and opens a
  GitHub page with it filled in. You submit it there. Onsite itself sends nothing. Anything you submit on GitHub is public and tied to
  your GitHub account." Keep the Google Form text for the survey.
- `PRIVACY.md`: add the GitHub path. "Nothing is sent to the developer" stays true because the user's browser submits.
- `SECURITY.md`: do not report a vulnerability through the in-app report. `CONTRIBUTING.md` "Reporting a bug": use the button or the form.
  The report view also says so.
- `README.md` line 23: point to the in-app button.

## Files that change (implementation phase)

| File | Change |
|---|---|
| `src-tauri/src/report.rs` (new) | input structs, allow-lists, `TITLE_PREFIX`, `redact()`, `build_url()`, context, log tail |
| `src-tauri/src/commands.rs` | `report_context`, `report_preview`, `open_issue`; link key `issues_search` |
| `src-tauri/src/lib.rs` | `mod report`, register commands |
| `ui/index.html`, `ui/app.js`, `ui/app.css` | report view, entry points, `lastView`, `lastErr` in `toast()`, "Report this", clipboard, retire the `fbDetails` append; tokens only |
| `.github/ISSUE_TEMPLATE/*` | done in this pass |
| `PRIVACY.md`, `SECURITY.md`, `CONTRIBUTING.md`, `README.md`, `CLAUDE.md`, `docs/engineering/` | text updates |
| `site/` | Stacy: footer links to `/issues/new/choose`, mention the in-app button |

## Test plan

- Rust unit tests: redaction fixtures (every key prefix, home path on Windows/Linux/macOS, a resume line in log vs in a typed field
  where it is flagged not dropped, a `.pdf` name); allow-lists reject unknown ids and dropdown values; host and path asserts;
  encoding of `&`, `#`, `+`, newlines, unicode; shrink order and `MAX_URL_BYTES`; the template-match test above; no em or en dashes.
- Preview: edits re-run `report_preview` (debounced); `open_issue` output equals the redacted preview for the same input.
- UI: each entry point presets the category; "remove" blanks a field; toast "Report this" fills `error`; clipboard failure shows the select box.
- Windows opener (manual): open 2 KB, 4 KB and 6 KB encoded URLs through `storage::open_url` and confirm the browser gets the full URL.
  Raise `MAX_URL_BYTES` only to the largest size that passes.
- Live GitHub check (manual, after templates and labels are pushed; replaces the old "read docs live" question): on a test issue verify
  each field prefills, dropdown labels match, `title` replaces the `[Bug] ` prefix (not appends), labels from the template apply, checkboxes
  stay unchecked, a logged-out user keeps the params after login, and a 9 KB URL gives 414. Try Edge, Chrome, Firefox on Windows, plus macOS and Linux.
- Privacy: run a test round holding a fake key and a resume line; neither may appear in the URL.

## Labels the owner must create on GitHub

`bug`, `enhancement`, `question`, `needs-triage` (the first three usually exist on a new repo; check). Templates set them, since a reporter
without triage rights cannot add labels by URL.

## Built differently from the first design

- Typed lines that look like private text are flagged in the preview and sent unless the user edits them. There is no
  separate per-line "keep" list; editing the line is the control.
- "Screen" holds a plain screen name (Setup, Call room, Report), mapped in `app.js` from the nav key, to match the template.
- Windows paths containing spaces are cut at the first space by the path rule; the safety net is the preview.
- "Save as a text file" (`report_save`, Downloads) is always available, not only after a failure.
- The clipboard is written before `open_issue` runs, because the browser takes focus once it opens.
