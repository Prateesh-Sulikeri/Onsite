# STORAGE — `src/store/`, `src/storage.rs`, privacy

All data local. One SQLite file + a few files. Shape lives in Rust types (JSON columns, `#[serde(default)]` so new fields
need no migration).

## Locations

| Data | Windows | macOS · Linux |
|---|---|---|
| `onsite.db` | `%APPDATA%\dev.onsite.app\` (Tauri app data, identifier `dev.onsite.app`) | `~/Library/Application Support/dev.onsite.app/` · `~/.local/share/dev.onsite.app/` |
| `dossiers/<id>.json`, `backdrop.{jpg,png,webp}` | app data | app data |
| `onsite.log` | `%LOCALAPPDATA%\Onsite\` | `~/Library/Logs/Onsite/` · `$XDG_STATE_HOME/Onsite/` or `~/.local/state/Onsite/` |
| temp | `onsite-round-*`, `onsite-job-*` (auto-deleted), `onsite-stt/` (Whisper wavs), `onsite-update/` (installers) | system temp |
| UI prefs | WebView localStorage `onsite.*` | same |

## Schema (`store/schema.sql`; `PRAGMA user_version` = `SCHEMA_VERSION` = 1; WAL; foreign_keys ON)

```text
settings(key PK, value JSON)
loops(id PK, config LoopSetup JSON, verdict JSON?, created_at)
rounds(id PK, loop_id→loops, idx, round_key, name, persona, minutes, model, claude_session_id?,
       transcript Turn[] JSON default '[]', workspace {code,notes} JSON default '{}', started_at, ended_at? -- NULL = live)
reports(id PK, round_id→rounds UNIQUE, data JSON? -- NULL = unparsable, raw TEXT, created_at)
dossiers(id PK slug, company, role, level, data Dossier JSON, created_at)
research_jobs(id PK, request JSON, status running|done|failed, progress, log, raw_output?, dossier_id→dossiers?, created_at, updated_at)
```

Times RFC 3339 UTC. Migration: tables are `CREATE TABLE IF NOT EXISTS`; a real migration bumps `SCHEMA_VERSION` and adds a
step in `Store::init`.

`Turn {who "iv"|"me", text, at_min, ttft_ms?, shared_workspace?, sent?}`; `sent` = exact text the model received.

Store API (`store/mod.rs`): settings get/set · `create_loop loop_config set_loop_verdict loop_verdict` · `create_round
save_round_progress finish_round round recent_transcripts loop_round_ids delete_round delete_unanswered_rounds
rounds_over_limit delete_empty_loops history counts vacuum` · `save_report report` · `save_dossier dossiers dossier
delete_dossier` · `create_job update_job job jobs delete_job fail_running_jobs`.
`delete_round` also deletes its report and nulls the loop verdict.

## Settings keys

| Key | Value |
|---|---|
| `app` | `Settings {provider, claude: ClaudeLocation?, interviewer_model (sonnet), report_model (opus), voice: VoiceSettings {engine, builtin_voice, ...}}` |
| `api_keys` | `{provider_id: key}` — never to UI/logs |
| `storage_limits` | `{keep_rounds 0=all, keep_days 0=forever, log_max_mb 20}` |
| `weekly_goal` | `{round_key: n}`, default `{coding:2, sysdesign:1, behavioral:1}`; `set_goal` drops unknown keys/zeros |
| `openers:<round_key>` | recent openers |
| `ats_job:<hash>` / `ats_screen:<hash>` / `ats_last` | resume-check caches / last result |
| `dossiers_seeded` | bool |

localStorage: `onsite.theme {family, mode, image, plain, dim}` · `onsite.cfg` (last setup) · `onsite.resumeFile`
(**base64 resume file**) · `onsite.atsJd` · `onsite.setSec` · `onsite.rail` · `onsite.probShare` · `onsite.readyEvery`.

## Retention (`storage::enforce`, at startup and on `set_storage_limits`)

Finished rounds over `keep_rounds` (newest kept) or older than `keep_days` → `delete_round`; empty loops older than 1 day
deleted; log over `log_max_mb` → truncated in place. Live rounds (`ended_at IS NULL`) never touched. Startup also:
`delete_unanswered_rounds`, `fail_running_jobs`. Clean-up commands: `clear_log`, `clear_temp` (onsite-update, onsite-stt;
in-use files skipped), `vacuum_db`, `open_folder`.
Backdrop: UI shrinks to ≤ 2560×1600 JPEG → base64 → `set_backdrop` checks type from bytes (`image_ext`: jpg/png/webp),
writes `backdrop.<ext>`, removes others.

## Uninstall (`uninstall.rs`, `src-tauri/windows/hooks.nsh`)

Two tiers. **Residue** (voice models `<data>/models`, log dir, temp `onsite-update`, `onsite-stt`, `onsite-round-*`,
`onsite-job-*`) goes on every real uninstall. **User data** (app data dir with `onsite.db`, dossiers, backdrop; app
local data = WebView storage incl. the localStorage resume copy; macOS `~/Library/{WebKit,HTTPStorages}/<id>`, saved
state, prefs plist) goes unless the user keeps it.
- Windows: NSIS hooks. `POSTUNINSTALL` deletes residue unless `$UpdateMode` (`/UPDATE`); Tauri's own "Delete the
  application data" box deletes `$APPDATA`/`$LOCALAPPDATA\<id>`. `PREUNINSTALL` ticks that box when `/PURGE` is passed.
  In-app updates run the setup with `/UPDATE`, so the old uninstaller (and the hook) never runs on update. A manual
  setup where the user picks "uninstall first" does run it: models re-download (~580 MB).
- Settings → Storage → Uninstall (`uninstall_app {keepData}`), refused while a round/research runs
  (`updates::ensure_idle`): deletes `Onsite · *` profiles in VoiceStudio (best effort, 20 s), then Windows runs
  `uninstall.exe /P [/PURGE]`; macOS/AppImage spawn a detached `sh` that waits for the pid, then `rm -rf` the files
  and the .app / AppImage; .deb first runs `pkexec apt-get remove -y <dpkg -S pkg>`, then the same `sh` for files.
  Keep the two residue lists (Rust `residue()`, hooks.nsh) in step.
- Not removed: anything VoiceStudio owns; the Windows uninstaller path doesn't touch VoiceStudio voices.
- macOS and Linux paths not yet run by a person (2026-10-30). The NSIS hook is checked only by the CI build.

## Concurrency

One `rusqlite::Connection` in `Arc<std::sync::Mutex<Store>>`. Never hold across `.await`: read → drop lock → model call →
lock → save.

## Privacy flows (UI text in `ui/index.html` Privacy section + `PRIVACY.md`; keep both true)

Local: resume/JD, transcripts, code, whiteboards, feedback, dossiers, settings, voice audio, API keys (unencrypted), log
(errors/timings only). Outbound: chosen AI provider (prompt, resume, JD, answers, shared workspace; also feedback,
resume check, research) · research web search by that provider · GitHub release lists on Check for updates. Never:
accounts, analytics, telemetry, data to the developer.
**Rule:** a new outbound flow ships only with updated Privacy text + `PRIVACY.md`.

## Gaps

- Privacy text says resume is stored in the database; it is also in localStorage (`onsite.resumeFile`).
- API keys unencrypted (documented to user).
