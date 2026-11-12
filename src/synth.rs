//! Procedural race synth shared by every audio backend.
//!
//! Native (`audio_out`) streams [`Synth::sample`] to the device; the web
//! backend (`audio_web`) plays the same voices as pre-rendered buffers and the
//! engine as an oscillator built from [`ENGINE_PARTIALS`], so both sound alike.

#[cfg(any(not(target_arch = "wasm32"), test))]
use crate::audio::Command;
use crate::audio::{Cue, Whoosh};
use std::f32::consts::TAU;

/// Most overlapping one-shot voices; extra cues are dropped, never queued.
pub const MAX_VOICES: usize = 8;
pub const MASTER_GAIN: f32 = 0.35;
/// Engine level per unit of commanded gain, before [`MASTER_GAIN`].
pub const ENGINE_LEVEL: f32 = 0.3;
/// Engine one-pole glide rate (1/s): ~25 ms, so per-frame updates never click.
pub const ENGINE_GLIDE: f32 = 40.0;
/// Buzzy hover-turbine: (multiple of the engine note, amplitude). The fundamental,
/// two harmonics, and a half-rate rumble.
pub const ENGINE_PARTIALS: [(f32, f32); 4] = [(1.0, 1.0), (2.0, 0.5), (3.0, 0.3), (0.5, 0.4)];

#[derive(Clone, Copy, Debug)]
struct Voice {
    cue: Cue,
    t: f32,
    len: f32,
    phase: f32,
    noise_lp: f32,
}

/// xorshift32: deterministic, allocation-free white noise.
#[derive(Clone, Copy, Debug)]
struct Noise(u32);

impl Noise {
    const SEED: Self = Self(0x9e37_79b9);

    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

/// Mono procedural mixer: one engine oscillator plus a few one-shot voices.
/// The web backend renders the same voices but runs its engine on an oscillator.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub struct Synth {
    dt: f32,
    engine_phase: f32,
    engine_hz: f32,
    engine_gain: f32,
    target_hz: f32,
    target_gain: f32,
    voices: Vec<Voice>,
    noise: Noise,
}

#[cfg(any(not(target_arch = "wasm32"), test))]
impl Synth {
    pub fn new(sample_rate: f32) -> Self {
        Self {
            dt: 1.0 / sample_rate.max(1.0),
            engine_phase: 0.0,
            engine_hz: crate::audio::ENGINE_IDLE_HZ,
            engine_gain: 0.0,
            target_hz: crate::audio::ENGINE_IDLE_HZ,
            target_gain: 0.0,
            voices: Vec::with_capacity(MAX_VOICES),
            noise: Noise::SEED,
        }
    }

    pub fn command(&mut self, command: Command) {
        match command {
            Command::Engine { hz, gain } => {
                self.target_hz = hz.clamp(20.0, 2000.0);
                self.target_gain = gain.clamp(0.0, 1.0);
            }
            Command::Play(cue) if self.voices.len() < MAX_VOICES => self.voices.push(Voice {
                cue,
                t: 0.0,
                len: cue_length(cue),
                phase: 0.0,
                noise_lp: 0.0,
            }),
            Command::Play(_) => {}
        }
    }

    #[cfg(test)]
    pub fn active_voices(&self) -> usize {
        self.voices.len()
    }

    pub fn sample(&mut self) -> f32 {
        let glide = (self.dt * ENGINE_GLIDE).min(1.0);
        self.engine_hz += (self.target_hz - self.engine_hz) * glide;
        self.engine_gain += (self.target_gain - self.engine_gain) * glide;
        self.engine_phase = (self.engine_phase + self.engine_hz * self.dt).fract();
        let p = self.engine_phase * TAU;
        let engine: f32 = ENGINE_PARTIALS
            .iter()
            .map(|&(k, a)| a * (k * p).sin())
            .sum();
        let mut mix = ENGINE_LEVEL * self.engine_gain * engine;

        let dt = self.dt;
        for i in 0..self.voices.len() {
            let white = self.noise.next();
            mix += voice_sample(&mut self.voices[i], white, dt);
        }
        self.voices.retain(|v| v.t < v.len);
        (mix * MASTER_GAIN).tanh()
    }
}

pub fn cue_length(cue: Cue) -> f32 {
    match cue {
        Cue::CountBeep => 0.16,
        Cue::Go => 0.55,
        Cue::Whoosh(Whoosh::Pad) => 0.4,
        Cue::Whoosh(Whoosh::Launch) => 0.7,
        Cue::Whoosh(Whoosh::Rejoin) => 0.3,
    }
}

