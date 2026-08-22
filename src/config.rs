#[derive(serde::Deserialize)]
pub struct Body {
    pub visual: blade_engine::config::Visual,
    pub collider: blade_engine::config::Collider,
}

#[derive(serde::Deserialize)]
pub struct Wheel {
    pub visual: blade_engine::config::Visual,
    pub collider: blade_engine::config::Collider,
}

#[derive(serde::Deserialize)]
pub struct Axle {
    /// Side offset for each wheel.
    pub x_wheels: Vec<f32>,
    /// Height offset from the body.
    pub y: f32,
    /// Forward offset from the body.
    pub z: f32,
    #[serde(default)]
    pub max_steering_angle: f32,
    #[serde(default)]
    pub max_suspension_offset: f32,
    #[serde(default)]
    pub suspension: blade_engine::config::Motor,
    #[serde(default)]
    pub steering: blade_engine::config::Motor,
}

fn default_additional_mass() -> blade_engine::config::AdditionalMass {
    blade_engine::config::AdditionalMass {
        density: 0.0,
        shape: blade_engine::config::Shape::Ball { radius: 0.0 },
    }
}

#[derive(serde::Deserialize)]
pub struct Vehicle {
    pub body: Body,
    pub wheel: Wheel,
    #[serde(default = "default_additional_mass")]
    pub suspender: blade_engine::config::AdditionalMass,
    pub drive_factor: f32,
    pub jump_impulse: f32,
    pub roll_impulse: f32,
    pub body_mass: f32,
    pub wheel_mass: f32,
    pub axles: Vec<Axle>,
}

#[derive(Clone, Copy)]
pub struct Camera {
    pub azimuth: f32,
    pub altitude: f32,
    pub distance: f32,
    pub height: f32,
    pub speed: f32,
    pub fov: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            azimuth: 0.0,
            altitude: 0.35,
            distance: 8.0,
            height: 1.6,
            speed: 6.0,
            fov: 1.05,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Planet {
    pub radius: f32,
    pub subdivisions: u32,
    pub height_amp: f32,
    pub track_width: f32,
    pub track_lat_amp: f32,
    pub gravity: f32,
    pub seed: u32,
    pub decoration_count: u32,
}

impl Default for Planet {
    fn default() -> Self {
        Self {
            radius: 80.0,
            subdivisions: 5,
            height_amp: 7.2,
            track_width: 14.0,
            track_lat_amp: 0.28,
            gravity: 20.0,
            seed: 0x4D415253,
            decoration_count: 360,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Race {
    pub laps: u32,
    pub checkpoint_count: u32,
}

impl Default for Race {
    fn default() -> Self {
        Self {
            laps: 3,
            checkpoint_count: 16,
        }
    }
}
