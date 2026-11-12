//! Web Audio output: the same race sounds as native, built from browser nodes.
//!
//! The engine is one `OscillatorNode` whose periodic wave holds the native
//! synth's partials, gliding with the native time constant. One-shot cues are
//! the native voices pre-rendered into `AudioBuffer`s. Everything goes through
//! a tanh `WaveShaperNode`, the native master stage. No asset files.
//!
//! Browsers start an `AudioContext` suspended until a user gesture, so we try
//! to resume on load (scripted runs, permissive autoplay) and again on the
//! first click / key / touch. Until then cues are dropped, never queued. If the
//! context cannot be created, resumed, or played, we log once and go silent;
//! nothing here ever blocks the sim or the page. `?mute=1` skips audio.

use crate::audio::{Command, Cue, Whoosh};
use crate::synth::{ENGINE_GLIDE, ENGINE_LEVEL, ENGINE_PARTIALS, MASTER_GAIN, MAX_VOICES};

/// Every cue, for pre-rendering one buffer each.
pub const ALL_CUES: [Cue; 5] = [
    Cue::CountBeep,
    Cue::Go,
    Cue::Whoosh(Whoosh::Pad),
    Cue::Whoosh(Whoosh::Launch),
    Cue::Whoosh(Whoosh::Rejoin),
];
/// The oscillator runs at the lowest partial; every partial is a harmonic of it.
const LOWEST_PARTIAL: f32 = 0.5;
/// Periodic-wave length: harmonics 0..=6 of the oscillator cover all partials.
pub const ENGINE_WAVE_LEN: usize = 7;
/// Engine `setTargetAtTime` time constant (s), the native one-pole glide.
pub const ENGINE_TAU: f64 = 1.0 / ENGINE_GLIDE as f64;
/// The shaper curve spans input ±1, so the bus scales by 1/RANGE and the
/// curve is tanh(RANGE·x): together exactly native's `tanh(mix)` up to ±RANGE.
pub const SHAPER_RANGE: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Suspended,
    Running,
}

/// The few Web Audio calls the player makes; mocked in tests.
pub trait Backend {
    fn now(&self) -> f64;
    /// `Err` once the context is closed or a resume was rejected.
    fn state(&self) -> Result<State, String>;
    fn resume(&mut self) -> Result<(), String>;
    /// Glide the engine oscillator to `osc_hz` and its gain to `level`.
    fn set_engine(&mut self, osc_hz: f32, level: f32) -> Result<(), String>;
    /// Start a one-shot source playing the cue's pre-rendered buffer.
    fn play(&mut self, cue: Cue) -> Result<(), String>;
}

/// Oscillator frequency and gain-node level for an engine command, with the
/// same clamps as the native synth.
pub fn engine_params(hz: f32, gain: f32) -> (f32, f32) {
    let hz = hz.clamp(20.0, 2000.0);
    let level = ENGINE_LEVEL * MASTER_GAIN * gain.clamp(0.0, 1.0);
    (hz * LOWEST_PARTIAL, level)
}

/// Sine (`imag`) terms of the engine's periodic wave, by oscillator harmonic.
pub fn engine_wave() -> [f32; ENGINE_WAVE_LEN] {
    let mut imag = [0.0; ENGINE_WAVE_LEN];
    for &(multiple, amp) in &ENGINE_PARTIALS {
        imag[(multiple / LOWEST_PARTIAL).round() as usize] += amp;
    }
    imag
}

/// `WaveShaperNode` curve over input -1..=1.
pub fn shaper_curve(points: usize) -> Vec<f32> {
    let last = points.max(2) - 1;
    (0..=last)
        .map(|i| (SHAPER_RANGE * (2.0 * i as f32 / last as f32 - 1.0)).tanh())
        .collect()
}

/// `?mute=1` (or a bare `?mute`) silences the web build, like `REDLINE_MUTE=1`.
pub fn muted(search: &str) -> bool {
    search
        .trim_start_matches('?')
        .split('&')
        .filter_map(|pair| {
            let mut kv = pair.splitn(2, '=');
            (kv.next() == Some("mute")).then(|| kv.next().unwrap_or("1"))
        })
        .any(|v| !v.is_empty() && v != "0")
}

/// Command handling shared by the real context and the test mock: drops cues
/// while locked, caps voices like native, and goes silent on the first error.
pub struct Player<B> {
    backend: Option<B>,
    voice_ends: Vec<f64>,
    engine: Option<(f32, f32)>,
}

