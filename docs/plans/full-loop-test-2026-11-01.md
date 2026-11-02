# Full-loop pre-release test, 2026-11-01

Branch m1, commit b82ac69. No code changed. The owner's onsite.exe (pid 13924, started by `cargo run`) stayed running and was not touched.
Claude Code 2.1.316 on Windows. VoiceStudio: not running (`127.0.0.1:3900/health` gave nothing). A USB microphone exists (seen in the app log).

## Results

| # | Area | Result | Evidence |
|---|---|---|---|
| 1a | `cargo test` (separate target dir) | PASS | lib 170 passed, 0 failed, 7 ignored; `ats_files` 8 passed; doc-tests 0 |
| 1b | `cargo clippy -- -D warnings` | PASS | finished clean (one harmless incremental-cache "access denied" note because onsite.exe locks files) |
| 1c | `node --check ui/app.js ui/tour.js` | PASS | no output, exit 0 |
| 1d | `cargo test -- --ignored` (question bank in Java, g++, node, python) | PASS (6 of 7), 1 SKIPPED | see section "Ignored tests" |
| 2 | `ats_check` on one.pdf, clean.docx, clean.rtf, messy.docx, scan.pdf, with and without a job json | PASS with one bug | Scores sane: clean files 100 parse, messy.docx flags two columns, text box, header-only contact, table, image; scan.pdf reads as empty with 0 on all six ATSs; the job file lowered match to 83 and listed "Rust" as missing. Bug B1 below (en dash in a tip). |
| 3 | Bundled projects (8) | PASS with one bug | `cargo test` project loader tests passed. `git apply --check` passes for 7 of 8. ttl-cache-python fails (B2). Diffs applied in temp copies: removed lines are present before, gone after (moved lines excepted). Python compiles, JS passes `node --check`, checkout-retries-java and wallet-transfer-java compile with javac after the fix. orders-consumer-java needs the Kafka client jar: SKIPPED. webhook-receiver-go: SKIPPED (no go). The project test files are "not run" by design (they need Redis, Postgres, Kafka), so planted-bug behaviour was checked by diff only, not by running tests. |
| 4 | Claude Code health check and flags | PASS | `claude -p "Reply with OK" --output-format json` ok in 6.6 s. Interviewer flag set (`--tools "" --setting-sources "" --strict-mcp-config --disable-slash-commands --system-prompt-file`) accepted. init event: `"tools": []`, `"mcp_servers": []`, `"slash_commands": []`, `"skills": []`. 2 model calls total. Note: init also lists built-in `agents` and builtin plugins (cc-plugin-agents-md, telemetry, plugin-authoring). Tools are empty so they are inert, but CLAUDE.md says tested on 2.1.310; this is 2.1.316. |
| 5 | UI with mocked backend (system Chrome via playwright-core) | PASS with notes | Screens in `scratchpad/img/`. All 8 Settings sections render. About: text correct, version pill filled "v0.1.2", 11 link buttons, narrow width (390 px) has no horizontal scroll (overflow 0). Light themes Nord, Rosé Pine and Onsite light: pill colors follow tokens (dark text on filled light chip). Gruvbox dark ok. Console: one page error only from the mock (see N1). |
| 5b | `split(":")` label parsers (app.js:782, 783, 856) | PASS | Temperament options give "Supportive", "Realistic", "Bar-raiser". Panel options give "Indian Tech Panel", "International Tech Panel". Summary line read "Realistic · Mixed interviewers", then "Bar-raiser · Indian Tech Panel" after changing both. Mixed uses a fixed string. Dossier label "Razorpay: SDE 2, backend" is not split anywhere (only displayed in the select). |
| 5c | Comprehension option in setup | PASS | Tile "Code comprehension and review" adds step to the loop; the project note appears with "Use a ready-made project instead". |
| 5d | Dash regressions | FAIL (B1) | No raw em or en dash in ui, prompts, docs/engineering. `decClass` maps "Borderline, more data needed" to `d-lean` (app.js:1967) correctly. Feedback modes keep em dash escapes in `ui/app.js:2753` and `src-tauri/src/feedback.rs:27` to match the live form: intended. B1 is a user-visible en dash escape. |
| 6 | Real app over CDP | SKIPPED | See below. |
| 7a | Secrets in log | PASS | `%LOCALAPPDATA%\Onsite\onsite.log` (579 lines): 0 matches for sk-, sk-ant, AIza, gsk_, Bearer, api_key, x-goog-api-key; 0 matches for hidden-issue text. I did not read the `api_keys` row. |
| 7b | Interviewer no tools | PASS | init `tools: []` in my own run with the same flags (step 4). |
| 7c | Privacy text vs data flow | PASS | Text in `ui/index.html` (Privacy section) matches code: log has no transcripts or keys, updates only on Check, feedback only on Send, Groq/OpenAI etc. only via chosen provider. |
| 7d | open_link whitelist | PASS (code review) | `commands.rs:656` matches fixed keys only, else `Invalid("Unknown link.")`. All 11 `data-link` keys in index.html exist in the match. In the mock, clicking "repo" invoked `open_link {"which":"repo"}`. Not invoked against the real OS. |

