# Voice Pipeline Rebuild Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the interviewer's voice clean and the candidate's speech correctly heard on every supported laptop,
by rebuilding the code around the speech models and testing better recognizers side by side.

**Architecture:** Interviewer text goes splitter -> speech-text normalizer -> guarded generator -> sentence-level
scheduler -> lock-free ring -> speaker. Mic audio goes gain + Silero VAD -> segments -> recognizer, with a per-round
vocabulary feeding the transcript repair and the interviewer prompt. A bench (`voice_bench`) measures every step on a
fixed set and picks the recognizer.

**Tech Stack:** Rust (Tauri 2 core), `sherpa-onnx` 2.0.1 (Supertonic 3, Parakeet TDT v2, Silero VAD, Qwen3-ASR),
`cpal` 0.18, `regex` 1 (already a dependency), `rtrb` (new, lock-free ring buffer).

**Spec:** `docs/plans/voice-pipeline-rebuild.md`. Read it first. Cause ids (S1-S9, L1-L5) below refer to its tables.

## Global Constraints

- Run every command from `src-tauri/`. Build folder: `--target-dir D:\cargo-target\onsite-test` on every `cargo`
  command (the running app locks the dev folder). Never create another target dir.
- Before each commit: `cargo clippy --target-dir D:\cargo-target\onsite-test -- -D warnings` and
  `cargo test --target-dir D:\cargo-target\onsite-test`. Both must pass.
- No em or en dashes anywhere (code, comments, docs, prompts, UI copy). If one is functionally needed, write it as a
  `\u` escape.
- User-facing text: short, plain sentences.
- Never log speech text, transcripts, names or vocabulary terms. Log counts and lengths only.
- Speech stays local. No network call is added by this plan. Model downloads for the bench are manual.
- No provider SDKs. New crate allowed by this plan: `rtrb` only.
- The screen and the saved transcript keep the interviewer's original text. Only the text sent to the voice changes.
- Work on a new branch `voice/rebuild`. Precondition (owner): the uncommitted work on `voice/remove-voicestudio` is
  committed first. If `git status` shows modified files when you start, stop and ask.
- One commit per task, message `Voice: <what>`, ending with the attribution lines the session provides.
- After code changes run `graphify update .` from the repo root (skip if `graphify-out/graph.json` is missing).
- Update `docs/engineering/VOICE.md` in the same commit as any code that makes it wrong.

## Review Focus

Inputs the spec implies but its phases don't name. Each has a test in the task shown.

1. A sentence that is empty after normalization (only emoji, markdown or brackets): nothing is spoken, no TTS call,
   no error. Test in Task 4.
2. A huge token (a 300-character base64 string or hash): spoken as "a long value", never read out. Test in Task 4.
3. Names with accents or non-Latin letters ("José", "Zoë"): passed through untouched by the normalizer and by the
   vocabulary builder. Tests in Tasks 4 and 13.
4. A resume with thousands of capitalized words: the round vocabulary is capped at 150 terms and built in under
   50 ms. Test in Task 13.
5. Keyboard typing and room noise with no speech: the answer does not start. Test in Task 15.

## File Structure

| File | Responsibility |
|---|---|
| `src/voice/score.rs` (new) | Pure scoring for the bench: WER, missing words, term recall |
| `examples/voice_bench/main.rs`, `engines.rs` (new) | Bench CLI: `tts`, `stt`, `sweep`; candidate recognizers live here until one wins |
| `tests/fixtures/voice/tts_tech_lines.tsv`, `stt_terms.tsv` (new) | Fixed sets |
| `src/voice/health.rs` (new) | Per-round counters and the end-of-round log line |
| `src/voice/speech_text.rs` (new) | Written text -> spoken text. Pure |
| `prompts/pronounce.json` (new) | Pronunciation lexicon (data) |
| `src/voice/tier.rs` (new) | Machine tier from measured speed, and the settings each tier uses |
| `src/voice/vocab.rs` (new) | Per-round vocabulary from resume, JD, dossier, names |
| `prompts/stt_aliases.json` (new) | Known mishearings (data) |
| `prompts/models/silero_vad.onnx` (new, bundled) | VAD model |
| `src/voice/builtin.rs` | `Synth`, guarded generation, edges, pieces by tier |
| `src/voice/audio.rs` | Ring buffer player, resampler, `Agc`, `SpeechGate` |
| `src/voice/engine.rs`, `mod.rs`, `stt.rs` | Wiring: lexicon, tier, scheduler, vocabulary |
| `src/brain/splitter.rs` | Exposes the one sentence-boundary rule |
| `src/interview/mod.rs`, `live.rs`, `report.rs`, `prompts/interviewer.md`, `prompts/report.md` | Vocabulary into prompts and into the round's voice settings |

Order: Tasks 1-3 (bench), then the recognizer bake-off (Task 3b) as soon as recordings exist, then 4-18 in order.

---

# Part 0: bench

### Task 1: scoring functions

**Files:**
- Create: `src-tauri/src/voice/score.rs`
- Modify: `src-tauri/src/voice/mod.rs` (add `pub mod score;`)

**Interfaces:**
- Produces:
  - `pub fn words(text: &str) -> Vec<String>`: lowercase; split on whitespace; strip every char that is not
    alphanumeric from each token; drop empty tokens. Hyphens split words ("read-only" -> "read", "only").
  - `pub struct Wer { pub edits: usize, pub ref_words: usize, pub missing: Vec<String> }` with
    `pub fn rate(&self) -> f32` (`edits / ref_words`, 0.0 when `ref_words == 0`).
  - `pub fn wer(reference: &str, hyp: &str) -> Wer`: word-level Levenshtein over `words()`. `missing` = reference
    words deleted or substituted in the best alignment, in order.
  - `pub fn term_recall(hyp: &str, terms: &[String]) -> (usize, usize)`: (terms found, terms total). A term is found
    when its `words()` joined with no separator occurs in the hyp's `words()` joined with single spaces, comparing
    with spaces removed on both sides inside a sliding window of the term's word count +1. Case-insensitive.

- [ ] **Step 1: Write the failing tests** (in `score.rs`, `#[cfg(test)] mod tests`)

```rust
#[test]
fn words_strip_punctuation_and_case() {
    assert_eq!(words("Hello, World! It's read-only."), ["hello", "world", "its", "read", "only"]);
    assert!(words("  ... ").is_empty());
}

#[test]
fn wer_counts_edits_and_missing_words() {
    let w = wer("the cache is in Redis today", "the cash is in today");
    assert_eq!((w.edits, w.ref_words), (2, 6));
    assert_eq!(w.missing, ["cache", "redis"]);
    assert!((w.rate() - 2.0 / 6.0).abs() < 1e-6);
    assert_eq!(wer("", "anything").rate(), 0.0);
    assert_eq!(wer("same words", "Same, words.").edits, 0);
}

#[test]
fn term_recall_ignores_case_and_spacing() {
    let terms: Vec<String> = ["PostgreSQL", "Spring Boot", "Kafka"].map(String::from).to_vec();
    assert_eq!(term_recall("we use postgre sql and spring boot", &terms), (2, 3));
    assert_eq!(term_recall("", &terms), (0, 3));
}
```

- [ ] **Step 2:** Run `cargo test --target-dir D:\cargo-target\onsite-test voice::score`. Expected: compile error, `score` not found.
- [ ] **Step 3:** Implement the three functions and `Wer`. For `missing`, keep the DP table and walk it back.
- [ ] **Step 4:** Run the same command. Expected: 3 passed.
- [ ] **Step 5:** Commit `Voice: scoring functions for the bench`.

### Task 2: fixtures and a raw synthesis entry point

**Files:**
- Create: `src-tauri/tests/fixtures/voice/tts_tech_lines.tsv`, `src-tauri/tests/fixtures/voice/stt_terms.tsv`
- Modify: `src-tauri/src/voice/builtin.rs`
- Modify: `src-tauri/tests/fixtures/voice/answers.json` (add `"terms": []` to every answer; the owner fills them)

**Interfaces:**
- Produces (in `builtin.rs`):
  - `pub struct Synth { pub sid: i32, pub speed: f32, pub steps: i32 }`
  - `pub fn Supertonic::load_with(models_root: &Path, threads: i32) -> Result<Self>`; `load` calls it with `threads()`.
  - `pub fn Supertonic::synth(&self, text: &str, s: &Synth, stop: &AtomicBool) -> Result<Vec<f32>>`: one sherpa call,
    returns all samples at `self.rate`, no cap, no splitting. The sherpa callback returns `!stop.load(SeqCst)`.
    `extra` keeps `lang: "en"`.
  - `pub fn ParakeetStt::load_with(models_root: &Path, threads: i32) -> Result<Self>`; `load` calls it.
  - `pub fn ParakeetStt::transcribe_f32(&self, pcm16k: &[f32]) -> String`.

`tts_tech_lines.tsv`: UTF-8, one line per case, `written<TAB>spoken`, `#` lines are comments. Exact content:

```
# written	spoken   (Task 4 must turn column 1 into column 2 exactly)
It costs 1.5 ms.	It costs one point five milliseconds.
We hit 99.99% uptime.	We hit ninety nine point nine nine percent uptime.
The p99 is 250 ms.	The P ninety nine is two hundred fifty milliseconds.
Upgrade to v2.1 first.	Upgrade to version two point one first.
That is O(n log n) time.	That is O of n log n time.
Can you get it to O(1)?	Can you get it to O of one?
It was O(n^2) before.	It was O of n squared before.
We serve 10k QPS.	We serve ten thousand Q P S.
Each node has 256 GB of RAM.	Each node has two hundred fifty six gigabytes of RAM.
Call getUserById with the order_id.	Call get User By Id with the order id.
What if retries >= 3?	What if retries greater than or equal to three?
So A -> B -> C.	So A to B to C.
Is that read/write or read-only?	Is that read write or read-only?
It takes 10-20 seconds.	It takes ten to twenty seconds.
Use user.id as the key.	Use user dot id as the key.
The 2nd attempt failed.	The second attempt failed.
We store 5M rows.	We store five million rows.
That is 2x faster.	That is two times faster.
See https://example.com/docs for more.	See a link for more.
Timeout is 30s.	Timeout is thirty seconds.
Budget is $5 per user.	Budget is five dollars per user.
Wait... is it idempotent?	Wait, is it idempotent?
Use a HashMap<String, Integer>.	Use a Hash Map String, Integer.
1 ms is fine.	one millisecond is fine.
Set TTL = 60.	Set TTL equals sixty.
x && y must hold.	x and y must hold.
It is ~3 GB.	It is about three gigabytes.
About 1,000,000 users.	About one million users.
Okay.	Okay.
Tell me about a time you disagreed with your manager.	Tell me about a time you disagreed with your manager.
```

