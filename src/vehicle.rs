use std::f32::consts;

use crate::{config, planet};

#[derive(Clone)]
pub struct Wheel {
    pub object: blade_engine::ObjectHandle,
    local_position: glam::Vec3,
    flip: glam::Quat,
    steerable: bool,
}

pub struct Vehicle {
    pub body_handle: blade_engine::ObjectHandle,
    pub jump_impulse: f32,
    pub roll_impulse: f32,
    pub body_mass: f32,
    pub wheels: Vec<Wheel>,
    steering_angle: f32,
    wheel_spin: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Isometry {
    pub position: glam::Vec3,
    pub orientation: glam::Quat,
}

impl From<blade_engine::Transform> for Isometry {
    fn from(transform: blade_engine::Transform) -> Self {
        Self {
            position: transform.position.into(),
            orientation: transform.orientation.into(),
        }
    }
}

impl Isometry {
    pub fn to_blade(&self) -> blade_engine::Transform {
        blade_engine::Transform {
            position: self.position.into(),
            orientation: self.orientation.into(),
        }
    }
}

pub fn spawn(
    engine: &mut blade_engine::Engine,
    veh_config: &config::Vehicle,
    pose: Isometry,
) -> Vehicle {
    let body_config = blade_engine::config::Object {
        name: "vehicle/body".to_string(),
        visuals: vec![clone_visual(&veh_config.body.visual)],
        colliders: vec![clone_collider(&veh_config.body.collider)],
        additional_mass: None,
    };
    let mut vehicle = Vehicle {
        body_handle: engine.add_object(
            &body_config,
            pose.to_blade(),
            blade_engine::DynamicInput::Full,
        ),
        jump_impulse: veh_config.jump_impulse,
        roll_impulse: veh_config.roll_impulse,
        body_mass: veh_config.body_mass,
        wheels: Vec::new(),
        steering_angle: 0.0,
        wheel_spin: 0.0,
    };
    // The car can cover more than one small terrain triangle per simulation step.
    // Swept collision detection prevents it from tunnelling into rocks or the road.
    engine.set_ccd_enabled(vehicle.body_handle, true);

    let wheel_config = blade_engine::config::Object {
        name: "vehicle/wheel".to_string(),
        visuals: vec![clone_visual(&veh_config.wheel.visual)],
        colliders: vec![],
        additional_mass: None,
    };

    let spawn_pos = pose.position;
    let spawn_rot = pose.orientation;
    for axle in veh_config.axles.iter() {
        for &wheel_x in axle.x_wheels.iter() {
            let local = glam::Vec3::new(wheel_x, axle.y, axle.z);
            let offset = spawn_rot * local;
            let flip = if wheel_x > 0.0 {
                glam::Quat::from_rotation_y(consts::PI)
            } else {
                glam::Quat::IDENTITY
            };
            let wheel_rot = spawn_rot * flip;

            let wheel_handle = engine.add_object(
                &wheel_config,
                blade_engine::Transform {
                    position: (spawn_pos + offset).into(),
                    orientation: wheel_rot.into(),
                },
                blade_engine::DynamicInput::SetPosition,
            );
            vehicle.wheels.push(Wheel {
                object: wheel_handle,
                local_position: local,
                flip,
                steerable: axle.max_steering_angle > 0.0,
            });
        }
    }

    vehicle
}

impl Vehicle {
    pub fn drive(
        &mut self,
        engine: &mut blade_engine::Engine,
        target_speed: f32,
        steering_angle: f32,
        dt: f32,
    ) {
        engine.wake_up(self.body_handle);
        self.steering_angle = steering_angle;
        let pose = self.pose(engine);
        let up = pose.position.normalize_or_zero();
        let forward = (pose.orientation * glam::Vec3::Z).reject_from(up);
        if forward.length_squared() < 1e-5 {
            return;
        }
        let forward = forward.normalize();
        let (linear, angular) = engine.get_velocity(self.body_handle);
        let linear = glam::Vec3::from(linear);
        let angular = glam::Vec3::from(angular);
        let forward_speed = linear.dot(forward);
        let speed_error = (target_speed - forward_speed).clamp(-12.0, 12.0);
        let drive_impulse = forward * speed_error * self.body_mass * 3.2 * dt.min(0.05);
        engine.apply_linear_impulse(self.body_handle, drive_impulse.into());

        let desired_yaw = forward_speed * steering_angle.tan() / 1.55;
        let yaw_error = desired_yaw - angular.dot(up);
        let steering_impulse = up * yaw_error * self.body_mass * 1.5 * dt.min(0.05);
        engine.apply_angular_impulse(self.body_handle, steering_impulse.into());
        self.wheel_spin += forward_speed * dt / 0.28;
    }

