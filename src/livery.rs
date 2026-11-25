//! Team liveries: each racer id gets one original team with bold color
//! blocks, a big team mark, a race number and a katakana/kanji word.
//!
//! The art is vector shapes (convex polygons) clipped straight into the
//! body's paint triangles, then emitted as a thin untextured shell over the
//! stock body: one mesh per team color, the same plain-material path as the
//! other generated models, so no new texture, shader or blend mode reaches
//! WebGL2 and edges stay crisp at any distance. Visual only: collider and mass
//! come from `vehicle.ron` and the kit, never from this mesh.
//!
//! Color grammar (see #44): amber means edge/limit and cyan means boost/go, so
//! no team color sits in either hue band (`tests::teams_avoid_amber_and_cyan`).

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Mutex,
};

use glam::Vec2;

use crate::glb;

type Rgb = [u8; 3];

const PAPER: Rgb = [242, 240, 234];
const INK: Rgb = [16, 16, 20];
/// Kazan Heavy's pinstripe accent.
const CRIMSON: Rgb = [214, 22, 52];

pub struct Team {
    pub name: &'static str,
    pub number: u8,
    /// Main block color, second block color, and the word's ink.
    pub colors: [Rgb; 3],
    /// Strokes of the katakana/kanji word, one glyph per entry.
    pub word: &'static [Glyph],
}

type Glyph = &'static [&'static [(f32, f32)]];

// Glyph strokes in a unit box (x right, y down).
const YA: &[&[(f32, f32)]] = &[
    &[(0.06, 0.44), (0.92, 0.28), (0.66, 0.62)],
    &[(0.30, 0.08), (0.54, 0.94)],
];
const RI: &[&[(f32, f32)]] = &[
    &[(0.24, 0.12), (0.24, 0.62)],
    &[(0.74, 0.08), (0.74, 0.58), (0.42, 0.94)],
];
const KA: &[&[(f32, f32)]] = &[
    &[(0.08, 0.34), (0.86, 0.34), (0.80, 0.90), (0.62, 0.80)],
    &[(0.46, 0.06), (0.42, 0.54), (0.12, 0.94)],
];
const ZE: &[&[(f32, f32)]] = &[
    &[(0.04, 0.48), (0.80, 0.36), (0.56, 0.64)],
    &[(0.30, 0.12), (0.30, 0.86), (0.84, 0.86)],
    &[(0.74, 0.04), (0.80, 0.18)],
    &[(0.90, 0.02), (0.96, 0.16)],
];
const HO: &[&[(f32, f32)]] = &[
    &[(0.08, 0.30), (0.92, 0.30)],
    &[(0.50, 0.04), (0.50, 0.94), (0.38, 0.84)],
    &[(0.28, 0.52), (0.10, 0.80)],
    &[(0.72, 0.52), (0.90, 0.80)],
];
const SHI: &[&[(f32, f32)]] = &[
    &[(0.14, 0.12), (0.32, 0.24)],
    &[(0.08, 0.40), (0.26, 0.52)],
    &[(0.14, 0.90), (0.56, 0.70), (0.90, 0.16)],
];
const HI: &[&[(f32, f32)]] = &[
    &[(0.24, 0.30), (0.14, 0.52)],
    &[(0.80, 0.24), (0.66, 0.50)],
    &[(0.50, 0.06), (0.50, 0.44), (0.10, 0.94)],
    &[(0.50, 0.44), (0.92, 0.94)],
];
const YAMA: &[&[(f32, f32)]] = &[
    &[(0.50, 0.06), (0.50, 0.86)],
    &[(0.12, 0.34), (0.12, 0.86), (0.88, 0.86)],
    &[(0.88, 0.34), (0.88, 0.86)],
];

