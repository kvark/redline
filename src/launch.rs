//! Grid launch: throttle timing against GO earns a short thrust, a jump start
//! bogs the craft down in wheelspin. AI racers get a varied reaction time.
//!
//! Times are seconds relative to GO (negative = before the lights go out).

/// A hold that began inside `[PERFECT_EARLY, PERFECT_LATE]` is a perfect launch.
pub const PERFECT_EARLY: f32 = -0.20;
pub const PERFECT_LATE: f32 = 0.10;
/// Outside perfect but inside `[GOOD_EARLY, GOOD_LATE]` is a good launch.
pub const GOOD_EARLY: f32 = -0.45;
pub const GOOD_LATE: f32 = 0.30;
/// Thrust (m/s²) and duration (s) for each rating; a negative accel is the
/// wheelspin drag of a jump start.
const PERFECT_THRUST: (f32, f32) = (9.0, 0.9);
const GOOD_THRUST: (f32, f32) = (5.0, 0.7);
const JUMP_BOG: (f32, f32) = (-4.0, 0.8);
/// How long the rating banner stays up after it resolves.
pub const BANNER_SECS: f32 = 1.6;
/// The scripted driver squeezes the throttle this long before GO.
pub const SCRIPT_LEAD: f32 = 0.15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rating {
    Perfect,
    Good,
    /// Throttle held before the lights went out: wheelspin.
    Jump,
    /// No throttle in time: no bonus, no penalty.
    Late,
}

impl Rating {
    pub fn label(self) -> &'static str {
        match self {
            Self::Perfect => "PERFECT LAUNCH",
            Self::Good => "GOOD LAUNCH",
            Self::Jump => "JUMP START · WHEELSPIN",
            Self::Late => "LATE LAUNCH",
        }
    }

    /// (accel m/s², seconds) applied along the craft's heading after GO.
    pub fn effect(self) -> (f32, f32) {
        match self {
            Self::Perfect => PERFECT_THRUST,
            Self::Good => GOOD_THRUST,
            Self::Jump => JUMP_BOG,
            Self::Late => (0.0, 0.0),
        }
    }
}

/// Rate a throttle hold that began `start` seconds from GO.
pub fn rate(start: f32) -> Rating {
    if start < GOOD_EARLY {
        Rating::Jump
    } else if start < PERFECT_EARLY {
        Rating::Good
    } else if start <= PERFECT_LATE {
        Rating::Perfect
    } else if start <= GOOD_LATE {
        Rating::Good
    } else {
        Rating::Late
    }
}

/// Watches one driver's throttle through the lights and resolves a rating.
#[derive(Clone, Debug, Default)]
pub struct Tracker {
    hold_start: Option<f32>,
    resolved: Option<(Rating, Option<f32>)>,
}

impl Tracker {
    /// Feed the throttle state at time `t` from GO. Returns the rating (and
    /// the hold start, if any) on the step it resolves.
    pub fn observe(&mut self, t: f32, held: bool) -> Option<(Rating, Option<f32>)> {
        if self.resolved.is_some() {
            return None;
        }
        if !held {
            self.hold_start = None;
        } else if self.hold_start.is_none() {
            self.hold_start = Some(t);
        }
        let out = match self.hold_start {
            Some(start) if t >= 0.0 => (rate(start), Some(start)),
            None if t > GOOD_LATE => (Rating::Late, None),
            _ => return None,
        };
        self.resolved = Some(out);
        Some(out)
    }
}

/// Deterministic AI reaction (s from GO) for racer `id` in race `seed`:
/// mostly decent launches, with the odd jump start or sleepy one.
pub fn ai_reaction(id: usize, seed: u32) -> f32 {
    let mut x = (id as u32).wrapping_mul(0x9E37_79B9) ^ seed.wrapping_mul(0x85EB_CA6B);
    x = (x ^ (x >> 15)).wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 13;
    let u = (x >> 8) as f32 / 16_777_216.0;
    GOOD_EARLY - 0.12 + u * (GOOD_LATE + 0.22 - GOOD_EARLY)
}

