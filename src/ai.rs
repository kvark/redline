use crate::{boost_pad, config, control, planet, race, race_stats, racecraft, vehicle};

/// Snapshot of one racer for this physics step (0 = player, then AI in order).
#[derive(Clone, Copy, Debug)]
pub struct Racer {
    pub sample: usize,
    pub position: glam::Vec3,
    pub lateral: f32,
    pub speed: f32,
}

impl Racer {
    pub fn observe(
        vehicle: &vehicle::Vehicle,
        engine: &blade_engine::Engine,
        track: &[planet::TrackSample],
    ) -> Self {
        let position = vehicle.pose(engine).position;
        let query = planet::query_track(position, track);
        let (linear, _) = engine.get_velocity(vehicle.body_handle);
        Self {
            sample: query.index,
            position,
            lateral: query.lateral,
            speed: glam::Vec3::from(linear).length(),
        }
    }
}

/// Track context shared by every AI driver for a step.
pub struct Course<'a> {
    pub track: &'a [planet::TrackSample],
    pub track_width: f32,
    /// Mean metres per track sample.
    pub spacing: f32,
    pub pads: &'a [boost_pad::Pad],
    pub gravity: f32,
    /// Mean race distance (m) of the AI field, when there is a pack to keep.
    pub pack_mean: Option<f32>,
}

/// Each driver is a touch stronger in some sectors than others, so the pack
/// reshuffles over a lap instead of settling into a fixed train.
const PACE_SWING: f32 = 0.06;
/// Metres of upcoming ribbon used to pick the racing line.
const LINE_LOOKAHEAD: f32 = 26.0;
/// Lane error (m) tolerated before the driver backs off the throttle.
const LANE_DEADBAND: f32 = 1.5;
/// Steering correction per metre of error from the planned lane.
const LANE_GAIN: f32 = 0.05;
/// Keep the craft centre this far inside the ribbon edge (m).
const EDGE_MARGIN: f32 = 2.5;

pub struct Driver {
    pub vehicle: vehicle::Vehicle,
    /// Checkpoint / lap progress for live place + finish order.
    pub race: race::Race,
    steering: f32,
    target_speed: f32,
    lateral_offset: f32,
    stuck_time: f32,
    /// Racer id (0 is the player).
    id: usize,
    pace_phase: f32,
    planner: racecraft::Planner,
    odometer: race_stats::Odometer,
    last_sample: Option<usize>,
    /// Pads driven past (hit or missed), for the pad hit rate.
    pads_passed: u32,
}

impl Driver {
    pub fn spawn(
        engine: &mut blade_engine::Engine,
        config: &config::Vehicle,
        track: &[planet::TrackSample],
        track_index: usize,
        lateral_offset: f32,
        target_speed: f32,
        kit: vehicle::Kit,
    ) -> Self {
        let pose = vehicle::Vehicle::spawn_pose_at(
            track,
            track_index,
            vehicle::SPAWN_HOVER,
            lateral_offset,
        );
        let vehicle = vehicle::spawn(engine, config, pose, Some(kit));
        Self {
            vehicle,
            // Same default race config as the player (shared checkpoint grid).
            race: race::Race::new(track, config::Race::default()),
            steering: 0.0,
            target_speed,
            lateral_offset,
            stuck_time: 0.0,
            id: 1,
            pace_phase: 2.1,
            planner: racecraft::Planner::new(lateral_offset),
            odometer: race_stats::Odometer::default(),
            last_sample: None,
            pads_passed: 0,
        }
    }

    /// Racer id in the field snapshot (the player is 0, AI start at 1).
    pub fn with_id(mut self, id: usize) -> Self {
        self.id = id;
        self.pace_phase = id as f32 * 2.1;
        self
    }

    pub fn pads_passed(&self) -> u32 {
        self.pads_passed
    }

    /// Race distance (m) as of the last update.
    pub fn distance(&self) -> f32 {
        self.odometer.distance
    }

    pub fn tally(&self) -> racecraft::Tally {
        self.planner.tally
    }

    /// Sector pace rhythm: a deterministic per-driver strength profile.
    fn sector_pace(&self) -> f32 {
        let sector = self.race.sector() as f32;
        1.0 + PACE_SWING * (self.pace_phase + sector * 1.3).sin()
    }

    fn situation_rivals(
        &self,
        me: &Racer,
        field: &[Racer],
        course: &Course,
    ) -> Vec<racecraft::Rival> {
        let len = course.track.len();
        let tangent = course.track[me.sample].tangent;
        field
            .iter()
            .enumerate()
            .filter(|&(id, _)| id != self.id)
            .filter_map(|(id, other)| {
                let samples = race_stats::wrap_delta(other.sample, me.sample, len);
                let coarse = samples as f32 * course.spacing;
                if coarse.abs() > racecraft::YIELD_RANGE + 4.0 {
                    return None;
                }
                // Close up, project on the ribbon tangent for sub-sample precision.
                let ahead = (other.position - me.position).dot(tangent);
                Some(racecraft::Rival {
                    id,
                    ahead,
                    lateral: other.lateral,
                    closing: me.speed - other.speed,
                })
            })
            .collect()
    }

