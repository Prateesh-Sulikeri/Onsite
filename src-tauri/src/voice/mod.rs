//! Voice turn-taking: speak the interviewer's sentences, then listen for the answer.
//! Mic stays closed while the interviewer talks (no echo handling yet). Tapping the mic
//! then interrupts: the rest of the reply is dropped and the mic opens.

pub mod audio;
pub mod builtin;
pub mod engine;
pub mod health;
pub mod models;
pub mod score;
pub mod speech_text;
pub mod stt;
pub mod tier;
pub mod vocab;

use crate::error::{AppError, Result};
use audio::Player;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stt::{ListenCtl, ListenEnd};
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use tokio::sync::oneshot;
use engine::{Engine, PersonaVoice, Tts};
use health::{Health, Summary};
use tier::{Drift, Tier};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceSettings {
    /// Off = typed rounds (chat only).
    pub enabled: bool,
    /// The interviewer speaks but the candidate types: the round starts muted and the mic isn't checked.
    /// Unmuting still opens the mic.
    #[serde(default)]
    pub listen_only: bool,
    /// The voice for rounds without a panel interviewer (a clip id from `builtin::VOICES`).
    pub builtin_voice: String,
    pub speed: f32,
    /// Silence that ends an answer. 0 = I press Done.
    pub pause_ms: u32,
    /// cpal device ids; "" = the Windows default.
    pub input_device: String,
    pub output_device: String,
    /// Set per round for panel interviewers; overrides `builtin_voice`. Never saved.
    #[serde(skip)]
    pub persona: Option<PersonaVoice>,    /// Names and terms for this round (resume, job description, dossier, setup). Never saved.
    #[serde(skip)]
    pub vocab: Vec<String>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            listen_only: false,
            builtin_voice: builtin::DEFAULT_VOICE.into(),
            speed: 1.0,
            pause_ms: 3000,
            input_device: String::new(),
            output_device: String::new(),
            persona: None,
            vocab: Vec::new(),
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

/// Measures this machine's tier: speak `TIER_LINE` twice at Fast settings and time the second run.
const TIER_LINE: &str = "Okay, let's begin. Thanks for joining me today.";

/// Sentences shorter than this ("Okay.", "Got it.", "Hey Prateesh.") wait briefly for
/// the next one and go to the TTS together. Fewer tiny calls means fewer gaps.
const SHORT_WORDS: usize = 4;
const SHORT_CHARS: usize = 24;
/// Never merge past this, so one call never gets slow.
const MAX_BATCH_CHARS: usize = 220;
/// Extra wait for the next sentence when nothing is playing (the first audio of a reply).
const WAIT_IDLE: Duration = Duration::from_millis(150);
/// Extra wait while earlier audio is still playing; it hides behind that audio.
const WAIT_BUSY: Duration = Duration::from_millis(300);
/// A reply that opens with one or two words ("Okay.") waits this long for the next sentence. Spoken
/// alone, it played in 0.6 s while the next sentence took 1.7-2.9 s to make: "Okay." ... gap ... rest.
const WAIT_TINY: Duration = Duration::from_millis(700);
const TINY_WORDS: usize = 2;

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

fn coalesce_wait(player_idle: bool, text: &str) -> Duration {
    match (player_idle, text.split_whitespace().count() <= TINY_WORDS) {
        (true, true) => WAIT_TINY,
        (true, false) => WAIT_IDLE,
        (false, _) => WAIT_BUSY,
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

/// From [`Voice::begin`]: `ready` gives the interviewer's voice name once the engine has spoken
/// with it (or why it couldn't); sending `go` lets queued speech play.
pub struct VoiceStart {
    pub ready: oneshot::Receiver<std::result::Result<String, String>>,
    pub go: oneshot::Sender<()>,
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
    /// An answer was sent for the turn in progress. Set at once by `answer_sent`, so a turn end that is still
    /// queued (audio still playing) doesn't open the mic after the answer is already in.
    sent: Arc<AtomicBool>,
    /// Counters for the end-of-round `voice health` log line.
    health: Arc<Health>,
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

    /// Starts the voice loop for a round. Speech follows `say`/`turn_done` calls, but nothing
    /// plays until `go` is sent (or dropped), so the loader can finish first.
    pub fn begin(&self, app: &AppHandle, round_id: i64, settings: VoiceSettings, engine: Engine, cache: Arc<builtin::Cache>) -> Result<VoiceStart> {
        self.end();
        let player = self.player()?;
        let (tx, mut rx) = unbounded_channel::<Cmd>();
        let (ready_tx, ready) = oneshot::channel();
        let (go, go_rx) = oneshot::channel::<()>();
        let ctl: Arc<Mutex<Arc<ListenCtl>>> = Default::default();
        let muted = Arc::new(AtomicBool::new(settings.listen_only));
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
        let sent: Arc<AtomicBool> = Default::default();
        let task_sent = sent.clone();
        let health: Arc<Health> = Default::default();
        let task_health = health.clone();
        let task = tokio::spawn(async move {
            // One concrete voice for the whole round, so every sentence is the same speaker.
            let voice = engine.voice(&settings);
            tracing::info!(name = %voice.name(), "interviewer voice");
            // Warm up while Claude writes its first reply (loads the voice clip, ~1 s); sentences queue meanwhile.
            // Sent to the loader once the voice has spoken (or can't).
            let t = Instant::now();
            let warmed = match cache.tier() {
                Some(_) => voice.speak("Okay, let's begin.", settings.speed, engine.tier.steps(), engine.tier.piece_words(), Default::default(), |_| {}).await,
                None => measure_tier(&voice, &cache).await,
            };
            let mut drift = Drift::new(cache.tier().unwrap_or(engine.tier));
            let warm = match warmed {
                Ok(()) => {
                    tracing::info!(ms = t.elapsed().as_millis() as u64, "voice warm for the round");
                    Ok(voice.name())
                }
                Err(e) => {
                    tracing::warn!("voice warm-up failed: {e}");
                    Err(e.to_string())
                }
            };
            let _ = ready_tx.send(warm);
            let _ = go_rx.await;
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
                        let deadline = tokio::time::Instant::now() + coalesce_wait(player.is_idle(), &batch.0);
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
                        if batch.0.is_empty() {
                            continue;
                        }
                        if !speaking {
                            speaking = true;
                            // The reply is still being generated: playback prebuffers and waits out dry spells.
                            player.set_prebuffer_ms(drift.tier().prebuffer_ms());
                            player.set_streaming(true);
                            task_speaking.store(true, Ordering::SeqCst);
                            task_health.reply();
                            emit(VoiceEvent::Speaking { active: true });
                        }
                        // Playback runs from the Player queue, so the next batch is generated
                        // while this one plays. A cut-in abandons the call still generating.
                        let tier = drift.tier();
                        let rtf = tokio::select! {
                            r = speak_into(&player, &voice, &settings, tier, &batch.0, &task_answered, &task_alive, &task_cut, &task_health, &emit) => r,
                            _ = until_set(&task_cut) => None,
                        };
                        if let Some(slower) = rtf.and_then(|r| drift.piece(r)) {
                            cache.set_tier(slower);
                            tracing::info!(tier = slower.label(), "voice tier lowered during the round");
                        }
                    }
                    Cmd::TurnDone { ended } => {
                        // Every part is generated by now (Say is handled in order), so play out what is queued.
                        player.set_streaming(false);
                        while !player.is_idle() && !task_cut.load(Ordering::SeqCst) {
                            tokio::time::sleep(Duration::from_millis(50)).await;
                        }
                        let dry = player.take_underruns();
                        task_health.dry_spells(dry);
                        if dry > 0 {
                            tracing::info!(dry, "reply playback ran dry");
                        }
                        if task_cut.swap(false, Ordering::SeqCst) {
                            player.clear();
                        }
                        // Always sent, even when nothing was spoken (a code-only reply, or TTS
                        // failed), so the UI never waits for audio that isn't coming.
                        speaking = false;
                        task_speaking.store(false, Ordering::SeqCst);
                        emit(VoiceEvent::Speaking { active: false });
                        // An answer already sent for this turn means the mic stays closed.
                        let answered = task_sent.swap(false, Ordering::SeqCst);
                        awaiting_answer = !ended && !answered;
                        if should_listen(awaiting_answer, task_muted.load(Ordering::SeqCst)) {
                            awaiting_answer = !listen_once(&engine, &settings, &task_ctl, &task_hold, &task_answered, &emit).await;
                        }
                    }
                    Cmd::Listen => {
                        if should_listen(awaiting_answer, task_muted.load(Ordering::SeqCst)) {
                            awaiting_answer = !listen_once(&engine, &settings, &task_ctl, &task_hold, &task_answered, &emit).await;
                        }
                    }
                    Cmd::AnswerSent => {
                        awaiting_answer = false;
                        task_sent.store(false, Ordering::SeqCst);
                        task_cut.store(false, Ordering::SeqCst);
                    }
                }
            }
            player.clear();
        });

        *self.current.lock().unwrap() = Some(RoundVoice { round_id, tx, ctl, muted, hold, task, alive, speaking: speaking_flag, cut, sent, health });
        Ok(VoiceStart { ready, go })
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
            // Set now, not when the loop reads the command: the loop may be about to open the mic for this turn.
            r.sent.store(true, Ordering::SeqCst);
            r.hold.store(false, Ordering::SeqCst);
            r.ctl.lock().unwrap().cancel.notify_one();
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
            let h = r.health.summary();
            if h.pieces > 0 {
                let Summary { replies, pieces, dry_spells, cap_hits, retries, rtf_p50, rtf_p90 } = h;
                tracing::info!(replies, pieces, dry_spells, cap_hits, retries, rtf_p50, rtf_p90, "voice health");
            }
            r.alive.store(false, Ordering::SeqCst);
            r.task.abort();
            r.hold.store(false, Ordering::SeqCst);
            r.ctl.lock().unwrap().cancel.notify_one();
        }
        if let Some(p) = self.player.lock().unwrap().as_ref() {
            p.clear();
        }
    }

    /// Speaks `text` once in the voice chosen in `settings`.
    pub async fn test_speak(&self, engine: &Engine, settings: &VoiceSettings, text: &str) -> Result<u64> {
        self.speak_as(&engine.voice(settings), settings, text).await
    }

    /// Speaks `text` once in a given voice (a panel interviewer's preview). Returns ms to first audio.
    pub async fn speak_as(&self, voice: &Tts, settings: &VoiceSettings, text: &str) -> Result<u64> {
        let player = self.player()?;
        let started = Instant::now();
        let mut first_ms = None;
        let mut rs = player.resampler_from(voice.rate());
        voice
            .speak(text, settings.speed, voice.tier.steps(), voice.tier.piece_words(), Default::default(), |piece| {
                first_ms.get_or_insert(started.elapsed().as_millis() as u64);
                player.push(&mut rs, &piece.pcm);
            })
            .await?;
        player.flush(&mut rs);
        first_ms.ok_or_else(|| AppError::Voice("The voice returned no audio.".into()))
    }
}

