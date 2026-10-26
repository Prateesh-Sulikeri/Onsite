//! Voice turn-taking: speak the interviewer's sentences, then listen for the answer.
//! Mic stays closed while the interviewer talks (no echo handling yet).

pub mod audio;
pub mod stt;
pub mod vs;

use crate::error::{AppError, Result};
use audio::Player;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stt::{ListenCtl, ListenEnd};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use vs::{VoiceStudio, TTS_RATE};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceSettings {
    pub enabled: bool,
    pub base_url: String,
    /// VoiceStudio profile id, or "" for its default voice.
    pub voice: String,
    /// 8 = fast, 16 = balanced, 32 = best.
    pub steps: u32,
    pub speed: f32,
    /// Silence that ends an answer. 0 = I press Done.
    pub pause_ms: u32,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self { enabled: true, base_url: "http://127.0.0.1:3900".into(), voice: String::new(), steps: 8, speed: 1.0, pause_ms: 3000 }
    }
}

/// Sent to the UI as the `voice` Tauri event.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VoiceEvent {
    Speaking { active: bool },
    Listening { active: bool },
    SpeechStart,
    Partial { text: String },
    Final { text: String, stt_ms: u64 },
    /// From the end of the candidate's speech to the first interviewer audio.
    Latency { ms: u64 },
    Level { mic: f32, out: f32 },
    Error { message: String },
}

enum Cmd {
    Say(String),
    TurnDone { ended: bool },
    Listen,
}

struct RoundVoice {
    round_id: i64,
    tx: UnboundedSender<Cmd>,
    ctl: Arc<Mutex<Arc<ListenCtl>>>,
    muted: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Default)]
pub struct Voice {
    player: Mutex<Option<Arc<Player>>>,
    current: Mutex<Option<RoundVoice>>,
}

impl Voice {
    fn player(&self) -> Result<Arc<Player>> {
        let mut p = self.player.lock().unwrap();
        if let Some(p) = p.as_ref() {
            return Ok(p.clone());
        }
        let opened = Arc::new(Player::open()?);
        *p = Some(opened.clone());
        Ok(opened)
    }

