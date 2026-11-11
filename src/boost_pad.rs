//! Ribbon boost pads: glowing chevron strips laid once per timed sector.
//!
//! A pad is a rectangle on the ribbon surface. Rolling onto one arms a short
//! forward thrust that [`crate::vehicle::Vehicle::drive`] applies through the
//! chassis rigid body (never a teleport), so player, AI and scripted drives all
//! get the same physics.

use std::path::{Path, PathBuf};

use crate::{config, glb, planet};

/// Pads laid in each timed sector (the launch sector stays clear).
pub const PADS_PER_SECTOR: usize = 1;
/// Half length of a pad along the ribbon (m).
pub const PAD_HALF_LENGTH: f32 = 2.6;
/// Upper bound on half width across the ribbon (m); narrow ribbons shrink it.
pub const PAD_MAX_HALF_WIDTH: f32 = 3.0;
/// The chassis may ride this high over the pad and still trigger it (m).
pub const PAD_TRIGGER_HEIGHT: f32 = 2.2;
/// Seconds of thrust after rolling onto a pad.
pub const PAD_BOOST_SECS: f32 = 0.5;
/// Seconds before any pad can fire again for the same craft.
pub const PAD_COOLDOWN_SECS: f32 = 0.7;
/// Peak extra target-speed fraction from a pad (1.0 + this).
pub const PAD_SPEED_BONUS: f32 = 0.06;
/// Forward accel (m/s²) applied as a chassis impulse at peak pad intensity.
pub const PAD_ACCEL: f32 = 4.5;
/// Cap on the summed pad + scrub→rejoin target-speed bonus.
pub const MAX_SPEED_BONUS: f32 = 0.36;
/// Cap on the summed pad + scrub→rejoin forward accel (m/s²).
pub const MAX_BOOST_ACCEL: f32 = 26.0;
/// Keep pads this many track samples away from sector gates.
const GATE_MARGIN: usize = 3;
/// Height of the chevrons above the ribbon (m) so the faceted terrain never eats them.
const CHEVRON_LIFT: f32 = 0.09;
const PLATE_LIFT: f32 = 0.06;

#[derive(Clone, Copy, Debug)]
pub struct Pad {
    /// 1-based sector (matches the HUD's "Sector n / m").
    pub sector: usize,
    /// Track sample index at the pad center.
    pub sample: usize,
    pub position: glam::Vec3,
    pub normal: glam::Vec3,
    pub tangent: glam::Vec3,
    pub side: glam::Vec3,
    pub half_width: f32,
}

impl Pad {
    pub fn contains(&self, position: glam::Vec3) -> bool {
        let delta = position - self.position;
        let reach = PAD_HALF_LENGTH + self.half_width + PAD_TRIGGER_HEIGHT;
        if delta.length_squared() > reach * reach {
            return false;
        }
        let height = delta.dot(self.normal);
        delta.dot(self.tangent).abs() <= PAD_HALF_LENGTH
            && delta.dot(self.side).abs() <= self.half_width
            && (-1.0..=PAD_TRIGGER_HEIGHT).contains(&height)
    }
}

pub fn pad_half_width(track_width: f32) -> f32 {
    (track_width * 0.5 * 0.55).min(PAD_MAX_HALF_WIDTH)
}