`stt_terms.tsv`: `sentence<TAB>comma-separated terms`. Exact content:

```
We keep the session in Redis and the orders in PostgreSQL.	Redis,PostgreSQL
The Kafka consumer commits the offset after the write.	Kafka
I used Spring Boot with Hibernate on the JVM.	Spring Boot,Hibernate,JVM
The service talks gRPC inside and GraphQL at the edge.	gRPC,GraphQL
We made the payment API idempotent with an idempotency key.	idempotent,idempotency
At Razorpay we moved from RabbitMQ to Kafka Streams.	Razorpay,RabbitMQ,Kafka Streams
We deploy on Kubernetes and watch it with Prometheus and Grafana.	Kubernetes,Prometheus,Grafana
The token is a JWT issued through OAuth.	JWT,OAuth
We shard MongoDB and cache hot rows in DynamoDB.	MongoDB,DynamoDB
My name is Prateesh and I worked with Karthik at Juspay.	Prateesh,Karthik,Juspay
We send SWIFT MT103 messages and are moving to ISO 20022.	SWIFT,MT103,ISO 20022
Elasticsearch handles search and Cassandra holds the events.	Elasticsearch,Cassandra
```

- [ ] **Step 1: Write the failing test** (in `builtin.rs` tests)

```rust
#[test]
fn fixtures_parse() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice");
    let tech = std::fs::read_to_string(dir.join("tts_tech_lines.tsv")).unwrap();
    let rows: Vec<_> = tech.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).collect();
    assert_eq!(rows.len(), 30);
    assert!(rows.iter().all(|r| r.split('\t').count() == 2));
    let stt = std::fs::read_to_string(dir.join("stt_terms.tsv")).unwrap();
    assert_eq!(stt.lines().filter(|l| l.split('\t').count() == 2).count(), 12);
}
```

- [ ] **Step 2:** Run `cargo test ... voice::builtin::tests::fixtures_parse`. Expected: FAIL (file not found).
- [ ] **Step 3:** Create both files with real tab characters. Add `Synth`, `load_with`, `synth`, `transcribe_f32`.
  Leave `speak` as it is for now.
- [ ] **Step 4:** Run `cargo test ... voice::builtin`. Expected: all pass.
- [ ] **Step 5:** Commit `Voice: bench fixtures and raw synthesis entry point`.

### Task 3: `voice_bench` example and the baseline

**Files:**
- Create: `src-tauri/examples/voice_bench/main.rs`, `src-tauri/examples/voice_bench/engines.rs`
- Create: `docs/plans/voice-bench-results.md`
- Modify: `src-tauri/Cargo.toml` only if examples are listed explicitly (a folder example with `main.rs` is found automatically)

Copy the argument and model-root handling from `examples/builtin_check.rs`. Models root default:
`D:\cargo-target\voice-bench\models` (layout `<root>/<pin id>/`). Output folder: `D:\cargo-target\voice-bench\out`.

**CLI (exact):**

```
voice_bench tts   [--models <root>] [--set tech|lines|all] [--sid <n>] [--speed <f>] [--steps <n>]
                  [--threads <n>] [--runs <n>] [--raw] [--save-wav]
voice_bench sweep [--models <root>] [--threads <n>]
voice_bench stt   --engine parakeet|orukeet|qwen3 [--models <root>] [--source synthetic|recorded]
                  [--context] [--threads <n>]
voice_bench silence --engine parakeet|orukeet|qwen3 [--models <root>]
```

Defaults: `--set all`, `--sid 5`, `--speed 1.0`, `--steps 8`, `--threads` = `builtin::threads()`, `--runs 1`,
`--source synthetic`.

**Behaviour:**
- `tts`: for each line, text to speak = column 1 (`tech`) or the line (`lines` = `tts_lines.txt`). With `--raw`, or
  until Task 4 exists, speak it as written. Call `Supertonic::synth`, resample to 16 kHz with
  `sherpa_onnx::LinearResampler`, transcribe with Parakeet, score against column 2 (or the line) with `score::wer`.
  Per line record: `wer`, `missing`, `audio_s`, `gen_s`, `rtf = gen_s / audio_s`, `sec_per_word = audio_s / words`.
  Print one row per line and a summary: mean WER, lines with any missing word, RTF p50/p90/max, min `sec_per_word`.
  Write `tts-<unix time>.json` with the same numbers and the CLI arguments.
- `sweep`: runs `tts --set all` for `sid` 0..=9 x `speed` in [0.9, 1.0, 1.1, 1.2, 1.3]. Prints a grid of
  "lines with a missing word" and words per second at speed 1.0. Saves `sweep-<unix time>.json`. With `--save-wav`,
  writes `listen/sid<N>.wav` (line 4 of `tts_lines.txt` at speed 1.0) for the owner to hear.
- `stt --source synthetic`: speak each `stt_terms.tsv` sentence with sid 0 and sid 5 (Task 4 normalizer off), then
  recognise it. `--source recorded`: every answer in `answers.json` with non-empty `text` and a file
  `tests/fixtures/voice/answers/<id>.wav` (16 kHz mono; skip the others and print how many were skipped).
  Per item: WER, term recall (`score::term_recall`), `decode_s`, RTF. Summary: mean WER, total term recall, decode
  p50/p90. `--context` passes all of the item's terms plus `stt::TECH_TERMS` to the engine.
- `silence`: feed 5 s of zeros, then 5 s of noise at RMS 0.005 (deterministic LCG). Print the text returned. Pass =
  empty text for both.
- `--threads 2` together with running the exe as `start /affinity 0x3 ...` is the "old laptop" mode. Print a reminder
  line of that command when `--threads 2` is used.

**`engines.rs` interface:**

```rust
pub trait Recognizer {
    fn name(&self) -> &'static str;
    /// `pcm` is 16 kHz mono. `context` is empty unless --context was given.
    fn decode(&self, pcm: &[f32], context: &[String]) -> String;
}
pub fn open(engine: &str, models_root: &Path, threads: i32) -> Result<Box<dyn Recognizer>, String>;
```

- `parakeet`: `ParakeetStt::load_with` + `transcribe_f32`. Ignores `context`.
- `orukeet`: same `OfflineTransducerModelConfig` + `model_type = "nemo_transducer"` as Parakeet, model folder
  `<root>/orukeet/` (encoder/decoder/joiner/tokens file names as found in the archive). Ignores `context`.
- `qwen3`: `OfflineRecognizerConfig` with `model_config.qwen3_asr = OfflineQwen3ASRModelConfig { conv_frontend,
  encoder, decoder, tokenizer, hotwords, ..Default::default() }`, folder `<root>/qwen3-asr-0.6b/`. The `hotwords`
  field is set at creation, so with `--context` build one recognizer per distinct context (cache by the joined
  string). Format of `hotwords`: check the doc comment for `hotwords` in
  `~/.cargo/registry/src/*/sherpa-onnx-sys-2.0.1/` (search `Qwen3`), and write what you found into the results doc.
  If the format is not documented there, use a comma-separated list.

**Model files (manual, not committed):**
- [ ] **Step 1:** Find the archives:
  `gh release view asr-models -R k2-fsa/sherpa-onnx --json assets -q ".assets[].name" | grep -i -E "qwen3|orukeet"`.
  If Orukeet is not there, use the sherpa-onnx archive linked from `https://huggingface.co/oruk/orukeet`.
  Download with `curl -L`, unpack into the folders above, and record in `voice-bench-results.md`: URL, bytes,
  SHA-256 (`sha256sum`), unpacked size, licence text as stated on the source page.
  If a model can't be found or loaded with crate 2.0.1, write that down with the error and continue without it.
- [ ] **Step 2:** Write `main.rs` and `engines.rs` as specified. No unit tests here: the example is the test tool.
- [ ] **Step 3:** Build: `cargo build --release --example voice_bench --target-dir D:\cargo-target\onsite-test`.
  Expected: builds with no warnings.
- [ ] **Step 4: Baseline.** Run and paste the summaries into `voice-bench-results.md` under "Baseline (today's build)":
  `voice_bench tts --raw`, `voice_bench tts --raw --sid 0`, `voice_bench tts --raw --threads 2`,
  `voice_bench sweep --save-wav`, `voice_bench stt --engine parakeet`, `voice_bench silence --engine parakeet`.
- [ ] **Step 5: Three facts**, written into `docs/engineering/VOICE.md` under a new "Measured facts" heading:
  1. Does sherpa-onnx split Supertonic input itself? Synthesize a two-sentence text with a callback that counts
     calls; more than one call = yes.
  2. Can ONNX Runtime thread spinning be turned off through sherpa-onnx 2.0.1? Search the `sherpa-onnx-sys` bindings
     for `spin` and `session_options`. Record yes (how) or no.
  3. CPU use of the process for 5 s after a synthesis ends (Task Manager or `Get-Counter`). Above ~20% of one core
     while idle means the threads spin.
- [ ] **Step 6:** Commit `Voice: voice_bench example and baseline numbers`.

### Task 3b: recognizer bake-off (run when recordings exist; do not block Tasks 4-12 on it)

**Files:** `docs/plans/voice-bench-results.md` only.

- [ ] **Step 1:** Ask the owner for the recordings (`cargo run --example voice_spike -- record <id>` for each id in
  `answers.json`, then fill `text` and `terms`). Without them, run only `--source synthetic` and mark the result
  "synthetic speech only, not a decision".
- [ ] **Step 2:** Run for each of `parakeet`, `orukeet`, `qwen3`, and `qwen3 --context`: `stt --source recorded`,
  `stt --source synthetic`, `stt --source recorded --threads 2`, `silence`. Record peak RAM (Task Manager) per engine.
- [ ] **Step 3:** Fill this table in the results doc:

| Engine | WER recorded | Term recall recorded | Decode p90 (8 threads / 2 threads) | RAM | Silence | Numbers as digits? | Licence |
|---|---|---|---|---|---|---|---|

- [ ] **Step 4: Decision rule.** A candidate replaces Parakeet v2 for final text only if all hold: term recall at
  least 10 points higher on recorded speech, WER not worse, silence test passes, decode p90 at or under 1.5 s on
  2 threads for a 10 s segment, licence allows download by a free MIT app. Write the verdict.
  **Then stop and show the owner the table. Do not start Task 18 without their pick.**
- [ ] **Step 5:** Commit `Docs: recognizer bake-off results`.

### Task 3c: round health line

**Files:**
- Create: `src-tauri/src/voice/health.rs`
- Modify: `src-tauri/src/voice/mod.rs`

