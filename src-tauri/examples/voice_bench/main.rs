//! Voice bench: measures the speech pipeline on fixed sets, so every change is compared with numbers.
//!
//! ```text
//! voice_bench tts   [--models <root>] [--set tech|lines|all] [--sid <n>] [--speed <f>] [--steps <n>]
//!                   [--threads <n>] [--runs <n>] [--raw] [--save-wav]
//! voice_bench sweep [--models <root>] [--threads <n>] [--save-wav]
//! voice_bench stt   --engine parakeet|orukeet|qwen3 [--models <root>] [--source synthetic|recorded]
//!                   [--context] [--threads <n>]
//! voice_bench silence --engine parakeet|orukeet|qwen3 [--models <root>]
//! voice_bench facts [--models <root>]   (Supertonic splitting, idle CPU after a synthesis)
//! ```
//! Models root default `D:\cargo-target\voice-bench\models` (layout `<root>/<pin id>/`, plus `orukeet/` and
//! `qwen3-asr-0.6b/`). Output: `D:\cargo-target\voice-bench\out`. Results: `docs/plans/voice-bench-results.md`.
//! Old laptop mode: `--threads 2` and run the exe with `start /affinity 0x3`.

mod engines;

use onsite_lib::voice::builtin::{self, ParakeetStt, Supertonic, Synth};
use onsite_lib::voice::speech_text::{to_speech, Lexicon};
use onsite_lib::voice::{score, stt};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

const STT_RATE: i32 = 16_000;

struct Args(Vec<String>);

impl Args {
    fn get(&self, k: &str) -> Option<String> {
        self.0.iter().position(|a| a == k).and_then(|i| self.0.get(i + 1)).cloned()
    }
    fn has(&self, k: &str) -> bool {
        self.0.iter().any(|a| a == k)
    }
    fn num<T: std::str::FromStr>(&self, k: &str, default: T) -> T {
        self.get(k).and_then(|v| v.parse().ok()).unwrap_or(default)
    }
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice")
}

/// (text to speak, expected spoken form).
fn tts_set(set: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if set == "tech" || set == "all" {
        let tsv = std::fs::read_to_string(fixtures().join("tts_tech_lines.tsv")).expect("tts_tech_lines.tsv");
        for row in tsv.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
            let (w, s) = row.split_once('\t').expect("two columns");
            out.push((w.to_string(), s.to_string()));
        }
    }
    if set == "lines" || set == "all" {
        let txt = std::fs::read_to_string(fixtures().join("tts_lines.txt")).expect("tts_lines.txt");
        out.extend(txt.lines().filter(|l| !l.trim().is_empty()).map(|l| (l.to_string(), l.to_string())));
    }
    out
}

/// Nearest-rank percentile of an unsorted list; 0 when empty.
fn pct(v: &[f32], p: f32) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let rank = ((p / 100.0) * s.len() as f32).ceil().max(1.0) as usize;
    s[rank.min(s.len()) - 1]
}

fn to_16k(pcm: &[f32], rate: u32) -> Vec<f32> {
    if rate as i32 == STT_RATE {
        return pcm.to_vec();
    }
    sherpa_onnx::LinearResampler::create(rate as i32, STT_RATE).expect("resampler").resample(pcm, true)
}

fn unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn save_json(out: &Path, prefix: &str, v: &Value) {
    std::fs::create_dir_all(out).unwrap();
    let p = out.join(format!("{prefix}-{}.json", unix()));
    std::fs::write(&p, serde_json::to_string_pretty(v).unwrap()).unwrap();
    println!("saved {}", p.display());
}

