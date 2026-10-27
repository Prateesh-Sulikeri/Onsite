# Onsite

Practise real software-engineering interview loops, out loud, on your own machine.

- **General loops:** pick any rounds (resume screen, project deep dive, fundamentals, DSA, code comprehension, LLD, system design, behavioral, hiring manager) and a company style.
- **Company-specific loops:** Onsite researches recent candidate reports for a company and role, then runs that company's real loop with its reported questions and interviewer style.
- **Spoken calls:** local speech via VoiceStudio. Nothing leaves your machine except the model calls to the AI you connect.
- **Honest feedback:** rubric scores with evidence, "fix this first", stronger answers for key moments, and a hiring-committee verdict after a full loop.
- **Any of these AIs:** Claude Code (Claude Pro or Max login), or your own API key for Claude, OpenAI or Gemini (Gemini has a free tier).

## Install (for friends)
1. Install [VoiceStudio](https://github.com/debpalash/VoiceStudio) and open it once so it downloads its voice model.
2. Run the Onsite installer (`Onsite_x.y.z_x64-setup.exe` from the GitHub release).
3. Open Onsite. It starts on **Settings → AI model**. Pick one:
   - **Claude Code** if you have Claude Pro or Max and Claude Code installed. No key.
   - **Claude API key** from console.anthropic.com (pay per use; works without Pro).
   - **OpenAI key** from platform.openai.com (billed apart from ChatGPT Plus).
   - **Gemini key**, free, from aistudio.google.com.
4. Paste the key, press **Save key**, then **Run health check**.

Everything else comes with the installer or is set up on first run: prompts, the question bank, the Razorpay
dossier, interviewer voices (built in VoiceStudio), and the English speech model (downloaded through VoiceStudio).
To **Run** code you need that language installed (Java, g++, Node). Python falls back to VoiceStudio's own copy,
which also builds the Submit tests. Whisper (the accurate transcript) is used when VoiceStudio has a
large-v3 model and a CUDA GPU; otherwise captions come from Parakeet.

Keys are stored only on your machine, in Onsite's local database. Only research jobs get web search;
the interviewer gets no tools.

## Company-specific loops
Open **Dossiers**, name a company, role and level, and start research. It takes a few minutes on an API key
(10–40 on Claude Code). Then pick **Company-specific** on the interview screen. Dossiers can be exported to
Downloads and imported on another machine.

## Building
- Windows 10/11, Rust (stable), and the [Tauri prerequisites for Windows](https://tauri.app/start/prerequisites/).
- `cargo install tauri-cli --version "^2" --locked`, then `cargo tauri build` in `src-tauri/`. The installer lands in
  `src-tauri/target/release/bundle/nsis/`.
- Or push a tag like `v0.2.0`: GitHub Actions builds the installer and attaches it to a draft release.

## Status
Under construction. See `CLAUDE.md` for the spec and milestones.
