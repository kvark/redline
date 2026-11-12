//! Native audio output: a tiny procedural synth on cpal's callback thread.
//!
//! Every sound is generated (no asset files), and the device is opened on a
//! background thread (ALSA via `dlopen` on Linux, cpal elsewhere). If there is no device, no backend, or the stream fails,
//! the thread logs once and exits; [`Output::send`] then drops commands, so the
//! race always runs, just silently. `REDLINE_MUTE=1` skips the device entirely.

use crate::audio::{Command, Cue, Whoosh};
use std::f32::consts::TAU;
use std::sync::{Arc, atomic, mpsc};
use std::thread;
#[cfg(not(target_os = "linux"))]
use std::time::Duration;

/// Most overlapping one-shot voices; extra cues are dropped, never queued.
const MAX_VOICES: usize = 8;
const MASTER_GAIN: f32 = 0.35;

pub struct Output {
    tx: mpsc::Sender<Command>,
    stop: Arc<atomic::AtomicBool>,
}

impl Output {
    /// Never blocks: the device is opened (or not) on its own thread.
    pub fn start() -> Self {
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(atomic::AtomicBool::new(false));
        let muted = std::env::var("REDLINE_MUTE").is_ok_and(|v| !v.is_empty() && v != "0");
        if muted {
            log::info!("audio: muted by REDLINE_MUTE");
        } else {
            let stop = Arc::clone(&stop);
            let spawned = thread::Builder::new()
                .name("redline-audio".to_string())
                .spawn(move || match run(rx, &stop) {
                    Ok(()) => {}
                    Err(err) => log::info!("audio: racing silent ({err})"),
                });
            if let Err(err) = spawned {
                log::info!("audio: racing silent (no thread: {err})");
            }
        }
        Self { tx, stop }
    }

    /// Fire-and-forget; a dead or absent audio thread just ignores it.
    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        self.stop.store(true, atomic::Ordering::Relaxed);
    }
}

