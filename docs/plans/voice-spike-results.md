# Voice spike results (Phase 0)

Plan: `voice-without-voicestudio.md`. Code: `src-tauri/examples/voice_spike.rs`. Started 2026-10-30.

## Setup

| | |
|---|---|
| Machine | AMD Ryzen 7 250 (8 cores / 16 threads), Radeon 780M iGPU, RTX 5050 laptop GPU, 23.3 GB RAM, Windows 11 |
| Library | `sherpa-onnx` crate 2.0.1 (Apache-2.0), static link, prebuilt libs fetched from github.com by its build script. The spike exe is 22 MB |
| Provider | CPU for every number below, 4 threads unless noted |
| Models | `src-tauri/target/voice-spike/models` (gitignored), from the k2-fsa/sherpa-onnx GitHub releases `tts-models`, `asr-models`, `vocoder-models` |
| Persona clips | `target/voice-spike/personas/<persona_id>.wav`, copied from `%APPDATA%/OmniVoice/voices` (ids mapped via VoiceStudio `/profiles`). Transcript = `vs.rs::PERSONA_SAMPLE` |

Rerun: `cargo build --release --example voice_spike`, then `target/release/examples/voice_spike.exe tts pocket --personas all --runs 2`
(or `stt parakeet`, `record <id>`). Every run writes WAVs and a JSON of the numbers to `target/voice-spike/out/`.

## Status

| Area | Status |
|---|---|
| 0.1 Test set | TTS lines written (`tests/fixtures/voice/tts_lines.txt`, 25). Answer prompts in `answers.json`; **recordings by the owner pending** (`voice_spike record <id>`; WAVs gitignored) |
| 0.2 Baseline | Not run yet |
| TTS speed, CPU | **ZipVoice fails, Pocket TTS passes** (below) |
| TTS identity, early check | Pocket renders ready for the owner to listen to: `target/voice-spike/out/pocket/` |
| STT | Code ready; waits for the recordings |
| Build on macOS/Linux | Not run yet (CI dry run) |

## TTS speed, CPU

"First audio" = time until the first sentence's samples come back (sherpa-onnx calls back per sentence; it does not
stream inside a sentence). Gate: ≤ 1.5 s for a short sentence, real-time factor (RTF) < 1.

| Engine | Load | First audio, short lines p50 / p90 | Long lines p50 / p90 | RTF p50 / max | Gate |
|---|---|---|---|---|---|
| ZipVoice distill int8 (zh-en Emilia) + Vocos 24 kHz, 4 threads | 3.0 s | 4.6 s / 5.0 s | 5.3 s / 6.7 s | 2.61 / 10.3 | **Fail** |
| same, 8 threads | 2.9 s | 4.4 s / 5.0 s | 5.4 s / 7.0 s | 2.61 / 9.3 | **Fail** |
| Pocket TTS int8 (2026-01-26), 4 threads | 1.2 s | 0.89 s / 1.04 s | 1.30 s / 2.50 s | 0.45 / 0.79 | **Pass** |

Notes:
- ZipVoice is a flow-matching model that runs over reference + target each call; with a 10.5 s reference it is ~4×
  too slow on this CPU and more threads don't help. Dropped for the CPU baseline. (It may still be usable on a GPU;
  not tested.)
- ZipVoice also needs a word lexicon (`lexicon-en-us.txt`); tech words like "Redis", "PostgreSQL", "idempotency" are
  not in it. Pocket TTS reads raw text, no lexicon.
- Pocket long sentences: one 20-word sentence takes ~2.5 s before it can play. The app already speaks sentence by
  sentence; splitting long sentences at commas would keep first audio near 1 s.
- Pocket with `seed` fixed (42) gives byte-identical audio on both runs (early check, 4 personas × 25 lines × 2).
  Early-check totals: load 0.86 s; short lines p50 0.79 s, p90 1.69 s; RTF p50 0.45.
- **Runaway on a one-word line.** "Okay." gave 17 s (Neha) and 33 s (Ethan) of audio instead of ~0.5 s. The
  app must cap output length per sentence (e.g. ~0.6 s per word + 1 s) or join very short lines to the next one.
