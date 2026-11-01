# CODING: question bank, runner, workspace

Code: `src/bank.rs`, `src/runner/{mod,gen,tests}.rs`, `prompts/questions.csv`, `src/interview/live.rs::{run_code,set_round_lang,problem_view,problem_block}`;
UI: `ui/app.js` *coding problem + runner*, editor `ui/vendor/editor.js` (built from `ui-vendor/src/editor.js`), whiteboard
`ui/vendor/whiteboard/` (from `ui-vendor/src/whiteboard.js`).

## Bank (`questions.csv`, 81 rows: 20 Easy, 50 Medium, 11 Hard; 20 originals + rest of Blind 75)

Columns: `id title difficulty levels topics companies function params returns compare statement constraints tests hints
time space follow_ups reference stress kind methods` (`;` lists, `|` hints/follow-ups, `tests` = JSON
`[{args, expected, example?, explanation?}]`, `reference` = Python solution, `stress` = Python generator).
Edit in a text editor only (Excel re-quotes header, scrambles ids).

Types: `int long double bool string char T[] list<T> ListNode TreeNode Node void`. ListNode `[1,2,3]` or `[[vals],pos]`
(cycle); TreeNode level order with nulls; Node = Clone Graph adjacency (node i+1 at index i; returning an input node =
error); void = in-place, result is first arg after call.
Kinds: `""` (76) `Solution.<function>`; `design` (3) class + LeetCode operations list (`methods` lines
`insert(word: string) -> void`); `roundtrip` (2) Codec `decode(encode(x)) == x` on two instances (JS: two top-level fns).

`Bank::parse` skips bad rows into `Bank.errors` (not shown in UI: a gap). `Question::check` compares per `compare`.

