//! VoiceStudio HTTP client (loopback, port 3900). Field names checked against its live OpenAPI.

use crate::error::{AppError, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

pub const TTS_RATE: u32 = 24_000;

/// A VoiceStudio voice profile. A `design` profile carries a rendered sample
/// (`ref_audio_path`) that the engine clones, plus `instruct` and `seed`; that sample is
/// what keeps the voice the same from one call to the next.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub instruct: String,
    #[serde(default)]
    pub seed: Option<i64>,
    #[serde(default)]
    pub language: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub ref_audio_path: String,
    #[serde(default)]
    pub ref_text: String,
    #[serde(default)]
    pub personality: String,
}

/// A panel interviewer's voice, created in VoiceStudio on first use.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonaVoice {
    pub name: String,
    pub instruct: String,
    pub states: serde_json::Value,
}

pub fn persona_profile_name(person: &str) -> String {
    format!("Onsite · {person}")
}

/// Line rendered as a persona's identity sample (~10 s). The ~4 s default clip was too short to
/// hold the speaker; long replies drifted to other voices.
pub const PERSONA_SAMPLE: &str = "Hi, thanks for joining today. We'll start with a quick introduction, \
and then move on to the main problem. Take your time, think out loud, and ask me anything that is unclear.";

/// (seed, steps) tried in order when rendering a persona sample. OmniVoice's voice design is
/// hit-and-miss: some renders are near-silent hiss (cloning one gave static), so each is scored.
const SAMPLE_TRIES: &[(i64, u32)] = &[(42, 32), (7, 16), (42, 16), (1234, 16), (99, 32), (2024, 32), (5, 16), (77, 32)];

/// Stored in the profile's `personality` field: which checks and voice spec built it. A change
/// to either (e.g. a persona's pitch in panels.json) rebuilds the voice.
pub fn persona_tag(instruct: &str) -> String {
    format!("onsite-voice v2 | {instruct}")
}

/// A persona profile is usable only when it clones a sample we rendered, checked and uploaded
/// for this voice spec. Design profiles render their own sample unchecked, or none when the
/// engine is cold (then every call is a new random speaker).
pub fn persona_profile_ok(p: &Profile, instruct: &str) -> bool {
    p.kind == "clone" && !p.ref_audio_path.trim().is_empty() && p.ref_text == PERSONA_SAMPLE && p.personality == persona_tag(instruct)
}

/// One persona build at a time: the startup prebuild and a round starting must not both create one.
static BUILD: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// When VoiceStudio last produced speech for us. Its weights unload after ~15 min idle.
static LAST_SPEECH: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// True when VoiceStudio spoke within `d`, so its weights are still loaded.
pub fn spoke_within(d: Duration) -> bool {
    LAST_SPEECH.lock().unwrap().is_some_and(|t| t.elapsed() < d)
}

/// What a rendered sample sounds like, measured on 24 kHz PCM.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SampleScore {
    /// Share of 50 ms frames with speech-level energy.
    pub voiced: f32,
    /// Mean zero-crossing rate of voiced frames, in Hz. Hiss sits far above speech.
    pub zcr: f32,
    /// Median pitch of voiced frames, in Hz (0 = none found).
    pub f0: f32,
}

pub fn score_sample(pcm: &[i16]) -> SampleScore {
    let rate = TTS_RATE as usize;
    let x: Vec<f32> = pcm.iter().map(|&v| v as f32 / 32768.0).collect();
    let win = rate / 20;
    let (mut frames, mut voiced, mut zcr) = (0usize, 0usize, 0f32);
    for f in x.chunks_exact(win) {
        frames += 1;
        if crate::voice::audio::rms(f) > 0.02 {
            voiced += 1;
            zcr += f.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count() as f32 * rate as f32 / (2.0 * win as f32);
        }
    }
    // Pitch: autocorrelation over 40 ms frames, 70-400 Hz.
    let win = rate / 25;
    let mut f0s: Vec<f32> = x
        .chunks_exact(win)
        .filter(|f| crate::voice::audio::rms(f) > 0.03)
        .filter_map(|f| {
            let e: f32 = f.iter().map(|v| v * v).sum();
            let (lag, c) = (rate / 400..rate / 70)
                .map(|lag| (lag, f.iter().zip(&f[lag..]).map(|(a, b)| a * b).sum::<f32>() / e))
                .max_by(|a, b| a.1.total_cmp(&b.1))?;
            (c > 0.5).then_some(rate as f32 / lag as f32)
        })
        .collect();
    f0s.sort_by(f32::total_cmp);
    SampleScore {
        voiced: if frames == 0 { 0.0 } else { voiced as f32 / frames as f32 },
        zcr: if voiced == 0 { f32::MAX } else { zcr / voiced as f32 },
        f0: f0s.get(f0s.len() / 2).copied().unwrap_or(0.0),
    }
}

