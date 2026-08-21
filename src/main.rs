#![allow(
    irrefutable_let_patterns,
    clippy::new_without_default,
    clippy::needless_borrowed_reference
)]
#![warn(
    trivial_casts,
    trivial_numeric_casts,
    unused_extern_crates,
    unused_qualifications,
    clippy::pattern_type_mismatch
)]

mod config;
mod glb;
mod planet;
mod race;
mod vehicle;

use std::{env, f32::consts, fs, path::PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use std::time;
#[cfg(target_arch = "wasm32")]
use web_time as time;

use vehicle::Isometry;

struct Game {
    engine: blade_engine::Engine,
    last_physics_update: time::Instant,
    last_camera_update: time::Instant,
    last_camera_orient: glam::Quat,
    is_paused: bool,
    window: winit::window::Window,
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
    /// Frame counter used by the camera journal / state trace.
    frame_index: u32,
}

struct QuitEvent;

impl Drop for Game {
    fn drop(&mut self) {
        self.engine.destroy();
    }
}

impl Game {
    fn new(event_loop: &winit::event_loop::ActiveEventLoop) -> Self {
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
        #[cfg(target_arch = "wasm32")]
        mount_embedded_assets();

        let mut engine = blade_engine::Engine::new(&window, blade_engine::config::Engine {
            ray_trace: env::var_os("REDLINE_RT").is_some(),
            ..Default::default()
        });

        let planet_cfg = config::Planet::default();
        let planet = planet::generate(&mut engine, planet_cfg, &assets);

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
        if hash_frame(dt, pose.position) < (speed * 0.015).clamp(0.05, 0.55) {
            let down = -pose.position.normalize_or_zero();
            let pos = pose.position + down * 0.4;
            self.engine.emit_particles(
                self.dust,
                blade_particle::EmitRequest {
                    position: pos.into(),
                    direction: down.into(),
                    count: 1,
                },
            );
        }
    }

    fn on_event(
        &mut self,
        event: &winit::event::WindowEvent,
    ) -> Result<winit::event_loop::ControlFlow, QuitEvent> {
        let response = self.egui_state.on_window_event(&self.window, event);
        if response.repaint {
            self.window.request_redraw();
        }
        if response.consumed {
            return Ok(winit::event_loop::ControlFlow::Poll);
        }

        match event {
            winit::event::WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key: winit::keyboard::PhysicalKey::Code(code),
                        state: winit::event::ElementState::Pressed,
                        ..
                    },
                ..
            } => match code {
                winit::keyboard::KeyCode::Escape => return Err(QuitEvent),
                winit::keyboard::KeyCode::KeyR => self.respawn(),
                winit::keyboard::KeyCode::Space => {
                    let pose = self.vehicle.pose(&self.engine);
                    let up = pose.position.normalize_or_zero();
                    self.engine.apply_impulse(
                        self.vehicle.body_handle,
                        (self.vehicle.jump_impulse * up).into(),
                        [0.0; 3],
                    );
                }
                winit::keyboard::KeyCode::Comma => {
                    let pose = self.vehicle.pose(&self.engine);
                    let forward = pose.orientation * glam::Vec3::Z;
                    self.engine.apply_impulse(
                        self.vehicle.body_handle,
                        [0.0; 3],
                        (self.vehicle.roll_impulse * forward).into(),
                    );
                }
                winit::keyboard::KeyCode::Period => {
                    let pose = self.vehicle.pose(&self.engine);
                    let forward = pose.orientation * glam::Vec3::Z;
                    self.engine.apply_impulse(
                        self.vehicle.body_handle,
                        [0.0; 3],
                        (-self.vehicle.roll_impulse * forward).into(),
                    );
                }
                winit::keyboard::KeyCode::ArrowUp
                | winit::keyboard::KeyCode::KeyW => self.throttle = 1.0,
                winit::keyboard::KeyCode::ArrowDown
                | winit::keyboard::KeyCode::KeyS => self.throttle = -1.0,
                winit::keyboard::KeyCode::ArrowLeft
                | winit::keyboard::KeyCode::KeyA => self.steer = -1.0,
                winit::keyboard::KeyCode::ArrowRight
                | winit::keyboard::KeyCode::KeyD => self.steer = 1.0,
                _ => {}
            },
            winit::event::WindowEvent::KeyboardInput {
                event:
                    winit::event::KeyEvent {
                        physical_key: winit::keyboard::PhysicalKey::Code(code),
                        state: winit::event::ElementState::Released,
                        ..
                    },
                ..
            } => match code {
                winit::keyboard::KeyCode::ArrowUp
                | winit::keyboard::KeyCode::KeyW
                | winit::keyboard::KeyCode::ArrowDown
                | winit::keyboard::KeyCode::KeyS => self.throttle = 0.0,
                winit::keyboard::KeyCode::ArrowLeft
                | winit::keyboard::KeyCode::KeyA
                | winit::keyboard::KeyCode::ArrowRight
                | winit::keyboard::KeyCode::KeyD => self.steer = 0.0,
                _ => {}
            },
            winit::event::WindowEvent::CloseRequested => return Err(QuitEvent),
            winit::event::WindowEvent::RedrawRequested => {
                let wait = self.on_draw();
                if let Some(ref mut left) = self.smoke_frames_left {
                    *left = left.saturating_sub(1);
                    if *left == 0 {
                        log::info!("Smoke test finished");
                        return Err(QuitEvent);
                    }
                }
                return Ok(if let Some(when) = time::Instant::now().checked_add(wait) {
                    winit::event_loop::ControlFlow::WaitUntil(when)
                } else {
                    winit::event_loop::ControlFlow::Wait
                });
            }
            _ => {}
        }

        // Apply continuous vehicle controls while keys are held.
        if self.throttle != 0.0 || self.steer != 0.0 {
            if matches!(
                event,
                winit::event::WindowEvent::KeyboardInput {
                    event:
                        winit::event::KeyEvent {
                            physical_key: winit::keyboard::PhysicalKey::Code(
                                winit::keyboard::KeyCode::ArrowUp
                                | winit::keyboard::KeyCode::KeyW
                                | winit::keyboard::KeyCode::ArrowDown
                                | winit::keyboard::KeyCode::KeyS
                                | winit::keyboard::KeyCode::ArrowLeft
                                | winit::keyboard::KeyCode::KeyA
                                | winit::keyboard::KeyCode::ArrowRight
                                | winit::keyboard::KeyCode::KeyD,
                            ),
                            ..
                        },
                    ..
                }
            ) {
                self.vehicle
                    .set_velocity(&mut self.engine, self.throttle * 110.0);
                self.vehicle
                    .set_steering(&mut self.engine, self.steer * 0.9);
            }
        }

        Ok(winit::event_loop::ControlFlow::Poll)
    }

    fn respawn(&mut self) {
        self.vehicle.teleport(&mut self.engine, &self.spawn);
        self.race.reset();
    }

    fn recover(&mut self) {
        let pose = self.vehicle.pose(&self.engine);
        let up = pose.position.normalize_or_zero();
        let fwd = project_tangent(pose.orientation * glam::Vec3::Z, up);
        let next = Isometry {
            position: pose.position + up * 2.0,
            orientation: planet::surface_quat(up, fwd),
        };
        self.vehicle.teleport(&mut self.engine, &next);
    }

    fn on_draw(&mut self) -> time::Duration {
        self.update_time();

        let raw_input = self.egui_state.take_egui_input(&self.window);
        let egui_output = self.egui_state.egui_ctx().run(raw_input, |ctx| {
            egui::Window::new("HUD")
                .default_pos([12.0, 12.0])
                .default_width(220.0)
                .show(ctx, |ui| {
                    ui.label(format!("Lap {}/{}", self.race.lap + 1, self.race.laps));
                    ui.label(format!(
                        "Checkpoint {}/{}",
                        self.race.next_checkpoint + 1,
                        self.race.checkpoints.len()
                    ));
                    if self.race.finished {
                        ui.colored_label(egui::Color32::LIGHT_GREEN, "Circuit complete");
                    }
                    let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
                    ui.label(format!("Speed {:.0} m/s", glam::Vec3::from(lin).length()));
                    ui.separator();
                    ui.label("W/↑ throttle   S/↓ brake   A/D steer");
                    ui.label("R respawn   Space jump   ,/. roll");

                    egui::CollapsingHeader::new("Camera")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.add(
                                egui::Slider::new(&mut self.cam_config.distance, 3.0..=40.0)
                                    .text("Distance"),
                            );
                            ui.add(
                                egui::Slider::new(&mut self.cam_config.height, 0.4..=8.0)
                                    .text("Height"),
                            );
                            ui.add(
                                egui::Slider::new(
                                    &mut self.cam_config.azimuth,
                                    -consts::PI..=consts::PI,
                                )
                                .text("Azimuth"),
                            );
                            ui.add(
                                egui::Slider::new(
                                    &mut self.cam_config.altitude,
                                    -0.15..=1.1,
                                )
                                .text("Altitude"),
                            );
                            ui.add(
                                egui::Slider::new(&mut self.cam_config.fov, 0.6..=1.6)
                                    .text("FOV"),
                            );
                            ui.toggle_value(&mut self.is_paused, "Pause");
                        });

                    ui.horizontal(|ui| {
                        if ui.button("Recover").clicked() {
                            self.recover();
                        }
                        if ui.button("Respawn").clicked() {
                            self.respawn();
                        }
                    });
                });
        });

        self.egui_state
            .handle_platform_output(&self.window, egui_output.platform_output);

        let camera = self.follow_camera();
        let primitives = self
            .egui_state
            .egui_ctx()
            .tessellate(egui_output.shapes, egui_output.pixels_per_point);
        self.engine.render(
            &camera,
            &primitives,
            &egui_output.textures_delta,
            self.window.inner_size(),
            self.window.scale_factor() as f32,
        );
        egui_output.viewport_output[&self.egui_viewport_id].repaint_delay
    }

    fn follow_camera(&mut self) -> blade_engine::FrameCamera {
        let pose = self.vehicle.pose(&self.engine);
        // Radial outward from planet center — the only reliable "up" on a sphere.
        let up = pose.position.normalize_or_zero();
        let fwd = project_tangent(pose.orientation * glam::Vec3::Z, up);
        let desired = planet::surface_quat(up, fwd);

        let dt = self.last_camera_update.elapsed().as_secs_f32();
        self.last_camera_update = time::Instant::now();
        let t = 1.0 - (-dt * self.cam_config.speed).exp();
        let smooth = self.last_camera_orient.slerp(desired, t.clamp(0.0, 1.0));
        self.last_camera_orient = smooth;

        let cc = &self.cam_config;
        let back = smooth * -glam::Vec3::Z;
        let cam_up = smooth * glam::Vec3::Y;
        let cam_right = smooth * glam::Vec3::X;
        let yaw = glam::Quat::from_axis_angle(cam_up, cc.azimuth);
        // Positive altitude raises the camera (we look slightly down at the car).
        // The previous sign was inverted, so the default altitude=0.35 placed the
        // eye *under* the vehicle — that is what produced the "from the bottom"
        // view. Radial clamps below still guard against extreme negative values
        // and against the camera falling underground.
        let pitch = glam::Quat::from_axis_angle(cam_right, cc.altitude);
        let orbit = yaw * pitch;
        let mut offset = orbit * (back * cc.distance + cam_up * cc.height);

        // Keep the eye strictly above the car (positive radial component).
        // This is the primary defence against "looking from the bottom".
        let min_above = (cc.height * 0.55).max(1.0);
        let radial_before = offset.dot(up);
        if radial_before < min_above {
            offset += up * (min_above - radial_before);
        }

        let mut eye = pose.position + offset;

        // Never let the camera fall underground relative to the vehicle radius.
        // Terrain height variation (~height_amp) is small vs chase distance.
        let vehicle_r = pose.position.length();
        let eye_r = eye.length();
        let min_r = vehicle_r + 0.6;
        if eye_r < min_r {
            eye = eye.normalize_or_zero() * min_r;
        }

        let target = pose.position + cam_up * 0.6;
        let view = glam::Mat4::look_at_rh(eye, target, cam_up);
        let world = view.inverse();
        let (_, rot, trans) = world.to_scale_rotation_translation();

        // Lightweight state-trace journal for vehicle / camera debugging.
        // A short --smoke run already yields several samples.
        if self.frame_index % 8 == 0 {
            let rel_h = (eye - pose.position).dot(up);
            let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
            let speed = glam::Vec3::from(lin).length();
            log::debug!(
                "cam_trace frame={} veh_r={:.2} eye_r={:.2} rel_h={:.2} radial_before={:.2} speed={:.1} cam_up·up={:.3} alt={:.2}",
                self.frame_index,
                vehicle_r,
                eye.length(),
                rel_h,
                radial_before,
                speed,
                cam_up.dot(up),
                cc.altitude,
            );
        }
        self.frame_index = self.frame_index.wrapping_add(1);

        blade_engine::FrameCamera {
            transform: blade_engine::Transform {
                position: trans.into(),
                orientation: rot.into(),
            },
            fov_y: cc.fov,
        }
    }
}