/// Speaks [`TIER_LINE`] twice at Fast settings (audio discarded), times the second run and stores the tier.
async fn measure_tier(voice: &Tts, cache: &builtin::Cache) -> Result<()> {
    let mut rtf = 0.0;
    for _ in 0..2 {
        let started = Instant::now();
        let mut samples = 0usize;
        voice.speak(TIER_LINE, 1.0, Tier::Fast.steps(), Tier::Fast.piece_words(), Default::default(), |p| samples += p.pcm.len()).await?;
        rtf = started.elapsed().as_secs_f32() / (samples as f32 / voice.rate().max(1) as f32).max(0.01);
    }
    let tier = Tier::from_rtf(rtf);
    cache.set_tier(tier);
    tracing::info!(rtf, tier = tier.label(), "voice tier");
    Ok(())
}

/// True when held audio goes to the speaker now: every piece on Fast, else at a sentence end
/// or at the end of the call.
fn release_now(tier: Tier, ends_sentence: bool, last: bool) -> bool {
    tier == Tier::Fast || ends_sentence || last
}

/// One TTS call into the speaker queue. Errors are reported, never replaced by another voice.
/// Returns the call's real-time factor when it produced audio.
#[allow(clippy::too_many_arguments)]
async fn speak_into(
    player: &Player,
    voice: &Tts,
    settings: &VoiceSettings,
    tier: Tier,
    text: &str,
    answered_at: &Mutex<Option<Instant>>,
    alive: &AtomicBool,
    cut: &AtomicBool,
    health: &Arc<Health>,
    emit: &impl Fn(VoiceEvent),
) -> Option<f32> {
    let mut rs = player.resampler_from(voice.rate());
    let started = Instant::now();
    let mut first_ms: Option<u64> = None;
    let mut samples = 0usize;
    // Pieces held until a sentence ends, so any pause falls between sentences (not on Fast).
    let mut held: Vec<i16> = Vec::new();
    let mut release = |pcm: &[i16], first_ms: &mut Option<u64>| {
        if first_ms.is_none() {
            *first_ms = Some(started.elapsed().as_millis() as u64);
            if let Some(t) = answered_at.lock().unwrap().take() {
                let ms = t.elapsed().as_millis() as u64;
                tracing::info!(ms, "first interviewer audio after answer");
                emit(VoiceEvent::Latency { ms });
            }
        }
        player.push(&mut rs, pcm);
    };
    let res = voice
        .speak(text, settings.speed, tier.steps(), tier.piece_words(), health.clone(), |piece| {
            samples += piece.pcm.len();
            if !alive.load(Ordering::SeqCst) || cut.load(Ordering::SeqCst) {
                held.clear();
                return;
            }
            held.extend_from_slice(&piece.pcm);
            if release_now(tier, piece.ends_sentence, false) {
                release(&held, &mut first_ms);
                held.clear();
            }
        })
        .await;
    // rtf > 1 = speech is made slower than it plays: the reply will pause between parts.
    if alive.load(Ordering::SeqCst) && !cut.load(Ordering::SeqCst) {
        if release_now(tier, false, true) && !held.is_empty() {
            release(&held, &mut first_ms);
        }
        player.flush(&mut rs);
    }
    let (total_ms, audio_ms) = (started.elapsed().as_millis() as u64, samples as u64 * 1000 / voice.rate().max(1) as u64);
    let rtf_value = total_ms as f32 / audio_ms.max(1) as f32;
    if audio_ms > 0 {
        health.piece(rtf_value);
    }
    let rtf = format!("{rtf_value:.2}");
    tracing::info!(chars = text.chars().count(), first_ms, total_ms, audio_ms, rtf, "tts");
    if let Err(e) = res {
        tracing::warn!("tts failed: {e}");
        emit(VoiceEvent::Error { message: e.to_string() });
    }
    (audio_ms > 0).then_some(rtf_value)
}

