//! Phase 0 spike for `docs/plans/voice-without-voicestudio.md`: speech models run in-process through
//! sherpa-onnx, measured on the fixed test set. The app is not changed.
//!
//! Models live in `target/voice-spike/models` (see `docs/plans/voice-spike-results.md` for the exact files),
//! persona clips in `target/voice-spike/personas/<persona_id>.wav`. Output goes to `target/voice-spike/out`.
//!
//! ```text
//! cargo run --release --example voice_spike -- record <answer id>
//! cargo run --release --example voice_spike -- tts <zipvoice|pocket> [--personas a,b|all] [--runs 2] [--threads 4] [--steps 4] [--provider cpu]
//! cargo run --release --example voice_spike -- stt <parakeet|nemo480|nemotron> [--threads 4] [--provider cpu]
//! ```

use onsite_lib::voice::audio::{i16_to_f32, rms, Mic, Vad, VadEvent, MIC_RATE};
use onsite_lib::voice::stt::{clean_transcript, domain_vocab};
use serde_json::{json, Value};
use sherpa_onnx::{
    GenerationConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig, OfflineTts, OfflineTtsConfig,
    OfflineTtsModelConfig, OfflineTtsPocketModelConfig, OfflineTtsZipvoiceModelConfig, OnlineRecognizer, OnlineRecognizerConfig,
    OnlineTransducerModelConfig, Wave,
};
use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Instant;

const PERSONAS: [&str; 10] = [
    "arjun_mehta",
    "neha_kapoor",
    "karthik_reddy",
    "vikram_desai",
    "sameer_khanna",
    "ethan_brooks",
    "olivia_chen",
    "marcus_reed",
    "priya_shah",
    "daniel_foster",
];
/// The 4-persona early check from the plan.
const EARLY: [&str; 4] = ["arjun_mehta", "neha_kapoor", "ethan_brooks", "priya_shah"];

// Segmenting, copied from `voice/stt.rs` so the spike cuts answers where the app does.
const FRAME: usize = (MIC_RATE / 50) as usize;
const PREROLL: usize = (MIC_RATE as usize * 400) / 1000;
const SEG_MIN: usize = MIC_RATE as usize * 3;
const SEG_LONG: usize = MIC_RATE as usize * 20;
const SEG_PAUSE_MS: u32 = 700;
const SEG_PAUSE_LONG_MS: u32 = 300;
/// Option A: how often the open segment is re-decoded for a caption.
const CAPTION_EVERY: usize = MIC_RATE as usize / 2;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/voice-spike")
}

/// The test set. `VOICE_SPIKE_SET=<dir>` points `stt` at another `answers.json` + `answers/` (smoke tests).
fn fixtures() -> PathBuf {
    std::env::var_os("VOICE_SPIKE_SET").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/voice"))
}

fn file(dir: &str, file: &str) -> Option<String> {
    let p = root().join("models").join(dir).join(file);
    assert!(p.exists(), "missing model file {}", p.display());
    Some(p.to_string_lossy().into_owned())
}

struct Opts {
    threads: i32,
    provider: String,
    runs: usize,
    steps: i32,
    personas: Vec<String>,
    /// Pocket tuning (spike): `--fp` full precision, `--temp`, `--seed`, `--reflen`, `--lines 4,7,..`, `--refdir <dir>` (clips `<id>.wav`), `--tag <out subfolder>`.
    fp: bool,
    temp: Option<f64>,
    seed: i64,
    reflen: f64,
    lines: Option<Vec<usize>>,
    refdir: Option<String>,
    tag: Option<String>,
}

