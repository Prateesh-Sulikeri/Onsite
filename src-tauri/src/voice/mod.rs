//! Voice turn-taking: speak the interviewer's sentences, then listen for the answer.
//! Mic stays closed while the interviewer talks (no echo handling yet). Tapping the mic
//! then interrupts: the rest of the reply is dropped and the mic opens.

pub mod audio;
pub mod stt;
pub mod vs;
pub mod whisper;

use crate::error::{AppError, Result};
use audio::Player;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stt::{ListenCtl, ListenEnd};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use vs::{Profile, VoiceStudio, TTS_RATE};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceSettings {
    pub enabled: bool,
    pub base_url: String,
    /// VoiceStudio profile id, or "" for the first profile. VoiceStudio's own "default"
    /// voice is never used: it picks a new random speaker on every call.
    pub voice: String,
    /// 8 = fast, 16 = balanced, 32 = best.
    pub steps: u32,
    pub speed: f32,
    /// Silence that ends an answer. 0 = I press Done.
    pub pause_ms: u32,
    /// cpal device ids; "" = the Windows default.
    pub input_device: String,
    pub output_device: String,
    /// Set per round for panel interviewers; overrides `voice`. Never saved.
    #[serde(skip)]
    pub persona: Option<vs::PersonaVoice>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            base_url: "http://127.0.0.1:3900".into(),
            voice: String::new(),
            steps: 8,
            speed: 1.0,
            pause_ms: 3000,
            input_device: String::new(),
            output_device: String::new(),
            persona: None,
        }
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
    /// Mute cut off an answer: what was heard so far, for the typing box.
    Held { text: String },
}

/// Sentences shorter than this ("Okay.", "Got it.", "Hey Prateesh.") wait briefly for
/// the next one and go to VoiceStudio together. Fewer tiny calls means less drift and
/// fewer gaps.
const SHORT_WORDS: usize = 4;
const SHORT_CHARS: usize = 24;
/// Never merge past this, so one call never gets slow.
const MAX_BATCH_CHARS: usize = 220;
/// Extra wait for the next sentence when nothing is playing (the first audio of a reply).
const WAIT_IDLE: Duration = Duration::from_millis(150);
/// Extra wait while earlier audio is still playing; it hides behind that audio.
const WAIT_BUSY: Duration = Duration::from_millis(300);

fn is_short(s: &str) -> bool {
    s.split_whitespace().count() < SHORT_WORDS || s.trim().chars().count() < SHORT_CHARS
}

/// Text for one TTS call, built from one or more consecutive sentences.
#[derive(Debug, PartialEq)]
struct Batch(String);

impl Batch {
    fn new(s: &str) -> Self {
        Self(s.trim().to_string())
    }
    fn wants_more(&self) -> bool {
        is_short(&self.0)
    }
    /// Appends `s` if it fits; false means "speak what you have, then start a new batch".
    fn push(&mut self, s: &str) -> bool {
        let s = s.trim();
        if self.0.chars().count() + 1 + s.chars().count() > MAX_BATCH_CHARS {
            return false;
        }
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        self.0.push_str(s);
        true
    }
}

fn coalesce_wait(player_idle: bool) -> Duration {
    if player_idle {
        WAIT_IDLE
    } else {
        WAIT_BUSY
    }
}

/// What to do with a cancelled answer: keep it only when mute cut it off.
fn held_event(muted: bool, heard: &str) -> Option<VoiceEvent> {
    let text = heard.trim();
    (muted && !text.is_empty()).then(|| VoiceEvent::Held { text: text.to_string() })
}

enum Cmd {
    Say(String),
    TurnDone { ended: bool },
    /// Open the mic again (unmute, orb tap). Ignored unless an answer is due.
    Listen,
    /// An answer reached the interviewer (typed or spoken): stop waiting for one.
    AnswerSent,
}

