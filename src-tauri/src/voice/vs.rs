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

/// Line VoiceStudio renders as a persona's identity sample. About 10 s: the default line
/// gave a ~4 s clip, too short to hold the speaker, and long replies drifted to other voices.
pub const PERSONA_SAMPLE: &str = "Hi, thanks for joining today. We'll start with a quick introduction, \
and then move on to the main problem. Take your time, think out loud, and ask me anything that is unclear.";

/// A persona profile is usable only when its sample was rendered from our current line.
/// Without a sample, VoiceStudio falls back to instruct-only design: a new speaker every call.
pub fn persona_profile_ok(p: &Profile) -> bool {
    !p.ref_audio_path.trim().is_empty() && p.ref_text == PERSONA_SAMPLE
}

/// The voice to use: the saved profile id, else the first profile. Never VoiceStudio's
/// "default" voice: with no profile, OmniVoice samples a new random speaker on every call,
/// so each sentence came out in a different voice.
pub fn resolve_profile(saved: &str, profiles: &[Profile]) -> Result<Profile> {
    let saved = saved.trim();
    if let Some(p) = profiles.iter().find(|p| !saved.is_empty() && p.id == saved) {
        return Ok(p.clone());
    }
    let first = profiles.first().cloned().ok_or_else(|| {
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

    /// The panel interviewer's own voice: the profile named after them, created from their
    /// voice-design tags. A design profile clones one fixed sample, so the same person sounds
    /// the same in every session. Profiles with no sample, or the old short one, are rebuilt.
    pub async fn persona_voice(&self, p: &PersonaVoice) -> Result<Profile> {
        let name = persona_profile_name(&p.name);
        for old in self.profiles().await?.into_iter().filter(|x| x.name == name) {
            if persona_profile_ok(&old) {
                return Ok(old);
            }
            tracing::info!(id = %old.id, %name, "rebuilding interviewer voice (missing or short sample)");
            self.delete_profile(&old.id).await?;
        }
        let made = self.create_persona(&name, p).await?;
        if persona_profile_ok(&made) {
            return Ok(made);
        }
        // VoiceStudio saves a design profile without its sample when the engine isn't loaded.
        // Load it with one throwaway line, then render the sample again.
        tracing::info!(%name, "voice sample pending; warming VoiceStudio and retrying");
        let _ = self.speak("Okay.", &made, 8, 1.0, |_| {}).await;
        self.delete_profile(&made.id).await?;
        let made = self.create_persona(&name, p).await?;
        if !persona_profile_ok(&made) {
            self.delete_profile(&made.id).await?;
            return Err(err("create voice", format!("{name} has no voice sample yet. Try again in a minute.")));
        }
        Ok(made)
    }

    async fn create_persona(&self, name: &str, p: &PersonaVoice) -> Result<Profile> {
        let form = reqwest::multipart::Form::new()
            .text("name", name.to_string())
            .text("kind", "design")
            .text("instruct", p.instruct.clone())
            .text("language", "English")
            .text("ref_text", PERSONA_SAMPLE)
            .text("vd_states", p.states.to_string());
        let r = self
            .req(reqwest::Method::POST, "/profiles")
            .timeout(Duration::from_secs(180))
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
        tracing::info!(%name, id = %made.id, instruct = %p.instruct, sample = !made.ref_audio_path.is_empty(), "created interviewer voice");
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
    fn persona_profile_needs_current_sample() {
        let mut p = prof("n");
        assert!(!persona_profile_ok(&p), "no sample: instruct-only, random speaker");
        p.ref_audio_path = "n.wav".into();
        assert!(!persona_profile_ok(&p), "old default sample is too short");
        p.ref_text = PERSONA_SAMPLE.into();
        assert!(persona_profile_ok(&p));
    }

    #[test]
    fn ws_url() {
        assert_eq!(
            VoiceStudio::new("http://127.0.0.1:3900/").stt_ws_url(16_000),
            "ws://127.0.0.1:3900/v1/audio/transcriptions/stream?pcm=1&sr=16000"
        );
    }
}
