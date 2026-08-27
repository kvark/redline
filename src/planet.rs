use std::{
    collections::HashMap,
    f32::consts,
    path::{Path, PathBuf},
};

use crate::{config, glb};

const GOLDEN_ANGLE: f32 = 2.399_963_2;
pub const SUN_DIRECTION: glam::Vec3 = glam::Vec3::new(0.45, 0.72, 0.28);

#[derive(Clone, Copy)]
pub struct TrackSample {
    pub position: glam::Vec3,
    pub tangent: glam::Vec3,
    pub normal: glam::Vec3,
}

#[derive(Clone, Copy)]
pub struct TrackQuery {
    pub index: usize,
    pub sample: TrackSample,
    pub tangent: glam::Vec3,
    pub side: glam::Vec3,
    pub lateral: f32,
    pub radial_error: f32,
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
    pub collider_points: Vec<glam::Vec3>,
    pub kind: DecorationKind,
    /// Linear RGB intensity if this decoration is a glowing crystal.
    pub glow: Option<[f32; 3]>,
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
    let mut track_weights = Vec::with_capacity(positions.len());
    for dir in positions.iter().copied() {
        let (height, track_weight) = sample_height(dir, config, &track_dirs);
        displaced.push(dir * height);
        track_weights.push(track_weight);
    }

    let mut dust = MeshBuilder::new("ochre-dust");
    let mut lowlands = MeshBuilder::new("shadowed-lowlands");
    let mut basalt = MeshBuilder::new("basalt-highlands");
    let mut iron = MeshBuilder::new("iron-outcrops");
    let mut track_mesh = MeshBuilder::new("mars-track");
    for face in faces.iter() {
        let a = displaced[face[0] as usize];
        let b = displaced[face[1] as usize];
        let c = displaced[face[2] as usize];
        let trackish = (track_weights[face[0] as usize] > 0.45) as u8
            + (track_weights[face[1] as usize] > 0.45) as u8
            + (track_weights[face[2] as usize] > 0.45) as u8
            >= 2;
        if trackish {
            track_mesh.push_triangle(a, b, c, config.radius);
        } else {
            let center = (a + b + c) / 3.0;
            let relative_height = (center.length() - config.radius) / config.height_amp;
            let material_noise = fbm(
                center.normalize_or_zero() * 10.0 + seed_offset(config.seed ^ 0x51A7),
                3,
            );
            if relative_height < -0.16 {
                lowlands.push_triangle(a, b, c, config.radius);
            } else if relative_height > 0.48 {
                basalt.push_triangle(a, b, c, config.radius);
            } else if material_noise > 0.68 {
                iron.push_triangle(a, b, c, config.radius);
            } else {
                dust.push_triangle(a, b, c, config.radius);
            }
        }
    }

    let planet_model = out_dir.join("mars.glb");
    let mut surface_meshes = Vec::new();
    for (mesh, look) in [
        (dust, ([0.43, 0.19, 0.10, 1.0], 0.0, 0.96)),
        (lowlands, ([0.15, 0.075, 0.065, 1.0], 0.02, 0.98)),
        (basalt, ([0.095, 0.085, 0.09, 1.0], 0.06, 0.88)),
        (iron, ([0.31, 0.105, 0.055, 1.0], 0.12, 0.74)),
        (track_mesh, ([0.27, 0.12, 0.075, 1.0], 0.0, 0.82)),
    ] {
        if !mesh.is_empty() {
            surface_meshes.push(mesh.finish(look.0, look.1, look.2, [0.0; 3]));
        }
    }
    glb::write_glb(&planet_model, &surface_meshes).expect("failed to write planet glb");