**Interfaces:**
- Produces:
  - `#[derive(Default)] pub struct Health` with methods `reply()`, `dry_spells(n: u32)`, `cap_hit()`, `retry()`,
    `piece(rtf: f32)`, and `pub fn summary(&self) -> Summary`.
  - `#[derive(Debug, PartialEq)] pub struct Summary { pub replies: u32, pub pieces: u32, pub dry_spells: u32,
    pub cap_hits: u32, pub retries: u32, pub rtf_p50: f32, pub rtf_p90: f32 }`. Percentile = nearest rank on the
    sorted list; 0.0 when no pieces.
  - `RoundVoice` gains `health: Arc<Health>`; `Voice::end` logs
    `tracing::info!(replies, pieces, dry_spells, cap_hits, retries, rtf_p50, rtf_p90, "voice health")` before dropping
    the round, only when `pieces > 0`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn summary_counts_and_percentiles() {
    let h = Health::default();
    h.reply();
    for r in [0.2, 0.3, 0.4, 0.5, 1.2] { h.piece(r); }
    h.dry_spells(2);
    h.cap_hit();
    h.retry();
    let s = h.summary();
    assert_eq!((s.replies, s.pieces, s.dry_spells, s.cap_hits, s.retries), (1, 5, 2, 1, 1));
    assert_eq!((s.rtf_p50, s.rtf_p90), (0.4, 1.2));
    assert_eq!(Health::default().summary().rtf_p90, 0.0);
}
```

- [ ] **Step 2:** Run `cargo test ... voice::health`. Expected: compile error.
- [ ] **Step 3:** Implement with atomics and a `Mutex<Vec<f32>>`. In `mod.rs`: call `health.reply()` where
  `speaking` turns true, `health.piece(rtf)` next to the existing `tts` log line in `speak_into`,
  `health.dry_spells(dry)` where `take_underruns` is read.
- [ ] **Step 4:** Run `cargo test ... voice::`. Expected: pass.
- [ ] **Step 5:** Commit `Voice: per-round health line`.

---

# Part 1: speech text (S1, S5)

### Task 4: `speech_text.rs`

**Files:**
- Create: `src-tauri/src/voice/speech_text.rs`, `prompts/pronounce.json`
- Modify: `src-tauri/src/voice/mod.rs` (`pub mod speech_text;`)

**Interfaces:**
- Produces:
  - `pub struct Lexicon` (private fields), `impl Default` = empty.
  - `pub fn Lexicon::from_json(json: &str) -> Result<Lexicon, String>`; shape `{"say":[{"term":"..","as":".."}]}`.
  - `pub fn Lexicon::load(prompts_dir: &Path) -> Lexicon`: reads `pronounce.json`; on any error logs a warning
    (`tracing::warn!`, no content) and returns the empty lexicon.
  - `pub fn int_words(n: u64) -> String`
  - `pub fn to_speech(text: &str, lex: &Lexicon) -> String`

**`prompts/pronounce.json` starter content** (bench-driven: after Task 5, remove any entry the bench shows the voice
already says right without it, and add entries for terms it gets wrong):

```json
{ "say": [
  { "term": "PostgreSQL", "as": "Postgres Q L" },
  { "term": "gRPC", "as": "G R P C" },
  { "term": "nginx", "as": "engine x" },
  { "term": "k8s", "as": "kubernetes" },
  { "term": "OAuth", "as": "oh auth" },
  { "term": "SQL", "as": "sequel" },
  { "term": "MySQL", "as": "my sequel" },
  { "term": "NoSQL", "as": "no sequel" },
  { "term": "JWT", "as": "J W T" },
  { "term": "C++", "as": "C plus plus" },
  { "term": "C#", "as": "C sharp" },
  { "term": "I/O", "as": "I O" },
  { "term": "TCP/IP", "as": "T C P I P" },
  { "term": "CI/CD", "as": "C I C D" },
  { "term": "24/7", "as": "twenty four seven" },
  { "term": "e.g.", "as": "for example" },
  { "term": "i.e.", "as": "that is" },
  { "term": "etc.", "as": "et cetera" },
  { "term": "vs", "as": "versus" },
  { "term": "vs.", "as": "versus" }
] }
```

**Rules of `to_speech`, applied in this order.** Use the `regex` crate; compile each pattern once
(`std::sync::LazyLock`).

1. **Clean.** Remove `*`, backticks, leading `#`. Replace U+2014 and U+2013 (write them as `\u{2014}`, `\u{2013}`)
   with `, `. Replace `...` and U+2026 with `,`. Remove characters in the emoji and pictograph ranges
   (U+1F000..=U+1FAFF, U+2600..=U+27BF, U+FE0F).
2. **URLs.** `https?://\S+` and `www\.\S+` -> `a link` (keep a trailing `.`, `,`, `?`, `!` that ended the URL).
3. **Long values.** Any whitespace-delimited token longer than 40 characters -> `a long value`.
4. **Lexicon.** Longest term first. A match needs a non-alphanumeric character or the text edge on both sides.
   Case-sensitive. Wrap each replacement in `\u{E000}` .. `\u{E001}`; every later rule skips text inside these marks.
5. **Big-O.** `O\(([^)]*)\)` -> `O of <inner>`, where inner gets: `^2` -> ` squared`, `^3` -> ` cubed`,
   `*` -> ` times `, and a lone number -> `int_words`.
6. **Generics and brackets.** `<` or `>` with no space on either side (as in `HashMap<String, Integer>`) -> space.
   Then remove `(`, `)`, `[`, `]`, `{`, `}` (keep what is inside).
7. **Operators.** In this order, each surrounded by optional spaces, replaced with ` <words> `:
   `->`, `=>`, U+2192 -> `to`; `>=` -> `greater than or equal to`; `<=` -> `less than or equal to`;
   `!=` -> `not equal to`; `==` -> `equals`; `&&` -> `and`; `||` -> `or`; ` = ` -> `equals`; ` < ` -> `less than`;
   ` > ` -> `greater than`; ` + ` -> `plus`; ` & ` -> `and`; `@` -> `at`.
   `~` directly before a digit -> `about `. `|` -> space.
8. **Numbers.** `d` = digits, commas between digits are dropped first (`1,000,000` -> `1000000`).
   - Money: `\$(d+(\.d+)?)` -> `<number> dollars` (`one dollar` when the value is exactly 1).
   - Percentile: `\b[pP](d+(\.d+)?)\b` -> `P <number>`.
   - Version: `\bv(d+(\.d+)+)\b` -> `version <a> point <b> point <c>` (each part with `int_words`).
   - Ordinal: `\b(d+)(st|nd|rd|th)\b` -> word from a table for 1..=31; above 31 leave the digits.
   - Range: `\b(d+)-(d+)\b` -> `<a> to <b>`.
   - Percent: `<number>%` -> `<number> percent`.
   - Number + suffix, attached or with one space, suffix followed by a non-letter:
     `ms` milliseconds, `s` (attached only) seconds, `KB` kilobytes, `MB` megabytes, `GB` gigabytes, `TB` terabytes,
     `QPS` Q P S, `RPS` R P S, `x` (attached only) times, `k` or `K` (attached only) thousand,
     `M` (attached only) million, `B` (attached only) billion. Units become singular when the number is exactly 1
     (`1 ms` -> `one millisecond`). `k`, `M`, `B` multiply: `10k` -> `ten thousand`. A multiplier may be followed
     by a space and a unit from this list (`10k QPS` -> `ten thousand Q P S`); match that as one pattern.
   - Decimal: `d+\.d+` -> `int_words(whole) point <each fraction digit as a word>` (`99.99` -> `ninety nine point
     nine nine`, `0.5` -> `zero point five`).
   - Letters and digits in one token (`MT103`, `S3`, `sha256`, `base64`, `pacs.008` after rule 9): split into letter
     runs and digit runs joined by spaces. A digit run of 1 or 2 digits -> `int_words`; 3 or more -> digit by digit,
     `0` said as `oh`. An all-uppercase letter run of 2-4 letters is spelled with spaces (`MT` -> `M T`).
   - Plain integer `\b\d+\b` -> `int_words` (up to 999,999,999,999; longer -> digit by digit).
9. **Dots and identifiers.**
   - `.` with a letter on both sides, or a letter before and a digit after (`user.id`, `Node.js`) -> ` dot `.
   - `_` between alphanumerics -> space.
   - `/` between letters -> space.
   - Camel case: inside a token, insert a space at every lowercase-to-uppercase boundary (`getUserById` ->
     `get User By Id`, `HashMap` -> `Hash Map`). Case is kept.
10. **Tidy.** Remove the private-use marks. Collapse runs of spaces. Remove a space before `. , ? ! ; :`.
    Collapse `, ,` and `,,` to `,`. Trim. If nothing but punctuation is left, return the empty string.

`int_words`: no "and", no hyphens: `256` -> `two hundred fifty six`, `1000000` -> `one million`, `0` -> `zero`,
`1001` -> `one thousand one`.

- [ ] **Step 1: Write the failing tests**

```rust
fn lex() -> Lexicon {
    Lexicon::from_json(include_str!("../../../prompts/pronounce.json")).unwrap()
}

#[test]
fn int_words_basics() {
    for (n, w) in [(0, "zero"), (7, "seven"), (13, "thirteen"), (40, "forty"), (99, "ninety nine"),
        (100, "one hundred"), (256, "two hundred fifty six"), (1001, "one thousand one"),
        (10_000, "ten thousand"), (1_000_000, "one million"), (2_500_000_000, "two billion five hundred million")] {
        assert_eq!(int_words(n), w);
    }
}

#[test]
fn fixture_lines_become_their_spoken_form() {
    let tsv = include_str!("../../tests/fixtures/voice/tts_tech_lines.tsv");
    let empty = Lexicon::default();
    for row in tsv.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let (written, spoken) = row.split_once('\t').unwrap();
        assert_eq!(to_speech(written, &empty), spoken, "{written}");
    }
}

#[test]
fn lexicon_terms_win_over_rules() {
    let l = lex();
    assert_eq!(to_speech("We use PostgreSQL and gRPC, e.g. for C++ services.", &l),
        "We use Postgres Q L and G R P C, for example for C plus plus services.");
    assert_eq!(to_speech("It runs 24/7 on k8s.", &l), "It runs twenty four seven on kubernetes.");
    // A term inside a longer word is not touched.
    assert_eq!(to_speech("The SQLite file.", &l), "The SQLite file.");
}

#[test]
fn codes_and_mixed_tokens() {
    let e = Lexicon::default();
    assert_eq!(to_speech("Send an MT103.", &e), "Send an M T one oh three.");
    assert_eq!(to_speech("Store it in S3 with sha256.", &e), "Store it in S three with sha two five six.");
    assert_eq!(to_speech("Use Node.js here.", &e), "Use Node dot js here.");
}

#[test]
fn review_focus_inputs() {
    let e = Lexicon::default();
    // 1. Nothing left to say.
    assert_eq!(to_speech("**\u{1F600}** ()", &e), "");
    assert_eq!(to_speech("   ", &e), "");
    // 2. A huge token is never read out.
    let blob = "A".repeat(300);
    assert_eq!(to_speech(&format!("The key is {blob} today."), &e), "The key is a long value today.");
    // 3. Names with accents pass through.
    assert_eq!(to_speech("Thanks, Jos\u{e9} and Zo\u{eb}.", &e), "Thanks, Jos\u{e9} and Zo\u{eb}.");
    // Plain sentences are unchanged.
    let plain = "Walk me through a project you are proud of.";
    assert_eq!(to_speech(plain, &e), plain);
}

#[test]
fn bad_lexicon_file_gives_an_empty_lexicon() {
    assert!(Lexicon::from_json("{not json").is_err());
    let dir = std::env::temp_dir().join("onsite-no-such-prompts");
    assert_eq!(to_speech("SQL", &Lexicon::load(&dir)), "SQL");
}
```

