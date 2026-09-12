//! Brief championship banners when crossing the line: lap time, FINAL LAP, FINISH,
//! plus a light BEST SECTOR flash on a personal-best gate split.

/// How long a completed-lap time flash stays up.
pub const LAP_HOLD_SECS: f32 = 1.35;
/// How long the FINAL LAP callout stays up.
pub const FINAL_HOLD_SECS: f32 = 1.75;
/// How long FINISH stays up after the checker.
pub const FINISH_HOLD_SECS: f32 = 2.2;
/// How long a best-sector gate flash stays up (lighter than lap banners).
pub const SECTOR_HOLD_SECS: f32 = 0.95;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CalloutKind {
    /// Mid-race line cross — flash the lap just completed.
    LapComplete { lap_time: f32 },
    /// Entering the last lap of the race (flash prior lap time underneath).
    FinalLap { previous_lap: f32 },
    /// Race over.
    Finish,
    /// Personal-best sector split at a mid-lap gate.
    SectorBest { sector_time: f32 },
}

#[derive(Clone, Debug)]
pub struct LapCallout {
    kind: CalloutKind,
    elapsed: f32,
}

impl LapCallout {
    pub fn lap_complete(lap_time: f32) -> Self {
        Self {
            kind: CalloutKind::LapComplete { lap_time },
            elapsed: 0.0,
        }
    }

    pub fn final_lap(previous_lap: f32) -> Self {
        Self {
            kind: CalloutKind::FinalLap { previous_lap },
            elapsed: 0.0,
        }
    }

    pub fn finish() -> Self {
        Self {
            kind: CalloutKind::Finish,
            elapsed: 0.0,
        }
    }

    pub fn sector_best(sector_time: f32) -> Self {
        Self {
            kind: CalloutKind::SectorBest { sector_time },
            elapsed: 0.0,
        }
    }

    /// Advance by a physics step. Returns `true` when the banner should clear.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.elapsed = (self.elapsed + dt.max(0.0)).min(self.hold());
        self.is_finished()
    }

    pub fn is_finished(&self) -> bool {
        self.elapsed >= self.hold()
    }

    pub fn kind(&self) -> CalloutKind {
        self.kind
    }

    fn hold(&self) -> f32 {
        match self.kind {
            CalloutKind::LapComplete { .. } => LAP_HOLD_SECS,
            CalloutKind::FinalLap { .. } => FINAL_HOLD_SECS,
            CalloutKind::Finish => FINISH_HOLD_SECS,
            CalloutKind::SectorBest { .. } => SECTOR_HOLD_SECS,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CalloutKind, FINAL_HOLD_SECS, LapCallout, SECTOR_HOLD_SECS};

    #[test]
    fn final_lap_clears_after_hold() {
        let mut c = LapCallout::final_lap(61.2);
        assert_eq!(c.kind(), CalloutKind::FinalLap { previous_lap: 61.2 });
        assert!(!c.tick(FINAL_HOLD_SECS * 0.5));
        assert!(c.tick(FINAL_HOLD_SECS));
        assert!(c.is_finished());
    }

    #[test]
    fn lap_complete_keeps_time() {
        let c = LapCallout::lap_complete(42.5);
        assert_eq!(c.kind(), CalloutKind::LapComplete { lap_time: 42.5 });
    }

    #[test]
    fn sector_best_is_brief() {
        let mut c = LapCallout::sector_best(11.2);
        assert_eq!(c.kind(), CalloutKind::SectorBest { sector_time: 11.2 });
        assert!(!c.tick(SECTOR_HOLD_SECS * 0.5));
        assert!(c.tick(SECTOR_HOLD_SECS));
        assert!(c.is_finished());
    }
}
