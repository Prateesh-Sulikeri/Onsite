# Contributing to Onsite

Thanks for helping. Bug reports, fixes, new interview questions and dossiers are all welcome.

## Reporting a bug
Use the button in the app: the **Report a bug** button in the sidebar. It shows you the full report, fills in
your version, system and AI provider, removes keys and file paths, and opens the Onsite issue form on GitHub with
everything filled in. You review it and press Submit there. I aim to reply within a week.
You can also open an issue with the **Bug report** form yourself. Include your system, the Onsite version
(Settings → Updates) and which AI you use. The log helps most: tick "Copy recent log lines" in the app, or use
Settings → Storage → Open log folder. Remove anything private before you paste it.

Never paste an API key, your resume or an interview transcript into an issue. Onsite never puts keys in the log.
Questions go to [Discussions](https://github.com/Prateesh-Sulikeri/Onsite/discussions). Security problems go
through the private route in `SECURITY.md`, not a public issue.

## Sending a change
1. Fork the repo and make a branch from `main`.
2. Build it (see **Building** in `README.md`).
3. Before you open the pull request, in `src-tauri/`:
   ```
   cargo clippy -- -D warnings
   cargo test
   ```
4. Keep each pull request to one change, and say what it fixes and how you tested it.

`CLAUDE.md` describes how the app is put together: modules, verified Claude Code flags, VoiceStudio endpoints.
Read the part for the area you are changing.

## House rules
- Follow the [Code of Conduct](CODE_OF_CONDUCT.md).
- Write UI text in short, plain sentences.
- No AI provider SDKs. Calls use plain `reqwest`.
- The interviewer never gets tools. Only the research job may search the web.
- Use the CSS colour tokens in `ui/app.css`, not new colour values, so every theme still works.
- Third-party UI code goes through `ui-vendor/` (`npm install && npm run build`), and the built `ui/vendor/` is committed.
- Questions live in `prompts/questions.csv`. Edit it in a text editor, not Excel.

## License
By sending a pull request you agree that your change is released under the MIT License (`LICENSE`).
