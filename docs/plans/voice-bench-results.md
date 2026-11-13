# Voice bench results

Numbers from `examples/voice_bench` (plan: `voice-pipeline-rebuild-impl.md`). Owner's laptop: Ryzen 7 250, 16 logical CPUs,
release build, models in `D:\cargo-target\voice-bench\models`. Raw JSON per run in `D:\cargo-target\voice-bench\out`.

## Models for the bake-off (downloaded 2026-11-13, not committed)

| Model | URL | Bytes | SHA-256 | Unpacked | Licence (as stated at the source) |
|---|---|---|---|---|---|
| Qwen3-ASR 0.6B int8 | github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/sherpa-onnx-qwen3-asr-0.6B-int8-2026-03-25.tar.bz2 | 878,702,423 | 393f8a14e2f5fb96746aaab342997a40641001fbd5bf9592a080a8329178ee96 | 955 MB | Not stated in the archive. Its README points to the ONNX export (github.com/Wasser1462/Qwen3-ASR-onnx) and to QwenLM/Qwen3-ASR; check both before shipping |
| Orukeet v0.1.0 int8 | huggingface.co/oruk/orukeet/resolve/55a984d46f68323301837194ce647c702f55facc/onnx/sherpa-onnx-orukeet-v0.1.0-int8.tar.bz2 | 486,807,585 | f9191f30178cc9122ce2f023bf9fefafc822028307b0efa4caff645ba3fe8d0a (matches the model card) | 641 MB | "Code is MIT (LICENSE); weights and fitted kernels are CC BY-SA 4.0 (LICENSE-WEIGHTS), retaining NVIDIA's foundation attribution." |

- Qwen3 `hotwords`: no doc comment in `sherpa-onnx-sys-2.0.1`, so the bench passes a comma-separated list.
- Both load with crate 2.0.1 (`silence` ran for Orukeet; Qwen3 not run yet).

## Scoring note

Parakeet writes numbers as digits ("10,000", "99.9%", "P99"). From Task 5 on, the bench runs `to_speech` on both the
reference and the transcript before WER, so number formatting is not counted as a hearing error. The raw baseline
below was scored without that and is inflated by it; the Task 5 gate re-runs `--raw` with the new scoring.

## Baseline (today's build, before Task 4)

TTS round trip, 55 lines (30 tech + 25 conversation), 8 steps, speed 1.0, text as written (`--raw`):

| Run | Mean WER | Lines with a missing word | RTF p50 / p90 / max | Min s/word | Words/s |
|---|---|---|---|---|---|
| `tts --raw` (sid 5, 4 threads) | 25.4% | 31/55 | 0.33 / 0.43 / 0.53 | 0.215 | 2.58 |
| `tts --raw --sid 0` | 27.9% | 32/55 | 0.28 / 0.36 / 0.43 | 0.215 | 2.58 |
| `tts --raw --threads 2` (no affinity) | 25.0% | 30/55 | 0.26 / 0.33 / 0.36 | 0.215 | 2.58 |

What the transcripts show (sid 5): "1.5 ms" heard as "15Ns" or "one five ms" (the decimal is lost), "O(n^2)" as
"Owen and two", "A -> B -> C" as "A", "x && y" as nothing, "getUserById" as "Jit Userbide", URLs read out, "Redis" as
"Rita's". These are the S1 faults the normalizer targets.

STT, synthetic speech (12 `stt_terms.tsv` sentences x sid 0 and 5), Parakeet, raw-digit scoring:
mean WER 12.6%, term recall 46/56 (82%), decode p50 0.21 s, p90 0.25 s, RTF p50 0.05.

Silence (5 s zeros + 5 s noise at RMS 0.005): Parakeet PASS, Orukeet PASS (both returned empty text).

Sweep (`sweep --save-wav`, raw text): running at the time of writing; grid and `listen/sid<N>.wav` go here.

## Measured facts

See `docs/engineering/VOICE.md` "Measured facts": no input splitting by sherpa, no spin switch in 2.0.1, 0% idle CPU.
