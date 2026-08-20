use std::{
    collections::HashMap,
    f32::consts,
    path::{Path, PathBuf},
};

use crate::{config, glb};

const GOLDEN_ANGLE: f32 = 2.399_963_2;

#[derive(Clone, Copy)]
pub struct TrackSample {
    pub position: glam::Vec3,
    pub tangent: glam::Vec3,
    pub normal: glam::Vec3,
}

pub struct GeneratedPlanet {
    pub radius: f32,
    pub track_width: f32,
    pub track: Vec<TrackSample>,
    pub planet_model: PathBuf,
    pub decorations: Vec<Decoration>,
}

#[derive(Clone)]
pub struct Decoration {
    pub model: PathBuf,
    pub position: glam::Vec3,
    pub orientation: glam::Quat,
    pub scale: f32,
    pub half_extents: glam::Vec3,
    pub kind: DecorationKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DecorationKind {
    Stone,
    Crystal,
}

pub fn generate(config: config::Planet, out_dir: &Path) -> GeneratedPlanet {
    let _ = std::fs::create_dir_all(out_dir);
    let (positions, faces) = icosphere(config.subdivisions);
    let track_dirs = build_track_dirs(256, config.track_lat_amp);

    let mut displaced = Vec::with_capacity(positions.len());
    let mut on_track = Vec::with_capacity(positions.len());
    for dir in positions.iter().copied() {
        let (height, track_weight) = sample_height(dir, config, &track_dirs);
        displaced.push(dir * height);
        on_track.push(track_weight > 0.45);
    }

    let mut terrain = MeshBuilder::new("mars-terrain");
    let mut track_mesh = MeshBuilder::new("mars-track");
    for face in faces.iter() {
        let a = displaced[face[0] as usize];
        let b = displaced[face[1] as usize];
        let c = displaced[face[2] as usize];
        let trackish = on_track[face[0] as usize] as u8
            + on_track[face[1] as usize] as u8
            + on_track[face[2] as usize] as u8
            >= 2;
        if trackish {
            track_mesh.push_triangle(a, b, c, config.radius);
        } else {
            terrain.push_triangle(a, b, c, config.radius);
        }
    }

    let planet_model = out_dir.join("mars.glb");
    glb::write_glb(
        &planet_model,
        &[
            terrain.finish([0.62, 0.30, 0.17, 1.0], 0.02, 0.92, [0.0; 3]),
            track_mesh.finish([0.38, 0.18, 0.11, 1.0], 0.0, 0.78, [0.0; 3]),
        ],
    )
    .expect("failed to write planet glb");

    let stone_models = write_stone_models(out_dir, config.seed);
    let crystal_models = write_crystal_models(out_dir, config.seed);
    let track = sample_track_surface(&track_dirs, config);
    let decorations = place_decorations(config, &track, &stone_models, &crystal_models);

    GeneratedPlanet {
        radius: config.radius,
        track_width: config.track_width,
        track,
        planet_model,
        decorations,
    }
}

pub fn track_progress(position: glam::Vec3, track: &[TrackSample]) -> (usize, f32) {
    let dir = position.normalize_or_zero();
    let mut best_i = 0usize;
    let mut best_dot = -1.0f32;
    for (index, sample) in track.iter().enumerate() {
        let dot = dir.dot(sample.normal);
        if dot > best_dot {
            best_dot = dot;
            best_i = index;
        }
    }
    (best_i, best_i as f32 / track.len().max(1) as f32)
}

fn build_track_dirs(count: usize, lat_amp: f32) -> Vec<glam::Vec3> {
    (0..count)
        .map(|i| {
            let t = i as f32 / count as f32;
            let lon = t * consts::TAU;
            let lat = lat_amp * (2.0 * lon).sin() + 0.08 * lat_amp * (5.0 * lon).sin();
            spherical(lat, lon)
        })
        .collect()
}

fn sample_track_surface(dirs: &[glam::Vec3], config: config::Planet) -> Vec<TrackSample> {
    let mut samples = Vec::with_capacity(dirs.len());
    for (index, dir) in dirs.iter().copied().enumerate() {
        let (height, _) = sample_height(dir, config, dirs);
        let next = dirs[(index + 1) % dirs.len()];
        let position = dir * height;
        let tangent = (next * height - position).normalize_or_zero();
        samples.push(TrackSample {
            position,
            tangent,
            normal: dir,
        });
    }
    samples
}

fn sample_height(dir: glam::Vec3, config: config::Planet, track: &[glam::Vec3]) -> (f32, f32) {
    let n1 = fbm(dir * 3.1 + seed_offset(config.seed), 5);
    let n2 = fbm(dir * 7.4 + seed_offset(config.seed ^ 0x9E37), 4);
    let ridges = 1.0 - (n1 * 2.0 - 1.0).abs();
    let raw = config.radius + config.height_amp * (0.55 * n1 + 0.30 * n2 + 0.25 * ridges * ridges);
    let dist = angular_distance_to_polyline(dir, track) * config.radius;
    let half = config.track_width * 0.5;
    let track_weight = 1.0 - smoothstep(half * 0.55, half, dist);
    let track_height = config.radius + config.height_amp * 0.08;
    let height = lerp(raw, track_height, track_weight);
    (height, track_weight)
}

fn angular_distance_to_polyline(dir: glam::Vec3, track: &[glam::Vec3]) -> f32 {
    let mut best = consts::PI;
    for window in track.windows(2) {
        best = best.min(angular_distance_to_segment(dir, window[0], window[1]));
    }
    if let (Some(first), Some(last)) = (track.first(), track.last()) {
        best = best.min(angular_distance_to_segment(dir, *last, *first));
    }
    best
}

fn angular_distance_to_segment(p: glam::Vec3, a: glam::Vec3, b: glam::Vec3) -> f32 {
    // Project p onto the great-circle arc from a to b.
    let n = a.cross(b);
    if n.length_squared() < 1e-8 {
        return angular_distance(p, a);
    }
    let n = n.normalize();
    let closest = (p - n * p.dot(n)).normalize_or_zero();
    let ab = angular_distance(a, b);
    let ac = angular_distance(a, closest);
    let cb = angular_distance(closest, b);
    if (ac + cb - ab).abs() < 0.05 {
        angular_distance(p, closest)
    } else {
        angular_distance(p, a).min(angular_distance(p, b))
    }
}

fn angular_distance(a: glam::Vec3, b: glam::Vec3) -> f32 {
    a.dot(b).clamp(-1.0, 1.0).acos()
}

fn place_decorations(
    config: config::Planet,
    track: &[TrackSample],
    stones: &[PathBuf],
    crystals: &[PathBuf],
) -> Vec<Decoration> {
    let track_dirs: Vec<glam::Vec3> = track.iter().map(|s| s.normal).collect();
    let keep_angle = (config.track_width * 0.62) / config.radius;
    let mut out = Vec::new();
    let count = config.decoration_count as usize;
    for i in 0..count {
        let y = 1.0 - (i as f32 / (count - 1).max(1) as f32) * 2.0;
        let radius = (1.0 - y * y).max(0.0).sqrt();
        let theta = GOLDEN_ANGLE * i as f32;
        let dir = glam::Vec3::new(theta.cos() * radius, y, theta.sin() * radius).normalize();
        let dist = angular_distance_to_polyline(dir, &track_dirs);
        if dist < keep_angle {
            continue;
        }
        let (height, _) = sample_height(dir, config, &track_dirs);
        let rng = hash_u32(config.seed.wrapping_add((i as u32).wrapping_mul(747796405)));
        let is_crystal = (rng & 0xFF) < 80;
        let tilt = ((rng >> 8) as f32 / 16_777_215.0 - 0.5) * 0.55;
        let spin = ((rng >> 16) as f32 / 65_535.0) * consts::TAU;
        let scale = if is_crystal {
            2.4 + ((rng >> 4) & 0xFF) as f32 / 255.0 * 6.5
        } else {
            1.8 + ((rng >> 4) & 0xFF) as f32 / 255.0 * 5.2
        };
        let tangent = orthonormal(dir);
        let tilted = (dir + tangent * tilt).normalize();
        let orientation = surface_quat(tilted, rotate_around(tangent, tilted, spin));
        let model = if is_crystal {
            crystals[i % crystals.len()].clone()
        } else {
            stones[i % stones.len()].clone()
        };
        let half = if is_crystal {
            glam::Vec3::new(0.18 * scale, 0.55 * scale, 0.18 * scale)
        } else {
            glam::Vec3::new(0.28 * scale, 0.48 * scale, 0.24 * scale)
        };
        out.push(Decoration {
            model,
            position: dir * (height + half.y * 0.35),
            orientation,
            scale,
            half_extents: half,
            kind: if is_crystal {
                DecorationKind::Crystal
            } else {
                DecorationKind::Stone
            },
        });
    }
    out
}

fn write_stone_models(out_dir: &Path, seed: u32) -> Vec<PathBuf> {
    let colors = [
        [0.32, 0.18, 0.13, 1.0],
        [0.24, 0.14, 0.11, 1.0],
        [0.40, 0.22, 0.14, 1.0],
    ];
    colors
        .iter()
        .enumerate()
        .map(|(i, color)| {
            let path = out_dir.join(format!("stone-{i}.glb"));
            let mesh = elongated_rock(
                seed.wrapping_add((i as u32).wrapping_mul(17)),
                *color,
                false,
            );
            glb::write_glb(&path, &[mesh]).expect("stone glb");
            path
        })
        .collect()
}

fn write_crystal_models(out_dir: &Path, seed: u32) -> Vec<PathBuf> {
    let looks = [
        ([0.35, 0.75, 0.95, 1.0], [0.08, 0.35, 0.55]),
        ([0.78, 0.28, 0.95, 1.0], [0.35, 0.08, 0.45]),
        ([0.95, 0.82, 0.35, 1.0], [0.40, 0.22, 0.04]),
    ];
    looks
        .iter()
        .enumerate()
        .map(|(i, &(color, emit))| {
            let path = out_dir.join(format!("crystal-{i}.glb"));
            let mesh = crystal_spike(seed.wrapping_add((i as u32).wrapping_mul(29)), color, emit);
            glb::write_glb(&path, &[mesh]).expect("crystal glb");
            path
        })
        .collect()
}

fn elongated_rock(seed: u32, color: [f32; 4], crystal: bool) -> glb::MeshData {
    let (verts, faces) = icosphere(1);
    let stretch = if crystal { 2.6 } else { 2.1 };
    let mut builder = MeshBuilder::new("rock");
    let jitter = seed_offset(seed);
    for face in faces.iter() {
        let mut tri = [glam::Vec3::ZERO; 3];
        for (k, index) in face.iter().copied().enumerate() {
            let mut p = verts[index as usize];
            p.y *= stretch;
            let warp = fbm(p * 2.4 + jitter, 3);
            p *= 0.55 + 0.45 * warp;
            p.y *= 1.15;
            tri[k] = p;
        }
        builder.push_triangle(tri[0], tri[1], tri[2], 1.0);
    }
    if crystal {
        builder.finish(color, 0.15, 0.22, [color[0], color[1], color[2]])
    } else {
        builder.finish(color, 0.04, 0.95, [0.0; 3])
    }
}

fn crystal_spike(seed: u32, color: [f32; 4], emit: [f32; 3]) -> glb::MeshData {
    let sides = 6usize;
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    let rng = hash_u32(seed) as f32 / u32::MAX as f32;
    let twist = rng * 0.4;
    let mid_y = 0.12;
    positions.push([0.0, 1.0, 0.0]);
    positions.push([0.0, -0.35, 0.0]);
    for i in 0..sides {
        let a = i as f32 / sides as f32 * consts::TAU + twist;
        let r = 0.22 + 0.05 * ((i as f32) * 1.7).sin();
        positions.push([a.cos() * r, mid_y, a.sin() * r]);
    }
    for i in 0..sides {
        let a = 2 + i as u32;
        let b = 2 + ((i + 1) % sides) as u32;
        indices.extend_from_slice(&[0, a, b]);
        indices.extend_from_slice(&[1, b, a]);
    }
    let normals = compute_normals(&positions, &indices);
    let tex_coords = positions
        .iter()
        .map(|p| {
            let dir = glam::Vec3::from(*p).normalize_or_zero();
            [
                0.5 + 0.5 * dir.x.atan2(dir.z) / consts::PI,
                0.5 - 0.5 * dir.y,
            ]
        })
        .collect();
    glb::MeshData {
        name: "crystal".into(),
        positions,
        normals,
        tex_coords,
        indices,
        base_color: color,
        metallic: 0.18,
        roughness: 0.18,
        emissive: emit,
    }
}

struct MeshBuilder {
    name: String,
    positions: Vec<[f32; 3]>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            positions: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn push_triangle(&mut self, a: glam::Vec3, b: glam::Vec3, c: glam::Vec3, _ref_radius: f32) {
        let normal = (b - a).cross(c - a);
        let center = (a + b + c) / 3.0;
        let (a, b, c) = if normal.dot(center) < 0.0 {
            (a, c, b)
        } else {
            (a, b, c)
        };
        let base = self.positions.len() as u32;
        self.positions.push(a.into());
        self.positions.push(b.into());
        self.positions.push(c.into());
        self.indices.extend_from_slice(&[base, base + 1, base + 2]);
    }

