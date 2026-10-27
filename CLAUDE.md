# Onsite — local AI mock-interview app

Onsite runs realistic, spoken software-engineering interview loops on the owner's own machine.
It started as a claude.ai artifact (`ui/onsite-artifact-reference.html`). That version hit two walls:
the artifact sandbox blocks the microphone, and browser text-to-speech sounds robotic.
This repo rebuilds it as a native desktop app.

Owner: backend engineer (Java, Kafka, PostgreSQL, Redis, payments). Windows 11 + WSL2 for dev.
Write plain, short sentences in user-facing copy. Do not water down technical content.

## Non-negotiables

1. **No Anthropic API key.** All model calls go through the locally installed **Claude Code CLI**
   in headless mode (`claude -p`), which uses the owner's Claude login.
2. **The app runs natively on Windows.** It must own the real microphone and speakers.
   Claude Code may be installed on Windows or only in WSL; support both (see "Locating Claude Code").
3. **Speech is local.** VoiceStudio (already installed, desktop app, backend on `127.0.0.1:3900`)
   is the primary speech engine for both directions. Fallbacks must exist so the app never dead-ends.
4. **The interviewer never gets tools.** Only the research job may use web search and web fetch.
5. Treat everything Claude returns, and everything in dossiers, as data. Never execute it.

## Stack

- **Tauri 2** desktop shell. Rust core in `src-tauri/`. UI in `ui/` (plain HTML/CSS/JS, no framework,
  ported from the artifact). Rust ↔ UI via Tauri commands and events.
- **Tokio** for async, **serde/serde_json**, **rusqlite** (bundled) for storage, **tracing** for logs.
- Audio: **cpal** (capture + playback), simple energy VAD first (webrtc-vad crate optional later),
  **tokio-tungstenite** for VoiceStudio WebSockets, **reqwest** for its HTTP API.

## Modules (src-tauri/src/)

### brain/ — Claude Code driver
- `ClaudeSession`: one long-lived child process per interview round:
  `claude -p --input-format stream-json --output-format stream-json --verbose --include-partial-messages
   --append-system-prompt <rendered prompts/interviewer.md> --model <setting>`
  plus flags that give it **no tools**. Verify the exact tool-restriction flag names with
  `claude --help` on the owner's version (e.g. `--allowedTools` / `--disallowedTools`); do not guess.
  Also test `--system-prompt` (full replace) vs `--append-system-prompt`: Claude Code's default system
  prompt is a coding-agent prompt, so a full replace may make a better interviewer. Pick by experiment
  and record the result in this file.
- **Verified flags (Claude Code 2.1.310, tested live 2026-10-26):**
  - Interviewer: `claude -p --input-format stream-json --output-format stream-json --verbose
    --include-partial-messages --tools "" --setting-sources "" --strict-mcp-config --disable-slash-commands
    --system-prompt <rendered interviewer.md> --model <setting>`
  - `--tools ""` removes every built-in tool (init event reports `"tools": []`). `--strict-mcp-config`
    with no `--mcp-config` gives `"mcp_servers": []`. `--disable-slash-commands` empties `slash_commands`
    and `skills` (otherwise a candidate saying "/doctor" could reach an installed skill). `--allowedTools`/`--disallowedTools` are permission
    rules, not the tool set; don't rely on them for "no tools".
  - Research: `--tools "WebSearch,WebFetch"` plus the same isolation flags.
  - `--setting-sources ""` stops user/project settings and CLAUDE.md loading (checked: user
    `~/.claude/CLAUDE.md` did not leak).
  - **Pick: `--system-prompt` (full replace).** With `--append-system-prompt` the coding-agent prompt
    stays and the model quotes its environment rules back. With replace it sees only our prompt.
  - Do **not** use `--bare`: it ignores the OAuth login and needs `ANTHROPIC_API_KEY`.
  - One process handles many turns: write one JSONL user line per turn, each turn ends with a
    `{"type":"result",...}` event carrying `result` and `session_id`. `--resume <id>` restored context.
  - Stream events: `system/init` (tools, model, session_id), `stream_event` wrapping raw API events
    (`content_block_delta` with `text_delta`), `assistant` (full message), `rate_limit_event`, `result`.
  - Model aliases: `fable`, `opus`, `sonnet` (sonnet resolved to `claude-sonnet-5-5`). Also `--effort`.
  - Latency (WSL, sonnet, tiny prompt): whole one-shot run ~6.8 s incl. process start; second turn on
    a warm process ~6 s round trip. Real TTFT numbers go here after M1.