    pub fn apply_gravity(&self, engine: &mut blade_engine::Engine, gravity: f32, dt: f32) {
        self.apply_radial_impulse(engine, self.body_handle, self.body_mass, gravity, dt);
    }

    /// Keep the chassis aligned with the local horizon while preserving yaw and jumps.
    /// This behaves like a low center of gravity rather than a hard orientation lock.
    pub fn apply_stability(&self, engine: &mut blade_engine::Engine, dt: f32, steering: f32) {
        let pose = self.pose(engine);
        let radial_up = pose.position.normalize_or_zero();
        let body_up = pose.orientation * glam::Vec3::Y;
        if radial_up.length_squared() < 1e-6 {
            return;
        }

        let (linear, angular) = engine.get_velocity(self.body_handle);
        let linear = glam::Vec3::from(linear);
        let angular = glam::Vec3::from(angular);
        let yaw = radial_up * angular.dot(radial_up);
        let roll_pitch = angular - yaw;
        let error_axis = body_up.cross(radial_up);
        let upright = body_up.dot(radial_up);

        // A nearly upside-down cross product has no preferred direction. Use the car's
        // forward axis as a deterministic escape so it cannot settle on its roof.
        let correction_axis = if error_axis.length_squared() < 1e-5 && upright < 0.0 {
            pose.orientation * glam::Vec3::Z
        } else {
            error_axis
        };
        let strength = if upright < 0.15 { 18.0 } else { 10.0 };
        let damping = 2.8;
        let impulse =
            (correction_axis * strength - roll_pitch * damping) * self.body_mass * dt.min(0.05);
        engine.apply_angular_impulse(self.body_handle, impulse.into());

        // Rapier contact friction does not distinguish tire rolling and lateral directions.
        // Remove lateral scrub explicitly and damp local yaw only while steering is centered.
        // This lets the car hold a heading without fighting intentional cornering.
        let forward = (pose.orientation * glam::Vec3::Z).reject_from(radial_up);
        if forward.length_squared() > 1e-5 {
            let right = radial_up.cross(forward.normalize()).normalize_or_zero();
            let lateral_speed = linear.dot(right);
            let grip = if steering.abs() < 0.05 { 7.0 } else { 3.0 };
            let lateral_impulse = -right * lateral_speed * self.body_mass * grip * dt.min(0.05);
            engine.apply_linear_impulse(self.body_handle, lateral_impulse.into());

            let center_weight = (1.0 - steering.abs()).clamp(0.0, 1.0).powi(2);
            let yaw_rate = angular.dot(radial_up);
            let yaw_impulse =
                -radial_up * yaw_rate * self.body_mass * 0.9 * center_weight * dt.min(0.05);
            engine.apply_angular_impulse(self.body_handle, yaw_impulse.into());
        }
    }

    fn apply_radial_impulse(
        &self,
        engine: &mut blade_engine::Engine,
        handle: blade_engine::ObjectHandle,
        mass: f32,
        gravity: f32,
        dt: f32,
    ) {
        let pos = glam::Vec3::from(engine.get_object_position(handle));
        let dir = -pos.normalize_or_zero();
        if dir.length_squared() < 1e-6 {
            return;
        }
        engine.wake_up(handle);
        engine.apply_linear_impulse(handle, (dir * mass * gravity * dt).into());
    }

    pub fn teleport(&mut self, engine: &mut blade_engine::Engine, pose: &Isometry) {
        engine.teleport_object(self.body_handle, pose.to_blade());
        self.sync_wheels_to_pose(engine, pose);
    }

    pub fn pose(&self, engine: &blade_engine::Engine) -> Isometry {
        engine
            .get_object_transform(
                self.body_handle,
                blade_engine::Prediction::IntegrateVelocityAndForces,
            )
            .into()
    }

    pub fn sync_wheels(&self, engine: &mut blade_engine::Engine) {
        let pose = self.pose(engine);
        self.sync_wheels_to_pose(engine, &pose);
    }

