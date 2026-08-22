use std::{env, f32::consts, fs, path::PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use std::time;
#[cfg(target_arch = "wasm32")]
use web_time as time;

use crate::config;
use crate::planet;
use crate::race;
use crate::vehicle;
use crate::vehicle::Isometry;

pub struct Game {
    engine: blade_engine::Engine,
    last_physics_update: time::Instant,
    last_camera_update: time::Instant,
    last_camera_orient: glam::Quat,
    is_paused: bool,
    pub(crate) window: winit::window::Window,
    egui_state: egui_winit::State,
    egui_viewport_id: egui::ViewportId,
    vehicle: vehicle::Vehicle,
    cam_config: config::Camera,
    planet: planet::GeneratedPlanet,
    planet_cfg: config::Planet,
    race: race::Race,
    spawn: Isometry,
    throttle: f32,
    steer: f32,
    throttle_forward: bool,
    throttle_reverse: bool,
    steer_left: bool,
    steer_right: bool,
    dust: usize,
    smoke_frames_left: Option<u32>,
    /// Frame counter for the camera / vehicle state-trace journal.
    frame_index: u32,
}

pub struct QuitEvent;

impl Drop for Game {
    fn drop(&mut self) {
        self.engine.destroy();
    }
}

impl Game {
    pub fn new(event_loop: &winit::event_loop::ActiveEventLoop) -> Self {
        log::info!("Initializing Redline");

        let window = event_loop
            .create_window(
                winit::window::Window::default_attributes().with_title("Redline — Mars Circuit"),
            )
            .unwrap();

        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowExtWebSys as _;
            let canvas = window.canvas().expect("winit canvas");
            canvas.set_id(blade_graphics::CANVAS_ID);
            web_sys::window()
                .and_then(|win| win.document())
                .and_then(|doc| doc.body())
                .and_then(|body| body.append_child(&web_sys::Element::from(canvas)).ok())
                .expect("couldn't append canvas to document body");
        }

        let assets = assets_dir();
        let generated = assets.join("generated");
        let planet_cfg = config::Planet::default();
        let planet = planet::generate(planet_cfg, &generated);

        let ray_trace = env::var_os("REDLINE_RT").is_some();
        let mut engine = blade_engine::Engine::new(
            blade_engine::Presentation::Window(&window),
            &blade_engine::config::Engine {
                shader_path: assets.join("shaders").to_string_lossy().into_owned(),
                data_path: assets.to_string_lossy().into_owned(),
                cache_path: "asset-cache".to_string(),
                time_step: 0.01,
                render_backend: if ray_trace {
                    blade_engine::config::RenderBackend::RayTracer
                } else {
                    blade_engine::config::RenderBackend::Rasterizer
                },
                gui_enabled: true,
            },
        );
        engine.set_gravity(0.0);
        engine.set_average_luminosity(0.18);
        engine.set_raster_config(blade_render::RasterConfig {
            clear_color: blade_graphics::TextureColor::OpaqueBlack,
            light_dir: mint::Vector3 {
                x: 0.45,
                y: 0.72,
                z: 0.28,
            },
            light_color: mint::Vector3 {
                x: 4.2,
                y: 3.1,
                z: 2.2,
            },
            ambient_color: mint::Vector3 {
                x: 0.07,
                y: 0.045,
                z: 0.035,
            },
            space_sky: true,
        });
        // A higher resolution environment keeps individual stars point-like instead of
        // turning every texel into a large square on the sky dome.
        engine.create_environment_map("mars-sky", 1024, 512, &starfield(1024, 512));

        let planet_rel = relative_model(&assets, &planet.planet_model);
        let planet_object = blade_engine::config::Object {
            name: "mars".to_string(),
            visuals: vec![blade_engine::config::Visual {
                model: planet_rel.clone(),
                ..Default::default()
            }],
            colliders: vec![blade_engine::config::Collider {
                density: 1.0,
                friction: 1.35,
                restitution: 0.02,
                shape: blade_engine::config::Shape::TriMesh {
                    model: planet_rel,
                    convex: false,
                    border_radius: 0.0,
                },
                pos: [0.0; 3].into(),
                rot: [0.0; 3].into(),
            }],
            additional_mass: None,
        };
        let _planet_handle = engine.add_object(
            &planet_object,
            blade_engine::Transform::default(),
            blade_engine::DynamicInput::Empty,
        );

        for (index, deco) in planet.decorations.iter().enumerate() {
            let model = relative_model(&assets, &deco.model);
            let object = blade_engine::config::Object {
                name: format!("deco-{index}"),
                visuals: vec![blade_engine::config::Visual {
                    model,
                    scale: deco.scale,
                    ..Default::default()
                }],
                colliders: vec![blade_engine::config::Collider {
                    density: 1.0,
                    friction: 0.9,
                    restitution: if deco.kind == planet::DecorationKind::Crystal {
                        0.15
                    } else {
                        0.05
                    },
                    shape: blade_engine::config::Shape::Cuboid {
                        half: deco.half_extents.into(),
                    },
                    pos: [0.0; 3].into(),
                    rot: [0.0; 3].into(),
                }],
                additional_mass: None,
            };
            let _handle = engine.add_object(
                &object,
                blade_engine::Transform {
                    position: deco.position.into(),
                    orientation: deco.orientation.into(),
                },
                blade_engine::DynamicInput::Empty,
            );
        }

        spawn_props(&mut engine, &planet);

        let veh_config: config::Vehicle =
            ron::de::from_bytes(&read_asset_bytes(&assets.join("vehicle.ron")))
                .expect("unable to parse vehicle config");
        let spawn = vehicle::Vehicle::spawn_pose(&planet.track, 1.4);
        let vehicle = vehicle::spawn(&mut engine, &veh_config, spawn.clone());

        let dust = engine.create_particle_system(
            "dust",
            &blade_particle::ParticleEffect {
                capacity: 4096,
                emitter: blade_particle::Emitter {
                    rate: 0.0,
                    burst_count: 24,
                    shape: blade_particle::EmitterShape::Sphere { radius: 0.4 },
                    cone_angle: 0.8,
                },
                particle: blade_particle::ParticleConfig {
                    life: [0.4, 1.1],
                    speed: [1.0, 4.0],
                    scale: [0.08, 0.28],
                    color: blade_particle::ColorConfig::Palette(vec![
                        [180, 90, 50, 200],
                        [140, 70, 40, 180],
                        [210, 140, 90, 160],
                    ]),
                },
            },
        );

        let race = race::Race::new(&planet.track, config::Race::default());

        let egui_context = egui::Context::default();
        let egui_viewport_id = egui_context.viewport_id();
        let egui_state =
            egui_winit::State::new(egui_context, egui_viewport_id, &window, None, None, None);

        Self {
            engine,
            last_physics_update: time::Instant::now(),
            last_camera_update: time::Instant::now(),
            last_camera_orient: spawn.orientation,
            is_paused: false,
            window,
            egui_state,
            egui_viewport_id,
            vehicle,
            cam_config: config::Camera::default(),
            planet,
            planet_cfg,
            race,
            spawn,
            throttle: 0.0,
            steer: 0.0,
            throttle_forward: false,
            throttle_reverse: false,
            steer_left: false,
            steer_right: false,
            dust,
            smoke_frames_left: smoke_frame_budget(),
            frame_index: 0,
        }
    }

    fn update_time(&mut self) {
        let dt = self.last_physics_update.elapsed().as_secs_f32();
        self.last_physics_update = time::Instant::now();
        if self.is_paused {
            return;
        }
        self.update_vehicle_controls(dt);
        self.vehicle
            .apply_gravity(&mut self.engine, self.planet_cfg.gravity, dt);
        self.vehicle.apply_stability(&mut self.engine, dt);
        self.engine.update(dt);
        let pose = self.vehicle.pose(&self.engine);
        self.race.update(pose.position, dt);
        self.emit_dust(&pose, dt);
    }

    fn emit_dust(&mut self, pose: &Isometry, dt: f32) {
        let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
        let speed = glam::Vec3::from(lin).length();
        if speed < 6.0 {
            return;
        }
        if hash_frame(dt, pose.position) < (speed * 0.015).clamp(0.05, 0.55) {
            let down = -pose.position.normalize_or_zero();
            let pos = pose.position + down * 0.4;
            self.engine
                .particle_burst(self.dust, 12, [pos.x, pos.y, pos.z]);
        }
    }

    fn update_vehicle_controls(&mut self, dt: f32) {
        let throttle_target = match (self.throttle_forward, self.throttle_reverse) {
            (true, false) => 1.0,
            (false, true) => -0.35,
            _ => 0.0,
        };
        let steer_target = match (self.steer_left, self.steer_right) {
            (true, false) => 1.0,
            (false, true) => -1.0,
            _ => 0.0,
        };

        // Frame-rate independent response. Steering returns more quickly than it turns in,
        // which gives the wheel a positive, natural self-centering feel.
        let throttle_response = 1.0 - (-dt.min(0.1) * 10.0).exp();
        let steer_speed = if steer_target == 0.0 { 18.0 } else { 14.0 };
        let steer_response = 1.0 - (-dt.min(0.1) * steer_speed).exp();
        self.throttle += (throttle_target - self.throttle) * throttle_response;
        self.steer += (steer_target - self.steer) * steer_response;

        let (linear, _) = self.engine.get_velocity(self.vehicle.body_handle);
        let speed = glam::Vec3::from(linear).length();
        let steering_limit = 0.52 * (1.0 / (1.0 + speed / 48.0)).clamp(0.42, 1.0);
        self.vehicle
            .set_velocity(&mut self.engine, self.throttle * 110.0);
        self.vehicle
            .set_steering(&mut self.engine, self.steer * steering_limit);
    }
}

