use crate::{config, planet};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RaceEvent {
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
    lap_start: f32,
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
            lap_start: 0.0,
        }
    }

    /// Elapsed time on the lap currently in progress (0 until the race starts).
    pub fn current_lap_time(&self) -> f32 {
        if !self.started || self.finished {
            return self.last_lap_time.unwrap_or(0.0);
        }
        (self.time - self.lap_start).max(0.0)
    }

    /// 1-based sector index matching the next gate the player must hit.
    pub fn sector(&self) -> usize {
        self.next_checkpoint + 1
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
        if !self.started {
            self.started = true;
            self.lap_start = self.time;
        }
        let wrapped = self.next_checkpoint == 0;
        self.next_checkpoint = (self.next_checkpoint + 1) % self.checkpoints.len();
        if !wrapped {
            return None;
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
        self.lap_start = 0.0;
    }
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
}
