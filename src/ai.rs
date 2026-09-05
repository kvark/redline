use crate::{config, control, planet, vehicle};

pub struct Driver {
    pub vehicle: vehicle::Vehicle,
    steering: f32,
    target_speed: f32,
    lateral_offset: f32,
    stuck_time: f32,
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
            steering: 0.0,
            target_speed,
            lateral_offset,
            stuck_time: 0.0,
        }
    }

    pub fn update(
        &mut self,
        engine: &mut blade_engine::Engine,
        track: &[planet::TrackSample],
        gravity: f32,
        dt: f32,
    ) {
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
            self.steering = 0.0;
            self.stuck_time = 0.0;
            return;
        }
        let look_ahead = (8 + (speed * 0.34) as usize).min(20);
        let target = track[(nearest + look_ahead) % track.len()];

        let up = pose.position.normalize_or_zero();
        let forward = reject_from(pose.orientation * glam::Vec3::Z, up).normalize_or_zero();
        let target_side = target.normal.cross(target.tangent).normalize_or_zero();
        let lane_target = target.position + target_side * self.lateral_offset;
        let desired = reject_from(lane_target - pose.position, up).normalize_or_zero();
        let heading_error = control::signed_heading_error(forward, desired, up);
        let query = planet::query_track(pose.position, track);
        let lane_error = query.lateral - self.lateral_offset;
        let steering_target = (heading_error * 1.75 - lane_error * 0.025).clamp(-1.0, 1.0);
        let response = if self.vehicle.is_recoiling() {
            1.0 - (-dt.min(0.1) * 2.2).exp()
        } else {
            1.0 - (-dt.min(0.1) * 7.0).exp()
        };
        self.steering += (steering_target - self.steering) * response;

        let steering_limit = 0.48 * (1.0 / (1.0 + speed / 42.0)).clamp(0.45, 1.0);
        let turn_slowdown = (1.0 - heading_error.abs() * 0.42).clamp(0.58, 1.0);
        let lane_slowdown = (1.0 - lane_error.abs() * 0.035).clamp(0.72, 1.0);
        let cruise_speed = self.target_speed * turn_slowdown * lane_slowdown;
        let target_speed = if self.vehicle.is_recoiling() {
            self.target_speed * 0.35
        } else if speed > self.target_speed * 1.12 {
            0.0
        } else {
            cruise_speed
        };
        self.vehicle
            .drive(engine, target_speed, self.steering * steering_limit, dt);
        self.vehicle.apply_gravity(engine, gravity, dt);
        self.vehicle.apply_stability(engine, dt);
    }
}

fn reject_from(vector: glam::Vec3, normal: glam::Vec3) -> glam::Vec3 {
    vector - normal * vector.dot(normal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_error_turns_toward_the_target() {
        let up = glam::Vec3::Y;
        let forward = glam::Vec3::Z;
        assert!(control::signed_heading_error(forward, glam::Vec3::X, up) > 0.0);
        assert!(control::signed_heading_error(forward, -glam::Vec3::X, up) < 0.0);
        assert_eq!(control::signed_heading_error(forward, forward, up), 0.0);
    }
}
