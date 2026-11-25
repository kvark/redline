//! Team liveries: one original team per racer id, with bold color blocks, a
//! big mark, a race number and a katakana/kanji word. Teams are data
//! ([`TEAMS`]) drawn by one layout ([`art`]) as convex shapes clipped into the
//! body's paint triangles, emitted as a thin untextured shell over the stock
//! body (plain materials only, so WebGL2-safe and crisp at any distance).
//! Visual only. Color grammar (#44): amber is edge/limit and cyan is
//! boost/go, so no team color sits in either hue band.

use std::path::Path;

use glam::Vec2;

use crate::glb;

type Rgb = [u8; 3];
type Points = &'static [(f32, f32)];

const PAPER: Rgb = [242, 240, 234];
const INK: Rgb = [16, 16, 20];

/// Art regions of a body: right flank, left flank, roof, tail.
const FLANKS: u8 = 0b0011;
const TOP: u8 = 0b0100;
/// Every tail carries a band of the second color under its number.
#[rustfmt::skip]
const TAIL_BAND: (u8, usize, Points) = (
    0b1000,
    1,
    &[(0.0, 0.66), (1.0, 0.66), (1.0, 1.0), (0.0, 1.0)],
);

pub struct Team {
    pub name: &'static str,
    pub number: u8,
    /// Main block, second block, word ink, accent.
    pub colors: [Rgb; 4],
    /// Katakana/kanji word: per glyph, strokes as polylines in a unit box (y down).
    word: &'static [&'static [Points]],
    /// Mark polygons in a unit box centered on 0 (y down).
    mark: &'static [Points],
    /// Mark color slot on the flanks and on the roof (see [`slot`]).
    mark_slots: [usize; 2],
    /// Color blocks: region mask, color slot, polygon in the region's unit box.
    blocks: &'static [(u8, usize, Points)],
}