    let stone_models = write_stone_models(out_dir, config.seed);
    let spire_models = write_spire_models(out_dir, config.seed);
    let crystal_models = write_crystal_models(out_dir, config.seed);
    let track = sample_track_surface(&track_dirs, config);
    let decorations = place_decorations(
        config,
        &track,
        &stone_models,
        &spire_models,
        &crystal_models,
    );
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

pub fn query_track(position: glam::Vec3, track: &[TrackSample]) -> TrackQuery {
    let (index, _) = track_progress(position, track);
    let sample = track[index];
    let next = track[(index + 1) % track.len().max(1)];
    let mut tangent = (next.position - sample.position).normalize_or_zero();
    if tangent.length_squared() < 1e-6 {
        tangent = sample.tangent;
    }
    let side = sample.normal.cross(tangent).normalize_or_zero();
    let delta = position - sample.position;
    TrackQuery {
        index,
        sample,
        tangent,
        side,
        lateral: delta.dot(side),
        radial_error: delta.dot(sample.normal),
    }
}

pub fn off_track_distance(query: &TrackQuery, track_width: f32) -> f32 {
    query.lateral.abs() - track_width * 0.5
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
    let continental = fbm(dir * 1.65 + seed_offset(config.seed), 5);
    let broad = fbm(dir * 1.10 + seed_offset(config.seed ^ 0x7137_1A11), 3);
    let rolling = fbm(dir * 4.8 + seed_offset(config.seed ^ 0x9E37), 4);
    let detail = fbm(dir * 13.0 + seed_offset(config.seed ^ 0xC0FF_EE11), 3);
    let ridges = 1.0 - (rolling * 2.0 - 1.0).abs();
    let mesa = smoothstep(0.58, 0.76, continental) * (0.35 + 0.65 * rolling);
    let craters = crater_relief(dir, config.seed);
    let raw = config.radius
        + config.height_amp
            * (1.15 * (continental - 0.48)
                + 0.42 * (rolling - 0.5)
                + 0.30 * (detail - 0.5)
                + 0.44 * ridges.powi(4)
                + 0.52 * mesa
                + craters);
    let dist = angular_distance_to_polyline(dir, track) * config.radius;
    let half = config.track_width * 0.5;
    let track_weight = 1.0 - smoothstep(half * 0.62, half * 1.08, dist);
    // A wide runoff keeps leaving the road from becoming a cliff of FBM ridges.
    let shoulder_weight = 1.0 - smoothstep(half * 1.0, half * 3.6, dist);
    let track_height = config.radius + config.height_amp * (0.68 * (broad - 0.48) + 0.10);
    let runoff = config.radius
        + config.height_amp
            * (1.02 * (continental - 0.48) + 0.26 * (rolling - 0.5) + 0.16 * mesa + 0.32 * craters);
    let height = lerp(
        lerp(raw, runoff, shoulder_weight),
        track_height,
        track_weight,
    );
    (height, track_weight)
}

fn crater_relief(dir: glam::Vec3, seed: u32) -> f32 {
    let mut relief = 0.0f32;
    for i in 0..18u32 {
        let h0 = hash_u32(seed ^ i.wrapping_mul(0x9E37_79B9));
        let h1 = hash_u32(h0 ^ 0x85EB_CA6B);
        let y = hash01(h0) * 2.0 - 1.0;
        let radial = (1.0 - y * y).max(0.0).sqrt();
        let azimuth = hash01(h1) * consts::TAU;
        let center = glam::Vec3::new(radial * azimuth.cos(), y, radial * azimuth.sin());
        let radius = 0.045 + 0.075 * hash01(h1 ^ 0xC2B2_AE35);
        let d = angular_distance(dir, center) / radius;
        let scale = 0.45 + 0.55 * hash01(h0 ^ 0x27D4_EB2F);
        let bowl = if d < 1.0 {
            -0.72 * (1.0 - d * d).powi(2)
        } else {
            0.0
        };
        let rim_distance = (d - 1.0).abs();
        let rim = if rim_distance < 0.28 {
            0.34 * (1.0 - rim_distance / 0.28).powi(2)
        } else {
            0.0
        };
        relief += (bowl + rim) * scale;
    }
    relief
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
    spires: &[PathBuf],
    crystals: &[PathBuf],
) -> Vec<Decoration> {
    let track_dirs: Vec<glam::Vec3> = track.iter().map(|s| s.normal).collect();
    let keep_angle = (config.track_width * 1.05) / config.radius;
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
        let is_crystal = (rng & 0xFF) < 38;
        let tilt = ((rng >> 8) as f32 / 16_777_215.0 - 0.5) * 0.55;
        let spin = ((rng >> 16) as f32 / 65_535.0) * consts::TAU;
        let scale = if is_crystal {
            3.5 + ((rng >> 4) & 0xFF) as f32 / 255.0 * 9.0
        } else {
            1.6 + ((rng >> 4) & 0xFF) as f32 / 255.0 * 7.5
        };
        let tangent = orthonormal(dir);
        let tilted = (dir + tangent * tilt).normalize();
        let orientation = surface_quat(tilted, rotate_around(tangent, tilted, spin));
        let model = if is_crystal {
            crystals[i % crystals.len()].clone()
        } else {
            stones[i % stones.len()].clone()
        };
        let (collider_points, lift) = if is_crystal {
            (crystal_collider(scale), 0.35 * scale)
        } else {
            (elongated_collider(scale, 1.0), 1.85 * scale)
        };
        let kind = if is_crystal {
            DecorationKind::Crystal
        } else {
            DecorationKind::Stone
        };
        let glow = if is_crystal {
            Some(match i % 3 {
                0 => [0.35, 2.8, 3.6],
                1 => [3.8, 0.45, 0.28],
                _ => [0.55, 2.6, 0.7],
            })
        } else {
            None
        };
        out.push(Decoration {
            model,
            position: dir * height + tilted * lift,
            orientation,
            scale,
            collider_points,
            kind,
            glow,
        });
    }

