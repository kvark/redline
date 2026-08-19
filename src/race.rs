use crate::{config, planet};

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

    pub fn update(&mut self, position: glam::Vec3, dt: f32) {
        if self.finished {
            return;
        }
        if self.started {
            self.time += dt;
        }
        let target = self.checkpoints[self.next_checkpoint];
        let radius = position.length().max(1.0);
        let gate = (12.0 / radius).max(0.08);
        if angular_close(position, target, gate) {
            if !self.started {
                self.started = true;
                self.lap_start = self.time;
            }
            let wrapped = self.next_checkpoint == 0;
            self.next_checkpoint = (self.next_checkpoint + 1) % self.checkpoints.len();
            if wrapped {
                let lap_time = self.time - self.lap_start;
                self.last_lap_time = Some(lap_time);
                self.best_lap = Some(match self.best_lap {
                    Some(best) => best.min(lap_time),
                    None => lap_time,
                });
                self.lap_start = self.time;
                if self.lap >= self.laps_to_win {
                    self.finished = true;
                } else {
                    self.lap += 1;
                }
            }
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
    use super::Race;
    use crate::{config, planet};

    #[test]
    fn completing_circuit_counts_a_lap() {
        let samples: Vec<planet::TrackSample> = (0..32)
            .map(|i| {
                let a = i as f32 / 32.0 * std::f32::consts::TAU;
                let n = glam::Vec3::new(a.sin(), 0.0, a.cos());
                planet::TrackSample {
                    position: n * 80.0,
                    tangent: glam::Vec3::Y,
                    normal: n,
                }
            })
            .collect();
        let mut race = Race::new(&samples, config::Race::default());
        assert_eq!(race.lap, 1);
        for sample in samples.iter().cycle().take(40) {
            race.update(sample.position, 0.05);
        }
        assert!(race.lap >= 2 || race.finished);
    }
}
