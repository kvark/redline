//! Time-trial ghost: the player's best lap on a circuit, recorded at a low
//! fixed rate and replayed as a see-through hologram craft on the next race.
//!
//! Laps here are measured on the player's ribbon odometer, line to line, so a
//! ghost lap always starts and ends at the start/finish line. Playback is
//! visual only: the hologram has no collider and never feeds the sim.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{gate_pylons, glb};

/// Samples per second of ghost lap; playback interpolates between them.
pub const RATE_HZ: f32 = 20.0;
const FRAME_DT: f32 = 1.0 / RATE_HZ;
/// Longest lap we keep (two minutes); slower laps are not ghost material.
pub const MAX_FRAMES: usize = 120 * RATE_HZ as usize;
/// Bump when the on-disk layout changes; older files are ignored.
pub const FORMAT: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub pos: [f32; 3],
    pub rot: [f32; 4],
    /// Metres along the ribbon from the line (non-decreasing).
    pub s: f32,
    pub boost: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Ghost {
    pub format: u32,
    pub circuit: String,
    /// Lap length (m) the ghost was recorded on; guards against a changed circuit.
    pub lap_len: f32,
    pub lap_time: f32,
    /// Frame `k` is at `k / RATE_HZ` seconds into the lap.
    pub frames: Vec<Frame>,
}

impl Ghost {
    /// Interpolated pose `t` seconds into the lap (held at the line after it ends).
    pub fn pose(&self, t: f32) -> Option<(glam::Vec3, glam::Quat, bool)> {
        let last = self.frames.len().checked_sub(1)?;
        let x = (t.max(0.0) / FRAME_DT).min(last as f32);
        let i = (x.floor() as usize).min(last.saturating_sub(1));
        let (a, b) = (&self.frames[i], &self.frames[(i + 1).min(last)]);
        let f = (x - i as f32).clamp(0.0, 1.0);
        let pos = glam::Vec3::from(a.pos).lerp(b.pos.into(), f);
        let rot = glam::Quat::from_array(a.rot).slerp(glam::Quat::from_array(b.rot), f);
        Some((
            pos,
            rot.normalize(),
            if f < 0.5 { a.boost } else { b.boost },
        ))
    }

    /// When the ghost reached `s` metres into the lap; None past its line.
    pub fn time_at(&self, s: f32) -> Option<f32> {
        let first = self.frames.first()?;
        if s <= first.s {
            return Some(0.0);
        }
        let i = self.frames.partition_point(|f| f.s < s);
        if i >= self.frames.len() {
            // Between the last sample and the line, finish at `lap_time`.
            let last = self.frames.len() - 1;
            let (t0, s0) = (last as f32 * FRAME_DT, self.frames[last].s);
            if s > self.lap_len || self.lap_len <= s0 {
                return None;
            }
            return Some(t0 + (self.lap_time - t0) * (s - s0) / (self.lap_len - s0));
        }
        let (a, b) = (&self.frames[i - 1], &self.frames[i]);
        let f = if b.s > a.s {
            (s - a.s) / (b.s - a.s)
        } else {
            0.0
        };
        Some((i as f32 - 1.0 + f) * FRAME_DT)
    }

    /// Seconds behind (+) or ahead of (−) the ghost at `s` metres, `clock` into the lap.
    pub fn split(&self, clock: f32, s: f32) -> Option<f32> {
        Some(clock - self.time_at(s.max(0.0))?)
    }
}

/// "+0.4s" behind the ghost, "-0.3s" ahead of it (same style as the leader split).
pub fn split_text(gap: f32) -> String {
    if gap >= 0.0 {
        format!("+{gap:.1}s")
    } else {
        format!("-{:.1}s", -gap)
    }
}

/// Keep `candidate` only if it beats what is stored. Never overwrites a faster lap.
pub fn keep_faster(stored: &mut Option<Ghost>, candidate: Ghost) -> bool {
    if stored
        .as_ref()
        .is_some_and(|best| best.lap_time <= candidate.lap_time)
    {
        return false;
    }
    *stored = Some(candidate);
    true
}