/// Resolves once `flag` is set. Polled, so a cut-in from an earlier turn can't leave a stale wake-up.
async fn until_set(flag: &AtomicBool) {
    while !flag.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
}

/// Listens for one answer. True when it produced text (the answer is no longer due).
async fn listen_once(
    engine: &Engine,
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
    let refs: Vec<&str> = settings.vocab.iter().map(String::as_str).collect();
    let opts = stt::SttOptions { vocab: stt::domain_vocab(&refs), caption_every: engine.tier.caption_every(), vad_model: Some(engine.vad_model.clone()) };
    match stt::listen_with(engine, settings.pause_ms, &ctl, &opts, emit.clone()).await {
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
        let first = "Let's start with your current project.";
        assert!(coalesce_wait(true, first) <= Duration::from_millis(150));
        assert!(coalesce_wait(false, first) >= coalesce_wait(true, first));
    }

    #[test]
    fn a_one_word_opener_waits_for_the_next_sentence() {
        assert_eq!(coalesce_wait(true, "Okay."), WAIT_TINY);
        assert_eq!(coalesce_wait(true, "Got it."), WAIT_TINY);
        assert_eq!(coalesce_wait(true, "Got it, thanks."), WAIT_IDLE);
        assert_eq!(coalesce_wait(false, "Okay."), WAIT_BUSY, "mid-reply it hides behind the audio playing");
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
    fn old_voicestudio_settings_still_load() {
        // Installs from the VoiceStudio days saved engine, base_url, voice and steps. They are ignored now.
        let old: VoiceSettings =
            serde_json::from_str(r#"{"enabled":true,"engine":"voicestudio","steps":16,"base_url":"http://127.0.0.1:3900","voice":"f34","speed":1.1}"#).unwrap();
        assert!(old.enabled);
        assert_eq!(old.builtin_voice, builtin::DEFAULT_VOICE);
        assert_eq!(old.speed, 1.1);
        let chosen: VoiceSettings = serde_json::from_str(r#"{"engine":"builtin","builtin_voice":"priya_shah"}"#).unwrap();
        assert_eq!(chosen.builtin_voice, "priya_shah");
        let saved = serde_json::to_value(VoiceSettings::default()).unwrap();
        assert!(saved.get("engine").is_none() && saved.get("base_url").is_none());
    }

    #[test]
    fn slower_machines_release_whole_sentences() {
        assert!(release_now(Tier::Fast, false, false));
        assert!(!release_now(Tier::Ok, false, false));
        assert!(release_now(Tier::Ok, true, false));
        assert!(!release_now(Tier::Slow, false, false));
        assert!(release_now(Tier::Slow, false, true), "the end of the call always flushes");
    }

    #[test]
    fn held_serde_kind() {
        let v = serde_json::to_value(VoiceEvent::Held { text: "hi".into() }).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "held", "text": "hi"}));
        let v = serde_json::to_value(VoiceEvent::SpeechStart).unwrap();
        assert_eq!(v, serde_json::json!({"kind": "speech_start"}));
    }
}
