//! The speech engine: Supertonic TTS and Parakeet STT, run inside Onsite through `sherpa-onnx`.
//! `voice/mod.rs` speaks through [`Tts`] and `stt.rs` listens through [`Engine::stt`].

use super::builtin::{self, ParakeetStt, Piece, Supertonic, Synth};
use super::health::Health;
use super::tier::Tier;
use super::speech_text::{to_speech, Lexicon};
use super::VoiceSettings;
use crate::error::{AppError, Result};
use crate::AppState;
use std::sync::atomic::{AtomicBool, Ordering};
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
    /// How tech terms are said (`prompts/pronounce.json`).
    pub lexicon: Arc<Lexicon>,
    /// This machine's tier; `Tier::Ok` until the first warm-up measures it.
    pub tier: Tier,
    /// Silero VAD model, bundled with the prompts.
    pub vad_model: std::path::PathBuf,
}

impl Engine {
    /// Loads the models if they aren't loaded yet.
    pub async fn load(state: &AppState) -> Result<Self> {
        let (tts, stt) = tokio::try_join!(state.builtin.tts(&state.data_dir), state.builtin.stt(&state.data_dir))?;
        let lexicon = Arc::new(Lexicon::load(&state.prompts_dir));
        Ok(Self { tts, stt, lexicon, tier: state.builtin.tier().unwrap_or(Tier::Ok), vad_model: state.prompts_dir.join("models/silero_vad.onnx") })
    }

    /// The voice for a round: a panel interviewer's, else the one chosen in Settings.
    pub fn voice(&self, v: &VoiceSettings) -> Tts {
        let clip = v.persona.as_ref().map_or(v.builtin_voice.as_str(), |p| p.clip.as_str());
        Tts::new(&self.tts, clip, self.lexicon.clone(), self.tier)
    }
}

pub struct Tts {
    tts: Arc<Supertonic>,
    clip: &'static str,
    lexicon: Arc<Lexicon>,
    /// Steps and piece size for one-off lines (previews).
    pub tier: Tier,
}

impl Tts {
    pub fn new(tts: &Arc<Supertonic>, clip: &str, lexicon: Arc<Lexicon>, tier: Tier) -> Self {
        Self { tts: tts.clone(), clip: builtin::known_clip(clip), lexicon, tier }
    }

    pub fn name(&self) -> String {
        builtin::VOICES.iter().find(|v| v.0 == self.clip).map_or(self.clip, |v| v.1).to_string()
    }

    /// Sample rate of the PCM handed to `on_pcm`.
    pub fn rate(&self) -> u32 {
        self.tts.rate
    }

    /// Speaks `text`, calling `on_piece` with each generated piece (PCM16 at [`Tts::rate`]). The text is first
    /// rewritten for the ear (numbers, symbols, tech terms); when nothing is left to say, no TTS call is made.
    /// Dropping the future (a cut-in) stops the generator at once.
    pub async fn speak(
        &self,
        text: &str,
        speed: f32,
        steps: i32,
        max_piece_words: usize,
        health: Arc<Health>,
        mut on_piece: impl FnMut(Piece),
    ) -> Result<()> {
        let text = to_speech(text, &self.lexicon);
        if text.is_empty() {
            return Ok(());
        }
        let synth = Synth { sid: builtin::speaker_id(self.clip), speed: builtin::safe_speed(speed), steps };
        let stop = Arc::new(AtomicBool::new(false));
        let _stop_on_drop = StopOnDrop(stop.clone());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Piece>();
        let tts = self.tts.clone();
        let job = tokio::task::spawn_blocking(move || -> Result<()> {
            for piece in builtin::pieces(&text, max_piece_words) {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let p = tts.speak_piece(&piece, &synth, &stop, &health)?;
                if tx.send(p).is_err() {
                    break;
                }
            }
            Ok(())
        });
        while let Some(p) = rx.recv().await {
            on_piece(p);
        }
        job.await.map_err(|e| AppError::Voice(e.to_string()))??;
        Ok(())
    }
}

/// Sets the stop flag when dropped, so a cancelled `speak` ends the generation in progress.
struct StopOnDrop(Arc<AtomicBool>);

impl Drop for StopOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}