- [ ] **Step 2:** Run `cargo test ... voice::speech_text`. Expected: compile error.
- [ ] **Step 3:** Implement. If a fixture row cannot be produced by the rules as written, fix the code, not the
  fixture; if a rule above is truly wrong for a row, stop and report the row.
- [ ] **Step 4:** Run the same command. Expected: 6 passed.
- [ ] **Step 5:** Commit `Voice: speech text normalizer and pronunciation lexicon`.

### Task 5: wire the normalizer in, one sentence rule

**Files:**
- Modify: `src-tauri/src/brain/splitter.rs`, `src-tauri/src/voice/builtin.rs`, `src-tauri/src/voice/engine.rs`,
  `prompts/interviewer.md`, `src-tauri/examples/voice_bench/main.rs`

**Interfaces:**
- Consumes: `speech_text::{Lexicon, to_speech}`.
- Produces:
  - `pub fn splitter::first_sentence_len(s: &str) -> Option<usize>` (today's private `complete_prefix_len_first`,
    renamed and made `pub`).
  - `builtin::sentences` uses `first_sentence_len` in a loop and takes the rest as the last sentence. Its old byte
    scan is deleted.
  - `Engine` gains `pub lexicon: Arc<Lexicon>`, loaded in `Engine::load` with `Lexicon::load(&state.prompts_dir)`.
  - `Tts::new(tts, clip, lexicon: Arc<Lexicon>)`; `Tts::speak` calls `to_speech(text, &lexicon)` first and returns
    `Ok(())` without a TTS call when the result is empty.

- [ ] **Step 1: Failing tests** (in `builtin.rs` tests)

```rust
#[test]
fn pieces_use_the_splitter_rules() {
    // "e.g." does not end a piece any more.
    assert_eq!(pieces("Use a map, e.g. a hash map. It is fast."), ["Use a map, e.g. a hash map. It is fast."]);
    assert_eq!(sentences("Use a map, e.g. a hash map. It is fast."), ["Use a map, e.g. a hash map.", "It is fast."]);
}
```

- [ ] **Step 2:** Run `cargo test ... voice::builtin`. Expected: FAIL on the `sentences` assertion.
- [ ] **Step 3:** Make the changes. Update every `Tts::new` caller (`Engine::voice`, `commands.rs` previews).
- [ ] **Step 4:** In `prompts/interviewer.md`, replace the "Write for the ear" bullet with exactly:
  `- Write for the ear: plain sentences, no symbols or abbreviations that read badly aloud. Say "O of n log n", not "O(n log n)". Say "one point five milliseconds", not "1.5 ms". Say "the user id field", not "user_id".`
- [ ] **Step 5:** In `voice_bench tts`, apply `to_speech` with the real lexicon unless `--raw` is given.
- [ ] **Step 6:** Run `cargo test ...` (all). Expected: pass.
- [ ] **Step 7: Gate.** Run `voice_bench tts --set tech` and `--set tech --sid 0`. Record under "After Task 5" in
  the results doc. Pass = mean WER on the tech set at or under 3% and no line where the transcript shows a sentence
  break at a decimal. For each line that still fails on a term, add or fix a `pronounce.json` entry and re-run.
  Remove starter entries the `--raw` baseline already got right.
- [ ] **Step 8:** Commit `Voice: speak normalized text; one sentence rule`.

---

# Part 2: generation (S2, S3, S4, S9)

### Task 6: guard, retry and edges

**Files:**
- Modify: `src-tauri/src/voice/builtin.rs`

**Interfaces:**
- Produces (all in `builtin.rs`):
  - `pub enum Verdict { Ok, TooShort, TooLong { keep: usize } }`
  - `pub fn judge(samples: usize, words: usize, speed: f32, rate: u32) -> Verdict` with
    `max_s = words as f32 * 0.9 / speed.clamp(0.5, 2.0) + 3.0` and
    `min_s = words as f32 * MIN_S_PER_WORD / speed.clamp(0.5, 2.0)`; `TooShort` only when `words >= 4`.
    `pub const MIN_S_PER_WORD: f32 = 0.16;` (after this task, set it to 0.7 x the smallest `sec_per_word` in the
    Task 5 gate run and note the number in a comment).
  - `pub fn fade_in(s: &mut [f32], n: usize)`, `pub fn fade_out(s: &mut [f32], n: usize)`: linear ramps over the
    first or last `n` samples (or the whole slice when shorter).
  - `pub fn trim_silence(s: &[f32], rate: u32) -> &[f32]`: drops leading and trailing 10 ms windows whose RMS is
    under 0.004, but keeps 20 ms on each side.
  - `pub fn gap_ms(piece: &str) -> u32`: 250 when the trimmed piece ends with `.`, `!` or `?`, else 80.
  - `pub struct Piece { pub pcm: Vec<i16>, pub ends_sentence: bool }`
  - `pub fn Supertonic::speak_piece(&self, text: &str, s: &Synth, stop: &AtomicBool, health: &Health) -> Result<Piece>`:
    `synth`; `judge` with `words = text.split_whitespace().count()`; on `TooShort` call `health.retry()` and synth
    once more with `steps + 2`, keep the longer result; on `TooLong { keep }` truncate to `keep`, `fade_out` 30 ms,
    `health.cap_hit()`; then `trim_silence`, `fade_in` and `fade_out` 10 ms, append `gap_ms` of zeros, convert to i16.
  - The old `Supertonic::speak` and `send` are deleted, with their test `audio_is_capped_and_stops_when_closed`.

- [ ] **Step 1: Failing tests**

```rust
#[test]
fn judge_scales_with_speed_and_words() {
    let r = 44_100;
    assert!(matches!(judge(r as usize * 4, 10, 1.0, r), Verdict::Ok));
    // 10 words in 1 s: words were dropped.
    assert!(matches!(judge(r as usize, 10, 1.0, r), Verdict::TooShort));
    // Short lines are never "too short".
    assert!(matches!(judge(r as usize / 4, 3, 1.0, r), Verdict::Ok));
    // 10 words: cap is 12 s at speed 1.0 and 14.25 s at 0.8.
    assert!(matches!(judge(r as usize * 13, 10, 1.0, r), Verdict::TooLong { keep } if keep == r as usize * 12));
    assert!(matches!(judge(r as usize * 13, 10, 0.8, r), Verdict::Ok));
}

#[test]
fn fades_and_trim() {
    let mut s = vec![1.0f32; 100];
    fade_in(&mut s, 10);
    fade_out(&mut s, 10);
    assert_eq!(s[0], 0.0);
    assert!(s[5] > 0.3 && s[5] < 0.7 && s[50] == 1.0 && s[99] < 0.15);
    let rate = 16_000;
    let mut clip = vec![0.0f32; 4800];
    clip.extend(vec![0.5f32; 1600]);
    clip.extend(vec![0.0f32; 4800]);
    let t = trim_silence(&clip, rate);
    // 100 ms of sound plus 20 ms kept on each side.
    assert!((2200..=2600).contains(&t.len()), "{}", t.len());
    assert_eq!(trim_silence(&[0.0; 3200], rate).len(), 0);
}

#[test]
fn gaps_follow_punctuation() {
    assert_eq!(gap_ms("Okay, got it."), 250);
    assert_eq!(gap_ms("If the key repeats,"), 80);
    assert_eq!(gap_ms("Why?  "), 250);
}
```

- [ ] **Step 2:** Run `cargo test ... voice::builtin`. Expected: compile errors.
- [ ] **Step 3:** Implement. In `engine.rs`, `Tts::speak` becomes
  `pub async fn speak(&self, text: &str, speed: f32, steps: i32, max_piece_words: usize, health: Arc<Health>, mut on_piece: impl FnMut(Piece)) -> Result<()>`:
  normalize, `pieces(text, max_piece_words)`, one `speak_piece` per piece inside `spawn_blocking`, an
  `Arc<AtomicBool>` stop flag set when the receiver is dropped. `pieces` gains the `max_words: usize` parameter
  (replacing `MAX_PIECE_WORDS`); existing tests pass `14`. Callers pass `steps = 8`, `max_piece_words = 14` for now.
  `ends_sentence` = the piece text ends with `.`, `!` or `?`.
- [ ] **Step 4:** Update callers in `mod.rs` (`speak_into`, `speak_as`, warm-up) and `commands.rs` to push
  `piece.pcm` as before. Behaviour is otherwise unchanged in this task.
- [ ] **Step 5:** Run `cargo test ...` (all). Expected: pass.
- [ ] **Step 6:** Commit `Voice: guarded generation with retry, fades and fixed gaps`.

### Task 7: voices and speed range from the sweep

**Files:**
- Modify: `src-tauri/src/voice/builtin.rs`, `docs/plans/voice-bench-results.md`

- [ ] **Step 1:** Re-run `voice_bench sweep --save-wav` (now with the normalizer). Paste the grid into the results doc.
- [ ] **Step 2: Pick by rule.** Female voice: among sids 0-4, the one with zero missing-word lines at speed 1.0 and
  1.1 and the lowest mean WER; keep sid 0 on a tie. Male voice: among sids 5-9, zero missing-word lines at 1.0 and
  1.1, and words per second at speed 1.0 closest to the female pick's. `SPEED_MAX` = the highest swept speed at
  which both picks have zero missing-word lines (at least 1.0). Write the picks and the numbers in the results doc.