#[derive(Clone, Copy, Debug)]
struct Effect {
    accel: f32,
    duration: f32,
    left: f32,
}

/// The whole grid's launch: player tracker, AI reactions, live effects.
/// Index 0 is the player, AI racers follow in spawn order.
#[derive(Clone, Debug)]
pub struct Start {
    tracker: Tracker,
    reactions: Vec<f32>,
    pub ratings: Vec<Option<Rating>>,
    /// When the player's throttle hold began (s from GO), if it counted.
    pub player_start: Option<f32>,
    /// Estimated time gained (+) or lost (−) by the player's launch.
    pub player_gain: Option<f32>,
    effects: Vec<Option<Effect>>,
    banner_left: f32,
}

impl Start {
    pub fn new(ai_count: usize, seed: u32) -> Self {
        Self {
            tracker: Tracker::default(),
            reactions: (1..=ai_count).map(|id| ai_reaction(id, seed)).collect(),
            ratings: vec![None; ai_count + 1],
            player_start: None,
            player_gain: None,
            effects: vec![None; ai_count + 1],
            banner_left: 0.0,
        }
    }

    /// Resolve launches at time `t` from GO given the player's throttle.
    pub fn observe(&mut self, t: f32, player_held: bool) {
        if let Some((rating, start)) = self.tracker.observe(t, player_held) {
            self.player_start = start;
            self.banner_left = BANNER_SECS;
            self.begin(0, rating);
        }
        for i in 0..self.reactions.len() {
            let reaction = self.reactions[i];
            if self.ratings[i + 1].is_none() && t >= reaction.max(0.0) {
                self.begin(i + 1, rate(reaction));
            }
        }
    }

    fn begin(&mut self, racer: usize, rating: Rating) {
        self.ratings[racer] = Some(rating);
        let (accel, duration) = rating.effect();
        if duration > 0.0 {
            self.effects[racer] = Some(Effect {
                accel,
                duration,
                left: duration,
            });
        } else if racer == 0 {
            self.player_gain = Some(0.0);
        }
    }

    /// Advance live effects; returns the forward accel (m/s²) per racer this
    /// step. `player_speed` sizes the time-gain estimate when the effect ends.
    pub fn tick(&mut self, dt: f32, player_speed: f32) -> Vec<f32> {
        self.banner_left = (self.banner_left - dt).max(0.0);
        let mut out = vec![0.0; self.effects.len()];
        for (racer, slot) in self.effects.iter_mut().enumerate() {
            let Some(effect) = slot.as_mut() else {
                continue;
            };
            out[racer] = effect.accel;
            effect.left -= dt;
            if effect.left <= 0.0 {
                if racer == 0 {
                    self.player_gain =
                        Some(estimate_gain(effect.accel, effect.duration, player_speed));
                }
                *slot = None;
            }
        }
        out
    }

