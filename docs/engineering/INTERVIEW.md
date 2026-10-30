# INTERVIEW — loops, rounds, reports

Code: `src/interview/{mod,live,report}.rs`, `src/catalog.rs`, `src/panels.rs`, `prompts/{catalog.json,panels.json,interviewer.md,persona_block.md,dossier_block.md,report.md,verdict.md}`; UI: `ui/app.js` sections *setup*, *call room*, *preparing*, *feedback + report*, *interview feedback*, *dashboard*.

## Map

| Item | Where |
|---|---|
| `LoopSetup {rounds, company, level, temperament, lang, resume, jd, notes, minutes?, panel, dossier_id, dossier_rounds, dossier?}` + `validate`, `plan`, `company_name`, `company_for_prompt` | `interview/mod.rs` |
| `PlannedRound {key, name, persona, persona_id, persona_role, minutes, tab}` | `interview/mod.rs` |
| `interviewer_prompt`, `pick_opener`, `session_block`, `opening_excerpt`, `opening_message`, `Workspace {code, notes}`, `candidate_message` | `interview/mod.rs` |
| `create_loop`, `start_round` (loader), `spawn_session`, `send_answer`, `run_code`, `set_round_lang`, `end_round`, `generate_report`, `loop_verdict`, `loop_feedback`, `loop_feedback_saved`, `ActiveRound`, `RoundLog`, `RoundInfo`, `problem_view`, `problem_block`, `difficulty_for` | `interview/live.rs` |
| `enough_to_review` (≥ 2 answers, ≥ 40 words), `metrics`, `transcript_text`, `report_prompt`, `round_summary`, `verdict_prompt` | `interview/report.rs` |
| `Catalog {rounds, companies, levels, level_bar, names, temper}` (JSON keys SCREAMING_SNAKE), `persona(idx)` = `names[(idx*5+3) % len]` | `catalog.rs` |
| `Panels::{load, list, persona, persona_vars, all_personas, contains}`, `voice_design` | `panels.rs` |

## Catalog content

Rounds (key · name · min · default tab · rubric):
`resume` Resume screen 30 notes [Clarity of story, Depth behind claims, Role fit, Motivation, Communication] ·
`project` Project deep dive 45 notes [Ownership, Technical depth, Trade-off reasoning, Impact and metrics, Communication] ·
`fundamentals` 45 notes [Accuracy, Depth, Applied reasoning, Communication] ·
`coding` DSA coding 45 editor [Problem solving, Code quality, Testing and edge cases, Complexity analysis, Communication] ·
`comprehension` 45 shared [Reading accuracy, Bug finding, Prioritisation and judgment, Fix quality, Testing mindset, Communication] ·
`lld` 60 notes [Requirements, Object modelling, Code quality and SOLID, Extensibility, Concurrency] ·
`sysdesign` 60 notes [Requirements and scoping, Architecture, Deep dive, Scale and reliability, Trade-offs, Driving the discussion] ·
`behavioral` 45 notes [Specificity, Ownership, Self-awareness, Collaboration and conflict, Values alignment] ·
`hm` Hiring manager 45 notes [Scope and impact, Judgment, Team fit, Motivation, Quality of their questions].
Each round also has `blurb`, `guide` (sent to interviewer), `openers` (7–19; none for coding).
Companies: `auto` (infer from JD) google amazon meta microsoft apple netflix uber stripe atlassian flipkart inpay bank startup.
Levels: ng sde1 sde2 senior staff (+`LEVEL_BAR` text). Temper: supportive realistic barraiser.
Panels (`panels.json`): `indian_techies`, `international_techies`; 5 interviewers each with traits, strictness 1–5,
backstory, owned round types, speed factor, voice design. Temperament overrides persona strictness for hinting/warmth.

## Prompt assembly (`interviewer_prompt` + `start_round`)

`interviewer.md` rendered with: `persona persona_role persona_block company_line round_name round_minutes round_index
round_count round_guide company_style level_name level_bar temperament lang dossier_block resume jd candidate_notes`
(resume/JD clipped; defaults when empty). Then appended: `problem_block(q)` (coding) and `session_block(opener, earlier)`.

Variety (no temperature on CLI): `pick_opener` skips recent openers (settings `openers:<round_key>`, keeps half the
list); `earlier` = `opening_excerpt` of first two interviewer turns of the last 4 rounds of that key (`store.recent_transcripts`),
skipped for coding. Block is marked private ("never mention").

Company loop: `create_loop` freezes a dossier copy with only chosen rounds; `rounds[i] = process[i].kind`; `plan` takes
name/minutes from the dossier round; `dossier_block` per round (RESEARCH.md).

## `start_round(loop_id, index)` — returns when the room is ready

1. load + `validate` setup; `plan`; build prompt; coding → `bank.match_reported(dossier questions)` else
   `bank.pick(difficulty_for(level), [], company, {}, seed)`; opener.
2. Panel persona → `settings.voice.persona` + speed × `speed_factor`.
3. `me = prep_gen += 1`; `prep claude run`; `backend()`; `close_other_rounds`; `store.create_round`.
4. `voice.begin` → `VoiceStart {ready, go}`; `spawn_session` + `send(opening_message(minutes))`.
5. `join!(wait_claude, prepare_voice)` raced against `prep_gen != me` (cancel). Steps emitted as `prep`:
   `claude` (Started) · `question` (first Sentence) · `voicestudio` · `speaker` · `voice` (persona profile + warm-up) ·
   `stt` (Parakeet v2 chosen) · `whisper` (Ready, waits ≤ 60 s) · `mic` (3 s of audio).