- [ ] **Step 3:** **Stop and ask the owner to listen** to `D:\cargo-target\voice-bench\out\listen\sid<N>.wav` for
  the two picks. Continue with their answer (they may choose another sid from the same table).
- [ ] **Step 4:** Apply: update the sid numbers in `VOICES`; delete `tempo` and its uses (speed is the user's speed
  x the persona's pace only); add `pub const SPEED_RANGE: (f32, f32) = (0.9, <SPEED_MAX>);` and clamp the final
  speed to it where `Synth` is built. Update the test `voices_map_to_supertonic_speakers` to the new ids and delete
  its `tempo` assertion. Add:

```rust
#[test]
fn speed_is_clamped_to_the_safe_range() {
    assert_eq!(safe_speed(2.0), SPEED_RANGE.1);
    assert_eq!(safe_speed(0.5), SPEED_RANGE.0);
    assert_eq!(safe_speed(1.0), 1.0);
}
```

  with `pub fn safe_speed(speed: f32) -> f32`.
- [ ] **Step 5: Gate.** `voice_bench tts` for both picks, 3 runs each (`--runs 3`). Pass = zero cap hits and zero
  lines with a missing word after the retry. If it fails, record the failing lines and run the fallback bake-off:
  add `kokoro` and `kitten` engines to a new `voice_bench tts --engine` flag (configs `OfflineTtsKokoroModelConfig`,
  `OfflineTtsKittenModelConfig`, models from the sherpa-onnx `tts-models` release), report, and stop for the owner.
- [ ] **Step 6:** Run all tests, commit `Voice: voices and speed range chosen from the sweep`.

---

# Part 3: playback and compute (S4, S6, S7, S8)

### Task 8: machine tier

**Files:**
- Create: `src-tauri/src/voice/tier.rs`
- Modify: `src-tauri/src/voice/mod.rs`, `builtin.rs` (`Cache`), `engine.rs`, `src-tauri/src/commands.rs`, `ui/app.js`, `ui/index.html`

**Interfaces:**
- Produces:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier { Fast, Ok, Slow }

impl Tier {
    pub fn from_rtf(rtf: f32) -> Tier;            // < 0.35 Fast, < 0.7 Ok, else Slow
    pub fn steps(self) -> i32;                    // 8, 6, 5
    pub fn piece_words(self) -> usize;            // 40, 24, 14
    pub fn caption_every(self) -> Option<Duration>; // 500 ms, 1 s, None
    pub fn prebuffer_ms(self) -> u32;             // 300, 500, 800
    pub fn slower(self) -> Tier;                  // Fast -> Ok -> Slow -> Slow
    pub fn label(self) -> &'static str;           // "fast", "okay", "slow"
}

/// Running check during a round: demote when the last 5 pieces average above the tier's bound x 1.2.
pub struct Drift { .. }
impl Drift {
    pub fn new(tier: Tier) -> Self;
    pub fn tier(&self) -> Tier;
    /// Returns the new tier when this piece caused a demotion.
    pub fn piece(&mut self, rtf: f32) -> Option<Tier>;
}
```

  - `Cache` gains `tier: Mutex<Option<Tier>>` with `pub fn tier(&self) -> Option<Tier>` and `pub fn set_tier(&self, t: Tier)`.
    `unload` clears it.
  - `Engine` gains `pub tier: Tier` (from the cache; `Tier::Ok` until measured).
  - `VoiceStatus` (commands.rs) gains `tier: Option<&'static str>` = `label()`.

**Measuring:** in `Voice::begin`'s warm-up, when `cache.tier()` is `None`: speak
`"Okay, let's begin. Thanks for joining me today."` twice at 8 steps, discard the audio, and take
`rtf = generation seconds / audio seconds` of the second run. `set_tier(Tier::from_rtf(rtf))` and log
`tracing::info!(rtf, tier = tier.label(), "voice tier")`. When a tier is cached, keep today's single short warm-up.
The round then uses `tier.steps()` and `tier.piece_words()` in every `Tts::speak` call and a `Drift` in `speak_into`
(on a demotion: `cache.set_tier`, log it, use the new settings from the next sentence).

- [ ] **Step 1: Failing tests** (in `tier.rs`)

```rust
#[test]
fn tiers_from_speed() {
    assert_eq!(Tier::from_rtf(0.2), Tier::Fast);
    assert_eq!(Tier::from_rtf(0.35), Tier::Ok);
    assert_eq!(Tier::from_rtf(0.69), Tier::Ok);
    assert_eq!(Tier::from_rtf(0.7), Tier::Slow);
    assert_eq!((Tier::Fast.steps(), Tier::Ok.steps(), Tier::Slow.steps()), (8, 6, 5));
    assert_eq!((Tier::Fast.piece_words(), Tier::Ok.piece_words(), Tier::Slow.piece_words()), (40, 24, 14));
    assert_eq!(Tier::Slow.caption_every(), None);
    assert_eq!(Tier::Slow.slower(), Tier::Slow);
}

#[test]
fn drift_demotes_once_after_five_slow_pieces() {
    let mut d = Drift::new(Tier::Fast);
    for _ in 0..4 { assert_eq!(d.piece(0.6), None); }
    assert_eq!(d.piece(0.6), Some(Tier::Ok));
    // The window restarts after a demotion.
    for _ in 0..4 { assert_eq!(d.piece(0.6), None); }
    assert_eq!(d.piece(0.6), None, "0.6 is inside the Ok bound of 0.7 x 1.2");
    assert_eq!(d.tier(), Tier::Ok);
    // One slow piece among fast ones does nothing.
    let mut d = Drift::new(Tier::Fast);
    for r in [0.2, 0.2, 1.5, 0.2, 0.2] { assert_eq!(d.piece(r), None); }
}
```

- [ ] **Step 2:** Run `cargo test ... voice::tier`. Expected: compile error.
- [ ] **Step 3:** Implement `tier.rs` and the wiring above.
- [ ] **Step 4:** UI: in the Settings > Voice section of `ui/index.html` add one line element
  `<p id="voiceTier" class="hint" hidden></p>`. In `ui/app.js`, where the `voice_status` result is rendered, set its
  text to `This laptop: fast.`, `This laptop: okay.` or `This laptop: slow. Replies may take longer to start.` and
  unhide it when `tier` is not null. Use existing classes and tokens only.
- [ ] **Step 5:** Run all tests. Expected: pass.
- [ ] **Step 6:** Commit `Voice: machine tier from measured speed`.

### Task 9: lock-free player and realtime audio thread

**Files:**
- Modify: `src-tauri/Cargo.toml` (`rtrb = "0.3"`; `cpal = { version = "0.18.2", features = ["realtime"] }`),
  `src-tauri/src/voice/audio.rs`

**Design (decided):**
- `Queue` and its `Mutex` go away. `Player` holds `producer: Mutex<rtrb::Producer<f32>>` (only producers lock it)
  and `shared: Arc<Shared>`; the device callback owns the `rtrb::Consumer<f32>` and a `Jitter`.
- `struct Shared { streaming: AtomicBool, flush: AtomicBool, queued: AtomicUsize, prebuffer: AtomicUsize, underruns: AtomicU32 }`.
- Ring capacity: 30 s at the device rate. `Player::push` writes what fits; if the ring is full it waits in 5 ms
  sleeps up to 2 s for room, then drops the rest and logs a warning once per reply. `queued` += written.
- Callback, per buffer: if `flush` is set, pop everything, `queued = 0`, clear `flush`, reset `jitter.rebuffering`
  to `streaming`. Then for each frame call `jitter.next(...)`; `queued` -= popped.
- `Jitter::next` becomes generic over a tiny trait so the tests keep working:

```rust
pub trait Source { fn len(&self) -> usize; fn pop(&mut self) -> Option<f32>; }
impl Source for VecDeque<f32> { .. }      // used by the existing jitter tests
impl Source for rtrb::Consumer<f32> { .. } // len = slots()
pub fn next(&mut self, q: &mut impl Source, streaming: bool) -> f32;
```

  `Jitter.prebuffer` is read from `Shared.prebuffer` at the top of each callback; `Jitter.underruns` increments go to
  `Shared.underruns`.
- `Player::clear` sets `flush` and `streaming = false`. `is_idle` = `dead || queued == 0`.
  `set_streaming`, `take_underruns` use the atomics. New `pub fn set_prebuffer_ms(&self, ms: u32)`.
- `ensure` (device change) builds a new ring and stream; the old producer is replaced under its mutex.
- No allocation, lock or log call inside the callback except the existing level meter store.

- [ ] **Step 1: Failing test** (the three existing `jitter_*` tests must keep passing through the `Source` trait; add)

```rust
#[test]
fn ring_source_reports_len_and_pops_in_order() {
    let (mut p, mut c) = rtrb::RingBuffer::<f32>::new(8);
    for v in [0.1, 0.2, 0.3] { p.push(v).unwrap(); }
    assert_eq!(Source::len(&c), 3);
    assert_eq!(Source::pop(&mut c), Some(0.1));
    let mut j = Jitter { prebuffer: 2, ..Default::default() };
    assert_eq!(j.next(&mut c, true), 0.2);
    assert_eq!(j.next(&mut c, true), 0.3);
    assert_eq!(j.next(&mut c, true), 0.0, "dry: silence and an underrun");
    assert_eq!(j.underruns, 1);
}
```

- [ ] **Step 2:** Run `cargo test ... voice::audio`. Expected: compile error.
- [ ] **Step 3:** Implement the design. Keep `Player`'s public method names so `mod.rs` compiles unchanged.
- [ ] **Step 4:** Run `cargo test ...` (all) and `cargo build --target-dir D:\cargo-target\onsite-test`. Expected: pass.
  If the `realtime` feature fails to build on Windows, remove the feature, leave a one-line comment with the error,
  and record it in `VOICE.md` (the ring buffer still ships).
- [ ] **Step 5: Manual check.** `cargo run --release --example builtin_check --target-dir D:\cargo-target\onsite-test`
  plays a reply; listen for clean audio. Unplug or switch the output device mid-play: the next reply plays on the
  new device and nothing panics.
- [ ] **Step 6:** Commit `Voice: lock-free speaker queue and realtime audio thread`.

### Task 10: filtered resampling for playback

**Files:**
- Modify: `src-tauri/src/voice/audio.rs`

**Interfaces:**
- `Resampler` keeps its name and `new(from, to)` / `process(&mut self, &[f32]) -> Vec<f32>`. Inside, when
  `from != to` it wraps `sherpa_onnx::LinearResampler::create(from as i32, to as i32)` and calls
  `resample(input, false)`. If `create` returns `None`, fall back to the current linear code (kept as a private fn)
  and log a warning once. Add `pub fn flush(&mut self) -> Vec<f32>` (`resample(&[], true)`), called by `speak_into`
  after the last piece of a reply.
- `Downsampler` (mic path) keeps using the private linear code after its own low-pass. It must not change.

- [ ] **Step 1: Failing test** (replaces `resample_length_and_dc` and `resample_is_continuous_across_chunks`, which
  assume linear interpolation; keep the mic tests untouched by pointing `Downsampler` at the private linear type)

```rust
#[test]
fn playback_resampler_keeps_level_and_length() {
    let input = tone(44_100, 1000.0, 1.0);
    let mut r = Resampler::new(44_100, 48_000);
    let mut out = Vec::new();
    for c in input.chunks(441) { out.extend(r.process(c)); }
    out.extend(r.flush());
    assert!((47_500..=48_500).contains(&out.len()), "{}", out.len());
    let level = rms(&out[4800..43_200]);
    assert!((level / (0.5 / 2f32.sqrt()) - 1.0).abs() < 0.05, "{level}");
    // Same rate: untouched.
    assert_eq!(Resampler::new(48_000, 48_000).process(&[0.25; 10]), vec![0.25; 10]);
}
```

- [ ] **Step 2:** Run `cargo test ... voice::audio`. Expected: FAIL (`flush` missing).
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run all tests. Expected: pass, including the `downsampler_*` tests.
- [ ] **Step 5:** Commit `Voice: filtered resampling for playback`.

### Task 11: sentence-level release

**Files:**
- Modify: `src-tauri/src/voice/mod.rs`

**Rule (decided):** audio reaches the player a whole sentence at a time. `speak_into` collects `Piece.pcm` into a
`Vec<i16>` and pushes it when a piece has `ends_sentence == true` or the call ends. Exception: on `Tier::Fast`,
pieces are pushed as they arrive. At the start of each reply call `player.set_prebuffer_ms(tier.prebuffer_ms())`.
The first-audio latency event fires on the first push, not the first piece.

**Interfaces:**
- Produces: `fn release_now(tier: Tier, ends_sentence: bool, last: bool) -> bool` =
  `tier == Tier::Fast || ends_sentence || last`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn slower_machines_release_whole_sentences() {
    assert!(release_now(Tier::Fast, false, false));
    assert!(!release_now(Tier::Ok, false, false));
    assert!(release_now(Tier::Ok, true, false));
    assert!(!release_now(Tier::Slow, false, false));
    assert!(release_now(Tier::Slow, false, true), "the end of the call always flushes");
}
```

- [ ] **Step 2:** Run `cargo test ... voice::tests`. Expected: compile error.
- [ ] **Step 3:** Implement. `Tts::speak`'s `on_piece` already gives `Piece`; pass `last` by flushing after the call
  returns. A cut-in (`cut` set) drops the held buffer.
- [ ] **Step 4:** Run all tests. Expected: pass.
- [ ] **Step 5: Gate.** Two runs, recorded in the results doc under "After Task 11":
  1. A real spoken round on the owner's laptop (or `builtin_check`): the `voice health` line shows
     `dry_spells = 0`, and the `first interviewer audio after answer` line is at or under 1500 ms after the first
     sentence arrives.
  2. Slow-laptop mode: `start /affinity 0x3 voice_bench.exe tts --threads 2`: RTF p90 under 1.0 at the Slow tier's
     5 steps (`--steps 5`). If it is not, write the numbers down and stop for the owner (this is the trigger for the
     fallback voice bake-off in Task 7 Step 5).
- [ ] **Step 6:** Commit `Voice: pauses fall between sentences`.

### Task 12: captions follow the tier; spinning

**Files:**
- Modify: `src-tauri/src/voice/stt.rs`, `mod.rs`, `builtin.rs`

**Interfaces:**
- `SttOptions` gains `pub caption_every: Option<Duration>` (default `Some(500 ms)`). `run` builds its caption
  interval from it and never calls `spawn_caption` when it is `None`. `CAPTION_EVERY` is deleted.
- `listen_once` builds `SttOptions` from the engine's tier (`engine.tier.caption_every()`).
- If Task 3 Step 5 found a way to turn off ONNX Runtime spinning through sherpa-onnx: apply it in
  `Supertonic::load_with` and `ParakeetStt::load_with`, and re-run `voice_bench tts` to confirm RTF p50 is within
  15% of before. If it is slower than that, leave spinning on and record both numbers (it can't be set per tier:
  the models load before the tier is known). If no way was found, skip this part.

- [ ] **Step 1: Failing test** (in `stt.rs`)

```rust
#[test]
fn options_default_to_half_second_captions() {
    assert_eq!(SttOptions::default().caption_every, Some(Duration::from_millis(500)));
}
```

- [ ] **Step 2:** Run `cargo test ... voice::stt`. Expected: compile error.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run all tests. Expected: pass.
- [ ] **Step 5:** Commit `Voice: caption rate follows the machine tier`.

---

# Part 4: hearing names and terms (L1-L5)

### Task 13: round vocabulary

**Files:**
- Create: `src-tauri/src/voice/vocab.rs`
- Modify: `src-tauri/src/voice/mod.rs`, `stt.rs`, `src-tauri/src/interview/live.rs`

**Interfaces:**
- Produces (in `vocab.rs`):

```rust
pub const MAX_TERMS: usize = 150;

#[derive(Default)]
pub struct Sources<'a> {
    pub resume: &'a str,
    pub jd: &'a str,
    pub notes: &'a str,
    /// Company, persona and problem names, already as terms.
    pub names: Vec<String>,
    /// Dossier text (company style, round names, reported questions), free text.
    pub dossier: &'a str,
}

