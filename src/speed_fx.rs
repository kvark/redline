//! Speed feedback: chase-camera FOV kick and screen-space speed lines.
//!
//! Everything here is a pure function of speed / boost / time so it is cheap,
//! deterministic, and identical on native and WASM.

/// Speed (m/s) where the FOV starts to open up.
pub const FOV_SPEED_START: f32 = 12.0;
/// Speed (m/s) at which the speed share of the kick is fully applied.
pub const FOV_SPEED_FULL: f32 = 30.0;
/// Extra vertical FOV (radians) at full speed.
pub const FOV_SPEED_KICK: f32 = 0.09;
/// Extra vertical FOV (radians) at full boost, on top of the speed share.
pub const FOV_BOOST_KICK: f32 = 0.13;
/// Rate (1/s) the kick opens toward its target, and the slower ease back.
const FOV_ATTACK: f32 = 7.0;
const FOV_RELEASE: f32 = 2.2;

fn speed_share(speed: f32) -> f32 {
    ((speed - FOV_SPEED_START) / (FOV_SPEED_FULL - FOV_SPEED_START)).clamp(0.0, 1.0)
}

/// Target FOV kick (radians) for the current speed and boost intensity.
pub fn fov_kick_target(speed: f32, boost: f32) -> f32 {
    let s = speed_share(speed);
    FOV_SPEED_KICK * s * s + FOV_BOOST_KICK * boost.clamp(0.0, 1.0)
}

/// Ease `current` toward `target`: snappy open, gentle settle back.
pub fn ease_kick(current: f32, target: f32, dt: f32) -> f32 {
    let rate = if target > current {
        FOV_ATTACK
    } else {
        FOV_RELEASE
    };
    let t = 1.0 - (-dt.max(0.0) * rate).exp();
    current + (target - current) * t
}

/// Speed-line strength in [0, 1]: faint at pace, full while boosting.
pub fn line_intensity(speed: f32, boost: f32) -> f32 {
    (0.35 * speed_share(speed) + boost.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// One radial streak in normalized screen space: `angle` around the focus,
/// `inner`/`outer` as fractions of the focus-to-corner radius.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Streak {
    pub angle: f32,
    pub inner: f32,
    pub outer: f32,
    pub alpha: f32,
}

pub const MAX_STREAKS: usize = 48;

/// Streaks for this frame. `phase` is a monotonically increasing time (s);
/// each streak flies outward and respawns, so the field animates cheaply.
pub fn streaks(intensity: f32, phase: f32) -> Vec<Streak> {
    let intensity = intensity.clamp(0.0, 1.0);
    if intensity < 0.02 {
        return Vec::new();
    }
    let count = (10.0 + (MAX_STREAKS as f32 - 10.0) * intensity) as usize;
    (0..count.min(MAX_STREAKS))
        .map(|i| {
            let h = hash(i as u32);
            let angle = (h & 0xFFFF) as f32 / 65_535.0 * std::f32::consts::TAU;
            let speed = 1.4 + ((h >> 16) & 0xFF) as f32 / 255.0 * 1.6;
            let offset = ((h >> 24) & 0xFF) as f32 / 255.0;
            let life = (phase * speed + offset).fract();
            // Keep the middle of the screen (the craft) clean.
            let inner = 0.45 + 0.5 * life;
            let length = 0.06 + 0.22 * intensity;
            let fade = (1.0 - life) * life * 4.0;
            Streak {
                angle,
                inner,
                outer: inner + length,
                alpha: (0.55 * intensity * fade).clamp(0.0, 1.0),
            }
        })
        .collect()
}

fn hash(x: u32) -> u32 {
    let mut x = x.wrapping_mul(0x9E37_79B9).wrapping_add(0x7F4A_7C15);
    x = (x ^ (x >> 16)).wrapping_mul(0x85EB_CA6B);
    x = (x ^ (x >> 13)).wrapping_mul(0xC2B2_AE35);
    x ^ (x >> 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fov_kick_grows_with_speed_and_boost() {
        assert_eq!(fov_kick_target(5.0, 0.0), 0.0);
        let cruise = fov_kick_target(22.0, 0.0);
        let flat_out = fov_kick_target(40.0, 0.0);
        assert!(cruise > 0.0 && cruise < flat_out);
        assert!((flat_out - FOV_SPEED_KICK).abs() < 1e-6);
        assert!(fov_kick_target(22.0, 1.0) > flat_out);
    }

    #[test]
    fn kick_opens_fast_and_eases_back_slowly() {
        let up = ease_kick(0.0, 0.2, 0.1);
        let down = 0.2 - ease_kick(0.2, 0.0, 0.1);
        assert!(up > down, "attack {up} should beat release {down}");
        let mut k = 0.2;
        for _ in 0..300 {
            k = ease_kick(k, 0.0, 1.0 / 60.0);
        }
        assert!(k < 1e-3);
    }

    #[test]
    fn streaks_scale_with_intensity_and_stay_off_center() {
        assert!(streaks(0.0, 1.0).is_empty());
        let calm = streaks(0.3, 1.0);
        let boost = streaks(1.0, 1.0);
        assert!(boost.len() > calm.len());
        assert!(boost.len() <= MAX_STREAKS);
        for s in boost.iter() {
            assert!(s.inner >= 0.45 && s.outer > s.inner);
            assert!((0.0..=1.0).contains(&s.alpha));
        }
        assert!(line_intensity(30.0, 0.0) < line_intensity(30.0, 0.8));
    }
}