## Bugs, ranked

1. **B1, low/medium (dash rule, user-visible).** `src-tauri/src/ats/score.rs:420` builds the tip with `\u{2013}` (en dash): `Jan 2020` then an en dash then `Mar 2023`. It shows in Resume check. Fix: use `"Jan 2020 to Mar 2023"` or a hyphen. The text.rs:231 regex that matches dashes in resumes is fine (it parses input).
2. **B2, low (data).** `projects/ttl-cache-python/solution.diff` is a corrupt patch (`git apply` says corrupt patch at line 15). The second hunk header counts do not match, and the last line is a prose note inside the diff. It is only the hidden reference text, so the round still works, but the interviewer reads a damaged reference. Fix: regenerate with `git diff --no-index` and move the "(Also: ...)" note into `project.json` good_fix text.
3. **B3, low/medium (temp hygiene).** `%TEMP%\onsite-round-3re7io\.system-prompt.md` (17 KB, 21:46) survived with no claude process using it. It contains the hidden "Planted issues ... HIDDEN" block and the reference fix for a comprehension round. Also four empty `onsite-job-*` dirs from 22:02 to 22:18. `TempDir` cleans on drop, so this happens when the process is killed or restarted mid-round (likely the dev `cargo run` being restarted). Suggested fix: on startup delete `onsite-round-*` and `onsite-job-*` older than the process start (Storage "delete temp" may already do it; the startup sweep does not seem to). Unconfirmed cause.
4. **N1, info (mock only).** `ui/app.js:2977` throws when `storage_info` has no `counts` (`docs/engineering/tools/shots-mock.js` returns `{items,total}`). The real command returns counts and limits, so not a product bug. Fix the mock if you keep taking Storage screenshots. A guard in the UI would also avoid a blank section.
5. **N2, info.** The hidden issues and reference fix DO go into the interviewer system prompt on purpose (`projects.rs:212-241`, "HIDDEN, never read out"), and a snapshot copy is saved in the round's workspace (`interview/live.rs:705`, `846`, `interview/mod.rs:283`) for the report. `Project::view()` (projects.rs:163) sent to the UI has no issues, solution or good_fix. The test brief asked that they never reach the interviewer prompt; they do, by design. Confirm that is what you want.
6. **N3, cosmetic.** About: the "Full notices" link sits alone under the list with different indent. On a 390 px screen the "New here? Show me around" button and tour banner wrap tightly.

## Ignored tests

Run with `-- --ignored`: 6 of 7 pass: `nested_types_and_doubles`, `quadratic_solution_fails_the_stress_test`, `returning_the_original_graph_is_caught`, `bank_tests_agree_with_reference_solutions` (every reference), `new_shapes_run_in_every_language`, `starter_code_compiles_and_fails_cleanly`. The seventh, `voice::whisper::tests::whisper_live_next_to_tts`, needs VoiceStudio running and a WAV (`ONSITE_TEST_WAV`); VoiceStudio is down, so it hung and I killed it. SKIPPED. The question-bank runner ran Java, g++, node and python only if those tests exercise Go: Go is not installed, so the Go path either skips or is not covered by this run (not verified). The first attempt's output was lost when the shell was backgrounded, so the run was repeated; the second run is the evidence.

## User journeys

Mode: **M** = mocked UI in Chrome, **S** = static code or unit tests, **C** = command line, **X** = SKIPPED.