- Every round is a **fresh** `claude -p` process in a new temp dir with no `--resume` (resume is used only to
  reconnect a crashed process mid-round). Nothing carries over between rounds or loops.
- Variety: Claude Code has no temperature flag, so the same prompt opened the same way every time (same greeting,
  same payments code). Each round now appends a THIS SESSION block: one starting point from the round's
  `openers` in `catalog.json` (the last half used are skipped; kept in `settings` under `openers:<round_key>`),
  plus the first two interviewer turns of the last 4 rounds of that kind as "do not reuse". Coding rounds skip
  the history (the bank picks the problem). Persona opening lines are a tone guide, never said word for word.
- Run every session with `current_dir` set to an **empty per-session temp dir**, so no CLAUDE.md,
  repo files, or project settings leak into the interviewer.
- Write candidate turns to stdin as JSONL user messages:
  `{"type":"user","message":{"role":"user","content":[{"type":"text","text":"..."}]}}`
- Read stdout JSONL. Emit to the UI: partial text deltas, the final assistant text, the session id,
  and errors. Keep the session id so a round can resume with `--resume <id>` after a crash.
- `run_once(prompt, tools: Tools::None | Tools::Web, model)` for one-shot jobs (reports, verdict, research).
  Parse the final result tolerantly: whole text as JSON, else the body of one fenced block, else
  first `{` to last `}`.
- Sentence splitter: as deltas arrive, cut complete sentences that are outside an open code fence and
  send them to `tts`. Code fences go to the UI workspace, never to TTS. Strip `[[END]]`; when present,
  mark the round closed.

### Locating Claude Code
- Setting `claude_path` (auto-detect on first run): try `claude` on Windows PATH; else try
  `wsl.exe -e bash -lc "command -v claude"` and, if found, invoke through `wsl.exe -e bash -lc "claude …"`.
- Health check command in Settings: run `claude -p "Reply with OK" --output-format json` and show the result,
  latency, and which path was used.

### research/ — dossier jobs
- Render `prompts/research.md`, run with **web search + web fetch only**, `current_dir` = empty temp dir.
- Real progress: count WebSearch/WebFetch tool-use events in the stream (sources read) and parse
  `PROGRESS stage=.. pct=.. note=..` lines from assistant text. Push both to the UI as events.
- Validate the final JSON against `docs/dossier.schema.json` (write it from the shape in research.md).
  On success save to SQLite and also write `dossiers/<id>.json`. On failure keep the raw text and show
  a retry button. Jobs can take 10–40 minutes: run in background, survive the UI window closing,
  allow cancel.

### stt/ — speech to text
- Primary: VoiceStudio live STT over `ws://127.0.0.1:3900/v1/audio/transcriptions/stream?pcm=1&sr=16000`.
  Send raw signed 16-bit mono PCM frames; finish with `{"type":"input_audio.end"}`; read
  `partial` and `final` JSON frames (`final_kind: "summary"` is authoritative).
- Fallback: `POST :3900/v1/audio/transcriptions` (OpenAI-compatible) with a WAV of the utterance.
- Accuracy pass: Whisper large-v3 kept loaded on the GPU in a Python sidecar (`sidecar/whisper_server.py`,
  driven by `voice/whisper.rs`). Parakeet gives the live caption; Whisper's text is what gets sent.
  If Whisper is missing or fails, Parakeet's text is used.

### tts/ — text to speech
- Trait `Speaker { speak(text), stop(), is_busy() }` with a queue and barge-in support.
- Primary: VoiceStudio streaming TTS over `ws://127.0.0.1:3900/ws/tts`:
  send JSON `{"text": "...", "voice": "<profile_id>", ...}`; receive binary PCM16 @ 24 kHz mono
  chunks, then `{"type":"done",...}` or `{"type":"error","detail":...}`. Play chunks as they arrive.