fn smoke_frame_budget() -> Option<u32> {
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

fn assets_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

fn read_asset_bytes(path: &std::path::Path) -> Vec<u8> {
    if let Some(bytes) = blade_engine::vfs::read(path) {
        return bytes;
    }
    fs::read(path).unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()))
}

fn relative_model(assets: &std::path::Path, model: &std::path::Path) -> String {
    model
        .strip_prefix(assets)
        .unwrap_or(model)
        .to_string_lossy()
        .into_owned()
}

fn spawn_props(engine: &mut blade_engine::Engine, planet: &planet::GeneratedPlanet) {
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

fn project_tangent(v: glam::Vec3, up: glam::Vec3) -> glam::Vec3 {
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

fn starfield(width: u32, height: u32) -> Vec<[u8; 4]> {
    let mut pixels = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        let v = (y as f32 + 0.5) / height as f32;
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let band = ((v - 0.58 - 0.10 * (u * consts::TAU).sin()).abs() / 0.09).powi(2);
            let dust = (-band).exp();
            let grain = (hash_u32(x.wrapping_add(y.wrapping_mul(width))) & 15) as f32 / 15.0;
            pixels.push([
                (2.0 + dust * (5.0 + grain * 3.0)) as u8,
                (3.0 + dust * (4.0 + grain * 2.0)) as u8,
                (7.0 + dust * (7.0 + grain * 4.0)) as u8,
                255,
            ]);
        }
    }

    // Most stars are dim and neutral; a few carry the subtle warm/cool color seen
    // in real stellar populations. One texel at this resolution is only 0.35° wide.
    for i in 0..3100u32 {
        let h = hash_u32(i.wrapping_mul(747_796_405).wrapping_add(2_891_336_453));
        let x = h % width;
        let y = hash_u32(h ^ 0xA341_316C) % height;
        let magnitude = ((h >> 16) & 0xFF) as f32 / 255.0;
        let bright = (55.0 + 200.0 * magnitude.powf(3.2)) as u8;
        let tint = (h >> 8) & 3;
        let color = match tint {
            0 => [
                bright,
                bright.saturating_sub(14),
                bright.saturating_sub(30),
                255,
            ],
            1 => [
                bright.saturating_sub(18),
                bright.saturating_sub(8),
                bright,
                255,
            ],
            _ => [bright, bright, bright.saturating_sub(4), 255],
        };
        pixels[(y * width + x) as usize] = color;
    }

    // A distant, small sun with a restrained one-pixel corona.
    let sun_x = width * 3 / 4;
    let sun_y = height / 3;
    pixels[(sun_y * width + sun_x) as usize] = [255, 236, 205, 255];
    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
        let x = (sun_x as i32 + dx) as u32;
        let y = (sun_y as i32 + dy) as u32;
        pixels[(y * width + x) as usize] = [92, 66, 48, 255];
    }
    pixels
}