/// The text the voice is given for `written`: as written with `--raw`, else through the normalizer and the
/// real lexicon, as in a round.
fn speech_text(written: &str, raw: bool) -> String {
    static LEX: std::sync::LazyLock<Lexicon> =
        std::sync::LazyLock::new(|| Lexicon::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../prompts")));
    if raw {
        written.to_string()
    } else {
        to_speech(written, &LEX)
    }
}

/// Word error rate with both sides in spoken form, so "10,000" and "ten thousand" count as the same words.
/// Recognizers differ in how they write numbers; that is formatting, not hearing.
fn spoken_wer(reference: &str, heard: &str) -> score::Wer {
    let empty = Lexicon::default();
    score::wer(&to_speech(reference, &empty), &to_speech(heard, &empty))
}

struct Line {
    wer: f32,
    missing: Vec<String>,
    audio_s: f32,
    gen_s: f32,
    sec_per_word: f32,
}

struct Bench {
    tts: Supertonic,
    stt: ParakeetStt,
}

impl Bench {
    fn speak_and_score(&self, text: &str, reference: &str, synth: &Synth) -> (Line, Vec<f32>, String) {
        let stop = Arc::new(AtomicBool::new(false));
        let t = Instant::now();
        let pcm = self.tts.synth(text, synth, &stop).expect("synth");
        let gen_s = t.elapsed().as_secs_f32();
        let audio_s = pcm.len() as f32 / self.tts.rate as f32;
        let heard = self.stt.transcribe_f32(&to_16k(&pcm, self.tts.rate));
        let w = spoken_wer(reference, &heard);
        let nwords = score::words(reference).len().max(1);
        let line = Line { wer: w.rate(), missing: w.missing, audio_s, gen_s, sec_per_word: audio_s / nwords as f32 };
        (line, pcm, heard)
    }
}

fn load(root: &Path, threads: i32) -> Bench {
    let t = Instant::now();
    let tts = Supertonic::load_with(root, threads).expect("load Supertonic");
    let stt = ParakeetStt::load_with(root, threads).expect("load Parakeet");
    println!("models loaded in {} ms, {threads} threads", t.elapsed().as_millis());
    Bench { tts, stt }
}

struct Summary {
    mean_wer: f32,
    missing_lines: usize,
    rtf: Vec<f32>,
    min_spw: f32,
    words_per_s: f32,
}

fn summarize(lines: &[Line]) -> Summary {
    let rtf: Vec<f32> = lines.iter().map(|l| l.gen_s / l.audio_s.max(0.01)).collect();
    let audio: f32 = lines.iter().map(|l| l.audio_s).sum();
    let words: f32 = lines.iter().map(|l| l.audio_s / l.sec_per_word.max(1e-6)).sum();
    Summary {
        mean_wer: lines.iter().map(|l| l.wer).sum::<f32>() / lines.len().max(1) as f32,
        missing_lines: lines.iter().filter(|l| !l.missing.is_empty()).count(),
        rtf,
        min_spw: lines.iter().map(|l| l.sec_per_word).fold(f32::INFINITY, f32::min),
        words_per_s: words / audio.max(0.01),
    }
}

fn run_tts(a: &Args, root: &Path, out: &Path, threads: i32) {
    let set = a.get("--set").unwrap_or_else(|| "all".into());
    let synth = Synth { sid: a.num("--sid", 5), speed: a.num("--speed", 1.0), steps: a.num("--steps", 8) };
    let runs: usize = a.num("--runs", 1);
    let raw = a.has("--raw");
    let b = load(root, threads);
    let items = tts_set(&set);
    let mut lines = Vec::new();
    let mut rows = Vec::new();
    for run in 0..runs {
        for (i, (written, spoken)) in items.iter().enumerate() {
            let text = speech_text(written, raw);
            let (l, pcm, heard) = b.speak_and_score(&text, spoken, &synth);
            println!(
                "{run}.{i:02} wer {:.2} rtf {:.2} {:.2}s/w audio {:.1}s missing {:?}",
                l.wer,
                l.gen_s / l.audio_s.max(0.01),
                l.sec_per_word,
                l.audio_s,
                l.missing
            );
            if a.has("--save-wav") {
                let dir = out.join("tts");
                std::fs::create_dir_all(&dir).unwrap();
                sherpa_onnx::write(&dir.join(format!("sid{}_{i:02}.wav", synth.sid)).to_string_lossy(), &pcm, b.tts.rate as i32);
            }
            rows.push(json!({"run": run, "line": i, "text": text, "heard": heard, "wer": l.wer, "missing": l.missing,
                "audio_s": l.audio_s, "gen_s": l.gen_s, "rtf": l.gen_s / l.audio_s.max(0.01), "sec_per_word": l.sec_per_word}));
            lines.push(l);
        }
    }
    let s = summarize(&lines);
    println!(
        "SUMMARY set={set} sid={} speed={} steps={} threads={threads} runs={runs} raw={raw}: mean WER {:.2}%, lines with a missing word {}/{}, RTF p50 {:.2} p90 {:.2} max {:.2}, min s/word {:.3}, words/s {:.2}",
        synth.sid,
        synth.speed,
        synth.steps,
        s.mean_wer * 100.0,
        s.missing_lines,
        lines.len(),
        pct(&s.rtf, 50.0),
        pct(&s.rtf, 90.0),
        pct(&s.rtf, 100.0),
        s.min_spw,
        s.words_per_s
    );
    save_json(out, "tts", &json!({"args": a.0, "mean_wer": s.mean_wer, "missing_lines": s.missing_lines, "lines": lines.len(),
        "rtf_p50": pct(&s.rtf, 50.0), "rtf_p90": pct(&s.rtf, 90.0), "rtf_max": pct(&s.rtf, 100.0), "min_sec_per_word": s.min_spw,
        "words_per_s": s.words_per_s, "rows": rows}));
}

fn run_sweep(a: &Args, root: &Path, out: &Path, threads: i32) {
    const SPEEDS: [f32; 5] = [0.9, 1.0, 1.1, 1.2, 1.3];
    let raw = a.has("--raw");
    let b = load(root, threads);
    let items = tts_set("all");
    let listen_line = std::fs::read_to_string(fixtures().join("tts_lines.txt")).unwrap().lines().nth(3).unwrap_or("Hello.").to_string();
    let mut grid = Vec::new();
    for sid in 0..=9 {
        let mut row = Vec::new();
        for speed in SPEEDS {
            let synth = Synth { sid, speed, steps: 8 };
            let lines: Vec<Line> = items.iter().map(|(w, s)| b.speak_and_score(&speech_text(w, raw), s, &synth).0).collect();
            let s = summarize(&lines);
            println!("sid {sid} speed {speed}: missing {} mean WER {:.2}% words/s {:.2}", s.missing_lines, s.mean_wer * 100.0, s.words_per_s);
            row.push(json!({"speed": speed, "missing_lines": s.missing_lines, "mean_wer": s.mean_wer, "words_per_s": s.words_per_s}));
        }
        if a.has("--save-wav") {
            let dir = out.join("listen");
            std::fs::create_dir_all(&dir).unwrap();
            let pcm = b.tts.synth(&listen_line, &Synth { sid, speed: 1.0, steps: 8 }, &Arc::new(AtomicBool::new(false))).unwrap();
            sherpa_onnx::write(&dir.join(format!("sid{sid}.wav")).to_string_lossy(), &pcm, b.tts.rate as i32);
        }
        grid.push(json!({"sid": sid, "speeds": row}));
    }
    println!("\nlines with a missing word (rows sid, columns speed {SPEEDS:?}); words/s at 1.0");
    for g in &grid {
        let cells: Vec<String> = g["speeds"].as_array().unwrap().iter().map(|c| format!("{:>3}", c["missing_lines"])).collect();
        println!("sid {}: {}   {:.2} w/s", g["sid"], cells.join(" "), g["speeds"][1]["words_per_s"].as_f64().unwrap_or(0.0));
    }
    save_json(out, "sweep", &json!({"args": a.0, "lines": items.len(), "grid": grid}));
}

/// (id, audio at 16 kHz, reference text, terms).
fn stt_items(a: &Args, root: &Path, threads: i32) -> Vec<(String, Vec<f32>, String, Vec<String>)> {
    let source = a.get("--source").unwrap_or_else(|| "synthetic".into());
    let mut out = Vec::new();
    if source == "recorded" {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(fixtures().join("answers.json")).unwrap()).unwrap();
        let mut skipped = 0;
        for ans in v["answers"].as_array().unwrap() {
            let id = ans["id"].as_str().unwrap_or_default();
            let text = ans["text"].as_str().unwrap_or_default();
            let wav = fixtures().join("answers").join(format!("{id}.wav"));
            if text.is_empty() || !wav.exists() {
                skipped += 1;
                continue;
            }
            let w = sherpa_onnx::Wave::read(&wav.to_string_lossy()).expect("read wav");
            assert_eq!(w.sample_rate(), STT_RATE, "{id}: expected 16 kHz");
            let terms = ans["terms"].as_array().map(|t| t.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
            out.push((id.to_string(), w.samples().to_vec(), text.to_string(), terms));
        }
        println!("recorded: {} answers, {skipped} skipped (no text or no wav)", out.len());
    } else {
        let tts = Supertonic::load_with(root, threads).expect("load Supertonic");
        let tsv = std::fs::read_to_string(fixtures().join("stt_terms.tsv")).unwrap();
        for (i, row) in tsv.lines().filter(|l| l.contains('\t')).enumerate() {
            let (sentence, terms) = row.split_once('\t').unwrap();
            let terms: Vec<String> = terms.split(',').map(|t| t.trim().to_string()).collect();
            for sid in [0, 5] {
                let pcm = tts.synth(sentence, &Synth { sid, speed: 1.0, steps: 8 }, &Arc::new(AtomicBool::new(false))).unwrap();
                out.push((format!("{i:02}-sid{sid}"), to_16k(&pcm, tts.rate), sentence.to_string(), terms.clone()));
            }
        }
    }
    out
}

fn run_stt(a: &Args, root: &Path, out: &Path, threads: i32) {
    let engine = a.get("--engine").unwrap_or_else(|| "parakeet".into());
    let items = stt_items(a, root, threads);
    let t = Instant::now();
    let rec = engines::open(&engine, root, threads).unwrap_or_else(|e| panic!("{engine}: {e}"));
    println!("{} loaded in {} ms", rec.name(), t.elapsed().as_millis());
    let use_ctx = a.has("--context");
    let (mut wers, mut decode, mut rtfs, mut found, mut total) = (Vec::new(), Vec::new(), Vec::new(), 0, 0);
    let mut rows = Vec::new();
    for (id, pcm, reference, terms) in &items {
        let ctx: Vec<String> = if use_ctx {
            terms.iter().cloned().chain(stt::TECH_TERMS.iter().map(|s| s.to_string())).collect()
        } else {
            Vec::new()
        };
        let t = Instant::now();
        let heard = rec.decode(pcm, &ctx);
        let d = t.elapsed().as_secs_f32();
        let audio_s = pcm.len() as f32 / STT_RATE as f32;
        let w = spoken_wer(reference, &heard);
        // Score only terms the reference itself contains: a label like "Audio Confirmation" was never said.
        let said: Vec<String> = terms.iter().filter(|t| score::term_recall(reference, std::slice::from_ref(t)).0 == 1).cloned().collect();
        let (f, n) = score::term_recall(&heard, &said);
        found += f;
        total += n;
        println!("{id}: wer {:.2} terms {f}/{n} decode {d:.2}s audio {audio_s:.1}s\n  heard: {heard}", w.rate());
        rows.push(json!({"id": id, "heard": heard, "wer": w.rate(), "missing": w.missing, "terms_found": f, "terms": n, "decode_s": d, "audio_s": audio_s}));
        wers.push(w.rate());
        decode.push(d);
        rtfs.push(d / audio_s.max(0.01));
    }
    let mean = wers.iter().sum::<f32>() / wers.len().max(1) as f32;
    let recall = found as f32 / total.max(1) as f32;
    println!(
        "SUMMARY engine={engine} context={use_ctx} threads={threads} items={}: mean WER {:.2}%, term recall {found}/{total} ({:.0}%), decode p50 {:.2}s p90 {:.2}s, RTF p50 {:.2}",
        items.len(),
        mean * 100.0,
        recall * 100.0,
        pct(&decode, 50.0),
        pct(&decode, 90.0),
        pct(&rtfs, 50.0)
    );
    save_json(out, "stt", &json!({"args": a.0, "engine": engine, "mean_wer": mean, "terms_found": found, "terms_total": total,
        "decode_p50": pct(&decode, 50.0), "decode_p90": pct(&decode, 90.0), "rows": rows}));
}

fn run_silence(a: &Args, root: &Path, threads: i32) {
    let engine = a.get("--engine").unwrap_or_else(|| "parakeet".into());
    let rec = engines::open(&engine, root, threads).unwrap_or_else(|e| panic!("{engine}: {e}"));
    let zeros = vec![0.0f32; 5 * STT_RATE as usize];
    let mut seed = 12345u32;
    let noise: Vec<f32> = (0..5 * STT_RATE as usize)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            // Uniform in -1..1 has RMS 1/sqrt(3); scale to RMS 0.005.
            ((seed >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0) * 0.005 * 3f32.sqrt()
        })
        .collect();
    let z = rec.decode(&zeros, &[]);
    let n = rec.decode(&noise, &[]);
    println!("silence: {z:?}\nnoise:   {n:?}");
    println!("SUMMARY engine={engine} silence {}", if z.trim().is_empty() && n.trim().is_empty() { "PASS" } else { "FAIL" });
}

