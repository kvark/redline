//! Pre-race vehicle and circuit selection.

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
            Self::RaceFuture => "Race Future",
            Self::Hatchback => "Hatchback Sports",
            Self::SedanSports => "Sedan Sports",
            Self::Taxi => "Taxi",
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

pub const KIT_HATCH: vehicle::Kit = vehicle::Kit {
    body_model: "models/hatchback-sports-body.glb",
    wheel_model: "models/wheel-racing.glb",
    tint: [1.0, 0.42, 0.32, 1.0],
    half_track: 0.32,
};

pub const KIT_SEDAN: vehicle::Kit = vehicle::Kit {
    body_model: "models/sedan-sports-body.glb",
    wheel_model: "models/wheel-dark.glb",
    tint: [0.42, 0.72, 1.0, 1.0],
    half_track: 0.32,
};

pub const KIT_TAXI: vehicle::Kit = vehicle::Kit {
    body_model: "models/taxi-body.glb",
    wheel_model: "models/wheel-dark.glb",
    tint: [1.0, 1.0, 1.0, 1.0],
    half_track: 0.32,
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
            Self::MarsClassic => "Mars Classic",
            Self::PhobosTight => "Phobos Tight",
            Self::HellasWide => "Hellas Wide",
            Self::OlympusClimb => "Olympus Climb",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Self::MarsClassic => "The original equatorial ribbon around Mars.",
            Self::PhobosTight => "A smaller moonlet with a narrow racing line.",
            Self::HellasWide => "Broad basins and a forgiving track width.",
            Self::OlympusClimb => "Steep latitude swings and taller terrain.",
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
