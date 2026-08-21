use crate::planet;
use std::{env, fs, path::PathBuf};


pub fn smoke_frame_budget() -> Option<u32> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--smoke" {
            return Some(
                args.next()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(20),
            );
        }
    }
    match env::var("REDLINE_SMOKE") {
        Ok(value) if value.is_empty() || value == "1" => Some(20),
        Ok(value) => Some(value.parse().expect("REDLINE_SMOKE must be a frame count")),
        Err(_) => None,
    }
}

pub fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

pub fn read_asset_bytes(path: &std::path::Path) -> Vec<u8> {
    if let Some(bytes) = blade_engine::vfs::read(path) {
        return bytes;
    }
    fs::read(path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

#[cfg(target_arch = "wasm32")]
fn mount_embedded_assets() {
    use include_dir::{Dir, include_dir};
    static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets");
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    fn walk(dir: &Dir, root: &std::path::Path) {
        for file in dir.files() {
            blade_engine::vfs::mount(root.join(file.path()), file.contents().to_vec());
        }
        for child in dir.dirs() {
            walk(child, root);
        }
    }
    walk(&ASSETS, &root);
}

pub fn relative_model(assets: &std::path::Path, model: &std::path::Path) -> String {
    model
        .strip_prefix(assets)
        .unwrap_or(model)
        .to_string_lossy()
        .into_owned()
}

pub fn spawn_props(engine: &mut blade_engine::Engine, planet: &planet::GeneratedPlanet) {
    let start = &planet.track[0];
    let flag = blade_engine::config::Object {
        name: "start-flag".to_string(),
        visuals: vec![blade_engine::config::Visual {
            model: "models/flag-checkers.glb".to_string(),
            scale: 2.4,
            ..Default::default()
        }],
        colliders: vec![],
        additional_mass: None,
    };
    engine.add_object(
        &flag,
        blade_engine::Transform {
            position: (start.position + start.normal * 0.2).into(),
            orientation: planet::surface_quat(start.normal, start.tangent).into(),
        },
        blade_engine::DynamicInput::Empty,
    );

    let pylon = blade_engine::config::Object {
        name: "pylon".to_string(),
        visuals: vec![blade_engine::config::Visual {
            model: "models/pylon.glb".to_string(),
            scale: 1.6,
            ..Default::default()
        }],
        colliders: vec![],
        additional_mass: None,
    };
    let stride = (planet.track.len() / 14).max(1);
    for sample in planet.track.iter().step_by(stride) {
        let side = sample.normal.cross(sample.tangent).normalize_or_zero();
        let offset = planet.track_width * 0.52;
        for sign in [-1.0, 1.0] {
            let pos = sample.position + side * (offset * sign) + sample.normal * 0.1;
            engine.add_object(
                &pylon,
                blade_engine::Transform {
                    position: pos.into(),
                    orientation: planet::surface_quat(sample.normal, sample.tangent).into(),
                },
                blade_engine::DynamicInput::Empty,
            );
        }
    }
}

pub fn project_tangent(v: glam::Vec3, up: glam::Vec3) -> glam::Vec3 {
    let t = v - up * v.dot(up);
    if t.length_squared() < 1e-5 {
        let fallback = up.cross(glam::Vec3::X);
        if fallback.length_squared() < 1e-5 {
            up.cross(glam::Vec3::Z).normalize_or_zero()
        } else {
            fallback.normalize()
        }
    } else {
        t.normalize()
    }
}

pub fn starfield(width: u32, height: u32) -> Vec<[u8; 4]> {
    let mut pixels = vec![[3, 3, 8, 255]; (width * height) as usize];
    for i in 0..2400u32 {
        let h = hash_u32(i.wrapping_mul(747796405).wrapping_add(2891336453));
        let x = h % width;
        let y = (h >> 10) % height;
        let bright = 140 + ((h >> 20) % 115) as u8;
        pixels[(y * width + x) as usize] = [bright, bright, bright.saturating_sub(8), 255];
    }
    // Warm sun disc in the upper-right of the equirect map.
    let cx = width as i32 * 3 / 4;
    let cy = height as i32 / 3;
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let dx = x - cx;
            let dy = y - cy;
            let d2 = dx * dx + dy * dy;
            if d2 < 18 {
                let idx = (y as u32 * width + x as u32) as usize;
                pixels[idx] = [255, 210, 140, 255];
            } else if d2 < 48 {
                let idx = (y as u32 * width + x as u32) as usize;
                pixels[idx] = [90, 55, 30, 255];
            }
        }
    }
    pixels
}

pub fn format_time(seconds: f32) -> String {
    let m = (seconds / 60.0).floor() as u32;
    let s = seconds - m as f32 * 60.0;
    format!("{m}:{s:05.2}")
}

pub fn hash_u32(x: u32) -> u32 {
    let mut x = x.wrapping_add(0x9E37_79B9);
    x = (x ^ (x >> 16)).wrapping_mul(0x7FEB_352D);
    x = (x ^ (x >> 15)).wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

pub fn hash_frame(dt: f32, p: glam::Vec3) -> f32 {
    let bits = dt.to_bits() ^ p.x.to_bits() ^ p.z.to_bits();
    (hash_u32(bits) >> 8) as f32 / 16_777_215.0
}