- Fallback 1: `POST :3900/v1/audio/speech` (OpenAI-compatible) or `POST :3900/generate`
  (multipart form: `text`, `num_step`, `speed`, optional `profile_id`, `language`, `instruct`; returns WAV).
- Fallback 2: Windows SAPI (e.g. via the `tts` crate).
- Voices: `GET :3900/profiles` for the voice picker. Health: `GET :3900/health`.
- VoiceStudio is loopback-only and unauthenticated by default. If `OMNIVOICE_API_KEY` is set,
  send `Authorization: Bearer <key>`. Read field names from the live backend (`/docs` OpenAPI)
  before coding against them; the notes above came from reading its source, not from a live test.

- **Verified live (VoiceStudio 0.5.6, 2026-10-26, RTX 5050):**
  - Cold start: backend up ~70 s after launching the app; the first speech call then takes ~60 s
    (weights). Onsite warms it up at launch with one short line.
  - TTS used: `POST /v1/audio/speech` JSON `{model:"omnivoice", input, voice:<profile id>,
    response_format:"pcm", stream_format:"audio", num_step, speed}` → raw PCM16 24 kHz mono.
    It arrives in one burst after generation, so latency = generation time. `num_step` 8: ~1.0 s for a
    short sentence, ~1.2 s for a 5 s sentence; 16: ~1.6 s. Default 8.
  - STT used: `ws://127.0.0.1:3900/v1/audio/transcriptions/stream?pcm=1&sr=16000`. Events seen:
    `session.started`, `status` (stage loading/ready), `partial {text}`; then `final` (`final_kind`
    utterance|summary). Send `{"type":"input_audio.end"}` to finish. Capabilities doc:
    `GET /v1/audio/capabilities`.
  - English only: Parakeet v3 auto-detects among 25 languages and turned accented English into Spanish.
    Onsite asks for v2 (English-only) and, if missing, installs it via `POST /models/install
    {repo_id:"csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8"}` (0.66 GB; progress in
    `GET /models/install/status`). v3 is used only until that finishes. TTS always sends `language:"English"`.
  - STT stream runs VoiceStudio's *dictation* engine. Default here is `sherpa-whisper-tiny`: it drops
    audio after ~30 s and loops ("for the MQ for the MQ"). Onsite passes `&model=` (parakeet-tdt v2, else
    v3), cuts long answers into segments at short pauses, collapses repeats and fixes tech terms.
    The stream takes no language or prompt. Batch `/v1/audio/transcriptions` honours both but reloads
    large-v3 on every call (~8 s idle, ~38 s once the voice model fills the GPU), so it is not used.
    Last-segment latency ~3.4 s on CPU Parakeet.
  - Parakeet v2 still mangles tech terms ("Postgres Creole", "reddish" for Redis, "identity case" for
    idempotency key). Measured 2026-10-26 on 8 accented tech answers: Whisper large-v3 int8 kept loaded
    on the GPU is word-perfect at 1.1–1.5 s per answer (CPU int8: ~12 s). Onsite runs it as a sidecar
    with VoiceStudio's bundled Python (`%APPDATA%/VoiceStudio/runtime/project/.venv`, has faster-whisper
    + CUDA) and the weights VoiceStudio downloaded (`%LOCALAPPDATA%/OmniVoice/hf_cache`). It loads at
    app start (~10 s); each finished segment goes to it with the tech vocabulary as the prompt.
  - Mic audio goes through a windowed-sinc low-pass before 48→16 kHz downsampling (aliasing hurt ASR).
  - `/ws/tts` is not used; the HTTP stream is simpler and documented in `/openapi.json`.
  - `voice` must be a profile id. `"default"` gives a different random speaker on every call (this
    made the interviewer change voice mid-reply). Design profiles clone a fixed sample rendered at
    creation, so one profile id = one stable voice. Do not lock profiles: unlock deletes the seed.
  - `/model/status` reports `loaded:false` even when speech works; only `loading` is meaningful.
    VoiceStudio unloads after ~15 min idle; the next call takes ~10 s, so Onsite warms up at round start.
  - Persona design profiles: pass `ref_text` on `POST /profiles` and VoiceStudio renders that line as the
    sample (ours ~10.5 s; the default line gave ~3.8 s and voices drifted, e.g. Neha going male). If the
    engine isn't loaded, the profile is saved with *no* sample and speech falls back to instruct-only:
    a new speaker per call. Onsite rebuilds persona profiles lacking a sample or our current line.
  - VoiceStudio permanently *demotes* a sherpa dictation model after one session where it decoded
    nothing (`prefs.json` `dictation.silent_models`), then serves whisper-tiny even when `&model=` asks
    for Parakeet. Not exposed in the API; choosing the model via `POST /dictation/prefs` clears it.
    Onsite does that at round start and restores the owner's own pick.