/// One team per racer id (0 is the player, 1.. the AI in grid order), so a
/// craft wears the same colors in every race and in the championship table.
pub const TEAMS: [Team; 4] = [
    Team {
        name: "Yari Works",
        number: 7,
        colors: [[200, 16, 46], PAPER, INK],
        word: &[YA, RI],
    },
    Team {
        name: "Kaze Racing",
        number: 23,
        colors: [[98, 36, 204], [255, 66, 162], PAPER],
        word: &[KA, ZE],
    },
    Team {
        name: "Hoshi Motive",
        number: 58,
        colors: [[134, 206, 40], INK, INK],
        word: &[HO, SHI],
    },
    Team {
        name: "Kazan Heavy",
        number: 91,
        colors: [[28, 60, 212], PAPER, PAPER],
        word: &[HI, YAMA],
    },
];

/// Shell offset off the stock paint, so the two never z-fight.
const LIFT: f32 = 0.006;

/// A convex polygon of one color in a region's 2D art space (meters).
struct Shape {
    color: Rgb,
    poly: Vec<Vec2>,
}

fn quad(color: Rgb, a: Vec2, b: Vec2, half_width: f32) -> Shape {
    let dir = (b - a).normalize_or_zero();
    let (a, b) = (a - dir * half_width, b + dir * half_width);
    let side = dir.perp() * half_width;
    Shape {
        color,
        poly: vec![a - side, b - side, b + side, a + side],
    }
}

fn poly(color: Rgb, points: &[(f32, f32)]) -> Shape {
    Shape {
        color,
        poly: points.iter().map(|&(x, y)| Vec2::new(x, y)).collect(),
    }
}

