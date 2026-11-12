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

/// Ghost state for the session: what plays this race and what gets saved.
pub struct Session {
    /// Where `ghost-<circuit>.ron` lives; None keeps ghosts in memory only.
    dir: Option<PathBuf>,
    circuit: String,
    /// Fastest known lap on `circuit`: next race's ghost.
    best: Option<Ghost>,
    /// Fixed for the race in progress.
    pub playing: Option<Ghost>,
    pub lapper: Lapper,
    /// This race set a new best (and saved it, when persisting).
    pub saved: bool,
}

impl Session {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self {
            dir,
            circuit: String::new(),
            best: None,
            playing: None,
            lapper: Lapper::default(),
            saved: false,
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        let dir = self.dir.as_ref()?;
        Some(dir.join(format!("ghost-{}.ron", self.circuit)))
    }

    /// New race on `circuit`: play the best lap so far, record a fresh one.
    pub fn begin(&mut self, circuit: &str, lap_len: f32) {
        if self.circuit != circuit {
            self.circuit = circuit.to_string();
            self.best = self.path().and_then(|p| load(&p));
        }
        let fits = |g: &Ghost| g.circuit == circuit && (g.lap_len - lap_len).abs() < 1.0;
        self.best = self.best.take().filter(fits);
        self.playing = self.best.clone();
        self.lapper = Lapper::new(circuit, lap_len);
        self.saved = false;
    }

    /// Offer this race's best lap; saved only when it beats the stored ghost.
    pub fn commit(&mut self) -> bool {
        let Some(lap) = self.lapper.best.take() else {
            return false;
        };
        if !keep_faster(&mut self.best, lap) {
            return false;
        }
        if let (Some(path), Some(best)) = (self.path(), self.best.as_ref()) {
            match save(&path, best) {
                Ok(()) => log::info!("ghost saved {:.2}s to {}", best.lap_time, path.display()),
                Err(err) => log::warn!("ghost not saved to {}: {err}", path.display()),
            }
        }
        self.saved = true;
        true
    }

    pub fn summary(&self) -> String {
        let secs = |g: Option<&Ghost>| g.map_or("-".to_string(), |g| format!("{:.2}s", g.lap_time));
        format!(
            "Ghost: playing={} best={} laps_recorded={} saved={} file={}",
            secs(self.playing.as_ref()),
            secs(self.best.as_ref()),
            self.lapper.laps,
            self.saved,
            self.path()
                .map_or("-".to_string(), |p| p.display().to_string()),
        )
    }
}

pub fn load(path: &Path) -> Option<Ghost> {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let text = std::fs::read_to_string(path).ok()?;
        let ghost: Ghost = ron::from_str(&text)
            .map_err(|err| log::warn!("ignoring ghost {}: {err}", path.display()))
            .ok()?;
        (ghost.format == FORMAT).then_some(ghost)
    }
}

pub fn save(path: &Path, ghost: &Ghost) -> std::io::Result<()> {
    let text = ron::to_string(ghost).map_err(std::io::Error::other)?;
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (path, text);
        Ok(())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, text)
    }
}

/// `REDLINE_GHOST_DIR`, else `$XDG_DATA_HOME/redline`, `~/.local/share/redline`
/// or `%APPDATA%\redline`. Delete `ghost-<circuit>.ron` there to clear a ghost.
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

    #[test]
    fn session_saves_per_circuit_and_keeps_the_faster_file() {
        let dir = std::env::temp_dir().join(format!("redline-ghost-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut session = Session::new(Some(dir.clone()));
        session.begin("test", LEN);
        assert!(session.playing.is_none());
        session.lapper.best = Some(ghost(25.0));
        assert!(session.commit());
        let path = dir.join("ghost-test.ron");
        let saved = load(&path).expect("saved ghost");
        // A slower race leaves the file alone.
        let mut next = Session::new(Some(dir.clone()));
        next.begin("test", LEN);
        assert_eq!(next.playing.as_ref(), Some(&saved));
        next.lapper.best = Some(ghost(20.0));
        assert!(!next.commit());
        assert_eq!(load(&path), Some(saved));
        // Other circuits, or a changed lap length, do not play this ghost.
        next.begin("other", LEN);
        assert!(next.playing.is_none());
        next.begin("test", LEN * 2.0);
        assert!(next.playing.is_none());
        let _ = std::fs::remove_dir_all(&dir);
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