| # | Journey | Mode | Result |
|---|---|---|---|
| 1 | First run: provider default, AI model health check, About, Privacy, Storage | M, C | Health check command PASS in CLI (step 4). About, Privacy and Storage render (Storage hits N1 in the mock only). In-app health button: X (real app not driven). |
| 2 | Setup: General vs Company, every round tile (9 present: resume, project, fundamentals, coding, comprehension, lld, sysdesign, behavioral, hm), temperament, panel, dossier select | M | PASS. Mode switch works, dossier select shows "Razorpay: SDE 2, backend". Level, language, duration inputs were not each cycled. |
| 2b | Resume and JD drop: PDF, docx, rtf, bad file, empty | S, C | Extraction covered by `ats_files` tests (pdf, docx, rtf, password pdf, scanned pdf) and `ats_check`. Actual drop in the UI: X. |
| 2c | Pick each of 8 bundled projects, brief, tree, readonly test file | S | Loader tests pass; every project.json has a readonly test file and an open file. UI file tree: X (needs a live round). |
| 3 | Round lifecycle: Get ready, resume block, loader steps, Start typed, cancel_start, send turn, workspace share, run/submit, [[END]], End early, timer, crash resume, report, verdict, history, dashboard, weekly goal | S, M | Unit tests (170) cover splitter, [[END]], session queue/crash, runner in every language (`--ignored`). Dashboard and history render from mock history. Live typed round: X (see below). |
| 4 | Comprehension: edit file, diff to interviewer, hidden data not in UI | S | `Project::diffs` reviewed: only changed files, read-only skipped, unsafe paths ignored. `view()` has no hidden fields (N2 for the prompt). Log has no hidden text. Live edit: X. |
| 5 | Dash regressions | M, S | See 5, 5b, 5d. B1. |
| 6 | Settings sections, themes x light/dark, storage limits, clear log, Updates, feedback, About links | M, S | Sections render. Themes: 5 families checked in light or dark for About. System mode not cycled. Clear log, limits, Updates Check/Install, Send feedback submit: X by rule (not pressed). Feedback form modes carry the escape (view only). |

## SKIPPED, with reasons

- **Real app over CDP (step 6 and journeys 1, 3, 4 live parts).** The running onsite.exe was started without a remote debugging port, and port 9222 on this machine belongs to another program (LenovoVantage). Restarting it would disturb the owner's session. Starting a second copy is unsafe: Tauri's app data dir ignores the APPDATA env var, so a second copy would open the owner's real `onsite.db` (which holds an `api_keys` row and 26 loops), run startup retention and "running jobs become failed" logic, warm up the voice engine on the GPU, and write test rounds into their history. I did not do that. A person can launch with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9333` on a spare Windows profile and rerun `scratchpad/ui.mjs` style checks via `connectOverCDP`.
- No live typed round, so no TTFT numbers, loader steps, workspace share, report, verdict or history entry were checked on the real app. The Claude quota use was 2 one-shot calls.
- Voice: microphone, speakers, VoiceStudio (not running), Whisper sidecar, built-in voice engine, barge-in.
- Claude API, OpenAI, Gemini, Groq: no keys.
- macOS and Linux builds, installer, updater (Check and Install), uninstall, code signing, GitHub release and experimental workflows, website deploy.
- Go project (webhook-receiver-go): no Go toolchain. orders-consumer-java: Kafka jar missing for javac.
- Google Form feedback submission: not submitted.
- MCP server (phase 3): not tested.

## Side effects

- App data `%APPDATA%\dev.onsite.app`: before and after row counts identical (settings 13, dossiers 1, research_jobs 0, loops 26, rounds 1, reports 1). Opened read-only only. No file added or removed in the data dir by me (the db-wal changes were the owner's running app).
- `%LOCALAPPDATA%\Onsite\onsite.log`: read only.
- Repo git status: clean apart from two untracked plan files I did not create (`docs/plans/ai-assistant-variant.md`, `docs/plans/next-up.md`) and this report.
- Build output went to `src-tauri/target/full-loop` (gitignored; delete when done). Test temp copies of projects were made under the system temp dir via `mktemp` (bash) and are small.
- Processes: I started node/chrome for screenshots (closed), `claude` one-shots (exited), cargo test (finished). The owner's onsite.exe (13924) and its parent cargo are unchanged. No claude.exe or python sidecar is tied to onsite temp dirs.
- Network: only the two Claude Code calls to the owner's Claude login. No Updates check, no feedback request, no GitHub call by me.
- Stray temp items found, not mine: B3.

## Test data created

None in the app. `scratchpad/job.json` has company "ZZ-TEST" and lives only in the session scratchpad.