fn opts(args: &[String]) -> Opts {
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let personas = match get("--personas").as_deref() {
        None => EARLY.iter().map(|s| s.to_string()).collect(),
        Some("all") => PERSONAS.iter().map(|s| s.to_string()).collect(),
        Some(list) => list.split(',').map(str::to_string).collect(),
    };
    Opts {
        threads: get("--threads").map_or(4, |v| v.parse().expect("--threads")),
        provider: get("--provider").unwrap_or_else(|| "cpu".into()),
        runs: get("--runs").map_or(1, |v| v.parse().expect("--runs")),
        steps: get("--steps").map_or(0, |v| v.parse().expect("--steps")),
        personas,
        fp: args.iter().any(|a| a == "--fp"),
        temp: get("--temp").map(|v| v.parse().expect("--temp")),
        seed: get("--seed").map_or(42, |v| v.parse().expect("--seed")),
        reflen: get("--reflen").map_or(12.0, |v| v.parse().expect("--reflen")),
        lines: get("--lines").map(|v| v.split(',').map(|x| x.parse().expect("--lines")).collect()),
        refdir: get("--refdir"),
        tag: get("--tag"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("record"), Some(id)) => record(id),
        (Some("tts"), Some(engine)) => tts(engine, &opts(&args)),
        (Some("stt"), Some(engine)) => stt(engine, &opts(&args)),
        (Some("score"), Some(dir)) => score(dir, &args),
        _ => {
            eprintln!("usage: voice_spike record <id> | tts <zipvoice|pocket> [opts] | stt <parakeet|nemo480|nemotron> [opts]");
            std::process::exit(2);
        }
    }
}

// ---------- record ----------

/// Records one answer from the real mic, through the app's own capture and 16 kHz downsampler.
fn record(id: &str) {
    let answers: Value = serde_json::from_str(&std::fs::read_to_string(fixtures().join("answers.json")).expect("answers.json")).expect("json");
    let prompt = answers["answers"].as_array().unwrap().iter().find(|a| a["id"] == id).map(|a| a["prompt"].as_str().unwrap().to_string());
    match prompt {
        Some(p) => println!("Prompt: {p}"),
        None => println!("(id '{id}' is not in answers.json; recording anyway)"),
    }
    print!("Press Enter to start, then Enter again to stop. ");
    std::io::stdout().flush().ok();
    std::io::stdin().read_line(&mut String::new()).ok();

    let rt = tokio::runtime::Runtime::new().unwrap();
    let pcm = rt.block_on(async {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<i16>>();
        let mic = Mic::open(tx).expect("open mic");
        println!("Recording...");
        let stop = tokio::task::spawn_blocking(|| std::io::stdin().read_line(&mut String::new()));
        tokio::pin!(stop);
        let mut pcm: Vec<i16> = Vec::new();
        loop {
            tokio::select! {
                _ = &mut stop => break,
                c = rx.recv() => match c { Some(c) => pcm.extend(c), None => break },
            }
        }
        drop(mic);
        while let Ok(c) = rx.try_recv() {
            pcm.extend(c);
        }
        pcm
    });
    let dir = fixtures().join("answers");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{id}.wav"));
    assert!(sherpa_onnx::write(&path.to_string_lossy(), &i16_to_f32(&pcm), MIC_RATE as i32), "write wav");
    println!("Saved {} ({:.1} s, peak level {:.2})", path.display(), pcm.len() as f32 / MIC_RATE as f32, rms(&i16_to_f32(&pcm)));
    println!("Now type what you said into \"text\" for '{id}' in answers.json.");
}

// ---------- tts ----------

