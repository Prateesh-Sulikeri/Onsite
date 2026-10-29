# Onsite

Practise real software-engineering interview loops, out loud, on your own machine.

- **General loops:** pick any rounds (resume screen, project deep dive, fundamentals, DSA, code comprehension, LLD, system design, behavioral, hiring manager) and a company style.
- **Company-specific loops:** Onsite researches recent candidate reports for a company and role, then runs that company's real loop with its reported questions and interviewer style.
- **Spoken calls:** local speech via VoiceStudio. Nothing leaves your machine except the model calls to the AI you connect.
- **Honest feedback:** rubric scores with evidence, "fix this first", stronger answers for key moments, and a hiring-committee verdict after a full loop.
- **Any of these AIs:** Claude Code (Claude Pro or Max login), or your own API key for Claude, OpenAI, Gemini or Groq (Gemini and Groq have free tiers).

**[Download the latest Windows installer](https://github.com/Prateesh-Sulikeri/Onsite/releases/latest)**

## Install (for friends)
1. Install [VoiceStudio](https://github.com/debpalash/VoiceStudio) and open it once so it downloads its voice model.
2. Download `Onsite_x.y.z_x64-setup.exe` from the [latest release](https://github.com/Prateesh-Sulikeri/Onsite/releases/latest)
   and run it. Windows SmartScreen may warn because the installer isn't signed: **More info → Run anyway**.
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
**Settings → Updates → Check for updates** lists every published release. Pick one to download its installer
from GitHub; Onsite closes, the installer runs, and your interviews, dossiers and settings stay. Onsite only
contacts GitHub when you press Check.

## Company-specific loops
Open **Dossiers**, name a company, role and level, and start research. It takes a few minutes on an API key
(10–40 on Claude Code). Then pick **Company-specific** on the interview screen. Dossiers can be exported to
Downloads and imported on another machine.

## Building
- Windows 10/11, Rust (stable), and the [Tauri prerequisites for Windows](https://tauri.app/start/prerequisites/).
- `cargo install tauri-cli --version "^2" --locked`, then `cargo tauri build` in `src-tauri/`. The installer lands in
  `src-tauri/target/release/bundle/nsis/`.

## Releasing
1. Set the same version in `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml` (e.g. `0.2.0`), commit.
2. `git tag v0.2.0 && git push origin v0.2.0`.
3. GitHub Actions (`.github/workflows/release.yml`) runs the tests, builds the NSIS installer and publishes the
   release with `Onsite_0.2.0_x64-setup.exe` attached. It stops if the tag and the app version differ.
   Tags with a hyphen (`v0.3.0-rc1`) become pre-releases.

The in-app Updates screen reads the same releases: it skips drafts, needs a `vX.Y.Z` tag and a `*-setup.exe`
asset, and offers the newest non-pre-release that is newer than the running app.

## Status
Under construction. See `CLAUDE.md` for the spec and milestones.
