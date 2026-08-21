use std::{env, f32::consts};

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

        let assets = crate::helpers::assets_dir();
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
                ray_trace,
            },
        );

        crate::helpers::spawn_props(&mut engine, &planet);

        let veh_config: config::Vehicle =
            ron::de::from_bytes(&crate::helpers::read_asset_bytes(&assets.join("vehicle.ron")))
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
            smoke_frames_left: crate::helpers::smoke_frame_budget(),
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
        if crate::helpers::hash_frame(dt, pose.position) < (speed * 0.015).clamp(0.05, 0.55) {
            let down = -pose.position.normalize_or_zero();
            let pos = pose.position + down * 0.4;
            self.engine
                .particle_burst(self.dust, 12, [pos.x, pos.y, pos.z]);
        }
    }

}

include!("game_ui.inc.rs");
