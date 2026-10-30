# CODING — question bank, runner, workspace

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

`Bank::parse` skips bad rows into `Bank.errors` (not shown in UI — gap). `Question::check` compares per `compare`.

Picking: company loop → `match_reported(questions, seed)` (title words minus stop words/plural s; ≥ 60% coverage; most
words, then seeded tie-break); else `pick(difficulty_for(level), topics, company, used, seed)` = same difficulty → max
(2×topic overlap + company tag) → seeded shuffle. `difficulty_for`: ng Easy, senior/staff Hard, else Medium.
Candidate sees `problem_view` (statement, examples, constraints, hidden count, runnable, starter). Interviewer gets
`problem_block` (+ optimal time/space, hints one at a time, follow-ups; told problem is on screen, don't read it).

## Runner (`runner::run(q, lang, code, tests)`)

| Lang | Files | Compile | Run | per-test limit |
|---|---|---|---|---|
| Python | `Solution.py driver.py sig.json` | — | `python -X utf8 driver.py Solution.py sig.json tests.json` | 6 s |
| JS | `solution.js driver.js sig.json` | — | `node driver.js …` (worker, 256 MB stack) | 3 s |
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
- **Add a runnable language:** `Lang` variant (`from_name`, `label`, `limit_ms`, `ALL`) → `plan` → driver in `gen.rs` →
  starter generation → `toolchains` → tests in `runner/tests.rs`.
- **Editor change:** edit `ui-vendor/src/editor.js` → `cd ui-vendor && npm install && npm run build` → commit `ui/vendor/`.

## Gaps

`pick` gets an empty `used` set → repeats · bank row errors invisible · whiteboard always dark.