/// Whether a `Listen` request should open the mic: only while an answer is due and not muted.
fn should_listen(awaiting_answer: bool, muted: bool) -> bool {
    awaiting_answer && !muted
}

struct RoundVoice {
    round_id: i64,
    tx: UnboundedSender<Cmd>,
    ctl: Arc<Mutex<Arc<ListenCtl>>>,
    muted: Arc<AtomicBool>,
    /// Why the current answer is being cancelled: true = mute (keep the text), false = typed or ended.
    hold: Arc<AtomicBool>,
    /// The round's voice task. Aborted on end, so queued sentences are never spoken after hang-up.
    task: tokio::task::JoinHandle<()>,
    /// Cleared on end; stops audio already being received from reaching the speaker.
    alive: Arc<AtomicBool>,
    /// True while the interviewer's reply is being spoken (from first audio to turn end).
    speaking: Arc<AtomicBool>,
    /// The candidate cut in: drop the rest of this reply. Reset when the turn ends.
    cut: Arc<AtomicBool>,
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
        stt::reset_model_choice();
        // Normally already loaded at app start; restarts it if it died.
        whisper::start();
        let player = self.player()?;
        let vs = VoiceStudio::new(&settings.base_url);
        let (tx, mut rx) = unbounded_channel::<Cmd>();
        let ctl: Arc<Mutex<Arc<ListenCtl>>> = Default::default();
        let muted: Arc<AtomicBool> = Default::default();
        let hold: Arc<AtomicBool> = Default::default();
        // When the candidate stopped talking; taken at the first interviewer audio.
        let answered_at: Arc<Mutex<Option<Instant>>> = Default::default();
        let emit = {
            let app = app.clone();
            move |ev: VoiceEvent| {
                let _ = app.emit("voice", &ev);
            }
        };