    /// Starts the voice loop for a round. Speech follows `say`/`turn_done` calls.
    pub fn begin(&self, app: &AppHandle, round_id: i64, settings: VoiceSettings) -> Result<()> {
        self.end();
        let player = self.player()?;
        let vs = VoiceStudio::new(&settings.base_url);
        let (tx, mut rx) = unbounded_channel::<Cmd>();
        let ctl: Arc<Mutex<Arc<ListenCtl>>> = Default::default();
        let muted: Arc<std::sync::atomic::AtomicBool> = Default::default();
        // When the candidate stopped talking; taken at the first interviewer audio.
        let answered_at: Arc<Mutex<Option<Instant>>> = Default::default();
        let emit = {
            let app = app.clone();
            move |ev: VoiceEvent| {
                let _ = app.emit("voice", &ev);
            }
        };

        // Meter ticker for the UI waveform.
        let (meter_ctl, meter_player, meter_emit) = (ctl.clone(), player.clone(), emit.clone());
        let meter_tx = tx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(80));
            while !meter_tx.is_closed() {
                tick.tick().await;
                let mic = meter_ctl.lock().unwrap().mic_level.get();
                meter_emit(VoiceEvent::Level { mic, out: meter_player.level.get() });
            }
        });

        let (task_ctl, task_muted, task_answered) = (ctl.clone(), muted.clone(), answered_at);
        tokio::spawn(async move {
            let mut speaking = false;
            while let Some(cmd) = rx.recv().await {
                match cmd {
                    Cmd::Say(text) => {
                        if !speaking {
                            speaking = true;
                            emit(VoiceEvent::Speaking { active: true });
                        }
                        let mut rs = player.resampler_from(TTS_RATE);
                        let mut first = true;
                        let res = vs
                            .speak(&text, &settings.voice, settings.steps, settings.speed, |pcm| {
                                if first {
                                    first = false;
                                    if let Some(t) = task_answered.lock().unwrap().take() {
                                        let ms = t.elapsed().as_millis() as u64;
                                        tracing::info!(ms, "first interviewer audio after answer");
                                        emit(VoiceEvent::Latency { ms });
                                    }
                                }
                                player.push(&mut rs, pcm);
                            })
                            .await;
                        if let Err(e) = res {
                            tracing::warn!("tts failed: {e}");
                            emit(VoiceEvent::Error { message: e.to_string() });
                        }
                    }
                    Cmd::TurnDone { ended } => {
                        while !player.is_idle() {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        if speaking {
                            speaking = false;
                            emit(VoiceEvent::Speaking { active: false });
                        }
                        if !ended && !task_muted.load(std::sync::atomic::Ordering::SeqCst) {
                            listen_once(&vs, &settings, &task_ctl, &task_answered, &emit).await;
                        }
                    }
                    Cmd::Listen => listen_once(&vs, &settings, &task_ctl, &task_answered, &emit).await,
                }
            }
            player.clear();
        });

        *self.current.lock().unwrap() = Some(RoundVoice { round_id, tx, ctl, muted });
        Ok(())
    }

    fn with_round(&self, round_id: i64, f: impl FnOnce(&RoundVoice)) {
        if let Some(r) = self.current.lock().unwrap().as_ref().filter(|r| r.round_id == round_id) {
            f(r)
        }
    }

    pub fn say(&self, round_id: i64, text: &str) {
        self.with_round(round_id, |r| {
            let _ = r.tx.send(Cmd::Say(text.to_string()));
        });
    }

    pub fn turn_done(&self, round_id: i64, ended: bool) {
        self.with_round(round_id, |r| {
            let _ = r.tx.send(Cmd::TurnDone { ended });
        });
    }

    /// Ends the current answer now (manual mode, or "I'm done").
    pub fn finish_answer(&self) {
        if let Some(r) = self.current.lock().unwrap().as_ref() {
            r.ctl.lock().unwrap().finish.notify_one();
        }
    }

    /// Mute closes the mic and drops what was heard. Unmute starts listening again.
    pub fn set_muted(&self, muted: bool) {
        if let Some(r) = self.current.lock().unwrap().as_ref() {
            r.muted.store(muted, std::sync::atomic::Ordering::SeqCst);
            if muted {
                r.ctl.lock().unwrap().cancel.notify_one();
            } else {
                let _ = r.tx.send(Cmd::Listen);
            }
        }
    }

    /// The candidate typed instead: stop listening without sending speech.
    pub fn cancel_listen(&self) {
        if let Some(r) = self.current.lock().unwrap().as_ref() {
            r.ctl.lock().unwrap().cancel.notify_one();
        }
    }

    pub fn end(&self) {
        if let Some(r) = self.current.lock().unwrap().take() {
            r.ctl.lock().unwrap().cancel.notify_one();
        }
        if let Some(p) = self.player.lock().unwrap().as_ref() {
            p.clear();
        }
    }

    pub async fn test_speak(&self, settings: &VoiceSettings, text: &str) -> Result<u64> {
        let player = self.player()?;
        let vs = VoiceStudio::new(&settings.base_url);
        let started = Instant::now();
        let mut first_ms = None;
        let mut rs = player.resampler_from(TTS_RATE);
        vs.speak(text, &settings.voice, settings.steps, settings.speed, |pcm| {
            first_ms.get_or_insert(started.elapsed().as_millis() as u64);
            player.push(&mut rs, pcm);
        })
        .await?;
        first_ms.ok_or_else(|| AppError::Voice("VoiceStudio returned no audio.".into()))
    }
}

async fn listen_once(
    vs: &VoiceStudio,
    settings: &VoiceSettings,
    ctl_slot: &Mutex<Arc<ListenCtl>>,
    answered_at: &Mutex<Option<Instant>>,
    emit: &(impl Fn(VoiceEvent) + Clone),
) {
    // Fresh control per answer, so a stale finish/cancel can't end the next one.
    let ctl = Arc::new(ListenCtl::default());
    *ctl_slot.lock().unwrap() = ctl.clone();
    match stt::listen(vs, settings.pause_ms, &ctl, emit.clone()).await {
        Ok(ListenEnd::Text { text, stt_ms }) => {
            *answered_at.lock().unwrap() = Some(Instant::now() - Duration::from_millis(stt_ms));
            emit(VoiceEvent::Final { text, stt_ms });
        }
        Ok(ListenEnd::NoSpeech) | Ok(ListenEnd::Cancelled) => {}
        Err(e) => {
            tracing::warn!("listen failed: {e}");
            emit(VoiceEvent::Error { message: e.to_string() });
        }
    }
}