/// One team per racer id (0 is the player, 1.. the AI in grid order), so a
/// craft wears the same colors in every race and in the championship table.
#[rustfmt::skip]
pub const TEAMS: [Team; 4] = [
    Team {
        name: "Yari Works", number: 7,
        colors: [[200, 16, 46], PAPER, INK, INK],
        // ヤリ
        word: &[
            &[&[(0.06, 0.44), (0.92, 0.28), (0.66, 0.62)], &[(0.30, 0.08), (0.54, 0.94)]],
            &[&[(0.24, 0.12), (0.24, 0.62)], &[(0.74, 0.08), (0.74, 0.58), (0.42, 0.94)]],
        ],
        // Spearhead on a shaft.
        mark: &[&[(0.0, -0.48), (0.42, 0.05), (-0.42, 0.05)], &[(-0.08, 0.04), (0.08, 0.04), (0.08, 0.46), (-0.08, 0.46)]],
        mark_slots: [1, 3],
        blocks: &[
            (FLANKS, 1, &[(0.42, 0.0), (1.0, 0.0), (1.0, 1.0), (0.66, 1.0)]),
            (TOP, 1, &[(0.38, 0.0), (0.62, 0.0), (0.62, 1.0), (0.38, 1.0)]),
        ],
    },
    Team {
        name: "Kaze Racing", number: 23,
        colors: [[98, 36, 204], [255, 66, 162], PAPER, PAPER],
        // カゼ
        word: &[
            &[&[(0.08, 0.34), (0.86, 0.34), (0.80, 0.90), (0.62, 0.80)], &[(0.46, 0.06), (0.42, 0.54), (0.12, 0.94)]],
            &[&[(0.04, 0.48), (0.80, 0.36), (0.56, 0.64)], &[(0.30, 0.12), (0.30, 0.86), (0.84, 0.86)],
              &[(0.74, 0.04), (0.80, 0.18)], &[(0.90, 0.02), (0.96, 0.16)]],
        ],
        // Three wind slashes.
        mark: &[
            &[(-0.62, 0.4), (-0.3, -0.4), (-0.14, -0.4), (-0.46, 0.4)],
            &[(-0.24, 0.4), (0.08, -0.4), (0.24, -0.4), (-0.08, 0.4)],
            &[(0.14, 0.4), (0.46, -0.4), (0.62, -0.4), (0.3, 0.4)],
        ],
        mark_slots: [3, 3],
        blocks: &[
            (FLANKS, 1, &[(0.0, 0.64), (1.0, 0.64), (1.0, 1.0), (0.0, 1.0)]),
            (FLANKS, 3, &[(0.0, 0.56), (1.0, 0.56), (1.0, 0.6), (0.0, 0.6)]),
            (TOP, 1, &[(0.31, 0.0), (0.39, 0.0), (0.39, 1.0), (0.31, 1.0)]),
            (TOP, 1, &[(0.61, 0.0), (0.69, 0.0), (0.69, 1.0), (0.61, 1.0)]),
        ],
    },
    Team {
        name: "Hoshi Motive", number: 58,
        colors: [[134, 206, 40], INK, INK, INK],
        // ホシ
        word: &[
            &[&[(0.08, 0.30), (0.92, 0.30)], &[(0.50, 0.04), (0.50, 0.94), (0.38, 0.84)],
              &[(0.28, 0.52), (0.10, 0.80)], &[(0.72, 0.52), (0.90, 0.80)]],
            &[&[(0.14, 0.12), (0.32, 0.24)], &[(0.08, 0.40), (0.26, 0.52)], &[(0.14, 0.90), (0.56, 0.70), (0.90, 0.16)]],
        ],
        // Four-point star.
        mark: &[&[(0.0, -0.48), (0.12, 0.0), (0.0, 0.48), (-0.12, 0.0)], &[(-0.48, 0.0), (0.0, -0.12), (0.48, 0.0), (0.0, 0.12)]],
        mark_slots: [1, 4],
        blocks: &[
            (FLANKS, 1, &[(0.84, 0.0), (0.9, 0.0), (0.96, 1.0), (0.9, 1.0)]),
            (FLANKS, 1, &[(0.93, 0.0), (1.0, 0.0), (1.0, 1.0), (0.99, 1.0)]),
            (TOP, 1, &[(0.38, 0.0), (0.62, 0.0), (0.62, 1.0), (0.38, 1.0)]),
        ],
    },
    Team {
        name: "Kazan Heavy", number: 91,
        colors: [[28, 60, 212], PAPER, PAPER, [214, 22, 52]],
        // 火山
        word: &[
            &[&[(0.24, 0.30), (0.14, 0.52)], &[(0.80, 0.24), (0.66, 0.50)],
              &[(0.50, 0.06), (0.50, 0.44), (0.10, 0.94)], &[(0.50, 0.44), (0.92, 0.94)]],
            &[&[(0.50, 0.06), (0.50, 0.86)], &[(0.12, 0.34), (0.12, 0.86), (0.88, 0.86)], &[(0.88, 0.34), (0.88, 0.86)]],
        ],
        // Twin peaks.
        mark: &[&[(-0.14, -0.40), (0.30, 0.38), (-0.58, 0.38)], &[(0.22, -0.18), (0.56, 0.38), (-0.12, 0.38)]],
        mark_slots: [1, 4],
        blocks: &[
            (FLANKS, 3, &[(0.24, 1.0), (1.0, 0.08), (1.0, 0.18), (0.3, 1.0)]),
            (FLANKS, 1, &[(0.3, 1.0), (1.0, 0.18), (1.0, 1.0)]),
            (TOP, 3, &[(0.46, 0.0), (0.54, 0.0), (0.54, 1.0), (0.46, 1.0)]),
        ],
    },
];

