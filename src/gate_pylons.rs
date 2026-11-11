//! Emissive twin pylons joined by a lit arch over every timed gate, baked
//! into one world-space GLB. Visual only: no colliders, so physics never sees them.

use std::path::{Path, PathBuf};

use crate::{config, glb, planet, race};

/// Clearance outside the ribbon edge, past the small edge cones.
const EDGE_GAP: f32 = 1.5;
/// Tall enough that the chase camera (~4.5 m up) always passes under.
const HEIGHT: f32 = 7.0;
const HALF_THICK: f32 = 0.2;
/// Footing sunk into the regolith so uneven ground never shows a gap.
const SINK: f32 = 0.5;

#[derive(Default)]
struct Boxes {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl Boxes {
    /// Axis-aligned box in the (x, y, z) frame around `center`.
    fn push(&mut self, center: glam::Vec3, axes: [glam::Vec3; 3], half: glam::Vec3) {
        let half = half.to_array();
        for (axis, &normal) in axes.iter().enumerate() {
            let (u, v) = (axes[(axis + 1) % 3], axes[(axis + 2) % 3]);
            let (hu, hv) = (half[(axis + 1) % 3], half[(axis + 2) % 3]);
            for sign in [1.0f32, -1.0] {
                let n = normal * sign;
                let c = center + n * half[axis];
                let base = self.positions.len() as u32;
                for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    self.positions
                        .push((c + u * (a * hu) + v * (b * hv)).into());
                    self.normals.push(n.into());
                }
                // Frames may be left-handed, so pick the outward winding per face.
                let p = |k: u32| glam::Vec3::from(self.positions[(base + k) as usize]);
                if (p(1) - p(0)).cross(p(2) - p(0)).dot(n) > 0.0 {
                    self.indices.extend_from_slice(&[
                        base,
                        base + 1,
                        base + 2,
                        base,
                        base + 2,
                        base + 3,
                    ]);
                } else {
                    self.indices.extend_from_slice(&[
                        base,
                        base + 2,
                        base + 1,
                        base,
                        base + 3,
                        base + 2,
                    ]);
                }
            }
        }
    }

    fn finish(self, name: &str, base_color: [f32; 4], emissive: [f32; 3]) -> glb::MeshData {
        glb::MeshData {
            name: name.to_string(),
            tex_coords: vec![[0.5, 0.5]; self.positions.len()],
            positions: self.positions,
            normals: self.normals,
            indices: self.indices,
            base_color,
            metallic: 0.3,
            roughness: 0.45,
            emissive,
        }
    }
}

/// One pylon base per side of every gate: (position on the ground, up, side, tangent).
pub fn footings(
    track: &[planet::TrackSample],
    track_width: f32,
    checkpoint_count: u32,
    ground: impl Fn(glam::Vec3) -> f32,
) -> Vec<(usize, glam::Vec3, [glam::Vec3; 3])> {
    let mut out = Vec::new();
    for (gate, index) in race::checkpoint_samples(track.len(), checkpoint_count)
        .into_iter()
        .enumerate()
    {
        let sample = &track[index];
        let side = sample.normal.cross(sample.tangent).normalize_or_zero();
        for sign in [-1.0f32, 1.0] {
            let rough = sample.position + side * (sign * (track_width * 0.5 + EDGE_GAP));
            let up = rough.normalize_or_zero();
            let foot = up * ground(up);
            let tangent = sample.tangent.reject_from(up).normalize_or_zero();
            // Inner face (+x) points back across the ribbon.
            let across = up.cross(tangent).normalize_or_zero() * -sign;
            out.push((gate, foot, [across, up, tangent]));
        }
    }
    out
}

pub fn write_model(
    out_dir: &Path,
    track: &[planet::TrackSample],
    track_width: f32,
    planet_cfg: config::Planet,
    checkpoint_count: u32,
) -> Option<PathBuf> {
    let dirs: Vec<glam::Vec3> = track.iter().map(|s| s.normal).collect();
    let ground = |up: glam::Vec3| planet::surface_height(up, planet_cfg, &dirs);
    let (mut body, mut glow, mut start) = (Boxes::default(), Boxes::default(), Boxes::default());
    let feet = footings(track, track_width, checkpoint_count, ground);
    for pair in feet.chunks(2) {
        let strips = if pair[0].0 == 0 {
            &mut start
        } else {
            &mut glow
        };
        let mut tops = Vec::new();
        for &(_, foot, axes) in pair {
            let up = axes[1];
            let mid = (HEIGHT - SINK) * 0.5;
            let half = glam::Vec3::new(HALF_THICK, (HEIGHT + SINK) * 0.5, HALF_THICK);
            body.push(foot + up * mid, axes, half);
            // Light strip on the face toward the ribbon.
            strips.push(
                foot + axes[0] * (HALF_THICK + 0.02) + up * (HEIGHT * 0.5),
                axes,
                glam::Vec3::new(0.04, HEIGHT * 0.42, 0.11),
            );
            tops.push(foot + up * HEIGHT);
        }
        if let [a, b] = tops[..] {
            // Crossbar with a glowing underside that reads at speed.
            let center = (a + b) * 0.5;
            let x = (b - a).normalize_or_zero();
            let up = center
                .normalize_or_zero()
                .reject_from(x)
                .normalize_or_zero();
            let axes = [x, up, x.cross(up)];
            let span = (b - a).length() * 0.5;
            body.push(
                center,
                axes,
                glam::Vec3::new(span + HALF_THICK, 0.22, HALF_THICK),
            );
            strips.push(
                center - up * 0.24,
                axes,
                glam::Vec3::new(span - 0.3, 0.03, 0.12),
            );
        }
    }
    if body.positions.is_empty() {
        return None;
    }
    let path = out_dir.join("gate-pylons.glb");
    let meshes = [
        body.finish("gate-body", [0.09, 0.07, 0.07, 1.0], [0.0, 0.0, 0.0]),
        glow.finish("gate-glow", [1.0, 0.55, 0.2, 1.0], [6.0, 2.2, 0.5]),
        start.finish("gate-start", [1.0, 0.25, 0.2, 1.0], [6.5, 0.7, 0.5]),
    ];
    glb::write_glb(&path, &meshes).ok()?;
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pylons_flank_every_gate_off_the_ribbon() {
        let track: Vec<planet::TrackSample> = (0..64)
            .map(|i| {
                let a = i as f32 / 64.0 * std::f32::consts::TAU;
                let position = glam::Vec3::new(a.cos(), 0.0, a.sin()) * 50.0;
                planet::TrackSample {
                    position,
                    tangent: glam::Vec3::new(-a.sin(), 0.0, a.cos()),
                    normal: position.normalize(),
                }
            })
            .collect();
        let feet = footings(&track, 8.0, 16, |_| 50.0);
        assert_eq!(feet.len(), 32);
        for (gate, foot, axes) in feet {
            let query = planet::query_track(foot, &track);
            assert!(
                planet::off_track_distance(&query, 8.0) > 1.0,
                "gate {gate} sits off-ribbon"
            );
            // The strip face points back toward the ribbon centre.
            let to_track = (track[query.index].position - foot).reject_from(axes[1]);
            assert!(axes[0].dot(to_track) > 0.0);
        }
    }
}
