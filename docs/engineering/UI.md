# UI — `ui/`

Plain HTML/CSS/JS in the system WebView. No framework, no build step (except vendored bundles). Tauri serves only `ui/`.
CSP: `default-src 'self'`, no inline script, no network except IPC.

## Files

| File | Role |
|---|---|
| `index.html` | all views as `<section class="view" id="view-<name>" hidden>`, SVG icon sprite (`#i-*`), dialogs `#askDlg`, `#termDlg`, `#toasts`, `#srLive` |
| `app.js` (~2.6K lines, globals) | sections in order: round guidance (`PLANS`, `TIPS`, `ROUND_ICONS`) · shell (`nav`, `ask`, `toast`, `announce`, `refreshStatus`, `PROVIDERS`) · dashboard · setup · call room · preparing · coding problem + runner · rendering (`addMsg`, `md`, `inlineMd`, `esc`) · feedback + report · interview feedback · resume check · dossiers and research · settings · audio devices · get ready · init |
| `app.css` (~920) | `:root` tokens (Onsite dark) → primitives → one `/* ---------- name ---------- */` section per screen |
| `themes.css` | other themes: base tokens per `[data-theme]`; shared block derives the rest via `color-mix`; light-mode tweaks |
| `theme-boot.js` | in `<head>`: `OnsiteTheme.read/apply/save/setImage`; sets `data-theme="<family>-<mode>"`, `data-mode` before paint |
| `ipc.js` | `window.api.invoke(cmd,args)` / `api.listen(event,cb)` / `api.log`; logs invoke failures + window errors via `ui_log` |
| `vendor/editor.js` | CodeMirror 6 bundle → `OnsiteEditor.create(el,{lang})` (`set`, `setLang`, …) |
| `vendor/whiteboard/` | Excalidraw 0.18 split modules, `import()` on first Whiteboard open → `OnsiteBoard` (`describe`, `reset`, `refresh`) |
| `assets/` | `logo.png`, `twilight.webp` (default backdrop) |

`ui-vendor/` (npm, esbuild) builds `ui/vendor/`: `npm install && npm run build`, commit output.

## Conventions

- Navigation: `[data-nav=x]` click → `nav(x)` hides other views, marks sidebar, calls the view's render fn. `data-sec` on a
  settings link sets `SET_SEC`. Leaving a live round asks first.
- Global state: `CAT PANELS SETTINGS AIKEYS VS DOSSIERS loop{id,setup,count} session{roundId,index,key,persona,phase,voiceOn,muted,closed,busy,…}`.
- **Every dynamic value through `esc()` before `innerHTML`.** Interviewer text via `md()` (escape-first subset).
- Errors: `errText(e)` + `toast(msg,"err")`. Confirms: `await ask(title, body, okLabel, danger)`.
- localStorage only for UI prefs (`onsite.*`; list in STORAGE.md), wrapped by `LS.get/set`.
- First run with no AI configured → Settings › AI model.

## Design tokens ("night violet")

Sampled from `assets/twilight.webp`. OKLCH. Navy neutrals hue 266, violet accent hue 290, warm accents rare.

| Token | Value (Onsite dark) | Use |
|---|---|---|
| `--bg` `--side` `--surface` `--surface-2` `--surface-3` | oklch(.145 .042 266) · (.13 .04 266) · (.19 .05 268) · (.23 .06 270) · (.28 .075 274) | backgrounds, panels |
| `--ink` `--ink-2` `--muted` | (.97 .01 280) · (.86 .03 280) · (.74 .045 278) | text |
| `--accent` `--accent-hi` `--accent-text` | (.57 .21 290) · (.68 .18 292) · (.8 .12 294) | primary actions, focus, live state |
| `--warm` `--warm-2` | (.83 .1 46) peach · (.6 .15 337) magenta | rare emphasis, tips |
| `--ok` `--warn` `--bad` | (.76 .15 160) · (.83 .12 70) · (.69 .19 18) | status only |
| `--syn-*` | kw/type/fn/str/num/comment/meta/gutter/line/select/match | editor |
| derived | `--panel --field --line --line-2 --hover --sel-bg --chip-bg --accent-line --accent-wash --glow --glow-sm --shadow --bubble --bubble-me --pane --scrim --focus …` | components |
| `--font` / `--font-display` / `--mono` | Segoe UI Variable Text / Display / Cascadia Code | type |
| `--r-sm --r --r-lg --r-xl` | 9 / 12 / 16 / 20 px | radii |
| `--ease` | cubic-bezier(.22,1,.36,1) | transitions |

Themes: families `onsite nord rosepine gruvbox` × `light dark`, mode `system|light|dark`; default Onsite dark (= app.css
`:root` unchanged). Other themes set ~25 base tokens; derived ones come from `themes.css` shared block. Backdrop
`--backdrop`: theme image, user image (dim veil 0–85%, default 35%), or none (`plain`); never in the call room.
Whiteboard is always dark.

## Rules for new UI

1. Tokens only; no colour literals (only status glows exist as literals). New token → add to `:root` and derive in `themes.css`.
2. One primary (violet) action per area; destructive = `.danger` + confirm.
3. `esc()` everything; no `innerHTML` with raw model/Rust strings.
4. Copy: short plain sentences, name the fix; write empty + error states; never placeholder numbers.
5. No new frameworks; third-party code goes through `ui-vendor/`, lazy-loaded if heavy.
6. Accessibility: roles/labels for non-native controls (`radiogroup`, `tablist`, `log`, focusable `separator`), keep
   `:focus-visible` (2 px `--accent-hi`), `aria-live` for status, `prefers-reduced-motion` for animations.
