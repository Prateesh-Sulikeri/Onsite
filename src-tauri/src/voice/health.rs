//! Per-round voice counters, logged as one line when the round ends. No speech text, only counts.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

#[derive(Default)]
pub struct Health {
    replies: AtomicU32,
    dry_spells: AtomicU32,
    cap_hits: AtomicU32,
    retries: AtomicU32,
    rtf: Mutex<Vec<f32>>,
}

#[derive(Debug, PartialEq)]
pub struct Summary {
    pub replies: u32,
    pub pieces: u32,
    pub dry_spells: u32,
    pub cap_hits: u32,
    pub retries: u32,
    pub rtf_p50: f32,
    pub rtf_p90: f32,
}

/// Nearest-rank percentile of a sorted list; 0.0 when empty.
fn nearest_rank(sorted: &[f32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((p / 100.0) * sorted.len() as f32).ceil().max(1.0) as usize;
    sorted[rank.min(sorted.len()) - 1]
}

impl Health {
    /// An interviewer reply started speaking.
    pub fn reply(&self) {
        self.replies.fetch_add(1, Ordering::Relaxed);
    }

    /// The speaker ran dry `n` times during a reply.
    pub fn dry_spells(&self, n: u32) {
        self.dry_spells.fetch_add(n, Ordering::Relaxed);
    }

    /// A piece hit the length guard and was cut.
    pub fn cap_hit(&self) {
        self.cap_hits.fetch_add(1, Ordering::Relaxed);
    }

    /// A piece came back too short and was generated again.
    pub fn retry(&self) {
        self.retries.fetch_add(1, Ordering::Relaxed);
    }

    /// One generated piece and its real-time factor (generation time / audio time).
    pub fn piece(&self, rtf: f32) {
        self.rtf.lock().unwrap().push(rtf);
    }

    pub fn summary(&self) -> Summary {
        let mut rtf = self.rtf.lock().unwrap().clone();
        rtf.sort_by(|a, b| a.total_cmp(b));
        Summary {
            replies: self.replies.load(Ordering::Relaxed),
            pieces: rtf.len() as u32,
            dry_spells: self.dry_spells.load(Ordering::Relaxed),
            cap_hits: self.cap_hits.load(Ordering::Relaxed),
            retries: self.retries.load(Ordering::Relaxed),
            rtf_p50: nearest_rank(&rtf, 50.0),
            rtf_p90: nearest_rank(&rtf, 90.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_counts_and_percentiles() {
        let h = Health::default();
        h.reply();
        for r in [0.2, 0.3, 0.4, 0.5, 1.2] {
            h.piece(r);
        }
        h.dry_spells(2);
        h.cap_hit();
        h.retry();
        let s = h.summary();
        assert_eq!((s.replies, s.pieces, s.dry_spells, s.cap_hits, s.retries), (1, 5, 2, 1, 1));
        assert_eq!((s.rtf_p50, s.rtf_p90), (0.4, 1.2));
        assert_eq!(Health::default().summary().rtf_p90, 0.0);
    }
}