    /// Drive one step. `field` holds every racer's snapshot, indexed by id.
    /// Returns true when the driver had to be reset onto the ribbon.
    pub fn update(
        &mut self,
        engine: &mut blade_engine::Engine,
        course: &Course,
        field: &[Racer],
        dt: f32,
    ) -> bool {
        let track = course.track;
        let track_width = course.track_width;
        let pads = course.pads;
        let gravity = course.gravity;
        let pose = self.vehicle.pose(engine);
        let (nearest, _) = planet::track_progress(pose.position, track);
        let (linear, _) = engine.get_velocity(self.vehicle.body_handle);
        let speed = glam::Vec3::from(linear).length();

        if speed < 1.0 {
            self.stuck_time += dt.min(0.1);
        } else {
            self.stuck_time = 0.0;
        }
        if self.stuck_time > 2.5 {
            // A lightweight recovery keeps simple drivers from spending the race
            // wedged against scenery. Advance slightly so the same obstacle is not
            // selected immediately after respawning.
            let respawn = vehicle::Vehicle::spawn_pose_at(
                track,
                nearest + 3,
                vehicle::SPAWN_HOVER,
                self.lateral_offset,
            );
            self.vehicle.teleport(engine, &respawn);
            self.planner.reset_to(self.lateral_offset);
            self.steering = 0.0;
            self.stuck_time = 0.0;
            return true;
        }
        let me = field[self.id];
        let distance = self.odometer.update(nearest, track.len(), course.spacing);
        if self
            .last_sample
            .is_some_and(|last| race_stats::wrap_delta(nearest, last, track.len()) > 0)
            && pads.iter().any(|pad| pad.sample == nearest)
        {
            self.pads_passed += 1;
        }
        self.last_sample = Some(nearest);
        let pack = course
            .pack_mean
            .map_or(1.0, |mean| racecraft::pack_pace(mean - distance));
        let rivals = self.situation_rivals(&me, field, course);
        let line_samples = ((LINE_LOOKAHEAD / course.spacing.max(0.1)) as usize).max(2);
        let half_room = (track_width * 0.5 - EDGE_MARGIN).max(0.5);
        let situation = racecraft::Situation {
            lateral: me.lateral,
            half_room,
            turn: signed_turn(track, nearest + 2, line_samples),
            pad: next_pad(pads, nearest, track.len(), course.spacing),
            rivals: &rivals,
        };
        let before = std::mem::discriminant(&self.planner.mode);
        let plan = self.planner.plan(&situation, dt);
        if std::mem::discriminant(&self.planner.mode) != before {
            log::debug!(
                "AI{} lap={} s={} lat={:.1} -> {:?} rivals={:?}",
                self.id,
                self.race.lap,
                nearest,
                me.lateral,
                self.planner.mode,
                rivals
            );
        }
        let lateral_offset = plan.lateral;
        let pace = plan.pace * self.sector_pace() * pack;
        let look_ahead = (8 + (speed * 0.34) as usize).min(20);
        let target = track[(nearest + look_ahead) % track.len()];

        let up = pose.position.normalize_or_zero();
        let forward = reject_from(pose.orientation * glam::Vec3::Z, up).normalize_or_zero();
        let target_side = target.normal.cross(target.tangent).normalize_or_zero();
        let lane_target = target.position + target_side * lateral_offset;
        let desired = reject_from(lane_target - pose.position, up).normalize_or_zero();
        let heading_error = control::signed_heading_error(forward, desired, up);
        let query = planet::query_track(pose.position, track);
        let off_track = planet::off_track_distance(&query, track_width);
        let lane_error = query.lateral - lateral_offset;
        let steering_target = (heading_error * 1.75 - lane_error * LANE_GAIN).clamp(-1.0, 1.0);
        let response = if self.vehicle.is_recoiling() {
            1.0 - (-dt.min(0.1) * 2.2).exp()
        } else {
            1.0 - (-dt.min(0.1) * 7.0).exp()
        };
        self.steering += (steering_target - self.steering) * response;

        let steering_limit = 0.48 * (1.0 / (1.0 + speed / 42.0)).clamp(0.45, 1.0);
        let turn_slowdown = (1.0 - heading_error.abs() * 0.42).clamp(0.58, 1.0);
        // Racecraft moves the lane on purpose; only slow for a real miss.
        let lane_miss = (lane_error.abs() - LANE_DEADBAND).max(0.0);
        let lane_slowdown = (1.0 - lane_miss * 0.035).clamp(0.72, 1.0);
        let cruise = self.target_speed * pace;
        let cruise_speed = cruise * turn_slowdown * lane_slowdown;
        let target_speed = if self.vehicle.is_recoiling() {
            self.target_speed * 0.35
        } else if speed > cruise * (1.12 + self.vehicle.speed_bonus()) {
            // Coast above cruise, but let a live boost carry its extra pace.
            0.0
        } else {
            cruise_speed
        };
        self.vehicle.drive(
            engine,
            target_speed,
            self.steering * steering_limit,
            off_track,
            pads,
            dt,
        );
        self.vehicle.apply_gravity(engine, gravity, dt);
        self.vehicle.apply_stability(engine, dt);
        false
    }
}

