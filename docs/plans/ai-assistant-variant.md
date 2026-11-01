# Plan: "AI assistant allowed" variant of the comprehension round

Status: plan only (2026-11-01). Nothing built. Post-release feature: not in the 0.1.x public release.
Background: `docs/comprehension-round-research.md` (Meta's AI-enabled CoderPad round), `docs/plans/comprehension-workspace.md`
(the multi-file project this builds on, now shipped). Checked against the code on 2026-11-01.

## Goal

In a comprehension round (multi-file project) the candidate can switch on an AI assistant. A chat panel appears in the
workspace. They ask it to explain code, suggest fixes, write tests. The interviewer sees what they asked and what the
assistant said, and grades whether they understand and verify the answers. Using the assistant is not scored by itself.
Meta's round works this way. The research notes list it as open question 2.

## What the code does today (checked 2026-11-01)

- One model session per round: `brain::Session` (`Cli(ClaudeSession)` or `Api(ApiSession)`), made by `live.rs::spawn_session`
  and kept in `ActiveRound`. Its events (`SessionEvent`) feed the sentence splitter and TTS, are emitted as `round`, and are
  recorded as interviewer turns. Everything about it assumes it is the interviewer.
- `SessionConfig {backend, model, system_prompt, resume}` is enough to start a second, independent session. The CLI session
  gets `--tools ""`, `--setting-sources ""`, `--strict-mcp-config`, `--disable-slash-commands`, an empty temp cwd and the
  prompt as a file (`locate::isolation_args`). `ApiSession` sends no tools on chat.
- `Project` (`projects.rs`) holds the hidden `issues`, `extension` and `solution` in the same struct as the files the
  candidate sees. `Project::view()` is the candidate-safe shape. `interviewer_block()` is the only place hidden fields are
  rendered. `Project::diffs()` already makes unified diffs against the originals with size clipping.
- `send_answer` builds the interviewer message (`candidate_message`): answer, `[Workspace]` block (diffs in a project round),
  `[Code run]`, `[Elapsed]`. It tracks what was already sent in `last_ws`.
- Rounds are stored in `rounds` (`transcript` JSON of `Turn`, `workspace` JSON snapshot). The workspace snapshot is
  overwritten on each send, so it is the wrong place for a growing chat log.
- Settings has `interviewer_model` and `report_model` only. `ApiClient`/`Provider::default_models` returns that pair.
- Groq free tier is 8K tokens per minute, and a coding turn already sends about 5.4K (CLAUDE.md). A project is 200-400 lines,
  roughly 3-6K tokens. The numbers above are the main constraint on the design.

## Decisions

| # | Question | Choice | Why |
|---|---|---|---|
| 1 | Who is the assistant | A **second `Session`**, never the interviewer. Same isolation flags, `Tools::None`, empty temp cwd | Non-negotiable 4 stays untouched. The interviewer must not help with the task, and the assistant must not know the grading notes. Two sessions make both true by construction. A shared session would put hidden issues in the assistant's context |
| 2 | Model | New setting `assistant_model`, default is a small model per provider, empty = "same as interviewer" | Assistant turns are many and short. Quality like a Copilot-class tool is the point |
| 3 | What it sees | Brief, file list, current text of attached files. Built by a new type that cannot hold hidden fields | See "Hiding the answers" |
| 4 | What the interviewer sees | A capped log block of new exchanges inside the next candidate message, plus a short policy in the system prompt | See "Interviewer awareness" |
| 5 | Mistakes | Realistic, not planted. No deliberate flaws in v1 | See "Assistant behaviour" |
| 6 | Voice rounds | Assistant chat is typed only. Never spoken, never captured by the mic | See "Voice rounds" |
| 7 | Reuse of session code | Reuse `Session` and `TurnState` unchanged. A small forwarder maps its events to a new `assistant` event | Avoids a second streaming stack. `Sentence`/`Code` events are ignored, the full text comes from `TurnDone` |
| 8 | Storage | One new `rounds.assistant` column (JSON), not the workspace snapshot | The snapshot is replaced on every send |

### 1. Second session, with the code to prove it

- `AssistantRound` (new `src/assistant.rs`) owns a `Session`, its own history cursor and a `Vec<AssistantTurn>`. It lives in
  `ActiveRound.assistant: Option<AssistantRound>`. `end_round`, terminate and a cancelled start shut it down with the interviewer.
- It is started in `start_round` **without blocking the loader**. A CLI process takes seconds to warm up; the panel shows
  "Assistant starting" and queues the first prompt. A start failure is a warning, not a failed round (see failure handling).
- Chat text is data. The panel renders it as escaped markdown (no HTML), code blocks as plain `<pre>`. Code goes into a file
  only when the candidate presses **Insert** (same rule as "Copy to editor"). Nothing the assistant writes is run; nothing runs in this round anyway.
- The CLI and the API paths are both covered: `Session::start` with `Backend::Cli` or `Backend::Api` already exists.
  The Claude Code backend gets the alias (for example `haiku`), the APIs get an id.

### 2. Model per provider and rate limits

Add `assistant_model` to `Settings` (`lib.rs`), saved and shown next to the interviewer and report pickers (Settings > AI model,
`#ivModel`, `#repModel` in `ui/app.js`, a new `#asModel`). Empty string = same as the interviewer.

Defaults, in a new `Provider::default_assistant_model(models: &[String])` that picks from the live `/models` list
(`list_models`) because ids change fast and CLAUDE.md warns against guessing them:

| Provider | Default | Fallback if not listed |
|---|---|---|
| Claude Code | `haiku` alias (verify with `claude --help` and a live run) | interviewer model |
| Claude API | first listed id containing `haiku` | interviewer model |
| OpenAI | first listed id ending in `mini` of the interviewer's family | interviewer model |
| Gemini | `gemini-flash-lite-latest` if listed, else `gemini-flash-latest` (free tier is Flash only) | `gemini-flash-latest` |
| Groq | a small model other than the interviewer's, for example `openai/gpt-oss-20b`, picked from the live list | interviewer model |

Open: exact ids, to be confirmed live before M1 ships.

Groq free tier (8K tokens per minute per model; to be confirmed that the bucket is per model, not per key):
- Different model for the assistant means a different bucket, so the interviewer is not starved. This is the main reason the Groq
  default is not "same as interviewer".
- Keep prompts small. Context sent per assistant turn: the **open file** and files the prompt names by path, not the whole
  project. A chip row above the input shows what is attached and has "Add all files". On Groq the first turn never attaches
  more than about 3K tokens of code; the panel says so when a file is dropped.
- After the first turn, files go in as **diffs** since the last assistant turn (`Project::diffs`), not full text again.
- A 429 uses the existing wait logic (up to 60 s per wait, 90 s total). The panel shows "Waiting for the rate limit, 12 s".
- One assistant request at a time per round. A second prompt while one is running is queued, not sent in parallel.

### 3. Hiding the answers from the assistant (guarantee)

The assistant must never get `issues[]`, `extension`, `solution.diff` or `good_fix`. Rules, strongest first:

1. **Type separation.** New `Project::assistant_context() -> AssistantContext {title, language, brief, files: Vec<(path, text, readonly)>}`.
   `AssistantContext` has no field for hidden data, and `assistant.rs` takes only that type. It never receives a `Project`.
   Compile-time guarantee, not a convention.
2. **Edits only.** Later turns carry the candidate's own edits (diffs of files they changed), which come from the UI, not from the
   hidden fields.
3. **Own prompt.** `prompts/assistant.md` has placeholders `{{lang}} {{title}} {{brief}} {{level_name}}` only. No `{{issues}}`.
   A test renders it with sentinel strings stuffed into every hidden field of a `Project` and asserts none appear in the
   rendered system prompt or the first turn.
4. **No shared history.** The assistant's `Session` has its own history. Interviewer replies never enter it, and the assistant
   is never told what the interviewer asked.
5. **No tool or file route.** `--tools ""`, empty cwd, `--strict-mcp-config`, no `--resume` of the interviewer's id. API sessions send no tools.
6. **Generated projects.** `projgen` keeps writing `solution_diff` into the same JSON as the files. It is split off in
   `Project::from_generated` before anything reaches `assistant_context`. A test covers a bundled and a generated project.

The level and the brief are fine to share: the candidate sees both. The company name is not sent (not needed).

Known leak the design cannot remove: the model may guess a bug by reading the code. That is the point of the tool.

### 4. Interviewer awareness

**System prompt.** `Project::interviewer_block()` gets an `ai_block(on)` part, added only when the assistant is on:

~~~text
AI ASSISTANT IS ALLOWED in this round. The candidate has a chat panel with a coding assistant (a separate, weaker model that cannot
see your notes or the planted issues). It can be wrong. Using it is fine and is not scored by itself. You grade what they do with it:
do they ask a precise question, read the answer, test it against the code, catch a wrong suggestion, and explain in their own words why a
change is right? Their messages carry an [Assistant log] block with the new exchanges. Use it. Ask "why did you accept that?", "what
would break if the assistant is wrong here?", "walk me through that line". Never answer for the assistant and never say what it
should have said. Do not penalise a short prompt or a long one. Penalise an accepted suggestion they cannot explain.
~~~

**Per message.** `candidate_message` (`interview/mod.rs`) appends the block, after `[Workspace]` and before `[Elapsed]`. It holds only
exchanges not yet sent (cursor `assistant.sent_upto`):

~~~text
[Assistant log, new since your last message]
#4 at 12.4 min | asked: "why does put() keep the old expiry?" | assistant: "put() copies the entry but reuses expires_at from the first call. Set it again in the update branch." | inserted: no
#5 at 13.1 min | asked: "write a test for that" | assistant: "[code, 14 lines, python] Adds test_put_resets_expiry." | inserted: yes, test/test_cache.py
(2 earlier exchanges left out)
~~~

Caps (constants in `assistant.rs`): candidate prompt 300 chars, assistant reply 500 chars, code fences replaced by
`[code, N lines, lang]`, at most 6 entries and 2,400 chars per message (newest kept, count of the rest). `inserted` is set by the
Insert button (pasting by hand is unknown, and the log says so: `inserted: unknown` is not used, only yes/no from the button).
Costs the interviewer about 0.6K tokens per message at most, which fits Groq.

If nothing new happened, no block is added. If the assistant failed, the entry says `assistant: (no reply, it was unavailable)`.

**Why a block in the candidate's message and not a second channel.** The interviewer is a persistent session that only reads user
turns. A block in the next turn needs no new plumbing, is cut off by the same `last_ws` style cursor, and is resent on a
session restart because it is part of `sent` text. A candidate who uses the assistant for three silent minutes is seen at their
next message or **Share now**. v1 does not interrupt the interviewer.

**Report.** `report.md` gets `{{assistant_log}}`: the full exchange list capped at 6K chars (candidate prompt 400, reply 700 per
entry, evenly thinned if over), or "(assistant off)". Rubric: when on, `report_prompt` adds the dimension **AI collaboration**
to the rubric list (`catalog.json` stays unchanged, so rounds without the assistant are unaffected). Description given to the model:
"Asks specific questions, verifies output against the code, catches wrong or risky suggestions, explains accepted changes in their own words.
Score null if they did not use the assistant. Not using it is never a penalty." Dashboard skill grouping by dimension name picks it up
with no code change, and a null score is already ignored.

### 5. Assistant behaviour

`prompts/assistant.md` (header lists placeholders, like the other prompts):

- Role: a coding assistant built into the editor, like the ones candidates use at work. Short answers, code in fences, plain words.
- It sees the project files it was given and says when a file was not attached ("I have not seen `OrderRepo.java`").
- It answers what is asked. It does not volunteer a full audit of the project, and does not say which parts "are the planted bugs".
  (It does not know there are any; the prompt calls the code "the candidate's project".)
- It does not do the interviewer's job: no grading, no hints about time or the round.
- It may be wrong. It is not told to be wrong. A small model is wrong often enough, and a model told to hide errors would be
  hard to keep honest.

**Deliberate mistakes: no in v1.** Reasons: (a) real assistants are already imperfect, the small default model supplies that
naturally; (b) a planted wrong answer needs a second hidden answer key that nobody can run or check (nothing executes in this
round), so a trap that is itself wrong would grade the candidate unfairly; (c) the interviewer would have to know the trap
and the assistant would have to carry it, which is another hidden-field leak path; (d) Meta's round does not publish such traps.

**Opt-in "flawed assistant" later.** Needs: `traps[]` in `project.json` and the projgen schema (`{when, wrong_claim, why_wrong}`,
checked like `issues`); a validation pass that the wrong claim is plausible but wrong; the assistant prompt gets only `when` and
`wrong_claim` (never `why_wrong`), told to give it once and confidently; the interviewer block lists each trap so it can grade
caught/accepted; the report gets `ai_traps` per trap; a setup switch separate from "AI assistant allowed"; a warning that it is
a teaching mode. Not before there is user demand.

### 6. Voice rounds

- The assistant is typed only. No TTS for its replies (a spoken assistant would talk over the interviewer), no STT for its input.
  The panel input is a normal text box. Mic and VAD are untouched: keystrokes are not speech.
- Spoken flow stays: candidate talks to the interviewer. They may say "let me ask the assistant" and type. The next spoken answer
  or **Share now** carries the log block. The interviewer's reply to it is spoken as usual.
- If the candidate is typing in the assistant box while the interviewer is mid-sentence, nothing changes. No barge-in is raised.
- Transcript: `Turn`s stay interviewer and candidate only. The assistant exchange is stored in its own column and shown in its own
  section of the History view. The two channels are never merged in storage. The report sees them as two labelled sections.
- A spoken answer is not echoed into the assistant, and an assistant reply is not read into the interviewer's `Turn.text`.

### 7. UI

- **Panel.** In the Editor pane, right of the editor: file tree left, editor middle, assistant right (Meta's layout). Collapsible,
  width in localStorage `onsite.asW`, same `split-v` bar pattern as `#treeSplit` (`ui/app.js` *resizable panes*).
  On narrow windows it becomes a tab-like drawer. Header: model name, "Clear view" (keeps the log), a status dot.
  Message list, then a multi-line input (Enter sends, Shift+Enter newline), attach chips, a Stop button while streaming.
  Each code block has Copy and **Insert** (menu: new file, or at cursor in the open file).
- **Toggle.** Setup: a checkbox next to the ready-made project box in `#projNote` (`ui/index.html:223`), label "Let me use an AI
  assistant (like Meta's AI-enabled round)". Default off. Also shown in `#projDlg` before each comprehension round, answer kept in
  `loop.projChoice` next to the existing project choice, passed to `start_round` as `assistant: Option<bool>`. `LoopSetup`
  gets `ai_assistant: bool` with `#[serde(default)]`, the same way `ready_made_project` was added (old loops load as off).
- **Settings > AI model.** New "Assistant model" select with an "Same as interviewer" option, the 429 note for Groq, and one
  line saying that assistant prompts go to the same provider.
- **Style.** Theme tokens only: `--pane`, `--hover`, `--accent-line`, `--sel-bg`. No new colours. Candidate bubbles reuse the chat
  bubble classes; assistant bubbles use a quieter token so they do not look like the interviewer. A small "AI assistant" label,
  so nobody mistakes it for the interviewer.
- **Copy.** Short sentences. No em or en dashes anywhere in docs, prompts, UI or site text.
- **Not shown when off.** The panel, chips and the Settings model row (row stays, hint says it is used only when you switch it on).

### 8. Storage, retention, export, privacy

- `schema.sql`: `ALTER TABLE rounds ADD COLUMN assistant TEXT NOT NULL DEFAULT ''` through the same migration path used for
  earlier column additions in `store/mod.rs` (check the exact helper before coding). Empty = off. Otherwise
  `{"model": "...", "turns": [{"id", "at_min", "ask", "reply", "ok", "inserted": null | {"file"}}]}`.
  Writes happen on each finished assistant turn, in the same call path as `RoundLog::persist` (own method `persist_assistant`).
- Retention: the data lives in the round row, so "keep N rounds", "keep N days" and Delete-round (`storage.rs`, `rounds_over_limit`)
  already remove it. `storage.rs` size counts add the column's byte length. Terminate (`delete_round`) removes it too.
- Export: rounds are not exported today (dossiers only), so nothing to add. History shows the exchange read-only.
- Privacy must stay true. Edit in the same commit: `ui/index.html` Privacy list (line 622: add "and, if you switch on the assistant,
  what you type to it and your project files") and `PRIVACY.md` (same sentence in the "To the AI you connect" item).
  Assistant prompts go only to the provider the user already chose. Keys stay in `api_keys`. No logging of assistant text: tracing
  logs only ids, sizes and timings, as for the interviewer.

### 9. Cost, latency and failure

Budget per round (45 min, about 10-15 assistant prompts): first turn about 2-4K input tokens (brief and up to 3 files), later turns
about 1-2K (diffs and short history), reply about 300-600 tokens. Roughly 25-50K input tokens in total on a small model, cents on paid
APIs, free on Gemini and Groq within limits. History grows in memory; trim to the last 12 exchanges (older ones dropped, files still
come as diffs). Interviewer cost up by at most 0.6K tokens per message.

Latency targets: first token under 2 s on APIs. CLI is about 6 s per turn warm (AI.md), so on Claude Code the panel shows progress
and a stop button; it is a limit of the CLI, as for the interviewer.

Failure handling (the round never stops because of the assistant):
- Start fails or process dies: panel shows "Assistant unavailable. Your interview goes on." with **Try again**. Same restart-with-resume
  logic as `send_answer` for the CLI, one retry.
- A request fails or times out (60 s for first token, 3 min total): the turn is marked `ok:false`, input is restored in the box.
  The interviewer log marks it as unavailable so it does not look like the candidate ignored the answer.
- Rate limit: wait with a visible countdown (existing logic), then fail plainly.
- Assistant calls run in their own task. An assistant error never touches `r.session`, `r.last_ws` or the report.
- If the user turns the setting off mid-round: not supported in v1 (per-round choice at start). Open question below.

## Testing

Unit (Rust, `cargo test`):
- `assistant_prompt_never_contains_hidden_fields`: build a `Project` with sentinels (`SECRET_ISSUE_WHY`, `SECRET_FIX`, `SECRET_EXT`,
  `SECRET_SOLUTION`) in each hidden field, render the system prompt and the first and a later turn, assert none appear. Run for a bundled
  project and a `from_generated` one.
- `assistant_context_has_no_hidden_fields`: struct-shape test (serialises to the expected keys only).
- `log_caps`: long prompts and replies are clipped, code fences become `[code, N lines, lang]`, over 6 entries or 2,400 chars drops the oldest
  with a count, nothing new yields no block, cursor advances only after a successful send, restart resends nothing twice.
- `report_prompt` includes the AI collaboration dimension and the log only when on; rounds with it off render exactly as before (snapshot).
- Store: migration on an old DB, round delete removes the column, storage size includes it.
- Interviewer block: `ai_block` present only when on.

Live (`src-tauri/examples/api_e2e.rs`): new flag `--assistant`. Starts a project round with the assistant on, sends three assistant prompts
(explain, fix suggestion, write a test), checks: TTFT, no hidden sentinel in what was sent (the example logs the request body size and
runs the sentinel check against the real prompt), the log block appears in the next interviewer message, and the report has the
AI collaboration dimension. `echo <key> | cargo run --example api_e2e -- groq --assistant`. Record TTFT per provider in CLAUDE.md.

Manual script (owner):
1. Setup, comprehension round, tick the assistant box. Join. Panel shows, interviewer opening does not mention hidden issues.
2. Ask "what does this do?" Reply streams. Ask for a fix, press Insert. A new file or the cursor insert happens, nothing else.
3. Send an answer by voice. Interviewer asks about the assistant exchange without quoting the assistant's text word for word.
4. Kill the assistant (disconnect network for it, or wrong model id). Panel shows the error, the round continues.
5. Groq: back-to-back prompts show the wait countdown, interviewer reply is not blocked.
6. End, report: AI collaboration dimension, plus History shows the exchange. Delete the round: gone.
7. Round with the box off: no panel, no log block, report as before.

## Milestones (each shippable)

1. **Backend, hidden.** `assistant_model` setting + defaults, `AssistantContext`, `prompts/assistant.md`, `assistant.rs`
   (session, forwarder, caps, log builder), commands (`assistant_send`, `assistant_stop`, `assistant_inserted`), event `assistant`,
   storage column, unit tests, `api_e2e --assistant`. No UI. Safe to merge: it changes nothing for a user.
2. **Panel and interviewer awareness together.** Chat panel, Insert, setup/dialog toggle, Settings model row, interviewer `ai_block`,
   `[Assistant log]` in messages, failure UI. These two ship together because an assistant the interviewer cannot see makes the round
   confusing.
3. **Report and History.** AI collaboration dimension, `{{assistant_log}}`, History section for the exchange, storage size counts,
   privacy text. Dashboard shows the new skill automatically.
4. **Polish.** Live TTFT numbers per provider written to CLAUDE.md, docs and site pages (below), screenshots, a Meta dossier flag
   (see open questions).
5. **Later, only on demand.** Flawed-assistant mode, interviewer nudge when the candidate is silent in the panel for a long time,
   assistant on other round types (machine coding).

## File-by-file change list

| File | Change |
|---|---|
| `src-tauri/src/assistant.rs` (new) | `AssistantRound`, `AssistantTurn`, `AssistantContext` user, prompt render, event forwarder, `log_block()`, `report_log()`, caps |
| `src-tauri/src/lib.rs` | `Settings.assistant_model`, module declaration, `AppState::assistant_model()` |
| `src-tauri/src/brain/api.rs` | `Provider::default_assistant_model(&models)` |
| `src-tauri/src/brain/chat.rs` | no change expected (reuse `Session`); check that `TurnDone.text` is the full text with code fences |
| `src-tauri/src/projects.rs` | `Project::assistant_context()`, `ai_block(on)` in `interviewer_block`, tests |
| `src-tauri/src/interview/mod.rs` | `LoopSetup.ai_assistant` (serde default), `candidate_message` takes the log block |
| `src-tauri/src/interview/live.rs` | `ActiveRound.assistant`, start in `start_round` (non-blocking), shutdown in `end_round`/cancel, log cursor in `send_answer`, persist |
| `src-tauri/src/interview/report.rs` | `ReportInput.assistant`, rubric extension, `{{assistant_log}}` |
| `src-tauri/src/store/schema.sql`, `store/mod.rs` | `rounds.assistant` column, migration, `save_assistant`, read in `round()` |
| `src-tauri/src/storage.rs` | size counts |
| `src-tauri/src/commands.rs` | new commands, settings save/load of `assistant_model`, register in `lib.rs` |
| `src-tauri/examples/api_e2e.rs` | `--assistant` check |
| `prompts/assistant.md` (new) | assistant system prompt |
| `prompts/interviewer.md` | header note only if a new placeholder is added (the block is appended in Rust, like `problem_block`) |
| `prompts/report.md` | `{{assistant_log}}`, AI collaboration guidance |
| `ui/index.html` | panel markup in `.ws-split`, toggle in `#projNote` and `#projDlg`, `#asModel` in Settings, privacy sentence |
| `ui/app.js` | panel logic, `assistant` event handler, Insert, chips, resize bar, `start_round` argument, Settings load/save, History section |
| `ui/app.css` | panel styles on existing tokens |
| `PRIVACY.md` | privacy sentence |

## Docs and website to update when it ships

- `CLAUDE.md`: provider notes (assistant model defaults, TTFT numbers, Groq bucket finding), `projects.rs` paragraph.
- `docs/engineering/AI.md` (second session, `assistant_model`, event), `INTERVIEW.md` (log block, report dimension, `start_round` arg),
  `CODING.md` (Projects: panel, hidden-field rule), `STORAGE.md` (column, settings key, retention), `UI.md` (panel, tokens), `PRODUCT.md` (status).
- `docs/comprehension-round-research.md`: mark open question 2 answered and link this plan. `docs/plans/comprehension-workspace.md`: one-line pointer.
- Website: `site/src/index.html` (feature list), `how-it-works.html` (two sessions, isolation, hidden-field rule), `guides.html` (Settings
  screenshot: new Assistant model row; retake with `npm run guide-shots`), changelog is generated.
- `docs/engineering/README.md` does not list plans, so no index entry.

## Open questions for the owner

1. Default state of the toggle per dossier: should a company dossier for Meta set it on automatically (a `ai_allowed` field in the
   dossier schema, set by research), or always manual?
2. Should the candidate be able to turn the assistant on or off during a round? v1 says no (start-time choice, simpler log and report).
3. Is a different, smaller model right for the Claude Code default (`haiku`), or should the CLI path use the interviewer model to avoid
   a second login/model surprise? Needs a live check of the alias.
4. Show the assistant's model name to the candidate? Showing it is honest and cheap. Hiding it matches a real tool less.
5. Report weight: keep AI collaboration out of the overall score (current plan), or let it move the score a little?
6. Insert button only, or also a "diff view" (apply suggestion as a patch)? The patch view needs a parser for model diffs; skipped in v1.
7. Is the flawed-assistant mode wanted at all, or is natural error enough?

## Risks

- **Hidden-answer leak.** Mitigated by type separation and sentinel tests. A future edit that passes a `Project` into `assistant.rs`
  would break the guarantee: the tests and a code comment on `AssistantContext` say so.
- **Groq free tier.** Even with a separate model and diffs, a long project plus the interviewer turn can hit 8K per minute. The panel
  must show the wait, and the doc must say Groq is the weakest provider for this round.
- **Grading drift.** The interviewer might punish or praise AI use itself. The prompt wording and the report instruction both say not
  to; check in the live runs and adjust.
- **Interviewer context growth.** The log block adds tokens every message. Caps and the "new since last message" cursor keep it small.
- **Wrong assistant answers read as app bugs.** The panel label says "AI assistant, can be wrong".
- **Two sessions double the processes on the CLI.** More memory and a second cold start. Warm-up is non-blocking; if it hurts, start the
  assistant on first panel open.
- **Silent minutes in the panel.** The interviewer only learns at the next message. Accepted for v1.
- **Projects are unverified.** Generated projects may have wrong planted bugs (known risk). An assistant may then "fix" a non-bug, which
  makes grading noisier. The setup warning already covers the project; consider mentioning it for this mode.
- **Privacy text drift.** Any new place that sends assistant text (a nudge, an export) must update the privacy text in the same commit.