/// Seven-segment race digits: segment polylines and per-digit masks.
#[rustfmt::skip]
const SEGMENTS: [Points; 7] = [
    &[(0.12, 0.06), (0.88, 0.06)], &[(0.88, 0.06), (0.88, 0.5)], &[(0.88, 0.5), (0.88, 0.94)],
    &[(0.12, 0.94), (0.88, 0.94)], &[(0.12, 0.5), (0.12, 0.94)], &[(0.12, 0.06), (0.12, 0.5)],
    &[(0.12, 0.5), (0.88, 0.5)],
];
const DIGITS: [u8; 10] = [0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F];

/// Shell offset off the stock paint, so the two never z-fight.
const LIFT: f32 = 0.006;

/// A convex polygon of one color in a region's 2D art space (meters).
struct Shape {
    color: Rgb,
    poly: Vec<Vec2>,
}

/// Art under construction, bottom shape first. `flip` mirrors text runs for
/// the right flank, whose art space is seen from outside mirrored.
struct Canvas {
    out: Vec<Shape>,
    flip: bool,
}

impl Canvas {
    fn poly(&mut self, color: Rgb, points: &[(f32, f32)], at: impl Fn(Vec2) -> Vec2) {
        let poly = points.iter().map(|&(x, y)| at(Vec2::new(x, y))).collect();
        self.out.push(Shape { color, poly });
    }

    /// Glyph strokes (polylines in a unit box), `size` tall, centered on `c`.
    fn text<G: AsRef<[Points]>>(&mut self, color: Rgb, glyphs: &[G], c: Vec2, size: f32) {
        let (sign, run) = (if self.flip { -1.0 } else { 1.0 }, glyphs.len() as f32);
        for (i, glyph) in glyphs.iter().enumerate() {
            let x0 = i as f32 + 0.08 - run * 0.5;
            let at = |(x, y): (f32, f32)| c + Vec2::new(sign * (x0 + x * 0.84), y - 0.5) * size;
            for w in glyph.as_ref().iter().flat_map(|line| line.windows(2)) {
                let (a, b) = (at(w[0]), at(w[1]));
                let d = (b - a).normalize_or_zero() * 0.09 * size;
                let (a, b, s) = (a - d, b + d, d.perp());
                let poly = vec![a - s, b - s, b + s, a + s];
                self.out.push(Shape { color, poly });
            }
        }
    }

    /// Ink race number on a paper disc (or plate), `size` tall, centered on `c`.
    fn number(&mut self, n: u8, c: Vec2, size: f32, disc: bool) {
        let digits: Vec<Vec<Points>> = [n / 10, n % 10]
            .into_iter()
            .skip((n < 10) as usize)
            .map(|d| {
                (0..7)
                    .filter(|b| DIGITS[d as usize] & (1 << b) != 0)
                    .map(|b| SEGMENTS[b])
                    .collect()
            })
            .collect();
        let backing: Vec<(f32, f32)> = if disc {
            let ring = |i| Vec2::from_angle(i as f32 / 24.0 * std::f32::consts::TAU) * 0.82;
            (0..24).map(|i| ring(i).into()).collect()
        } else {
            let (hx, hy) = (digits.len() as f32 * 0.5 + 0.14, 0.64);
            vec![(-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy)]
        };
        self.poly(PAPER, &backing, |p| c + p * size);
        self.text(INK, &digits, c, size);
    }
}

