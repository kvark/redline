//! Rust-red plumes kicked up when a craft runs the ribbon edge or scrubs.
//! The GPU pool is fixed (`POOL`) and a CPU ledger refuses bursts that would
//! overflow it, so the particle count is hard capped regardless of traffic.
//! Emission is gated on the fixed-step counter, never on wall clock or RNG.

use std::collections::VecDeque;

/// Particles the edge-dust system can ever hold.
pub const POOL: u32 = 768;
/// Upper particle lifetime (s); the ledger frees a burst only after this.
pub const LIFE_MAX: f32 = 1.5;
/// Each craft may emit on one fixed step in this many.
const STEP_GATE: u32 = 3;
/// Within this many metres of the edge a craft starts kicking dust; the
/// plume is full strength from `EDGE_FULL` inside the edge outward.
const EDGE_BAND: f32 = 2.0;
const EDGE_FULL: f32 = 0.4;

pub fn effect() -> blade_particle::ParticleEffect {
    blade_particle::ParticleEffect {
        capacity: POOL,
        emitter: blade_particle::Emitter {
            rate: 0.0,
            burst_count: 0,
            shape: blade_particle::EmitterShape::Sphere { radius: 0.55 },
            cone_angle: 1.05,
        },
        particle: blade_particle::ParticleConfig {
            life: [0.7, LIFE_MAX],
            speed: [0.8, 3.2],
            scale: [0.22, 0.62],
            color: blade_particle::ColorConfig::Palette(vec![
                [168, 74, 40, 170],
                [192, 98, 54, 150],
                [138, 58, 34, 180],
                [206, 132, 86, 120],
            ]),
        },
    }
}

/// Burst size for one craft this step: 0 unless it is near/over the edge or
/// scrubbing sideways at speed.
pub fn plume(off_track: f32, lateral_speed: f32, speed: f32) -> u32 {
    if speed < 5.0 {
        return 0;
    }
    let edge = ((off_track + EDGE_BAND) / (EDGE_BAND - EDGE_FULL)).clamp(0.0, 1.0);
    let scrub = ((lateral_speed.abs() - 2.5) / 4.0).clamp(0.0, 1.0);
    let pace = (speed / 18.0).clamp(0.4, 1.0);
    (edge.max(scrub) * pace * 9.0).round() as u32
}

/// Whether craft `id` may emit on fixed step `step` (staggers the field).
pub fn gate(step: u32, id: usize) -> bool {
    (step + id as u32).is_multiple_of(STEP_GATE)
}

/// CPU ledger of live particles: bursts expire `LIFE_MAX` after emission.
#[derive(Default)]
pub struct Ledger {
    live: VecDeque<(f32, u32)>,
    total: u32,
    peak: u32,
    emitted: u64,
    refused: u64,
}

impl Ledger {
    /// Grant up to `want` particles at sim time `now` without exceeding `POOL`.
    pub fn reserve(&mut self, now: f32, want: u32) -> u32 {
        while let Some(&(expiry, count)) = self.live.front() {
            if expiry > now {
                break;
            }
            self.total -= count;
            self.live.pop_front();
        }
        let granted = want.min(POOL - self.total);
        self.refused += u64::from(want - granted);
        if granted > 0 {
            self.total += granted;
            self.emitted += u64::from(granted);
            self.live.push_back((now + LIFE_MAX, granted));
            self.peak = self.peak.max(self.total);
        }
        granted
    }

    #[cfg(test)]
    fn live(&self) -> u32 {
        self.total
    }

    pub fn summary(&self) -> String {
        format!(
            "Edge dust: peak_live={} pool={} emitted={} refused={}",
            self.peak, POOL, self.emitted, self.refused
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plume_needs_edge_or_scrub_and_speed() {
        assert_eq!(plume(-3.0, 0.0, 20.0), 0, "mid-ribbon stays clean");
        assert_eq!(plume(0.5, 0.0, 3.0), 0, "crawling kicks nothing");
        assert!(plume(-0.2, 0.0, 20.0) > 0, "edge kicks dust");
        assert!(plume(-3.0, 6.0, 20.0) > 0, "scrub kicks dust");
        assert!(plume(1.0, 0.0, 20.0) >= plume(-0.6, 0.0, 20.0));
    }

    #[test]
    fn gate_staggers_cars() {
        let fires = (0..30).filter(|&s| gate(s, 0)).count();
        assert_eq!(fires, 10);
        assert!((0..3).any(|s| gate(s, 1) && !gate(s, 0)));
    }

    #[test]
    fn ledger_hard_caps_and_recycles() {
        let mut ledger = Ledger::default();
        let mut t = 0.0;
        for _ in 0..1_000 {
            ledger.reserve(t, 40);
            assert!(ledger.live() <= POOL);
            t += 0.01;
        }
        assert_eq!(ledger.peak, POOL);
        assert!(ledger.refused > 0);
        // Everything expires once LIFE_MAX passes with no new bursts.
        assert_eq!(ledger.reserve(t + LIFE_MAX + 0.1, 0), 0);
        assert_eq!(ledger.live(), 0);
    }
}