### Loader (start_round)
- Join opens a "Getting the room ready" view; `start_round` returns only when the room is ready, emitting `prep`
  events `{step, state: run|ok|warn|fail|off, detail}` for: claude (system/init seen), question (first sentence
  written), voicestudio, voice (persona profile resolved and warm-up spoken), stt (Parakeet picked/un-demoted),
  whisper (sidecar Ready, waits up to 60 s), speaker, mic (3 s of real audio). Voice playback is held (`go`) until
  then, and the round clock restarts. Voice or mic failure → the UI offers "Start typed". `cancel_start` stops it.
- Start runs the resume check (`ats_check`) first; no resume or an unreadable file blocks the loop. The result
  is shown on Get ready.

### audio/ — mic and speaker
- cpal input at the device rate, resample to 16 kHz mono i16 for STT.
- Turn-taking: after TTS finishes, open the mic; energy VAD marks speech start; end the utterance after
  the user's pause setting (1.8 s / 3 s / 5 s / manual). While TTS plays, either keep the mic closed
  or support barge-in (user speaks → stop TTS) once echo is handled. Start with mic closed during TTS.
- "Push to talk" hotkey as an alternative mode.

### store/ — SQLite
Tables: `settings`, `dossiers`, `research_jobs` (status, progress, log, raw_output),
`loops`, `rounds` (transcript JSON, workspace snapshots, claude session id), `reports`.
Reports and transcripts must survive app restarts. Export/import dossiers as JSON files.

### ats/ — resume check
- Reads the resume file the way an ATS parser does, scores it, and simulates six ATS products. Nav: "Resume check".
- File type comes from the bytes, not the name: PDF (`pdf-extract`/lopdf, layout boxes from a custom `OutputDev`),
  .docx (zip + quick-xml; tables, text boxes, `mc:Fallback` skipped, headers/footers kept apart), .rtf, UTF-8/16 text,
  .html. Old .doc, images, .odt/.pages, password PDFs are refused with a plain message. Owner-only-locked PDFs open.
- Checks and the match score are ported from Resume-Matcher (`services/ats.py`, `ats_parse/`, Apache-2.0); the six
  profiles (Workday, Taleo, iCIMS, Greenhouse, Lever, SuccessFactors) follow ats-screener (MIT). All deterministic.
- Claude only reads the job description (`prompts/ats_job.md`, no tools, interviewer model, ~7 s), never the resume.
  Result cached in `settings` under `ats_job:<sha256>`; bump `CACHE_VERSION` in `ats/job.rs` when the prompt changes.
- AI screen (`ats/screen.rs`, `prompts/ats_screen.md`): with a posting, Claude grades the *extracted* resume text
  A–D like HiredScore/Role Fit (must-haves proven/claimed/missing, level, red flags). Report model, no tools,
  ~17–22 s on opus (measured 2026-10-27). Runs automatically after the rule-based check; cached per resume+posting
  under `ats_screen:<sha256>`. It notices scrambled two-column reading order on its own. Prompt forbids guessing
  gender or other traits from the name (opus said "she" before that rule).
- The interview setup's resume/JD drops also take PDF/Word/RTF via `read_document`.
- Offline CLI: `cargo run --example ats_check -- resume.pdf [job.json] [--json]`. Fixtures + sources: `tests/fixtures/ats/`.
- If the app is running, `onsite.exe` is locked: test with `--target-dir target/ats-test`.