/// The art of team `index` for region `region` (0/1 right/left flank, 2 roof,
/// 3 tail), `w` by `h` meters. Flanks run `x` back from the nose and `y` down
/// from the roof; the roof runs across then back; the tail across then down.
fn art(index: usize, region: usize, w: f32, h: f32) -> Vec<Shape> {
    let team = &TEAMS[index];
    let [c0, c1, c2, c3] = team.colors;
    let palette = [c0, c1, c2, c3, PAPER, INK];
    let at = |x: f32, y: f32| Vec2::new(x * w, y * h);
    let unit = |p: Vec2| at(p.x, p.y);
    let mut art = Canvas {
        out: Vec::new(),
        flip: region == 0,
    };
    art.poly(c0, &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)], unit);
    for &(mask, s, points) in team.blocks.iter().chain([&TAIL_BAND]) {
        if mask & (1 << region) != 0 {
            art.poly(palette[s], points, unit);
        }
    }
    let mark = |art: &mut Canvas, s: usize, c: Vec2, size: f32| {
        for m in team.mark {
            art.poly(palette[team.mark_slots[s]], m, |p| c + p * size);
        }
    };
    // Flank marks, numbers and words are sized to read from the chase camera.
    match region {
        0 | 1 => {
            mark(&mut art, 0, at(0.15, 0.52), 0.62);
            art.number(team.number, at(0.42, 0.58), 0.4, true);
            let word = 0.34;
            let c = at(0.6, 0.22) + Vec2::new(team.word.len() as f32, 1.0) * word * 0.5;
            art.text(c2, team.word, c, word);
        }
        2 => {
            mark(&mut art, 1, Vec2::new(0.5 * w, 0.32), 0.5);
            art.number(team.number, at(0.5, 0.5), 0.36, false);
        }
        _ => art.number(team.number, at(0.5, 0.42), 0.3, false),
    }
    art.out
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

fn face_normal(p: [glam::Vec3; 3]) -> glam::Vec3 {
    (p[1] - p[0]).cross(p[2] - p[0])
}

/// Build team `index`'s livery shell for a body GLB. `forward_z` is +1 for
/// bodies whose nose points at +Z in model space, -1 for -Z.
pub fn shell(body: &[u8], index: usize, forward_z: f32) -> Option<Vec<glb::MeshData>> {
    let prims = glb::read_glb(body)?;
    // Paint is the material (or colormap swatch) with the most upward- or
    // sideways-facing area; glass, trim and the underside keep the stock look.
    let key = |prim: &glb::Primitive, tri: &[u32]| -> String {
        let name = prim.material["name"].as_str().unwrap_or("").to_string();
        if prim.material["pbrMetallicRoughness"]["baseColorTexture"].is_null() {
            return name;
        }
        let uv = tri.iter().fold([0.0f32; 2], |s, &i| {
            let uv = prim.uvs[i as usize];
            [s[0] + uv[0] / 3.0, s[1] + uv[1] / 3.0]
        });
        let cell = [(uv[0] * 16.0) as i32, (uv[1] * 4.0) as i32];
        format!("{name}@{cell:?}")
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
    let regions = [0, 1, 2, 3].map(|r| {
        art(
            index,
            r,
            [size.z, size.z, size.x, size.x][r],
            [size.y, size.y, size.z, size.y][r],
        )
    });

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
                    base_color: glb::srgb_to_linear(color),
                    metallic: 0.1,
                    roughness: 0.42,
                    ..Default::default()
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
/// `body_model`, (re)written when its bytes change.
pub fn model(
    assets: &Path,
    body_model: &str,
    racer: usize,
    read_file: &dyn Fn(&Path) -> Option<Vec<u8>>,
) -> Option<String> {
    let index = racer % TEAMS.len();
    let stem = Path::new(body_model).file_stem()?.to_str()?;
    // Kenney car-kit bodies face +Z; the race-future body faces -Z.
    let forward_z = [1.0, -1.0][stem.starts_with("race-future") as usize];
    let meshes = shell(&read_file(&assets.join(body_model))?, index, forward_z)?;
    let rel = format!("generated/livery-{index}-{stem}.glb");
    // Unchanged files keep their mtime, so Blade's asset cache stays warm.
    let path = assets.join(&rel);
    if read_file(&path) != glb::encode_glb(&meshes).ok() {
        glb::write_glb(&path, &meshes).ok()?;
    }
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
            let shapes = [0, 1, 2, 3].map(|r| art(index, r, 2.6, 1.0));
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
        let shapes = art(0, 1, 2.6, 1.0);
        let total: f32 = paint(frag, &shapes)
            .iter()
            .map(|&(_, ref p)| area(p).abs())
            .sum();
        assert!((total - 2.0).abs() < 1e-3, "{total}");
    }
}
