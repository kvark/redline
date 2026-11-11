//! Equirect Mars sky for the environment map: a dusty butterscotch dome that
//! sinks to rose and mauve away from the sun, plus Phobos and Deimos.
//! Baked once on the CPU into the existing env-map path, so no new shader,
//! blend mode or texture format reaches WebGL2.

use std::f32::consts;

use crate::planet;

/// Sun disc and halo half-angles (radians).
const SUN_CORE: f32 = 0.09;
const SUN_GLOW: f32 = 0.42;
/// From the Martian surface Phobos spans ~0.6 of the Sun's disc and Deimos
/// is close to a bright point (~0.1). Deimos is nudged up to stay visible.
const PHOBOS_RADIUS: f32 = SUN_CORE * 0.6;
const DEIMOS_RADIUS: f32 = SUN_CORE * 0.15;

pub const PHOBOS_DIR: glam::Vec3 = glam::Vec3::new(0.844, 0.03, -0.535);
pub const DEIMOS_DIR: glam::Vec3 = glam::Vec3::new(0.643, 0.503, -0.577);

/// Sky stops: anti-sun mauve, mid rose, butterscotch around the sun.
const FAR: glam::Vec3 = glam::Vec3::new(72.0, 38.0, 32.0);
const MID: glam::Vec3 = glam::Vec3::new(160.0, 98.0, 76.0);
const NEAR: glam::Vec3 = glam::Vec3::new(206.0, 148.0, 94.0);

/// Inverse of Blade's raster `map_equirect_dir_to_uv`.
fn texel_dir(x: u32, y: u32, width: u32, height: u32) -> glam::Vec3 {
    let u = (x as f32 + 0.5) / width as f32;
    let v = (y as f32 + 0.5) / height as f32;
    let yaw = (u * 2.0 - 1.0) * consts::PI;
    let pitch = (v - 0.5) * consts::PI;
    glam::Vec3::new(
        pitch.cos() * yaw.sin(),
        pitch.sin(),
        pitch.cos() * yaw.cos(),
    )
}

/// Lit moon color at `dir`, if it falls on the disc (`squash` < 1 flattens it).
fn moon(
    dir: glam::Vec3,
    center: glam::Vec3,
    radius: f32,
    squash: f32,
    sun: glam::Vec3,
) -> Option<glam::Vec3> {
    let center = center.normalize();
    if dir.dot(center) < (radius * 1.6).cos() {
        return None;
    }
    let e1 = center.any_orthonormal_vector();
    let e2 = center.cross(e1);
    let x = dir.dot(e1) / radius;
    let y = dir.dot(e2) / (radius * squash);
    let r2 = x * x + y * y;
    if r2 >= 1.0 {
        return None;
    }
    // The visible hemisphere faces the observer, i.e. -center.
    let normal = e1 * x + e2 * y - center * (1.0 - r2).sqrt();
    let lit = normal.dot(sun).max(0.0);
    // Soft limb so the disc does not alias into a hard square edge.
    let limb = ((1.0 - r2) * 6.0).min(1.0);
    // Coarse albedo blotches hint at craters (Stickney) without a texture.
    let blotch = 0.85 + 0.15 * ((x * 5.3).sin() * (y * 4.1 + x * 2.0).cos());
    Some(glam::Vec3::new(132.0, 112.0, 98.0) * (0.12 + 0.88 * lit) * limb * blotch)
}

pub fn sky(width: u32, height: u32) -> Vec<[u8; 4]> {
    let sun = planet::SUN_DIRECTION.normalize();
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            let dir = texel_dir(x, y, width, height);
            let t = (dir.dot(sun) + 1.0) * 0.5;
            let mut color = if t < 0.6 {
                FAR.lerp(MID, (t / 0.6).powf(1.4))
            } else {
                MID.lerp(NEAR, ((t - 0.6) / 0.4).powf(0.8))
            };
            // Fine suspended-dust grain keeps the dome from banding.
            let grain = (crate::game::hash_u32(x ^ y.wrapping_mul(width)) & 7) as f32 - 3.5;
            color += glam::Vec3::splat(grain * 0.8);
            // Only the brightest stars cut through, on the darkest side.
            let h = crate::game::hash_u32(x.wrapping_mul(9_781) ^ y.wrapping_mul(6_271));
            if t < 0.3 && h.is_multiple_of(2_400) {
                color += glam::Vec3::splat(90.0 * (1.0 - t / 0.3));
            }
            let angle = dir.dot(sun).clamp(-1.0, 1.0).acos();
            if angle < SUN_GLOW {
                let core = (1.0 - (angle / SUN_CORE).min(1.0)).powf(0.6);
                let halo = (1.0 - angle / SUN_GLOW).powf(2.2) * 0.45;
                color = color.lerp(glam::Vec3::new(255.0, 238.0, 200.0), (core + halo).min(1.0));
            }
            for (center, radius, squash) in [
                (PHOBOS_DIR, PHOBOS_RADIUS, 0.8),
                (DEIMOS_DIR, DEIMOS_RADIUS, 1.0),
            ] {
                if let Some(lit) = moon(dir, center, radius, squash, sun) {
                    color = lit;
                }
            }
            let c = color.clamp(glam::Vec3::ZERO, glam::Vec3::splat(255.0));
            pixels.push([c.x as u8, c.y as u8, c.z as u8, 255]);
        }
    }
    pixels
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(sky: &[[u8; 4]], w: u32, h: u32, dir: glam::Vec3) -> [u8; 4] {
        let dir = dir.normalize();
        let u = (dir.x.atan2(dir.z) / consts::PI + 1.0) * 0.5;
        let v = dir.y.clamp(-1.0, 1.0).asin() / consts::PI + 0.5;
        let x = ((u * w as f32) as u32).min(w - 1);
        let y = ((v * h as f32) as u32).min(h - 1);
        sky[(y * w + x) as usize]
    }

    #[test]
    fn dome_is_butterscotch_near_sun_and_dark_away() {
        let (w, h) = (512, 256);
        let sky = sky(w, h);
        assert_eq!(sky.len(), (w * h) as usize);
        let sun = planet::SUN_DIRECTION.normalize();
        let side = sun.any_orthonormal_vector();
        let near = at(&sky, w, h, sun + side * 0.7);
        let far = at(&sky, w, h, -sun);
        assert!(
            near[0] > near[1] && near[1] > near[2],
            "warm butterscotch {near:?}"
        );
        assert!(far[0] < near[0] / 2, "anti-sun stays dusky {far:?}");
    }

    #[test]
    fn phobos_reads_larger_than_deimos() {
        // Same angular sweep for both, so the hit count measures disc width.
        let disc = |center: glam::Vec3, radius: f32| {
            let e1 = center.normalize().any_orthonormal_vector();
            (0..400)
                .filter(|i| {
                    let off = (*i as f32 / 200.0 - 1.0) * 0.12;
                    let dir = (center.normalize() + e1 * off).normalize();
                    moon(dir, center, radius, 1.0, planet::SUN_DIRECTION.normalize()).is_some()
                })
                .count()
        };
        let phobos = disc(PHOBOS_DIR, PHOBOS_RADIUS);
        let deimos = disc(DEIMOS_DIR, DEIMOS_RADIUS);
        assert!(phobos > deimos * 3, "phobos {phobos} deimos {deimos}");
        assert!(deimos > 0);
    }
}
