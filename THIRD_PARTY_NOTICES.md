# Third-party notices

Onsite is MIT-licensed (see `LICENSE`). It includes or is based on the work below. Each keeps its own license.

## Code ported into Onsite

### Resume-Matcher — Apache License 2.0
https://github.com/srbhr/Resume-Matcher. Copyright the Resume-Matcher authors.

The resume checks and the job match score were ported from Python to Rust and changed to fit Onsite:
- `src-tauri/src/ats/checks.rs` from `services/ats_parse/checks.py`
- `src-tauri/src/ats/score.rs` (match score) from `services/ats.py`
- `src-tauri/src/ats/text.rs` from `services/text_match.py` and `services/ats_parse/{normalize,headings,checks}.py`

These files stay under the Apache License 2.0. Full text: `LICENSES/Apache-2.0.txt`.

### ats-screener — MIT License
https://github.com/sunnypatell/ats-screener. Copyright (c) 2026 Sunny Patel.

The six ATS profiles in `src-tauri/src/ats/score.rs` follow its `src/lib/engine/scorer/profiles`.
Full text: `LICENSES/ats-screener-MIT.txt`.

## Bundled in `ui/vendor/` (built by `ui-vendor/`)

| Project | License | Used for |
| --- | --- | --- |
| [CodeMirror 6](https://codemirror.net/) (`@codemirror/*`, `@lezer/*`) | MIT | Code editor |
| [Excalidraw](https://github.com/excalidraw/excalidraw) | MIT | Whiteboard |
| [React](https://react.dev/) | MIT | Runs Excalidraw |

esbuild writes the license comments of everything in each bundle to the `*.LEGAL.txt` file next to it.

## Rust crates

Onsite's Rust core uses crates from crates.io, among them Tauri, Tokio, serde, rusqlite (bundles SQLite, public
domain), reqwest, cpal, tokio-tungstenite, pdf-extract, quick-xml and zip. Most are MIT or Apache-2.0; `Cargo.lock`
lists the exact versions. To produce the full list with license texts:

```
cargo install cargo-about
cd src-tauri && cargo about generate about.hbs > ../licenses-rust.html
```

## Images

The background (`ui/assets/twilight.webp`, from `public/images/`) was generated with ChatGPT. OpenAI's Terms of Use
assign their rights in generated output to the user, so it is released here under the MIT License with the rest of Onsite.

## Not bundled

Onsite talks to these but does not include them: VoiceStudio, Whisper and Parakeet models (downloaded by
VoiceStudio), Claude Code, and the AI provider APIs. Their own terms apply.
