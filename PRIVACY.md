# Privacy policy

Plain terms for what Onsite does with your data. The same text is in the app under Settings → Privacy.

Onsite sends data only to services you choose: the AI provider you connect, and GitHub when you press
Check for updates. It has no accounts, analytics, telemetry, ads or tracking, and sends nothing to the developer.

## What stays on this computer
- Your resume, job descriptions, transcripts, code, whiteboards, feedback, dossiers and settings, in Onsite's
  database in your app data folder.
- Your voice. VoiceStudio and Whisper run locally; audio is never uploaded by Onsite.
- API keys, stored in the same local database. They are not encrypted, so anyone with access to your account on
  this computer can read them. Remove a key in Settings → AI model when you stop using it.
- The log file, which holds errors and timings, not transcripts or keys.

## What leaves this computer
- **To the AI you connect** (Claude Code, the Claude API, OpenAI, Gemini or Groq): the interview prompt, your resume
  and job description, what you say or type, and your code and whiteboard when you share them. Feedback, the resume
  check and research send the same kind of data. That provider's own privacy policy applies:
  [Anthropic](https://www.anthropic.com/legal/privacy), [OpenAI](https://openai.com/policies/privacy-policy/),
  [Google Gemini API](https://ai.google.dev/gemini-api/terms), [Groq](https://groq.com/privacy-policy/).
- **Research jobs** ask that provider to search the web. The company, role and job description you enter are part of
  the request.
- **Update checks** ask GitHub for the release lists of Onsite and VoiceStudio, only when you press Check for updates.
  Installing an update downloads the file from GitHub. GitHub sees your IP address, as with any website:
  [GitHub privacy statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement).

Onsite talks to VoiceStudio only on this computer (`127.0.0.1`). VoiceStudio is a separate app with its own terms.

## What Onsite never does
- No accounts, analytics, telemetry, ads or tracking.
- No data is sent to the Onsite developer.
- The interviewer gets no tools: it can't browse, run commands or read your files.
- Model replies and dossiers are shown and saved as text, never run as code.

## Deleting your data
- Delete single rounds from Feedback, dossiers from Dossiers, and old rounds automatically with the limits in Storage.
- Uninstalling Onsite leaves your data folder in place. Delete it from Settings → Storage → Open data folder to remove
  everything.

## Changes
Changes to this policy are made in this file, so its git history shows every change.
