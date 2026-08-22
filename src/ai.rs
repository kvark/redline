use crate::{config, planet, vehicle};

pub struct Driver {
    pub vehicle: vehicle::Vehicle,
    steering: f32,
    target_speed: f32,
}

impl Driver {
    pub fn spawn(
        engine: &mut blade_engine::Engine,
        config: &config::Vehicle,
        track: &[planet::TrackSample],
        track_index: usize,
        lateral_offset: f32,
        target_speed: f32,
        tint: [f32; 4],
    ) -> Self {
        let pose = vehicle::Vehicle::spawn_pose_at(track, track_index, 1.4, lateral_offset);
        let vehicle = vehicle::spawn(engine, config, pose);
        engine.set_color_tint(vehicle.body_handle, tint);
        Self {
            vehicle,
            steering: 0.0,
            target_speed,
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
        let look_ahead = (7 + (speed * 0.32) as usize).min(18);
        let target = track[(nearest + look_ahead) % track.len()];

        let up = pose.position.normalize_or_zero();
        let forward = reject_from(pose.orientation * glam::Vec3::Z, up).normalize_or_zero();
        let desired = reject_from(target.position - pose.position, up).normalize_or_zero();
        let heading_error = desired
            .cross(forward)
            .dot(up)
            .atan2(desired.dot(forward).clamp(-1.0, 1.0));
        let steering_target = (heading_error * 1.8).clamp(-1.0, 1.0);
        let response = 1.0 - (-dt.min(0.1) * 7.0).exp();
        self.steering += (steering_target - self.steering) * response;

        let steering_limit = 0.48 * (1.0 / (1.0 + speed / 42.0)).clamp(0.45, 1.0);
        let target_speed = if speed > self.target_speed * 1.12 {
            0.0
        } else {
            self.target_speed
        };
        self.vehicle
            .drive(engine, target_speed, self.steering * steering_limit, dt);
        self.vehicle.apply_gravity(engine, gravity, dt);
        self.vehicle.apply_stability(engine, dt, self.steering);
    }
}

fn reject_from(vector: glam::Vec3, normal: glam::Vec3) -> glam::Vec3 {
    vector - normal * vector.dot(normal)
}