    fn finish(
        self,
        base_color: [f32; 4],
        metallic: f32,
        roughness: f32,
        emissive: [f32; 3],
    ) -> glb::MeshData {
        let normals = compute_normals(&self.positions, &self.indices);
        let tex_coords = self
            .positions
            .iter()
            .map(|p| {
                let dir = glam::Vec3::from(*p).normalize_or_zero();
                [
                    0.5 + 0.5 * dir.x.atan2(dir.z) / consts::PI,
                    0.5 - dir.y * 0.5,
                ]
            })
            .collect();
        glb::MeshData {
            name: self.name,
            positions: self.positions,
            normals,
            tex_coords,
            indices: self.indices,
            base_color,
            metallic,
            roughness,
            emissive,
        }
    }
}

fn compute_normals(positions: &[[f32; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![glam::Vec3::ZERO; positions.len()];
    for tri in indices.chunks_exact(3) {
        let a = glam::Vec3::from(positions[tri[0] as usize]);
        let b = glam::Vec3::from(positions[tri[1] as usize]);
        let c = glam::Vec3::from(positions[tri[2] as usize]);
        let n = (b - a).cross(c - a);
        normals[tri[0] as usize] += n;
        normals[tri[1] as usize] += n;
        normals[tri[2] as usize] += n;
    }
    normals
        .into_iter()
        .map(|n| n.normalize_or_zero().into())
        .collect()
}

fn icosphere(subdivisions: u32) -> (Vec<glam::Vec3>, Vec<[u32; 3]>) {
    let t = (1.0 + 5.0f32.sqrt()) * 0.5;
    let raw = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ];
    let mut verts: Vec<glam::Vec3> = raw
        .iter()
        .map(|p| glam::Vec3::from(*p).normalize())
        .collect();
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        let mut midpoints = HashMap::new();
        let mut next = Vec::with_capacity(faces.len() * 4);
        for face in faces.iter() {
            let a = midpoint(&mut verts, &mut midpoints, face[0], face[1]);
            let b = midpoint(&mut verts, &mut midpoints, face[1], face[2]);
            let c = midpoint(&mut verts, &mut midpoints, face[2], face[0]);
            next.push([face[0], a, c]);
            next.push([face[1], b, a]);
            next.push([face[2], c, b]);
            next.push([a, b, c]);
        }
        faces = next;
    }
    (verts, faces)
}

fn midpoint(
    verts: &mut Vec<glam::Vec3>,
    cache: &mut HashMap<(u32, u32), u32>,
    a: u32,
    b: u32,
) -> u32 {
    let key = if a < b { (a, b) } else { (b, a) };
    if let Some(&index) = cache.get(&key) {
        return index;
    }
    let mid = (verts[a as usize] + verts[b as usize]).normalize();
    let index = verts.len() as u32;
    verts.push(mid);
    cache.insert(key, index);
    index
}

pub fn surface_quat(normal: glam::Vec3, tangent: glam::Vec3) -> glam::Quat {
    let y = normal.normalize_or_zero();
    let z = (tangent - y * tangent.dot(y)).normalize_or_zero();
    let z = if z.length_squared() < 1e-6 {
        orthonormal(y)
    } else {
        z
    };
    let x = y.cross(z).normalize_or_zero();
    let z = x.cross(y).normalize_or_zero();
    glam::Quat::from_mat3(&glam::Mat3::from_cols(x, y, z))
}

fn orthonormal(n: glam::Vec3) -> glam::Vec3 {
    let axis = if n.y.abs() < 0.9 {
        glam::Vec3::Y
    } else {
        glam::Vec3::X
    };
    n.cross(axis).normalize_or_zero()
}

fn rotate_around(v: glam::Vec3, axis: glam::Vec3, angle: f32) -> glam::Vec3 {
    glam::Quat::from_axis_angle(axis.normalize_or_zero(), angle) * v
}

fn spherical(lat: f32, lon: f32) -> glam::Vec3 {
    let (sl, cl) = lat.sin_cos();
    let (so, co) = lon.sin_cos();
    glam::Vec3::new(cl * so, sl, cl * co)
}

fn seed_offset(seed: u32) -> glam::Vec3 {
    glam::Vec3::new(
        hash01(seed),
        hash01(seed ^ 0xA341_316C),
        hash01(seed ^ 0xC801_3EA4),
    ) * 17.0
}

fn fbm(p: glam::Vec3, octaves: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 1.0;
    let mut norm = 0.0;
    for _ in 0..octaves {
        sum += amp * value_noise(p * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.07;
    }
    (sum / norm).clamp(0.0, 1.0)
}

fn value_noise(p: glam::Vec3) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (3.0 - 2.0 * f);
    let p000 = hash01_vec(i);
    let p100 = hash01_vec(i + glam::Vec3::X);
    let p010 = hash01_vec(i + glam::Vec3::Y);
    let p110 = hash01_vec(i + glam::Vec3::X + glam::Vec3::Y);
    let p001 = hash01_vec(i + glam::Vec3::Z);
    let p101 = hash01_vec(i + glam::Vec3::X + glam::Vec3::Z);
    let p011 = hash01_vec(i + glam::Vec3::Y + glam::Vec3::Z);
    let p111 = hash01_vec(i + glam::Vec3::ONE);
    let x00 = lerp(p000, p100, u.x);
    let x10 = lerp(p010, p110, u.x);
    let x01 = lerp(p001, p101, u.x);
    let x11 = lerp(p011, p111, u.x);
    let y0 = lerp(x00, x10, u.y);
    let y1 = lerp(x01, x11, u.y);
    lerp(y0, y1, u.z)
}

fn hash01_vec(p: glam::Vec3) -> f32 {
    let ix = p.x.floor() as i32;
    let iy = p.y.floor() as i32;
    let iz = p.z.floor() as i32;
    hash01(
        (ix as u32)
            .wrapping_mul(374761393)
            .wrapping_add((iy as u32).wrapping_mul(668265263))
            .wrapping_add((iz as u32).wrapping_mul(2147483647)),
    )
}

fn hash_u32(x: u32) -> u32 {
    let mut x = x.wrapping_add(0x9E37_79B9);
    x = (x ^ (x >> 16)).wrapping_mul(0x7FEB_352D);
    x = (x ^ (x >> 15)).wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

fn hash01(x: u32) -> f32 {
    (hash_u32(x) >> 8) as f32 / 16_777_216.0
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::{angular_distance_to_polyline, build_track_dirs, icosphere, sample_height};
    use crate::config;

    #[test]
    fn icosphere_counts() {
        let (verts, faces) = icosphere(0);
        assert_eq!(verts.len(), 12);
        assert_eq!(faces.len(), 20);
        let (verts, faces) = icosphere(2);
        assert!(verts.len() > 12);
        assert_eq!(faces.len(), 20 * 16);
    }

    #[test]
    fn track_is_cleared() {
        let cfg = config::Planet::default();
        let dirs = build_track_dirs(128, cfg.track_lat_amp);
        let on_path = dirs[0];
        let (_, weight) = sample_height(on_path, cfg, &dirs);
        assert!(
            weight > 0.8,
            "track center should be flattened, got {weight}"
        );
        let pole = glam::Vec3::Y;
        let dist = angular_distance_to_polyline(pole, &dirs);
        assert!(dist > 0.5);
    }

    #[test]
    fn decorations_do_not_overflow() {
        let cfg = config::Planet::default();
        let track = build_track_dirs(64, cfg.track_lat_amp)
            .into_iter()
            .map(|normal| super::TrackSample {
                position: normal * cfg.radius,
                tangent: super::orthonormal(normal),
                normal,
            })
            .collect::<Vec<_>>();
        let stones = vec![std::path::PathBuf::from("stone")];
        let crystals = vec![std::path::PathBuf::from("crystal")];
        let placed = super::place_decorations(cfg, &track, &stones, &crystals);
        assert!(!placed.is_empty());
        assert!(placed.len() < cfg.decoration_count as usize);
    }
}