/// Lay `per_sector` pads in every sector except the launch one, using the same
/// checkpoint stride as [`crate::race::Race::new`]. Each pad sits on the
/// straightest stretch of its slot so a boost never fires into a hairpin.
pub fn place(
    track: &[planet::TrackSample],
    checkpoint_count: usize,
    per_sector: usize,
    track_width: f32,
) -> Vec<Pad> {
    let count = checkpoint_count.max(4);
    let len = track.len();
    let stride = (len / count).max(1);
    if per_sector == 0 || stride <= GATE_MARGIN * 2 {
        return Vec::new();
    }
    let half_width = pad_half_width(track_width);
    let mut pads = Vec::new();
    for gate in 1..count {
        let start = gate * stride;
        let end = if gate + 1 == count {
            len
        } else {
            (gate + 1) * stride
        };
        let lo = start + GATE_MARGIN;
        let hi = end.saturating_sub(GATE_MARGIN).max(lo + 1);
        let slot = ((hi - lo) / per_sector).max(1);
        for k in 0..per_sector {
            // Search only the middle half of each slot so pads stay evenly
            // spaced instead of bunching on one long straight.
            let slot_lo = lo + k * slot + slot / 4;
            let slot_hi = (lo + k * slot + slot - slot / 4).min(hi);
            if slot_lo >= slot_hi {
                break;
            }
            let sample = (slot_lo..slot_hi)
                .min_by(|&a, &b| {
                    turn_rate(track, a)
                        .partial_cmp(&turn_rate(track, b))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(slot_lo)
                % len;
            let s = track[sample];
            let next = track[(sample + 1) % len];
            let mut tangent = (next.position - s.position).reject_from(s.normal);
            if tangent.length_squared() < 1e-6 {
                tangent = s.tangent;
            }
            let tangent = tangent.normalize_or_zero();
            pads.push(Pad {
                // Gate `g` closes HUD sector `g`; the pad is on the way to gate g+1.
                sector: gate + 1,
                sample,
                position: s.position,
                normal: s.normal,
                tangent,
                side: s.normal.cross(tangent).normalize_or_zero(),
                half_width,
            });
        }
    }
    pads
}

/// Geodesic turn (radians per sample pair) around the surface normal.
fn turn_rate(track: &[planet::TrackSample], index: usize) -> f32 {
    let len = track.len();
    let prev = track[(index + len - 2) % len];
    let here = track[index % len];
    let next = track[(index + 2) % len];
    let a = (here.position - prev.position).reject_from(here.normal);
    let b = (next.position - here.position).reject_from(here.normal);
    a.normalize_or_zero()
        .cross(b.normalize_or_zero())
        .dot(here.normal)
        .abs()
}

pub fn pad_under(pads: &[Pad], position: glam::Vec3) -> Option<usize> {
    pads.iter().position(|pad| pad.contains(position))
}

/// Per-craft pad trigger, thrust timer and cooldown.
#[derive(Clone, Debug, Default)]
pub struct PadBoost {
    timer: f32,
    cooldown: f32,
    /// Pad currently under the craft; a pad fires on entry, not while parked on it.
    inside: Option<usize>,
    hits: u32,
}

impl PadBoost {
    /// Advance one step. Returns thrust intensity in [0, 1] for this frame.
    pub fn tick(&mut self, pads: &[Pad], position: glam::Vec3, dt: f32) -> f32 {
        self.cooldown = (self.cooldown - dt).max(0.0);
        let under = pad_under(pads, position);
        if let Some(index) = under
            && self.inside != Some(index)
            && self.cooldown <= 0.0
        {
            self.timer = PAD_BOOST_SECS;
            self.cooldown = PAD_COOLDOWN_SECS;
            self.hits += 1;
        }
        self.inside = under;
        if self.timer > 0.0 {
            let intensity = (self.timer / PAD_BOOST_SECS).clamp(0.0, 1.0);
            self.timer = (self.timer - dt).max(0.0);
            // Ease-out so the shove front-loads and fades without a step.
            intensity.sqrt()
        } else {
            0.0
        }
    }

    pub fn hits(&self) -> u32 {
        self.hits
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Keep the hit count (race stats) but drop any live thrust.
    pub fn cancel(&mut self) {
        self.timer = 0.0;
        self.cooldown = 0.0;
        self.inside = None;
    }
}

/// Sum pad and scrub→rejoin boosts with caps so the two stack without launching
/// the craft. Returns (target-speed bonus fraction, forward accel m/s²).
pub fn combine(ribbon: f32, pad: f32, ribbon_speed_bonus: f32, ribbon_accel: f32) -> (f32, f32) {
    let speed = (ribbon * ribbon_speed_bonus + pad * PAD_SPEED_BONUS).min(MAX_SPEED_BONUS);
    let accel = (ribbon * ribbon_accel + pad * PAD_ACCEL).min(MAX_BOOST_ACCEL);
    (speed.max(0.0), accel.max(0.0))
}

/// Bake every pad into one world-space GLB: a dark plate plus three glowing
/// chevrons, draped on the ribbon height field.
pub fn write_model(
    out_dir: &Path,
    pads: &[Pad],
    planet_cfg: config::Planet,
    track: &[planet::TrackSample],
) -> Option<PathBuf> {
    if pads.is_empty() {
        return None;
    }
    let track_dirs: Vec<glam::Vec3> = track.iter().map(|s| s.normal).collect();
    let drape = |pad: &Pad, x: f32, z: f32, lift: f32| -> [f32; 3] {
        let p = pad.position + pad.side * x + pad.tangent * z;
        let dir = p.normalize_or_zero();
        (dir * (planet::surface_height(dir, planet_cfg, &track_dirs) + lift)).into()
    };
    let mut plate = QuadMesh::default();
    let mut chevrons = QuadMesh::default();
    for pad in pads.iter() {
        let w = pad.half_width;
        let l = PAD_HALF_LENGTH;
        // Subdivided so the plate follows the ribbon's curvature instead of
        // sinking into it mid-pad.
        plate.patch(
            pad,
            &drape,
            [[-w, -l], [w, -l], [w, l], [-w, l]],
            PLATE_LIFT,
            6,
        );
        // Three forward-pointing chevrons, tips toward +tangent.
        let arm = 0.42;
        let rise = w * 0.55;
        for k in 0..3 {
            let base = -l + 0.55 + k as f32 * (2.0 * l - 1.1) / 3.0;
            for sign in [-1.0f32, 1.0] {
                let outer = w * 0.86 * sign;
                // Arm from the outer foot (outer, base) to the tip (0, base + rise).
                chevrons.patch(
                    pad,
                    &drape,
                    [
                        [outer, base],
                        [0.0, base + rise],
                        [0.0, base + rise + arm],
                        [outer, base + arm],
                    ],
                    CHEVRON_LIFT,
                    4,
                );
            }
        }
    }
    let path = out_dir.join("boost-pads.glb");
    let meshes = [
        plate.finish("boost-plate", [0.02, 0.03, 0.05, 1.0], [0.01, 0.05, 0.09]),
        chevrons.finish("boost-chevrons", [0.05, 0.6, 0.8, 1.0], [0.25, 1.6, 2.4]),
    ];
    glb::write_glb(&path, &meshes).ok()?;
    Some(path)
}

#[derive(Default)]
struct QuadMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl QuadMesh {
    /// Push a quad given in pad-local (x across, z along) corners, split into
    /// `n`×`n` cells and draped on the terrain. Winding faces away from the planet.
    fn patch(
        &mut self,
        pad: &Pad,
        drape: &impl Fn(&Pad, f32, f32, f32) -> [f32; 3],
        corners: [[f32; 2]; 4],
        lift: f32,
        n: u32,
    ) {
        let lerp2 =
            |a: [f32; 2], b: [f32; 2], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
        let base = self.positions.len() as u32;
        for j in 0..=n {
            let v = j as f32 / n as f32;
            let left = lerp2(corners[0], corners[3], v);
            let right = lerp2(corners[1], corners[2], v);
            for i in 0..=n {
                let p = lerp2(left, right, i as f32 / n as f32);
                self.positions.push(drape(pad, p[0], p[1], lift));
                self.normals.push(pad.normal.into());
            }
        }
        let a = glam::Vec3::from(self.positions[base as usize]);
        let b = glam::Vec3::from(self.positions[(base + n) as usize]);
        let c = glam::Vec3::from(self.positions[(base + n * (n + 1) + n) as usize]);
        let flip = (b - a).cross(c - a).dot(pad.normal) < 0.0;
        let row = n + 1;
        for j in 0..n {
            for i in 0..n {
                let i0 = base + j * row + i;
                let (i1, i2, i3) = (i0 + 1, i0 + row + 1, i0 + row);
                if flip {
                    self.indices.extend_from_slice(&[i0, i2, i1, i0, i3, i2]);
                } else {
                    self.indices.extend_from_slice(&[i0, i1, i2, i0, i2, i3]);
                }
            }
        }
    }

    fn finish(self, name: &str, base_color: [f32; 4], emissive: [f32; 3]) -> glb::MeshData {
        let tex_coords = vec![[0.5, 0.5]; self.positions.len()];
        glb::MeshData {
            name: name.to_string(),
            positions: self.positions,
            normals: self.normals,
            tex_coords,
            indices: self.indices,
            base_color,
            metallic: 0.1,
            roughness: 0.35,
            emissive,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(count: usize, radius: f32) -> Vec<planet::TrackSample> {
        (0..count)
            .map(|i| {
                let a = i as f32 / count as f32 * std::f32::consts::TAU;
                let n = glam::Vec3::new(a.sin(), 0.0, a.cos());
                planet::TrackSample {
                    position: n * radius,
                    tangent: glam::Vec3::new(a.cos(), 0.0, -a.sin()),
                    normal: n,
                }
            })
            .collect()
    }

    #[test]
    fn pads_fill_every_timed_sector_and_skip_launch() {
        let track = ring(256, 80.0);
        let pads = place(&track, 16, 1, 14.0);
        assert_eq!(pads.len(), 15);
        let stride = 256 / 16;
        for pad in pads.iter() {
            assert!(pad.sample >= stride, "launch sector must stay clear");
            // Pad sample lies inside its own sector, clear of both gates.
            let gate = pad.sector - 1;
            assert!(pad.sample >= gate * stride + GATE_MARGIN);
            assert!(pad.sample < (gate + 1) * stride - GATE_MARGIN + 1);
        }
        let sectors: Vec<usize> = pads.iter().map(|p| p.sector).collect();
        assert_eq!(sectors, (2..=16).collect::<Vec<_>>());
    }

    #[test]
    fn multiple_pads_per_sector_stay_ordered() {
        let track = ring(256, 80.0);
        let pads = place(&track, 8, 2, 14.0);
        assert_eq!(pads.len(), 14);
        for pair in pads.windows(2) {
            assert!(pair[0].sample < pair[1].sample);
        }
    }

    #[test]
    fn narrow_ribbon_shrinks_pad_width() {
        assert_eq!(pad_half_width(14.0), PAD_MAX_HALF_WIDTH);
        assert!(pad_half_width(9.5) < PAD_MAX_HALF_WIDTH);
    }

    #[test]
    fn pad_contains_only_its_footprint() {
        let track = ring(256, 80.0);
        let pad = place(&track, 16, 1, 14.0)[0];
        let above = pad.position + pad.normal * 0.8;
        assert!(pad.contains(above));
        assert!(pad.contains(above + pad.tangent * (PAD_HALF_LENGTH - 0.1)));
        assert!(!pad.contains(above + pad.tangent * (PAD_HALF_LENGTH + 0.3)));
        assert!(!pad.contains(above + pad.side * (pad.half_width + 0.3)));
        assert!(!pad.contains(pad.position + pad.normal * (PAD_TRIGGER_HEIGHT + 0.5)));
    }

    #[test]
    fn trigger_fires_on_entry_then_decays() {
        let track = ring(256, 80.0);
        let pads = place(&track, 16, 1, 14.0);
        let on = pads[0].position + pads[0].normal * 0.6;
        let off = glam::Vec3::Y * 80.0;
        let mut boost = PadBoost::default();
        assert_eq!(boost.tick(&pads, off, 0.01), 0.0);
        let first = boost.tick(&pads, on, 0.01);
        assert!((first - 1.0).abs() < 1e-6);
        assert_eq!(boost.hits(), 1);
        // Sitting on the pad does not re-fire; the pulse decays to zero.
        let mut last = first;
        for _ in 0..((PAD_BOOST_SECS / 0.01) as usize + 2) {
            let now = boost.tick(&pads, on, 0.01);
            assert!(now <= last);
            last = now;
        }
        assert_eq!(last, 0.0);
        assert_eq!(boost.hits(), 1);
    }

    #[test]
    fn cooldown_blocks_back_to_back_pads() {
        let track = ring(256, 80.0);
        let pads = place(&track, 16, 1, 14.0);
        let a = pads[0].position + pads[0].normal * 0.6;
        let b = pads[1].position + pads[1].normal * 0.6;
        let mut boost = PadBoost::default();
        boost.tick(&pads, a, 0.01);
        assert_eq!(boost.hits(), 1);
        // Next pad reached inside the cooldown: no second pulse.
        boost.tick(&pads, b, PAD_COOLDOWN_SECS * 0.5);
        assert_eq!(boost.hits(), 1);
        // Leave and come back after the cooldown: fires again.
        boost.tick(&pads, glam::Vec3::Y * 80.0, PAD_COOLDOWN_SECS);
        assert!(boost.tick(&pads, b, 0.01) > 0.0);
        assert_eq!(boost.hits(), 2);
        boost.cancel();
        assert_eq!(boost.tick(&pads, glam::Vec3::Y * 80.0, 0.01), 0.0);
        assert_eq!(boost.hits(), 2);
        boost.reset();
        assert_eq!(boost.hits(), 0);
    }

    #[test]
    fn pad_and_rejoin_boosts_stack_with_caps() {
        let (s0, a0) = combine(0.0, 0.0, 0.26, 20.0);
        assert_eq!((s0, a0), (0.0, 0.0));
        let (s_pad, a_pad) = combine(0.0, 1.0, 0.26, 20.0);
        assert_eq!((s_pad, a_pad), (PAD_SPEED_BONUS, PAD_ACCEL));
        let (s_both, a_both) = combine(1.0, 1.0, 0.26, 20.0);
        assert!(s_both > s_pad && s_both <= MAX_SPEED_BONUS);
        assert!(a_both > a_pad && a_both <= MAX_BOOST_ACCEL);
    }
}