6. Cancel/failure → `voice.end`, `session.shutdown`, `delete_round`, Err. Voice-only failure → round continues with
   `voice:false` (UI offers *Start typed*).
7. Success → `go.send(())` (queued speech plays), `log.started = now` (clock starts here), insert `ActiveRound`,
   return `RoundInfo {…, problem: problem_view (no hidden tests/reference/hints), voice}`.

## `send_answer(round_id, text, ws, force)`

Busy → `Invalid(BUSY)`. `candidate_message` attaches editor/whiteboard only if changed vs `last_ws` (or `force` = Share now),
clipped; appends `[Code run] <summary>` if pending; always ends `[Elapsed: X of Y min]`. Send; on failure restart session
with `resume = session_id` and resend once; still failing → restore `last_ws`, `Ai("Could not reach the interviewer. Try again.")`.
Push `Turn{who:"me", text, at_min, shared_workspace, sent: exact content}`; persist; `voice.answer_sent`.
Interviewer turns recorded on `TurnDone` in `spawn_session` (with `ttft_ms`), persisted each turn.

Message shape:
~~~text
<answer text or "(The candidate shares their workspace without saying anything.)">

[Workspace]
Editor (Java):
```
…
```

Whiteboard:
…
[Code run] Run in Java: 2/3 tests passed. First failure (example): input … → expected …, got …
[Elapsed: 14 of 45 min]
~~~

## End and feedback

- `end_round`: `voice.end`, persist, `session.shutdown`, `finish_round`; delete if no `me` turn; returns kept?.
- UI Terminate (typed "terminate") = `end_round` + `delete_round`.
- `generate_report`: requires `enough_to_review`; metrics (answers, avg/longest words, planned vs actual min, code shares)
  → `report.md` → `run_once` no tools, report model → JSON object else `Ai("…wrong format…")`, raw saved only if no good report
  exists; adds `metrics`; `save_report`.
- `loop_feedback`: reports for rounds with enough text and no report, `PARALLEL_REPORTS = 3`, `loop_feedback` events;
  then `loop_verdict` if ≥ 2 rounds reported and (new reports or no verdict). `loop_verdict` needs ≥ 1 report.

Report JSON: `{overall 0-100, decision: Strong hire|Hire|Lean hire|Lean no hire|No hire, level_signal, headline,
dimensions[{name, score 1-4|null, evidence}], went_well[], held_back[], fix_first, moments[{question, said, better}],
practice_next[], metrics}` (2–4 items per list).
Verdict JSON: `{decision: Hire|No hire|Hire at lower level|Borderline — more data needed, rationale, strongest_signal,
biggest_risk, plan[]}`.

## Requirements (stable ids)

| Id | Rule |
|---|---|
| LOOP-R1 | Interviewer speaks 1–4 sentences, one question, no markdown in speech (prompt) |
| LOOP-R2 | Paces by `[Elapsed]`; wrap-up at ~85%; leaves time for candidate questions (prompt) |
| LOOP-R3 | Interviewer closes with `[[END]]` → UI "Get my feedback" |
| LOOP-R4 | No feedback/praise during round (prompt) |
| LOOP-R5 | Interviewer code → "From interviewer" tab, never TTS |
| LOOP-R6 | Changed workspace sent in `[Workspace]` |
| LOOP-R7 | Opener rotation + do-not-reuse list |
| LOOP-R8 | One live round |
| LOOP-R9 | Unanswered rounds deleted |
| LOOP-R10 | Terminate = typed confirm, nothing kept |
| LOOP-R11 | Report needs ≥ 2 answers and ≥ 40 words |
| LOOP-R12 | Dead interviewer → resume on next answer |
| LOOP-R13 | English only |

## Dashboard rules (`ui/app.js` dashboard section; data = `history` rows with report JSON)

Skill score = mean of dimension scores grouped by name. Practise next / highlights need a skill low (≤ 2) / high in 2+
rounds. Weekly goal = settings `weekly_goal` (default `{coding:2, sysdesign:1, behavioral:1}`). Trend hire bar = 70.
No placeholder numbers; empty states only.

## Change recipes

- **New round type:** `catalog.json` ROUNDS entry (name, min, tab, blurb, guide, rubric, openers) → `ui/app.js`
  `ROUND_ICONS`, `PLANS`, `TIPS` (+ `BOARD_ROUNDS` if whiteboard) → panel owners in `panels.json` →
  `dossier.rs::round_kind` keywords + `docs/dossier.schema.json` enum.
- **New company style:** `COMPANIES` entry `{name, style}`.
- **Interviewer behaviour:** edit `prompts/interviewer.md`; keep header placeholder list; dev build picks it up next round.

## Gaps

- Coding problem pick passes empty `used` set → problems repeat across rounds.
- Difficulty is level-mapped (ng Easy, senior/staff Hard, else Medium); no picker.
- Comprehension round: interviewer writes the code to review; redesign pending.