/// Line-to-line lap clock and recorder on the player's ribbon odometer.
#[derive(Clone, Debug, Default)]
pub struct Lapper {
    circuit: String,
    lap_len: f32,
    /// Odometer reading of the line this lap started from; None until GO.
    origin: Option<f32>,
    clock: f32,
    s: f32,
    /// This lap started while the race was live (not a cool-down lap).
    valid: bool,
    frames: Vec<Frame>,
    /// Fastest complete lap of this race.
    pub best: Option<Ghost>,
    pub laps: u32,
}

impl Lapper {
    pub fn new(circuit: &str, lap_len: f32) -> Self {
        Self {
            circuit: circuit.to_string(),
            lap_len,
            ..Self::default()
        }
    }

    pub fn running(&self) -> bool {
        self.origin.is_some()
    }

    pub fn clock(&self) -> f32 {
        self.clock
    }

    /// Metres into the current lap.
    pub fn s(&self) -> f32 {
        self.s
    }

    /// Drop the lap in progress (respawn); the next tick re-anchors on the line.
    pub fn restart(&mut self) {
        self.origin = None;
    }

    /// One physics step at odometer reading `d`. Returns a completed lap time.
    pub fn tick(
        &mut self,
        d: f32,
        pose: (glam::Vec3, glam::Quat, bool),
        racing: bool,
        dt: f32,
    ) -> Option<f32> {
        let len = self.lap_len.max(1.0);
        let Some(origin) = self
            .origin
            .filter(|o| (-0.25..1.5).contains(&((d - o) / len)))
        else {
            // GO, respawn or a teleport: anchor on the nearest line.
            let origin = (d / len).round() * len;
            self.origin = Some(origin);
            (self.clock, self.s, self.valid) = (0.0, d - origin, racing);
            self.frames.clear();
            self.push(pose);
            return None;
        };
        let (prev, mut s) = (self.s, d - origin);
        self.clock += dt;
        let mut done = None;
        if s >= len {
            let frac = ((len - prev) / (s - prev).max(1e-4)).clamp(0.0, 1.0);
            let lap_time = self.clock - dt * (1.0 - frac);
            if self.valid && self.frames.len() >= 2 {
                let frames = std::mem::take(&mut self.frames);
                let lap = Ghost {
                    format: FORMAT,
                    circuit: self.circuit.clone(),
                    lap_len: len,
                    lap_time,
                    frames,
                };
                keep_faster(&mut self.best, lap);
                self.laps += 1;
            }
            self.origin = Some(origin + len);
            self.clock -= lap_time;
            s -= len;
            self.frames.clear();
            self.valid = racing;
            done = Some(lap_time);
        }
        self.s = s;
        if self.clock + 1e-4 >= self.frames.len() as f32 * FRAME_DT {
            self.push(pose);
        }
        done
    }

    fn push(&mut self, (pos, rot, boost): (glam::Vec3, glam::Quat, bool)) {
        if self.frames.len() >= MAX_FRAMES {
            self.valid = false;
            return;
        }
        let s = self.frames.last().map_or(self.s, |f| f.s.max(self.s));
        self.frames.push(Frame {
            pos: pos.into(),
            rot: rot.to_array(),
            s,
            boost,
        });
    }
}

/// Storage key of a circuit's ghost: `ghost-<circuit>` (`.ron` on disk).
pub fn key(circuit: &str) -> String {
    format!("ghost-{circuit}")
}

/// Where ghosts persist: files on native, localStorage on the web. Failures
/// come back as text so callers can log them and carry on in memory.
pub trait Store {
    fn read(&self, key: &str) -> Option<String>;
    fn write(&mut self, key: &str, text: &str) -> Result<(), String>;
    /// Removing a missing entry is not an error.
    fn remove(&mut self, key: &str) -> Result<(), String>;
    fn describe(&self, key: &str) -> String;
}

pub fn boxed(store: impl Store + 'static) -> Box<dyn Store> {
    Box::new(store)
}

/// `<dir>/ghost-<circuit>.ron`.
#[cfg(not(target_arch = "wasm32"))]
pub struct DirStore(pub PathBuf);

