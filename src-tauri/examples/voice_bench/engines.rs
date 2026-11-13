//! Candidate recognizers for the bench. They live here until one wins the bake-off.

use onsite_lib::voice::builtin::ParakeetStt;
use sherpa_onnx::{OfflineQwen3ASRModelConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTransducerModelConfig};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub trait Recognizer {
    fn name(&self) -> &'static str;
    /// `pcm` is 16 kHz mono. `context` is empty unless --context was given.
    fn decode(&self, pcm: &[f32], context: &[String]) -> String;
}

pub fn open(engine: &str, models_root: &Path, threads: i32) -> Result<Box<dyn Recognizer>, String> {
    match engine {
        "parakeet" => Ok(Box::new(Parakeet(ParakeetStt::load_with(models_root, threads).map_err(|e| e.to_string())?))),
        "orukeet" => Ok(Box::new(Transducer::open(&models_root.join("orukeet"), threads)?)),
        "qwen3" => Ok(Box::new(Qwen3::open(&models_root.join("qwen3-asr-0.6b"), threads)?)),
        other => Err(format!("unknown engine {other} (parakeet, orukeet, qwen3)")),
    }
}

/// The first file under `dir` (searched two levels deep) whose name contains `part` and ends with `ext`.
/// An int8 file wins over a full-precision one.
fn find(dir: &Path, part: &str, ext: &str) -> Option<PathBuf> {
    let mut hits = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), 0)];
    while let Some((d, depth)) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p = e.path();
            let name = p.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
            if p.is_dir() && depth < 2 {
                stack.push((p.clone(), depth + 1));
            }
            if name.contains(part) && name.ends_with(ext) {
                hits.push(p);
            }
        }
    }
    hits.sort_by_key(|p| !p.to_string_lossy().contains("int8"));
    hits.into_iter().next()
}

fn path(p: Option<PathBuf>, what: &str, dir: &Path) -> Result<Option<String>, String> {
    p.map(|p| Some(p.to_string_lossy().into_owned())).ok_or_else(|| format!("no {what} in {}", dir.display()))
}

fn run(rec: &OfflineRecognizer, pcm: &[f32]) -> String {
    let s = rec.create_stream();
    s.accept_waveform(16_000, pcm);
    rec.decode(&s);
    s.get_result().map(|r| r.text.trim().to_string()).unwrap_or_default()
}

struct Parakeet(ParakeetStt);

impl Recognizer for Parakeet {
    fn name(&self) -> &'static str {
        "parakeet"
    }
    fn decode(&self, pcm: &[f32], _context: &[String]) -> String {
        self.0.transcribe_f32(pcm)
    }
}

/// A NeMo TDT transducer in any folder (Orukeet uses Parakeet's layout).
struct Transducer(OfflineRecognizer);

impl Transducer {
    fn open(dir: &Path, threads: i32) -> Result<Self, String> {
        let mut c = OfflineRecognizerConfig::default();
        c.model_config.transducer = OfflineTransducerModelConfig {
            encoder: path(find(dir, "encoder", ".onnx"), "encoder", dir)?,
            decoder: path(find(dir, "decoder", ".onnx"), "decoder", dir)?,
            joiner: path(find(dir, "joiner", ".onnx"), "joiner", dir)?,
        };
        c.model_config.tokens = path(find(dir, "tokens", ".txt"), "tokens.txt", dir)?;
        c.model_config.model_type = Some("nemo_transducer".into());
        c.model_config.num_threads = threads;
        c.model_config.provider = Some("cpu".into());
        OfflineRecognizer::create(&c).map(Self).ok_or_else(|| format!("sherpa-onnx could not load {}", dir.display()))
    }
}

impl Recognizer for Transducer {
    fn name(&self) -> &'static str {
        "orukeet"
    }
    fn decode(&self, pcm: &[f32], _context: &[String]) -> String {
        run(&self.0, pcm)
    }
}

/// Qwen3-ASR. Its `hotwords` are fixed when the recognizer is made, so one recognizer is kept per context.
struct Qwen3 {
    base: OfflineRecognizerConfig,
    by_context: Mutex<HashMap<String, OfflineRecognizer>>,
}

impl Qwen3 {
    fn open(dir: &Path, threads: i32) -> Result<Self, String> {
        let mut c = OfflineRecognizerConfig::default();
        c.model_config.qwen3_asr = OfflineQwen3ASRModelConfig {
            conv_frontend: path(find(dir, "conv", ".onnx"), "conv frontend", dir)?,
            encoder: path(find(dir, "encoder", ".onnx"), "encoder", dir)?,
            decoder: path(find(dir, "decoder", ".onnx"), "decoder", dir)?,
            tokenizer: path(tokenizer_dir(dir), "tokenizer", dir)?,
            ..Default::default()
        };
        c.model_config.num_threads = threads;
        c.model_config.provider = Some("cpu".into());
        let rec = OfflineRecognizer::create(&c).ok_or_else(|| format!("sherpa-onnx could not load {}", dir.display()))?;
        Ok(Self { base: c, by_context: Mutex::new(HashMap::from([(String::new(), rec)])) })
    }
}

/// The tokenizer is a folder (vocab.json, merges.txt) in the export.
fn tokenizer_dir(dir: &Path) -> Option<PathBuf> {
    find(dir, "vocab", ".json").and_then(|p| p.parent().map(Path::to_path_buf))
}

impl Recognizer for Qwen3 {
    fn name(&self) -> &'static str {
        "qwen3"
    }
    fn decode(&self, pcm: &[f32], context: &[String]) -> String {
        // No documented format in sherpa-onnx-sys 2.0.1: a comma-separated list.
        let key = context.join(",");
        let mut map = self.by_context.lock().unwrap();
        if !map.contains_key(&key) {
            let mut c = self.base.clone();
            c.model_config.qwen3_asr.hotwords = Some(key.clone());
            match OfflineRecognizer::create(&c) {
                Some(r) => {
                    map.insert(key.clone(), r);
                }
                None => return String::from("<load failed with hotwords>"),
            }
        }
        run(&map[&key], pcm)
    }
}