/// Median pitch range (Hz) that sounds natural for a voice spec. OmniVoice overshoots: a
/// "high pitch" female take at 267 Hz sounded shrill, so even "high" stays below 250.
pub fn pitch_range(instruct: &str) -> (f32, f32) {
    let low = instruct.to_lowercase();
    let female = low.contains("female");
    let male = !female && low.contains("male");
    let pitch = ["very low", "low", "moderate", "very high", "high"].into_iter().find(|p| low.contains(&format!("{p} pitch")));
    match (female, male, pitch) {
        (true, _, Some("very low" | "low")) => (150.0, 200.0),
        (true, _, Some("high" | "very high")) => (195.0, 250.0),
        (true, _, _) => (165.0, 235.0),
        (_, true, Some("very low" | "low")) => (75.0, 125.0),
        (_, true, Some("high" | "very high")) => (125.0, 170.0),
        (_, true, _) => (90.0, 150.0),
        _ => (80.0, 250.0),
    }
}

/// Clean speech in the persona's register. Measured: good renders 57-71 % voiced with ZCR
/// ~1000-1550 Hz; hiss renders 8-35 % voiced or ZCR ~3000 Hz.
pub fn sample_ok(s: &SampleScore, instruct: &str) -> bool {
    let (lo, hi) = pitch_range(instruct);
    s.voiced >= 0.45 && s.zcr < 1800.0 && (lo..=hi).contains(&s.f0)
}

/// 16-bit mono WAV at the TTS rate.
fn wav_bytes(pcm: &[i16]) -> Vec<u8> {
    let data = (pcm.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + data as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + data).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&TTS_RATE.to_le_bytes());
    b.extend_from_slice(&(TTS_RATE * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data.to_le_bytes());
    pcm.iter().for_each(|v| b.extend_from_slice(&v.to_le_bytes()));
    b
}

/// The voice to use: the saved profile id, else the first profile. Never VoiceStudio's
/// "default" voice: with no profile, OmniVoice samples a new random speaker on every call,
/// so each sentence came out in a different voice.
pub fn resolve_profile(saved: &str, profiles: &[Profile]) -> Result<Profile> {
    let saved = saved.trim();
    if let Some(p) = profiles.iter().find(|p| !saved.is_empty() && p.id == saved) {
        return Ok(p.clone());
    }
    // Persona voices belong to panel interviewers and get rebuilt; prefer the owner's own voices.
    let own = profiles.iter().find(|p| !p.name.starts_with(&persona_profile_name("")));
    let first = own.or(profiles.first()).cloned().ok_or_else(|| {
        AppError::Voice("VoiceStudio has no voice profiles. Make one in VoiceStudio, then pick it in Settings.".into())
    })?;
    if !saved.is_empty() {
        tracing::warn!(saved, using = %first.id, "saved voice profile not found; using the first profile");
    }
    Ok(first)
}

/// JSON body for `POST /v1/audio/speech`. The profile id makes VoiceStudio clone the
/// profile's sample; seed and language are sent too so nothing is left to chance.
pub fn speech_body(text: &str, voice: &Profile, steps: u32, speed: f32) -> Value {
    let mut body = json!({
        "model": "omnivoice",
        "input": text,
        "voice": voice.id,
        "response_format": "pcm",
        "stream_format": "audio",
        "num_step": steps,
        "speed": speed,
    });
    if let Some(seed) = voice.seed {
        body["seed"] = json!(seed);
    }
    let lang = voice.language.trim();
    if !lang.is_empty() && !lang.eq_ignore_ascii_case("auto") {
        body["language"] = json!(lang);
    }
    body
}

#[derive(Clone)]
pub struct VoiceStudio {
    http: reqwest::Client,
    base: String,
    key: Option<String>,
}

fn err(what: &str, e: impl std::fmt::Display) -> AppError {
    AppError::Voice(format!("VoiceStudio {what}: {e}"))
}

impl VoiceStudio {
    pub fn new(base: &str) -> Self {
        Self {
            http: reqwest::Client::builder().connect_timeout(Duration::from_secs(2)).build().expect("http client"),
            base: base.trim_end_matches('/').to_string(),
            key: std::env::var("OMNIVOICE_API_KEY").ok().filter(|k| !k.is_empty()),
        }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let r = self.http.request(method, format!("{}{path}", self.base));
        match &self.key {
            Some(k) => r.bearer_auth(k),
            None => r,
        }
    }

