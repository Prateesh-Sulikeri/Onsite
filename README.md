# Onsite

Practise real software-engineering interview loops, out loud, on your own machine.

- **General loops:** pick any rounds (resume screen, project deep dive, fundamentals, DSA, code comprehension, LLD, system design, behavioral, hiring manager) and a company style.
- **Company-specific loops:** Onsite researches recent candidate reports for a company and role, then runs that company's real loop with its reported questions and interviewer style.
- **Spoken calls:** local speech via VoiceStudio. Nothing leaves your machine except the model calls to the AI you connect.
- **Honest feedback:** rubric scores with evidence, "fix this first", stronger answers for key moments, and a hiring-committee verdict after a full loop.
- **Any of these AIs:** Claude Code (Claude Pro or Max login), or your own API key for Claude, OpenAI, Gemini or Groq (Gemini and Groq have free tiers).

## Download
Get the file for your system from the **[latest release](https://github.com/Prateesh-Sulikeri/Onsite/releases/latest)**:

| System | File |
|---|---|
| Windows 10/11 | `Onsite_x.y.z_x64-setup.exe` |
| macOS 11+ (Apple Silicon and Intel) | `Onsite_x.y.z_universal.dmg` |
| Linux x86-64, any distro | `Onsite_x.y.z_amd64.AppImage` |
| Debian / Ubuntu | `Onsite_x.y.z_amd64.deb` |

Windows is what Onsite is built and tested on. The macOS and Linux builds are new and less tested; spoken rounds
there need VoiceStudio for that system, and typed rounds work without it.

## Install (for friends)
1. Install [VoiceStudio](https://github.com/debpalash/VoiceStudio) and open it once so it downloads its voice model.
2. Download Onsite for your system from the [latest release](https://github.com/Prateesh-Sulikeri/Onsite/releases/latest) and install it:
   - **Windows:** run the `-setup.exe`. SmartScreen may warn because it isn't signed: **More info → Run anyway**.
   - **macOS:** open the `.dmg` and drag Onsite into Applications. It isn't notarized, so the first time run
     `xattr -dr com.apple.quarantine /Applications/Onsite.app` in Terminal (or right-click → Open). Allow the
     microphone when asked.
   - **Linux:** `chmod +x Onsite_*.AppImage` and run it, or `sudo apt install ./Onsite_*_amd64.deb`.
3. Open Onsite. It starts on **Settings → AI model**. Pick one:
   - **Claude Code** if you have Claude Pro or Max and Claude Code installed. No key.
   - **Claude API key** from console.anthropic.com (pay per use; works without Pro).
   - **OpenAI key** from platform.openai.com (billed apart from ChatGPT Plus).
   - **Gemini key**, free, from aistudio.google.com.
   - **Groq key**, free, from console.groq.com (API Keys; starts with `gsk_`). Very fast. The free tier allows
     8,000 tokens a minute, so in quick back-and-forth the interviewer may pause up to half a minute before replying.
4. Paste the key, press **Save key**, then **Run health check**.

Everything else comes with the installer or is set up on first run: prompts, the question bank, the Razorpay
dossier, interviewer voices (built in VoiceStudio), and the English speech model (downloaded through VoiceStudio).
To **Run** code you need that language installed (Java, g++, Node). Python falls back to VoiceStudio's own copy,
which also builds the Submit tests. Whisper (the accurate transcript) is used when VoiceStudio has a
large-v3 model and a CUDA GPU; otherwise captions come from Parakeet.

Keys are stored only on your machine, in Onsite's local database. Only research jobs get web search;
the interviewer gets no tools.

## Updates
**Settings → Updates → Check for updates** lists every published release and downloads the right file for your
system from GitHub. Your interviews, dossiers and settings stay. Onsite only contacts GitHub when you press Check.
- **Windows:** the installer runs and Onsite closes.
- **macOS:** the new disk image opens; quit Onsite and drag the new one into Applications.
- **Linux AppImage:** the new AppImage replaces the old file and Onsite restarts.
- **Linux .deb:** the package is saved to Downloads for you to install.

## Company-specific loops
Open **Dossiers**, name a company, role and level, and start research. It takes a few minutes on an API key
(10–40 on Claude Code). Then pick **Company-specific** on the interview screen. Dossiers can be exported to
Downloads and imported on another machine.

## Building
- Rust (stable) and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your system
  (Linux also needs `libasound2-dev`).
- `cargo install tauri-cli --version "^2" --locked`, then in `src-tauri/`:
  - Windows: `cargo tauri build` → `target/release/bundle/nsis/`
  - macOS: `cargo tauri build --bundles dmg` (add `--target universal-apple-darwin` for both chips)
  - Linux: `cargo tauri build --bundles appimage,deb`

## Releasing
1. Set the same version in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` (e.g. `0.2.0`), commit.
2. `git tag v0.2.0 && git push origin v0.2.0`.
3. GitHub Actions (`.github/workflows/release.yml`) checks the tag against the app version, creates a draft
   release, runs the tests and builds on Windows, macOS and Linux in parallel, uploads every file, and publishes
   the release only when all three have succeeded. Stable tags (`v0.2.0`) must be on `main`. Experimental tags (`v0.2.0-exp.1`, pre-releases) must be on `m1`.
   **Actions → Release → Run workflow** does a dry run: the same builds, kept as workflow artifacts, no release.

Every push to `m1` also publishes an experimental pre-release by itself (`.github/workflows/experimental.yml`, newest 3 kept).

The in-app Updates screen reads the same releases: it skips drafts, needs a `vX.Y.Z` tag, and picks the file for
its system (`*-setup.exe`, `*universal.dmg`, `*amd64.AppImage` when running as an AppImage, else `*amd64.deb`).
It offers the newest stable release that is newer than the running app. Switch the channel to Experimental in
Settings → Updates to get experimental builds first; they may break.

## Privacy
Onsite sends data only to the AI you connect, and to GitHub when you check for updates. No accounts, analytics or
tracking. Details: [PRIVACY.md](PRIVACY.md).

## Code signing policy
Free code signing provided by [SignPath.io](https://about.signpath.io/), certificate by
[SignPath Foundation](https://signpath.org/) (applied for; until it's approved, installers are unsigned). Who can commit
and approve, and what gets signed: [CODE_SIGNING.md](CODE_SIGNING.md).

## Contributing
Bug reports, fixes and new questions are welcome. See [CONTRIBUTING.md](CONTRIBUTING.md). Report security
problems privately: [SECURITY.md](SECURITY.md). `CLAUDE.md` has the spec and how the app is put together.

## License
[MIT](LICENSE). Parts of the resume check are ported from Resume-Matcher (Apache-2.0) and ats-screener (MIT);
see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

## Status
Under construction. See `CLAUDE.md` for the spec and milestones.
