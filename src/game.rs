use std::{
    collections::{HashMap, HashSet},
    env,
    f32::consts,
    fs,
    path::PathBuf,
};

#[cfg(not(target_arch = "wasm32"))]
use std::time;
#[cfg(target_arch = "wasm32")]
use web_time as time;

use crate::ai;
use crate::config;
use crate::control;
use crate::planet;
use crate::race;
use crate::trace;
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
    ai_drivers: Vec<ai::Driver>,
    cam_config: config::Camera,
    planet: planet::GeneratedPlanet,
    planet_cfg: config::Planet,
    race: race::Race,
    spawn: Isometry,
    controller: control::PlayerController,
    throttle_forward: bool,
    throttle_reverse: bool,
    steer_left: bool,
    steer_right: bool,
    dust: usize,
    smoke_frames_left: Option<u32>,
    /// Frame counter for the camera / vehicle state-trace journal.
    frame_index: u32,
    script: Option<trace::Script>,
    recorder: Option<trace::Recorder>,
    record_seconds: Option<f32>,
    sim_time: f32,
    recovered_this_step: u8,
}

pub struct QuitEvent;

impl Drop for Game {
    fn drop(&mut self) {
        if let Some(recorder) = self.recorder.take() {
            recorder.finish();
        }
        self.engine.destroy();
    }
}

impl Game {
    pub fn new(event_loop: &winit::event_loop::ActiveEventLoop) -> Self {
        log::info!("Initializing Redline");

        let window = event_loop
            .create_window(
                winit::window::Window::default_attributes()
                    .with_title("Redline — Mars Circuit")
                    .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
            )
            .unwrap();

        #[cfg(target_arch = "wasm32")]
        {
            use winit::platform::web::WindowExtWebSys as _;
            let canvas = window.canvas().expect("winit canvas");
            canvas.set_id(blade_graphics::CANVAS_ID);
            // CSS size fills the page; the drawing buffer is a separate
            // canvas.width/height and defaults to 300x150.
            let _ = canvas.style().set_property("width", "100%");
            let _ = canvas.style().set_property("height", "100%");
            web_sys::window()
                .and_then(|win| win.document())
                .and_then(|doc| doc.body())
                .and_then(|body| body.append_child(&web_sys::Element::from(canvas)).ok())
                .expect("couldn't append canvas to document body");
            sync_web_canvas(&window);
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
            light_dir: planet::SUN_DIRECTION.into(),
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
            directional_shadows: if cfg!(target_arch = "wasm32") {
                None
            } else {
                Some(blade_render::DirectionalShadowConfig {
                    resolution: 512,
                    distance: 52.0,
                    depth: 220.0,
                    strength: 0.9,
                    normal_bias: 0.07,
                })
            },
            ..Default::default()
        });
        // A higher resolution environment keeps individual stars point-like instead of
        // turning every texel into a large square on the sky dome. WebGL is happier
        // with a smaller map at boot.
        let (sky_w, sky_h) = if cfg!(target_arch = "wasm32") {
            (512, 256)
        } else {
            (1024, 512)
        };
        engine.create_environment_map("mars-sky", sky_w, sky_h, &starfield(sky_w, sky_h));