        // Speaker meter for the UI waveform; listen() reports the mic itself. A weak sender,
        // so the meter stops (and the loop below sees its channel close) once `end()` drops the round.
        let (meter_player, meter_emit, meter_tx) = (player.clone(), emit.clone(), tx.downgrade());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(80));
            while meter_tx.upgrade().is_some() {
                tick.tick().await;
                let out = meter_player.level.get();
                if out > 0.0 {
                    meter_emit(VoiceEvent::Level { mic: 0.0, out });
                }
            }
        });

        let (task_ctl, task_muted, task_hold, task_answered) = (ctl.clone(), muted.clone(), hold.clone(), answered_at);
        let alive = Arc::new(AtomicBool::new(true));
        let task_alive = alive.clone();
        let (speaking_flag, cut): (Arc<AtomicBool>, Arc<AtomicBool>) = Default::default();
        let (task_speaking, task_cut) = (speaking_flag.clone(), cut.clone());
        let task = tokio::spawn(async move {
            // One concrete profile for the whole round, so every sentence is the same voice.
            let resolved = match &settings.persona {
                Some(p) => vs.persona_voice(p).await,
                None => vs.resolve_voice(&settings.voice).await,
            };
            let voice = match resolved {
                Ok(p) => {
                    tracing::info!(id = %p.id, name = %p.name, seed = ?p.seed, "interviewer voice");
                    Some(p)
                }
                Err(e) => {
                    tracing::warn!("no interviewer voice: {e}");
                    emit(VoiceEvent::Error { message: e.to_string() });
                    None
                }
            };
            // VoiceStudio drops its weights after ~15 min idle (next call ~10 s, else ~1 s), and
            // `/model/status` can't tell us which. Warm up while Claude writes its first reply;
            // sentences queue meanwhile.
            // Skipped when VoiceStudio spoke recently (app start, last round): it adds ~1-2 s
            // before the first line.
            if let Some(v) = voice.as_ref().filter(|_| !vs::spoke_within(Duration::from_secs(300))) {
                let t = Instant::now();
                match vs.speak("Okay, let's begin.", v, settings.steps, settings.speed, |_| {}).await {
                    Ok(()) => tracing::info!(ms = t.elapsed().as_millis() as u64, "VoiceStudio warm for the round"),
                    Err(e) => tracing::warn!("VoiceStudio warm-up failed: {e}"),
                }
            }
            let mut speaking = false;
            // True from the end of an interviewer turn until an answer is sent.
            let mut awaiting_answer = false;
            let mut pending: Option<Cmd> = None;
            loop {
                let cmd = match pending.take() {
                    Some(c) => c,
                    None => match rx.recv().await {
                        Some(c) => c,
                        None => break,
                    },
                };
                match cmd {
                    Cmd::Say(text) => {
                        if task_cut.load(Ordering::SeqCst) {
                            continue;
                        }
                        // Merge short sentences with the next one if it comes soon. Order is kept:
                        // anything that doesn't fit waits in `pending` and goes next.
                        // One deadline per batch, so a chain of short lines can't add up.
                        let mut batch = Batch::new(&text);
                        let deadline = tokio::time::Instant::now() + coalesce_wait(player.is_idle());
                        while batch.wants_more() {
                            match tokio::time::timeout_at(deadline, rx.recv()).await {
                                Ok(Some(Cmd::Say(t))) => {
                                    if !batch.push(&t) {
                                        pending = Some(Cmd::Say(t));
                                        break;
                                    }
                                }
                                Ok(Some(other)) => {
                                    pending = Some(other);
                                    break;
                                }
                                Ok(None) | Err(_) => break,
                            }
                        }
                        let Some(voice) = voice.as_ref() else { continue };
                        if batch.0.is_empty() {
                            continue;
                        }
                        if !speaking {
                            speaking = true;
                            task_speaking.store(true, Ordering::SeqCst);
                            emit(VoiceEvent::Speaking { active: true });
                        }
                        // Playback runs from the Player queue, so the next batch is generated
                        // while this one plays. A cut-in abandons the call still generating.
                        tokio::select! {
                            _ = speak_into(&vs, &player, voice, &settings, &batch.0, &task_answered, &task_alive, &task_cut, &emit) => {}
                            _ = until_set(&task_cut) => {}
                        }
                    }
                    Cmd::TurnDone { ended } => {
                        while !player.is_idle() && !task_cut.load(Ordering::SeqCst) {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        if task_cut.swap(false, Ordering::SeqCst) {
                            player.clear();
                        }
                        // Always sent, even when nothing was spoken (a code-only reply, or TTS
                        // failed), so the UI never waits for audio that isn't coming.
                        speaking = false;
                        task_speaking.store(false, Ordering::SeqCst);
                        emit(VoiceEvent::Speaking { active: false });
                        awaiting_answer = !ended;
                        if should_listen(awaiting_answer, task_muted.load(Ordering::SeqCst)) {
                            awaiting_answer = !listen_once(&vs, &settings, &task_ctl, &task_hold, &task_answered, &emit).await;
                        }
                    }
                    Cmd::Listen => {
                        if should_listen(awaiting_answer, task_muted.load(Ordering::SeqCst)) {
                            awaiting_answer = !listen_once(&vs, &settings, &task_ctl, &task_hold, &task_answered, &emit).await;
                        }
                    }
                    Cmd::AnswerSent => {
                        awaiting_answer = false;
                        task_cut.store(false, Ordering::SeqCst);
                    }
                }
            }
            player.clear();
        });

        *self.current.lock().unwrap() = Some(RoundVoice { round_id, tx, ctl, muted, hold, task, alive, speaking: speaking_flag, cut });
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

    /// Mute closes the mic; what was heard so far comes back as `Held`. Unmute listens again.
    pub fn set_muted(&self, muted: bool) {
        if let Some(r) = self.current.lock().unwrap().as_ref() {
            r.muted.store(muted, Ordering::SeqCst);
            if muted {
                r.hold.store(true, Ordering::SeqCst);
                r.ctl.lock().unwrap().cancel.notify_one();
            } else {
                let _ = r.tx.send(Cmd::Listen);
            }
        }
    }

    /// The candidate tapped the mic while the interviewer was talking: stop the reply now.
    /// The mic opens as soon as the interviewer's turn has ended.
    pub fn interrupt(&self) {
        if let Some(r) = self.current.lock().unwrap().as_ref().filter(|r| r.speaking.load(Ordering::SeqCst)) {
            r.cut.store(true, Ordering::SeqCst);
            if let Some(p) = self.player.lock().unwrap().as_ref() {
                p.clear();
            }
        }
    }

    /// Another mic was picked: reopen the mic if it is waiting for an answer. What was heard
    /// on the old mic is dropped, so the UI only calls this before speech starts.
    pub fn relisten(&self) {
        if let Some(r) = self.current.lock().unwrap().as_ref() {
            r.hold.store(false, Ordering::SeqCst);
            r.ctl.lock().unwrap().cancel.notify_one();
            let _ = r.tx.send(Cmd::Listen);
        }
    }

    /// An answer reached the interviewer, so unmuting must not open the mic until the next turn.
    pub fn answer_sent(&self, round_id: i64) {
        self.with_round(round_id, |r| {
            let _ = r.tx.send(Cmd::AnswerSent);
        });
    }

    /// The candidate typed instead: stop listening without sending speech.
    pub fn cancel_listen(&self) {
        if let Some(r) = self.current.lock().unwrap().as_ref() {
            r.hold.store(false, Ordering::SeqCst);
            r.ctl.lock().unwrap().cancel.notify_one();
        }
    }

    pub fn end(&self) {
        if let Some(r) = self.current.lock().unwrap().take() {
            r.alive.store(false, Ordering::SeqCst);
            r.task.abort();
            r.hold.store(false, Ordering::SeqCst);
            r.ctl.lock().unwrap().cancel.notify_one();
        }
        if let Some(p) = self.player.lock().unwrap().as_ref() {
            p.clear();
        }
    }

    pub async fn test_speak(&self, settings: &VoiceSettings, text: &str) -> Result<u64> {
        let player = self.player()?;
        let vs = VoiceStudio::new(&settings.base_url);
        let voice = vs.resolve_voice(&settings.voice).await?;
        let started = Instant::now();
        let mut first_ms = None;
        let mut rs = player.resampler_from(TTS_RATE);
        vs.speak(text, &voice, settings.steps, settings.speed, |pcm| {
            first_ms.get_or_insert(started.elapsed().as_millis() as u64);
            player.push(&mut rs, pcm);
        })
        .await?;
        first_ms.ok_or_else(|| AppError::Voice("VoiceStudio returned no audio.".into()))
    }
}