/// Idle CPU after a synthesis: the process idles 8 s so its CPU use can be sampled from outside.
/// (Fact 1, whether sherpa splits Supertonic input, was measured with the old callback path: one call, no split.)
fn run_facts(root: &Path, threads: i32) {
    let tts = Supertonic::load_with(root, threads).expect("load Supertonic");
    let text = "The first sentence is about caching. The second one is about queues and retries.";
    tts.synth(text, &Synth { sid: 5, speed: 1.0, steps: 8 }, &Arc::new(AtomicBool::new(false))).expect("synth");
    println!("IDLE START pid {}", std::process::id());
    std::thread::sleep(std::time::Duration::from_secs(8));
    println!("IDLE END");
}

fn main() {
    let a = Args(std::env::args().skip(1).collect());
    let root = a.get("--models").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"D:\cargo-target\voice-bench\models"));
    let out = PathBuf::from(r"D:\cargo-target\voice-bench\out");
    let threads: i32 = a.num("--threads", builtin::threads());
    if threads == 2 {
        println!("old laptop mode: also run the exe as `start /affinity 0x3 voice_bench.exe ...` to pin it to 2 cores");
    }
    match a.0.first().map(String::as_str) {
        Some("tts") => run_tts(&a, &root, &out, threads),
        Some("sweep") => run_sweep(&a, &root, &out, threads),
        Some("stt") => run_stt(&a, &root, &out, threads),
        Some("silence") => run_silence(&a, &root, threads),
        Some("facts") => run_facts(&root, threads),
        _ => eprintln!("usage: voice_bench tts|sweep|stt|silence [options] (see the top of examples/voice_bench/main.rs)"),
    }
}