fn rect(color: Rgb, x0: f32, y0: f32, x1: f32, y1: f32) -> Shape {
    poly(color, &[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

fn disc(color: Rgb, center: Vec2, radius: f32) -> Shape {
    let poly = (0..24)
        .map(|i| center + Vec2::from_angle(i as f32 / 24.0 * std::f32::consts::TAU) * radius)
        .collect();
    Shape { color, poly }
}

/// Glyph strokes laid out left to right from `origin`, `size` meters tall.
/// `flip` mirrors the run (art space x grows right to left).
fn text<G: AsRef<[&'static [(f32, f32)]]>>(
    out: &mut Vec<Shape>,
    color: Rgb,
    glyphs: &[G],
    origin: Vec2,
    size: f32,
    weight: f32,
    flip: bool,
) {
    for (i, glyph) in glyphs.iter().enumerate() {
        for line in glyph.as_ref().iter() {
            for w in line.windows(2) {
                let at = |(x, y): (f32, f32)| {
                    let x = (i as f32 + 0.08 + x * 0.84) * size;
                    origin + Vec2::new(if flip { -x } else { x }, y * size)
                };
                out.push(quad(color, at(w[0]), at(w[1]), weight * size * 0.5));
            }
        }
    }
}

/// Seven-segment race digits, one segment per bit of the mask.
fn number(n: u8) -> Vec<Vec<&'static [(f32, f32)]>> {
    const SEGMENTS: [&[(f32, f32)]; 7] = [
        &[(0.12, 0.06), (0.88, 0.06)],
        &[(0.88, 0.06), (0.88, 0.5)],
        &[(0.88, 0.5), (0.88, 0.94)],
        &[(0.12, 0.94), (0.88, 0.94)],
        &[(0.12, 0.5), (0.12, 0.94)],
        &[(0.12, 0.06), (0.12, 0.5)],
        &[(0.12, 0.5), (0.88, 0.5)],
    ];
    const MASKS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];
    let digits: &[u8] = if n >= 10 { &[n / 10, n % 10] } else { &[n] };
    digits
        .iter()
        .map(|&d| {
            (0..7)
                .filter(|bit| MASKS[d as usize] & (1 << bit) != 0)
                .map(|bit| SEGMENTS[bit])
                .collect()
        })
        .collect()
}

/// The team mark centered on `c`, `size` meters across (art-space y down).
#[rustfmt::skip]
fn mark(out: &mut Vec<Shape>, index: usize, color: Rgb, c: Vec2, size: f32) {
    const MARKS: [&[&[(f32, f32)]]; 4] = [
        // Spearhead on a shaft.
        &[&[(0.0, -0.48), (0.42, 0.05), (-0.42, 0.05)], &[(-0.08, 0.04), (0.08, 0.04), (0.08, 0.46), (-0.08, 0.46)]],
        // Slashed ring (the ring itself is added below).
        &[&[(-0.36, 0.26), (-0.26, 0.36), (0.36, -0.26), (0.26, -0.36)]],
        // Four-point star.
        &[&[(0.0, -0.48), (0.12, 0.0), (0.0, 0.48), (-0.12, 0.0)], &[(-0.48, 0.0), (0.0, -0.12), (0.48, 0.0), (0.0, 0.12)]],
        // Twin peaks.
        &[&[(-0.14, -0.40), (0.30, 0.38), (-0.58, 0.38)], &[(0.22, -0.18), (0.56, 0.38), (-0.12, 0.38)]],
    ];
    let at = |(x, y): (f32, f32)| c + Vec2::new(x, y) * size;
    for points in MARKS[index] {
        out.push(Shape { color, poly: points.iter().copied().map(at).collect() });
    }
    if index == 1 {
        for i in 0..16 {
            let p = |k: i32, r: f32| at((Vec2::from_angle(k as f32 / 16.0 * std::f32::consts::TAU) * r).into());
            out.push(Shape { color, poly: vec![p(i, 0.28), p(i + 1, 0.28), p(i + 1, 0.44), p(i, 0.44)] });
        }
    }
}

/// Flank art, `a` meters back from the nose and `y` down from the roof.
/// Words read nose-first on the left flank, tail-first on the right one.
fn flank(index: usize, len: f32, height: f32, right_side: bool) -> Vec<Shape> {
    let team = &TEAMS[index];
    let [primary, secondary, ink] = team.colors;
    let (l, h) = (len, height);
    let mut out = vec![rect(primary, 0.0, 0.0, l, h)];
    match index {
        0 => out.push(poly(
            secondary,
            &[(0.42 * l, 0.0), (l, 0.0), (l, h), (0.66 * l, h)],
        )),
        1 => {
            out.push(rect(secondary, 0.0, 0.64 * h, l, h));
            out.push(rect(PAPER, 0.0, 0.56 * h, l, 0.6 * h));
        }
        2 => {
            for x in [0.82 * l, 0.82 * l + 0.17, 0.82 * l + 0.34] {
                let chevron = [
                    (x, 0.0),
                    (x + 0.08, 0.0),
                    (x + 0.2, 0.55 * h),
                    (x + 0.08, h),
                    (x, h),
                    (x + 0.12, 0.55 * h),
                ];
                out.push(poly(secondary, &chevron));
            }
        }
        _ => {
            out.push(poly(
                CRIMSON,
                &[(0.24 * l, h), (l, 0.08 * h), (l, 0.18 * h), (0.3 * l, h)],
            ));
            out.push(poly(secondary, &[(0.3 * l, h), (l, 0.18 * h), (l, h)]));
        }
    }
    // Start of a `run`-long word spanning `a0..a0 + run`, in reading order.
    let start = |a0: f32, run: f32| if right_side { a0 + run } else { a0 };
    // Door roundel with the race number.
    let center = Vec2::new(0.45 * l, 0.6 * h);
    out.push(disc(PAPER, center, 0.25));
    let digits = number(team.number);
    let size = 0.3;
    let run = size * digits.len() as f32;
    let origin = Vec2::new(start(center.x - run * 0.5, run), center.y - size * 0.5);
    text(&mut out, INK, &digits, origin, size, 0.2, right_side);
    let mark_color = if index == 1 { PAPER } else { secondary };
    mark(
        &mut out,
        index,
        mark_color,
        Vec2::new(0.16 * l, 0.55 * h),
        0.36,
    );
    let run = 0.24 * team.word.len() as f32;
    let origin = Vec2::new(start(0.63 * l, run), 0.3 * h);
    text(&mut out, ink, team.word, origin, 0.24, 0.16, right_side);
    out
}

/// Top art: `x` across from the car's left, `a` back from the nose.
fn top(index: usize, len: f32, width: f32) -> Vec<Shape> {
    let team = &TEAMS[index];
    let [primary, secondary, _] = team.colors;
    let (l, w) = (len, width);
    let mut out = vec![rect(primary, 0.0, 0.0, w, l)];
    let (c, half) = (w * 0.5, [0.16, 0.06, 0.16, 0.08][index]);
    let offsets: &[f32] = if index == 1 { &[-0.2, 0.2] } else { &[0.0] };
    for o in offsets {
        out.push(rect(secondary, c + o - half, 0.0, c + o + half, l));
    }
    let ink = if index == 0 { INK } else { PAPER };
    mark(&mut out, index, ink, Vec2::new(c, 0.32), 0.5);
    // Roof number plate, upright from the chase camera.
    plate(&mut out, team.number, Vec2::new(w * 0.5, 0.5 * l), 0.36);
    out
}

/// A paper number plate with ink digits `size` meters tall, centered on `c`.
fn plate(out: &mut Vec<Shape>, n: u8, c: Vec2, size: f32) {
    let digits = number(n);
    let run = size * digits.len() as f32;
    let half = Vec2::new(run * 0.5 + size * 0.14, size * 0.64);
    out.push(rect(
        PAPER,
        c.x - half.x,
        c.y - half.y,
        c.x + half.x,
        c.y + half.y,
    ));
    let origin = c - Vec2::new(run, size) * 0.5;
    text(out, INK, &digits, origin, size, 0.2, false);
}

/// Tail art: a band of the second color under the team number.
fn tail(index: usize, width: f32, height: f32) -> Vec<Shape> {
    let team = &TEAMS[index];
    let [primary, secondary, _] = team.colors;
    let (w, h) = (width, height);
    let mut out = vec![
        rect(primary, 0.0, 0.0, w, h),
        rect(secondary, 0.0, 0.66 * h, w, h),
    ];
    plate(&mut out, team.number, Vec2::new(w * 0.5, 0.42 * h), 0.3);
    out
}

/// Clip convex `poly` to the half-plane left of `a -> b` (or right, if `!keep_left`).
fn clip(poly: &[Vec2], a: Vec2, b: Vec2, keep_left: bool) -> Vec<Vec2> {
    let side = |p: Vec2| {
        let s = (b - a).perp_dot(p - a);
        if keep_left { s } else { -s }
    };
    let mut out = Vec::new();
    for (i, &p) in poly.iter().enumerate() {
        let q = poly[(i + 1) % poly.len()];
        let (sp, sq) = (side(p), side(q));
        if sp >= 0.0 {
            out.push(p);
        }
        if (sp >= 0.0) != (sq >= 0.0) {
            out.push(p + (q - p) * (sp / (sp - sq)));
        }
    }
    out
}

fn area(poly: &[Vec2]) -> f32 {
    (1..poly.len().saturating_sub(1))
        .map(|i| (poly[i] - poly[0]).perp_dot(poly[i + 1] - poly[0]))
        .sum::<f32>()
        * 0.5
}

/// Paint `shapes` (bottom to top) over the convex fragment `frag`, returning
/// the visible convex pieces with their colors.
fn paint(frag: Vec<Vec2>, shapes: &[Shape]) -> Vec<(Rgb, Vec<Vec2>)> {
    let mut pieces = vec![(PAPER, frag)];
    for shape in shapes.iter() {
        // Counter-clockwise shapes keep the left side of each edge.
        let ccw = area(&shape.poly) > 0.0;
        let mut next = Vec::with_capacity(pieces.len());
        for (color, piece) in pieces {
            let mut inside = piece;
            for (i, &a) in shape.poly.iter().enumerate() {
                let b = shape.poly[(i + 1) % shape.poly.len()];
                let outside = clip(&inside, a, b, !ccw);
                if area(&outside).abs() > 1e-7 {
                    next.push((color, outside));
                }
                inside = clip(&inside, a, b, ccw);
                if inside.len() < 3 {
                    break;
                }
            }
            if inside.len() >= 3 && area(&inside).abs() > 1e-7 {
                next.push((shape.color, inside));
            }
        }
        pieces = next;
    }
    pieces
}

struct Prim {
    positions: Vec<glam::Vec3>,
    normals: Vec<glam::Vec3>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
    material: serde_json::Value,
}

/// Flatten every mesh primitive of a GLB's default scene into model space.
fn read_glb(bytes: &[u8]) -> Option<(serde_json::Value, Vec<Prim>)> {
    let u32_at = |at: usize| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let json_len = u32_at(12)? as usize;
    let json: serde_json::Value = serde_json::from_slice(bytes.get(20..20 + json_len)?).ok()?;
    let bin = bytes.get(28 + json_len..)?;
    let read = |accessor: &serde_json::Value, width: usize| -> Option<Vec<f32>> {
        let view = &json["bufferViews"][accessor["bufferView"].as_u64()? as usize];
        let base =
            view["byteOffset"].as_u64().unwrap_or(0) + accessor["byteOffset"].as_u64().unwrap_or(0);
        let kind = accessor["componentType"].as_u64()?;
        let size = match kind {
            5121 => 1,
            5123 => 2,
            _ => 4,
        };
        let stride = view["byteStride"].as_u64().unwrap_or((size * width) as u64) as usize;
        let mut out = Vec::new();
        for i in 0..accessor["count"].as_u64()? as usize {
            for k in 0..width {
                let at = base as usize + i * stride + k * size;
                let raw = bin.get(at..at + size)?;
                out.push(match kind {
                    5121 => raw[0] as f32,
                    5123 => u16::from_le_bytes([raw[0], raw[1]]) as f32,
                    5125 => u32::from_le_bytes(raw.try_into().ok()?) as f32,
                    _ => f32::from_le_bytes(raw.try_into().ok()?),
                });
            }
        }
        Some(out)
    };
    let mut prims = Vec::new();
    let mut stack: Vec<(u64, glam::Affine3A)> = json["scenes"][0]["nodes"]
        .as_array()?
        .iter()
        .filter_map(|n| Some((n.as_u64()?, glam::Affine3A::IDENTITY)))
        .collect();
    while let Some((node_index, parent)) = stack.pop() {
        let node = &json["nodes"][node_index as usize];
        let vec = |key: &str, mut out: [f32; 4]| {
            let values = node[key].as_array().into_iter().flatten();
            values
                .zip(&mut out)
                .for_each(|(v, o)| *o = v.as_f64().unwrap_or(0.0) as f32);
            out
        };
        let [tx, ty, tz, _] = vec("translation", [0.0; 4]);
        let [sx, sy, sz, _] = vec("scale", [1.0; 4]);
        let rotation = glam::Quat::from_array(vec("rotation", [0.0, 0.0, 0.0, 1.0]));
        let local = glam::Affine3A::from_scale_rotation_translation(
            glam::Vec3::new(sx, sy, sz),
            rotation,
            glam::Vec3::new(tx, ty, tz),
        );
        let world = parent * local;
        for child in node["children"].as_array().into_iter().flatten() {
            stack.push((child.as_u64()?, world));
        }
        let Some(mesh) = node["mesh"].as_u64() else {
            continue;
        };
        for primitive in json["meshes"][mesh as usize]["primitives"].as_array()? {
            let attr = &primitive["attributes"];
            let accessor =
                |key: &str| &json["accessors"][attr[key].as_u64().unwrap_or(u64::MAX) as usize];
            let positions = read(accessor("POSITION"), 3)?;
            let normals = read(accessor("NORMAL"), 3)?;
            let uvs = read(accessor("TEXCOORD_0"), 2)
                .unwrap_or_else(|| vec![0.0; positions.len() / 3 * 2]);
            let indices = match primitive["indices"].as_u64() {
                Some(index) => read(&json["accessors"][index as usize], 1)?
                    .iter()
                    .map(|&i| i as u32)
                    .collect(),
                None => (0..positions.len() as u32 / 3).collect(),
            };
            prims.push(Prim {
                positions: positions
                    .chunks(3)
                    .map(|p| world.transform_point3(glam::Vec3::from_slice(p)))
                    .collect(),
                normals: normals
                    .chunks(3)
                    .map(|n| {
                        world
                            .transform_vector3(glam::Vec3::from_slice(n))
                            .normalize_or_zero()
                    })
                    .collect(),
                uvs: uvs.chunks(2).map(|uv| [uv[0], uv[1]]).collect(),
                indices,
                material: primitive["material"]
                    .as_u64()
                    .map(|m| json["materials"][m as usize].clone())
                    .unwrap_or_default(),
            });
        }
    }
    Some((json, prims))
}

fn face_normal(p: [glam::Vec3; 3]) -> glam::Vec3 {
    (p[1] - p[0]).cross(p[2] - p[0])
}

/// Build team `index`'s livery shell for a body GLB. `forward_z` is +1 for
/// bodies whose nose points at +Z in model space, -1 for -Z.
pub fn shell(body: &[u8], index: usize, forward_z: f32) -> Option<Vec<glb::MeshData>> {
    let (_, prims) = read_glb(body)?;
    // Paint is the material (or colormap swatch) with the most upward- or
    // sideways-facing area; glass, trim and the underside keep the stock look.
    let key = |prim: &Prim, tri: &[u32]| -> String {
        let name = prim.material["name"].as_str().unwrap_or("").to_string();
        if prim.material["pbrMetallicRoughness"]["baseColorTexture"].is_null() {
            return name;
        }
        let uv = tri.iter().fold([0.0f32; 2], |s, &i| {
            let uv = prim.uvs[i as usize];
            [s[0] + uv[0] / 3.0, s[1] + uv[1] / 3.0]
        });
        format!(
            "{name}@{}x{}",
            (uv[0] * 16.0).floor(),
            (uv[1] * 4.0).floor()
        )
    };
    let mut areas: Vec<(String, f32)> = Vec::new();
    for prim in prims.iter() {
        for tri in prim.indices.chunks(3) {
            let n = face_normal([0, 1, 2].map(|k| prim.positions[tri[k] as usize]));
            let k = key(prim, tri);
            let up = (n.normalize_or_zero().y > -0.3) as u8 as f32;
            let weight =
                n.length() * 0.5 * up + 1e3 * k.to_lowercase().contains("paint") as u8 as f32;
            match areas.iter_mut().find(|e| e.0 == k) {
                Some(entry) => entry.1 += weight,
                None => areas.push((k, weight)),
            }
        }
    }
    let paint_key = areas.iter().max_by(|a, b| a.1.total_cmp(&b.1))?.0.clone();

    let (mut min, mut max) = (glam::Vec3::splat(f32::MAX), glam::Vec3::splat(f32::MIN));
    for p in prims.iter().flat_map(|p| p.positions.iter()) {
        min = min.min(*p);
        max = max.max(*p);
    }
    let size = (max - min).max(glam::Vec3::splat(1e-3));
    let forward = glam::Vec3::new(0.0, 0.0, forward_z);
    let right = forward.cross(glam::Vec3::Y);
    let nose = if forward_z > 0.0 { max.z } else { -min.z };
    let regions = [
        flank(index, size.z, size.y, true),
        flank(index, size.z, size.y, false),
        top(index, size.z, size.x),
        tail(index, size.x, size.y),
    ];

    let mut meshes = std::collections::BTreeMap::<Rgb, glb::MeshData>::new();
    for prim in prims.iter() {
        for tri in prim.indices.chunks(3) {
            if key(prim, tri) != paint_key {
                continue;
            }
            let corners = [0, 1, 2].map(|k| tri[k] as usize);
            let p3 = corners.map(|i| prim.positions[i]);
            let n = face_normal(p3).normalize_or_zero();
            let side = n.dot(right);
            let region = if n.y > 0.55 {
                2
            } else if side.abs() >= n.dot(forward).abs() {
                if side > 0.0 { 0 } else { 1 }
            } else {
                3
            };
            let to_2d = |p: glam::Vec3| {
                let a = nose - p.dot(forward);
                let across = p.dot(right) + size.x * 0.5;
                let down = max.y - p.y;
                match region {
                    0 | 1 => Vec2::new(a, down),
                    2 => Vec2::new(across, a),
                    _ => Vec2::new(across, down),
                }
            };
            let p2 = p3.map(to_2d);
            let det = (p2[1] - p2[0]).perp_dot(p2[2] - p2[0]);
            // Undersides (and edge-on projections) just take the main color.
            let plain = n.y < -0.55 || det.abs() < 1e-7;
            let pieces = if plain {
                vec![(TEAMS[index].colors[0], p2.to_vec())]
            } else {
                paint(p2.to_vec(), &regions[region])
            };
            for (color, piece) in pieces {
                let mesh = meshes.entry(color).or_insert_with(|| glb::MeshData {
                    name: format!("{}-{color:02x?}", TEAMS[index].name),
                    positions: Vec::new(),
                    normals: Vec::new(),
                    tex_coords: Vec::new(),
                    indices: Vec::new(),
                    base_color: glb::srgb_to_linear(color),
                    metallic: 0.1,
                    roughness: 0.42,
                    emissive: [0.0; 3],
                });
                let base = mesh.positions.len() as u32;
                for (k, q) in piece.iter().enumerate() {
                    // Barycentric weights of `q` in the 2D triangle.
                    let (w1, w2) = if plain {
                        ((k == 1) as u8 as f32, (k == 2) as u8 as f32)
                    } else {
                        (
                            (*q - p2[0]).perp_dot(p2[2] - p2[0]) / det,
                            (p2[1] - p2[0]).perp_dot(*q - p2[0]) / det,
                        )
                    };
                    let w0 = 1.0 - w1 - w2;
                    let normal = (prim.normals[corners[0]] * w0
                        + prim.normals[corners[1]] * w1
                        + prim.normals[corners[2]] * w2)
                        .normalize_or(n);
                    let pos = p3[0] * w0 + p3[1] * w1 + p3[2] * w2 + n * LIFT;
                    mesh.positions.push(pos.into());
                    mesh.normals.push(normal.into());
                    mesh.tex_coords.push([0.0; 2]);
                }
                // Keep the stock winding: flip the fan if projection mirrored it.
                let flip = !plain && (det > 0.0) != (area(&piece) > 0.0);
                for k in 1..piece.len() as u32 - 1 {
                    let (b, c) = if flip { (k + 1, k) } else { (k, k + 1) };
                    mesh.indices.extend_from_slice(&[base, base + b, base + c]);
                }
            }
        }
    }
    let meshes: Vec<glb::MeshData> = meshes.into_values().collect();
    (!meshes.is_empty()).then_some(meshes)
}

/// Model path (relative to `assets`) of racer `racer`'s livery shell for
/// `body_model`, generated the first time this process asks for it.
pub fn model(
    assets: &Path,
    body_model: &str,
    racer: usize,
    read_file: &dyn Fn(&Path) -> Option<Vec<u8>>,
) -> Option<String> {
    static WRITTEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let index = racer % TEAMS.len();
    let stem = Path::new(body_model).file_stem()?.to_str()?;
    let rel = format!("generated/livery-{index}-{stem}.glb");
    let mut written = WRITTEN.lock().ok()?;
    let written = written.get_or_insert_with(HashSet::new);
    if written.contains(&rel) {
        return Some(rel);
    }
    // Kenney car-kit bodies face +Z; the race-future body faces -Z.
    let forward_z = [1.0, -1.0][stem.starts_with("race-future") as usize];
    let meshes = shell(&read_file(&assets.join(body_model))?, index, forward_z)?;
    let bytes = glb::encode_glb(&meshes).ok()?;
    // Unchanged files keep their mtime, so Blade's asset cache stays warm.
    let path: PathBuf = assets.join(&rel);
    if read_file(&path).as_deref() != Some(bytes.as_slice()) {
        glb::write_glb(&path, &meshes).ok()?;
    }
    written.insert(rel.clone());
    Some(rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hue_sat(c: Rgb) -> (f32, f32) {
        let [r, g, b] = c.map(|v| v as f32 / 255.0);
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let d = max - min;
        if d < 1e-4 {
            return (0.0, 0.0);
        }
        let h = if max == r {
            60.0 * ((g - b) / d).rem_euclid(6.0)
        } else if max == g {
            60.0 * ((b - r) / d + 2.0)
        } else {
            60.0 * ((r - g) / d + 4.0)
        };
        (h, d / max)
    }

    #[test]
    fn teams_avoid_amber_and_cyan() {
        for team in TEAMS.iter() {
            for c in team.colors.iter().copied().chain([PAPER, INK]) {
                let (h, s) = hue_sat(c);
                let reserved = (25.0..62.0).contains(&h) || (165.0..205.0).contains(&h);
                assert!(s < 0.3 || !reserved, "{} uses {c:?} (hue {h})", team.name);
            }
        }
        // Every shape the art draws uses a team color or paper/ink.
        for index in 0..TEAMS.len() {
            let shapes = [
                flank(index, 2.6, 1.0, true),
                top(index, 2.6, 1.3),
                tail(index, 1.3, 1.0),
            ];
            for shape in shapes.iter().flatten() {
                let (h, s) = hue_sat(shape.color);
                assert!(s < 0.3 || !((25.0..62.0).contains(&h) || (165.0..205.0).contains(&h)));
            }
        }
    }

    #[test]
    fn teams_are_distinct() {
        for (i, a) in TEAMS.iter().enumerate() {
            for b in TEAMS.iter().skip(i + 1) {
                assert_ne!(a.colors[0], b.colors[0]);
                assert_ne!(a.number, b.number);
            }
        }
    }

    #[test]
    fn shells_paint_with_every_team_color() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/models");
        for (index, team) in TEAMS.iter().enumerate() {
            let (model, forward) = [
                ("race-future-body.glb", -1.0),
                ("hatchback-sports-body.glb", 1.0),
                ("sedan-sports-body.glb", 1.0),
                ("taxi-body.glb", 1.0),
            ][index];
            let Ok(body) = std::fs::read(dir.join(model)) else {
                return;
            };
            let meshes = shell(&body, index, forward).expect("shell");
            let colors: Vec<[f32; 4]> = meshes.iter().map(|m| m.base_color).collect();
            for c in team.colors.iter().chain([&PAPER]) {
                assert!(
                    colors.contains(&glb::srgb_to_linear(*c)),
                    "{} lacks {c:?}",
                    team.name
                );
            }
            for mesh in meshes.iter() {
                assert_eq!(mesh.indices.len() % 3, 0);
                let bad = mesh
                    .positions
                    .iter()
                    .filter(|p| !p.iter().all(|v| v.is_finite()))
                    .count();
                assert_eq!(bad, 0, "{} of {}", mesh.name, mesh.positions.len());
            }
        }
    }

    #[test]
    fn paint_covers_the_fragment_exactly() {
        let frag = vec![Vec2::ZERO, Vec2::new(2.0, 0.0), Vec2::new(0.0, 2.0)];
        let shapes = flank(0, 2.6, 1.0, false);
        let total: f32 = paint(frag, &shapes)
            .iter()
            .map(|&(_, ref p)| area(p).abs())
            .sum();
        assert!((total - 2.0).abs() < 1e-3, "{total}");
    }
}
