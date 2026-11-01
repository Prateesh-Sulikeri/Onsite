# Plan: multi-file workspace for the code comprehension round

Status: steps 1-4 built (2026-11-01), bundled in `projects/` instead of a separate repo and not downloaded yet (step 5 partial: picking and fallback done). Direction changed 2026-11-01: projects are written by the AI per loop (JD first, company dossier for company loops, then resume), bundled projects are the fallback, and no projects repo/download is planned. Test files read-only. See CODING.md (Projects). Not yet run in the live app. Background and research: `docs/comprehension-round-research.md`.

## Goal

In the comprehension round the candidate gets a small project (3-5 files) with a file tree, reads it, edits the
files to fix bugs or extend it, and sends the work to the interviewer for review. Nothing is run. The projects
are curated data from a separate open-source repo, not code the model writes.

## What the app does today (checked 2026-10-31)

- The interviewer pastes **one 60-120 line file** in one code fence (`prompts/catalog.json` → `comprehension.guide`).
- Each fence lands in the **From interviewer** tab (`ui/app.js` `session.shared`): read-only, line numbers, a
  "Snippet N" dropdown.
- The **Open in workspace** button on a code block only switches to that read-only tab (`ui/app.js:1374`). It does
  not load the code into the editor, so fixing a bug means copying it across by hand. The label is misleading.
- The **Editor** is one CodeMirror buffer per language (LeetCode style). No files, names or folders anywhere in
  the UI or in what is sent to the model.
- Lint is **syntax errors only** (Lezer parser): broken braces, bad indents. No unused-variable, type or null
  warnings.
- Run/Submit works only for question-bank problems (`runner/` needs tests). Nothing can run interviewer code.

## Decisions made

| Question | Choice | Why |
|---|---|---|
| Fork Zed or VS Code for editor + file tree + terminal | **No** | Zed renders with GPUI (no webview embed) and its editor crates are GPL-3.0 (Onsite is MIT). Code-OSS is Electron; code-server/openvscode-server means bundling Node + a server (100+ MB) and a local port. `monaco-vscode-api` adds tens of MB. A file tree on top of CodeMirror is far less work |
| Monaco instead of CodeMirror | **No** | CodeMirror already follows the theme tokens; Monaco needs JS theme objects and workers. Its smart completion is only good for JS/TS |
| Language servers (LSP) | **Not needed.** Lint is enough | See "LSP findings" below. Also, LSP warnings would point straight at planted bugs |
| Run tests in this round | **No** | Candidates code and send their work to the AI for review. No toolchains, nothing executed |
| Where projects come from | **The AI writes one per loop** (JD first, company dossier, then resume); 2 bundled projects are the fallback | Changed 2026-11-01. Fits company-specific loops. Nothing is run, so a planted bug can be wrong; setup warns about it. The separate repo and download are dropped |

### LSP findings (kept for later)

- `@codemirror/lsp-client` 6.3.0 (MIT) is the official client; its transport is pluggable, so a Rust stdio bridge
  over Tauri events would work.
- Owner's Windows PATH (2026-10-31): only `rust-analyzer`. No jdtls, clangd, pyright/basedpyright, pylsp,
  typescript-language-server or gopls. Java 26 is installed (jdtls needs 21+).
- VS Code's Pylance and cpptools are licence-locked to Microsoft's VS Code; another app can't use them. So "use
  installed servers" would give nothing for Java/Python on most machines; Onsite would have to download open
  servers (jdtls EPL-2.0, basedpyright MIT, clangd Apache-2.0, typescript-language-server MIT, gopls BSD).
- If ever built: jdtls must not see model-written build files (it runs Maven/Gradle); rust-analyzer with build
  scripts and proc macros off; gopls with `GOPROXY=off`.

## Design

### 1. Projects repo format

One folder per project:

```
projects/
  checkout-idempotency-java/
    project.json      metadata + hidden interviewer notes
    src/...           files the candidate sees
    solution.diff     reference fix (hidden)
```

