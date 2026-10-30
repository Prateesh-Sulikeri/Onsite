//! One interface over the two speech engines. `voice/mod.rs` speaks through [`Tts`] and `stt.rs`
//! listens through [`Engine`]; VoiceStudio and the built-in engine differ only in here and in `stt::run`'s segment source.

use super::builtin::{self, ParakeetStt, PocketTts};
use super::vs::{Profile, VoiceStudio, TTS_RATE};
use super::VoiceSettings;
use crate::error::{AppError, Result};
use crate::AppState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Which engine speaks and transcribes. Saved in `VoiceSettings.engine`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VoiceEngine {
    Builtin,
    Voicestudio,
}

/// Installs from before the built-in engine have no value and keep VoiceStudio. A fresh install has no
/// saved settings at all, so it gets `Default` (built-in).
pub fn legacy_engine() -> VoiceEngine {
    VoiceEngine::Voicestudio
}

pub struct Builtin {
    pub tts: Arc<PocketTts>,
    pub stt: Arc<ParakeetStt>,
}

#[derive(Clone)]
pub enum Engine {
    Studio(VoiceStudio),
    Builtin(Arc<Builtin>),
}

impl Engine {
    /// The engine chosen in settings. Loads the built-in models if they aren't loaded yet.
    pub async fn from_settings(state: &AppState, v: &VoiceSettings) -> Result<Self> {
        match v.engine {
            VoiceEngine::Voicestudio => Ok(Self::Studio(VoiceStudio::new(&v.base_url))),
            VoiceEngine::Builtin => {
                let voices = state.prompts_dir.join("voices");
                let (tts, stt) = tokio::try_join!(state.builtin.tts(&state.data_dir, &voices), state.builtin.stt(&state.data_dir))?;
                Ok(Self::Builtin(Arc::new(Builtin { tts, stt })))
            }
        }
    }

    /// The voice for a round: a panel interviewer's, else the one chosen in Settings. VoiceStudio needs a
    /// profile first (built or looked up); the built-in engine only needs a clip name.
    pub async fn voice(&self, v: &VoiceSettings) -> Result<Tts> {
        match self {
            Self::Studio(vs) => {
                let profile = match &v.persona {
                    Some(p) => vs.persona_voice(p).await?,
                    None => vs.resolve_voice(&v.voice).await?,
                };
                Ok(Tts::Studio { vs: vs.clone(), profile: Box::new(profile) })
            }
            Self::Builtin(b) => {
                let clip = v.persona.as_ref().map_or(v.builtin_voice.as_str(), |p| p.clip.as_str());
                Ok(Tts::builtin(&b.tts, clip))
            }
        }
    }
}

pub enum Tts {
    Studio { vs: VoiceStudio, profile: Box<Profile> },
    Builtin { tts: Arc<PocketTts>, clip: &'static str },
}

impl Tts {
    pub fn builtin(tts: &Arc<PocketTts>, clip: &str) -> Self {
        Self::Builtin { tts: tts.clone(), clip: builtin::known_clip(clip) }
    }

    pub fn name(&self) -> String {
        match self {
            Self::Studio { profile, .. } => profile.name.clone(),
            Self::Builtin { clip, .. } => builtin::VOICES.iter().find(|v| v.0 == *clip).map_or(*clip, |v| v.1).to_string(),
        }
    }

    /// Sample rate of the PCM handed to `on_pcm`.
    pub fn rate(&self) -> u32 {
        match self {
            Self::Studio { .. } => TTS_RATE,
            Self::Builtin { tts, .. } => tts.rate,
        }
    }

    /// Speaks `text`, calling `on_pcm` with PCM16 as it arrives. `steps` only matters to VoiceStudio.
    pub async fn speak(&self, text: &str, steps: u32, speed: f32, mut on_pcm: impl FnMut(&[i16])) -> Result<()> {
        match self {
            Self::Studio { vs, profile } => vs.speak(text, profile, steps, speed, on_pcm).await,
            Self::Builtin { tts, clip } => {
                let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<i16>>();
                let (tts, clip, text) = (tts.clone(), *clip, text.to_string());
                let job = tokio::task::spawn_blocking(move || tts.speak(&text, clip, speed, tx));
                // Dropping this future (a cut-in) drops `rx`, which makes the generator stop at its next sentence.
                while let Some(pcm) = rx.recv().await {
                    on_pcm(&pcm);
                }
                job.await.map_err(|e| AppError::Voice(e.to_string()))??;
                Ok(())
            }
        }
    }
}
