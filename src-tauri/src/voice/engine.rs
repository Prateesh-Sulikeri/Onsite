//! The speech engine: Supertonic TTS and Parakeet STT, run inside Onsite through `sherpa-onnx`.
//! `voice/mod.rs` speaks through [`Tts`] and `stt.rs` listens through [`Engine::stt`].

use super::builtin::{self, ParakeetStt, Supertonic};
use super::VoiceSettings;
use crate::error::{AppError, Result};
use crate::AppState;
use std::sync::Arc;

/// A panel interviewer's voice: who they are and which shipped clip they use (`builtin::VOICES`).
#[derive(Debug, Clone, PartialEq)]
pub struct PersonaVoice {
    pub name: String,
    pub clip: String,
}

#[derive(Clone)]
pub struct Engine {
    pub tts: Arc<Supertonic>,
    pub stt: Arc<ParakeetStt>,
}

impl Engine {
    /// Loads the models if they aren't loaded yet.
    pub async fn load(state: &AppState) -> Result<Self> {
        let (tts, stt) = tokio::try_join!(state.builtin.tts(&state.data_dir), state.builtin.stt(&state.data_dir))?;
        Ok(Self { tts, stt })
    }

    /// The voice for a round: a panel interviewer's, else the one chosen in Settings.
    pub fn voice(&self, v: &VoiceSettings) -> Tts {
        let clip = v.persona.as_ref().map_or(v.builtin_voice.as_str(), |p| p.clip.as_str());
        Tts::new(&self.tts, clip)
    }
}

pub struct Tts {
    tts: Arc<Supertonic>,
    clip: &'static str,
}

impl Tts {
    pub fn new(tts: &Arc<Supertonic>, clip: &str) -> Self {
        Self { tts: tts.clone(), clip: builtin::known_clip(clip) }
    }

    pub fn name(&self) -> String {
        builtin::VOICES.iter().find(|v| v.0 == self.clip).map_or(self.clip, |v| v.1).to_string()
    }

    /// Sample rate of the PCM handed to `on_pcm`.
    pub fn rate(&self) -> u32 {
        self.tts.rate
    }

    /// Speaks `text`, calling `on_pcm` with PCM16 as it arrives.
    pub async fn speak(&self, text: &str, speed: f32, mut on_pcm: impl FnMut(&[i16])) -> Result<()> {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<i16>>();
        let (tts, clip, text) = (self.tts.clone(), self.clip, text.to_string());
        // One call per clause-sized piece, so the first audio and every later gap stay short on a slow CPU.
        let job = tokio::task::spawn_blocking(move || -> Result<()> {
            for piece in builtin::pieces(&text) {
                if tx.is_closed() {
                    break;
                }
                tts.speak(&piece, clip, speed, tx.clone())?;
            }
            Ok(())
        });
        // Dropping this future (a cut-in) drops `rx`, which makes the generator stop at its next sentence.
        while let Some(pcm) = rx.recv().await {
            on_pcm(&pcm);
        }
        job.await.map_err(|e| AppError::Voice(e.to_string()))??;
        Ok(())
    }
}
