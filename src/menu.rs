//! Pre-race craft and circuit selection (championship / planet-ribbon fantasy).

use crate::{config, vehicle};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VehicleId {
    #[default]
    RaceFuture,
    Hatchback,
    SedanSports,
    Taxi,
}

impl VehicleId {
    pub const ALL: [Self; 4] = [
        Self::RaceFuture,
        Self::Hatchback,
        Self::SedanSports,
        Self::Taxi,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::RaceFuture => "Vector Spear",
            Self::Hatchback => "Ember Hatch",
            Self::SedanSports => "Ion Coupe",
            Self::Taxi => "Stripe Cab",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::RaceFuture => "Flagship anti-grav craft — clean lines, championship baseline.",
            Self::Hatchback => "Hot hatch on a ribbon — light, snappy throttle, sticky tires.",
            Self::SedanSports => "Low coupe — softer launch, planted grip, long-gear feel.",
            Self::Taxi => "Championship wildcard — heavy chassis, lazy jump, loose rear.",
        }
    }

    pub fn kit(self) -> Option<vehicle::Kit> {
        match self {
            Self::RaceFuture => None,
            Self::Hatchback => Some(KIT_HATCH),
            Self::SedanSports => Some(KIT_SEDAN),
            Self::Taxi => Some(KIT_TAXI),
        }
    }
}

// Drive-feel scales are relative to assets/vehicle.ron (Vector Spear baseline).
// AI opponents reuse these kits, so player and field share the same personality.

pub const KIT_HATCH: vehicle::Kit = vehicle::Kit {
    body_model: "models/hatchback-sports-body.glb",
    wheel_model: "models/wheel-racing.glb",
    tint: [1.0, 0.42, 0.32, 1.0],
    half_track: 0.32,
    drive_factor_scale: 1.28,
    motor_max_force_scale: 1.15,
    body_mass_scale: 0.82,
    wheel_friction_scale: 1.12,
    grip_scale: 1.18,
    jump_impulse: Some(16.5),
};

pub const KIT_SEDAN: vehicle::Kit = vehicle::Kit {
    body_model: "models/sedan-sports-body.glb",
    wheel_model: "models/wheel-dark.glb",
    tint: [0.42, 0.72, 1.0, 1.0],
    half_track: 0.32,
    drive_factor_scale: 0.78,
    motor_max_force_scale: 1.05,
    body_mass_scale: 1.05,
    wheel_friction_scale: 1.08,
    grip_scale: 1.22,
    jump_impulse: Some(11.0),
};

pub const KIT_TAXI: vehicle::Kit = vehicle::Kit {
    body_model: "models/taxi-body.glb",
    wheel_model: "models/wheel-dark.glb",
    tint: [1.0, 1.0, 1.0, 1.0],
    half_track: 0.32,
    drive_factor_scale: 0.88,
    motor_max_force_scale: 1.25,
    body_mass_scale: 1.28,
    wheel_friction_scale: 0.88,
    grip_scale: 0.86,
    jump_impulse: Some(8.0),
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MapId {
    #[default]
    MarsClassic,
    PhobosTight,
    HellasWide,
    OlympusClimb,
}

impl MapId {
    pub const ALL: [Self; 4] = [
        Self::MarsClassic,
        Self::PhobosTight,
        Self::HellasWide,
        Self::OlympusClimb,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::MarsClassic => "Mars Equator",
            Self::PhobosTight => "Phobos Needle",
            Self::HellasWide => "Hellas Sweep",
            Self::OlympusClimb => "Olympus Ascent",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::MarsClassic => {
                "Flagship circuit — the rusty equatorial ribbon that started it all."
            }
            Self::PhobosTight => "Moonlet sprint — thin ribbon, no room to breathe.",
            Self::HellasWide => "Basin flyer — wide line for high-speed drafts.",
            Self::OlympusClimb => "Volcano climb — hard latitude swings and steep walls.",
        }
    }

    pub fn planet(self) -> config::Planet {
        match self {
            Self::MarsClassic => config::Planet::default(),
            Self::PhobosTight => config::Planet {
                radius: 55.0,
                track_width: 9.5,
                height_amp: 5.5,
                track_lat_amp: 0.22,
                seed: 0x50484F42,
                decoration_count: 220,
                ..config::Planet::default()
            },
            Self::HellasWide => config::Planet {
                radius: 95.0,
                track_width: 20.0,
                height_amp: 6.0,
                track_lat_amp: 0.24,
                seed: 0x48454C4C,
                decoration_count: 420,
                ..config::Planet::default()
            },
            Self::OlympusClimb => config::Planet {
                radius: 82.0,
                track_width: 13.0,
                height_amp: 11.5,
                track_lat_amp: 0.42,
                seed: 0x4F4C594D,
                decoration_count: 380,
                ..config::Planet::default()
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kit scales are `const`; keep these as runtime checks so Clippy does not
    /// treat them as `assertions_on_constants`.
    fn lt(a: f32, b: f32) -> bool {
        a < b
    }
    fn gt(a: f32, b: f32) -> bool {
        a > b
    }

    #[test]
    fn craft_kits_differ_in_drive_feel() {
        assert!(lt(KIT_HATCH.body_mass_scale, 1.0));
        assert!(gt(KIT_HATCH.drive_factor_scale, 1.0));
        assert!(lt(KIT_SEDAN.drive_factor_scale, 1.0));
        assert!(gt(KIT_SEDAN.grip_scale, 1.0));
        assert!(gt(KIT_TAXI.body_mass_scale, 1.0));
        assert!(lt(KIT_TAXI.grip_scale, 1.0));
        assert_ne!(KIT_HATCH.jump_impulse, KIT_TAXI.jump_impulse);
        assert!(VehicleId::RaceFuture.kit().is_none());
        assert!(VehicleId::Hatchback.kit().is_some());
    }
}