fn tts(engine: &str, o: &Opts) {
    let mut model = OfflineTtsModelConfig { num_threads: o.threads, provider: Some(o.provider.clone()), ..Default::default() };
    match engine {
        "zipvoice" => {
            let z = "sherpa-onnx-zipvoice-distill-int8-zh-en-emilia";
            model.zipvoice = OfflineTtsZipvoiceModelConfig {
                tokens: file(z, "tokens.txt"),
                encoder: file(z, "encoder.int8.onnx"),
                decoder: file(z, "decoder.int8.onnx"),
                vocoder: file("shared", "vocos_24khz.onnx"),
                lexicon: Some(format!("{},{}", file(z, "lexicon.txt").unwrap(), file("shared", "lexicon-en-us.txt").unwrap())),
                feat_scale: 0.1,
                t_shift: 0.5,
                target_rms: 0.1,
                guidance_scale: 1.0,
                ..Default::default()
            }
        }
        "pocket" => {
            let (p, q) = if o.fp { ("sherpa-onnx-pocket-tts-2026-01-26", "") } else { ("sherpa-onnx-pocket-tts-int8-2026-01-26", ".int8") };
            model.pocket = OfflineTtsPocketModelConfig {
                lm_flow: file(p, &format!("lm_flow{q}.onnx")),
                lm_main: file(p, &format!("lm_main{q}.onnx")),
                encoder: file(p, "encoder.onnx"),
                decoder: file(p, &format!("decoder{q}.onnx")),
                text_conditioner: file(p, "text_conditioner.onnx"),
                vocab_json: file(p, "vocab.json"),
                token_scores_json: file(p, "token_scores.json"),
                voice_embedding_cache_capacity: 16,
            }
        }
        other => panic!("unknown tts engine {other}"),
    }
    let t0 = Instant::now();
    let tts = OfflineTts::create(&OfflineTtsConfig { model, ..Default::default() }).expect("create tts");
    let load_ms = t0.elapsed().as_millis();
    println!("{engine}: loaded in {load_ms} ms, {} Hz, threads {}, provider {}", tts.sample_rate(), o.threads, o.provider);

    let lines: Vec<String> =
        std::fs::read_to_string(fixtures().join("tts_lines.txt")).unwrap().lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect();
    // The text spoken in every persona clip (`vs.rs::PERSONA_SAMPLE`).
    let ref_text = onsite_lib::voice::vs::PERSONA_SAMPLE;
    let out = root().join("out").join(o.tag.clone().unwrap_or_else(|| engine.to_string()));
    let mut rows = Vec::new();
    for persona in &o.personas {
        let clip_dir = o.refdir.as_ref().map(PathBuf::from).unwrap_or_else(|| root().join("personas"));
        let clip = Wave::read(&clip_dir.join(format!("{persona}.wav")).to_string_lossy()).expect("persona clip");
        for run in 1..=o.runs {
            let dir = out.join(persona).join(format!("r{run}"));
            std::fs::create_dir_all(&dir).unwrap();
            for (i, line) in lines.iter().enumerate() {
                if o.lines.as_ref().is_some_and(|l| !l.contains(&(i + 1))) {
                    continue;
                }
                let mut extra = HashMap::new();
                if engine == "pocket" {
                    extra.insert("max_reference_audio_len".to_string(), json!(o.reflen));
                    extra.insert("seed".to_string(), json!(o.seed));
                    if let Some(t) = o.temp {
                        extra.insert("temperature".to_string(), json!(t));
                    }
                } else {
                    extra.insert("min_char_in_sentence".to_string(), json!(10));
                }
                let gen = GenerationConfig {
                    speed: 1.0,
                    reference_audio: Some(clip.samples().to_vec()),
                    reference_sample_rate: clip.sample_rate(),
                    reference_text: (engine == "zipvoice").then(|| ref_text.to_string()),
                    num_steps: o.steps,
                    extra: Some(extra),
                    ..Default::default()
                };
                let start = Instant::now();
                // First callback with samples = when the first sentence's audio could start playing.
                let first = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
                let seen = first.clone();
                let audio = tts
                    .generate_with_config(
                        line,
                        &gen,
                        Some(move |s: &[f32], _p: f32| {
                            if !s.is_empty() {
                                let _ = seen.compare_exchange(0, start.elapsed().as_millis() as u64, Ordering::SeqCst, Ordering::SeqCst);
                            }
                            true
                        }),
                    )
                    .expect("generate");
                let gen_ms = start.elapsed().as_millis();
                let first_ms = Some(first.load(Ordering::SeqCst) as u128).filter(|&v| v > 0);
                let secs = audio.samples().len() as f32 / audio.sample_rate() as f32;
                let rtf = gen_ms as f32 / 1000.0 / secs.max(0.01);
                audio.save(&dir.join(format!("{:02}.wav", i + 1)).to_string_lossy());
                println!("{persona} r{run} #{:02} gen {gen_ms:>5} ms  first {:>5} ms  audio {secs:5.2} s  rtf {rtf:.2}", i + 1, first_ms.unwrap_or(gen_ms));
                rows.push(json!({"persona": persona, "run": run, "line": i + 1, "words": line.split_whitespace().count(),
                    "gen_ms": gen_ms, "first_ms": first_ms.unwrap_or(gen_ms), "audio_s": secs, "rtf": rtf}));
            }
        }
    }
    summarize_tts(engine, load_ms, &rows);
    std::fs::write(out.join("tts.json"), serde_json::to_string_pretty(&json!({"engine": engine, "threads": o.threads,
        "provider": o.provider, "steps": o.steps, "load_ms": load_ms, "rows": rows})).unwrap())
    .unwrap();
}

