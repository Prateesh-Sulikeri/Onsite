# docs/engineering — agent docs

Docs for AI coding agents working on Onsite. Dense on purpose. Open only the file you need.
Humans: read `pdf/` (snapshot of tag `docs-human-2026-10-30`).

## Index

| File | Read when the task touches… |
|---|---|
| [ARCHITECTURE.md](ARCHITECTURE.md) | anything cross-cutting: processes, modules, `AppState`, commands, events, invariants, errors |
| [PRODUCT.md](PRODUCT.md) | scope decisions: non-negotiables, users, feature status, open questions |
| [AI.md](AI.md) | model calls: `brain/`, providers, Claude Code flags, sessions, one-shot jobs, splitter, JSON parsing |
| [INTERVIEW.md](INTERVIEW.md) | loops/rounds: `interview/`, catalog, panels, prompt assembly, loader, turns, reports, verdict, dashboard rules |
| [VOICE.md](VOICE.md) | `voice/`, VoiceStudio, Whisper sidecar, VAD, TTS batching, barge-in, mute |
| [RESEARCH.md](RESEARCH.md) | `research.rs`, `dossier.rs`, company-specific loops |
| [RESUME-CHECK.md](RESUME-CHECK.md) | `ats/`: extraction, checks, ATS profiles, AI screen |
| [CODING.md](CODING.md) | `bank.rs`, `runner/`, `questions.csv`, workspace (editor/whiteboard/notes), Run/Submit |
| [STORAGE.md](STORAGE.md) | `store/`, `storage.rs`, schema, settings keys, files on disk, retention, privacy flows |
| [RELEASE.md](RELEASE.md) | CI release workflow, installers, `updates.rs`, code signing, platform quirks |
| [UI.md](UI.md) | `ui/`: structure, conventions, tokens, themes, screens, adding a screen |

`CLAUDE.md` (always loaded) holds verified external facts and measurements. These docs hold code structure and
rules. Don't duplicate between them; link instead.

## Conventions

- Code refs: `path::function` (no line numbers). Paths relative to repo root; `src/` = `src-tauri/src/`.
- Requirement ids (`LOOP-R3`) are stable; cite them in commits.
- Status: **shipped** · **partial** · **planned** · **gap** (known defect or mismatch, not yet fixed).
- Each doc: purpose → map → invariants → flow/contracts → constants → change recipes → gotchas → gaps.

## Maintenance

- Edit the doc in the same commit as the code that makes it wrong. Keep it terse; no narrative.
- PDFs are frozen. Their sources are the human Markdown at tag `docs-human-2026-10-30`. To refresh: check the tag
  out as a worktree, update those sources from these docs + code, then
  `cd docs/engineering/tools && npm run build -- --src <that folder>/docs/engineering` (see header of `tools/build.mjs`).
  `npm run shots` retakes UI screenshots (mock backend) into `img/`.