    async fn get_json(&self, path: &str) -> Result<Value> {
        let r = self.req(reqwest::Method::GET, path).timeout(Duration::from_secs(5)).send().await.map_err(|e| err(path, e))?;
        r.error_for_status().map_err(|e| err(path, e))?.json().await.map_err(|e| err(path, e))
    }

    pub async fn health(&self) -> Result<Value> {
        self.get_json("/health").await
    }

    /// Ready unless weights are loading. `/model/status` reports `loaded: false` while idle
    /// (it loads on demand in about a second), so only `loading` means "not yet".
    pub async fn ready(&self) -> Result<bool> {
        Ok(!self.get_json("/model/status").await?["loading"].as_bool().unwrap_or(false))
    }

    pub async fn profiles(&self) -> Result<Vec<Profile>> {
        Ok(serde_json::from_value(self.get_json("/profiles").await?)?)
    }

    /// Fetches the profiles and picks the voice to use (see `resolve_profile`).
    pub async fn resolve_voice(&self, saved: &str) -> Result<Profile> {
        resolve_profile(saved, &self.profiles().await?)
    }

    /// The panel interviewer's own voice: a clone profile named after them, made once from a
    /// sample we render from their voice-design tags and check (clean, right pitch). Cloning one
    /// fixed sample keeps the same person sounding the same in every session.
    pub async fn persona_voice(&self, p: &PersonaVoice) -> Result<Profile> {
        let _one = BUILD.lock().await;
        let name = persona_profile_name(&p.name);
        for old in self.profiles().await?.into_iter().filter(|x| x.name == name) {
            if persona_profile_ok(&old, &p.instruct) {
                return Ok(old);
            }
            tracing::info!(id = %old.id, %name, "rebuilding interviewer voice");
            self.delete_profile(&old.id).await?;
        }
        let (seed, pcm) = self.render_persona_sample(&name, &p.instruct).await?;
        self.create_clone(&name, seed, &pcm, &persona_tag(&p.instruct)).await
    }

    /// Renders the sample line until one take passes [`sample_ok`].
    async fn render_persona_sample(&self, name: &str, instruct: &str) -> Result<(i64, Vec<i16>)> {
        for &(seed, steps) in SAMPLE_TRIES {
            let body = json!({
                "model": "omnivoice",
                "input": PERSONA_SAMPLE,
                "voice": "default",
                "instruct": instruct,
                "seed": seed,
                "num_step": steps,
                "language": "English",
                "response_format": "pcm",
                "stream_format": "audio",
            });
            let r = self
                .req(reqwest::Method::POST, "/v1/audio/speech")
                .timeout(Duration::from_secs(180))
                .json(&body)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|e| err("voice sample", e))?;
            let bytes = r.bytes().await.map_err(|e| err("voice sample", e))?;
            let pcm = bytes_to_i16(&mut None, &bytes);
            let score = score_sample(&pcm);
            let ok = sample_ok(&score, instruct);
            tracing::info!(%name, seed, steps, voiced = score.voiced, zcr = score.zcr, f0 = score.f0, ok, "voice sample take");
            if ok {
                return Ok((seed, pcm));
            }
        }
        Err(err("create voice", format!("no clean voice sample for {name} after {} tries", SAMPLE_TRIES.len())))
    }

    async fn create_clone(&self, name: &str, seed: i64, pcm: &[i16], tag: &str) -> Result<Profile> {
        let audio = reqwest::multipart::Part::bytes(wav_bytes(pcm)).file_name("sample.wav").mime_str("audio/wav").map_err(|e| err("create voice", e))?;
        let form = reqwest::multipart::Form::new()
            .text("name", name.to_string())
            .text("kind", "clone")
            .text("language", "English")
            .text("seed", seed.to_string())
            .text("ref_text", PERSONA_SAMPLE)
            .text("personality", tag.to_string())
            .part("ref_audio", audio);
        let r = self
            .req(reqwest::Method::POST, "/profiles")
            .timeout(Duration::from_secs(60))
            .multipart(form)
            .send()
            .await
            .map_err(|e| err("create voice", e))?;
        if !r.status().is_success() {
            let status = r.status();
            let detail = r.text().await.unwrap_or_default();
            return Err(err("create voice", format!("{status} {}", detail.chars().take(200).collect::<String>())));
        }
        let made = self.profiles().await?.into_iter().find(|x| x.name == name).ok_or_else(|| err("create voice", "profile not listed after creation"))?;
        tracing::info!(%name, id = %made.id, seed, "created interviewer voice");
        Ok(made)
    }

    async fn delete_profile(&self, id: &str) -> Result<()> {
        self.req(reqwest::Method::DELETE, &format!("/profiles/{id}"))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| err("delete voice", e))?;
        Ok(())
    }

    /// Streams raw 24 kHz mono PCM16 to `on_pcm` as it arrives. No fallback engine or voice:
    /// a failure is returned, so the interviewer never switches to a different voice.
    pub async fn speak(&self, text: &str, voice: &Profile, steps: u32, speed: f32, mut on_pcm: impl FnMut(&[i16])) -> Result<()> {
        if voice.id.is_empty() {
            return Err(err("speech", "no voice profile chosen"));
        }
        let body = speech_body(text, voice, steps, speed);
        let r = self
            .req(reqwest::Method::POST, "/v1/audio/speech")
            .timeout(Duration::from_secs(120))
            .json(&body)
            .send()
            .await
            .map_err(|e| err("speech", e))?;
        if !r.status().is_success() {
            let status = r.status();
            let detail = r.text().await.unwrap_or_default();
            return Err(err("speech", format!("{status} {}", detail.chars().take(200).collect::<String>())));
        }
        let mut carry: Option<u8> = None;
        let mut stream = r.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| err("speech stream", e))?;
            let samples = bytes_to_i16(&mut carry, &chunk);
            if !samples.is_empty() {
                on_pcm(&samples);
            }
        }
        *LAST_SPEECH.lock().unwrap() = Some(std::time::Instant::now());
        Ok(())
    }

    pub fn stt_ws_url(&self, sample_rate: u32) -> String {
        let ws = self.base.replacen("http", "ws", 1);
        format!("{ws}/v1/audio/transcriptions/stream?pcm=1&sr={sample_rate}")
    }
}