fn pct(mut v: Vec<f64>, p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[((v.len() - 1) as f64 * p).round() as usize]
}

fn summarize_tts(engine: &str, load_ms: u128, rows: &[Value]) {
    let f = |k: &str, short: Option<bool>| -> Vec<f64> {
        rows.iter()
            .filter(|r| short.is_none_or(|s| (r["words"].as_u64().unwrap() <= 8) == s))
            .map(|r| r[k].as_f64().unwrap())
            .collect()
    };
    println!("\n== {engine} summary: load {load_ms} ms ==");
    println!("short lines (<= 8 words): first audio p50 {:.0} ms, p90 {:.0} ms", pct(f("first_ms", Some(true)), 0.5), pct(f("first_ms", Some(true)), 0.9));
    println!("long lines: first audio p50 {:.0} ms, p90 {:.0} ms", pct(f("first_ms", Some(false)), 0.5), pct(f("first_ms", Some(false)), 0.9));
    println!("rtf p50 {:.2}, p90 {:.2}, max {:.2}", pct(f("rtf", None), 0.5), pct(f("rtf", None), 0.9), pct(f("rtf", None), 1.0));
}

// ---------- stt ----------

struct Segment {
    pcm: Vec<f32>,
    /// Sample offset in the answer where the segment closed.
    end_at: usize,
}

/// Cuts an answer into segments the way `stt.rs::run` does (energy VAD, pre-roll, 700/300 ms pauses).
/// The answer itself ends at end of file, not on the user's long pause.
fn segments(pcm: &[f32]) -> Vec<Segment> {
    let mut vad = Vad::new(20, 0);
    let mut segs = Vec::new();
    let mut cur: Option<Vec<f32>> = None;
    let mut preroll: Vec<f32> = Vec::new();
    for (n, frame) in pcm.chunks(FRAME).enumerate() {
        let level = rms(frame);
        let loud = level > vad.threshold();
        let started = matches!(vad.push(level), VadEvent::SpeechStart);
        if cur.is_none() && (started || (loud && vad.is_speaking())) {
            cur = Some(std::mem::take(&mut preroll));
        }
        match cur.as_mut() {
            Some(seg) => {
                seg.extend_from_slice(frame);
                let pause = match seg.len() {
                    l if l < SEG_MIN => None,
                    l if l < SEG_LONG => Some(SEG_PAUSE_MS),
                    _ => Some(SEG_PAUSE_LONG_MS),
                };
                if vad.is_speaking() && pause.is_some_and(|p| vad.quiet_ms() >= p) {
                    segs.push(Segment { pcm: cur.take().unwrap(), end_at: (n + 1) * FRAME });
                }
            }
            None => {
                preroll.extend_from_slice(frame);
                let extra = preroll.len().saturating_sub(PREROLL);
                preroll.drain(..extra);
            }
        }
    }
    if let Some(seg) = cur {
        segs.push(Segment { pcm: seg, end_at: pcm.len() });
    }
    segs
}

fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect()
}

/// Word edit distance between reference and hypothesis.
fn word_errors(reference: &str, hyp: &str) -> (usize, usize) {
    let (r, h) = (words(reference), words(hyp));
    let mut prev: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut cur = vec![i; h.len() + 1];
        for j in 1..=h.len() {
            cur[j] = (prev[j - 1] + usize::from(r[i - 1] != h[j - 1])).min(prev[j] + 1).min(cur[j - 1] + 1);
        }
        prev = cur;
    }
    (prev[h.len()], r.len())
}

enum Asr {
    Offline(OfflineRecognizer),
    Online(OnlineRecognizer),
}