/// Terms worth protecting in a transcript, most important first, at most MAX_TERMS.
pub fn round_terms(s: &Sources) -> Vec<String>;
/// Candidate terms in free text, in order of first appearance.
pub fn terms_in(text: &str) -> Vec<String>;
/// The candidate's name when the resume's first non-empty line looks like one.
pub fn candidate_name(resume: &str) -> Option<String>;
```

- **`terms_in` rules.** Tokenize on whitespace; strip leading and trailing punctuation. A token is a term when any holds:
  1. it has an uppercase letter after its first character and at least one lowercase letter (`PostgreSQL`, `gRPC`, `DynamoDB`);
  2. it mixes letters and digits and is 2-12 characters (`S3`, `MT103`, `k8s`);
  3. it is all uppercase letters, 2-6 long, and not in `STOP_CAPS` (`I`, `A`, `AND`, `OR`, `THE`, `OF`, `IN`, `TO`,
     `GPA`, `CV`, `USA`, `UK`, `PDF`, `CEO`, `CTO`, `HR`, `MBA`, `BE`, `BS`, `MS`, `PHD`, `IT`);
  4. it starts with an uppercase letter, the rest is lowercase, it is at least 4 letters long, it is not the first
     token after a sentence end or line start, and it is not in `COMMON` (months, weekdays, and: `Developed`,
     `Built`, `Designed`, `Led`, `Worked`, `Engineer`, `Senior`, `Software`, `Experience`, `Education`, `Skills`,
     `Projects`, `University`, `College`, `Bachelor`, `Master`, `Technology`, `Team`, `India`, `Present`, `Remote`).
  Two rule-4 tokens next to each other also yield the pair as one term (`Spring Boot`, `Goldman Sachs`).
  Letters are tested with `char::is_uppercase` / `is_lowercase`, so accented names work. Dedupe case-insensitively.
- **`candidate_name`.** First non-empty line of the resume: 2-4 tokens, each alphabetic (any script), each starting
  uppercase, total length at most 40. Returns the full name; `round_terms` also adds its first token.
- **`round_terms` order.** candidate name parts, `names`, then `terms_in` of `jd`, `dossier`, `resume`, `notes`.
  Dedupe case-insensitively, drop terms already in `stt::TECH_TERMS`, truncate to `MAX_TERMS`.
- `VoiceSettings` gains `#[serde(skip)] pub vocab: Vec<String>` (per round, never saved, like `persona`).
  Remove `PartialEq`'s reliance if needed by adding the field to the derive (it is `Vec<String>`, fine).
- `listen_once` builds `SttOptions { vocab: stt::domain_vocab(&refs), caption_every }` from `settings.vocab` and
  calls `stt::listen_with`. `stt::listen` (used by the mic test in `commands.rs`) stays on the default vocabulary.
- `live.rs` `start_round`, next to the persona block (around the comment "A panel interviewer always speaks in their
  own voice"): set `settings.voice.vocab = vocab::round_terms(&Sources { resume: &setup.resume, jd: &setup.jd,
  notes: &setup.notes, names: vec![plan.persona.clone(), setup.company_name(&state.catalog)] plus the coding
  problem's title when the round has one, dossier: <the dossier's company, style and this round's reported
  questions joined with newlines, or ""> })`. Log only `terms = vocab.len()`.

- [ ] **Step 1: Failing tests** (in `vocab.rs`)

```rust
const RESUME: &str = "Prateesh Sulikeri\nBackend Engineer\n\nBuilt payment services at Razorpay using Spring Boot, \
    Kafka and PostgreSQL. Moved settlement files to S3. Worked with Karthik on the Juspay integration.";

#[test]
fn finds_names_products_and_codes() {
    let t = terms_in(RESUME);
    for want in ["Razorpay", "Spring Boot", "PostgreSQL", "S3", "Karthik", "Juspay"] {
        assert!(t.iter().any(|x| x == want), "{want} missing from {t:?}");
    }
    for not in ["Built", "Moved", "Worked", "Engineer"] {
        assert!(!t.iter().any(|x| x == not), "{not} should not be a term");
    }
}

#[test]
fn candidate_name_from_first_line() {
    assert_eq!(candidate_name(RESUME).as_deref(), Some("Prateesh Sulikeri"));
    assert_eq!(candidate_name("Jos\u{e9} Garc\u{ed}a\nEngineer").as_deref(), Some("Jos\u{e9} Garc\u{ed}a"));
    assert_eq!(candidate_name("RESUME\nName: x"), None);
    assert_eq!(candidate_name("Senior backend engineer with 6 years of experience"), None);
    assert_eq!(candidate_name(""), None);
}