Draft `project.json`:

```json
{
  "schema": 1,
  "id": "checkout-idempotency-java",
  "title": "Checkout with retries",
  "language": "java",
  "difficulty": "Medium",
  "levels": ["sde2", "senior"],
  "topics": ["idempotency", "concurrency"],
  "companies": ["fintech"],
  "minutes": 45,
  "brief": "Customers report being charged twice when the payment gateway times out. Find out why.",
  "files": [
    {"path": "src/CheckoutService.java", "open": true},
    {"path": "src/PaymentGateway.java"},
    {"path": "src/OrderRepo.java"},
    {"path": "test/CheckoutServiceTest.java", "readonly": true}
  ],
  "issues": [
    {
      "id": "retry-no-key",
      "file": "src/CheckoutService.java",
      "lines": [42, 55],
      "category": "idempotency",
      "why": "Retries call charge() again with no idempotency key.",
      "good_fix": "Generate one key per order and pass it on every retry; store the result."
    }
  ],
  "extension": "Add partial refunds without breaking the retry fix."
}
```

`brief` is shown to the candidate. `issues`, `extension` and `solution.diff` go only to the interviewer and the
report. A public repo holding the answers is fine; `questions.csv` already ships reference solutions.

Repo rules:
- 3-5 files, about 250-400 lines in total (see the token budget below).
- One language per project. Start with Java and Python; add others as people contribute them.
- No comments that give a planted bug away. Every bug can be found by reading.
- MIT or Apache-2.0. Write originals inspired by Defects4J / BugsInPy; don't copy them.

### 2. Getting projects into Onsite

- Proposed: download a release zip of the projects repo from github.com (same rule as `updates.rs`) into
  `<app data>/projects/<version>/`, with an "Update projects" button. Ignore manifests with an unknown `schema`.
- Treat it as data (non-negotiable 5): reject absolute paths and `..`, cap file count and total size, never
  execute anything.
- Picking: like `Bank::pick`: language, then level/difficulty, topics, company; skip recently used ids.
- Fallback when nothing matches or nothing is downloaded: today's mode (the model writes one file).

### 3. Workspace UI

- File tree on the left of the Editor pane (collapsible); tabs for open files; a dot on modified files.
- Per-file **Reset** and a **Changes** view (diff against the original). **New file** for the extension task.
- Language per file from its extension, so the current CodeMirror setup and syntax lint keep working.
- Fix the **Open in workspace** button: interviewer code should open in an editable buffer.
- The From interviewer tab stays for snippets typed in chat.

### 4. Sending work to the AI

- **First turn:** the interviewer gets the hidden notes plus the full project once, so it can discuss any line. The
  code is on screen; the interviewer never pastes it.
- **Later sends:** only unified diffs against the original, for files changed since the last send, with paths.
  Extends today's "send only what changed" + **Share now** (`WS-R1`).
- **Checking:** the interviewer compares the diff with the planted issue and `good_fix` / `solution.diff`, so
  review without running is grounded, not a guess.
- **Report:** final full diff + `issues` → each bug graded found / fixed / fixed badly / missed.
- **Storage:** final diff in the round's workspace snapshot; History shows it.

### 5. Token budget

Groq free tier is 8K tokens/minute and a coding turn already sends ~5.4K. The first comprehension turn carries
the whole project. Keep projects small, or show "this round needs a paid provider" on Groq.

### 6. Later reuse

Same format with `issues` empty and a starter skeleton covers the **machine-coding** and **LLD** rounds.

## Open questions

All answered 2026-11-01: projects are AI-written with bundled fallbacks (no runtime download); test files are read-only; the model-written single file is only the last fallback.

## Suggested order

1. Draft the `project.json` schema in the new repo; build 2-3 example projects.
2. Fix Open in workspace (editable buffer).
3. File tree, tabs, Reset, Changes view.
4. Diff-only sending + interviewer and report prompt changes.
5. Project download, picking and fallback.