fn load_asr(engine: &str, o: &Opts) -> Asr {
    match engine {
        "parakeet" => {
            let d = "sherpa-onnx-nemo-parakeet-tdt-0.6b-v2-int8";
            let mut c = OfflineRecognizerConfig::default();
            c.model_config.transducer = OfflineTransducerModelConfig {
                encoder: file(d, "encoder.int8.onnx"),
                decoder: file(d, "decoder.int8.onnx"),
                joiner: file(d, "joiner.int8.onnx"),
            };
            c.model_config.tokens = file(d, "tokens.txt");
            c.model_config.model_type = Some("nemo_transducer".into());
            c.model_config.num_threads = o.threads;
            c.model_config.provider = Some(o.provider.clone());
            Asr::Offline(OfflineRecognizer::create(&c).expect("create parakeet"))
        }
        "nemo480" | "nemotron" => {
            let d = if engine == "nemo480" {
                "sherpa-onnx-nemo-streaming-fast-conformer-transducer-en-480ms-int8"
            } else {
                "sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25"
            };
            let onnx = |prefix: &str| -> Option<String> {
                let dir = root().join("models").join(d);
                let hit = std::fs::read_dir(&dir)
                    .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .find(|n| n.starts_with(prefix) && n.ends_with(".onnx"));
                Some(dir.join(hit.unwrap_or_else(|| panic!("no {prefix}*.onnx in {d}"))).to_string_lossy().into_owned())
            };
            let mut c = OnlineRecognizerConfig::default();
            c.model_config.transducer = OnlineTransducerModelConfig { encoder: onnx("encoder"), decoder: onnx("decoder"), joiner: onnx("joiner") };
            c.model_config.tokens = file(d, "tokens.txt");
            c.model_config.num_threads = o.threads;
            c.model_config.provider = Some(o.provider.clone());
            c.decoding_method = Some("greedy_search".into());
            Asr::Online(OnlineRecognizer::create(&c).expect("create streaming asr"))
        }
        other => panic!("unknown stt engine {other}"),
    }
}

fn offline_text(r: &OfflineRecognizer, pcm: &[f32]) -> String {
    let s = r.create_stream();
    s.accept_waveform(MIC_RATE as i32, pcm);
    r.decode(&s);
    s.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default()
}