#[cfg(not(target_arch = "wasm32"))]
impl DirStore {
    pub fn path(&self, key: &str) -> PathBuf {
        self.0.join(format!("{key}.ron"))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Store for DirStore {
    fn read(&self, key: &str) -> Option<String> {
        std::fs::read_to_string(self.path(key)).ok()
    }

    fn write(&mut self, key: &str, text: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.0).map_err(|err| err.to_string())?;
        std::fs::write(self.path(key), text).map_err(|err| err.to_string())
    }

    fn remove(&mut self, key: &str) -> Result<(), String> {
        match std::fs::remove_file(self.path(key)) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.to_string()),
            _ => Ok(()),
        }
    }

    fn describe(&self, key: &str) -> String {
        self.path(key).display().to_string()
    }
}

/// `localStorage["redline:ghost-<circuit>"]`; the prefix keeps it apart from
/// other pages on a shared origin (github.io). Every call can throw (quota,
/// private mode, storage disabled), so errors are returned, never unwrapped.
#[cfg(target_arch = "wasm32")]
pub struct WebStore(web_sys::Storage);

#[cfg(target_arch = "wasm32")]
impl WebStore {
    const PREFIX: &str = "redline:";

    /// None when the page has no usable localStorage.
    pub fn open() -> Option<Self> {
        match web_sys::window()?.local_storage() {
            Ok(Some(storage)) => Some(Self(storage)),
            Ok(None) => None,
            Err(err) => {
                log::warn!("ghost: localStorage unavailable ({err:?}); session only");
                None
            }
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl Store for WebStore {
    fn read(&self, key: &str) -> Option<String> {
        self.0
            .get_item(&format!("{}{key}", Self::PREFIX))
            .ok()
            .flatten()
    }

    fn write(&mut self, key: &str, text: &str) -> Result<(), String> {
        self.0
            .set_item(&format!("{}{key}", Self::PREFIX), text)
            .map_err(|err| format!("{err:?}"))
    }

    fn remove(&mut self, key: &str) -> Result<(), String> {
        self.0
            .remove_item(&format!("{}{key}", Self::PREFIX))
            .map_err(|err| format!("{err:?}"))
    }

    fn describe(&self, key: &str) -> String {
        format!("localStorage[{}{key}]", Self::PREFIX)
    }
}

/// `?ghost=1` opts a scripted web run into localStorage, like `REDLINE_GHOST_DIR`.
#[cfg(any(target_arch = "wasm32", test))]
pub fn web_opt_in(search: &str) -> bool {
    search
        .trim_start_matches('?')
        .split('&')
        .any(|pair| matches!(pair, "ghost" | "ghost=1"))
}

/// Compact RON payload (no pretty-printing): ~45 KB for a 20 s lap.
pub fn encode(ghost: &Ghost) -> Option<String> {
    ron::to_string(ghost).ok()
}

/// Parse a stored ghost; anything corrupt, from another format or circuit,
/// or physically impossible is skipped with a log line, never a panic.
pub fn decode(text: &str, circuit: &str) -> Option<Ghost> {
    let ghost: Ghost = ron::from_str(text)
        .map_err(|err| log::warn!("ignoring ghost for {circuit}: {err}"))
        .ok()?;
    let finite = |v: &[f32]| v.iter().all(|x| x.is_finite());
    let sane = ghost.format == FORMAT
        && ghost.circuit == circuit
        && ghost.lap_len.is_finite()
        && ghost.lap_len > 0.0
        && ghost.lap_time.is_finite()
        && ghost.lap_time > 0.0
        && (2..=MAX_FRAMES).contains(&ghost.frames.len())
        && ghost
            .frames
            .iter()
            .all(|f| finite(&f.pos) && finite(&f.rot) && f.s.is_finite())
        && ghost.frames.windows(2).all(|w| w[1].s >= w[0].s);
    if !sane {
        log::warn!("ignoring ghost for {circuit}: wrong circuit, format or data");
    }
    sane.then_some(ghost)
}

/// Ghost state for the session: what plays this race and what gets saved.
pub struct Session {
    /// Where `ghost-<circuit>` persists; None keeps ghosts in memory only.
    store: Option<Box<dyn Store>>,
    circuit: String,
    /// Fastest known lap on `circuit`: next race's ghost.
    best: Option<Ghost>,
    /// Fixed for the race in progress.
    pub playing: Option<Ghost>,
    pub lapper: Lapper,
    /// This race set a new best (and saved it, when persisting).
    pub saved: bool,
    /// Menu's view of a circuit's stored lap time, so it is not reread every frame.
    menu_cache: Option<(String, Option<f32>)>,
    /// Storage failures log one warning per session; the rest go to debug.
    warned: bool,
}

impl Session {
    pub fn new(store: Option<Box<dyn Store>>) -> Self {
        Self {
            store,
            circuit: String::new(),
            best: None,
            playing: None,
            lapper: Lapper::default(),
            saved: false,
            menu_cache: None,
            warned: false,
        }
    }

    fn warn(&mut self, what: &str, err: &str) {
        let place = self
            .store
            .as_ref()
            .map_or_else(String::new, |s| s.describe(&key(&self.circuit)));
        if std::mem::replace(&mut self.warned, true) {
            log::debug!("ghost {what} {place}: {err}");
        } else {
            log::warn!("ghost {what} {place}: {err}; keeping it in memory");
        }
    }

    fn read(&self, circuit: &str) -> Option<Ghost> {
        let text = self.store.as_ref()?.read(&key(circuit))?;
        decode(&text, circuit)
    }

    /// New race on `circuit`: play the best lap so far, record a fresh one.
    pub fn begin(&mut self, circuit: &str, lap_len: f32) {
        if self.circuit != circuit {
            self.circuit = circuit.to_string();
            self.best = self.read(circuit);
        }
        let fits = |g: &Ghost| g.circuit == circuit && (g.lap_len - lap_len).abs() < 1.0;
        self.best = self.best.take().filter(fits);
        self.playing = self.best.clone();
        self.lapper = Lapper::new(circuit, lap_len);
        self.saved = false;
    }

    /// Offer this race's best lap; saved only when it beats the stored ghost.
    /// A failed write keeps the new best in memory for the session.
    pub fn commit(&mut self) -> bool {
        let Some(lap) = self.lapper.best.take() else {
            return false;
        };
        if !keep_faster(&mut self.best, lap) {
            return false;
        }
        self.saved = true;
        self.menu_cache = None;
        let (Some(best), Some(store)) = (self.best.as_ref(), self.store.as_mut()) else {
            return true;
        };
        let k = key(&self.circuit);
        let result = encode(best)
            .ok_or_else(|| "encode failed".to_string())
            .and_then(|text| store.write(&k, &text));
        match result {
            Ok(()) => log::info!(
                "ghost saved {:.2}s to {}",
                best.lap_time,
                store.describe(&k)
            ),
            Err(err) => self.warn("not saved to", &err),
        }
        true
    }

    /// Lap time of the ghost the menu offers to clear on `circuit`, if any.
    pub fn stored(&mut self, circuit: &str) -> Option<f32> {
        if circuit == self.circuit
            && let Some(best) = self.best.as_ref()
        {
            return Some(best.lap_time);
        }
        if let Some(&(ref cached, lap)) = self.menu_cache.as_ref()
            && cached == circuit
        {
            return lap;
        }
        let lap = self.read(circuit).map(|g| g.lap_time);
        self.menu_cache = Some((circuit.to_string(), lap));
        lap
    }

    /// Menu "Clear Ghost": forget `circuit`'s ghost in storage and in memory,
    /// so its next race starts without one. Other circuits are untouched.
    pub fn clear(&mut self, circuit: &str) {
        if let Some(Err(err)) = self.store.as_mut().map(|s| s.remove(&key(circuit))) {
            self.warn("not removed from", &err);
        }
        if circuit == self.circuit {
            self.best = None;
            self.playing = None;
            self.lapper.best = None;
            self.saved = false;
        }
        self.menu_cache = Some((circuit.to_string(), None));
        log::info!("ghost cleared for {circuit}");
    }

    pub fn summary(&self) -> String {
        let secs = |g: Option<&Ghost>| g.map_or("-".to_string(), |g| format!("{:.2}s", g.lap_time));
        format!(
            "Ghost: playing={} best={} laps_recorded={} saved={} file={}",
            secs(self.playing.as_ref()),
            secs(self.best.as_ref()),
            self.lapper.laps,
            self.saved,
            self.store
                .as_ref()
                .map_or("-".to_string(), |s| s.describe(&key(&self.circuit))),
        )
    }
}

/// `REDLINE_GHOST_DIR`, else `$XDG_DATA_HOME/redline`, `~/.local/share/redline`
/// or `%APPDATA%\redline`. The menu's Clear Ghost deletes `ghost-<circuit>.ron`.
#[cfg(not(target_arch = "wasm32"))]
pub fn default_dir() -> Option<PathBuf> {
    use std::env::var_os;
    if let Some(dir) = var_os("REDLINE_GHOST_DIR") {
        return Some(dir.into());
    }
    let base = var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        .or_else(|| var_os("APPDATA").map(PathBuf::from))?;
    Some(base.join("redline"))
}

/// Alpha-blended cyan haze inside the lattice: the same particle path the
/// ribbon dust uses on native and WebGL2, so it is truly see-through.
pub fn glow_effect() -> blade_particle::ParticleEffect {
    blade_particle::ParticleEffect {
        capacity: 256,
        emitter: blade_particle::Emitter {
            rate: 0.0,
            burst_count: 0,
            shape: blade_particle::EmitterShape::Sphere { radius: 0.7 },
            cone_angle: 3.1,
        },
        particle: blade_particle::ParticleConfig {
            life: [0.12, 0.3],
            speed: [0.0, 0.4],
            scale: [0.25, 0.5],
            color: blade_particle::ColorConfig::Palette(vec![
                [60, 200, 255, 80],
                [120, 225, 255, 55],
            ]),
        },
    }
}

/// Open emissive lattice of the craft (body frame: +Z nose, +Y up). Blade's
/// raster path is opaque-only, so "see-through" means thin glowing struts you
/// look between, on the same GLB + emissive path the gate pylons use.
pub fn write_model(out_dir: &Path) -> Option<PathBuf> {
    let mut lattice = gate_pylons::Boxes::default();
    let (x, y0, y1, z0, z1) = (0.62, -0.3, 0.2, -1.05, 1.1);
    let mut bar = |a: glam::Vec3, b: glam::Vec3| {
        let dir = (b - a).normalize_or_zero();
        let side = dir.any_orthonormal_vector();
        let half = glam::Vec3::new(0.06, 0.06, (b - a).length() * 0.5 + 0.06);
        lattice.push((a + b) * 0.5, [side, dir.cross(side), dir], half);
    };
    let corner = |i: u32| {
        glam::Vec3::new(
            if i & 1 == 0 { -x } else { x },
            if i & 2 == 0 { y0 } else { y1 },
            if i & 4 == 0 { z0 } else { z1 },
        )
    };
    for i in 0..8u32 {
        for bit in [1, 2, 4] {
            if i & bit == 0 {
                bar(corner(i), corner(i | bit));
            }
        }
    }
    let nose = glam::Vec3::new(0.0, -0.05, 1.6);
    for i in [4, 5, 6, 7] {
        bar(corner(i), nose);
    }
    bar(
        glam::Vec3::new(0.0, y1, z0),
        glam::Vec3::new(0.0, y1 + 0.35, z0 - 0.15),
    );
    let path = out_dir.join("ghost-craft.glb");
    let mesh = lattice.finish("ghost-craft", [0.1, 0.7, 1.0, 1.0], [0.1, 1.3, 2.6]);
    glb::write_glb(&path, &[mesh]).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEN: f32 = 100.0;

    /// Drive `laps` laps at `speed` m/s from the line; returns the lapper.
    fn drive(lapper: &mut Lapper, start: f32, speeds: &[f32]) -> Vec<f32> {
        let dt = 0.01;
        let mut d = start;
        let mut done = Vec::new();
        for &speed in speeds {
            let target = (d / LEN).floor() * LEN + LEN;
            while d < target + 0.5 {
                d += speed * dt;
                let pos = glam::Vec3::new(d, 0.0, 0.0);
                if let Some(t) = lapper.tick(d, (pos, glam::Quat::IDENTITY, speed > 20.0), true, dt)
                {
                    done.push(t);
                }
            }
        }
        done
    }

    fn ghost(speed: f32) -> Ghost {
        let mut lapper = Lapper::new("test", LEN);
        lapper.tick(
            0.0,
            (glam::Vec3::ZERO, glam::Quat::IDENTITY, false),
            true,
            0.01,
        );
        drive(&mut lapper, 0.0, &[speed]);
        lapper.best.expect("a lap")
    }

    #[test]
    fn records_one_lap_at_the_fixed_rate() {
        let g = ghost(20.0);
        assert!((g.lap_time - 5.0).abs() < 0.02, "lap {}", g.lap_time);
        assert_eq!(g.circuit, "test");
        // 20 Hz over 5 s, plus the frame at the line.
        assert!((99..=101).contains(&g.frames.len()), "{}", g.frames.len());
        assert!(g.frames.windows(2).all(|w| w[1].s >= w[0].s));
    }

    #[test]
    fn keeps_the_fastest_lap_of_the_race() {
        let mut lapper = Lapper::new("test", LEN);
        lapper.tick(
            0.0,
            (glam::Vec3::ZERO, glam::Quat::IDENTITY, false),
            true,
            0.01,
        );
        let laps = drive(&mut lapper, 0.0, &[20.0, 25.0, 16.0]);
        assert_eq!(laps.len(), 3);
        assert_eq!(lapper.laps, 3);
        let best = lapper.best.as_ref().unwrap();
        assert!((best.lap_time - 4.0).abs() < 0.02, "best {}", best.lap_time);
        assert!(best.frames[2..].iter().all(|f| f.boost));
    }

    #[test]
    fn playback_interpolates_between_samples() {
        let g = ghost(20.0);
        let (p, q, _) = g.pose(0.125).unwrap();
        // Recorded positions are the odometer along +X: 20 m/s * 0.125 s.
        assert!((p.x - g.frames[0].pos[0] - 2.5).abs() < 0.25, "{p}");
        assert!(q.is_normalized());
        // Past the end the hologram holds at the line.
        let last = g.frames.last().unwrap().pos;
        assert_eq!(g.pose(99.0).unwrap().0, glam::Vec3::from(last));
        assert!(
            Ghost {
                frames: vec![],
                ..g
            }
            .pose(1.0)
            .is_none()
        );
    }

    #[test]
    fn split_is_time_behind_the_ghost_at_the_same_spot() {
        let g = ghost(20.0);
        // At 32 m after 2.0 s at 16 m/s; the ghost was there at 1.6 s.
        let gap = g.split(2.0, 32.0).unwrap();
        assert!((gap - 0.4).abs() < 0.03, "gap {gap}");
        let ahead = g.split(1.0, 30.0).unwrap();
        assert!((ahead + 0.5).abs() < 0.03, "ahead {ahead}");
        assert_eq!(split_text(gap), "+0.4s");
        assert_eq!(split_text(-0.26), "-0.3s");
        // Near the line it finishes at the lap time; beyond the lap, no split.
        assert!((g.time_at(LEN).unwrap() - g.lap_time).abs() < 0.02);
        assert!(g.split(9.0, LEN + 5.0).is_none());
    }

    #[test]
    fn never_overwrites_a_faster_lap() {
        let (fast, slow) = (ghost(25.0), ghost(20.0));
        let mut stored = Some(fast.clone());
        assert!(!keep_faster(&mut stored, slow.clone()));
        assert_eq!(stored.as_ref(), Some(&fast));
        let mut empty = None;
        assert!(keep_faster(&mut empty, slow.clone()));
        assert!(keep_faster(&mut empty, fast.clone()));
        assert_eq!(empty, Some(fast));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn session_saves_per_circuit_and_keeps_the_faster_file() {
        let dir = std::env::temp_dir().join(format!("redline-ghost-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = || Some(boxed(DirStore(dir.clone())));
        let mut session = Session::new(files());
        session.begin("test", LEN);
        assert!(session.playing.is_none());
        session.lapper.best = Some(ghost(25.0));
        assert!(session.commit());
        let path = dir.join("ghost-test.ron");
        let load = || decode(&std::fs::read_to_string(&path).ok()?, "test");
        let saved = load().expect("saved ghost");
        // A slower race leaves the file alone.
        let mut next = Session::new(files());
        next.begin("test", LEN);
        assert_eq!(next.playing.as_ref(), Some(&saved));
        next.lapper.best = Some(ghost(20.0));
        assert!(!next.commit());
        assert_eq!(load(), Some(saved));
        // Other circuits, or a changed lap length, do not play this ghost.
        next.begin("other", LEN);
        assert!(next.playing.is_none());
        next.begin("test", LEN * 2.0);
        assert!(next.playing.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn native_clear_deletes_the_file_and_the_hologram() {
        let dir = std::env::temp_dir().join(format!("redline-ghost-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::new(Some(Box::new(DirStore(dir.clone()))));
        session.begin("test", LEN);
        session.lapper.best = Some(ghost(25.0));
        assert!(session.commit());
        session.begin("test", LEN);
        assert!(session.playing.is_some());
        assert!(session.stored("test").is_some());
        let path = dir.join("ghost-test.ron");
        assert!(path.exists());
        session.clear("test");
        assert!(!path.exists());
        assert!(session.playing.is_none());
        assert_eq!(session.stored("test"), None);
        // Clearing again (no file) is fine; the next race has no ghost.
        session.clear("test");
        session.begin("test", LEN);
        assert!(session.playing.is_none());
        let mut fresh = Session::new(Some(Box::new(DirStore(dir.clone()))));
        fresh.begin("test", LEN);
        assert!(fresh.playing.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn on(circuit: &str, speed: f32) -> Ghost {
        Ghost {
            circuit: circuit.to_string(),
            ..ghost(speed)
        }
    }

    /// localStorage stand-in: shared map, optionally full (quota) or throwing.
    #[derive(Clone, Default)]
    struct FakeStorage {
        items: std::rc::Rc<std::cell::RefCell<std::collections::HashMap<String, String>>>,
        quota: Option<usize>,
        throws: bool,
    }

    impl Store for FakeStorage {
        fn read(&self, key: &str) -> Option<String> {
            if self.throws {
                return None;
            }
            self.items.borrow().get(key).cloned()
        }
        fn write(&mut self, key: &str, text: &str) -> Result<(), String> {
            if self.throws || self.quota.is_some_and(|q| text.len() > q) {
                return Err("QuotaExceededError".to_string());
            }
            self.items.borrow_mut().insert(key.into(), text.into());
            Ok(())
        }
        fn remove(&mut self, key: &str) -> Result<(), String> {
            if self.throws {
                return Err("SecurityError".to_string());
            }
            self.items.borrow_mut().remove(key);
            Ok(())
        }
        fn describe(&self, key: &str) -> String {
            format!("fake[{key}]")
        }
    }

    fn web(storage: &FakeStorage) -> Session {
        Session::new(Some(Box::new(storage.clone())))
    }

    #[test]
    fn web_ghost_survives_a_reload_and_only_faster_replaces_it() {
        let storage = FakeStorage::default();
        let mut tab = web(&storage);
        tab.begin("mars", LEN);
        tab.lapper.best = Some(on("mars", 20.0));
        assert!(tab.commit());
        assert!(storage.items.borrow().contains_key("ghost-mars"));
        // "Reload": a new session over the same storage plays the saved lap.
        let mut reload = web(&storage);
        reload.begin("mars", LEN);
        let first = reload.playing.clone().expect("ghost after reload");
        assert!((first.lap_time - 5.0).abs() < 0.02);
        reload.lapper.best = Some(on("mars", 16.0));
        assert!(!reload.commit());
        reload.lapper.best = Some(on("mars", 25.0));
        assert!(reload.commit());
        let mut again = web(&storage);
        again.begin("mars", LEN);
        assert!((again.playing.unwrap().lap_time - 4.0).abs() < 0.02);
        // Compact enough for localStorage: well under 100 KB for a 5 s lap.
        assert!(storage.items.borrow()["ghost-mars"].len() < 20_000);
    }

    #[test]
    fn web_skips_other_circuits_and_bad_payloads() {
        let storage = FakeStorage::default();
        let mut tab = web(&storage);
        tab.begin("mars", LEN);
        tab.lapper.best = Some(on("mars", 20.0));
        tab.commit();
        let good = storage.items.borrow()["ghost-mars"].clone();
        let put = |text: &str| {
            storage
                .items
                .borrow_mut()
                .insert("ghost-mars".into(), text.into());
            let mut s = web(&storage);
            s.begin("mars", LEN);
            s.playing
        };
        assert!(put(&good).is_some());
        // A ghost recorded on another circuit, under this circuit's key.
        assert!(put(&good.replace("\"mars\"", "\"phobos\"")).is_none());
        assert!(put(&good.replace("format:1", "format:9")).is_none());
        assert!(put(&good[..good.len() / 2]).is_none());
        assert!(put("").is_none());
        assert!(put("{\"not\":\"ron\"}").is_none());
        assert!(put(&good.replacen("lap_time:", "lap_time:NaN,x:", 1)).is_none());
        assert!(put(&good.replacen("pos:(", "pos:(inf,", 1)).is_none());
        // Same circuit, different lap length: kept on disk but not played.
        let mut longer = web(&storage);
        storage
            .items
            .borrow_mut()
            .insert("ghost-mars".into(), good.clone());
        longer.begin("mars", LEN * 1.5);
        assert!(longer.playing.is_none());
        // Another circuit never picks up this circuit's entry.
        let mut other = web(&storage);
        other.begin("phobos", LEN);
        assert!(other.playing.is_none());
    }

    #[test]
    fn web_clear_ghost_removes_storage_and_memory_for_one_circuit() {
        let storage = FakeStorage::default();
        let mut tab = web(&storage);
        for circuit in ["mars", "phobos"] {
            tab.begin(circuit, LEN);
            tab.lapper.best = Some(on(circuit, 20.0));
            tab.commit();
        }
        tab.begin("mars", LEN);
        assert!(tab.playing.is_some());
        assert!(tab.stored("phobos").is_some());
        tab.clear("mars");
        assert!(tab.playing.is_none());
        assert_eq!(tab.stored("mars"), None);
        assert!(!storage.items.borrow().contains_key("ghost-mars"));
        assert!(storage.items.borrow().contains_key("ghost-phobos"));
        tab.begin("mars", LEN);
        assert!(tab.playing.is_none());
        let mut reload = web(&storage);
        reload.begin("mars", LEN);
        assert!(reload.playing.is_none());
        reload.begin("phobos", LEN);
        assert!(reload.playing.is_some());
    }

    #[test]
    fn web_storage_failures_keep_the_session_ghost() {
        for storage in [
            FakeStorage {
                quota: Some(100),
                ..FakeStorage::default()
            },
            FakeStorage {
                throws: true,
                ..FakeStorage::default()
            },
        ] {
            let mut tab = web(&storage);
            tab.begin("mars", LEN);
            tab.lapper.best = Some(on("mars", 20.0));
            assert!(tab.commit());
            assert!(storage.items.borrow().is_empty());
            tab.begin("mars", LEN);
            assert!(tab.playing.is_some(), "in-memory ghost for the session");
            assert!(tab.stored("mars").is_some());
            tab.clear("mars");
            assert!(tab.playing.is_none());
        }
    }

    #[test]
    fn ghost_query_opts_scripted_web_runs_in() {
        assert!(web_opt_in("?script=race&ghost=1"));
        assert!(web_opt_in("?ghost"));
        assert!(!web_opt_in("?script=race&ghost=0"));
        assert!(!web_opt_in("?ghosts=1"));
        assert!(!web_opt_in(""));
    }

    #[test]
    fn respawn_drops_the_lap_and_reanchors_on_the_line() {
        let mut lapper = Lapper::new("test", LEN);
        let pose = (glam::Vec3::ZERO, glam::Quat::IDENTITY, false);
        lapper.tick(-1.0, pose, true, 0.01);
        assert!(lapper.running());
        assert!((lapper.s() + 1.0).abs() < 1e-5);
        lapper.tick(40.0, pose, true, 0.01);
        lapper.restart();
        lapper.tick(201.0, pose, true, 0.01);
        assert!((lapper.s() - 1.0).abs() < 1e-4);
        assert_eq!(lapper.clock(), 0.0);
        assert!(lapper.best.is_none());
    }
}
