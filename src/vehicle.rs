use std::{f32::consts, mem, ops};

use crate::{config, planet};

pub const SPAWN_HOVER: f32 = 0.55;

#[derive(Clone)]
pub struct Wheel {
    pub object: blade_engine::ObjectHandle,
    spin_joint: blade_engine::JointHandle,
    suspender: Option<blade_engine::ObjectHandle>,
    steer_joint: Option<blade_engine::JointHandle>,
    /// Body-space X of the wheel. Negative is left.
    lateral: f32,
}

pub struct Vehicle {
    pub body_handle: blade_engine::ObjectHandle,
    pub jump_impulse: f32,
    pub roll_impulse: f32,
    pub body_mass: f32,
    pub wheels: Vec<Wheel>,
    wheel_radius: f32,
    wheel_mass: f32,
    stuck_time: f32,
    inverted_time: f32,
    prev_speed: f32,
    hopped: bool,
    recoil: f32,
}

/// Visual / stance override used so opponents are not clones of the player car.
#[derive(Clone, Copy)]
pub struct Kit {
    pub body_model: &'static str,
    pub wheel_model: &'static str,
    pub tint: [f32; 4],
    pub half_track: f32,
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
    pub fn inverse(&self) -> Self {
        let orientation = self.orientation.inverse();
        Self {
            position: orientation * -self.position,
            orientation,
        }
    }

    pub fn to_blade(&self) -> blade_engine::Transform {
        blade_engine::Transform {
            position: self.position.into(),
            orientation: self.orientation.into(),
        }
    }
}

impl ops::Mul<Isometry> for Isometry {
    type Output = Self;
    fn mul(self, other: Self) -> Self {
        Self {
            position: self.orientation * other.position + self.position,
            orientation: self.orientation * other.orientation,
        }
    }
}

