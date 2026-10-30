# AI — `src/brain/`

Hides which AI is used. Two entry points: a live `Session` per round, and `run_once` for one-shot jobs. Both work on
Claude Code CLI and four HTTP APIs. Verified CLI facts and measured latencies: CLAUDE.md → *brain/*.

## Map

| File | Contents |
|---|---|
| `chat.rs` | `Backend {Cli(ClaudeLocation), Api(ApiClient)}`, `Session {Cli(Box<ClaudeSession>), Api(ApiSession)}` (start/send/busy/is_alive/session_id/shutdown/discard), `ApiSession`, `api_once` |
| `session.rs` | `ClaudeSession` (child process), `read_loop`, `TurnState` (shared by both backends), `SessionConfig {backend, model, system_prompt, resume}`, `SessionEvent`, `crash_message`, `clean_final` |
| `api.rs` | `Provider` (claude_code/anthropic/openai/gemini/groq), `ApiClient::{chat, research, models}`, per-provider request/stream code, `SseParser`, retries, rate-limit waits, `chat_models` filter, `http_error` |
| `oneshot.rs` | `run_once`, `run_once_with_timeout`, `run_cli`, `OneShot {text, json, duration_ms, model}`, `JOB_SYSTEM_PROMPT` |
| `stream.rs` | `parse_line` / `from_value`: Claude stream-json → `StreamEvent` (TextDelta, AssistantText, ToolUse{name}, Result{…}, …) |
| `splitter.rs` | `SentenceSplitter` → `Piece::{Sentence, Code{lang,code}}`; strips `[[END]]` |
| `parse.rs` | `extract_json`: whole text → fenced block body → first `{`..last `}` |
| `prompts.rs` | `load` (drops leading `<!-- -->` header), `render` (`{{name}}`; unknown left as is) |
| `locate.rs` | `ClaudeLocation {Windows{exe}, Wsl}`, `detect`, `command`, `to_wsl_path`, `sh_quote`, `isolation_args`, `Tools {None, Web}` |

## CLI backend

`isolation_args(tools, model, prompt_file)` (every CLI call):
`-p --tools "" | "WebSearch,WebFetch" --setting-sources "" --strict-mcp-config --disable-slash-commands --model <m> --system-prompt-file <tmp>/.system-prompt.md`
Session adds: `--input-format stream-json --output-format stream-json --verbose --include-partial-messages [--resume <id>]`.
One-shot adds: `--output-format stream-json --verbose`; prompt written to **stdin** (Windows cmdline length).

- System prompt = full replace via **file** (`--system-prompt-file`; cmdline length).
- cwd = empty temp dir. Windows child processes use `CREATE_NO_WINDOW`; `kill_on_drop(true)`.
- WSL: `wsl.exe -e bash -lc "cd <wsl path> && exec claude <quoted args>"`; paths via `to_wsl_path`.
- `detect`: `claude.exe` on Windows PATH, else `wsl.exe -e bash -lc "command -v claude"`. Saved in settings; re-detected if
  the saved exe is gone (`AppState::claude_location`).
- Session turn: JSONL `{"type":"user","message":{"role":"user","content":[{"type":"text","text":…}]}}`; reply ends at
  `{"type":"result",…}`. `pending` queue of send instants → `busy(stale)`, TTFT.
- stderr logged; last 20 lines kept for `crash_message`. `shutdown`: close stdin, kill after 5 s. `discard`: kill without
  crash report (replacing a dead session).

## API backend

`ApiSession`: history in memory; one streamed request per turn, `Effort::Low`; failed turn keeps the user msg in
history (next answer joins it). One-shot: `chat` with `Effort::Medium`, or `research` (web tools) with `api_once`.

| | Claude API | OpenAI | Gemini | Groq |
|---|---|---|---|---|
| endpoint | `POST /v1/messages` SSE | `POST /v1/responses` | `models/{m}:streamGenerateContent?alt=sse` | `api.groq.com/openai/v1/chat/completions` |
| effort | `low` live / `medium` jobs, Claude 5 & 4.6+ (`claude_has_effort`) | `reasoning.effort` gpt-5/6/o* | — | `reasoning_effort` + `include_reasoning:false` on gpt-oss |
| extras | `fallbacks:"default"` + beta header on current models; 400 naming fallbacks → resend without | `store:false` | skip `thought` parts | `stop` on `[Elapsed:`, `[Workspace]`, `[The candidate`; strip `【…†L…】` citations |
| research | `web_search`/`web_fetch` (`_20260209` on 4.6+ else older); not streamed; `pause_turn` → resend ≤ 8× | `web_search` | `google_search`; count from `groundingMetadata.webSearchQueries` | `browser_search`, `tool_choice:"auto"`, not streamed, ≤ 3 tries on 500/"Parsing failed"/tool validation; model = report model if gpt-oss else `openai/gpt-oss-120b` |

`chat_models` hides `groq/compound*` (they run tools themselves). Defaults/model ids: CLAUDE.md.

## Events (`SessionEvent`, emitted as `round`)

`Started{session_id(""on API),model}` · `Delta{text}` · `Sentence{text}` · `Code{lang,code}` ·
`TurnDone{text,ttft_ms,total_ms,ended}` · `Error{message,recoverable}` · `Exited`. `TurnState::apply` produces them
from `StreamEvent`s for both backends.

## Splitter rules

Sentence ends at `. ! ?` (complete prefix), not after `e.g i.e E.g I.e vs etc Mr Mrs Ms Dr` (case-sensitive). Lines
inside ``` fences → `Piece::Code` (never TTS). `[[END]]` removed; reported by `finish()`.
Example: `"Okay. See e.g. retry." + "```java\nretry();\n```\nWhat breaks?\n[[END]]"` →
`Sentence(Okay.)`, `Sentence(See e.g. retry.)`, `Code{java,"retry();\n"}`, `Sentence(What breaks?)`, ended=true.

## One-shot callers

| Job | Prompt | Tools | Model setting | Caller |
|---|---|---|---|---|
| round report | `report.md` | none | report | `interview::live::generate_report` |
| loop verdict | `verdict.md` | none | report | `interview::live::loop_verdict` |
| research | `research.md` | web | report | `research::research` |
| ATS posting keywords | `ats_job.md` | none | interviewer | `ats::job::keywords` |
| AI screen | `ats_screen.md` | none | report | `ats::screen::screen` |

## Constants

| Name | Value | File |
|---|---|---|
| `CHAT_TIMEOUT` | 5 min | api.rs |
| `RESEARCH_TIMEOUT` (API) | 25 min | api.rs |
| 429 wait | ≤ 60 s each (`retry-after`), `MAX_RATE_WAIT` 90 s total | api.rs |
| `JOB_TIMEOUT` | 10 min | oneshot.rs |
| `RESEARCH_LIMIT` | 45 min | research.rs |
| `REPLY_STALE` | 120 s | interview/live.rs |

## Change recipes

- **New one-shot job:** template in `prompts/`, render with `prompts::render`, `run_once(&state.backend().await?, &p, Tools::None, &model, |_|{})`,
  use `out.json` (handle `None` with a plain error, keep `out.text`).
- **New provider:** add `Provider` variant (+`id`, `label`), request/stream in `ApiClient::chat` and `research`,
  model filter in `chat_models`, error text in `http_error`, Settings card in `ui/app.js` `PROVIDERS`, PRIVACY text.
  Must send no tools on chat. Live-test with `examples/api_e2e.rs`.
- **Changing CLI flags:** verify on the installed version (`claude --help`) and update CLAUDE.md.

## Gotchas

- `--allowedTools/--disallowedTools` are permission rules, not the tool set. Use `--tools ""`.
- `--append-system-prompt` keeps the coding-agent prompt (model quotes env rules). Use full replace.
- `--bare` ignores OAuth login.
- gpt-oss-120b on Groq runs on into a fake candidate turn without the `stop` list; Qwen is the Groq interviewer default.
- Groq "required" tool_choice made gpt-oss call a made-up `json` tool.

## Gaps

- Claude API, OpenAI, Gemini never run against a live key.
- CLI ~6 s/turn makes the voice latency target unreachable on Claude Code.
