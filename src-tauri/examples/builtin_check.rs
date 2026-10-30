//! Exercises the built-in voice engine outside the UI, through the same code a round uses
//! (`voice::models`, `voice::builtin`, `stt::clean_transcript`).
//!
//! ```text
//! cargo run --release --example builtin_check -- [--download] [--models <root>] [--wav <16 kHz mono wav>]
//! ```
//! `--download` installs both models into a temp app-data folder first (about 580 MB from github.com).
//! Otherwise models are read from `--models` (default `target/voice-spike/models`, layout `<root>/<id>/`).
//! Speech goes to `target/voice-spike/out/builtin/`.

use onsite_lib::voice::audio::{f32_to_i16, i16_to_f32, Resampler, MIC_RATE};
use onsite_lib::voice::builtin::{ParakeetStt, PocketTts, VOICES};
use onsite_lib::voice::models;
use onsite_lib::voice::stt::{clean_transcript, domain_vocab};
use std::path::{Path, PathBuf};
use std::time::Instant;

const LINES: [&str; 3] = [
    "Hi, I'm {name}. Let's start with a quick introduction.",
    "Okay.",
    "Walk me through how you would make a payment endpoint idempotent, using Redis and PostgreSQL.",
];

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut root = get("--models").map(PathBuf::from).unwrap_or_else(|| manifest.join("target/voice-spike/models"));
    let _tmp;
    if args.iter().any(|a| a == "--download") {
        let tmp = tempfile::tempdir().unwrap();
        for pin in models::PINS {
            let t = Instant::now();
            let last = std::sync::Mutex::new(0u64);
            models::install(tmp.path(), pin, move |p| {
                let mut l = last.lock().unwrap();
                if p.state != "downloading" || p.done / 50_000_000 != *l {
                    *l = p.done / 50_000_000;
                    println!("  {} {} {}/{} MB {}", p.id, p.state, p.done / 1_000_000, p.total / 1_000_000, p.message);
                }
            })
            .await
            .expect("install");
            println!("installed {} in {:.1} s", pin.label, t.elapsed().as_secs_f32());
            assert!(models::installed(tmp.path(), &pin));
        }
        root = models::root(tmp.path());
        _tmp = tmp;
    }
    println!("models: {}", root.display());

    let t = Instant::now();
    let tts = std::sync::Arc::new(PocketTts::load(&root, &manifest.join("../prompts/voices")).expect("load Pocket TTS"));
    println!("Pocket TTS loaded in {} ms ({} Hz)", t.elapsed().as_millis(), tts.rate);
    let t = Instant::now();
    let stt = ParakeetStt::load(&root).expect("load Parakeet");
    println!("Parakeet loaded in {} ms", t.elapsed().as_millis());

    let out = manifest.join("target/voice-spike/out/builtin");
    std::fs::create_dir_all(&out).unwrap();
    let vocab = domain_vocab(&[]);
    let mut spoken: Vec<(String, Vec<i16>)> = Vec::new();
    for (clip, name, _) in VOICES {
        for (n, line) in LINES.iter().enumerate() {
            let line = line.replace("{name}", name);
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<i16>>();
            let start = Instant::now();
            let (tts2, line2) = (tts.clone(), line.clone());
            let job = tokio::task::spawn_blocking(move || tts2.speak(&line2, clip, 1.0, tx));
            let (mut pcm, mut first) = (Vec::new(), None);
            while let Some(c) = rx.recv().await {
                first.get_or_insert(start.elapsed().as_millis());
                pcm.extend(c);
            }
            let res = job.await.unwrap();
            res.expect("speak");
            let total = start.elapsed().as_millis();
            let secs = pcm.len() as f32 / tts.rate as f32;
            println!("{name} line {}: first audio {} ms, total {total} ms, audio {secs:.1} s, rtf {:.2}", n + 1, first.unwrap_or(0), total as f32 / 1000.0 / secs.max(0.01));
            sherpa_onnx::write(&out.join(format!("{clip}_{}.wav", n + 1)).to_string_lossy(), &i16_to_f32(&pcm), tts.rate as i32);
            spoken.push((line, pcm));
        }
    }

    // Through the round's STT path: 16 kHz segment -> Parakeet -> clean_transcript. Also the caption re-decode.
    let mut inputs: Vec<(String, Vec<i16>)> = spoken
        .into_iter()
        .map(|(line, pcm)| {
            let down = Resampler::new(tts.rate, MIC_RATE).process(&i16_to_f32(&pcm));
            (line, f32_to_i16(&down))
        })
        .collect();
    if let Some(w) = get("--wav") {
        let wave = sherpa_onnx::Wave::read(&w).expect("read wav");
        assert_eq!(wave.sample_rate(), MIC_RATE as i32, "expected 16 kHz");
        inputs.push((format!("(file {w})"), f32_to_i16(wave.samples())));
    }
    for (line, pcm) in &inputs {
        let t = Instant::now();
        let raw = stt.transcribe(pcm);
        let ms = t.elapsed().as_millis();
        println!("STT {:.1} s audio in {ms} ms\n  said: {line}\n  heard: {}", pcm.len() as f32 / MIC_RATE as f32, clean_transcript(&raw, &vocab));
    }
    // Option A caption: re-decode the growing segment every 0.5 s of audio.
    if let Some((_, pcm)) = inputs.iter().max_by_key(|(_, p)| p.len()) {
        let step = MIC_RATE as usize / 2;
        let times: Vec<u128> = (step..=pcm.len()).step_by(step).map(|n| {
            let t = Instant::now();
            stt.caption(&pcm[..n]).expect("caption");
            t.elapsed().as_millis()
        }).collect();
        println!("captions over {:.1} s of audio: {} decodes, max {} ms, mean {} ms", pcm.len() as f32 / MIC_RATE as f32, times.len(), times.iter().max().unwrap_or(&0), times.iter().sum::<u128>() / times.len().max(1) as u128);
    }
}
