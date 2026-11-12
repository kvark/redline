//! Race audio cues: engine pitch, boost whooshes, and start-light beeps.
//!
//! This module is pure bookkeeping: it watches read-only race state and turns
//! edges into [`Cue`]s. Synthesis and the output device live in `audio_out`
//! (native only), on their own thread, so audio can never stall or steer the
//! fixed-step sim. On WASM, or with no device, everything here stays silent.

/// Engine note at a standstill (Hz).
pub const ENGINE_IDLE_HZ: f32 = 55.0;
/// Engine note at [`ENGINE_FULL_SPEED`] and above (Hz).
pub const ENGINE_TOP_HZ: f32 = 230.0;
/// Speed (m/s) where the engine note tops out.
pub const ENGINE_FULL_SPEED: f32 = 36.0;
/// Extra pitch fraction at full boost (the "afterburner" lift).
pub const ENGINE_BOOST_LIFT: f32 = 0.12;
/// Rejoin boost must rise past this to count as a fresh boost.
const REJOIN_EDGE: f32 = 0.05;

/// Engine fundamental (Hz) for a craft speed (m/s) and boost intensity 0..1.
/// Rises monotonically with speed, saturates at top speed, never below idle.
pub fn engine_pitch(speed: f32, boost: f32) -> f32 {
    let s = (speed.abs() / ENGINE_FULL_SPEED).clamp(0.0, 1.0);
    // Gentle ease-out so the low gears climb quickly, like a real rev-up.
    let shaped = 1.0 - (1.0 - s) * (1.0 - s);
    let base = ENGINE_IDLE_HZ + (ENGINE_TOP_HZ - ENGINE_IDLE_HZ) * shaped;
    base * (1.0 + ENGINE_BOOST_LIFT * boost.clamp(0.0, 1.0))
}

/// Engine loudness 0..1: an idle murmur that grows with throttle and speed.
pub fn engine_gain(speed: f32, throttle: f32) -> f32 {
    let s = (speed.abs() / ENGINE_FULL_SPEED).clamp(0.0, 1.0);
    (0.18 + 0.22 * throttle.abs().clamp(0.0, 1.0) + 0.2 * s).min(0.6)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Whoosh {
    /// Driving over a boost pad.
    Pad,
    /// PERFECT or GOOD launch off the grid.
    Launch,
    /// Scrub → rejoin boost when the craft gets back on the ribbon.
    Rejoin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cue {
    /// One of the 3 / 2 / 1 start lights.
    CountBeep,
    /// Lights out: a higher, longer two-tone chord.
    Go,
    Whoosh(Whoosh),
}

/// Messages from the game to the audio thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Engine { hz: f32, gain: f32 },
    Play(Cue),
}

/// Which cue (if any) a countdown label change should fire.
pub fn countdown_cue(prev: Option<&str>, now: Option<&str>) -> Option<Cue> {
    if prev == now {
        return None;
    }
    match now {
        Some("GO") => Some(Cue::Go),
        Some(_) => Some(Cue::CountBeep),
        None => None,
    }
}

/// Should this launch rating get a whoosh? Only boosts do; a jump start bogs.
pub fn launch_whooshes(rating: crate::launch::Rating) -> bool {
    matches!(
        rating,
        crate::launch::Rating::Perfect | crate::launch::Rating::Good
    )
}

/// Read-only snapshot of the player's race state, taken once per sim step.
#[derive(Clone, Copy, Debug, Default)]
pub struct Heard {
    pub countdown: Option<&'static str>,
    pub pad_hits: u32,
    pub launch: Option<crate::launch::Rating>,
    pub rejoin_boost: f32,
}

/// Edge detector that turns successive [`Heard`] snapshots into cues, and keeps
/// a per-kind tally for the end-of-race log.
#[derive(Clone, Debug, Default)]
pub struct CueTracker {
    last: Heard,
    counts: [u32; 5],
}

impl CueTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fresh race: forget edges and tallies.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    pub fn observe(&mut self, now: &Heard, mut emit: impl FnMut(Cue)) {
        let mut fire = |cue: Cue, counts: &mut [u32; 5]| {
            counts[slot(cue)] += 1;
            emit(cue);
        };
        if let Some(cue) = countdown_cue(self.last.countdown, now.countdown) {
            fire(cue, &mut self.counts);
        }
        // Hit counts only grow within a race; a respawn resets to 0 silently.
        if now.pad_hits > self.last.pad_hits {
            fire(Cue::Whoosh(Whoosh::Pad), &mut self.counts);
        }
        if let Some(rating) = now.launch
            && self.last.launch.is_none()
            && launch_whooshes(rating)
        {
            fire(Cue::Whoosh(Whoosh::Launch), &mut self.counts);
        }
        if now.rejoin_boost > REJOIN_EDGE && self.last.rejoin_boost <= REJOIN_EDGE {
            fire(Cue::Whoosh(Whoosh::Rejoin), &mut self.counts);
        }
        self.last = *now;
    }

    #[cfg(test)]
    pub fn count(&self, cue: Cue) -> u32 {
        self.counts[slot(cue)]
    }

    /// One-line tally of fired cues for the end-of-race log.
    pub fn summary(&self) -> String {
        let [beep, go, pad, launch, rejoin] = self.counts;
        format!(
            "Audio cues: beeps={beep} go={go} whoosh_pad={pad} whoosh_launch={launch} whoosh_rejoin={rejoin}"
        )
    }
}

fn slot(cue: Cue) -> usize {
    match cue {
        Cue::CountBeep => 0,
        Cue::Go => 1,
        Cue::Whoosh(Whoosh::Pad) => 2,
        Cue::Whoosh(Whoosh::Launch) => 3,
        Cue::Whoosh(Whoosh::Rejoin) => 4,
    }
}