    // Deliberate rows of narrow monoliths give the road a threatening silhouette.
    // Each one remains outside the driving line but leans upward and slightly inward.
    for (row, sample) in track.iter().step_by(8).enumerate() {
        let side = sample.normal.cross(sample.tangent).normalize_or_zero();
        for (side_index, sign) in [-1.0f32, 1.0].into_iter().enumerate() {
            let h = hash_u32(
                config.seed
                    ^ (row as u32).wrapping_mul(0x9E37_79B9)
                    ^ (side_index as u32).wrapping_mul(0x85EB_CA6B),
            );
            let offset = config.track_width * 0.5 + 9.0 + hash01(h) * 4.0;
            let point_dir = (sample.position + side * sign * offset).normalize_or_zero();
            let (height, _) = sample_height(point_dir, config, &track_dirs);
            let inward = -side * sign;
            let along = sample.tangent * (hash01(h ^ 0xC2B2_AE35) - 0.5) * 0.22;
            let spike_up = (point_dir + inward * (0.28 + hash01(h ^ 0x27D4_EB2F) * 0.28) + along)
                .normalize_or_zero();
            let scale = 3.8 + hash01(h ^ 0x1656_67B1) * 3.7;
            out.push(Decoration {
                model: spires[(row + side_index) % spires.len()].clone(),
                position: point_dir * height + spike_up * (scale * 1.85),
                orientation: surface_quat(spike_up, sample.tangent),
                scale,
                collider_points: elongated_collider(scale, 0.34),
                kind: DecorationKind::Stone,
                glow: None,
            });
        }
    }
    out
}

fn elongated_collider(scale: f32, width: f32) -> Vec<glam::Vec3> {
    let mut points = Vec::with_capacity(26);
    points.push(glam::Vec3::Y * (1.82 * scale));
    points.push(-glam::Vec3::Y * (1.82 * scale));
    for (y, radius) in [(-0.82f32, 0.64f32), (0.0, 0.78), (0.82, 0.64)] {
        for side in 0..8 {
            let angle = side as f32 / 8.0 * consts::TAU;
            points.push(
                glam::Vec3::new(
                    angle.cos() * radius * width,
                    y,
                    angle.sin() * radius * width,
                ) * scale,
            );
        }
    }
    points
}

fn crystal_collider(scale: f32) -> Vec<glam::Vec3> {
    let mut points = vec![glam::Vec3::Y * scale, -glam::Vec3::Y * (0.35 * scale)];
    for side in 0..6 {
        let angle = side as f32 / 6.0 * consts::TAU;
        points.push(glam::Vec3::new(angle.cos() * 0.21, 0.12, angle.sin() * 0.21) * scale);
    }
    points
}

fn write_stone_models(out_dir: &Path, seed: u32) -> Vec<PathBuf> {
    let colors = [
        [0.18, 0.12, 0.11, 1.0],
        [0.095, 0.085, 0.09, 1.0],
        [0.28, 0.12, 0.075, 1.0],
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
                1.0,
            );
            glb::write_glb(&path, &[mesh]).expect("stone glb");
            path
        })
        .collect()
}