/// One TTS call into the speaker queue. Errors are reported, never replaced by another voice.
#[allow(clippy::too_many_arguments)]
async fn speak_into(
    vs: &VoiceStudio,
    player: &Player,
    voice: &Profile,
    settings: &VoiceSettings,
    text: &str,
    answered_at: &Mutex<Option<Instant>>,
    alive: &AtomicBool,
    cut: &AtomicBool,
    emit: &impl Fn(VoiceEvent),
) {
    let mut rs = player.resampler_from(TTS_RATE);
    let started = Instant::now();
    let mut first_ms: Option<u64> = None;
    let res = vs
        .speak(text, voice, settings.steps, settings.speed, |pcm| {
            if !alive.load(Ordering::SeqCst) || cut.load(Ordering::SeqCst) {
                return;
            }
            if first_ms.is_none() {
                first_ms = Some(started.elapsed().as_millis() as u64);
                if let Some(t) = answered_at.lock().unwrap().take() {
                    let ms = t.elapsed().as_millis() as u64;
                    tracing::info!(ms, "first interviewer audio after answer");
                    emit(VoiceEvent::Latency { ms });
                }
            }
            player.push(&mut rs, pcm);
        })
        .await;
    tracing::info!(chars = text.chars().count(), first_ms, total_ms = started.elapsed().as_millis() as u64, "tts");
    if let Err(e) = res {
        tracing::warn!("tts failed: {e}");
        emit(VoiceEvent::Error { message: e.to_string() });
    }
}

