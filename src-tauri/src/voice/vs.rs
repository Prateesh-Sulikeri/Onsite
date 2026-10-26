//! VoiceStudio HTTP client (loopback, port 3900). Field names checked against its live OpenAPI.

use crate::error::{AppError, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

pub const TTS_RATE: u32 = 24_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub instruct: String,
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

    /// Streams raw 24 kHz mono PCM16 to `on_pcm` as it arrives.
    pub async fn speak(&self, text: &str, voice: &str, steps: u32, speed: f32, mut on_pcm: impl FnMut(&[i16])) -> Result<()> {
        let body = json!({
            "model": "omnivoice",
            "input": text,
            "voice": if voice.is_empty() { "default" } else { voice },
            "response_format": "pcm",
            "stream_format": "audio",
            "num_step": steps,
            "speed": speed,
        });
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

    #[test]
    fn ws_url() {
        assert_eq!(
            VoiceStudio::new("http://127.0.0.1:3900/").stt_ws_url(16_000),
            "ws://127.0.0.1:3900/v1/audio/transcriptions/stream?pcm=1&sr=16000"
        );
    }
}
