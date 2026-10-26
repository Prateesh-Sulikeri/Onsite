//! One spoken answer: mic -> VAD -> VoiceStudio streaming STT -> final text.

use super::audio::{rms, i16_to_f32, Level, Mic, Vad, VadEvent, MIC_RATE};
use super::vs::VoiceStudio;
use super::VoiceEvent;
use crate::error::{AppError, Result};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::time::{Duration, Instant};
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message;

const FRAME: usize = (MIC_RATE / 50) as usize; // 20 ms
const PREROLL: usize = (MIC_RATE as usize * 400) / 1000;

#[derive(Default)]
pub struct ListenCtl {
    pub finish: Notify,
    pub cancel: Notify,
    pub mic_level: Level,
}

#[derive(Debug, PartialEq)]
pub enum ListenEnd {
    Text { text: String, stt_ms: u64 },
    Cancelled,
    NoSpeech,
}

/// What we have heard so far from the STT stream.
#[derive(Debug, Default)]
pub struct SttState {
    pub partial: String,
    pub finals: Vec<String>,
    pub summary: Option<String>,
}

impl SttState {
    /// Applies one server message. Returns a new partial transcript to show, if any.
    pub fn absorb(&mut self, msg: &Value) -> Result<Option<String>> {
        let text = || msg["text"].as_str().unwrap_or_default().trim().to_string();
        match msg["type"].as_str() {
            Some("partial") => {
                self.partial = text();
                Ok(Some(self.preview()))
            }
            Some("final") => {
                if msg["final_kind"].as_str() == Some("summary") {
                    self.summary = Some(text());
                } else if !text().is_empty() {
                    self.finals.push(text());
                    self.partial.clear();
                }
                Ok(Some(self.preview()))
            }
            Some("error") => {
                let detail = msg["detail"].as_str().or(msg["message"].as_str()).unwrap_or("transcription failed");
                Err(AppError::Voice(format!("VoiceStudio STT: {detail}")))
            }
            _ => Ok(None),
        }
    }

    fn preview(&self) -> String {
        let mut parts = self.finals.clone();
        if !self.partial.is_empty() {
            parts.push(self.partial.clone());
        }
        parts.join(" ")
    }