fn voice_sample(v: &mut Voice, white: f32, dt: f32) -> f32 {
    let x = (v.t / v.len).clamp(0.0, 1.0);
    v.t += dt;
    match v.cue {
        Cue::CountBeep | Cue::Go => {
            // 880 Hz blip for the lights; GO is a brighter 1320+1760 Hz chord.
            let (hz, amp) = if v.cue == Cue::Go {
                (1320.0, 0.8)
            } else {
                (880.0, 0.7)
            };
            v.phase = (v.phase + hz * dt).fract();
            let p = v.phase * TAU;
            let tone = if v.cue == Cue::Go {
                p.sin() + 0.6 * (p * 4.0 / 3.0).sin()
            } else {
                p.sin() + 0.25 * (3.0 * p).sin()
            };
            let attack = (v.t / 0.005).min(1.0);
            amp * attack * (1.0 - x).powi(2) * tone
        }
        Cue::Whoosh(kind) => {
            // Noise through a one-pole lowpass whose cutoff sweeps up then down.
            let amp = match kind {
                Whoosh::Pad => 0.9,
                Whoosh::Launch => 1.1,
                Whoosh::Rejoin => 0.6,
            };
            let hump = (x * std::f32::consts::PI).sin();
            let cutoff = 300.0 + 4500.0 * hump;
            let k = (TAU * cutoff * dt).min(1.0);
            v.noise_lp += (white - v.noise_lp) * k;
            amp * hump * v.noise_lp * 2.0
        }
    }
}

/// One cue rendered alone, pre-[`MASTER_GAIN`]-saturation: the exact samples
/// the native synth would mix for it (whoosh noise from the same seed).
#[cfg(any(target_arch = "wasm32", test))]
pub fn render_cue(cue: Cue, sample_rate: f32) -> Vec<f32> {
    let dt = 1.0 / sample_rate.max(1.0);
    let mut voice = Voice {
        cue,
        t: 0.0,
        len: cue_length(cue),
        phase: 0.0,
        noise_lp: 0.0,
    };
    let mut noise = Noise::SEED;
    let mut out = Vec::with_capacity((voice.len * sample_rate) as usize + 1);
    while voice.t < voice.len {
        let white = noise.next();
        out.push(MASTER_GAIN * voice_sample(&mut voice, white, dt));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(synth: &mut Synth, samples: usize) -> f32 {
        (0..samples).map(|_| synth.sample().powi(2)).sum::<f32>() / samples as f32
    }

    #[test]
    fn silent_until_told_and_cues_decay() {
        let mut synth = Synth::new(48_000.0);
        assert_eq!(energy(&mut synth, 4800), 0.0);
        synth.command(Command::Play(Cue::CountBeep));
        assert!(energy(&mut synth, 4800) > 1e-4);
        // Beep is 0.16 s; well after that the voice is gone and output is quiet.
        let _ = energy(&mut synth, 9600);
        assert_eq!(synth.active_voices(), 0);
        assert_eq!(energy(&mut synth, 480), 0.0);
    }

    #[test]
    fn voices_are_capped_and_output_bounded() {
        let mut synth = Synth::new(44_100.0);
        synth.command(Command::Engine {
            hz: 200.0,
            gain: 1.0,
        });
        for _ in 0..(MAX_VOICES * 3) {
            synth.command(Command::Play(Cue::Whoosh(Whoosh::Launch)));
        }
        assert_eq!(synth.active_voices(), MAX_VOICES);
        for _ in 0..44_100 {
            let s = synth.sample();
            assert!(s.is_finite() && s.abs() <= 1.0);
        }
    }

    #[test]
    fn go_is_longer_than_a_beep() {
        assert!(cue_length(Cue::Go) > 2.0 * cue_length(Cue::CountBeep));
    }

    #[test]
    fn rendered_cue_matches_the_streaming_mix() {
        // Beeps carry no noise, so the buffer must equal the live mixer's output
        // once the shared tanh stage is applied.
        for cue in [Cue::CountBeep, Cue::Go] {
            let buf = render_cue(cue, 48_000.0);
            assert!(buf.len().abs_diff((cue_length(cue) * 48_000.0) as usize) <= 1);
            let mut synth = Synth::new(48_000.0);
            synth.command(Command::Play(cue));
            for (i, &s) in buf.iter().enumerate() {
                assert!(
                    (s.tanh() - synth.sample()).abs() < 1e-5,
                    "{cue:?} sample {i}"
                );
            }
        }
        let whoosh = render_cue(Cue::Whoosh(Whoosh::Pad), 44_100.0);
        assert!(whoosh.iter().all(|s| s.is_finite()));
        assert!(whoosh.iter().map(|s| s * s).sum::<f32>() > 1.0);
    }
}