fn format_time(seconds: f32) -> String {
    let m = (seconds / 60.0).floor() as u32;
    let s = seconds - m as f32 * 60.0;
    format!("{m}:{s:05.2}")
}

fn hash_u32(x: u32) -> u32 {
    let mut x = x.wrapping_add(0x9E37_79B9);
    x = (x ^ (x >> 16)).wrapping_mul(0x7FEB_352D);
    x = (x ^ (x >> 15)).wrapping_mul(0x846C_A68B);
    x ^ (x >> 16)
}

fn hash_frame(dt: f32, p: glam::Vec3) -> f32 {
    let bits = dt.to_bits() ^ p.x.to_bits() ^ p.z.to_bits();
    (hash_u32(bits) >> 8) as f32 / 16_777_215.0
}

include!("game_ui.inc.rs");

#[cfg(test)]
mod tests {
    #[test]
    fn starfield_is_sparse_and_point_like() {
        let sky = super::starfield(512, 256);
        assert_eq!(sky.len(), 512 * 256);
        let luminous = sky
            .iter()
            .filter(|pixel| pixel[0].max(pixel[1]).max(pixel[2]) > 50)
            .count();
        assert!(luminous > 1_000);
        assert!(
            luminous < sky.len() / 30,
            "stars should not flatten the sky"
        );
    }
}