fn write_spire_models(out_dir: &Path, seed: u32) -> Vec<PathBuf> {
    let colors = [
        [0.105, 0.085, 0.082, 1.0],
        [0.16, 0.075, 0.055, 1.0],
        [0.075, 0.072, 0.078, 1.0],
    ];
    colors
        .iter()
        .enumerate()
        .map(|(i, color)| {
            let path = out_dir.join(format!("spire-{i}.glb"));
            let mesh = elongated_rock(
                seed.wrapping_add(0x5A11).wrapping_add((i as u32) * 43),
                *color,
                false,
                0.34,
            );
            glb::write_glb(&path, &[mesh]).expect("spire glb");
            path
        })
        .collect()
}

fn write_crystal_models(out_dir: &Path, seed: u32) -> Vec<PathBuf> {
    let looks = [
        ([0.16, 0.36, 0.42, 1.0], [0.18, 1.35, 1.70]),
        ([0.44, 0.055, 0.035, 1.0], [2.40, 0.16, 0.08]),
        ([0.22, 0.34, 0.12, 1.0], [0.55, 1.40, 0.22]),
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

fn elongated_rock(seed: u32, color: [f32; 4], crystal: bool, width: f32) -> glb::MeshData {
    let (verts, faces) = icosphere(1);
    let stretch = if crystal { 2.6 } else { 2.1 };
    let mut builder = MeshBuilder::new("rock");
    let jitter = seed_offset(seed);
    for face in faces.iter() {
        let mut tri = [glam::Vec3::ZERO; 3];
        for (k, index) in face.iter().copied().enumerate() {
            let mut p = verts[index as usize];
            p.y *= stretch;
            p.x *= width;
            p.z *= width;
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

    fn is_empty(&self) -> bool {
        self.indices.is_empty()
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
    fn terrain_and_track_have_broad_elevation_changes() {
        let cfg = config::Planet::default();
        let track = build_track_dirs(256, cfg.track_lat_amp);
        let track_heights = track
            .iter()
            .map(|&dir| sample_height(dir, cfg, &track).0)
            .collect::<Vec<_>>();
        let track_range = track_heights.iter().copied().fold(f32::MIN, f32::max)
            - track_heights.iter().copied().fold(f32::MAX, f32::min);
        assert!(track_range > 1.0, "track should climb and descend");
        let max_step = track_heights
            .iter()
            .zip(track_heights.iter().cycle().skip(1))
            .take(track_heights.len())
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(
            max_step < 0.8,
            "track elevation changed {max_step:.2}m between adjacent samples"
        );

        let terrain_heights = (0..256)
            .map(|i| {
                let y = 1.0 - (i as f32 + 0.5) / 256.0 * 2.0;
                let radial = (1.0 - y * y).sqrt();
                let theta = super::GOLDEN_ANGLE * i as f32;
                let dir = glam::Vec3::new(theta.cos() * radial, y, theta.sin() * radial);
                sample_height(dir, cfg, &track).0
            })
            .collect::<Vec<_>>();
        let terrain_range = terrain_heights.iter().copied().fold(f32::MIN, f32::max)
            - terrain_heights.iter().copied().fold(f32::MAX, f32::min);
        assert!(
            terrain_range > cfg.height_amp,
            "terrain relief was only {terrain_range:.2}m"
        );
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
        let spires = vec![std::path::PathBuf::from("spire")];
        let placed = super::place_decorations(cfg, &track, &stones, &spires, &crystals);
        assert!(!placed.is_empty());
        assert!(placed.len() < cfg.decoration_count as usize + track.len());
        let placed_spires = placed
            .iter()
            .filter(|deco| deco.model == spires[0])
            .collect::<Vec<_>>();
        assert_eq!(placed_spires.len(), 2 * track.len().div_ceil(8));
        for spire in placed_spires {
            let up = spire.orientation * glam::Vec3::Y;
            let radial = spire.position.normalize_or_zero();
            let upward_lean = up.dot(radial);
            assert!(upward_lean > 0.75 && upward_lean < 0.99);
        }
    }
}