    /// The summary final is authoritative; then utterance finals; then the last partial.
    pub fn result(&self) -> String {
        match &self.summary {
            Some(s) if !s.is_empty() => s.clone(),
            _ => self.preview(),
        }
    }
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_msg(ws: &mut Option<Ws>) -> Option<std::result::Result<Message, tokio_tungstenite::tungstenite::Error>> {
    match ws.as_mut() {
        Some(w) => w.next().await,
        None => std::future::pending().await,
    }
}

fn pcm_bytes(s: &[i16]) -> Vec<u8> {
    s.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn handle(st: &mut SttState, m: Message, emit: &mut impl FnMut(VoiceEvent)) -> Result<bool> {
    match m {
        Message::Text(t) => {
            let v: Value = serde_json::from_str(t.as_str()).unwrap_or_default();
            if let Some(p) = st.absorb(&v)? {
                emit(VoiceEvent::Partial { text: p });
            }
            Ok(v["final_kind"] == "summary")
        }
        Message::Close(_) => Ok(true),
        _ => Ok(false),
    }
}

/// Listens until the pause after speech (or `finish`), then returns the transcript.
pub async fn listen(vs: &VoiceStudio, pause_ms: u32, ctl: &ListenCtl, mut emit: impl FnMut(VoiceEvent)) -> Result<ListenEnd> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<i16>>();
    let mic = Mic::open(tx)?;
    let mut vad = Vad::new(20, pause_ms);
    let mut frame: Vec<i16> = Vec::with_capacity(FRAME);
    let mut preroll: VecDeque<i16> = VecDeque::with_capacity(PREROLL + FRAME);
    let mut ws: Option<Ws> = None;
    let mut st = SttState::default();
    emit(VoiceEvent::Listening { active: true });

    loop {
        tokio::select! {
            _ = ctl.cancel.notified() => {
                emit(VoiceEvent::Listening { active: false });
                return Ok(ListenEnd::Cancelled);
            }
            _ = ctl.finish.notified() => break,
            chunk = rx.recv() => {
                let Some(chunk) = chunk else { break };
                ctl.mic_level.set(mic.level.get());
                let mut ended = false;
                let mut started = false;
                for &s in &chunk {
                    frame.push(s);
                    if frame.len() == FRAME {
                        match vad.push(rms(&i16_to_f32(&frame))) {
                            VadEvent::SpeechStart => started = true,
                            VadEvent::SpeechEnd => ended = true,
                            VadEvent::None => {}
                        }
                        frame.clear();
                    }
                }
                if started && ws.is_none() {
                    emit(VoiceEvent::SpeechStart);
                    let (mut w, _) = tokio_tungstenite::connect_async(vs.stt_ws_url(MIC_RATE))
                        .await
                        .map_err(|e| AppError::Voice(format!("VoiceStudio STT connect: {e}")))?;
                    let head: Vec<i16> = preroll.drain(..).collect();
                    w.send(Message::Binary(pcm_bytes(&head).into())).await.map_err(|e| AppError::Voice(e.to_string()))?;
                    ws = Some(w);
                }
                match ws.as_mut() {
                    Some(w) => w.send(Message::Binary(pcm_bytes(&chunk).into())).await.map_err(|e| AppError::Voice(e.to_string()))?,
                    None => {
                        preroll.extend(&chunk);
                        let extra = preroll.len().saturating_sub(PREROLL);
                        preroll.drain(..extra);
                    }
                }
                if ended {
                    break;
                }
            }
            msg = next_msg(&mut ws) => {
                match msg {
                    Some(Ok(m)) => { handle(&mut st, m, &mut emit)?; }
                    Some(Err(e)) => return Err(AppError::Voice(format!("VoiceStudio STT: {e}"))),
                    None => ws = None,
                }
            }
        }
    }

    drop(mic);
    ctl.mic_level.set(0.0);
    emit(VoiceEvent::Listening { active: false });
    let Some(mut w) = ws else { return Ok(ListenEnd::NoSpeech) };
    let ended_at = Instant::now();
    w.send(Message::Text(r#"{"type":"input_audio.end"}"#.into())).await.map_err(|e| AppError::Voice(e.to_string()))?;
    let deadline = tokio::time::sleep(Duration::from_secs(10));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _ = &mut deadline => { tracing::warn!("STT summary timed out; using what we have"); break; }
            _ = ctl.cancel.notified() => return Ok(ListenEnd::Cancelled),
            m = w.next() => match m {
                Some(Ok(m)) => if handle(&mut st, m, &mut emit)? { break },
                Some(Err(e)) => return Err(AppError::Voice(format!("VoiceStudio STT: {e}"))),
                None => break,
            }
        }
    }
    let _ = w.close(None).await;
    let text = st.result();
    let stt_ms = ended_at.elapsed().as_millis() as u64;
    tracing::info!(stt_ms, words = text.split_whitespace().count(), "utterance transcribed");
    Ok(if text.is_empty() { ListenEnd::NoSpeech } else { ListenEnd::Text { text, stt_ms } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stt_state_prefers_summary() {
        let mut s = SttState::default();
        assert_eq!(s.absorb(&json!({"type":"session.started"})).unwrap(), None);
        assert_eq!(s.absorb(&json!({"type":"partial","text":"Walk me"})).unwrap().as_deref(), Some("Walk me"));
        s.absorb(&json!({"type":"final","final_kind":"utterance","text":"Walk me through it."})).unwrap();
        assert_eq!(s.absorb(&json!({"type":"partial","text":"Then"})).unwrap().as_deref(), Some("Walk me through it. Then"));
        assert_eq!(s.result(), "Walk me through it. Then");
        s.absorb(&json!({"type":"final","final_kind":"summary","text":"Walk me through it. Then stop."})).unwrap();
        assert_eq!(s.result(), "Walk me through it. Then stop.");
        assert!(s.absorb(&json!({"type":"error","detail":"boom"})).is_err());
    }
}