#[test]
fn round_terms_order_dedupe_and_cap() {
    let s = Sources { resume: RESUME, jd: "We use Kafka and Hazelcast.", names: vec!["Neha".into(), "Razorpay".into()], ..Default::default() };
    let t = round_terms(&s);
    assert_eq!(&t[..3], ["Prateesh Sulikeri", "Prateesh", "Neha"]);
    assert_eq!(t.iter().filter(|x| x.eq_ignore_ascii_case("razorpay")).count(), 1);
    assert!(t.iter().any(|x| x == "Hazelcast"));
    assert!(!t.iter().any(|x| x == "Kafka"), "already in the static list");
    // Review focus 4: a huge resume is capped and fast.
    let big: String = (0..5000).map(|i| format!("we used Zorblat{i} daily. ")).collect();
    let started = std::time::Instant::now();
    let t = round_terms(&Sources { resume: &big, ..Default::default() });
    assert_eq!(t.len(), MAX_TERMS);
    assert!(started.elapsed() < std::time::Duration::from_millis(50));
}
```

- [ ] **Step 2:** Run `cargo test ... voice::vocab`. Expected: compile error.
- [ ] **Step 3:** Implement `vocab.rs` and the wiring. Fix the `VoiceSettings` literals in tests.
- [ ] **Step 4:** Run all tests. Expected: pass.
- [ ] **Step 5:** Commit `Voice: per-round vocabulary from resume, job description and names`.

### Task 14: tell the interviewer and the report which words to expect

**Files:**
- Modify: `prompts/interviewer.md`, `prompts/report.md`, `src-tauri/src/interview/mod.rs`, `src-tauri/src/interview/report.rs`, `src-tauri/src/interview/live.rs`

**Interfaces:**
- Consumes: `vocab::round_terms`.
- Produces: a `stt_terms` prompt variable in both templates = the first 60 round terms plus `stt::TECH_TERMS`
  entries that occur in the resume or JD (case-insensitive), joined with `, `; `(none)` when empty. Build it in one
  helper `pub fn stt_terms_line(terms: &[String], resume: &str, jd: &str) -> String` in `interview/mod.rs`.
- `prompts/interviewer.md`: add this bullet directly after the "Always speak English" bullet, exactly:
  `- The candidate's words reach you through speech recognition, which mishears names and technical terms. Terms likely in this interview: {{stt_terms}}. When a word sounds like one of them ("reddish" for Redis, "post grass" for Postgres), read it as that term and carry on. Do not mention the recogniser or correct their pronunciation.`
  Add `{{stt_terms}}` to the placeholder comment at the top of the file.
- `prompts/report.md`: append to the "Measured:" line, exactly:
  ` Terms likely in this interview: {{stt_terms}}. Read a near-miss of one of them as that term.`
  and add `{{stt_terms}}` to its placeholder comment.
- The round stores nothing new: both renders rebuild the terms from the loop's setup (the same `Sources` as Task 13;
  move that construction into `pub fn voice_sources(setup, plan, ..)` in `interview/mod.rs` so `live.rs` and
  `report.rs` share it).

- [ ] **Step 1: Failing tests** (in `interview/mod.rs` tests)

```rust
#[test]
fn stt_terms_line_lists_round_terms_then_static_ones_in_use() {
    let line = stt_terms_line(&["Prateesh".into(), "Hazelcast".into()], "I use Kafka daily", "Redis experience");
    assert_eq!(line, "Prateesh, Hazelcast, Kafka, Redis");
    assert_eq!(stt_terms_line(&[], "", ""), "(none)");
}
```

  and extend the existing `renders_full_prompt_without_placeholders` test to assert the rendered prompt contains
  `Terms likely in this interview:` and no `{{`.
- [ ] **Step 2:** Run `cargo test ... interview::`. Expected: FAIL.
- [ ] **Step 3:** Implement.
- [ ] **Step 4:** Run all tests. Expected: pass.
- [ ] **Step 5:** Commit `Interview: expected terms in the interviewer and report prompts`.

### Task 15: Silero VAD and gain

**Files:**
- Create: `prompts/models/silero_vad.onnx` (bundled with `prompts/`, already a Tauri resource)
- Modify: `src-tauri/src/voice/audio.rs`, `stt.rs`, `engine.rs`, `THIRD_PARTY`/notices file, `.graphifyignore` (add `prompts/models/`)

**Model:** download `https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad.onnx`
(about 2 MB, MIT). Record its SHA-256 in `docs/engineering/VOICE.md` and add Silero VAD (MIT, snakers4/silero-vad)
to the third-party notices.

**Interfaces:**
- Produces (in `audio.rs`):

```rust
/// Slow gain control toward a normal speech level. Pure.
pub struct Agc { gain: f32 }
impl Agc {
    pub const TARGET_RMS: f32 = 0.08;
    pub const MAX_GAIN: f32 = 8.0;
    pub fn new() -> Self;                       // gain 1.0
    /// Scales `chunk` in place. The gain moves 5% of the way to TARGET_RMS / rms per voiced chunk,
    /// stays put on unvoiced chunks, and is clamped to 1.0..=MAX_GAIN. Output is clamped to -1..=1.
    pub fn process(&mut self, chunk: &mut [f32], voiced: bool);
    pub fn gain(&self) -> f32;
}

/// Speech or not, per chunk of 16 kHz audio. Silero when the model loads, else None (energy VAD is used).
pub struct SpeechGate { .. }
impl SpeechGate {
    pub fn open(model: &Path) -> Option<Self>;  // threshold 0.5, min_silence 0.1 s, min_speech 0.1 s, window 512, 1 thread
    /// Feeds the chunk; true while Silero reports speech. Drains finished segments so the buffer stays small.
    pub fn voiced(&mut self, pcm16k: &[f32]) -> bool;
}
```

  - `Vad::push` is split: `pub fn push_flag(&mut self, voiced: bool, loud: bool) -> VadEvent` holds the state machine
    (start after `START_FRAMES` voiced frames, quiet counting, pause end); `push(frame_rms)` computes the two flags
    from energy as today and calls it. Existing `vad_*` tests must pass unchanged.
  - `Engine` gains `pub vad_model: PathBuf` (`state.prompts_dir.join("models/silero_vad.onnx")`).
  - `stt::run` takes the path through `SttOptions { pub vad_model: Option<PathBuf>, .. }`, opens a `SpeechGate`
    once per answer, and per 20 ms frame calls `vad.push_flag(gate_voiced, gate_voiced)` when the gate exists, else
    `vad.push(level)`. The gate is fed the whole chunk before the frame loop; its result applies to that chunk's frames.
    `Agc::process` runs on each chunk (as f32) before the gate and before the samples are stored in the segment.

- [ ] **Step 1: Failing tests** (in `audio.rs`)

```rust
#[test]
fn agc_raises_quiet_speech_and_leaves_noise_alone() {
    let mut a = Agc::new();
    let quiet = tone(16_000, 300.0, 0.02);            // rms ~0.35 at amplitude 0.5
    let mut scaled: Vec<f32> = quiet.iter().map(|v| v * 0.05).collect(); // rms ~0.018
    for _ in 0..200 { let mut c = scaled.clone(); a.process(&mut c, true); }
    assert!(a.gain() > 3.0 && a.gain() <= Agc::MAX_GAIN, "{}", a.gain());
    let before = a.gain();
    a.process(&mut scaled, false);
    assert_eq!(a.gain(), before, "unvoiced chunks do not move the gain");
    let mut loud = vec![0.9f32; 320];
    Agc { gain: 8.0 }.process(&mut loud, true);
    assert!(loud.iter().all(|v| v.abs() <= 1.0));
}

#[test]
fn vad_flags_drive_the_same_state_machine() {
    let mut v = Vad::new(20, 100);
    assert_eq!(v.push_flag(true, true), VadEvent::None);
    assert_eq!(v.push_flag(true, true), VadEvent::None);
    assert_eq!(v.push_flag(true, true), VadEvent::SpeechStart);
    let ends: Vec<_> = (0..5).map(|_| v.push_flag(false, false)).collect();
    assert_eq!(ends.last(), Some(&VadEvent::SpeechEnd));
}

/// Review focus 5: typing and room noise are not speech.
#[test]
fn silero_ignores_clicks_and_noise() {
    let model = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts/models/silero_vad.onnx");
    let mut gate = SpeechGate::open(&model).expect("bundled model loads");
    // 3 s of low noise with a 5 ms click every 150 ms (a keyboard).
    let mut seed = 1u32;
    let mut pcm: Vec<f32> = (0..48_000).map(|_| { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); ((seed >> 16) as f32 / 32768.0 - 1.0) * 0.004 }).collect();
    for start in (0..48_000).step_by(2400) { for s in pcm[start..start + 80].iter_mut() { *s = 0.6; } }
    let voiced = pcm.chunks(320).filter(|c| gate.voiced(c)).count();
    assert_eq!(voiced, 0, "{voiced} chunks flagged as speech");
    assert!(SpeechGate::open(std::path::Path::new("no-such-file.onnx")).is_none());
}
```

- [ ] **Step 2:** Run `cargo test ... voice::audio`. Expected: compile errors.
- [ ] **Step 3:** Implement. If the click test flags a few chunks with the bundled model, raise the gate threshold
  in 0.05 steps up to 0.7; if it still fails, keep 0.5, mark the test `#[ignore]` with the measured count in a
  comment, and report it.
- [ ] **Step 4:** Run all tests. Expected: pass.
- [ ] **Step 5: Manual check.** Settings > Voice > mic test: speak softly, then type on the keyboard without
  speaking. Speech is heard; typing alone does not start an answer.
- [ ] **Step 6:** Commit `Voice: Silero VAD and gain control on the mic`.

### Task 16: sound-alike repair and aliases

**Files:**
- Create: `prompts/stt_aliases.json`
- Modify: `src-tauri/src/voice/stt.rs`, `engine.rs`

**Interfaces:**
- `pub struct Vocab { pub base: Vec<String>, pub round: Vec<String> }`; `SttOptions.vocab: Vocab`;
  `domain_vocab(extra)` returns `Vocab { base: TECH_TERMS, round: extra deduped against base }`.
- `pub struct Aliases` with `from_json(&str) -> Result<Aliases, String>`, `load(prompts_dir) -> Aliases` (empty on
  error, warning logged), `Default` = empty. File shape:
  `{"aliases":[{"heard":["reddish","red is"],"term":"Redis","needs":["cache","key","ttl"]}]}`.
  `needs` may be empty or missing.
- `pub fn fix_vocab(text: &str, vocab: &Vocab, aliases: &Aliases) -> String`;
  `pub fn clean_transcript(text: &str, vocab: &Vocab, aliases: &Aliases) -> String`.