/// Little-endian PCM16 decode that tolerates odd-length network chunks.
pub fn bytes_to_i16(carry: &mut Option<u8>, chunk: &[u8]) -> Vec<i16> {
    let mut bytes: Vec<u8> = Vec::with_capacity(chunk.len() + 1);
    bytes.extend(carry.take());
    bytes.extend_from_slice(chunk);
    if bytes.len() % 2 == 1 {
        *carry = bytes.pop();
    }
    bytes.as_chunks::<2>().0.iter().map(|b| i16::from_le_bytes(*b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_decode_across_odd_chunks() {
        let samples: Vec<i16> = vec![1, -2, 300, i16::MIN, i16::MAX];
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut carry = None;
        let mut out = bytes_to_i16(&mut carry, &bytes[..3]);
        out.extend(bytes_to_i16(&mut carry, &bytes[3..8]));
        out.extend(bytes_to_i16(&mut carry, &bytes[8..]));
        assert_eq!(out, samples);
        assert_eq!(carry, None);
    }

    fn prof(id: &str) -> Profile {
        Profile { id: id.into(), name: id.into(), ..Default::default() }
    }

    #[test]
    fn resolve_prefers_saved_then_first() {
        let ps = vec![prof("a"), prof("b")];
        assert_eq!(resolve_profile("b", &ps).unwrap().id, "b");
        assert_eq!(resolve_profile("", &ps).unwrap().id, "a");
        assert_eq!(resolve_profile("gone", &ps).unwrap().id, "a");
        assert!(resolve_profile("", &[]).is_err());
        assert!(resolve_profile("a", &[]).is_err());
        let mut neha = prof("n");
        neha.name = persona_profile_name("Neha Kapoor");
        assert_eq!(resolve_profile("gone", &[neha.clone(), prof("mine")]).unwrap().id, "mine");
        assert_eq!(resolve_profile("n", &[neha.clone(), prof("mine")]).unwrap().id, "n", "an explicit pick still wins");
        assert_eq!(resolve_profile("", &[neha]).unwrap().id, "n");
    }

    #[test]
    fn body_pins_profile_seed_and_language() {
        let mut p = prof("f3409298");
        p.seed = Some(279285652);
        p.language = "English".into();
        let b = speech_body("Hi.", &p, 8, 1.0);
        assert_eq!(b["voice"], "f3409298");
        assert_eq!(b["model"], "omnivoice");
        assert_eq!(b["seed"], 279285652);
        assert_eq!(b["language"], "English");
        assert_eq!(b["response_format"], "pcm");
        assert_eq!(b["num_step"], 8);

        p.seed = None;
        p.language = "Auto".into();
        let b = speech_body("Hi.", &p, 8, 1.0);
        assert!(b.get("seed").is_none());
        assert!(b.get("language").is_none());
    }

    #[test]
    fn profile_parses_live_shape() {
        let raw = r#"[{"id":"f3409298","name":"Tech interviewer voice 1","ref_audio_path":"f3409298.wav","ref_text":"",
            "instruct":"male, middle-aged, moderate pitch, indian accent","language":"English","locked_audio_path":"",
            "seed":279285652,"is_locked":0,"kind":"design","created_at":1793006193.0}]"#;
        let ps: Vec<Profile> = serde_json::from_str(raw).unwrap();
        assert_eq!(ps[0].seed, Some(279285652));
        assert_eq!(ps[0].kind, "design");
        let none: Vec<Profile> = serde_json::from_str(r#"[{"id":"x","name":"y","seed":null}]"#).unwrap();
        assert_eq!(none[0].seed, None);
    }

    #[test]
    fn persona_profile_needs_checked_clone() {
        let mut p = prof("n");
        let spec = "female, young adult, moderate pitch, indian accent";
        p.kind = "design".into();
        p.ref_audio_path = "n.wav".into();
        p.ref_text = PERSONA_SAMPLE.into();
        p.personality = persona_tag(spec);
        assert!(!persona_profile_ok(&p, spec), "design samples are unchecked");
        p.kind = "clone".into();
        assert!(persona_profile_ok(&p, spec));
        assert!(!persona_profile_ok(&p, "female, young adult, high pitch, indian accent"), "spec changed");
        p.ref_text.clear();
        assert!(!persona_profile_ok(&p, spec));
    }

    fn tone(hz: f32, amp: f32, secs: f32) -> Vec<i16> {
        let n = (TTS_RATE as f32 * secs) as usize;
        (0..n).map(|i| (amp * (i as f32 * hz * std::f32::consts::TAU / TTS_RATE as f32).sin() * 32767.0) as i16).collect()
    }

    #[test]
    fn sample_score_tells_voices_and_hiss_apart() {
        // A harmonic-rich 230 Hz voice-like tone with gaps, a 130 Hz one, and loud noise.
        let mut female = tone(230.0, 0.3, 2.0);
        female.extend(vec![0i16; TTS_RATE as usize / 2]);
        let s = score_sample(&female);
        assert!((s.f0 - 230.0).abs() < 10.0, "{s:?}");
        assert!(s.voiced > 0.7 && s.voiced < 0.9, "{s:?}");
        assert!(sample_ok(&s, "female, young adult, high pitch, indian accent"));
        assert!(sample_ok(&s, "female, young adult, moderate pitch, indian accent"));
        assert!(!sample_ok(&s, "male, middle-aged"));
        // The shrill 267 Hz take that passed before must fail now.
        let shrill = SampleScore { f0: 267.0, ..s };
        assert!(!sample_ok(&shrill, "female, young adult, high pitch, indian accent"));
        assert!(!sample_ok(&SampleScore { f0: 307.0, ..s }, "female"));

        let s = score_sample(&tone(130.0, 0.3, 2.0));
        assert!((s.f0 - 130.0).abs() < 6.0, "{s:?}");
        assert!(sample_ok(&s, "male, middle-aged, moderate pitch"));
        assert!(!sample_ok(&s, "female, high pitch"));

        let mut seed = 1u32;
        let hiss: Vec<i16> = (0..TTS_RATE * 2)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                ((seed >> 16) as i16) / 4
            })
            .collect();
        let s = score_sample(&hiss);
        assert!(s.zcr > 3000.0, "{s:?}");
        assert!(!sample_ok(&s, "female"));
        assert!(!sample_ok(&score_sample(&vec![0; 48_000]), "female"), "silence");
    }

    #[test]
    fn wav_header_is_valid() {
        let w = wav_bytes(&[1, -1, 2]);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(w.len(), 44 + 6);
        assert_eq!(u32::from_le_bytes(w[24..28].try_into().unwrap()), TTS_RATE);
        assert_eq!(u32::from_le_bytes(w[40..44].try_into().unwrap()), 6);
    }

    #[test]
    fn ws_url() {
        assert_eq!(
            VoiceStudio::new("http://127.0.0.1:3900/").stt_ws_url(16_000),
            "ws://127.0.0.1:3900/v1/audio/transcriptions/stream?pcm=1&sr=16000"
        );
    }
}