        let planet_rel = relative_model(&assets, &planet.planet_model);
        let planet_object = blade_engine::config::Object {
            name: "mars".to_string(),
            visuals: vec![blade_engine::config::Visual {
                model: planet_rel.clone(),
                ..Default::default()
            }],
            colliders: vec![blade_engine::config::Collider {
                density: 1.0,
                friction: 0.85,
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
                    friction: 0.35,
                    restitution: if deco.kind == planet::DecorationKind::Crystal {
                        0.08
                    } else {
                        0.02
                    },
                    shape: blade_engine::config::Shape::ConvexHull {
                        points: deco
                            .collider_points
                            .iter()
                            .map(|point| (*point).into())
                            .collect(),
                        border_radius: 0.025,
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
        let cli = parse_cli();
        let spawn = vehicle::Vehicle::spawn_pose(&planet.track, vehicle::SPAWN_HOVER);
        let vehicle = vehicle::spawn(&mut engine, &veh_config, spawn.clone(), None);
        let ai_drivers = if cli.script.is_some() {
            Vec::new()
        } else {
            opponent_specs()
                .iter()
                .copied()
                .map(|spec| {
                    ai::Driver::spawn(
                        &mut engine,
                        &veh_config,
                        &planet.track,
                        spec.index,
                        spec.lane,
                        spec.speed,
                        spec.kit,
                    )
                })
                .collect()
        };

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
            ai_drivers,
            cam_config: config::Camera::default(),
            planet,
            planet_cfg,
            race,
            spawn,
            controller: control::PlayerController::default(),
            throttle_forward: false,
            throttle_reverse: false,
            steer_left: false,
            steer_right: false,
            dust,
            smoke_frames_left: cli.smoke_frames,
            frame_index: 0,
            script: cli.script,
            recorder: cli
                .record_path
                .map(|path| trace::Recorder::new(path, cli.script.unwrap_or(trace::Script::Lap))),
            record_seconds: cli.seconds,
            sim_time: 0.0,
            recovered_this_step: 0,
        }
    }

    fn update_time(&mut self) {
        let wall = self.last_physics_update.elapsed().as_secs_f32();
        self.last_physics_update = time::Instant::now();
        // Scripted traces use a fixed step so joint/control analysis is not
        // dominated by whatever frame time lavapipe happened to deliver.
        let dt = if self.script.is_some() {
            0.01
        } else {
            wall.min(0.05)
        };
        if self.is_paused {
            return;
        }
        self.sim_time += dt;
        self.update_vehicle_controls(dt);
        self.vehicle
            .apply_gravity(&mut self.engine, self.planet_cfg.gravity, dt);
        self.vehicle.apply_stability(&mut self.engine, dt);
        for driver in self.ai_drivers.iter_mut() {
            driver.update(
                &mut self.engine,
                &self.planet.track,
                self.planet_cfg.gravity,
                dt,
            );
        }
        self.engine.update(dt);
        self.apply_vehicle_bumps();
        self.recovered_this_step = u8::from(self.vehicle.recover_if_needed(
            &mut self.engine,
            &self.planet.track,
            self.planet.track_width,
            vehicle::SPAWN_HOVER,
            self.controller.throttle().abs() > 0.15,
            dt,
        ));
        let (pose, linear, angular, forward_speed, lateral_speed) =
            self.vehicle.motion(&self.engine);
        self.race.update(pose.position, dt);
        self.emit_dust(&pose, dt);
        self.record_sample(&pose, linear, angular, forward_speed, lateral_speed);
    }

    fn emit_dust(&mut self, pose: &Isometry, dt: f32) {
        let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
        let vel = glam::Vec3::from(lin);
        let speed = vel.length();
        if speed < 4.0 {
            return;
        }
        if hash_frame(dt, pose.position) > (speed * 0.045).clamp(0.1, 0.85) {
            return;
        }
        let up = pose.position.normalize_or_zero();
        let spray = (up * 0.4 - vel.normalize_or_zero() * 0.75).normalize_or_zero();
        if let Some(system) = self.engine.particle_system_mut(self.dust) {
            system.axis = [spray.x, spray.y, spray.z];
        }
        let radius = self.vehicle.wheel_radius();
        let count = if speed > 16.0 { 5 } else { 3 };
        for wheel in self.vehicle.wheels.iter() {
            let pos = glam::Vec3::from(self.engine.get_object_position(wheel.object));
            let contact = pos - up * radius;
            self.engine
                .particle_burst(self.dust, count, [contact.x, contact.y, contact.z]);
        }
    }

    fn update_vehicle_controls(&mut self, dt: f32) {
        let pose = self.vehicle.pose(&self.engine);
        let (linear, _) = self.engine.get_velocity(self.vehicle.body_handle);
        let speed = glam::Vec3::from(linear).length();
        let query = planet::query_track(pose.position, &self.planet.track);
        let off_track = planet::off_track_distance(&query, self.planet.track_width);
        let heading = trace::look_ahead_heading(&pose, &self.planet.track, speed, true);
        let command = if let Some(script) = self.script {
            let (throttle, steer) = script.analog(self.sim_time, heading, off_track);
            self.controller.analog_command(throttle, steer, speed, dt)
        } else {
            self.controller.update(
                control::Input {
                    throttle_forward: self.throttle_forward,
                    throttle_reverse: self.throttle_reverse,
                    steer_left: self.steer_left,
                    steer_right: self.steer_right,
                },
                speed,
                dt,
            )
        };
        self.vehicle.drive(
            &mut self.engine,
            command.target_speed,
            command.steering_angle,
            dt,
        );
    }

    fn record_sample(
        &mut self,
        pose: &Isometry,
        linear: glam::Vec3,
        angular: glam::Vec3,
        forward_speed: f32,
        lateral_speed: f32,
    ) {
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        let up = pose.position.normalize_or_zero();
        let query = planet::query_track(pose.position, &self.planet.track);
        recorder.push(trace::Sample {
            t: self.sim_time,
            throttle: self.controller.throttle(),
            steer: self.controller.steering(),
            position: pose.position,
            speed: linear.length(),
            forward_speed,
            lateral_speed,
            yaw_rate: angular.dot(up),
            upright: (pose.orientation * glam::Vec3::Y).dot(up),
            off_track: planet::off_track_distance(&query, self.planet.track_width),
            heading_error: trace::look_ahead_heading(
                pose,
                &self.planet.track,
                linear.length(),
                true,
            ),
            recovered: self.recovered_this_step,
        });
    }

    fn update_local_lights(&mut self, eye: glam::Vec3) {
        let mut ranked = self
            .planet
            .decorations
            .iter()
            .filter_map(|deco| {
                let color = deco.glow?;
                let delta = deco.position - eye;
                let dist2 = delta.length_squared().max(4.0);
                let intensity = color[0].max(color[1]).max(color[2]);
                Some((
                    intensity / dist2,
                    blade_render::PointLight {
                        position: deco.position.into(),
                        color: mint::Vector3 {
                            x: color[0],
                            y: color[1],
                            z: color[2],
                        },
                        radius: 11.0 + deco.scale * 0.65,
                    },
                ))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let lights: Vec<_> = ranked
            .into_iter()
            .take(blade_render::MAX_POINT_LIGHTS)
            .map(|(_, light)| light)
            .collect();
        self.engine.set_point_lights(&lights);
    }

    fn apply_vehicle_bumps(&mut self) {
        let mut owner = HashMap::new();
        for handle in self.vehicle.bump_handles() {
            owner.insert(handle, 0usize);
        }
        for (index, driver) in self.ai_drivers.iter().enumerate() {
            for handle in driver.vehicle.bump_handles() {
                owner.insert(handle, index + 1);
            }
        }
        let mut pairs = HashSet::new();
        for contact in self.engine.drain_contacts() {
            let Some(&a) = owner.get(&contact.object_a) else {
                continue;
            };
            let Some(&b) = owner.get(&contact.object_b) else {
                continue;
            };
            if a != b {
                pairs.insert(if a < b { (a, b) } else { (b, a) });
            }
        }
        for (a, b) in pairs {
            let (handle_a, pos_a, mass_a, recoil_a) = self.bump_state(a);
            let (handle_b, pos_b, mass_b, recoil_b) = self.bump_state(b);
            let up = ((pos_a + pos_b) * 0.5).normalize_or_zero();
            let lateral = (pos_b - pos_a).reject_from(up);
            if lateral.length_squared() > 1e-5 {
                let dir = lateral.normalize();
                if recoil_a <= 0.08 {
                    self.engine
                        .apply_linear_impulse(handle_a, (-dir * mass_a * 8.0).into());
                    self.engine.wake_up(handle_a);
                }
                if recoil_b <= 0.08 {
                    self.engine
                        .apply_linear_impulse(handle_b, (dir * mass_b * 8.0).into());
                    self.engine.wake_up(handle_b);
                }
            }
            self.vehicle_mut(a).register_bump();
            self.vehicle_mut(b).register_bump();
        }
    }

    fn bump_state(&self, id: usize) -> (blade_engine::ObjectHandle, glam::Vec3, f32, f32) {
        let vehicle = if id == 0 {
            &self.vehicle
        } else {
            &self.ai_drivers[id - 1].vehicle
        };
        (
            vehicle.body_handle,
            vehicle.pose(&self.engine).position,
            vehicle.body_mass,
            vehicle.recoil_time(),
        )
    }

    fn vehicle_mut(&mut self, id: usize) -> &mut vehicle::Vehicle {
        if id == 0 {
            &mut self.vehicle
        } else {
            &mut self.ai_drivers[id - 1].vehicle
        }
    }

    pub(crate) fn script_finished(&self) -> bool {
        self.record_seconds
            .is_some_and(|limit| self.sim_time >= limit)
    }
}

struct Cli {
    smoke_frames: Option<u32>,
    record_path: Option<PathBuf>,
    script: Option<trace::Script>,
    seconds: Option<f32>,
}

#[derive(Clone, Copy)]
struct OpponentSpec {
    index: usize,
    lane: f32,
    speed: f32,
    kit: vehicle::Kit,
}

const KIT_HATCH: vehicle::Kit = vehicle::Kit {
    body_model: "models/hatchback-sports-body.glb",
    wheel_model: "models/wheel-racing.glb",
    tint: [1.0, 0.42, 0.32, 1.0],
    half_track: 0.32,
};
#[allow(dead_code)]
const KIT_SEDAN: vehicle::Kit = vehicle::Kit {
    body_model: "models/sedan-sports-body.glb",
    wheel_model: "models/wheel-dark.glb",
    tint: [0.42, 0.72, 1.0, 1.0],
    half_track: 0.32,
};
#[allow(dead_code)]
const KIT_TAXI: vehicle::Kit = vehicle::Kit {
    body_model: "models/taxi-body.glb",
    wheel_model: "models/wheel-dark.glb",
    tint: [1.0, 1.0, 1.0, 1.0],
    half_track: 0.32,
};

fn opponent_specs() -> &'static [OpponentSpec] {
    // Extra vehicles are a full Rapier joint graph each. Debug keeps a lighter
    // set so the scene still boots at an interactive rate.
    #[cfg(target_arch = "wasm32")]
    {
        static SPECS: [OpponentSpec; 2] = [
            OpponentSpec {
                index: 8,
                lane: -2.1,
                speed: 15.5,
                kit: KIT_HATCH,
            },
            OpponentSpec {
                index: 13,
                lane: 2.0,
                speed: 14.5,
                kit: KIT_TAXI,
            },
        ];
        &SPECS
    }
    #[cfg(all(not(target_arch = "wasm32"), debug_assertions))]
    {
        static SPECS: [OpponentSpec; 1] = [OpponentSpec {
            index: 8,
            lane: -2.1,
            speed: 15.5,
            kit: KIT_HATCH,
        }];
        &SPECS
    }
    #[cfg(all(not(target_arch = "wasm32"), not(debug_assertions)))]
    {
        static SPECS: [OpponentSpec; 3] = [
            OpponentSpec {
                index: 8,
                lane: -2.1,
                speed: 15.5,
                kit: KIT_HATCH,
            },
            OpponentSpec {
                index: 13,
                lane: 2.0,
                speed: 14.5,
                kit: KIT_SEDAN,
            },
            OpponentSpec {
                index: 18,
                lane: -0.6,
                speed: 16.5,
                kit: KIT_TAXI,
            },
        ];
        &SPECS
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_cli() -> Cli {
    Cli {
        smoke_frames: None,
        record_path: None,
        script: None,
        seconds: None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_cli() -> Cli {
    let mut cli = Cli {
        smoke_frames: None,
        record_path: None,
        script: None,
        seconds: None,
    };
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--smoke" => {
                cli.smoke_frames = Some(
                    args.next()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(20),
                );
            }
            "--record" => {
                cli.record_path = args.next().map(PathBuf::from);
            }
            "--script" => {
                let name = args.next().unwrap_or_else(|| "lap".to_string());
                cli.script = Some(
                    trace::Script::parse(&name)
                        .unwrap_or_else(|| panic!("unknown script '{name}'")),
                );
            }
            "--seconds" => {
                cli.seconds = args.next().and_then(|value| value.parse().ok());
            }
            _ => {}
        }
    }
    if cli.smoke_frames.is_none() {
        cli.smoke_frames = match env::var("REDLINE_SMOKE") {
            Ok(value) if value.is_empty() || value == "1" => Some(20),
            Ok(value) => Some(value.parse().expect("REDLINE_SMOKE must be a frame count")),
            Err(_) => None,
        };
    }
    if cli.record_path.is_none()
        && let Some(script) = cli.script
    {
        cli.record_path = Some(PathBuf::from(format!(
            "/tmp/redline-{}.csv",
            script.as_str()
        )));
    }
    if cli.seconds.is_none() && (cli.script.is_some() || cli.record_path.is_some()) {
        cli.seconds = Some(10.0);
    }
    cli
}

/// WebGL's drawing buffer is `canvas.width` x `canvas.height`, not the CSS
/// size. If those stay at the HTML default (300x150) while Blade renders at
/// the window's device pixels, present blit crops a corner of the frame and
/// the chase camera looks like a close-up of the planet.
#[cfg(target_arch = "wasm32")]
fn sync_web_canvas(window: &winit::window::Window) {
    use winit::platform::web::WindowExtWebSys as _;
    let Some(canvas) = window.canvas() else {
        return;
    };
    let size = window.inner_size();
    let width = size.width.max(1);
    let height = size.height.max(1);
    if canvas.width() != width {
        canvas.set_width(width);
    }
    if canvas.height() != height {
        canvas.set_height(height);
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
    let sun = planet::SUN_DIRECTION.normalize();
    let sun_u = (sun.x.atan2(sun.z) / consts::PI + 1.0) * 0.5;
    let sun_v = sun.y.asin() / consts::PI + 0.5;
    let sun_x = (sun_u * width as f32) as u32 % width;
    let sun_y = (sun_v * height as f32) as u32 % height;
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
