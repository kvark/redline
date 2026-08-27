use std::{fs, io::Write, path::PathBuf};

use crate::control;
use crate::planet;
use crate::vehicle::Isometry;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    Accel,
    Steer,
    Offroad,
    Lap,
}

impl Script {
    pub fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "accel" => Self::Accel,
            "steer" => Self::Steer,
            "offroad" => Self::Offroad,
            "lap" => Self::Lap,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accel => "accel",
            Self::Steer => "steer",
            Self::Offroad => "offroad",
            Self::Lap => "lap",
        }
    }

    /// Analog throttle/steer in [-1, 1] for a canned trajectory.
    pub fn analog(&self, t: f32, heading_error: f32, off_track: f32) -> (f32, f32) {
        match *self {
            Self::Accel => (1.0, 0.0),
            Self::Steer => (1.0, if t >= 1.2 { 0.85 } else { 0.0 }),
            Self::Offroad => {
                if t < 1.1 {
                    (1.0, 0.0)
                } else if t < 3.6 {
                    (1.0, 0.9)
                } else {
                    let gain = if off_track > 0.4 { 1.0 } else { 1.6 };
                    (1.0, (heading_error * gain).clamp(-1.0, 1.0))
                }
            }
            Self::Lap => (1.0, (heading_error * 1.7).clamp(-1.0, 1.0)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub t: f32,
    pub throttle: f32,
    pub steer: f32,
    pub position: glam::Vec3,
    pub speed: f32,
    pub forward_speed: f32,
    pub lateral_speed: f32,
    pub yaw_rate: f32,
    pub upright: f32,
    pub off_track: f32,
    pub heading_error: f32,
    pub recovered: u8,
}

pub struct Recorder {
    path: PathBuf,
    script: Script,
    rows: Vec<Sample>,
}

impl Recorder {
    pub fn new(path: PathBuf, script: Script) -> Self {
        Self {
            path,
            script,
            rows: Vec::new(),
        }
    }

    pub fn push(&mut self, sample: Sample) {
        self.rows.push(sample);
    }

    pub fn finish(&self) {
        let mut body = String::from(
            "t,throttle,steer,px,py,pz,speed,fwd,lat,yaw,upright,off,head,recovered\n",
        );
        for row in self.rows.iter() {
            body.push_str(&format!(
                "{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{}\n",
                row.t,
                row.throttle,
                row.steer,
                row.position.x,
                row.position.y,
                row.position.z,
                row.speed,
                row.forward_speed,
                row.lateral_speed,
                row.yaw_rate,
                row.upright,
                row.off_track,
                row.heading_error,
                row.recovered,
            ));
        }
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        match fs::File::create(&self.path).and_then(|mut file| file.write_all(body.as_bytes())) {
            Ok(()) => log::info!(
                "Wrote {} samples to {}",
                self.rows.len(),
                self.path.display()
            ),
            Err(err) => log::error!("Failed to write {}: {err}", self.path.display()),
        }
        log_summary(self.script, &self.rows);
    }
}

fn log_summary(script: Script, rows: &[Sample]) {
    if rows.is_empty() {
        log::warn!("trace {}: no samples", script.as_str());
        return;
    }
    let n = rows.len() as f32;
    let max_speed = rows.iter().map(|r| r.speed).fold(0.0f32, f32::max);
    let mean_speed = rows.iter().map(|r| r.speed).sum::<f32>() / n;
    let mean_lat = rows.iter().map(|r| r.lateral_speed.abs()).sum::<f32>() / n;
    let max_off = rows
        .iter()
        .map(|r| r.off_track)
        .fold(f32::NEG_INFINITY, f32::max);
    let min_upright = rows.iter().map(|r| r.upright).fold(f32::INFINITY, f32::min);
    let recoveries = rows.iter().map(|r| r.recovered as u32).sum::<u32>();
    let stuck = rows.iter().filter(|r| r.speed < 1.0 && r.t > 1.5).count();
    let yaw_sign_flips = rows
        .windows(2)
        .filter(|pair| pair[0].yaw_rate * pair[1].yaw_rate < -0.15)
        .count();
    let start = rows.first().unwrap().position;
    let end = rows.last().unwrap().position;
    let travelled = start.distance(end);
    log::info!(
        "trace {} n={} t={:.1}s travelled={:.1} max_speed={:.1} mean_speed={:.1} mean_|lat|={:.2} max_off={:.1} min_upright={:.2} recoveries={} stuck_samples={} yaw_flips={}",
        script.as_str(),
        rows.len(),
        rows.last().unwrap().t,
        travelled,
        max_speed,
        mean_speed,
        mean_lat,
        max_off,
        min_upright,
        recoveries,
        stuck,
        yaw_sign_flips,
    );
}

pub fn look_ahead_heading(
    pose: &Isometry,
    track: &[planet::TrackSample],
    speed: f32,
    pull_to_road: bool,
) -> f32 {
    let query = planet::query_track(pose.position, track);
    let up = pose.position.normalize_or_zero();
    let forward = (pose.orientation * glam::Vec3::Z).reject_from(up);
    let forward = forward.normalize_or_zero();
    let look = (8 + (speed * 0.28) as usize).min(20);
    let target = track[(query.index + look) % track.len()];
    let mut desired = (target.position - pose.position).reject_from(up);
    if pull_to_road && query.lateral.abs() > 1.0 {
        desired -= query.side * query.lateral;
    }
    let desired = desired.normalize_or_zero();
    control::signed_heading_error(forward, desired, up)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offroad_script_turns_then_returns() {
        let (throttle, steer) = Script::Offroad.analog(2.0, 0.0, 5.0);
        assert!(throttle > 0.5);
        assert!(steer > 0.5);
        let (_, return_steer) = Script::Offroad.analog(5.0, -0.8, 6.0);
        assert!(return_steer < 0.0);
    }
}