pub fn spawn(
    engine: &mut blade_engine::Engine,
    veh_config: &config::Vehicle,
    pose: Isometry,
    kit: Option<Kit>,
) -> Vehicle {
    let mut body_visual = clone_visual(&veh_config.body.visual);
    let mut wheel_visual = clone_visual(&veh_config.wheel.visual);
    if let Some(kit) = kit {
        body_visual.model = kit.body_model.to_string();
        body_visual.pos = mint::Vector3 {
            x: 0.0,
            y: -0.05,
            z: 0.0,
        };
        // Kenney car-kit bodies already face +Z. race-future in vehicle.ron
        // uses a 180 yaw; applying that here made opponents drive backwards.
        body_visual.rot = mint::Vector3 {
            x: 0.0,
            y: 0.0,
            z: 0.0,
        };
        wheel_visual.model = kit.wheel_model.to_string();
    }
    let body_config = blade_engine::config::Object {
        name: "vehicle/body".to_string(),
        visuals: vec![body_visual],
        colliders: vec![clone_collider(&veh_config.body.collider)],
        additional_mass: None,
    };
    let body_handle = engine.add_object(
        &body_config,
        pose.to_blade(),
        blade_engine::DynamicInput::Full,
    );
    engine.set_ccd_enabled(body_handle, true);
    if let Some(kit) = kit {
        engine.set_color_tint(body_handle, kit.tint);
    }

    let wheel_radius = shape_radius(&veh_config.wheel.collider.shape).unwrap_or(0.28);
    let mut vehicle = Vehicle {
        body_handle,
        jump_impulse: veh_config.jump_impulse,
        roll_impulse: veh_config.roll_impulse,
        body_mass: veh_config.body_mass,
        wheels: Vec::new(),
        wheel_radius,
        wheel_mass: 8.0,
        stuck_time: 0.0,
        inverted_time: 0.0,
        prev_speed: 0.0,
        hopped: false,
        recoil: 0.0,
    };
    let wheel_config = blade_engine::config::Object {
        name: "vehicle/wheel".to_string(),
        visuals: vec![wheel_visual],
        colliders: vec![clone_collider(&veh_config.wheel.collider)],
        additional_mass: None,
    };
    let suspender_config = blade_engine::config::Object {
        name: "vehicle/suspender".to_string(),
        visuals: vec![],
        colliders: vec![],
        additional_mass: Some(clone_additional_mass(&veh_config.suspender)),
    };

    let spawn_pos = pose.position;
    let spawn_rot = pose.orientation;
    for axle in veh_config.axles.iter() {
        let wheel_xs: Vec<f32> = match kit {
            Some(kit) => axle
                .x_wheels
                .iter()
                .map(|x| x.signum() * kit.half_track)
                .collect(),
            None => axle.x_wheels.clone(),
        };
        for &wheel_x in wheel_xs.iter() {
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
                blade_engine::DynamicInput::Full,
            );

            let wheel_angular_freedoms = mint::Vector3 {
                x: Some(blade_engine::FreedomAxis {
                    limits: None,
                    motor: Some(blade_engine::config::Motor {
                        stiffness: 0.0,
                        damping: veh_config.drive_factor,
                        max_force: 2800.0,
                    }),
                }),
                y: None,
                z: None,
            };

            let max_steer = axle.max_steering_angle.to_radians();
            let has_steer = max_steer > 0.0;
            let has_suspension = axle.max_suspension_offset > 0.0;
            let suspender_handle = engine.add_object(
                &suspender_config,
                blade_engine::Transform {
                    position: (spawn_pos + offset).into(),
                    orientation: spawn_rot.into(),
                },
                blade_engine::DynamicInput::Full,
            );

            let suspension_joint = engine.add_joint(
                vehicle.body_handle,
                suspender_handle,
                blade_engine::JointDesc {
                    parent_anchor: blade_engine::Transform {
                        position: local.into(),
                        ..Default::default()
                    },
                    linear: mint::Vector3 {
                        x: None,
                        y: if has_suspension {
                            Some(blade_engine::FreedomAxis {
                                limits: Some(0.0..axle.max_suspension_offset),
                                motor: Some(axle.suspension),
                            })
                        } else {
                            None
                        },
                        z: None,
                    },
                    angular: mint::Vector3 {
                        x: None,
                        y: if has_steer {
                            Some(blade_engine::FreedomAxis {
                                limits: Some(-max_steer..max_steer),
                                motor: Some(axle.steering),
                            })
                        } else {
                            None
                        },
                        z: None,
                    },
                    ..Default::default()
                },
            );

            let spin_joint = engine.add_joint(
                suspender_handle,
                wheel_handle,
                blade_engine::JointDesc {
                    child_anchor: blade_engine::Transform {
                        orientation: flip.into(),
                        ..Default::default()
                    },
                    angular: wheel_angular_freedoms,
                    ..Default::default()
                },
            );

            // Contact filter: wheels must not collide with the chassis they hang from.
            let _ = engine.add_joint(
                vehicle.body_handle,
                wheel_handle,
                blade_engine::JointDesc {
                    linear: blade_engine::FreedomAxis::ALL_FREE,
                    angular: blade_engine::FreedomAxis::ALL_FREE,
                    ..Default::default()
                },
            );

            vehicle.wheels.push(Wheel {
                object: wheel_handle,
                spin_joint,
                suspender: Some(suspender_handle),
                steer_joint: if has_steer {
                    Some(suspension_joint)
                } else {
                    None
                },
                lateral: wheel_x,
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
        let mut target_speed = target_speed;
        if self.recoil > 0.0 {
            let damp = (self.recoil / 0.5).clamp(0.0, 1.0);
            self.recoil = (self.recoil - dt).max(0.0);
            target_speed *= 1.0 - 0.75 * damp;
        }
        engine.wake_up(self.body_handle);
        let pose = self.pose(engine);
        let up = pose.position.normalize_or_zero();
        let forward = (pose.orientation * glam::Vec3::Z).reject_from(up);
        let forward = forward.normalize_or_zero();
        let (linear, angular) = engine.get_velocity(self.body_handle);
        let linear = glam::Vec3::from(linear);
        let angular = glam::Vec3::from(angular);
        let forward_speed = linear.dot(forward);
        // Bicycle-model yaw for a 1.5m wheelbase. The wheel motors supply most of
        // the motion; this is a contact assist so high-speed steering still bites
        // on a sphere where the tire patch is tiny.
        let wheelbase = 1.5;
        let desired_yaw = forward_speed * steering_angle / wheelbase;
        for wheel in self.wheels.iter() {
            let wheel_speed = target_speed + desired_yaw * wheel.lateral;
            engine.set_joint_motor(
                wheel.spin_joint,
                blade_engine::JointAxis::AngularX,
                0.0,
                wheel_omega(wheel_speed, self.wheel_radius),
            );
            if let Some(handle) = wheel.steer_joint {
                engine.set_joint_motor(
                    handle,
                    blade_engine::JointAxis::AngularY,
                    steering_angle,
                    0.0,
                );
            }
        }

        let yaw_error = desired_yaw - angular.dot(up);
        if yaw_error.abs() > 1e-4 && forward.length_squared() > 1e-5 {
            let assist = up * yaw_error * self.body_mass * 1.1 * dt.min(0.05);
            engine.apply_angular_impulse(self.body_handle, assist.into());
        }

        // If the tires have lost the ground (beached on a rock, hung on a lip),
        // the spin motors cannot recover. A small chassis shove unsticks without
        // replacing ordinary wheel drive.
        if forward.length_squared() > 1e-5 && target_speed.abs() > 2.0 && forward_speed.abs() < 0.9
        {
            let shove = forward * target_speed.signum() * self.body_mass * 7.0 * dt.min(0.05);
            engine.apply_linear_impulse(self.body_handle, shove.into());
        }
    }

    pub fn apply_gravity(&self, engine: &mut blade_engine::Engine, gravity: f32, dt: f32) {
        self.apply_radial_impulse(engine, self.body_handle, self.body_mass, gravity, dt);
        for wheel in self.wheels.iter() {
            self.apply_radial_impulse(engine, wheel.object, self.wheel_mass, gravity, dt);
        }
    }

    /// Keep the chassis from settling inverted on a sphere. Yaw and ordinary
    /// roll from the suspension are left to the joints.
    pub fn apply_stability(&self, engine: &mut blade_engine::Engine, dt: f32) {
        let pose = self.pose(engine);
        let radial_up = pose.position.normalize_or_zero();
        let body_up = pose.orientation * glam::Vec3::Y;
        if radial_up.length_squared() < 1e-6 {
            return;
        }

        let (_, angular) = engine.get_velocity(self.body_handle);
        let angular = glam::Vec3::from(angular);
        let yaw = radial_up * angular.dot(radial_up);
        let roll_pitch = angular - yaw;
        let error_axis = body_up.cross(radial_up);
        let upright = body_up.dot(radial_up);
        let correction_axis = if error_axis.length_squared() < 1e-5 && upright < 0.0 {
            pose.orientation * glam::Vec3::Z
        } else {
            error_axis
        };
        let strength = if upright < 0.15 {
            14.0
        } else if upright < 0.55 {
            5.0
        } else {
            1.6
        };
        let impulse =
            (correction_axis * strength - roll_pitch * 2.2) * self.body_mass * dt.min(0.05);
        engine.apply_angular_impulse(self.body_handle, impulse.into());
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
        let old = Isometry::from(
            engine.get_object_transform(self.body_handle, blade_engine::Prediction::LastKnown),
        );
        engine.teleport_object(self.body_handle, pose.to_blade());
        let relative = pose.clone() * old.inverse();
        let wheels = mem::take(&mut self.wheels);
        for wheel in wheels.iter() {
            if let Some(suspender) = wheel.suspender {
                teleport_relative(engine, suspender, &relative);
            }
            teleport_relative(engine, wheel.object, &relative);
        }
        self.wheels = wheels;
        self.stuck_time = 0.0;
        self.inverted_time = 0.0;
        self.prev_speed = 0.0;
        self.hopped = false;
        self.recoil = 0.0;
        let up = pose.position.normalize_or_zero();
        let fwd = (pose.orientation * glam::Vec3::Z).reject_from(up);
        if fwd.length_squared() > 1e-5 {
            engine.apply_linear_impulse(
                self.body_handle,
                (fwd.normalize() * self.body_mass * 10.0).into(),
            );
        }
    }

    pub fn wheel_radius(&self) -> f32 {
        self.wheel_radius
    }

    pub fn register_bump(&mut self) {
        self.recoil = self.recoil.max(0.5);
    }

    pub fn is_recoiling(&self) -> bool {
        self.recoil > 0.0
    }

    pub fn recoil_time(&self) -> f32 {
        self.recoil
    }

    pub fn bump_handles(&self) -> impl Iterator<Item = blade_engine::ObjectHandle> + '_ {
        std::iter::once(self.body_handle).chain(self.wheels.iter().map(|wheel| wheel.object))
    }

    pub fn pose(&self, engine: &blade_engine::Engine) -> Isometry {
        engine
            .get_object_transform(
                self.body_handle,
                blade_engine::Prediction::IntegrateVelocityAndForces,
            )
            .into()
    }

    pub fn motion(
        &self,
        engine: &blade_engine::Engine,
    ) -> (Isometry, glam::Vec3, glam::Vec3, f32, f32) {
        let pose = self.pose(engine);
        let (linear, angular) = engine.get_velocity(self.body_handle);
        let linear = glam::Vec3::from(linear);
        let angular = glam::Vec3::from(angular);
        let up = pose.position.normalize_or_zero();
        let forward = (pose.orientation * glam::Vec3::Z).reject_from(up);
        let forward = forward.normalize_or_zero();
        let right = up.cross(forward).normalize_or_zero();
        (
            pose,
            linear,
            angular,
            linear.dot(forward),
            linear.dot(right),
        )
    }

    /// Lift, reorient, and nudge toward the road when the car is inverted or wedged.
    pub fn recover_if_needed(
        &mut self,
        engine: &mut blade_engine::Engine,
        track: &[planet::TrackSample],
        track_width: f32,
        hover: f32,
        powered: bool,
        dt: f32,
    ) -> bool {
        let pose = self.pose(engine);
        let (linear, _) = engine.get_velocity(self.body_handle);
        let speed = glam::Vec3::from(linear).length();
        let up = pose.position.normalize_or_zero();
        let upright = (pose.orientation * glam::Vec3::Y).dot(up);
        let query = planet::query_track(pose.position, track);
        let off = planet::off_track_distance(&query, track_width);

        if upright < 0.2 {
            self.inverted_time += dt.min(0.1);
        } else {
            self.inverted_time = 0.0;
        }
        let impact = self.prev_speed > 7.0 && speed < 1.8;
        let wedged = speed < 1.3 && (off > 0.4 || upright < 0.55);
        let spinning_tires = powered && speed < 0.45;
        if wedged || spinning_tires || impact {
            let gain = if impact { 3.0 } else { 1.0 };
            self.stuck_time += dt.min(0.1) * gain;
        } else {
            self.stuck_time = 0.0;
            self.hopped = false;
        }
        self.prev_speed = speed;

        if !self.hopped && self.stuck_time > 0.28 {
            // A radial hop plus a shove along heading usually clears a lip
            // without the full reorient. Teleport is the fallback.
            self.hopped = true;
            let fwd = (pose.orientation * glam::Vec3::Z).reject_from(up);
            let fwd = fwd.normalize_or_zero();
            let hop = up * self.body_mass * 9.0 + fwd * self.body_mass * 11.0;
            engine.apply_linear_impulse(self.body_handle, hop.into());
            engine.wake_up(self.body_handle);
        }

        let should_recover = self.inverted_time > 0.7 || self.stuck_time > 1.05;
        if !should_recover {
            return false;
        }
        let recovered = recovery_pose(&pose, &query, hover, off > 0.5);
        self.teleport(engine, &recovered);
        true
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

fn teleport_relative(
    engine: &mut blade_engine::Engine,
    handle: blade_engine::ObjectHandle,
    relative: &Isometry,
) {
    let prev =
        Isometry::from(engine.get_object_transform(handle, blade_engine::Prediction::LastKnown));
    engine.teleport_object(handle, (relative.clone() * prev).to_blade());
}

pub fn wheel_omega(speed: f32, radius: f32) -> f32 {
    speed / radius.max(1e-3)
}

pub fn recovery_pose(
    pose: &Isometry,
    query: &planet::TrackQuery,
    hover: f32,
    toward_track: bool,
) -> Isometry {
    let up = query.sample.normal.normalize_or_zero();
    let up = if up.length_squared() < 1e-6 {
        pose.position.normalize_or_zero()
    } else {
        up
    };
    let fwd = (pose.orientation * glam::Vec3::Z).reject_from(up);
    let fwd = if fwd.length_squared() < 1e-5 {
        query.tangent.reject_from(up)
    } else {
        fwd
    };
    let buried = (0.35 - query.radial_error).max(0.0);
    let mut position = pose.position + up * (hover.max(0.7) + buried);
    if toward_track {
        position -= query.side * query.lateral.signum() * query.lateral.abs().min(3.2);
        position += up * 0.25;
    }
    Isometry {
        position,
        orientation: planet::surface_quat(up, fwd),
    }
}

fn shape_radius(shape: &blade_engine::config::Shape) -> Option<f32> {
    match *shape {
        blade_engine::config::Shape::Ball { radius } => Some(radius),
        blade_engine::config::Shape::Cylinder { radius, .. } => Some(radius),
        _ => None,
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

fn clone_additional_mass(
    src: &blade_engine::config::AdditionalMass,
) -> blade_engine::config::AdditionalMass {
    blade_engine::config::AdditionalMass {
        density: src.density,
        shape: clone_shape(&src.shape),
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
    fn wheel_omega_matches_target_speed() {
        assert!((wheel_omega(28.0, 0.28) - 100.0).abs() < 1e-4);
    }

    #[test]
    fn recovery_pose_stands_up_and_leans_toward_the_road() {
        let pose = Isometry {
            position: glam::Vec3::new(4.0, 80.0, 0.0),
            orientation: glam::Quat::from_rotation_x(consts::PI),
        };
        let query = planet::TrackQuery {
            index: 0,
            sample: planet::TrackSample {
                position: glam::Vec3::new(0.0, 80.0, 0.0),
                tangent: glam::Vec3::Z,
                normal: glam::Vec3::Y,
            },
            tangent: glam::Vec3::Z,
            side: glam::Vec3::X,
            lateral: 4.0,
            radial_error: -0.2,
        };
        let recovered = recovery_pose(&pose, &query, 1.2, true);
        let up = recovered.position.normalize();
        assert!((recovered.orientation * glam::Vec3::Y).dot(up) > 0.95);
        assert!(recovered.position.x.abs() < pose.position.x.abs());
        assert!(recovered.position.y > pose.position.y);
    }
}
