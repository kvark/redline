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
        engine.create_environment_map("mars-sky", 256, 128, &starfield(256, 128));

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
        self.vehicle
            .apply_gravity(&mut self.engine, self.planet_cfg.gravity, dt);
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
    let mut pixels = vec![[3, 3, 8, 255]; (width * height) as usize];
    for i in 0..2400u32 {
        let h = hash_u32(i.wrapping_mul(747796405).wrapping_add(2891336453));
        let x = h % width;
        let y = (h >> 10) % height;
        let bright = 140 + ((h >> 20) % 115) as u8;
        pixels[(y * width + x) as usize] = [bright, bright, bright.saturating_sub(8), 255];
    }
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
