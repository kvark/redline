//! Native audio output: streams the shared [`Synth`] to the device.
//!
//! Every sound is generated (no asset files), and the device is opened on a
//! background thread (ALSA via `dlopen` on Linux, cpal elsewhere). If there is no device, no backend, or the stream fails,
//! the thread logs once and exits; [`Output::send`] then drops commands, so the
//! race always runs, just silently. `REDLINE_MUTE=1` skips the device entirely.

use crate::audio::Command;
use crate::synth::Synth;
use std::sync::{Arc, atomic, mpsc};
use std::thread;
#[cfg(not(target_os = "linux"))]
use std::time::Duration;

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