- Listening folder for the owner: `target/voice-spike/listen/` (current clip + 4 new lines per persona).

## Licensing record (so far)

| Asset | Source + revision | Licence | Notes |
|---|---|---|---|
| sherpa-onnx | crate 2.0.1 | Apache-2.0 | notice needed |
| Pocket TTS code | github.com/kyutai-labs/pocket-tts | MIT | |
| Pocket TTS weights | HF `kyutai/pocket-tts` @ db04f0e (gated, auto-approve); ONNX export HF `KevinAHM/pocket-tts-onnx` @ 58a6d00; sherpa-onnx `sherpa-onnx-pocket-tts-int8-2026-01-26` | CC-BY-4.0 | Gate terms forbid "voice impersonation or cloning without explicit and lawful consent". Our clips are synthetic voices, not real people. The sherpa-onnx tarball README says "It is for non-commercial", which conflicts with CC-BY-4.0 on both HF repos; Onsite is free and MIT, but ask/confirm before relying on it |
| ZipVoice code | github.com/k2-fsa/ZipVoice | Apache-2.0 | |
| ZipVoice weights | HF `k2-fsa/ZipVoice` @ 889dd60 | none stated | Trained on Emilia (CC-BY-NC-4.0). Moot while it fails the speed gate |
| Parakeet TDT 0.6B v2 | HF `nvidia/parakeet-tdt-0.6b-v2` @ ae9ad07 | CC-BY-4.0 | attribution |
| Nemotron speech streaming 0.6B (caption candidate) | HF `nvidia/nemotron-speech-streaming-en-0.6b` @ ebe59e5 | NVIDIA Open Model License | read the terms before choosing it |
| NeMo streaming FastConformer transducer 480 ms (caption candidate) | sherpa-onnx `asr-models` | to check | |
| Persona clips ×10 | made with OmniVoice | open (plan) | |

## Model sizes (unpacked, as downloaded)

| Model | Size |
|---|---|
| Pocket TTS int8 | 194 MB |
| ZipVoice distill int8 + Vocos + English lexicon | 145 + 58 MB |
| Parakeet TDT 0.6B v2 int8 | 631 MB (incl. test WAVs) |
| NeMo streaming FastConformer transducer 480 ms int8 | 132 MB |
| Nemotron speech streaming 0.6B 560 ms int8 | 633 MB |

Pocket + Parakeet + a small caption model ≈ 1 GB, inside the plan's 1–2 GB estimate.

## TTS identity: tuning for steady voices

Goal: same speaker and accent on every line (Pocket TTS, CPU). Owner heard Arjun and Neha drift; Ethan and Priya held.

**Score.** `voice_spike score <render dir> --ref <clip.wav> [--model wespeaker|titanet]`. Speaker-embedding cosine of every line to a reference clip (wespeaker ResNet34 VoxCeleb, also NeMo TitaNet-small), plus mean, min, and pairwise mean between lines. Models in `target/voice-spike/models/spk/`. It measures the *speaker*; it only partly sees accent. Sanity: separates different voices (Arjun render vs Neha clip 0.49, vs Ethan clip 0.63, own clip 0.70) and flags Neha as worst (0.53 mean, pairwise 0.57). It did not flag Arjun (0.70, pairwise 0.79), though the owner heard him drift: use the "own vs Ethan/Priya" margin below as the accent proxy.

**Sweep** (10 lines: 3,4,7,10,13,16,19,21,22,24; 5 Indian personas; wespeaker mean / pairwise; ref = 10.5 s clip unless noted; int8, temp 0.7, seed 42, default steps):