fn stt(engine: &str, o: &Opts) {
    let answers: Value = serde_json::from_str(&std::fs::read_to_string(fixtures().join("answers.json")).unwrap()).unwrap();
    let vocab = domain_vocab(&[]);
    let t0 = Instant::now();
    let asr = load_asr(engine, o);
    let load_ms = t0.elapsed().as_millis();
    println!("{engine}: loaded in {load_ms} ms, threads {}, provider {}", o.threads, o.provider);
    let (mut errs, mut errs_clean, mut total) = (0, 0, 0);
    let mut rows = Vec::new();
    for a in answers["answers"].as_array().unwrap() {
        let id = a["id"].as_str().unwrap();
        let reference = a["text"].as_str().unwrap_or("");
        let path = fixtures().join("answers").join(format!("{id}.wav"));
        if !path.exists() {
            continue;
        }
        let wave = Wave::read(&path.to_string_lossy()).expect("read answer");
        assert_eq!(wave.sample_rate(), MIC_RATE as i32, "{id}: expected 16 kHz");
        let pcm = wave.samples();
        let audio_s = pcm.len() as f32 / MIC_RATE as f32;
        let (hyp, extra) = match &asr {
            Asr::Offline(r) => {
                // Final text: one decode per segment, as the app would run them.
                let segs = segments(pcm);
                let mut texts = Vec::new();
                let mut last_ms = 0;
                let mut caption_max_ms = 0u128;
                let mut caption_total_ms = 0u128;
                let mut captions = 0;
                for seg in &segs {
                    // Option A captions: re-decode the open segment every 0.5 s.
                    let mut n = CAPTION_EVERY;
                    while n < seg.pcm.len() {
                        let t = Instant::now();
                        offline_text(r, &seg.pcm[..n]);
                        let ms = t.elapsed().as_millis();
                        caption_max_ms = caption_max_ms.max(ms);
                        caption_total_ms += ms;
                        captions += 1;
                        n += CAPTION_EVERY;
                    }
                    let t = Instant::now();
                    texts.push(offline_text(r, &seg.pcm));
                    last_ms = t.elapsed().as_millis();
                }
                let seg_lens: Vec<String> = segs.iter().map(|s| format!("{:.1}", s.pcm.len() as f32 / MIC_RATE as f32)).collect();
                let whole_t = Instant::now();
                let whole = offline_text(r, pcm);
                let whole_ms = whole_t.elapsed().as_millis();
                println!(
                    "  {id}: {} segs [{}], last seg {last_ms} ms, caption re-decode max {caption_max_ms} ms avg {} ms",
                    segs.len(),
                    seg_lens.join(", "),
                    caption_total_ms / captions.max(1) as u128
                );
                let (we, _) = word_errors(reference, &whole);
                (
                    texts.join(" "),
                    json!({"segments": segs.len(), "segment_s": seg_lens, "last_segment_ms": last_ms, "caption_max_ms": caption_max_ms,
                        "caption_avg_ms": caption_total_ms / captions.max(1) as u128, "whole_file_ms": whole_ms, "whole_file_errors": we,
                        "seg_end_s": segs.iter().map(|s| s.end_at as f32 / MIC_RATE as f32).collect::<Vec<_>>()}),
                )
            }
            Asr::Online(r) => {
                // Streaming: feed 100 ms chunks; time is compute time, audio time is position in the file.
                let s = r.create_stream();
                let chunk = MIC_RATE as usize / 10;
                let (mut first_partial_s, mut max_chunk_ms, mut updates, mut last_text) = (None, 0u128, 0, String::new());
                let mut finals: Vec<String> = Vec::new();
                for (k, c) in pcm.chunks(chunk).enumerate() {
                    let t = Instant::now();
                    s.accept_waveform(MIC_RATE as i32, c);
                    while r.is_ready(&s) {
                        r.decode(&s);
                    }
                    let text = r.get_result(&s).map(|x| x.text.trim().to_string()).unwrap_or_default();
                    max_chunk_ms = max_chunk_ms.max(t.elapsed().as_millis());
                    if !text.is_empty() && text != last_text {
                        updates += 1;
                        first_partial_s.get_or_insert((k + 1) as f32 / 10.0);
                    }
                    last_text = text;
                }
                s.accept_waveform(MIC_RATE as i32, &vec![0.0; MIC_RATE as usize / 2]);
                s.input_finished();
                let t = Instant::now();
                while r.is_ready(&s) {
                    r.decode(&s);
                }
                let tail_ms = t.elapsed().as_millis();
                finals.push(r.get_result(&s).map(|x| x.text.trim().to_string()).unwrap_or_default());
                let speech_start = segments(pcm).first().map_or(0.0, |s| (s.end_at - s.pcm.len()) as f32 / MIC_RATE as f32);
                println!(
                    "  {id}: first partial at {:.1} s (speech from ~{speech_start:.1} s), {updates} updates, max chunk {max_chunk_ms} ms, tail {tail_ms} ms",
                    first_partial_s.unwrap_or(-1.0)
                );
                (
                    finals.join(" "),
                    json!({"first_partial_s": first_partial_s, "speech_start_s": speech_start, "updates": updates,
                        "max_chunk_ms": max_chunk_ms, "tail_ms": tail_ms}),
                )
            }
        };
        let cleaned = clean_transcript(&hyp, &vocab);
        if reference.trim().is_empty() {
            println!("  {id}: no reference text yet\n    hyp: {hyp}");
            continue;
        }
        let (e, n) = word_errors(reference, &hyp);
        let (ec, _) = word_errors(reference, &cleaned);
        errs += e;
        errs_clean += ec;
        total += n;
        println!("    WER {:.1}% (cleaned {:.1}%) | {audio_s:.1} s\n    ref: {reference}\n    hyp: {cleaned}", 100.0 * e as f32 / n as f32, 100.0 * ec as f32 / n as f32);
        rows.push(json!({"id": id, "audio_s": audio_s, "errors": e, "errors_cleaned": ec, "ref_words": n, "hyp": hyp, "cleaned": cleaned, "detail": extra}));
    }
    if total > 0 {
        println!("\n== {engine}: WER {:.2}% raw, {:.2}% cleaned, over {total} words ==", 100.0 * errs as f32 / total as f32, 100.0 * errs_clean as f32 / total as f32);
    }
    let out = root().join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join(format!("stt-{engine}.json")), serde_json::to_string_pretty(&json!({"engine": engine, "threads": o.threads,
        "provider": o.provider, "load_ms": load_ms, "errors": errs, "errors_cleaned": errs_clean, "words": total, "rows": rows})).unwrap())
    .unwrap();
}