/// Resolves once `flag` is set. Polled, so a cut-in from an earlier turn can't leave a stale wake-up.
async fn until_set(flag: &AtomicBool) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// Listens for one answer. True when it produced text (the answer is no longer due).
async fn listen_once(
    vs: &VoiceStudio,
    settings: &VoiceSettings,
    ctl_slot: &Mutex<Arc<ListenCtl>>,
    hold: &AtomicBool,
    answered_at: &Mutex<Option<Instant>>,
    emit: &(impl Fn(VoiceEvent) + Clone),
) -> bool {
    // Fresh control per answer, so a stale finish/cancel can't end the next one.
    let ctl = Arc::new(ListenCtl::default());
    hold.store(false, Ordering::SeqCst);
    *ctl_slot.lock().unwrap() = ctl.clone();
    match stt::listen(vs, settings.pause_ms, &ctl, emit.clone()).await {
        Ok(ListenEnd::Text { text, stt_ms }) => {
            *answered_at.lock().unwrap() = Some(Instant::now() - Duration::from_millis(stt_ms));
            emit(VoiceEvent::Final { text, stt_ms });
            return true;
        }
        Ok(ListenEnd::Cancelled { heard }) => {
            if let Some(ev) = held_event(hold.swap(false, Ordering::SeqCst), &heard) {
                emit(ev);
            }
        }
        Ok(ListenEnd::NoSpeech) => {}
        Err(e) => {
            tracing::warn!("listen failed: {e}");
            emit(VoiceEvent::Error { message: e.to_string() });
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_sentences() {
        assert!(is_short("Okay."));
        assert!(is_short("Got it."));
        assert!(is_short("Hey Prateesh."));
        assert!(!is_short("Let's talk a little about your background."));
    }

    #[test]
    fn batch_merges_short_then_stops() {
        let mut b = Batch::new(" Okay. ");
        assert!(b.wants_more());
        assert!(b.push("Got it."));
        assert!(b.wants_more());
        assert!(b.push("Let's start with introductions and your core expertise."));
        assert_eq!(b.0, "Okay. Got it. Let's start with introductions and your core expertise.");
        assert!(!b.wants_more());
    }

    #[test]
    fn batch_caps_length() {
        let mut b = Batch::new("Okay.");
        let long = "word ".repeat(60);
        assert!(!b.push(&long));
        assert_eq!(b.0, "Okay.");
    }

    #[test]
    fn first_audio_waits_at_most_150ms() {
        assert!(coalesce_wait(true) <= Duration::from_millis(150));
        assert!(coalesce_wait(false) >= coalesce_wait(true));
    }

    #[test]
    fn held_only_on_mute_with_text() {
        assert_eq!(held_event(true, " I'd use a hash map "), Some(VoiceEvent::Held { text: "I'd use a hash map".into() }));
        assert_eq!(held_event(true, "  "), None);
        assert_eq!(held_event(false, "typed instead"), None);
    }

    #[test]
    fn listen_only_when_an_answer_is_due() {
        assert!(should_listen(true, false));
        assert!(!should_listen(true, true), "muted");
        assert!(!should_listen(false, false), "interviewer still talking or thinking");
    }

    #[test]
    fn held_serde_kind() {
        let v = serde_json::to_value(VoiceEvent::Held { text: "hi".into() }).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "held", "text": "hi"}));
        let v = serde_json::to_value(VoiceEvent::SpeechStart).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "speech_start"}));
    }
}