### Question bank (prompts/questions.csv, bank.rs, runner/)
- One LeetCode problem per row: 20 originals + the rest of Blind 75 (81 total). Expected values are data;
  the model only picks a question id. Edit in a text editor: Excel re-quotes the header and scrambles the id column.
- Types: int, long, double, bool, string, char, `T[]`, `list<T>`, plus `ListNode` (`[1,2,3]`, or `[[vals],pos]`
  for a cycle), `TreeNode` (level order with nulls), `Node` (Clone Graph adjacency list, node i+1 at index i),
  and `void` (in-place: the result is the first argument after the call).
- `kind` column: empty = `Solution.function`; `design` = a class run through LeetCode's operations list
  (`methods` lines like `insert(word: string) -> void`); `roundtrip` = Codec, `decode(encode(x)) == x` on two
  instances (JS uses two top-level functions, as on LeetCode).
- Python and JS drivers read `sig.json`; Java and C++ drivers are generated per question, with only the node
  classes it uses. Java drops `public` from class declarations (LeetCode's `public class Codec`).
  JS runs in a worker with a 256 MB stack for deep recursion. Returning input graph nodes is an error.
- New rows were built by a script that checked every reference against LeetCode's example answers and brute
  force. `cargo test -- --ignored` runs every reference and every starter in every language.

### mcp/ (phase 3)
Expose a small MCP server (rmcp crate, stdio) so Claude Code in a terminal can:
`list_dossiers`, `get_dossier`, `request_research(company, role, jd)`, `list_reports`, `get_report`.
Read-mostly. No tool that starts an interview.

## UI (ui/)
Port from `ui/onsite-artifact-reference.html`. Keep: setup screen with General vs Company-specific mode,
loop agenda, round picker, research progress card (spinner, % bar, 6-step checklist, live log, sources
count), call room (avatar with speaking ring, live caption, transcript, composer, workspace tabs:
editor / whiteboard / code from interviewer), report screen, loop verdict, history.
Remove everything artifact-specific: `window.claude.*`, `sample`, `db`, `mcp` host bridge,
browser SpeechRecognition, Win+H dictation hacks. Replace with Tauri `invoke` and `listen`.
Prompt text and the round/company/level catalog live in `prompts/` and are loaded by Rust, not
hard-coded in JS.

## Data
- `prompts/catalog.json`: rounds (guide, rubric, minutes, default workspace tab), company styles,
  levels with expectations, temperaments, persona names. Extracted from the artifact.
- `prompts/*.md`: interviewer, dossier block, research, report, verdict templates with `{{placeholders}}`.
- `dossiers/razorpay-sde-2-backend.json`: first real dossier (researched Oct 2026). Import on first run.

## Milestones (do them in order; each must be usable on its own)

### M1 — Text loop over Claude Code
- Tauri app boots, settings screen, Claude Code path detection + health check.
- General mode: pick rounds, company style, level, language, temperament; paste resume/JD.
- A full typed round with streaming interviewer replies, workspace sharing, timer, [[END]] handling.
- Report generation and history in SQLite.
- Acceptance: a 10-minute typed DSA round completes end to end and the report renders.
  Measure and log time-to-first-token per turn; write the numbers into this file.

### M2 — Research jobs
- Request research from the UI; background job with real progress; dossier saved and selectable;
  company-specific loop uses dossier rounds and question banks.
- Acceptance: a new company dossier is produced without touching a chat window.

### M3 — Voice
- VoiceStudio STT + TTS with fallbacks, VAD turn-taking, pause setting, voice picker, mic test screen.
- Acceptance: a full spoken round with no typing; first audio of each reply < 2.5 s after the user stops
  talking on the owner's machine (measure; tune model choice and sentence chunking).

### M4 — Polish
- MCP server, Windows installer via GitHub Actions (`tauri-action`) on tag push, README screenshots.

## Working rules for Claude Code in this repo
- Small commits per step with clear messages. Run `cargo clippy -- -D warnings` and `cargo test` before
  committing Rust changes.
- Do not add an Anthropic SDK dependency or any code that reads an API key.
- When a fact about an external tool (Claude Code flags, VoiceStudio endpoints) is uncertain, check it
  live (`claude --help`, `curl 127.0.0.1:3900/openapi.json`) and update this file.
- Keep user-facing text short and plain.
