//! Machine tier: how fast this computer makes speech, measured at warm-up, and the settings each tier uses.
//! Chosen by measured speed, never by CPU name or GPU presence.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Fast,
    Ok,
    Slow,
}

impl Tier {
    /// From the real-time factor of a warm-up line (generation time / audio time).
    pub fn from_rtf(rtf: f32) -> Tier {
        if rtf < 0.35 {
            Tier::Fast
        } else if rtf < 0.7 {
            Tier::Ok
        } else {
            Tier::Slow
        }
    }

    /// Upper RTF bound of the tier (none for Slow).
    fn bound(self) -> f32 {
        match self {
            Tier::Fast => 0.35,
            Tier::Ok => 0.7,
            Tier::Slow => f32::INFINITY,
        }
    }

    /// Flow-matching steps per piece.
    pub fn steps(self) -> i32 {
        match self {
            Tier::Fast => 8,
            Tier::Ok => 6,
            Tier::Slow => 5,
        }
    }

    /// Longest piece in words: whole sentences when fast, clause cuts when slow.
    pub fn piece_words(self) -> usize {
        match self {
            Tier::Fast => 40,
            Tier::Ok => 24,
            Tier::Slow => 14,
        }
    }

    /// How often the live caption is re-decoded. None = no live caption (the text appears at the end).
    pub fn caption_every(self) -> Option<Duration> {
        match self {
            Tier::Fast => Some(Duration::from_millis(500)),
            Tier::Ok => Some(Duration::from_secs(1)),
            Tier::Slow => None,
        }
    }

    /// Audio the speaker waits for before it starts, and after a dry spell.
    pub fn prebuffer_ms(self) -> u32 {
        match self {
            Tier::Fast => 300,
            Tier::Ok => 500,
            Tier::Slow => 800,
        }
    }

    pub fn slower(self) -> Tier {
        match self {
            Tier::Fast => Tier::Ok,
            Tier::Ok | Tier::Slow => Tier::Slow,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tier::Fast => "fast",
            Tier::Ok => "okay",
            Tier::Slow => "slow",
        }
    }
}

/// Pieces looked at by [`Drift`].
const DRIFT_WINDOW: usize = 5;

/// Running check during a round: demote when the median RTF of the last 5 pieces is above the tier's bound x 1.2.
/// The median, so one slow piece (a busy moment) does not demote.
pub struct Drift {
    tier: Tier,
    recent: Vec<f32>,
}

impl Drift {
    pub fn new(tier: Tier) -> Self {
        Self { tier, recent: Vec::with_capacity(DRIFT_WINDOW) }
    }

    pub fn tier(&self) -> Tier {
        self.tier
    }

    /// Returns the new tier when this piece caused a demotion.
    pub fn piece(&mut self, rtf: f32) -> Option<Tier> {
        if self.recent.len() == DRIFT_WINDOW {
            self.recent.remove(0);
        }
        self.recent.push(rtf);
        if self.recent.len() < DRIFT_WINDOW {
            return None;
        }
        let mut sorted = self.recent.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        if sorted[DRIFT_WINDOW / 2] > self.tier.bound() * 1.2 {
            self.tier = self.tier.slower();
            self.recent.clear();
            return Some(self.tier);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_from_speed() {
        assert_eq!(Tier::from_rtf(0.2), Tier::Fast);
        assert_eq!(Tier::from_rtf(0.35), Tier::Ok);
        assert_eq!(Tier::from_rtf(0.69), Tier::Ok);
        assert_eq!(Tier::from_rtf(0.7), Tier::Slow);
        assert_eq!((Tier::Fast.steps(), Tier::Ok.steps(), Tier::Slow.steps()), (8, 6, 5));
        assert_eq!((Tier::Fast.piece_words(), Tier::Ok.piece_words(), Tier::Slow.piece_words()), (40, 24, 14));
        assert_eq!(Tier::Slow.caption_every(), None);
        assert_eq!(Tier::Slow.slower(), Tier::Slow);
    }

    #[test]
    fn drift_demotes_once_after_five_slow_pieces() {
        let mut d = Drift::new(Tier::Fast);
        for _ in 0..4 {
            assert_eq!(d.piece(0.6), None);
        }
        assert_eq!(d.piece(0.6), Some(Tier::Ok));
        // The window restarts after a demotion.
        for _ in 0..4 {
            assert_eq!(d.piece(0.6), None);
        }
        assert_eq!(d.piece(0.6), None, "0.6 is inside the Ok bound of 0.7 x 1.2");
        assert_eq!(d.tier(), Tier::Ok);
        // One slow piece among fast ones does nothing.
        let mut d = Drift::new(Tier::Fast);
        for r in [0.2, 0.2, 1.5, 0.2, 0.2] {
            assert_eq!(d.piece(r), None);
        }
    }
}
