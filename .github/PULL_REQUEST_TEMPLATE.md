<!-- Before you open a pull request, read CONTRIBUTING.md and CODE_OF_CONDUCT.md. -->

## Summary

<!-- What does this change, and why? One or two sentences. -->

Closes #

## Type of change

<!-- Tick one. -->

- [ ] Bug fix
- [ ] Feature
- [ ] Docs
- [ ] Questions or dossiers
- [ ] Refactor
- [ ] CI or build

## How it was tested

<!-- Fill in what you ran. Write "not tested" if that is the case. -->

- OS: <!-- Windows / macOS / Linux -->
- AI provider: <!-- Claude Code CLI / Claude API / OpenAI / Gemini / Groq / none -->
- Voice engine: <!-- built-in / VoiceStudio / typed only -->
- Round: <!-- typed / spoken -->

## Checklist

<!-- Tick what applies. Leave the rest unticked and say why in the summary. -->

- [ ] Rust changes: `cargo clippy -- -D warnings` and `cargo test` pass (run in `src-tauri/`)
- [ ] Docs in `docs/engineering/` are updated in this PR, if the code they describe changed
- [ ] Privacy text (`PRIVACY.md` and the in-app privacy section) is still true
- [ ] No em or en dashes in code, docs, UI copy or prompts
- [ ] No AI provider SDK added (plain `reqwest` only)
- [ ] No API keys, resumes or transcripts in code, logs or screenshots
- [ ] UI change: screenshots or a short clip are attached
- [ ] User-facing text is short and plain

<!-- Note: `SECURITY.md` covers security problems. Do not describe an open security issue in this PR. -->
