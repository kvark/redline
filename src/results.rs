//! Brief post-FINISH championship results board (place + lap times).

/// How long the results board stays up before auto-returning to the menu.
pub const RESULTS_HOLD_SECS: f32 = 4.5;

#[derive(Clone, Debug)]
pub struct RaceResults {
    pub place: usize,
    pub field: usize,
    pub race_time: f32,
    pub best_lap: Option<f32>,
    pub last_lap: Option<f32>,
    elapsed: f32,
}

impl RaceResults {
    pub fn new(
        place: usize,
        field: usize,
        race_time: f32,
        best_lap: Option<f32>,
        last_lap: Option<f32>,
    ) -> Self {
        Self {
            place: place.max(1),
            field: field.max(1),
            race_time,
            best_lap,
            last_lap,
            elapsed: 0.0,
        }
    }

    /// Advance by a physics step. Returns `true` when the board should clear.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.elapsed = (self.elapsed + dt.max(0.0)).min(RESULTS_HOLD_SECS);
        self.is_finished()
    }

    pub fn is_finished(&self) -> bool {
        self.elapsed >= RESULTS_HOLD_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::{RESULTS_HOLD_SECS, RaceResults};

    #[test]
    fn results_clear_after_hold() {
        let mut board = RaceResults::new(2, 4, 125.0, Some(40.0), Some(41.0));
        assert_eq!(board.place, 2);
        assert_eq!(board.field, 4);
        assert!(!board.tick(RESULTS_HOLD_SECS * 0.5));
        assert!(board.tick(RESULTS_HOLD_SECS));
        assert!(board.is_finished());
    }
}
