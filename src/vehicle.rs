use std::{f32::consts, mem, ops};

use crate::{config, planet};

#[derive(Clone)]
pub struct Wheel {
    pub object: blade_engine::ObjectHandle,
    pub spin_joint: blade_engine::JointHandle,
    pub driven: bool,
    pub suspender: Option<blade_engine::ObjectHandle>,
    pub steer_joint: Option<blade_engine::JointHandle>,
}

pub struct Vehicle {
    pub body_handle: blade_engine::ObjectHandle,
    pub jump_impulse: f32,
    pub roll_impulse: f32,
    pub body_mass: f32,
    pub wheel_mass: f32,
    pub wheels: Vec<Wheel>,
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
        wheel_mass: veh_config.wheel_mass,
        wheels: Vec::new(),
    };

    let wheel_config = blade_engine::config::Object {
        name: "vehicle/wheel".to_string(),
        visuals: vec![clone_visual(&veh_config.wheel.visual)],
        colliders: vec![clone_collider(&veh_config.wheel.collider)],
        additional_mass: None,
    };
    let suspender_config = blade_engine::config::Object {
        name: "vehicle/suspender".to_string(),
        visuals: vec![],
        colliders: vec![],
        additional_mass: Some(clone_mass(&veh_config.suspender)),
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
                blade_engine::DynamicInput::Full,
            );
            let wheel_angular_freedoms = mint::Vector3 {
                x: Some(blade_engine::FreedomAxis {
                    limits: None,
                    motor: axle.driven.then_some(blade_engine::config::Motor {
                        stiffness: 0.0,
                        damping: veh_config.drive_factor,
                        max_force: 1800.0,
                    }),
                }),
                y: None,
                z: None,
            };

            vehicle.wheels.push(
                if axle.max_steering_angle > 0.0 || axle.max_suspension_offset > 0.0 {
                    let max_angle = axle.max_steering_angle.to_radians();
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
                                y: if axle.max_suspension_offset > 0.0 {
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
                                y: if axle.max_steering_angle > 0.0 {
                                    Some(blade_engine::FreedomAxis {
                                        limits: Some(-max_angle..max_angle),
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

                    let wheel_joint = engine.add_joint(
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

                    Wheel {
                        object: wheel_handle,
                        spin_joint: wheel_joint,
                        driven: axle.driven,
                        suspender: Some(suspender_handle),
                        steer_joint: if axle.max_steering_angle > 0.0 {
                            Some(suspension_joint)
                        } else {
                            None
                        },
                    }
                } else {
                    let wheel_joint = engine.add_joint(
                        vehicle.body_handle,
                        wheel_handle,
                        blade_engine::JointDesc {
                            parent_anchor: blade_engine::Transform {
                                position: local.into(),
                                ..Default::default()
                            },
                            child_anchor: blade_engine::Transform {
                                orientation: flip.into(),
                                ..Default::default()
                            },
                            angular: wheel_angular_freedoms,
                            ..Default::default()
                        },
                    );
                    Wheel {
                        object: wheel_handle,
                        spin_joint: wheel_joint,
                        driven: axle.driven,
                        suspender: None,
                        steer_joint: None,
                    }
                },
            );
        }
    }

    vehicle
}

impl Vehicle {
    pub fn set_velocity(&self, engine: &mut blade_engine::Engine, velocity: f32) {
        engine.wake_up(self.body_handle);
        for wheel in self.wheels.iter() {
            if wheel.driven {
                engine.set_joint_motor(
                    wheel.spin_joint,
                    blade_engine::JointAxis::AngularX,
                    0.0,
                    velocity,
                );
            }
        }
    }

    pub fn set_steering(&self, engine: &mut blade_engine::Engine, angle_rad: f32) {
        for wheel in self.wheels.iter() {
            if let Some(handle) = wheel.steer_joint {
                engine.set_joint_motor(handle, blade_engine::JointAxis::AngularY, angle_rad, 0.0);
            }
        }
    }

    pub fn apply_gravity(&self, engine: &mut blade_engine::Engine, gravity: f32, dt: f32) {
        self.apply_radial_impulse(engine, self.body_handle, self.body_mass, gravity, dt);
        for wheel in self.wheels.iter() {
            self.apply_radial_impulse(engine, wheel.object, self.wheel_mass, gravity, dt);
            if let Some(suspender) = wheel.suspender {
                self.apply_radial_impulse(engine, suspender, self.wheel_mass, gravity, dt);
            }
        }
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
        let old = Isometry::from(
            engine.get_object_transform(self.body_handle, blade_engine::Prediction::LastKnown),
        );
        engine.teleport_object(self.body_handle, pose.to_blade());
        let relative = pose.clone() * old.inverse();
        let wheels = mem::take(&mut self.wheels);
        for wheel in wheels.iter() {
            if let Some(suspender) = wheel.suspender {
                teleport_rel(engine, suspender, &relative);
            }
            teleport_rel(engine, wheel.object, &relative);
        }
        self.wheels = wheels;
    }

    pub fn pose(&self, engine: &blade_engine::Engine) -> Isometry {
        engine
            .get_object_transform(
                self.body_handle,
                blade_engine::Prediction::IntegrateVelocityAndForces,
            )
            .into()
    }

    pub fn spawn_pose(track: &[planet::TrackSample], hover: f32) -> Isometry {
        let start = track[0];
        let next = track[1.min(track.len() - 1)];
        let normal = start.normal;
        let tangent = (next.position - start.position).normalize_or_zero();
        Isometry {
            position: start.position + normal * hover,
            orientation: planet::surface_quat(normal, tangent),
        }
    }
}

fn teleport_rel(
    engine: &mut blade_engine::Engine,
    handle: blade_engine::ObjectHandle,
    isometry: &Isometry,
) {
    let prev = engine.get_object_transform(handle, blade_engine::Prediction::LastKnown);
    let next = isometry.clone() * Isometry::from(prev);
    engine.teleport_object(handle, next.to_blade());
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

fn clone_mass(src: &blade_engine::config::AdditionalMass) -> blade_engine::config::AdditionalMass {
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