7. Check all 4 families × light/dark and the call room.

## Components (class → use)

`.btn` (`.primary .ghost .danger`, `.sm .lg .xl`), `.icon-btn` · `.panel` (`.tight`, `.panel-head`) · `.panel.step` + `.step-n` ·
`.field` > label + `.sel` (icon + select) + `small.hint` · `.mode[role=radio]` · `.tiles > .chip` (round tiles) ·
`ol.timeline` (loop agenda) · `.pill` `.tag` · `.sys` + `.dot` ok/warn/bad (sidebar chips) · `<dialog class="dlg">` ·
`.orb` + `.wave` (`data-state`) · `.msg.iv/.me` > `.bubble` · `.tabs [role=tab]` + `.pane[data-pane]` · `aside.rail` ·
`ring()` / `.meter`.

## Screens

| View id | Purpose / key parts |
|---|---|
| `dashboard` | start / review last round, weekly goal, this week ring, skill performance, score trend (bar 70), practise next, calendar + streak, highlights, recent |
| `interviews` | mode cards (general/company) · steps 1–5: context (company style or dossier, temperament, panel, level, language) · round tiles · resume + JD drops · notes · round length → Start; right: loop timeline, what to expect |
| `ready` | mic/speaker pickers, live meter, Test speaker/mic, resume-check result, Join; "Check before every round" (`onsite.readyEvery`; else next rounds skip to loader) |
| `prep` | checklist from `prep` events (`PREP_STEPS`: AI model, First question, VoiceStudio, Interviewer's voice, Live captions, Accurate transcript, Speaker, Microphone; Waiting/Ready/Fallback/Failed/Off); Cancel; Start typed on voice failure |
| `room` | header (company, round, level, minutes, round x/n, interviewer, mic picker, countdown timer, Context, Terminate) · left: problem panel + split + transcript + closed banner + composer · controls: Mute, orb (phases: thinking, speaking "tap to cut in", listening, hearing, transcribing, muted, closed, idle), End · right: Editor/Whiteboard/Notes/From interviewer · rail: context chips, round plan (`PLANS`), tips. Typed round hides orb/mic controls |
| `report` | round report (score ring, decision, dimensions + evidence, went well/held back, fix first, moments, practise next, transcript) or loop feedback (verdict + rounds) |
| `feedback` | history grouped by loop, scores by band, delete round |
| `resume` | file drop + posting → score rings, ATS rows, keywords, checks, extracted preview, tips, AI screen card |
| `dossiers` | list + Import; research form; job cards (6 stages, %, log, searches/pages); dossier detail (use/export/delete) |
| `settings` | sections: appearance · models (provider cards, path/key, health check, two model pickers) · voice (status, start, devices, voice, quality, speed, pause, tests, interviewer cards + Play voice) · updates · privacy · storage |

Flow: dashboard → interviews → (resume check) → ready → prep → room → report → next round (ready/prep) … → loop feedback.

## Add a screen (recipe)

1. `index.html`: `<section class="view" id="view-x" hidden>` with `header.page-head` (`.crumb`, `h1` + `<em class="hl">`, `p.lede`).
2. Sidebar: `<button class="nav-item" data-nav="x"><svg><use href="#i-…"/></svg><span>X</span></button>`.
3. `app.js`: new `/* ---------------- x ---------------- */` section with `renderX()`; call it in `nav()`.
4. `app.css`: new section, tokens only.
5. New data → command in `commands.rs` + register in `lib.rs`; logic in a module.
6. Retake screenshots if the human PDFs get refreshed (`docs/engineering/tools`: `npm run shots`).

## Voice visibility (which engine runs a round)
- Helpers near `builtinVoice` in app.js: `clipFor/bvClip/bvName` (built-in voice per interviewer by gender; id `priya_shah` is shown as Charlotte), `needsVS(panel)` (any interviewer `voice` string mentions "indian"; data-driven, no ids), `panelBlocked(panel)` (voice enabled + engine builtin + needsVS), `modeLine(who)` ("Spoken · Built-in voice (Ethan)" / "Spoken · VoiceStudio" / "Typed").
- Room: `#voiceChip` in `.room-meta` and `#voiceWarn` banner under `.room-head`, both drawn by `renderVoiceChip()` (called from `applyVoiceUi`, so it follows `s.voiceOn` after `start_round`). Banner shows when `s.wanted && !s.voiceOn`; reason = first warn/fail `prep` detail for voicestudio/voice/speaker/mic (`s.voiceWhy`); dismiss sets `s.warnOff`. Not shown when voice is off in Settings.
- Setup: `syncPanels()` (run at end of `refreshStatus`) disables blocked `<option>`s, relabels them, moves a blocked selection to the first allowed panel or Mixed (`panelNote`), shows `#panelWarn`.
- Get ready: `#readyMode` line and `#readyPanelWarn` (loop already made with a blocked panel; does not block). Loader subtitle includes `modeLine`. Persona cards in Settings show the built-in voice, or "VoiceStudio only", when engine is builtin.

## Gaps

Whiteboard theme fixed dark · no full keyboard support in whiteboard · `--muted` contrast over photos depends on dim.
