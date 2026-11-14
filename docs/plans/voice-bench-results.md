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

Sweep (raw text, before the normalizer): see the Task 7 sweep below.

## Task 7 sweep (2026-11-14, normalizer on, 55 lines, 8 steps, 4 threads)

Lines (of 55) with a missing word / mean WER, per speed. "Missing" is the full round trip: a word the voice dropped
or one Parakeet misheard, so zero is not reachable for any voice.

| sid | 0.9 | 1.0 | 1.1 | 1.2 | 1.3 | words/s at 1.0 |
|---|---|---|---|---|---|---|
| 0 (F) | 9 / 3.62% | 10 / 4.27% | 14 / 7.09% | 16 / 7.25% | 26 / 16.73% | 2.55 |
| 1 (F) | 10 / 4.75% | 11 / 5.12% | 18 / 9.00% | 19 / 10.16% | 23 / 13.59% | 2.70 |
| 2 (F) | 9 / 4.34% | 13 / 6.23% | 15 / 6.15% | 23 / 13.23% | 29 / 19.00% | 2.47 |
| 3 (F) | 10 / 6.24% | 11 / 4.61% | 14 / 6.71% | 23 / 8.91% | 26 / 13.78% | 2.88 |
| 4 (F) | 15 / 6.96% | 13 / 6.55% | 17 / 7.86% | 20 / 11.94% | 31 / 19.22% | 2.63 |
| 5 (M) | 10 / 4.37% | 12 / 5.21% | 13 / 5.20% | 16 / 7.70% | 20 / 12.50% | 2.55 |
| 6 (M) | 13 / 4.93% | 15 / 7.36% | 14 / 6.56% | 22 / 13.24% | 31 / 18.17% | 2.61 |
| 7 (M) | 11 / 4.27% | 11 / 4.39% | 15 / 6.48% | 22 / 9.44% | 28 / 12.23% | 2.96 |
| 8 (M) | 12 / 4.59% | 11 / 4.61% | 14 / 7.23% | 19 / 8.14% | 25 / 12.12% | 2.71 |
| 9 (M) | 12 / 5.18% | 13 / 5.29% | 18 / 9.98% | 19 / 10.57% | 29 / 15.08% | 2.56 |

The plan's rule (zero missing-word lines at 1.0 and 1.1) can't be met by any voice, so the picks use the same order
of tests on the counts: female = fewest missing at 1.0 + 1.1, then lowest WER (sid 0: 10 + 14, tie rule keeps sid 0);
male = words/s at 1.0 closest to the female pick's 2.55 (sid 5: 2.55, also the fewest missing among 5-9).
Both are the current voices. Speed 1.3 is where every voice breaks down (missing 20-31, WER 12-19%); 1.2 holds
(sid 0: 16 / 7.3%, sid 5: 16 / 7.7%). Proposed `SPEED_MAX` = 1.2, pending the owner's listen.
Raw text before the normalizer, same grid: 31-36 missing and ~24-29% WER at every speed.

## Measured facts

See `docs/engineering/VOICE.md` "Measured facts": no input splitting by sherpa, no spin switch in 2.0.1, 0% idle CPU.