struct App {
    game: Option<Game>,
}

impl winit::application::ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        self.game = Some(Game::new(event_loop));
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if let Some(game) = self.game.as_ref() {
            game.window.request_redraw();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        let game = self.game.as_mut().unwrap();
        match game.on_event(&event) {
            Ok(control_flow) => event_loop.set_control_flow(control_flow),
            Err(QuitEvent) => event_loop.exit(),
        }
    }
}

fn main() {
    #[cfg(not(target_arch = "wasm32"))]
    env_logger::init();
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        console_log::init_with_level(log::Level::Info).ok();
    }
    let event_loop = winit::event_loop::EventLoop::new().unwrap();
    let mut app = App { game: None };
    event_loop.run_app(&mut app).unwrap();
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

#[cfg(target_arch = "wasm32")]
fn mount_embedded_assets() {
    use include_dir::{Dir, include_dir};
    static ASSETS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets");
    blade_engine::vfs::mount_dir("assets", &ASSETS);
}

fn spawn_props(engine: &mut blade_engine::Engine, planet: &planet::GeneratedPlanet) {
    for deco in &planet.decorations {
        let _ = engine.add_object(
            &blade_engine::config::Object {
                name: format!("deco/{}", deco.name),
                visuals: vec![blade_engine::config::Visual {
                    model: deco.model.clone(),
                    front_face: blade_engine::config::FrontFace::Ccw,
                    pos: [0.0; 3],
                    rot: [0.0; 4],
                    scale: [1.0; 3],
                }],
                colliders: vec![],
                additional_mass: None,
            },
            blade_engine::Transform {
                position: deco.position.into(),
                orientation: deco.orientation.into(),
            },
            blade_engine::DynamicInput::None,
        );
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
        if let Some(px) = pixels.get_mut((y * width + x) as usize) {
            *px = [bright, bright, bright.saturating_add(20), 255];
        }
    }
    // soft vignette
    let cx = width as i32 / 2;
    let cy = height as i32 / 3;
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let dx = (x - cx) as f32 / width as f32;
            let dy = (y - cy) as f32 / height as f32;
            let d = (dx * dx + dy * dy).sqrt();
            if d > 0.55 {
                let t = ((d - 0.55) / 0.45).clamp(0.0, 1.0);
                let i = (y as u32 * width + x as u32) as usize;
                let a = (255.0 * (1.0 - t * 0.55)) as u8;
                pixels[i][3] = a;
            }
        }
    }
    pixels
}

fn hash_u32(mut x: u32) -> u32 {
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    x = x.wrapping_mul(0xC2B2_AE35);
    x ^= x >> 16;
    x
}

fn hash_frame(dt: f32, p: glam::Vec3) -> f32 {
    let bits = dt.to_bits() ^ p.x.to_bits() ^ p.z.to_bits();
    (hash_u32(bits) >> 8) as f32 / 16_777_215.0
}