Picking: company loop → `match_reported(questions, seed)` (title words minus stop words/plural s; ≥ 60% coverage; most
words, then seeded tie-break); else `pick(difficulty_for(level), topics, company, used, seed)` = same difficulty → max
(2×topic overlap + company tag) → seeded shuffle. `difficulty_for`: ng Easy, senior/staff Hard, else Medium.
Candidate sees `problem_view` (statement, examples, constraints, hidden count, runnable, starter). Interviewer gets
`problem_block` (+ optimal time/space, hints one at a time, follow-ups; told problem is on screen, don't read it).

## Runner (`runner::run(q, lang, code, tests)`)

| Lang | Files | Compile | Run | per-test limit |
|---|---|---|---|---|
| Python | `Solution.py driver.py sig.json` | - | `python -X utf8 driver.py Solution.py sig.json tests.json` | 6 s |
| JS | `solution.js driver.js sig.json` | - | `node driver.js …` (worker, 256 MB stack) | 3 s |
| Java | `Solution.java` (imports on line 1, `public` stripped from types: `java_unpublic`), `Main.java` (generated per question) | `javac -encoding UTF-8 -nowarn -d out` | `java -Xss512m -XX:+UseSerialGC -cp out Main tests.json` | 2 s |
| C++ | `solution.cpp main.cpp` (generated) | `g++ -std=c++17 -O2` (+`-static -Wl,--stack,268435456` on Windows) | `main.exe tests.json` | 2 s |

- Compile limit 90 s; run budget = 3 s + limit × tests, ≤ 60 s; timeout → `kill_tree` (3 s grace).
- Output protocol: lines prefixed `\x1e`: `{"begin":i}`, `{"i":i,"out":…,"ms":…}` | `{"i":i,"err":…}` | `{"compile":…}`.
- `Status {Accepted, WrongAnswer, RuntimeError, TimeLimit, CompileError}`; `Report {status, lang, compile_error, cases, passed, total, max_ms}`;
  `Report::summary("Run"|"Submit")` → one line stored in `ActiveRound.pending_run`, sent with next answer as `[Code run] …`.
- Run = example tests; Submit = all tests + `stress_tests(q, 1)` (Python `stress` + `reference`, kept only if it passes
  `q.check`; built once per round outside the lock, 120 s limit).
- Python: PATH, else VoiceStudio's bundled Python; skips WindowsApps stub (`runner::python`). TypeScript/Go: editable, not runnable.
- `set_round_lang`: switches language, returns starter + runnable; starter not sent until edited.

## Workspace (UI)

Tabs: Editor (CodeMirror 6: Java/Python/C++/JS/TS/Go, bracket close, indent, completion, syntax lint) · Whiteboard
(Excalidraw 0.18, lazy `import()`; `OnsiteBoard.describe()` → text: shapes, containment, labelled arrows, loose text;
dark only) · Notes (per-round template) · From interviewer (interviewer code blocks, picker). Problem panel above transcript
with draggable split. Sent only when changed (`candidate_message`), clipped (`EDITOR_MAX`, `WHITEBOARD_MAX`); Share now = force.
`BOARD_ROUNDS = {project, lld, sysdesign}`.

## Requirements

WS-R1 diff-only send + Share now · R2 clipping · R3 resizable split · R4 `[Code run]` in next message · R5 time budget +
tree kill · R6 missing-toolchain message + Python fallback · R7 all references/starters tested in all langs · R8 whiteboard
theme follows app (**planned**).

## Change recipes

- **Add a problem:** new CSV row (copy similar; header untouched) → examples flagged `"example": true` + hidden tests →
  Python `reference` (+ `stress` for Submit) → `cargo test -- --ignored` (needs javac, g++, node, python) until all pass.
- **Add a runnable language:** `Lang` variant (`from_name`, `label`, `limit_ms`, test-only `ALL`) → `plan` → driver in `gen.rs` →
  starter generation → `toolchains` (test-only) → tests in `runner/tests.rs`.
- **Editor change:** edit `ui-vendor/src/editor.js` → `cd ui-vendor && npm install && npm run build` → commit `ui/vendor/`.

## Projects (comprehension round, `src/projects.rs`, `src/projgen.rs`, `prompts/project.md`, `projects/`)

Where the project comes from (`live.rs::choose_project`, loader step `project`): (1) the AI writes one for this job (`projgen::generate`: report model, no tools,
inputs = JD first, then the company dossier and this round's reported questions, then the resume, plus a scenario hint and the last 6 titles to avoid; the resume is always
present, so every loop has input); (2) if that fails twice or the candidate ticked "Use a ready-made project" (`LoopSetup.ready_made_project`), a bundled one in the same language;
(3) none in that language: the interviewer writes one file, as before. Setup shows a warning note (`#projNote`) when the round is picked, and `openRoom` asks in `#projDlg` before each comprehension round ("Write it with AI" / "Use a ready-made project" / Cancel; answer kept per round in `loop.projChoice`, passed as `start_round(ready_made)`; the setup tick skips the question). Generated projects are not cached
(each round should differ); recent titles are in setting `projgen:recent`.
`Project::from_generated` checks the model's JSON: 2-5 files, language extensions, safe paths, size limits, 2-4 issues in editable files with line numbers inside the file,
real `why`/`good_fix`, and no comment containing words like bug/TODO/race. A rejection is sent back as one retry. Nothing generated is compiled or run, so a planted bug can be wrong:
that is the known risk and the reason for the setup warning.

One folder per project: `project.json` (schema 1: id, title, language, difficulty, levels, topics, companies, minutes, `brief`,
`files[{path, open, readonly}]`, hidden `issues[{id,file,lines,category,why,good_fix}]`, hidden `extension`), the files, hidden `solution.diff`.
`Projects::load` skips bad folders into `errors` (unsafe path, > 8 files, 40 KB a file, 120 KB total, issue naming a missing file). Bundled via
`tauri.conf.json` resources; dev reads the repo folder. `Projects::pick(lang, level, company, recent, seed)`: language match first, then level, then company, then seeded shuffle.
- `start_round` (comprehension only) adds `Project::interviewer_block()` to the system prompt (brief, stages, hidden issues, reference fix, all files with line numbers)
  and returns `RoundInfo.project` (`Project::view()`: no issues, extension or fix).
- Nothing runs. UI keeps every file's text; `currentWorkspace` sends `files` (editable ones). `send_answer` turns them into unified diffs against the originals
  (`Project::diffs`, crate `similar`): the message carries only files changed since the last send (`last_ws.files`), Share now carries all differing files;
  `Workspace.diff` is that text. The saved snapshot holds the full diff and `issues`; `report.rs` then grades each issue found/fixed/fixed badly/missed.
- New files: up to 6, safe relative paths only (`safe_path`); read-only files are ignored in diffs. `project_diff` command feeds the Changes view.
- UI (`ui/app.js` *multi-file project*): file tree, open-file tabs, dirty dots, Reset (press twice), New file, Changes. One CodeMirror view; switching files swaps the
  document (undo history is not kept per file). `editor.js` gained `setReadOnly`. "Copy to editor" on an interviewer code block: new file in a project round,
  else appended to the editor (never overwrites).
- Layout (app.js *resizable panes*): drag bars `#colSplit` (conversation | workspace, share in localStorage `onsite.colShare`, 0.15-0.8) and `#treeSplit`
  (tree width `onsite.treeW`); double-click resets, arrow keys nudge. Editor always wraps lines (`EditorView.lineWrapping`). Right-click on the tab bar: Close, Close others,
  Close unchanged, Close all (no tab left shows a hint). Shortcuts list (F1 or the Shortcuts button) is `SHORTCUTS` in app.js; app-level keys: Mod+P go to file, Ctrl+Tab / PageUp/Down,
  Mod+W, Mod+B, Mod+Shift+D (Changes), Mod+Shift+S (Share now). The rest are CodeMirror defaults.
- Bundled projects (8, each with 3-4 hidden issues; the brief is a ticket and never names the bug):

| id | lang | level | issue ids |
|---|---|---|---|
| `checkout-retries-java` | Java | sde2+ | retry-no-key, check-then-act, swallowed-interrupt |
| `ttl-cache-python` | Python | ng-sde2 | evicts-newest, put-keeps-old-expiry, stampede-and-race, mutates-cached-object |
| `orders-consumer-java` | Java | sde2+, Hard | commit-before-process, non-idempotent-handler, poison-message-loop (Kafka) |
| `outbox-worker-python` | Python | sde2+, Hard | no-claim, sent-before-publish, unbounded-retry (Postgres outbox, 3 workers) |
| `rate-limiter-python` | Python | sde1-senior | read-modify-write-race, client-clock, no-ttl (Redis token bucket) |
| `wallet-transfer-java` | Java | sde1-senior | float-money, lost-top-up, lock-order-deadlock |
| `catalog-search-node` | JavaScript | ng-sde2 | sql-injection, n-plus-one, last-page-dropped |
| `webhook-receiver-go` | Go | sde2+ | non-constant-compare, no-replay-window, duplicate-delivery |

  `project.json` is the source of truth for exact lines and fixes.
- Add a project: new folder (3-5 files, ~250-400 lines, no comments that give a bug away) → `cargo test projects` checks that it loads and that issue lines exist.
  Planned: download a projects repo release instead of bundling.

## Gaps

`pick` gets an empty `used` set → repeats · bank row errors invisible · whiteboard always dark · project errors only in the log · projects not yet shown in History.
