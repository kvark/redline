//! Brief post-FINISH championship results board: place, lap times, launch
//! rating and the whole field with gaps to the winner.

use crate::{launch, race, split};

/// How long the results board stays up before auto-returning to the menu.
pub const RESULTS_HOLD_SECS: f32 = 6.0;

/// One row of the classification.
#[derive(Clone, Debug, PartialEq)]
pub struct Standing {
    /// Racer id: 0 is the player.
    pub racer: usize,
    /// Finish-clock gap, or the live split for a racer still on course.
    pub gap: Option<f32>,
}

impl Standing {
    pub fn name(&self) -> String {
        if self.racer == 0 {
            "You".to_string()
        } else {
            format!("AI {}", self.racer)
        }
    }

    /// "+1.4s" for chasers, "WINNER" for P1.
    pub fn gap_text(&self, place: usize) -> String {
        if place == 1 {
            "WINNER".to_string()
        } else {
            self.gap
                .map_or_else(|| "—".to_string(), |gap| format!("+{gap:.1}s"))
        }
    }
}

/// Classify every racer from the same physics tick, not from the split timer's
/// distance order. Stable order breaks exact ties by grid id.
pub fn classify(field: &[(&race::Race, f32)], timer: &split::GapTimer) -> Vec<Standing> {
    let mut ids: Vec<_> = (0..field.len()).collect();
    let score = |id: usize| field[id].0.progress_score(field[id].1);
    ids.sort_by(|&a, &b| score(b).total_cmp(&score(a)));
    let Some(&winner) = ids.first() else {
        return Vec::new();
    };
    ids.into_iter()
        .map(|racer| {
            let gap = match (field[winner].0.finish_time(), field[racer].0.finish_time()) {
                (Some(first), Some(time)) => Some((time - first).max(0.0)),
                _ => timer.gap(winner, racer),
            };
            Standing { racer, gap }
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct RaceResults {
    pub place: usize,
    pub field: usize,
    pub race_time: f32,
    pub best_lap: Option<f32>,
    pub last_lap: Option<f32>,
    pub launch: Option<launch::Rating>,
    /// Leader first; empty when nobody was timed.
    pub standings: Vec<Standing>,
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
            launch: None,
            standings: Vec::new(),
            elapsed: 0.0,
        }
    }

    /// Attach the classification; the player's row decides the place.
    pub fn with_field(mut self, launch: Option<launch::Rating>, standings: Vec<Standing>) -> Self {
        if let Some(index) = standings.iter().position(|s| s.racer == 0) {
            self.place = index + 1;
            self.field = standings.len();
        }
        self.launch = launch;
        self.standings = standings;
        self
    }

    /// Player gap to the winner, or margin over P2 when the player won.
    pub fn margin(&self) -> Option<split::Split> {
        let player = self.standings.get(self.place.checked_sub(1)?)?;
        if self.place == 1 {
            self.standings
                .get(1)
                .and_then(|p2| p2.gap.map(split::Split::Leading))
        } else {
            player.gap.map(split::Split::Behind)
        }
    }

    pub fn championship_order(
        &self,
        best_lap: impl Fn(usize) -> Option<f32>,
    ) -> Vec<(usize, Option<f32>)> {
        self.standings
            .iter()
            .map(|row| (row.racer, best_lap(row.racer)))
            .collect()
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
    use super::{RESULTS_HOLD_SECS, RaceResults, Standing};
    use crate::{launch::Rating, split::Split};

    #[test]
    fn results_clear_after_hold() {
        let mut board = RaceResults::new(2, 4, 125.0, Some(40.0), Some(41.0));
        assert_eq!(board.place, 2);
        assert_eq!(board.field, 4);
        assert!(!board.tick(RESULTS_HOLD_SECS * 0.5));
        assert!(board.tick(RESULTS_HOLD_SECS));
        assert!(board.is_finished());
    }

    #[test]
    fn field_sets_place_and_margins() {
        let rows = |ids: &[(usize, f32)]| {
            ids.iter()
                .map(|&(racer, gap)| Standing {
                    racer,
                    gap: Some(gap),
                })
                .collect::<Vec<_>>()
        };
        let board = RaceResults::new(1, 1, 60.0, None, None)
            .with_field(Some(Rating::Good), rows(&[(2, 0.0), (0, 1.4), (1, 3.0)]));
        assert_eq!((board.place, board.field), (2, 3));
        assert_eq!(board.margin(), Some(Split::Behind(1.4)));
        assert_eq!(board.standings[0].gap_text(1), "WINNER");
        assert_eq!(board.standings[1].gap_text(2), "+1.4s");
        assert_eq!(board.standings[1].name(), "You");
        assert_eq!(board.standings[2].name(), "AI 1");

        let won =
            RaceResults::new(3, 3, 60.0, None, None).with_field(None, rows(&[(0, 0.0), (1, 0.6)]));
        assert_eq!(won.place, 1);
        assert_eq!(won.margin(), Some(Split::Leading(0.6)));
        // No classification: keep the live place.
        let bare = RaceResults::new(2, 4, 60.0, None, None).with_field(None, Vec::new());
        assert_eq!((bare.place, bare.margin()), (2, None));
    }
}
