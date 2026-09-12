//! Championship start lights: 3 → 2 → 1 → GO, then release drive.

/// Seconds each numeral (3 / 2 / 1) stays on screen.
pub const BEAT_SECS: f32 = 0.85;
/// How long "GO" stays visible after controls unlock.
pub const GO_HOLD_SECS: f32 = 0.55;

#[derive(Clone, Debug)]
pub struct StartCountdown {
    elapsed: f32,
}

impl StartCountdown {
    pub fn new() -> Self {
        Self { elapsed: 0.0 }
    }

    /// Advance by a physics (or wall) step. Returns `true` when the sequence is done.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.elapsed = (self.elapsed + dt.max(0.0)).min(total_duration());
        self.is_finished()
    }

    pub fn is_finished(&self) -> bool {
        self.elapsed >= total_duration()
    }

    /// Player and AI drive stay frozen through 3-2-1; unlock when GO appears.
    pub fn controls_locked(&self) -> bool {
        self.elapsed < BEAT_SECS * 3.0
    }

    /// Big HUD label, if still showing.
    pub fn label(&self) -> Option<&'static str> {
        if self.is_finished() {
            return None;
        }
        let t = self.elapsed;
        if t < BEAT_SECS {
            Some("3")
        } else if t < BEAT_SECS * 2.0 {
            Some("2")
        } else if t < BEAT_SECS * 3.0 {
            Some("1")
        } else {
            Some("GO")
        }
    }
}

fn total_duration() -> f32 {
    BEAT_SECS * 3.0 + GO_HOLD_SECS
}

#[cfg(test)]
mod tests {
    use super::{BEAT_SECS, GO_HOLD_SECS, StartCountdown};

    #[test]
    fn phases_are_3_2_1_go_then_done() {
        let mut cd = StartCountdown::new();
        assert_eq!(cd.label(), Some("3"));
        assert!(cd.controls_locked());

        assert!(!cd.tick(BEAT_SECS - 0.01));
        assert_eq!(cd.label(), Some("3"));

        assert!(!cd.tick(0.02));
        assert_eq!(cd.label(), Some("2"));
        assert!(cd.controls_locked());

        assert!(!cd.tick(BEAT_SECS));
        assert_eq!(cd.label(), Some("1"));
        assert!(cd.controls_locked());

        assert!(!cd.tick(BEAT_SECS));
        assert_eq!(cd.label(), Some("GO"));
        assert!(!cd.controls_locked());

        assert!(!cd.tick(GO_HOLD_SECS * 0.5));
        assert_eq!(cd.label(), Some("GO"));

        assert!(cd.tick(GO_HOLD_SECS));
        assert!(cd.is_finished());
        assert_eq!(cd.label(), None);
        assert!(!cd.controls_locked());
    }

    #[test]
    fn zero_dt_does_not_finish() {
        let mut cd = StartCountdown::new();
        assert!(!cd.tick(0.0));
        assert_eq!(cd.label(), Some("3"));
    }
}