// ---------- score ----------

fn resample16(w: &Wave) -> Vec<f32> {
    let (sr, x) = (w.sample_rate() as f64, w.samples());
    if sr as i32 == 16000 {
        return x.to_vec();
    }
    let ratio = sr / 16000.0;
    // windowed-sinc low-pass then linear pick (quality is plenty for speaker embeddings)
    let cut = (0.45 / ratio).min(0.5);
    let half = 16isize;
    let n = (x.len() as f64 / ratio) as usize;
    (0..n)
        .map(|i| {
            let c = i as f64 * ratio;
            let (mut acc, mut ws) = (0.0f64, 0.0f64);
            for k in -half..=half {
                let j = c.round() as isize + k;
                if j < 0 || j as usize >= x.len() {
                    continue;
                }
                let d = j as f64 - c;
                let sinc = if d.abs() < 1e-9 { 2.0 * cut } else { (2.0 * std::f64::consts::PI * cut * d).sin() / (std::f64::consts::PI * d) };
                let win = 0.5 + 0.5 * (std::f64::consts::PI * d / (half as f64 + 1.0)).cos();
                acc += x[j as usize] as f64 * sinc * win;
                ws += sinc * win;
            }
            (acc / ws.max(1e-9)) as f32
        })
        .collect()
}

fn embed(ex: &sherpa_onnx::SpeakerEmbeddingExtractor, path: &Path) -> Option<Vec<f32>> {
    let w = Wave::read(&path.to_string_lossy())?;
    let pcm = resample16(&w);
    let st = ex.create_stream()?;
    st.accept_waveform(16000, &pcm);
    st.input_finished();
    if !ex.is_ready(&st) {
        return None;
    }
    ex.compute(&st)
}

fn cos(a: &[f32], b: &[f32]) -> f64 {
    let d: f64 = a.iter().zip(b).map(|(x, y)| (*x as f64) * (*y as f64)).sum();
    let n = |v: &[f32]| v.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    d / (n(a) * n(b)).max(1e-9)
}

/// `score <render dir with NN.wav> --ref <clip.wav> [--model wespeaker|titanet] [--quiet]`
/// Cosine similarity of every line to the reference clip, plus mean, min and the pairwise mean between lines.
fn score(dir: &str, args: &[String]) {
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let m = get("--model").unwrap_or_else(|| "wespeaker".into());
    let mf = if m == "titanet" { "nemo_en_titanet_small.onnx" } else { "wespeaker_en_voxceleb_resnet34.onnx" };
    let ex = sherpa_onnx::SpeakerEmbeddingExtractor::create(&sherpa_onnx::SpeakerEmbeddingExtractorConfig {
        model: Some(root().join("models/spk").join(mf).to_string_lossy().into_owned()),
        num_threads: 2,
        ..Default::default()
    })
    .expect("speaker model");
    let reference = embed(&ex, Path::new(&get("--ref").expect("--ref"))).expect("ref embedding");
    let quiet = args.iter().any(|a| a == "--quiet");
    let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "wav")).collect();
    files.sort();
    let mut embs = Vec::new();
    let mut sims = Vec::new();
    for f in &files {
        let secs = Wave::read(&f.to_string_lossy()).map_or(0.0, |w| w.samples().len() as f32 / w.sample_rate() as f32);
        if secs < 1.0 {
            continue; // too short for a stable embedding
        }
        if let Some(e) = embed(&ex, f) {
            let s = cos(&reference, &e);
            if !quiet {
                println!("{}  {:.3}  ({secs:.1}s)", f.file_name().unwrap().to_string_lossy(), s);
            }
            sims.push(s);
            embs.push(e);
        }
    }
    let mean = sims.iter().sum::<f64>() / sims.len().max(1) as f64;
    let min = sims.iter().cloned().fold(1.0, f64::min);
    let (mut ps, mut pn) = (0.0, 0);
    for i in 0..embs.len() {
        for j in i + 1..embs.len() {
            ps += cos(&embs[i], &embs[j]);
            pn += 1;
        }
    }
    println!("SCORE {dir} model={m} n={} mean={mean:.3} min={min:.3} pairwise={:.3}", sims.len(), ps / pn.max(1) as f64);
}