/// WASM has no native output yet: a silent sink with the same API.
#[cfg(target_arch = "wasm32")]
pub struct Output;

#[cfg(target_arch = "wasm32")]
impl Output {
    pub fn start() -> Self {
        Self
    }

    pub fn send(&self, _command: Command) {}
}

#[cfg(not(target_arch = "wasm32"))]
pub use crate::audio_out::Output;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::countdown::{BEAT_SECS, GO_HOLD_SECS, StartCountdown};
    use crate::launch::Rating;

    #[test]
    fn pitch_rises_with_speed_and_saturates() {
        assert_eq!(engine_pitch(0.0, 0.0), ENGINE_IDLE_HZ);
        let mut prev = engine_pitch(0.0, 0.0);
        for i in 1..=40 {
            let hz = engine_pitch(i as f32, 0.0);
            assert!(hz > prev || hz == ENGINE_TOP_HZ, "{i} m/s: {hz} <= {prev}");
            prev = hz;
        }
        assert!((engine_pitch(ENGINE_FULL_SPEED, 0.0) - ENGINE_TOP_HZ).abs() < 1e-3);
        assert_eq!(engine_pitch(80.0, 0.0), ENGINE_TOP_HZ);
        // Reversing revs the same as going forward.
        assert_eq!(engine_pitch(-10.0, 0.0), engine_pitch(10.0, 0.0));
    }

    #[test]
    fn boost_lifts_pitch_and_gain_is_bounded() {
        let base = engine_pitch(20.0, 0.0);
        let boosted = engine_pitch(20.0, 1.0);
        assert!((boosted / base - (1.0 + ENGINE_BOOST_LIFT)).abs() < 1e-4);
        assert_eq!(engine_pitch(20.0, 5.0), boosted);
        assert!(engine_gain(0.0, 0.0) > 0.0);
        assert!(engine_gain(30.0, 1.0) > engine_gain(0.0, 0.0));
        assert!(engine_gain(1e3, 1.0) <= 0.6);
    }

    /// Drive a real countdown at the fixed physics step and log the cues.
    #[test]
    fn countdown_beeps_three_times_then_distinct_go() {
        let mut lights = StartCountdown::new();
        let mut tracker = CueTracker::new();
        let mut log = Vec::new();
        let dt = 0.01;
        let mut t = 0.0;
        loop {
            let heard = Heard {
                countdown: lights.label(),
                ..Heard::default()
            };
            tracker.observe(&heard, |cue| log.push((t, cue)));
            if lights.tick(dt) {
                break;
            }
            t += dt;
        }
        let cues: Vec<_> = log.iter().map(|&(_, c)| c).collect();
        assert_eq!(
            cues,
            [Cue::CountBeep, Cue::CountBeep, Cue::CountBeep, Cue::Go]
        );
        assert_ne!(Cue::CountBeep, Cue::Go);
        // One beep per beat, GO exactly when the lights go out.
        for (i, &(at, _)) in log.iter().enumerate() {
            assert!((at - BEAT_SECS * i as f32).abs() < 0.02, "cue {i} at {at}");
        }
        assert!(t >= BEAT_SECS * 3.0 + GO_HOLD_SECS - 0.02);
        assert_eq!(tracker.count(Cue::Go), 1);
        // Once the lights are gone, nothing more fires.
        tracker.observe(&Heard::default(), |cue| panic!("stray {cue:?}"));
    }

    #[test]
    fn whoosh_triggers() {
        let mut tracker = CueTracker::new();
        let mut log = Vec::new();
        let mut step = |h: Heard, log: &mut Vec<Cue>| tracker.observe(&h, |c| log.push(c));

        // Pad hit: once per new hit, not every step on the pad.
        step(
            Heard {
                pad_hits: 1,
                ..Heard::default()
            },
            &mut log,
        );
        step(
            Heard {
                pad_hits: 1,
                ..Heard::default()
            },
            &mut log,
        );
        assert_eq!(log, [Cue::Whoosh(Whoosh::Pad)]);
        // Respawn resets the count: silent, then the next hit whooshes again.
        step(Heard::default(), &mut log);
        step(
            Heard {
                pad_hits: 1,
                ..Heard::default()
            },
            &mut log,
        );
        assert_eq!(log.len(), 2);

        // Launch: PERFECT and GOOD whoosh once; JUMP and LATE stay quiet.
        for (rating, expect) in [
            (Rating::Perfect, true),
            (Rating::Good, true),
            (Rating::Jump, false),
            (Rating::Late, false),
        ] {
            assert_eq!(launch_whooshes(rating), expect);
            let mut t = CueTracker::new();
            let mut got = Vec::new();
            for _ in 0..3 {
                let h = Heard {
                    launch: Some(rating),
                    ..Heard::default()
                };
                t.observe(&h, |c| got.push(c));
            }
            let want: &[Cue] = if expect {
                &[Cue::Whoosh(Whoosh::Launch)]
            } else {
                &[]
            };
            assert_eq!(got, want, "{rating:?}");
        }

        // Rejoin: on the rising edge only, re-armed after the boost fades.
        let mut log = Vec::new();
        let mut t = CueTracker::new();
        for boost in [0.0, 0.6, 0.4, 0.1, 0.0, 0.8] {
            let h = Heard {
                rejoin_boost: boost,
                ..Heard::default()
            };
            t.observe(&h, |c| log.push(c));
        }
        assert_eq!(log, [Cue::Whoosh(Whoosh::Rejoin); 2]);
        assert!(t.summary().contains("whoosh_rejoin=2"));
    }
}