    /// Player launch thrust left, 0..1 (for camera and HUD FX; 0 on a bog).
    pub fn player_thrust(&self) -> f32 {
        match self.effects[0] {
            Some(e) if e.accel > 0.0 => (e.left / e.duration).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    /// The player's rating while its banner should still be visible.
    pub fn banner(&self) -> Option<Rating> {
        self.ratings[0].filter(|_| self.banner_left > 0.0)
    }
}

/// Seconds gained by `accel` over `duration`: the extra distance at the end
/// of the push (a·T²/2) over the speed the craft carries out of it.
pub fn estimate_gain(accel: f32, duration: f32, speed: f32) -> f32 {
    accel * duration * duration * 0.5 / speed.max(4.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_windows() {
        assert_eq!(rate(-0.8), Rating::Jump);
        assert_eq!(rate(GOOD_EARLY - 0.01), Rating::Jump);
        assert_eq!(rate(GOOD_EARLY), Rating::Good);
        assert_eq!(rate(-0.3), Rating::Good);
        assert_eq!(rate(PERFECT_EARLY), Rating::Perfect);
        assert_eq!(rate(-SCRIPT_LEAD), Rating::Perfect);
        assert_eq!(rate(0.0), Rating::Perfect);
        assert_eq!(rate(PERFECT_LATE), Rating::Perfect);
        assert_eq!(rate(0.2), Rating::Good);
        assert_eq!(rate(GOOD_LATE), Rating::Good);
        assert_eq!(rate(GOOD_LATE + 0.01), Rating::Late);
    }

    fn run(press: impl Fn(f32) -> bool) -> (Rating, Option<f32>) {
        let mut tracker = Tracker::default();
        let mut t = -2.55;
        while t < 1.0 {
            if let Some(out) = tracker.observe(t, press(t)) {
                return out;
            }
            t += 0.01;
        }
        panic!("launch never resolved");
    }

    #[test]
    fn tracker_resolves_holds_and_releases() {
        // Held since "2": jump start, resolved at GO.
        assert_eq!(run(|t| t > -1.5).0, Rating::Jump);
        // Feathered early, then re-pressed just before GO: the last hold counts.
        let (rating, start) = run(|t| (-1.5..-1.2).contains(&t) || t > -0.12);
        assert_eq!(rating, Rating::Perfect);
        assert!((start.unwrap() + 0.12).abs() < 0.02);
        // Pressed just after GO.
        assert_eq!(run(|t| t > 0.2).0, Rating::Good);
        // Never pressed: late, with no hold start.
        assert_eq!(run(|_| false), (Rating::Late, None));
        // Released before GO and not re-pressed in time.
        assert_eq!(run(|t| (-0.3..-0.1).contains(&t)).0, Rating::Late);
    }

    #[test]
    fn ai_reactions_are_varied_and_deterministic() {
        let ratings: Vec<_> = (1..=12).map(|id| rate(ai_reaction(id, 0))).collect();
        assert_eq!(ai_reaction(2, 7), ai_reaction(2, 7));
        assert_ne!(ai_reaction(2, 7), ai_reaction(2, 8));
        assert!(ratings.contains(&Rating::Perfect));
        assert!(ratings.contains(&Rating::Good));
        assert!(
            ratings
                .iter()
                .any(|r| matches!(r, Rating::Jump | Rating::Late))
        );
    }

    #[test]
    fn start_applies_thrust_then_bog_and_estimates_gain() {
        let mut start = Start::new(0, 0);
        start.observe(-0.6, false);
        start.observe(-SCRIPT_LEAD, true);
        assert_eq!(start.ratings[0], None);
        start.observe(0.0, true);
        assert_eq!(start.ratings[0], Some(Rating::Perfect));
        assert_eq!(start.banner(), Some(Rating::Perfect));
        let first = start.tick(0.01, 2.0);
        assert_eq!(first[0], PERFECT_THRUST.0);
        assert!(start.player_thrust() > 0.9);
        for _ in 0..100 {
            start.tick(0.01, 12.0);
        }
        assert_eq!(start.player_thrust(), 0.0);
        let gain = start.player_gain.unwrap();
        assert!((gain - estimate_gain(9.0, 0.9, 12.0)).abs() < 1e-5 && gain > 0.0);
        assert_eq!(start.banner(), Some(Rating::Perfect));
        for _ in 0..70 {
            start.tick(0.01, 12.0);
        }
        assert_eq!(start.banner(), None);

        let mut jumped = Start::new(0, 0);
        jumped.observe(-1.0, true);
        jumped.observe(0.0, true);
        assert_eq!(jumped.ratings[0], Some(Rating::Jump));
        assert!(jumped.tick(0.01, 0.0)[0] < 0.0);
        assert_eq!(jumped.player_thrust(), 0.0);
    }
}