/// Signed heading change (rad) from `from` over the next `count` samples;
/// positive bends toward +lateral (`normal × tangent`).
pub fn signed_turn(track: &[planet::TrackSample], from: usize, count: usize) -> f32 {
    let len = track.len();
    if len < 3 {
        return 0.0;
    }
    let dir = |i: usize| {
        let a = track[i % len];
        let b = track[(i + 1) % len];
        reject_from(b.position - a.position, a.normal).normalize_or_zero()
    };
    let start = dir(from);
    let end = dir(from + count);
    let normal = track[(from + count / 2) % len].normal;
    let sin = start.cross(end).dot(normal);
    let cos = start.dot(end);
    sin.atan2(cos)
}

/// Nearest pad ahead within reach, as seen from track sample `nearest`.
fn next_pad(
    pads: &[boost_pad::Pad],
    nearest: usize,
    len: usize,
    spacing: f32,
) -> Option<racecraft::PadAhead> {
    pads.iter()
        .filter_map(|pad| {
            let samples = race_stats::wrap_delta(pad.sample, nearest, len);
            let distance = samples as f32 * spacing;
            (-boost_pad::PAD_HALF_LENGTH..=racecraft::PAD_LOOKAHEAD)
                .contains(&distance)
                .then_some(racecraft::PadAhead {
                    distance: distance.max(0.0),
                    lateral: 0.0,
                    half_width: pad.half_width,
                })
        })
        .min_by(|a, b| a.distance.total_cmp(&b.distance))
}

fn reject_from(vector: glam::Vec3, normal: glam::Vec3) -> glam::Vec3 {
    vector - normal * vector.dot(normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(count: usize, radius: f32) -> Vec<planet::TrackSample> {
        (0..count)
            .map(|i| {
                let a = i as f32 / count as f32 * std::f32::consts::TAU;
                // Ring in the XZ plane on a sphere: "up" is radial, the path
                // bends around +Y, which is the +lateral side when moving +a.
                let n = glam::Vec3::new(a.sin(), 0.3, a.cos()).normalize();
                planet::TrackSample {
                    position: n * radius,
                    tangent: glam::Vec3::new(a.cos(), 0.0, -a.sin()),
                    normal: n,
                }
            })
            .collect()
    }

    #[test]
    fn turn_sign_points_at_the_inside() {
        // A small circle above the equator curves toward the pole (+Y).
        let track = ring(128, 50.0);
        let turn = signed_turn(&track, 0, 10);
        let side = track[0].normal.cross(track[0].tangent);
        // + turn ⇒ inside is +side; +side must point up toward the circle's centre.
        assert!(turn.abs() > 1e-3);
        assert_eq!(turn > 0.0, side.y > 0.0);
    }

    #[test]
    fn pad_targeting_sees_pads_ahead_only() {
        let track = ring(128, 50.0);
        let pads = boost_pad::place(&track, 8, 1, 14.0);
        let pad = pads[0];
        let spacing = 3.0;
        let seen = next_pad(&pads, pad.sample - 8, track.len(), spacing).unwrap();
        assert_eq!(seen.distance, 24.0);
        assert_eq!(seen.half_width, pad.half_width);
        // Just past a pad it drops out; only the next pad can be seen.
        let from = pad.sample + 2;
        let next = (pads[1].sample - from) as f32 * spacing;
        let seen = next_pad(&pads, from, track.len(), spacing).map(|p| p.distance);
        if next <= racecraft::PAD_LOOKAHEAD {
            assert_eq!(seen, Some(next));
        } else {
            assert_eq!(seen, None);
        }
    }

    #[test]
    fn heading_error_turns_toward_the_target() {
        let up = glam::Vec3::Y;
        let forward = glam::Vec3::Z;
        assert!(control::signed_heading_error(forward, glam::Vec3::X, up) > 0.0);
        assert!(control::signed_heading_error(forward, -glam::Vec3::X, up) < 0.0);
        assert_eq!(control::signed_heading_error(forward, forward, up), 0.0);
    }
}
