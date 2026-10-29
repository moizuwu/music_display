//! Browsing the lyrics by hand: while the user scrolls, the view stops following the playing
//! line, and takes it up again after a few idle seconds (or on request). No egui types, so the
//! timing is tested without a window.
use std::time::{Duration, Instant};

/// How long the view stays where the user put it.
pub const IDLE: Duration = Duration::from_secs(6);

#[derive(Default)]
pub struct Browse {
    /// The last time the user moved the view (`None` = following the playing line).
    at: Option<Instant>,
    /// Rows asked for by key presses, not scrolled yet.
    pending: f32,
}

impl Browse {
    /// The user scrolled (wheel, scrollbar, drag): stop following for a while.
    pub fn touch(&mut self) {
        self.at = Some(Instant::now());
    }

    /// Scroll by `rows` (negative = up) from a key.
    pub fn by(&mut self, rows: f32) {
        self.pending += rows;
        self.touch();
    }

    /// Back to the playing line at once.
    pub fn follow(&mut self) {
        self.at = None;
        self.pending = 0.0;
    }

    pub fn active(&self) -> bool {
        self.active_at(Instant::now())
    }

    pub fn active_at(&self, now: Instant) -> bool {
        self.at.is_some_and(|t| now.saturating_duration_since(t) < IDLE)
    }

    /// Time to hand the view back? Clears the state when the idle time is over.
    pub fn expire(&mut self) -> bool {
        if self.at.is_some() && !self.active() {
            self.follow();
            return true;
        }
        false
    }

    /// The rows to scroll this frame (key presses since the last call).
    pub fn take_rows(&mut self) -> f32 {
        std::mem::take(&mut self.pending)
    }

    /// Time left before following resumes (to schedule a repaint), if browsing.
    pub fn remaining(&self) -> Option<Duration> {
        self.at.map(|t| IDLE.saturating_sub(t.elapsed()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browsing_suspends_following_for_six_seconds() {
        let mut b = Browse::default();
        assert!(!b.active());
        b.touch();
        let t0 = b.at.unwrap();
        assert!(b.active_at(t0 + Duration::from_secs(5)));
        assert!(!b.active_at(t0 + Duration::from_secs(7)));
    }

    #[test]
    fn keys_queue_rows_and_follow_clears_them() {
        let mut b = Browse::default();
        b.by(3.0);
        b.by(-1.0);
        assert!(b.active());
        assert_eq!(b.take_rows(), 2.0);
        assert_eq!(b.take_rows(), 0.0);
        b.by(5.0);
        b.follow();
        assert!(!b.active() && b.take_rows() == 0.0);
    }

    #[test]
    fn expire_hands_the_view_back_only_after_the_idle_time() {
        let mut b = Browse::default();
        assert!(!b.expire());
        b.touch();
        assert!(!b.expire() && b.active());
        b.at = Some(Instant::now() - Duration::from_secs(10));
        assert!(b.expire() && !b.active());
        assert!(!b.expire());
    }
}