impl<B: Backend> Player<B> {
    pub fn new(backend: Result<B, String>) -> Self {
        let backend = backend
            .map_err(|err| log::info!("audio: racing silent ({err})"))
            .ok();
        Self {
            backend,
            voice_ends: Vec::with_capacity(MAX_VOICES),
            engine: None,
        }
    }

    pub fn off() -> Self {
        Self {
            backend: None,
            voice_ends: Vec::new(),
            engine: None,
        }
    }

    pub fn is_live(&self) -> bool {
        self.backend.is_some()
    }

    /// Resume a suspended context; called on load and on user gestures.
    pub fn unlock(&mut self) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let result = match backend.state() {
            Ok(State::Suspended) => backend.resume(),
            Ok(State::Running) => Ok(()),
            Err(err) => Err(err),
        };
        self.check(result);
    }

    pub fn send(&mut self, command: Command) {
        let Some(backend) = self.backend.as_mut() else {
            return;
        };
        let result = match (backend.state(), command) {
            (Err(err), _) => Err(err),
            (Ok(_), Command::Engine { hz, gain }) => {
                let params = engine_params(hz, gain);
                if self.engine == Some(params) {
                    Ok(())
                } else {
                    self.engine = Some(params);
                    backend.set_engine(params.0, params.1)
                }
            }
            // Locked by autoplay policy: a late beep would be wrong, so drop it.
            (Ok(State::Suspended), Command::Play(_)) => Ok(()),
            (Ok(State::Running), Command::Play(cue)) => {
                let now = backend.now();
                self.voice_ends.retain(|&end| end > now);
                if self.voice_ends.len() < MAX_VOICES {
                    let len = crate::synth::cue_length(cue);
                    self.voice_ends.push(now + f64::from(len));
                    backend.play(cue)
                } else {
                    Ok(())
                }
            }
        };
        self.check(result);
    }

    fn check(&mut self, result: Result<(), String>) {
        if let Err(err) = result {
            log::info!("audio: racing silent ({err})");
            self.backend = None;
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use dom::Output;

#[cfg(target_arch = "wasm32")]
mod dom {
    use super::*;
    use std::{cell::RefCell, rc::Rc};
    use wasm_bindgen::{JsCast as _, JsValue, closure::Closure};
    use web_sys::{AudioBuffer, AudioContext, AudioContextState, GainNode, OscillatorNode};

    /// Events that count as a user gesture for autoplay unlock.
    const GESTURES: [&str; 4] = ["pointerdown", "keydown", "touchend", "mousedown"];

    /// First line of a JS error (`Debug` would include the whole stack).
    fn js(err: JsValue) -> String {
        let text = err.as_string().unwrap_or_else(|| format!("{err:?}"));
        let text = text.strip_prefix("JsValue(").unwrap_or(&text);
        let line = text.lines().next().unwrap_or_default();
        line.trim_end_matches(')').to_string()
    }

    pub struct WebAudio {
        ctx: AudioContext,
        osc: OscillatorNode,
        engine_gain: GainNode,
        bus: GainNode,
        buffers: Vec<(Cue, AudioBuffer)>,
        rejected: Rc<RefCell<Option<String>>>,
        on_reject: Closure<dyn FnMut(JsValue)>,
    }

    impl WebAudio {
        fn new() -> Result<Self, JsValue> {
            let ctx = AudioContext::new()?;
            let shaper = ctx.create_wave_shaper()?;
            shaper.set_curve_opt_f32_slice(Some(&mut shaper_curve(1025)));
            shaper.connect_with_audio_node(&ctx.destination())?;
            let bus = ctx.create_gain()?;
            bus.gain().set_value(1.0 / SHAPER_RANGE);
            bus.connect_with_audio_node(&shaper)?;

            let constraints = web_sys::PeriodicWaveConstraints::new();
            constraints.set_disable_normalization(true);
            let mut real = [0.0; ENGINE_WAVE_LEN];
            let mut imag = engine_wave();
            let wave =
                ctx.create_periodic_wave_with_constraints(&mut real, &mut imag, &constraints)?;
            let osc = ctx.create_oscillator()?;
            osc.set_periodic_wave(&wave);
            osc.frequency()
                .set_value(engine_params(crate::audio::ENGINE_IDLE_HZ, 0.0).0);
            let engine_gain = ctx.create_gain()?;
            engine_gain.gain().set_value(0.0);
            osc.connect_with_audio_node(&engine_gain)?;
            engine_gain.connect_with_audio_node(&bus)?;
            osc.start()?;

            let rate = ctx.sample_rate();
            let mut buffers = Vec::with_capacity(ALL_CUES.len());
            for cue in ALL_CUES {
                let samples = crate::synth::render_cue(cue, rate);
                let buffer = ctx.create_buffer(1, samples.len() as u32, rate)?;
                buffer.copy_to_channel(&samples, 0)?;
                buffers.push((cue, buffer));
            }
            let rejected = Rc::new(RefCell::new(None));
            let flag = Rc::clone(&rejected);
            let on_reject = Closure::new(move |err: JsValue| {
                *flag.borrow_mut() = Some(format!("resume rejected: {}", js(err)));
            });
            log::info!("audio: Web Audio {rate} Hz, {:?}", ctx.state());
            Ok(Self {
                ctx,
                osc,
                engine_gain,
                bus,
                buffers,
                rejected,
                on_reject,
            })
        }
    }

    impl Drop for WebAudio {
        fn drop(&mut self) {
            let _ = self.ctx.close();
        }
    }

    impl Backend for WebAudio {
        fn now(&self) -> f64 {
            self.ctx.current_time()
        }

        fn state(&self) -> Result<State, String> {
            if let Some(err) = self.rejected.borrow().clone() {
                return Err(err);
            }
            match self.ctx.state() {
                AudioContextState::Running => Ok(State::Running),
                AudioContextState::Suspended => Ok(State::Suspended),
                other => Err(format!("AudioContext {other:?}")),
            }
        }

        fn resume(&mut self) -> Result<(), String> {
            let promise = self.ctx.resume().map_err(js)?;
            let _ = promise.catch(&self.on_reject);
            Ok(())
        }

        fn set_engine(&mut self, osc_hz: f32, level: f32) -> Result<(), String> {
            let now = self.ctx.current_time();
            self.osc
                .frequency()
                .set_target_at_time(osc_hz, now, ENGINE_TAU)
                .map_err(js)?;
            self.engine_gain
                .gain()
                .set_target_at_time(level, now, ENGINE_TAU)
                .map_err(js)?;
            Ok(())
        }

        fn play(&mut self, cue: Cue) -> Result<(), String> {
            let Some(&(_, ref buffer)) = self.buffers.iter().find(|&&(c, _)| c == cue) else {
                return Ok(());
            };
            let source = self.ctx.create_buffer_source().map_err(js)?;
            source.set_buffer(Some(buffer));
            source.connect_with_audio_node(&self.bus).map_err(js)?;
            source.start().map_err(js)
        }
    }

    pub struct Output {
        player: Rc<RefCell<Player<WebAudio>>>,
        unlock: Option<Closure<dyn FnMut()>>,
    }

    impl Output {
        /// Never blocks: creating the context and its buffers is synchronous
        /// and cheap; the resume promise is left to settle on its own.
        pub fn start() -> Self {
            let window = web_sys::window();
            let search = window
                .as_ref()
                .and_then(|w| w.location().search().ok())
                .unwrap_or_default();
            if muted(&search) {
                log::info!("audio: muted by ?mute");
                return Self {
                    player: Rc::new(RefCell::new(Player::off())),
                    unlock: None,
                };
            }
            let player = Rc::new(RefCell::new(Player::new(WebAudio::new().map_err(js))));
            player.borrow_mut().unlock();
            let unlock = match window {
                Some(window) if player.borrow().is_live() => {
                    let shared = Rc::clone(&player);
                    let cb = Closure::<dyn FnMut()>::new(move || {
                        if let Ok(mut player) = shared.try_borrow_mut() {
                            player.unlock();
                        }
                    });
                    for event in GESTURES {
                        let _ = window.add_event_listener_with_callback_and_bool(
                            event,
                            cb.as_ref().unchecked_ref(),
                            true,
                        );
                    }
                    Some(cb)
                }
                _ => None,
            };
            Self { player, unlock }
        }

        /// Fire-and-forget; a dead or locked context just drops it.
        pub fn send(&self, command: Command) {
            if let Ok(mut player) = self.player.try_borrow_mut() {
                player.send(command);
            }
        }
    }

    impl Drop for Output {
        fn drop(&mut self) {
            if let (Some(cb), Some(window)) = (self.unlock.take(), web_sys::window()) {
                for event in GESTURES {
                    let _ = window.remove_event_listener_with_callback_and_bool(
                        event,
                        cb.as_ref().unchecked_ref(),
                        true,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::{CueTracker, Heard};
    use crate::countdown::StartCountdown;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Call {
        Resume,
        Engine(f32, f32),
        Play(Cue),
    }

    struct Mock {
        state: Result<State, String>,
        now: f64,
        fail_play: bool,
        fail_resume: bool,
        calls: Vec<Call>,
    }

    #[derive(Clone)]
    struct Shared(Rc<RefCell<Mock>>);

    impl Shared {
        fn new(state: State) -> Self {
            Self(Rc::new(RefCell::new(Mock {
                state: Ok(state),
                now: 0.0,
                fail_play: false,
                fail_resume: false,
                calls: Vec::new(),
            })))
        }
        fn calls(&self) -> Vec<Call> {
            std::mem::take(&mut self.0.borrow_mut().calls)
        }
    }

    impl Backend for Shared {
        fn now(&self) -> f64 {
            self.0.borrow().now
        }
        fn state(&self) -> Result<State, String> {
            self.0.borrow().state.clone()
        }
        fn resume(&mut self) -> Result<(), String> {
            let mut m = self.0.borrow_mut();
            m.calls.push(Call::Resume);
            if m.fail_resume {
                return Err("NotAllowedError".into());
            }
            m.state = Ok(State::Running);
            Ok(())
        }
        fn set_engine(&mut self, osc_hz: f32, level: f32) -> Result<(), String> {
            self.0.borrow_mut().calls.push(Call::Engine(osc_hz, level));
            Ok(())
        }
        fn play(&mut self, cue: Cue) -> Result<(), String> {
            let mut m = self.0.borrow_mut();
            m.calls.push(Call::Play(cue));
            if m.fail_play {
                return Err("InvalidStateError".into());
            }
            Ok(())
        }
    }

    #[test]
    fn every_cue_has_a_rendered_buffer() {
        for cue in ALL_CUES {
            let len = crate::synth::render_cue(cue, 48_000.0).len() as f32;
            assert!((len / 48_000.0 - crate::synth::cue_length(cue)).abs() < 1e-3);
        }
    }

    #[test]
    fn engine_wave_is_the_native_partials() {
        let imag = engine_wave();
        for i in 0..64 {
            let p = i as f32 * 0.37;
            let native: f32 = ENGINE_PARTIALS
                .iter()
                .map(|&(k, a)| a * (k * p).sin())
                .sum();
            // Oscillator phase runs at half the engine note.
            let q = p * LOWEST_PARTIAL;
            let web: f32 = (0..ENGINE_WAVE_LEN)
                .map(|h| imag[h] * (h as f32 * q).sin())
                .sum();
            assert!((native - web).abs() < 1e-4, "phase {p}: {native} vs {web}");
        }
        assert_eq!(imag[0], 0.0);
    }

    #[test]
    fn engine_params_match_native_levels_and_clamps() {
        let (hz, level) = engine_params(200.0, 0.5);
        assert_eq!(hz, 100.0);
        assert!((level - ENGINE_LEVEL * MASTER_GAIN * 0.5).abs() < 1e-6);
        assert_eq!(engine_params(1.0, -1.0), (10.0, 0.0));
        assert_eq!(engine_params(9e3, 9.0).0, 1000.0);
        assert!((ENGINE_TAU - 0.025).abs() < 1e-9);
    }

    #[test]
    fn shaper_stage_is_native_tanh() {
        let curve = shaper_curve(1025);
        assert_eq!(curve.len(), 1025);
        for &x in &[-1.5f32, -0.3, 0.0, 0.2, 0.9, 2.5] {
            // Bus scales by 1/RANGE, the shaper interpolates the curve.
            let u = (x / SHAPER_RANGE).clamp(-1.0, 1.0);
            let pos = (u + 1.0) * 0.5 * 1024.0;
            let (i, f) = (pos.floor() as usize, pos.fract());
            let y = curve[i] * (1.0 - f) + curve[(i + 1).min(1024)] * f;
            assert!((y - x.tanh()).abs() < 2e-3, "{x}: {y}");
        }
    }

    #[test]
    fn mute_query() {
        assert!(muted("?mute=1"));
        assert!(muted("?script=race&mute=1&seconds=120"));
        assert!(muted("?mute"));
        assert!(!muted("?mute=0"));
        assert!(!muted("?script=race&seconds=120"));
        assert!(!muted(""));
    }

    /// The real CueTracker drives the web player: same cues, right calls.
    #[test]
    fn tracker_cues_map_to_buffer_plays() {
        let mock = Shared::new(State::Running);
        let mut player = Player::new(Ok(mock.clone()));
        let mut tracker = CueTracker::new();
        let mut lights = StartCountdown::new();
        loop {
            let heard = Heard {
                countdown: lights.label(),
                ..Heard::default()
            };
            tracker.observe(&heard, |cue| player.send(Command::Play(cue)));
            if lights.tick(0.01) {
                break;
            }
            mock.0.borrow_mut().now += 0.01;
        }
        let beeps = [Call::Play(Cue::CountBeep); 3];
        assert_eq!(mock.calls(), [&beeps[..], &[Call::Play(Cue::Go)]].concat());
        for (pad_hits, launch) in [(1, Some(crate::launch::Rating::Perfect)), (2, None)] {
            let heard = Heard {
                pad_hits,
                launch,
                ..Heard::default()
            };
            tracker.observe(&heard, |cue| player.send(Command::Play(cue)));
        }
        assert_eq!(
            mock.calls(),
            [
                Call::Play(Cue::Whoosh(Whoosh::Pad)),
                Call::Play(Cue::Whoosh(Whoosh::Launch)),
                Call::Play(Cue::Whoosh(Whoosh::Pad)),
            ]
        );
        player.send(Command::Engine {
            hz: 120.0,
            gain: 0.4,
        });
        player.send(Command::Engine {
            hz: 120.0,
            gain: 0.4,
        });
        let (hz, level) = engine_params(120.0, 0.4);
        assert_eq!(mock.calls(), [Call::Engine(hz, level)]);
        assert!(player.is_live());
    }

    #[test]
    fn locked_context_drops_cues_until_a_gesture() {
        let mock = Shared::new(State::Suspended);
        let mut player = Player::new(Ok(mock.clone()));
        player.send(Command::Play(Cue::CountBeep));
        player.send(Command::Engine {
            hz: 80.0,
            gain: 0.2,
        });
        let (hz, level) = engine_params(80.0, 0.2);
        assert_eq!(mock.calls(), [Call::Engine(hz, level)]);
        player.unlock();
        player.unlock();
        assert_eq!(mock.calls(), [Call::Resume]);
        player.send(Command::Play(Cue::Go));
        assert_eq!(mock.calls(), [Call::Play(Cue::Go)]);
    }

    #[test]
    fn voices_are_capped_like_native() {
        let mock = Shared::new(State::Running);
        let mut player = Player::new(Ok(mock.clone()));
        for _ in 0..(MAX_VOICES * 3) {
            player.send(Command::Play(Cue::Whoosh(Whoosh::Launch)));
        }
        assert_eq!(mock.calls().len(), MAX_VOICES);
        mock.0.borrow_mut().now = 1.0;
        player.send(Command::Play(Cue::CountBeep));
        assert_eq!(mock.calls(), [Call::Play(Cue::CountBeep)]);
    }

    #[test]
    fn every_failure_goes_silent_for_good() {
        let quiet = |player: &mut Player<Shared>, mock: &Shared| {
            player.unlock();
            player.send(Command::Play(Cue::Go));
            player.send(Command::Engine {
                hz: 99.0,
                gain: 1.0,
            });
            assert!(!player.is_live());
            assert_eq!(mock.calls(), []);
        };
        // No AudioContext at all.
        let mut none = Player::<Shared>::new(Err("AudioContext is not defined".into()));
        quiet(&mut none, &Shared::new(State::Running));
        Player::<Shared>::off().send(Command::Play(Cue::Go));

        // Resume throws.
        let mock = Shared::new(State::Suspended);
        mock.0.borrow_mut().fail_resume = true;
        let mut player = Player::new(Ok(mock.clone()));
        player.unlock();
        assert_eq!(mock.calls(), [Call::Resume]);
        quiet(&mut player, &mock);

        // Resume rejected later, or context closed.
        let mock = Shared::new(State::Running);
        let mut player = Player::new(Ok(mock.clone()));
        mock.0.borrow_mut().state = Err("resume rejected".into());
        quiet(&mut player, &mock);

        // Playback throws.
        let mock = Shared::new(State::Running);
        mock.0.borrow_mut().fail_play = true;
        let mut player = Player::new(Ok(mock.clone()));
        player.send(Command::Play(Cue::CountBeep));
        assert_eq!(mock.calls(), [Call::Play(Cue::CountBeep)]);
        quiet(&mut player, &mock);
    }
}
