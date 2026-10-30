# PRODUCT

Onsite = desktop app for realistic spoken SWE interview loops on the user's own machine, with honest feedback.
Rebuild of a claude.ai artifact (sandbox blocked the mic; browser TTS was robotic).

## Users

- **Owner (primary):** backend engineer (Java, Kafka, PostgreSQL, Redis, payments), Windows 11 + WSL, Claude Pro,
  targeting SDE-2/senior.
- **Friends:** may lack Claude Pro; mostly Windows, some macOS/Linux; need install-to-first-round < 15 min.
- **Contributors:** open source (MIT).

## Non-negotiables (from CLAUDE.md; every change must respect)

| # | Rule | Implication |
|---|---|---|
| N1 | Bring your own AI: Claude Code CLI or user key for Claude API / OpenAI / Gemini / Groq; keys only in local SQLite; no SDKs | every model call works on all 5 providers |
| N2 | Native on Windows, owns real mic/speakers | no browser audio APIs |
| N3 | Speech local (VoiceStudio) with fallbacks; never dead-end | voice failure → "Start typed" |
| N4 | Interviewer never gets tools; only research gets web search/fetch | new calls default to no tools |
| N5 | Model output and dossiers are data, never executed | parse/escape only |

## Goals / non-goals

Goals: feel like a real interview (persona, follow-ups, clock, closes itself); whole loops; company-true loops from
research; feedback with evidence + "fix first" + verdict; private and cheap.
Non-goals: hosted service/accounts/telemetry; LeetCode replacement; live-interview assistance; multi-user;
non-English (deliberate; STT tuned for English).

## Feature status

| Area | Feature | Status | Doc |
|---|---|---|---|
| loops | 9 round types, 14 company styles, 5 levels, 3 temperaments | shipped | INTERVIEW |
| loops | 2 panels × 5 personas, fixed voices | shipped | INTERVIEW, VOICE |
| loops | round report, loop verdict, dashboard | shipped | INTERVIEW |
| loops | comprehension round redesign | planned (`docs/comprehension-round-research.md`) | INTERVIEW |
| voice | spoken rounds, Whisper pass, mute, barge-in by tap | shipped | VOICE |
| voice | first audio < 2.5 s after candidate stops | partial (API providers close; CLI ~6 s round trip) | VOICE |
| research | research jobs → dossiers; company loops; import/export | shipped | RESEARCH |
| resume | ATS parse check, 6 ATS profiles, keyword match, AI screen | shipped | RESUME-CHECK |
| coding | editor (6 langs), Run/Submit (Java/Python/C++/JS), whiteboard, notes | shipped | CODING |
| app | 5 providers, live model lists, health check | shipped; Claude API/OpenAI/Gemini not yet run with a live key | AI |
| app | 4 theme families × light/dark, custom backdrop | shipped | UI |
| app | in-app updates; storage limits | shipped | RELEASE, STORAGE |
| app | signed Windows installer (SignPath Foundation) | pending application (2026-10-30) | RELEASE |
| app | MCP server (read-only dossiers/reports) | planned (phase 3) | — |

## Milestones (all four met; M3 latency still tuned)

M1 typed round + reports over Claude Code · M2 research jobs + dossiers · M3 voice · M4 installers, updates, open source.

## Success signals (no telemetry; measured locally)

First audio latency (`voice latency` events) < 2.5 s · TTFT (`ttft_ms` per turn) < 2 s on APIs · rounds end with
`[[END]]` and a report · tech terms transcribed right · friends install in < 15 min.

## Constraints

Claude Code has no temperature flag (variety via openers, INTERVIEW.md) · Groq free tier ~8K tokens/min (coding turn
~5.4K) · VoiceStudio is external (never installed/updated by Onsite) · Whisper needs a GPU, else Parakeet text.

## Open questions

Comprehension round design (curated repos vs bug datasets vs mutation) · non-English rounds · "AI assistant allowed"
round mode · MCP server scope.