/// Linux: ALSA, loaded at runtime with `dlopen` so the build needs no system
/// headers and a box without `libasound.so.2` simply stays silent. ALSA's
/// `default` device also reaches PulseAudio / PipeWire through their plugins.
#[cfg(target_os = "linux")]
fn run(rx: mpsc::Receiver<Command>, stop: &atomic::AtomicBool) -> Result<(), String> {
    use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
    type Open = unsafe extern "C" fn(*mut *mut c_void, *const c_char, c_int, c_int) -> c_int;
    type SetParams =
        unsafe extern "C" fn(*mut c_void, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int;
    type Writei = unsafe extern "C" fn(*mut c_void, *const c_void, c_ulong) -> c_long;
    type Recover = unsafe extern "C" fn(*mut c_void, c_int, c_int) -> c_int;
    type Close = unsafe extern "C" fn(*mut c_void) -> c_int;
    const RATE: u32 = 48_000;
    const CHUNK: usize = 480; // 10 ms per write; ALSA's blocking write paces us.
    const PLAYBACK: c_int = 0;
    const S16_LE: c_int = 2;
    const RW_INTERLEAVED: c_int = 3;

    // SAFETY: libasound is a plain C library; every symbol below is called
    // with the signature from <alsa/pcm.h>, on this thread only, and the
    // handle is closed before the library is dropped.
    unsafe {
        let lib = libloading::Library::new("libasound.so.2").map_err(|e| e.to_string())?;
        let sym = |name: &[u8]| -> Result<*const (), String> {
            lib.get::<*const ()>(name)
                .map(|s| *s)
                .map_err(|e| e.to_string())
        };
        let open: Open = std::mem::transmute(sym(b"snd_pcm_open\0")?);
        let set_params: SetParams = std::mem::transmute(sym(b"snd_pcm_set_params\0")?);
        let writei: Writei = std::mem::transmute(sym(b"snd_pcm_writei\0")?);
        let recover: Recover = std::mem::transmute(sym(b"snd_pcm_recover\0")?);
        let close: Close = std::mem::transmute(sym(b"snd_pcm_close\0")?);

        let mut pcm = std::ptr::null_mut();
        if open(&mut pcm, c"default".as_ptr(), PLAYBACK, 0) < 0 || pcm.is_null() {
            return Err("no ALSA output device".to_string());
        }
        if set_params(pcm, S16_LE, RW_INTERLEAVED, 1, RATE, 1, 60_000) < 0 {
            close(pcm);
            return Err("ALSA rejected 48 kHz mono s16".to_string());
        }
        log::info!("audio: {RATE} Hz mono on ALSA default");
        let mut synth = Synth::new(RATE as f32);
        let mut buf = [0i16; CHUNK];
        let mut result = Ok(());
        while !stop.load(atomic::Ordering::Relaxed) {
            while let Ok(command) = rx.try_recv() {
                synth.command(command);
            }
            for s in buf.iter_mut() {
                *s = (synth.sample() * 32767.0) as i16;
            }
            let written = writei(pcm, buf.as_ptr().cast(), CHUNK as c_ulong);
            if written < 0 && recover(pcm, written as c_int, 1) < 0 {
                result = Err(format!("ALSA write failed ({written})"));
                break;
            }
        }
        close(pcm);
        result
    }
}

/// macOS / Windows / BSD: cpal over the platform's native audio API.
#[cfg(not(target_os = "linux"))]
fn run(rx: mpsc::Receiver<Command>, stop: &atomic::AtomicBool) -> Result<(), String> {
    use cpal::traits::{DeviceTrait as _, HostTrait as _, StreamTrait as _};
    let device = cpal::default_host()
        .default_output_device()
        .ok_or("no output device")?;
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let channels = usize::from(supported.channels()).max(1);
    let rate = supported.sample_rate();
    let mut synth = Synth::new(rate as f32);
    let mut fill = move |out: &mut [f32]| {
        while let Ok(command) = rx.try_recv() {
            synth.command(command);
        }
        for frame in out.chunks_mut(channels) {
            frame.fill(synth.sample());
        }
    };
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream::<f32, _, _>(
            supported.config(),
            move |out, _| fill(out),
            |err| log::warn!("audio stream: {err}"),
            None,
        ),
        other => return Err(format!("unsupported sample format {other:?}")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    log::info!("audio: {rate} Hz x{channels} on the default output");
    while !stop.load(atomic::Ordering::Relaxed) {
        thread::park_timeout(Duration::from_millis(250));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
struct Voice {
    cue: Cue,
    t: f32,
    len: f32,
    phase: f32,
    noise_lp: f32,
}

/// Mono procedural mixer: one engine oscillator plus a few one-shot voices.
pub struct Synth {
    dt: f32,
    engine_phase: f32,
    engine_hz: f32,
    engine_gain: f32,
    target_hz: f32,
    target_gain: f32,
    voices: Vec<Voice>,
    noise: u32,
}

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
            noise: 0x9e37_79b9,
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
        // ~25 ms glide so per-frame engine updates never click.
        let glide = (self.dt * 40.0).min(1.0);
        self.engine_hz += (self.target_hz - self.engine_hz) * glide;
        self.engine_gain += (self.target_gain - self.engine_gain) * glide;
        self.engine_phase = (self.engine_phase + self.engine_hz * self.dt).fract();
        let p = self.engine_phase * TAU;
        // Buzzy hover-turbine: fundamental, harmonics, and a half-rate rumble.
        let engine =
            p.sin() + 0.5 * (2.0 * p).sin() + 0.3 * (3.0 * p).sin() + 0.4 * (0.5 * p).sin();
        let mut mix = 0.3 * self.engine_gain * engine;

        let dt = self.dt;
        for i in 0..self.voices.len() {
            let white = self.next_noise();
            mix += voice_sample(&mut self.voices[i], white, dt);
        }
        self.voices.retain(|v| v.t < v.len);
        (mix * MASTER_GAIN).tanh()
    }

    fn next_noise(&mut self) -> f32 {
        // xorshift32: deterministic, allocation-free white noise.
        self.noise ^= self.noise << 13;
        self.noise ^= self.noise >> 17;
        self.noise ^= self.noise << 5;
        self.noise as f32 / u32::MAX as f32 * 2.0 - 1.0
    }
}

fn cue_length(cue: Cue) -> f32 {
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
}