| setting | arjun | neha | karthik | vikram | sameer | first audio avg |
|---|---|---|---|---|---|---|
| base | .72/.83 | .56/.62 | .77/.79 | .80/.81 | .80/.82 | 0.8-1.5 s |
| full precision | .71/.83 | .57/.59 | .77/.78 | .79/.86 | .82/.83 | 1.6-2.9 s (rtf .6-.9) |
| temp 0.3 | .71/.84 | .35/.48 | .70/.75 | .80/.83 | .80/.81 | same as base |
| temp 0.5 | .72/.85 | .46/.55 | .73/.77 | .80/.84 | .80/.82 | same |
| fp + temp 0.3 | .71/.83 | .40/.55 | .69/.81 | .72/.73 | .81/.85 | slow, rtf up to 1.8 |
| seed 7 | .71/.83 | .47/.53 | .75/.80 | .81/.81 | .80/.82 | same |
| seed 123 | .73/.84 | .58/.68 | .79/.81 | .77/.81 | .80/.84 | same |
| num_steps 4 / 2 | .73/.85, .74/.87 | .59/.63, .57/.61 | .75/.79, .76/.79 | .80/.82, .81/.82 | .80/.83, .81/.83 | same |
| **25 s ref, seed 123** | .75/.83 | **.79/.82** | .84/.88 | .78/.79 | .68/.82 | 0.8-0.9 s |
| **25 s ref, seed 42** | .72/.85 | **.80/.84** | .83/.87 | .82/.81 | .71/.84 | 0.7-1.1 s |

- Full precision: no gain, 2x slower. Do not use. Lower temperature: no gain, worse for Neha. Seeds and `num_steps`: noise.
- **What worked: a longer reference clip.** About 21 s rendered by VoiceStudio in the same voice, text = the intro line plus interview lines (`ref_text` is not used by Pocket). Set `max_reference_audio_len` to 25. Neha's pairwise jumped .62 to .84, min .45 to .73. Cost: none measurable.
- Accent proxy, own long clip vs Ethan / Priya long clips (mean cosine of 10 renders), base then 25 s ref: Arjun .75 vs .66/.51, then .76 vs .66/.50 (weak margin); Neha .58 vs .54/.63 (closer to Priya than herself!), then .83 vs .47/.66; Karthik .78 vs .55/.48, then .84 vs .56/.51 (best); Vikram .83 vs .72, then .85 vs .70; Sameer .83 vs .70, then .77 vs .75 (drifts toward Ethan).

**Pick** (25 s ref, int8, temp 0.7, seed 42, all 25 lines, own long clip as reference): Indian Karthik (.83 mean, .76 min, .85 pairwise) and Neha (.82, .73, .81); Western Ethan (.82, .75, .81) and Priya (.80, .71, .81). Ethan short-ref min was .23 (one bad line) and is .75 now. Indian voices now score the same as Western. Runner-up Indian: Vikram. Arjun and Sameer are the weakest on the accent proxy.
Speed (4 threads): short lines (<= 8 words) first audio p50 0.88 s, p90 1.3 s; long lines p50 1.4 s, p90 2.8 s; rtf p50 0.51, p90 0.67, max 1.07 (one-word lines). "Okay." stayed short (0.3-1.2 s audio); the run-on bug did not show in this run. Neha's first audio on #4 was 0.95 s.
Listening: `target/voice-spike/listen2/` (A = current VoiceStudio clip, B-F = new). Ears decide the accent; the score cannot.

## Decision (owner, 2026-10-30): American/British voices only

After listening to `listen2`, the owner judged the Indian voices (Karthik, Neha) still not Indian enough and dropped
them. Interviewer voices are American/British only; Indian accents are out of scope for now. Ethan (American) and
Priya (British) pass with the 21 s reference clips. Owner chose **Ethan and Priya only**; Olivia, Marcus, Daniel not tested
(not needed). Phase 1 needs a voice for every panel persona, so personas with Indian names will use one of these voices.

## Phase 1 build check (2026-10-30)

`cargo run --release --example builtin_check -- [--download]` (Ryzen 7 250, CPU, 4 threads): models downloaded and verified through
`voice/models.rs` (Pocket 98.3 MB in 126 s, Parakeet 482.5 MB in 557 s on this link; SHA-256 pins match). Pocket loads 1.4 s, Parakeet 3.4 s.
Speech (24 kHz): first audio 0.26-2.4 s, rtf 0.44-0.79. Parakeet decodes a 4-6 s segment in 0.3-0.5 s; a caption re-decode
takes mean 0.28 s, max 0.44 s over 5.6 s of audio. Details in `docs/engineering/VOICE.md`.
