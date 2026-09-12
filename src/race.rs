use crate::{config, planet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RaceEvent {
    /// Completed a mid-lap sector gate. `is_best` when this split is a new personal best.
    SectorComplete { sector_time: f32, is_best: bool },
    /// Completed a lap that was not the race finish. `entering_final` when the
    /// new current lap is the last one.
    LapComplete { lap_time: f32, entering_final: bool },
    /// Crossed the line on the final lap.
    Finished { lap_time: f32 },
}

pub struct Race {
    pub laps_to_win: u32,
    pub lap: u32,
    pub next_checkpoint: usize,
    pub checkpoints: Vec<glam::Vec3>,
    pub started: bool,
    pub finished: bool,
    pub time: f32,
    pub last_lap_time: Option<f32>,
    pub best_lap: Option<f32>,
    pub last_sector_time: Option<f32>,
    pub best_sector: Option<f32>,
    lap_start: f32,
    sector_start: f32,
}

impl Race {
    pub fn new(track: &[planet::TrackSample], config: config::Race) -> Self {
        let count = config.checkpoint_count.max(4) as usize;
        let stride = (track.len() / count).max(1);
        let checkpoints = (0..count)
            .map(|i| track[(i * stride) % track.len()].position)
            .collect();
        Self {
            laps_to_win: config.laps,
            lap: 1,
            next_checkpoint: 1,
            checkpoints,
            started: false,
            finished: false,
            time: 0.0,
            last_lap_time: None,
            best_lap: None,
            last_sector_time: None,
            best_sector: None,
            lap_start: 0.0,
            sector_start: 0.0,
        }
    }

    /// Elapsed time on the lap currently in progress (0 until the race starts).
    pub fn current_lap_time(&self) -> f32 {
        if !self.started || self.finished {
            return self.last_lap_time.unwrap_or(0.0);
        }
        (self.time - self.lap_start).max(0.0)
    }

    /// Elapsed time on the sector currently in progress (0 until the race starts).
    pub fn current_sector_time(&self) -> f32 {
        if !self.started || self.finished {
            return self.last_sector_time.unwrap_or(0.0);
        }
        (self.time - self.sector_start).max(0.0)
    }

    /// 1-based sector index for the gate the player is heading toward.
    pub fn sector(&self) -> usize {
        if self.next_checkpoint == 0 {
            self.checkpoints.len()
        } else {
            self.next_checkpoint
        }
    }

    pub fn sector_count(&self) -> usize {
        self.checkpoints.len()
    }

    pub fn update(&mut self, position: glam::Vec3, dt: f32) -> Option<RaceEvent> {
        if self.finished {
            return None;
        }
        if self.started {
            self.time += dt;
        }
        let target = self.checkpoints[self.next_checkpoint];
        let radius = position.length().max(1.0);
        let gate = (12.0 / radius).max(0.08);
        if !angular_close(position, target, gate) {
            return None;
        }

        // First gate arms the clock — not a timed sector.
        if !self.started {
            self.started = true;
            self.lap_start = self.time;
            self.sector_start = self.time;
            self.next_checkpoint = (self.next_checkpoint + 1) % self.checkpoints.len();
            return None;
        }

        let sector_time = (self.time - self.sector_start).max(0.0);
        let is_best = match self.best_sector {
            Some(best) => sector_time < best,
            None => true,
        };
        self.last_sector_time = Some(sector_time);
        if is_best {
            self.best_sector = Some(sector_time);
        }
        self.sector_start = self.time;

        let wrapped = self.next_checkpoint == 0;
        self.next_checkpoint = (self.next_checkpoint + 1) % self.checkpoints.len();
        if !wrapped {
            return Some(RaceEvent::SectorComplete {
                sector_time,
                is_best,
            });
        }

        let lap_time = self.time - self.lap_start;
        self.last_lap_time = Some(lap_time);
        self.best_lap = Some(match self.best_lap {
            Some(best) => best.min(lap_time),
            None => lap_time,
        });
        self.lap_start = self.time;
        if self.lap >= self.laps_to_win {
            self.finished = true;
            Some(RaceEvent::Finished { lap_time })
        } else {
            self.lap += 1;
            let entering_final = self.lap >= self.laps_to_win;
            Some(RaceEvent::LapComplete {
                lap_time,
                entering_final,
            })
        }
    }

    pub fn reset(&mut self) {
        self.lap = 1;
        self.next_checkpoint = 1;
        self.started = false;
        self.finished = false;
        self.time = 0.0;
        self.last_lap_time = None;
        self.best_lap = None;
        self.last_sector_time = None;
        self.best_sector = None;
        self.lap_start = 0.0;
        self.sector_start = 0.0;
    }

    /// Higher means further ahead. Finished racers beat anyone still racing;
    /// among finishers, earlier race time ranks higher.
    pub fn progress_score(&self, track_frac: f32) -> f64 {
        if self.finished {
            // Large base so finishers sort above the field; subtract time for order.
            return 1_000_000.0 + (100_000.0 - self.time as f64);
        }
        let frac = track_frac.clamp(0.0, 0.999_999) as f64;
        (self.lap.saturating_sub(1) as f64) + frac
    }
}

/// 1-based place for the player among `opponent_scores` (higher score = ahead).
pub fn player_place(player_score: f64, opponent_scores: impl IntoIterator<Item = f64>) -> usize {
    1 + opponent_scores
        .into_iter()
        .filter(|&score| score > player_score)
        .count()
}

fn angular_close(a: glam::Vec3, b: glam::Vec3, max_angle: f32) -> bool {
    let da = a.normalize_or_zero();
    let db = b.normalize_or_zero();
    da.dot(db) > max_angle.cos()
}

#[cfg(test)]
mod tests {
    use super::{Race, RaceEvent};
    use crate::{config, planet};

    fn sample_track() -> Vec<planet::TrackSample> {
        (0..32)
            .map(|i| {
                let a = i as f32 / 32.0 * std::f32::consts::TAU;
                let n = glam::Vec3::new(a.sin(), 0.0, a.cos());
                planet::TrackSample {
                    position: n * 80.0,
                    tangent: glam::Vec3::Y,
                    normal: n,
                }
            })
            .collect()
    }

    #[test]
    fn completing_circuit_counts_a_lap() {
        let samples = sample_track();
        let mut race = Race::new(&samples, config::Race::default());
        assert_eq!(race.lap, 1);
        for sample in samples.iter().cycle().take(40) {
            race.update(sample.position, 0.05);
        }
        assert!(race.lap >= 2 || race.finished);
    }

    #[test]
    fn final_lap_and_finish_events() {
        let samples = sample_track();
        let cfg = config::Race {
            laps: 2,
            checkpoint_count: 8,
        };
        let mut race = Race::new(&samples, cfg);
        let mut saw_final = false;
        let mut saw_finish = false;
        for sample in samples.iter().cycle().take(80) {
            match race.update(sample.position, 0.05) {
                Some(RaceEvent::LapComplete {
                    entering_final: true,
                    ..
                }) => saw_final = true,
                Some(RaceEvent::Finished { .. }) => saw_finish = true,
                _ => {}
            }
        }
        assert!(saw_final, "should enter final lap");
        assert!(saw_finish, "should finish the race");
        assert!(race.finished);
    }

    #[test]
    fn current_lap_time_ticks_after_start() {
        let samples = sample_track();
        let cfg = config::Race {
            checkpoint_count: 8,
            ..Default::default()
        };
        let mut race = Race::new(&samples, cfg);
        assert_eq!(race.current_lap_time(), 0.0);
        // Hit the first gated checkpoint to start the clock.
        let gate = race.checkpoints[race.next_checkpoint];
        race.update(gate, 0.0);
        assert!(race.started);
        // Stand far from the next gate and accumulate time.
        let away = -gate;
        for _ in 0..25 {
            assert!(race.update(away, 0.05).is_none());
        }
        let t = race.current_lap_time();
        assert!((1.2..=1.3).contains(&t), "got {t}");
    }

    #[test]
    fn sector_index_tracks_next_gate() {
        let samples = sample_track();
        let cfg = config::Race {
            checkpoint_count: 8,
            ..Default::default()
        };
        let mut race = Race::new(&samples, cfg);
        assert_eq!(race.sector(), 1);
        assert_eq!(race.sector_count(), 8);
        let gate = race.checkpoints[1];
        race.update(gate, 0.0);
        assert_eq!(race.sector(), 2);
        race.next_checkpoint = 0;
        assert_eq!(race.sector(), 8);
    }

    #[test]
    fn mid_gate_emits_sector_and_tracks_best() {
        let samples = sample_track();
        let cfg = config::Race {
            checkpoint_count: 8,
            ..Default::default()
        };
        let mut race = Race::new(&samples, cfg);
        // Start at CP1.
        let start = race.checkpoints[1];
        assert!(race.update(start, 0.0).is_none());
        assert!(race.started);
        assert_eq!(race.best_sector, None);

        // Drive toward CP2 with time on the clock.
        let next = race.checkpoints[2];
        let away = -next;
        for _ in 0..20 {
            assert!(race.update(away, 0.05).is_none());
        }
        let sector_t = race.current_sector_time();
        assert!((0.95..=1.05).contains(&sector_t), "got {sector_t}");

        match race.update(next, 0.0) {
            Some(RaceEvent::SectorComplete {
                sector_time,
                is_best,
            }) => {
                assert!(is_best);
                assert!((0.95..=1.05).contains(&sector_time), "got {sector_time}");
                assert_eq!(race.best_sector, Some(sector_time));
                assert_eq!(race.last_sector_time, Some(sector_time));
            }
            other => panic!("expected SectorComplete, got {other:?}"),
        }

        // Slower next sector is not a best.
        let next2 = race.checkpoints[3];
        for _ in 0..40 {
            assert!(race.update(-next2, 0.05).is_none());
        }
        match race.update(next2, 0.0) {
            Some(RaceEvent::SectorComplete { is_best, .. }) => assert!(!is_best),
            other => panic!("expected SectorComplete, got {other:?}"),
        }
        let best = race.best_sector.expect("best sector set");
        let last = race.last_sector_time.expect("last sector set");
        assert!(best < last);
    }

    #[test]
    fn progress_score_orders_by_lap_then_frac() {
        let samples = sample_track();
        let mut behind = Race::new(&samples, config::Race::default());
        let mut ahead = Race::new(&samples, config::Race::default());
        ahead.lap = 2;
        assert!(ahead.progress_score(0.1) > behind.progress_score(0.9));
        behind.lap = 2;
        assert!(behind.progress_score(0.8) > ahead.progress_score(0.1));
    }

    #[test]
    fn finished_racer_beats_field_and_earlier_time_wins() {
        let samples = sample_track();
        let mut done_fast = Race::new(&samples, config::Race::default());
        let mut done_slow = Race::new(&samples, config::Race::default());
        let mut racing = Race::new(&samples, config::Race::default());
        done_fast.finished = true;
        done_fast.time = 90.0;
        done_slow.finished = true;
        done_slow.time = 110.0;
        racing.lap = 3;
        assert!(done_fast.progress_score(0.0) > racing.progress_score(0.99));
        assert!(done_fast.progress_score(0.0) > done_slow.progress_score(0.0));
        assert_eq!(
            super::player_place(
                done_slow.progress_score(0.0),
                [done_fast.progress_score(0.0), racing.progress_score(0.5)]
            ),
            2
        );
    }

    #[test]
    fn reset_clears_best_lap_and_sector() {
        let samples = sample_track();
        let mut race = Race::new(&samples, config::Race::default());
        race.best_lap = Some(40.0);
        race.best_sector = Some(8.0);
        race.last_sector_time = Some(9.0);
        race.reset();
        assert_eq!(race.best_lap, None);
        assert_eq!(race.best_sector, None);
        assert_eq!(race.last_sector_time, None);
    }
}