    fn sync_wheels_to_pose(&self, engine: &mut blade_engine::Engine, pose: &Isometry) {
        for wheel in self.wheels.iter() {
            let wheel_pose = wheel_visual_pose(
                pose,
                wheel.local_position,
                wheel.flip,
                wheel.steerable,
                self.steering_angle,
                self.wheel_spin,
            );
            engine.teleport_object(wheel.object, wheel_pose.to_blade());
        }
    }

    pub fn spawn_pose(track: &[planet::TrackSample], hover: f32) -> Isometry {
        Self::spawn_pose_at(track, 0, hover, 0.0)
    }

    pub fn spawn_pose_at(
        track: &[planet::TrackSample],
        index: usize,
        hover: f32,
        lateral_offset: f32,
    ) -> Isometry {
        let index = index % track.len();
        let start = track[index];
        let next = track[(index + 1) % track.len()];
        let normal = start.normal;
        let tangent = (next.position - start.position).normalize_or_zero();
        let side = normal.cross(tangent).normalize_or_zero();
        Isometry {
            position: start.position + normal * hover + side * lateral_offset,
            orientation: planet::surface_quat(normal, tangent),
        }
    }
}

fn wheel_visual_pose(
    body_pose: &Isometry,
    local_position: glam::Vec3,
    flip: glam::Quat,
    steerable: bool,
    steering_angle: f32,
    wheel_spin: f32,
) -> Isometry {
    let steer = if steerable {
        glam::Quat::from_rotation_y(steering_angle)
    } else {
        glam::Quat::IDENTITY
    };
    let spin = glam::Quat::from_rotation_x(wheel_spin);
    Isometry {
        position: body_pose.position + body_pose.orientation * local_position,
        // Apply the cosmetic side flip last. Putting it before `spin` mirrors
        // the spin axis and makes the two sides appear to counter-rotate.
        orientation: body_pose.orientation * steer * spin * flip,
    }
}

fn clone_visual(src: &blade_engine::config::Visual) -> blade_engine::config::Visual {
    blade_engine::config::Visual {
        model: src.model.clone(),
        front_face: match src.front_face {
            blade_engine::config::FrontFace::Cw => blade_engine::config::FrontFace::Cw,
            blade_engine::config::FrontFace::Ccw => blade_engine::config::FrontFace::Ccw,
        },
        pos: src.pos,
        rot: src.rot,
        scale: src.scale,
    }
}

fn clone_collider(src: &blade_engine::config::Collider) -> blade_engine::config::Collider {
    blade_engine::config::Collider {
        density: src.density,
        shape: clone_shape(&src.shape),
        friction: src.friction,
        restitution: src.restitution,
        pos: src.pos,
        rot: src.rot,
    }
}

fn clone_shape(src: &blade_engine::config::Shape) -> blade_engine::config::Shape {
    match *src {
        blade_engine::config::Shape::Ball { radius } => {
            blade_engine::config::Shape::Ball { radius }
        }
        blade_engine::config::Shape::Cylinder {
            half_height,
            radius,
        } => blade_engine::config::Shape::Cylinder {
            half_height,
            radius,
        },
        blade_engine::config::Shape::Cuboid { half } => {
            blade_engine::config::Shape::Cuboid { half }
        }
        blade_engine::config::Shape::ConvexHull {
            ref points,
            border_radius,
        } => blade_engine::config::Shape::ConvexHull {
            points: points.clone(),
            border_radius,
        },
        blade_engine::config::Shape::TriMesh {
            ref model,
            convex,
            border_radius,
        } => blade_engine::config::Shape::TriMesh {
            model: model.clone(),
            convex,
            border_radius,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn left_and_right_wheels_have_the_same_world_spin_direction() {
        let body = Isometry {
            position: glam::Vec3::ZERO,
            orientation: glam::Quat::IDENTITY,
        };
        let step = 0.2;
        let mut deltas = Vec::new();
        for x in [-0.5, 0.5] {
            let flip = if x > 0.0 {
                glam::Quat::from_rotation_y(consts::PI)
            } else {
                glam::Quat::IDENTITY
            };
            let local_position = glam::Vec3::new(x, -0.1, 0.7);
            let before = wheel_visual_pose(&body, local_position, flip, true, 0.3, 0.0).orientation;
            let after = wheel_visual_pose(&body, local_position, flip, true, 0.3, step).orientation;
            deltas.push(after * before.inverse());
        }
        assert!(deltas[0].dot(deltas[1]).abs() > 0.9999);
    }
}