- `Engine` gains `pub aliases: Arc<Aliases>`; `SttOptions` carries it.
- **Matching rules, in this order per window of 1-3 words:**
  1. Exact match after `norm` (today's behaviour) against base and round terms.
  2. Alias: the window's words, lowercased and joined with single spaces, equal a `heard` entry, and (`needs` is
     empty, or any `needs` word occurs anywhere in the text, case-insensitive, or `term` is in `vocab.round`).
  3. Spelling distance (today's behaviour) with `allowed_edits`. Round terms get one more edit:
     `allowed_edits_round(len)`: 0-4 -> 0, 5-7 -> 1, 8+ -> 3. First and last letters must still match.
  4. Sound key: only for terms whose `norm` length is at least 8. `sound_key(window) == sound_key(term)`, same first
     letter. A window made only of words in `EVERYDAY` (the 300 most common English words; embed as a const list)
     never matches by rule 3 or 4.
- `pub fn sound_key(word: &str) -> String`: lowercase letters only; then, left to right: `ph` -> `f`, `ck` -> `k`,
  `sh` -> `s`, `ch` -> `k`, `c` before `e`, `i`, `y` -> `s`, other `c` -> `k`, `q` -> `k`, `x` -> `ks`, `z` -> `s`,
  `d` -> `t`, `b` -> `p`, `g` -> `k`, `v` -> `f`; drop `h`, `w`, `y` and all vowels except a leading one; collapse
  repeated letters.

**`prompts/stt_aliases.json` starter content:**

```json
{ "aliases": [
  { "heard": ["reddish", "red is", "readies", "red dis"], "term": "Redis", "needs": ["cache", "key", "ttl", "memory", "cluster", "store", "pub", "lua"] },
  { "heard": ["postgres creole", "post grass", "post gress", "postgres ql", "post gray sql"], "term": "PostgreSQL" },
  { "heard": ["cafca", "kafta", "cough ka"], "term": "Kafka" },
  { "heard": ["g r p c", "grpc", "gr pc"], "term": "gRPC" },
  { "heard": ["jay w t", "j w t", "jot"], "term": "JWT", "needs": ["token", "auth", "claim", "signed"] },
  { "heard": ["o auth", "oh auth", "oath"], "term": "OAuth", "needs": ["token", "login", "scope", "auth"] },
  { "heard": ["cube ernetes", "cooper netties"], "term": "Kubernetes" },
  { "heard": ["dynamo db", "dynamo d b"], "term": "DynamoDB" },
  { "heard": ["mongo db", "mongo d b"], "term": "MongoDB" },
  { "heard": ["rabbit mq", "rabbit m q"], "term": "RabbitMQ" },
  { "heard": ["graph ql", "graph q l"], "term": "GraphQL" },
  { "heard": ["elastic search"], "term": "Elasticsearch" }
] }
```

  Keep only mishearings that are not ordinary English on their own, or give them `needs`. After Task 3b, add the
  mishearings the bench actually produced.

- [ ] **Step 1: Failing tests** (in `stt.rs`; update the existing `fix_vocab` and `clean_transcript` tests to the
  new signatures with `&Aliases::default()`, they must still pass)

```rust
fn aliases() -> Aliases {
    Aliases::from_json(include_str!("../../../prompts/stt_aliases.json")).unwrap()
}

#[test]
fn sound_keys() {
    assert_eq!(sound_key("Kubernetes"), sound_key("cubernetties"));
    assert_eq!(sound_key("Elasticsearch"), sound_key("elastic surch"));
    assert_ne!(sound_key("Cassandra"), sound_key("Kubernetes"));
}

#[test]
fn aliases_fix_short_terms_only_in_context() {
    let v = domain_vocab(&[]);
    let a = aliases();
    assert_eq!(fix_vocab("we cache it in reddish with a ttl", &v, &a), "we cache it in Redis with a ttl");
    assert_eq!(fix_vocab("the sky was reddish at dusk", &v, &a), "the sky was reddish at dusk");
    assert_eq!(fix_vocab("stored in postgres creole tables", &v, &a), "stored in PostgreSQL tables");
}

#[test]
fn round_terms_match_more_loosely() {
    let v = domain_vocab(&["Juspay", "Prateesh", "Hazelcast"]);
    let a = Aliases::default();
    assert_eq!(fix_vocab("I worked at juspe for two years", &v, &a), "I worked at Juspay for two years");
    assert_eq!(fix_vocab("my name is pratish", &v, &a), "my name is Prateesh");
    assert_eq!(fix_vocab("a hazel cast cluster", &v, &a), "a Hazelcast cluster");
}

#[test]
fn ordinary_english_is_left_alone() {
    let v = domain_vocab(&["Juspay", "Prateesh", "Karthik", "Razorpay"]);
    let a = aliases();
    for s in [
        "I just pay attention to the reads and the rates",
        "we read this data from the other team and wrote it back",
        "there is a trade off between speed and the cost of the read",
        "I would post the results and press on with the next part",
        "the service was down for an hour and we were late",
    ] {
        assert_eq!(fix_vocab(s, &v, &a), s);
    }
}
```

  Decided trade-off: a window made only of everyday words ("just pay") is never turned into a term, even when it
  is what the recognizer heard for "Juspay". Wrongly rewriting ordinary speech is worse than missing one name; the
  interviewer prompt (Task 14) covers that case.

- [ ] **Step 2:** Run `cargo test ... voice::stt`. Expected: compile errors.
- [ ] **Step 3:** Implement. Thread `Vocab` and `Aliases` through `listen_with`, `run` and `listen_once`.
- [ ] **Step 4:** Run all tests. Expected: pass.
- [ ] **Step 5: Gate.** `voice_bench stt --engine parakeet --source synthetic` and `--source recorded` (if
  recordings exist), with the bench applying `clean_transcript` with each item's terms as round terms. Record under
  "After Task 16". Pass = term recall at or above 90%, or 15 points above the Task 3 baseline, whichever is higher,
  and WER not worse than baseline.
- [ ] **Step 6:** Commit `Voice: sound-alike repair and known mishearings`.

### Task 17: biased decoding (gated, may end as "not yet")

**Files:** `docs/engineering/VOICE.md`; code only if the gate opens.

- [ ] **Step 1:** Check `gh release list -R k2-fsa/sherpa-onnx --limit 5` and whether PR 4084
  ("Fix modified_beam_search dropping speech for NeMo TDT models") is in a released version, and whether crates.io
  has a `sherpa-onnx` crate newer than 2.0.1 built from it.
- [ ] **Step 2:** If not released: write one line in `VOICE.md` Gaps ("Decoder hotwords for Parakeet wait for a
  sherpa-onnx release with PR 4084, checked <date>") and finish this task. Commit `Docs: hotwords gate checked`.
- [ ] **Step 3:** If released: bump the crate, rebuild, re-run the full bench to confirm nothing moved. Add a
  `parakeet-hot` engine to `voice_bench` (`decoding_method = "modified_beam_search"`, `max_active_paths = 4`,
  `model_config.modeling_unit = "bpe"`, `bpe_vocab` exported from the model per the sherpa-onnx hotwords doc,
  `create_stream_with_hotwords` with the context terms uppercased or cased as the tokenizer expects, score 1.5).
  Run `stt` and `silence` 3 times each. Adopt it for final segment decodes only if term recall is higher than
  Task 16's, WER is not worse, and no run returns empty or shorter-than-half text; in the app, keep the greedy
  result as the fallback when the biased text is empty or under half the greedy length. Otherwise record the
  numbers and leave the code out.

### Task 18: adopt the recognizer the owner picked (only after Task 3b's decision)

**Files:** `src-tauri/src/voice/builtin.rs`, `models.rs`, `engine.rs`, `stt.rs`, `src-tauri/src/interview/live.rs`
(loader text), `ui/index.html` (Privacy and model names), docs.

Do this task only if the owner chose a candidate. If Parakeet v2 stays, skip it.

- **Orukeet chosen:** it is a pin swap. Add `pub const ORUKEET: Pin` with the URL, bytes, SHA-256 and files recorded
  in the results doc; `PINS = [SUPERTONIC, ORUKEET]`; `ParakeetStt::load_with` reads its folder; add
  `RETIRED_PARAKEET` handled like `RETIRED_POCKET` (deleted after the new model installs). If Orukeet writes numbers
  as words, add a numbers-to-digits pass before `fix_vocab` for the codes in `TECH_TERMS` (`MT103`, `ISO 20022`,
  `pacs.008`), with tests. Existing installs: `all_installed` is false until the new model is downloaded, so the
  first-run nudge appears; the loader message stays "Not downloaded yet. Settings > Voice > Download."
- **Qwen3-ASR chosen:** it becomes the final-text model on `Tier::Fast` and `Tier::Ok`; Parakeet stays for captions
  and for `Tier::Slow`. Add `pub const QWEN3: Pin` as an optional download (not in `all_installed`), a
  `FinalStt` struct in `builtin.rs` holding one recognizer per round built with that round's vocabulary as
  `hotwords`, loaded at round start after the tier is known, and `spawn_decode` uses it when present. Parakeet
  remains the fallback when the optional model is missing or fails to load. Settings > Voice lists it as
  "Better recognition of names and terms (optional, <size>)".
- Tests: pin table test (ids unique, URLs on github.com or huggingface.co only if `allowed_host` is extended, which
  needs the owner's explicit yes), fallback test, and a `voice_bench stt` run recorded as "After Task 18".
- Commit `Voice: <model> for final answer text`.

---

# Part 5: finish

### Task 19: docs and release

**Files:** `docs/engineering/VOICE.md`, `CLAUDE.md`, `docs/plans/voice-pipeline-rebuild.md`, `ui/index.html` (Privacy).

- [ ] **Step 1:** Rewrite the affected `VOICE.md` sections (file table, TTS, STT, Speaking, Listening, Change
  recipes, Gaps) to describe what now exists. Keep it terse. Remove the gaps this plan closed.
- [ ] **Step 2:** `CLAUDE.md` voice section: mention the normalizer + lexicon, tiers, Silero VAD, round vocabulary,
  and `voice_bench`. Two or three lines.
- [ ] **Step 3:** Privacy text in `ui/index.html`: add one sentence only if true and not already covered:
  "Words from your resume and the job description are used on this computer to help recognise what you say."
- [ ] **Step 4:** Set the status line of `voice-pipeline-rebuild.md` to built, with the date and the final gate
  numbers from the results doc.
- [ ] **Step 5:** `graphify update .`; run clippy and all tests; commit `Docs: voice pipeline rebuild`.
- [ ] **Step 6:** Do not tag or push a release. Tell the owner the branch is ready for the experimental channel.

## Stop points (ask the owner, do not guess)

| Where | Why |
|---|---|
| Start | Uncommitted files on the branch |
| Task 3b Step 4 | Recognizer pick |
| Task 7 Step 3 | Voice pick by ear |
| Task 7 Step 5, Task 11 Step 5 | A gate failed: fallback voice bake-off |
| Task 18 | Only with the owner's pick; extending `allowed_host` needs an explicit yes |
