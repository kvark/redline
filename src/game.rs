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
use crate::audio;
use crate::boost_pad;
use crate::config;
use crate::control;
use crate::countdown;
use crate::edge_dust;
use crate::gate_pylons;
use crate::lap_callout;
use crate::launch;
use crate::mars_sky;
use crate::menu;
use crate::planet;
use crate::race;
use crate::race_stats;
use crate::racecraft;
use crate::results;
use crate::speed_fx;
use crate::split;
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
    veh_config: config::Vehicle,
    ai_drivers: Vec<ai::Driver>,
    cam_config: config::Camera,
    planet: planet::GeneratedPlanet,
    planet_cfg: config::Planet,
    race: race::Race,
    /// Pack stats (passes, pad hits, contacts, recoveries) for the end-of-race log.
    race_stats: race_stats::RaceStats,
    /// Mean spacing (m) between track samples on the active circuit.
    track_spacing: f32,
    /// Ribbon boost pads for the active circuit (one strip per timed sector).
    boost_pads: Vec<boost_pad::Pad>,
    /// Eased chase-camera FOV kick (radians) from speed + boost.
    fov_kick: f32,
    spawn: Isometry,
    world_handles: Vec<blade_engine::ObjectHandle>,
    in_menu: bool,
    menu_vehicle: menu::VehicleId,
    menu_map: menu::MapId,
    active_map: menu::MapId,
    controller: control::PlayerController,
    throttle_forward: bool,
    throttle_reverse: bool,
    steer_left: bool,
    steer_right: bool,
    dust: usize,
    /// Fixed-pool rust plumes from ribbon-edge running and scrubs.
    edge_dust: usize,
    dust_ledger: edge_dust::Ledger,
    /// Fixed physics steps taken; gates edge-dust emission deterministically.
    step_index: u32,
    decoration_lights: Vec<blade_engine::LightHandle>,
    smoke_frames_left: Option<u32>,
    /// Frame counter for the camera / vehicle state-trace journal.
    frame_index: u32,
    script: Option<trace::Script>,
    recorder: Option<trace::Recorder>,
    record_seconds: Option<f32>,
    sim_time: f32,
    physics_accum: f32,
    recovered_this_step: u8,
    /// Championship 3-2-1-GO; None means racing (or scripts/smoke).
    start_countdown: Option<countdown::StartCountdown>,
    /// Brief lap / FINAL LAP / FINISH / best-sector banner (visual only; never blocks drive).
    lap_callout: Option<lap_callout::LapCallout>,
    /// Post-FINISH placement board; skipped for scripts/smoke.
    race_results: Option<results::RaceResults>,
    /// Board captured at the line, shown once the FINISH banner clears.
    pending_results: Option<results::RaceResults>,
    /// Grid launch ratings and pushes; None when the race had no lights.
    launch: Option<launch::Start>,
    /// Live time gaps for the leader split and the results classification.
    gap_timer: split::GapTimer,
    /// Races started this session; varies AI launches between rematches.
    race_seed: u32,
    /// `--freeze-at`: hold the sim at this time so captures share one pose.
    freeze_at: Option<f32>,
    /// CPU frame times (ms) for the end-of-script perf line.
    frame_ms: Vec<(f32, f32)>,
    /// Engine / whoosh / start-light sounds; silent without a device.
    audio: audio::Output,
    /// Edge detector feeding `audio`; reads race state, never writes it.
    audio_cues: audio::CueTracker,
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
                    .with_title("Redline")
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
        let cli = parse_cli();
        let in_menu = cli.script.is_none() && cli.smoke_frames.is_none();

        let ray_trace = env::var_os("REDLINE_RT").is_some();
        let mut engine = blade_engine::Engine::new(
            blade_engine::Presentation::Window(&window),
            &blade_engine::config::Engine {
                // WGSL sources ship inside blade-render (blade#391); no vendored assets/shaders.
                shader_path: blade_render::shader_dir().to_string_lossy().into_owned(),
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
        engine.set_average_luminosity(0.45);
        engine.set_raster_config(blade_render::RasterConfig {
            clear_color: blade_graphics::TextureColor::OpaqueBlack,
            light_dir: planet::SUN_DIRECTION.into(),
            // Lambert is /π, so these are illuminance. The previous 4.2/0.07
            // pair left equatorial terrain at ~0.15 linear after Reinhard.
            light_color: mint::Vector3 {
                x: 12.0,
                y: 9.2,
                z: 6.5,
            },
            ambient_color: mint::Vector3 {
                x: 0.46,
                y: 0.32,
                z: 0.24,
            },
            space_sky: true,
            // Blade #381 attaches `raster_shadow_fs` on wasm32/GLES so WebGL
            // can link the depth pass. Same map as native.
            directional_shadows: if env::var_os("REDLINE_NO_SHADOWS").is_some() {
                None
            } else {
                Some(blade_render::DirectionalShadowConfig {
                    resolution: 512,
                    distance: 52.0,
                    depth: 220.0,
                    strength: 0.72,
                    normal_bias: 0.07,
                    depth_bias: 0.08,
                })
            },
        });
        // Baked dusty Mars dome; WebGL is happier with a smaller map at boot.
        let (sky_w, sky_h) = if cfg!(target_arch = "wasm32") {
            (512, 256)
        } else {
            (1024, 512)
        };
        engine.create_environment_map("mars-sky", sky_w, sky_h, &mars_sky::sky(sky_w, sky_h));

        let active_map = menu::MapId::MarsClassic;
        let (planet, planet_cfg, world_handles, spawn, race, boost_pads) =
            populate_world(&mut engine, &assets, active_map);

        let veh_config: config::Vehicle =
            ron::de::from_bytes(&read_asset_bytes(&assets.join("vehicle.ron")))
                .expect("unable to parse vehicle config");
        let menu_vehicle = menu::VehicleId::default();
        let vehicle = vehicle::spawn(&mut engine, &veh_config, spawn.clone(), menu_vehicle.kit());
        let clear_ai = cli.script.is_some_and(|script| script.clears_ai());
        let ai_drivers = if clear_ai {
            Vec::new()
        } else {
            spawn_ai_drivers(&mut engine, &veh_config, &planet.track)
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

        let edge_dust = engine.create_particle_system("edge-dust", &edge_dust::effect());

        log::info!(
            "drivers ready: player + {} AI (script={:?}, menu={})",
            ai_drivers.len(),
            cli.script.map(|s| s.as_str()),
            in_menu
        );

        let spacing = track_spacing(&planet.track);
        let ai_count = ai_drivers.len();
        let egui_context = egui::Context::default();
        let egui_viewport_id = egui_context.viewport_id();
        let egui_state =
            egui_winit::State::new(egui_context, egui_viewport_id, &window, None, None, None);

        Self {
            engine,
            last_physics_update: time::Instant::now(),
            last_camera_update: time::Instant::now(),
            last_camera_orient: spawn.orientation,
            is_paused: in_menu,
            window,
            egui_state,
            egui_viewport_id,
            vehicle,
            veh_config,
            ai_drivers,
            cam_config: config::Camera::default(),
            planet,
            planet_cfg,
            race,
            race_stats: race_stats::RaceStats::default(),
            track_spacing: spacing,
            boost_pads,
            fov_kick: 0.0,
            spawn,
            world_handles,
            in_menu,
            menu_vehicle,
            menu_map: active_map,
            active_map,
            controller: control::PlayerController::default(),
            throttle_forward: false,
            throttle_reverse: false,
            steer_left: false,
            steer_right: false,
            dust,
            edge_dust,
            dust_ledger: edge_dust::Ledger::default(),
            step_index: 0,
            decoration_lights: Vec::new(),
            smoke_frames_left: cli.smoke_frames,
            frame_index: 0,
            script: cli.script,
            recorder: cli
                .record_path
                .map(|path| trace::Recorder::new(path, cli.script.unwrap_or(trace::Script::Lap))),
            record_seconds: cli.seconds,
            sim_time: 0.0,
            physics_accum: 0.0,
            recovered_this_step: 0,
            start_countdown: wants_lights(cli.script, cli.smoke_frames)
                .then(countdown::StartCountdown::new),
            lap_callout: None,
            race_results: None,
            pending_results: None,
            launch: wants_lights(cli.script, cli.smoke_frames)
                .then(|| launch::Start::new(ai_count, 0)),
            gap_timer: split::GapTimer::default(),
            race_seed: 0,
            freeze_at: cli.freeze_at,
            frame_ms: Vec::new(),
            audio: audio::Output::start(),
            audio_cues: audio::CueTracker::new(),
        }
    }

    fn build_world(&mut self, map: menu::MapId) {
        for handle in self.world_handles.drain(..) {
            self.engine.remove_object(handle);
        }
        let assets = assets_dir();
        let (planet, planet_cfg, world_handles, spawn, race, boost_pads) =
            populate_world(&mut self.engine, &assets, map);
        self.boost_pads = boost_pads;
        self.planet = planet;
        self.planet_cfg = planet_cfg;
        self.world_handles = world_handles;
        self.spawn = spawn;
        self.race = race;
        self.track_spacing = track_spacing(&self.planet.track);
        self.active_map = map;
        self.last_camera_orient = self.spawn.orientation;
    }

    fn open_menu(&mut self) {
        self.in_menu = true;
        self.is_paused = true;
        self.start_countdown = None;
        self.lap_callout = None;
        self.race_results = None;
        self.throttle_forward = false;
        self.throttle_reverse = false;
        self.steer_left = false;
        self.steer_right = false;
    }

    fn start_race(&mut self) {
        self.vehicle.despawn(&mut self.engine);
        for driver in self.ai_drivers.drain(..) {
            driver.vehicle.despawn(&mut self.engine);
        }
        if self.menu_map != self.active_map {
            self.build_world(self.menu_map);
        }
        self.spawn = vehicle::Vehicle::spawn_pose(&self.planet.track, vehicle::SPAWN_HOVER);
        self.vehicle = vehicle::spawn(
            &mut self.engine,
            &self.veh_config,
            self.spawn.clone(),
            self.menu_vehicle.kit(),
        );
        let clear_ai = self.script.is_some_and(|script| script.clears_ai());
        if !clear_ai {
            self.ai_drivers =
                spawn_ai_drivers(&mut self.engine, &self.veh_config, &self.planet.track);
        }
        self.race.reset();
        self.race_stats.reset();
        self.fov_kick = 0.0;
        self.last_camera_orient = self.spawn.orientation;
        self.in_menu = false;
        self.is_paused = false;
        self.throttle_forward = false;
        self.throttle_reverse = false;
        self.steer_left = false;
        self.steer_right = false;
        self.controller = control::PlayerController::default();
        self.lap_callout = None;
        self.race_results = None;
        self.pending_results = None;
        self.gap_timer.reset();
        self.race_seed = self.race_seed.wrapping_add(1);
        let lights = wants_lights(self.script, self.smoke_frames_left);
        self.start_countdown = lights.then(countdown::StartCountdown::new);
        self.launch = lights.then(|| launch::Start::new(self.ai_drivers.len(), self.race_seed));
        self.audio_cues.reset();
        self.window.set_title(&format!(
            "Redline — {} / {}",
            self.menu_vehicle.label(),
            self.active_map.label()
        ));
    }

    fn update_time(&mut self) {
        let wall = self.last_physics_update.elapsed().as_secs_f32();
        self.last_physics_update = time::Instant::now();
        if self.is_paused || self.frozen() {
            return;
        }
        // Fixed step keeps Rapier/controls identical on lavapipe, a real GPU,
        // and WebGL2 — frame time only changes how many steps we catch up.
        const PHYSICS_DT: f32 = 0.01;
        self.physics_accum = (self.physics_accum + wall).min(0.35);
        // Scripts may run dozens of steps per redraw so a 30s lap finishes in
        // seconds of wall time under software raster. Playable caps catch-up
        // to avoid a death spiral when a frame is slow.
        let max_steps = if self.script.is_some() { 80 } else { 5 };
        let mut steps = 0u32;
        let physics_started = time::Instant::now();
        while self.physics_accum >= PHYSICS_DT && steps < max_steps {
            if self.script_finished() || self.frozen() {
                break;
            }
            self.physics_accum -= PHYSICS_DT;
            self.step_physics(PHYSICS_DT);
            steps += 1;
        }
        if self.script.is_none() && steps >= max_steps {
            self.physics_accum = 0.0;
        }
        let physics_ms = physics_started.elapsed().as_secs_f32() * 1e3;
        if cfg!(debug_assertions) && self.frame_index.is_multiple_of(20) {
            log::debug!(
                "physics {:.1}ms ({} x {:.0}ms steps, accum leftover {:.1}ms)",
                physics_ms,
                steps,
                PHYSICS_DT * 1e3,
                self.physics_accum * 1e3
            );
        }
    }

    fn step_physics(&mut self, dt: f32) {
        self.sim_time += dt;
        let countdown_done = self
            .start_countdown
            .as_mut()
            .is_some_and(|lights| lights.tick(dt));
        if countdown_done {
            self.start_countdown = None;
        }
        if let (Some(lights), Some(start)) = (self.start_countdown.as_ref(), self.launch.as_mut()) {
            let t = lights.since_go();
            // The scripted driver squeezes the throttle just before GO.
            let held = match self.script {
                Some(_) => t >= -launch::SCRIPT_LEAD,
                None => self.throttle_forward,
            };
            start.observe(t, held);
        }
        let drive_locked = self
            .start_countdown
            .as_ref()
            .is_some_and(|c| c.controls_locked());
        if drive_locked {
            // Hold on the grid: gravity/stability still settle the craft, but
            // player + AI drive stay frozen until GO.
            // Grid is on-ribbon; pass 0 so grip bonus applies without arming boost.
            self.vehicle.drive(&mut self.engine, 0.0, 0.0, 0.0, &[], dt);
            self.vehicle
                .apply_gravity(&mut self.engine, self.planet_cfg.gravity, dt);
            self.vehicle.apply_stability(&mut self.engine, dt);
            for driver in self.ai_drivers.iter_mut() {
                driver
                    .vehicle
                    .drive(&mut self.engine, 0.0, 0.0, 0.0, &[], dt);
                driver
                    .vehicle
                    .apply_gravity(&mut self.engine, self.planet_cfg.gravity, dt);
                driver.vehicle.apply_stability(&mut self.engine, dt);
            }
        } else {
            self.update_vehicle_controls(dt);
            self.vehicle
                .apply_gravity(&mut self.engine, self.planet_cfg.gravity, dt);
            self.vehicle.apply_stability(&mut self.engine, dt);
            let track = &self.planet.track;
            let field: Vec<ai::Racer> = std::iter::once(&self.vehicle)
                .chain(self.ai_drivers.iter().map(|d| &d.vehicle))
                .map(|vehicle| ai::Racer::observe(vehicle, &self.engine, track))
                .collect();
            let course = ai::Course {
                track,
                track_width: self.planet.track_width,
                spacing: self.track_spacing,
                pads: &self.boost_pads,
                gravity: self.planet_cfg.gravity,
                pack_mean: (self.ai_drivers.len() > 1).then(|| {
                    self.ai_drivers.iter().map(|d| d.distance()).sum::<f32>()
                        / self.ai_drivers.len() as f32
                }),
            };
            for driver in self.ai_drivers.iter_mut() {
                let recovered = driver.update(&mut self.engine, &course, &field, dt);
                if recovered {
                    self.race_stats.record_recovery(true);
                }
            }
            self.apply_launch(dt);
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
        if !drive_locked {
            if let Some(event) = self.race.update(pose.position, dt) {
                self.on_race_event(event);
            }
            for driver in self.ai_drivers.iter_mut() {
                let ai_pose = driver.vehicle.pose(&self.engine);
                let _ = driver.race.update(ai_pose.position, dt);
            }
            if self.recovered_this_step != 0 {
                self.race_stats.record_recovery(false);
            }
            self.observe_race_stats(dt);
        }
        if self
            .lap_callout
            .as_mut()
            .is_some_and(|banner| banner.tick(dt))
        {
            let was_finish = matches!(
                self.lap_callout.as_ref().map(|b| b.kind()),
                Some(lap_callout::CalloutKind::Finish)
            );
            self.lap_callout = None;
            // Scripts/smoke must not block on the results board.
            if was_finish && self.script.is_none() && self.smoke_frames_left.is_none() {
                self.race_results = self
                    .pending_results
                    .take()
                    .or_else(|| Some(self.capture_results()));
            }
        }
        if self
            .race_results
            .as_mut()
            .is_some_and(|board| board.tick(dt))
        {
            self.race_results = None;
            self.open_menu();
        }
        self.emit_dust(&pose, dt);
        self.emit_edge_dust();
        self.listen(drive_locked);
        if !drive_locked {
            self.record_sample(&pose, linear, angular, forward_speed, lateral_speed);
        }
    }

    /// Turn this step's race state into audio cues (read-only for the sim).
    fn listen(&mut self, drive_locked: bool) {
        let heard = audio::Heard {
            countdown: self.start_countdown.as_ref().and_then(|c| c.label()),
            pad_hits: self.vehicle.pad_hits(),
            launch: self.launch.as_ref().and_then(|s| s.banner()),
            rejoin_boost: if drive_locked {
                0.0
            } else {
                self.vehicle.ribbon_boost_intensity()
            },
        };
        let (audio, sim_time) = (&self.audio, self.sim_time);
        self.audio_cues.observe(&heard, |cue| {
            log::debug!("audio cue {cue:?} at sim={sim_time:.2}s");
            audio.send(audio::Command::Play(cue));
        });
    }

    /// Engine note for this frame; muted in the menu and while paused.
    fn update_engine_audio(&self) {
        let (hz, gain) = if self.in_menu || self.is_paused {
            (audio::ENGINE_IDLE_HZ, 0.0)
        } else {
            let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
            let speed = glam::Vec3::from(lin).length();
            let boost = self.vehicle.boost_intensity().max(self.launch_thrust());
            (
                audio::engine_pitch(speed, boost),
                audio::engine_gain(speed, self.controller.throttle()),
            )
        };
        self.audio.send(audio::Command::Engine { hz, gain });
    }

    /// Launch thrust (or jump-start wheelspin drag) along each craft's heading.
    fn apply_launch(&mut self, dt: f32) {
        let Some(start) = self.launch.as_mut() else {
            return;
        };
        let (lin, _) = self.engine.get_velocity(self.vehicle.body_handle);
        let accels = start.tick(dt, glam::Vec3::from(lin).length());
        for (id, accel) in accels.into_iter().enumerate() {
            if accel == 0.0 || id > self.ai_drivers.len() {
                continue;
            }
            let vehicle = if id == 0 {
                &self.vehicle
            } else {
                &self.ai_drivers[id - 1].vehicle
            };
            let pose = vehicle.pose(&self.engine);
            let up = pose.position.normalize_or_zero();
            let forward = (pose.orientation * glam::Vec3::Z)
                .reject_from(up)
                .normalize_or_zero();
            let (lin, _) = self.engine.get_velocity(vehicle.body_handle);
            // Wheelspin only robs speed the craft has; it never pushes backwards.
            if accel < 0.0 && glam::Vec3::from(lin).dot(forward) < 0.5 {
                continue;
            }
            let (handle, mass) = (vehicle.body_handle, vehicle.body_mass);
            self.engine
                .apply_linear_impulse(handle, (forward * mass * accel * dt.min(0.05)).into());
        }
    }

    /// Launch thrust left on the player's craft, 0..1 (camera / HUD FX).
    fn launch_thrust(&self) -> f32 {
        self.launch.as_ref().map_or(0.0, |s| s.player_thrust())
    }

    /// One-line launch summary for the end-of-race log.
    pub(crate) fn launch_summary(&self) -> String {
        let Some(start) = self.launch.as_ref() else {
            return "Launch: no start lights".to_string();
        };
        let secs = |v: Option<f32>| v.map_or("-".to_string(), |t| format!("{t:+.2}s"));
        let name = |r: &Option<launch::Rating>| r.map_or("-", |r| r.label());
        let ai: Vec<_> = start.ratings[1..].iter().map(name).collect();
        let margin = self.pending_results.as_ref().and_then(|b| b.margin());
        format!(
            "Launch: player={} hold={} est_gain={} ai=[{}] split_at_finish={}",
            name(&start.ratings[0]),
            secs(start.player_start),
            secs(start.player_gain),
            ai.join(", "),
            margin.map_or("-".to_string(), |m| format!(
                "{} {}",
                m.text(),
                m.reference()
            )),
        )
    }

    fn on_race_event(&mut self, event: race::RaceEvent) {
        // Visual only — never gates controls or scripts.
        match event {
            race::RaceEvent::Finished { .. } => {
                self.lap_callout = Some(lap_callout::LapCallout::finish());
                // Classify at the line, before the field drives on.
                self.pending_results = Some(self.capture_results());
            }
            race::RaceEvent::LapComplete {
                lap_time,
                entering_final,
            } => {
                self.lap_callout = Some(if entering_final {
                    lap_callout::LapCallout::final_lap(lap_time)
                } else {
                    lap_callout::LapCallout::lap_complete(lap_time)
                });
            }
            race::RaceEvent::SectorComplete {
                sector_time,
                is_best,
            } => {
                // Light gate flash only on a new best; never stomps lap banners.
                if is_best && self.lap_callout.is_none() {
                    self.lap_callout = Some(lap_callout::LapCallout::sector_best(sector_time));
                }
            }
        }
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

    /// Rust plumes for every craft running the ribbon edge or scrubbing.
    fn emit_edge_dust(&mut self) {
        self.step_index = self.step_index.wrapping_add(1);
        let player_up = self.vehicle.pose(&self.engine).position.normalize_or_zero();
        let craft =
            std::iter::once(&self.vehicle).chain(self.ai_drivers.iter().map(|d| &d.vehicle));
        let mut bursts = Vec::new();
        for (id, vehicle) in craft.enumerate() {
            if !edge_dust::gate(self.step_index, id) {
                continue;
            }
            let (pose, linear, _, forward_speed, lateral_speed) = vehicle.motion(&self.engine);
            let query = planet::query_track(pose.position, &self.planet.track);
            let off = planet::off_track_distance(&query, self.planet.track_width);
            let want = edge_dust::plume(off, lateral_speed, linear.length());
            if want == 0 {
                continue;
            }
            let up = pose.position.normalize_or_zero();
            let back = pose.orientation * glam::Vec3::Z * forward_speed.signum();
            // Behind the craft, on the outside of the ribbon.
            let outward = query.side * query.lateral.signum() * 0.6;
            bursts.push((want, pose.position - up * 0.35 - back * 1.1 + outward));
        }
        for (want, at) in bursts {
            let granted = self.dust_ledger.reserve(self.sim_time, want);
            if granted > 0 {
                self.engine
                    .particle_burst(self.edge_dust, granted, at.into());
            }
        }
        if let Some(system) = self.engine.particle_system_mut(self.edge_dust) {
            system.axis = player_up.into();
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
            off_track,
            &self.boost_pads,
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
        let ribbon_boost = self.vehicle.ribbon_boost_intensity();
        let pad_boost = self.vehicle.pad_boost_intensity();
        let Some(recorder) = self.recorder.as_mut() else {
            return;
        };
        let up = pose.position.normalize_or_zero();
        let query = planet::query_track(pose.position, &self.planet.track);
        let (_idx, progress) = planet::track_progress(pose.position, &self.planet.track);
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
            track_progress: progress,
            checkpoint: self.race.next_checkpoint as u32,
            lap: self.race.lap,
            recovered: self.recovered_this_step,
            ribbon_boost,
            pad_boost,
        });
    }

    fn update_local_lights(&mut self, eye: glam::Vec3) {
        if self.decoration_lights.is_empty() {
            let off = blade_render::LocalLight {
                intensity: 0.0,
                range: 1.0,
                ..Default::default()
            };
            for _ in 0..blade_render::MAX_LOCAL_LIGHTS {
                self.decoration_lights.push(self.engine.add_light(off));
            }
        }
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
                    blade_render::LocalLight {
                        position: deco.position.into(),
                        color: mint::Vector3 {
                            x: color[0],
                            y: color[1],
                            z: color[2],
                        },
                        intensity: intensity.max(0.05),
                        range: 11.0 + deco.scale * 0.65,
                        angular: blade_render::LightAngularProfile::Omnidirectional,
                    },
                ))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let mut lights: Vec<_> = ranked
            .into_iter()
            .take(blade_render::MAX_LOCAL_LIGHTS)
            .map(|(_, light)| light)
            .collect();
        while lights.len() < self.decoration_lights.len() {
            lights.push(blade_render::LocalLight {
                intensity: 0.0,
                range: 1.0,
                ..Default::default()
            });
        }
        for (handle, light) in self.decoration_lights.iter().copied().zip(lights) {
            self.engine.set_light(handle, light);
        }
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
        self.race_stats.observe_contacts(&pairs);
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

    fn observe_race_stats(&mut self, dt: f32) {
        let track = &self.planet.track;
        let width = self.planet.track_width;
        let entry = |vehicle: &vehicle::Vehicle, is_ai: bool| {
            let query = planet::query_track(vehicle.pose(&self.engine).position, track);
            race_stats::Entry {
                sample: query.index,
                is_ai,
                off_track: planet::off_track_distance(&query, width),
            }
        };
        let entries: Vec<_> = std::iter::once(entry(&self.vehicle, false))
            .chain(self.ai_drivers.iter().map(|d| entry(&d.vehicle, true)))
            .collect();
        self.race_stats
            .observe(&entries, track.len(), self.track_spacing);
        self.gap_timer.observe(self.race_stats.distances(), dt);
    }

    /// One-line pack summary for the end-of-race log.
    pub(crate) fn race_stats_summary(&self) -> String {
        let stats = &self.race_stats;
        let ai_pad_hits: u32 = self.ai_drivers.iter().map(|d| d.vehicle.pad_hits()).sum();
        let ai_pads_passed: u32 = self.ai_drivers.iter().map(|d| d.pads_passed()).sum();
        let ai_distance: f32 = self.ai_drivers.iter().map(|d| d.distance()).sum();
        let order = stats
            .order()
            .iter()
            .map(|&id| {
                if id == 0 {
                    "P".to_string()
                } else {
                    format!("AI{id}")
                }
            })
            .collect::<Vec<_>>()
            .join(">");
        let tally = self
            .ai_drivers
            .iter()
            .fold(racecraft::Tally::default(), |mut sum, d| {
                let t = d.tally();
                sum.passes_committed += t.passes_committed;
                sum.passes_completed += t.passes_completed;
                sum.passes_aborted += t.passes_aborted;
                sum.defends += t.defends;
                sum
            });
        format!(
            "Race stats: ai_overtakes={} position_changes={} ai_pad_hits={} ai_pads_passed={} ai_distance={:.0}m contacts={} \
             ai_off_ribbon={} ai_recoveries={} player_off_ribbon={} player_recoveries={} \
             spread={:.1}m order={} ai_passes(committed/completed/aborted)={}/{}/{} ai_defends={}",
            stats.ai_overtakes,
            stats.position_changes,
            ai_pad_hits,
            ai_pads_passed,
            ai_distance,
            stats.contacts,
            stats.ai_off_ribbon,
            stats.ai_recoveries,
            stats.player_off_ribbon,
            stats.player_recoveries,
            stats.spread(),
            order,
            tally.passes_committed,
            tally.passes_completed,
            tally.passes_aborted,
            tally.defends,
        )
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

    fn field_size(&self) -> usize {
        1 + self.ai_drivers.len()
    }

    fn live_place(&self) -> usize {
        let player_pose = self.vehicle.pose(&self.engine);
        let (_, player_frac) = planet::track_progress(player_pose.position, &self.planet.track);
        let player_score = self.race.progress_score(player_frac);
        let opponent_scores = self.ai_drivers.iter().map(|driver| {
            let pose = driver.vehicle.pose(&self.engine);
            let (_, frac) = planet::track_progress(pose.position, &self.planet.track);
            driver.race.progress_score(frac)
        });
        race::player_place(player_score, opponent_scores)
    }

    fn capture_results(&self) -> results::RaceResults {
        results::RaceResults::new(
            self.live_place(),
            self.field_size(),
            self.race.time,
            self.race.best_lap,
            self.race.last_lap_time,
        )
        .with_field(
            self.launch.as_ref().and_then(|s| s.ratings[0]),
            self.gap_timer
                .standings()
                .into_iter()
                .map(|(racer, gap)| results::Standing { racer, gap })
                .collect(),
        )
    }

    fn frozen(&self) -> bool {
        self.freeze_at.is_some_and(|t| self.sim_time >= t)
    }

    /// One-line CPU frame-time summary for the end-of-script log.
    pub(crate) fn frame_summary(&self) -> String {
        let stats = |pick: fn(&(f32, f32)) -> f32| {
            let mut sorted: Vec<f32> = self.frame_ms.iter().map(pick).collect();
            sorted.sort_by(f32::total_cmp);
            let n = sorted.len().max(1);
            let mean = sorted.iter().sum::<f32>() / n as f32;
            (mean, sorted.get(n * 95 / 100).copied().unwrap_or(0.0))
        };
        let (mean, p95) = stats(|f| f.0);
        let (render_mean, render_p95) = stats(|f| f.1);
        format!(
            "Frame stats: frames={} cpu_mean={mean:.2}ms cpu_p95={p95:.2}ms render_mean={render_mean:.2}ms render_p95={render_p95:.2}ms",
            self.frame_ms.len()
        )
    }

    pub(crate) fn script_finished(&self) -> bool {
        let timed_out = self
            .record_seconds
            .is_some_and(|limit| self.sim_time >= limit);
        let lap_done = matches!(self.script, Some(trace::Script::Lap))
            && (self.race.lap > 1 || self.race.finished || self.race.last_lap_time.is_some());
        let race_done = matches!(self.script, Some(trace::Script::Race)) && self.race.finished;
        timed_out || lap_done || race_done
    }
}

/// Start lights run for play and for the scripted race (which practises its
/// launch); smoke and single-lap scripts skip them to stay fast.
fn wants_lights(script: Option<trace::Script>, smoke: Option<u32>) -> bool {
    smoke.is_none() && script.is_none_or(|s| s == trace::Script::Race)
}

struct Cli {
    freeze_at: Option<f32>,
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
                kit: menu::KIT_HATCH,
            },
            OpponentSpec {
                index: 13,
                lane: 2.0,
                speed: 14.5,
                kit: menu::KIT_TAXI,
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
            kit: menu::KIT_HATCH,
        }];
        &SPECS
    }
    #[cfg(all(not(target_arch = "wasm32"), not(debug_assertions)))]
    {
        // Quicker cars start further back so the pack closes up and has to
        // race through traffic instead of spreading out from the grid.
        static SPECS: [OpponentSpec; 3] = [
            OpponentSpec {
                index: 8,
                lane: -2.1,
                speed: 16.0,
                kit: menu::KIT_HATCH,
            },
            OpponentSpec {
                index: 12,
                lane: 2.0,
                speed: 15.6,
                kit: menu::KIT_SEDAN,
            },
            OpponentSpec {
                index: 16,
                lane: -0.6,
                speed: 15.2,
                kit: menu::KIT_TAXI,
            },
        ];
        &SPECS
    }
}

#[cfg(target_arch = "wasm32")]
fn parse_cli() -> Cli {
    // `?script=lap&seconds=40` or `?script=race&seconds=120` lets the WASM build
    // exercise the same fixed-step drives as native (race keeps AI on).
    // Recording still needs a filesystem, so it stays off.
    let mut cli = Cli {
        freeze_at: None,
        smoke_frames: None,
        record_path: None,
        script: None,
        seconds: None,
    };
    let search = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default();
    let query = search.trim_start_matches('?');
    for pair in query.split('&').filter(|part| !part.is_empty()) {
        let mut parts = pair.splitn(2, '=');
        let key = parts.next().unwrap_or("");
        let value = parts.next().unwrap_or("");
        match key {
            "script" => {
                cli.script = match value {
                    "accel" => Some(trace::Script::Accel),
                    "steer" => Some(trace::Script::Steer),
                    "offroad" => Some(trace::Script::Offroad),
                    "lap" => Some(trace::Script::Lap),
                    "race" => Some(trace::Script::Race),
                    _ => None,
                };
            }
            "seconds" => {
                cli.seconds = value.parse().ok();
            }
            "smoke" => {
                cli.smoke_frames = value.parse().ok();
            }
            _ => {}
        }
    }
    if cli.seconds.is_none() {
        cli.seconds = match cli.script {
            Some(trace::Script::Race) => Some(120.0),
            Some(_) => Some(30.0),
            None => None,
        };
    }
    cli
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_cli() -> Cli {
    let mut cli = Cli {
        freeze_at: None,
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
            "--freeze-at" => {
                cli.freeze_at = args.next().and_then(|value| value.parse().ok());
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
        cli.seconds = Some(match cli.script {
            Some(trace::Script::Race) => 120.0,
            _ => 10.0,
        });
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

/// Mean distance (m) between consecutive samples of the closed ribbon.
fn track_spacing(track: &[planet::TrackSample]) -> f32 {
    if track.len() < 2 {
        return 1.0;
    }
    let total: f32 = (0..track.len())
        .map(|i| (track[(i + 1) % track.len()].position - track[i].position).length())
        .sum();
    total / track.len() as f32
}

fn spawn_ai_drivers(
    engine: &mut blade_engine::Engine,
    veh_config: &config::Vehicle,
    track: &[planet::TrackSample],
) -> Vec<ai::Driver> {
    opponent_specs()
        .iter()
        .copied()
        .enumerate()
        .map(|(slot, spec)| {
            ai::Driver::spawn(
                engine, veh_config, track, spec.index, spec.lane, spec.speed, spec.kit,
            )
            .with_id(slot + 1)
        })
        .collect()
}

fn populate_world(
    engine: &mut blade_engine::Engine,
    assets: &std::path::Path,
    map: menu::MapId,
) -> (
    planet::GeneratedPlanet,
    config::Planet,
    Vec<blade_engine::ObjectHandle>,
    Isometry,
    race::Race,
    Vec<boost_pad::Pad>,
) {
    let planet_cfg = map.planet();
    let generated = assets.join("generated");
    let gen_started = time::Instant::now();
    let planet = planet::generate(planet_cfg, &generated);
    log::info!(
        "generated {} in {:.1}ms ({} decorations, {} track samples, {:.0}m lap, {:.1}m ribbon)",
        map.label(),
        gen_started.elapsed().as_secs_f32() * 1e3,
        planet.decorations.len(),
        planet.track.len(),
        track_spacing(&planet.track) * planet.track.len() as f32,
        planet.track_width,
    );

    let mut world_handles = Vec::new();
    let planet_rel = relative_model(assets, &planet.planet_model);
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
    let objects_started = time::Instant::now();
    world_handles.push(engine.add_object(
        &planet_object,
        blade_engine::Transform::default(),
        blade_engine::DynamicInput::Empty,
    ));
    for (index, deco) in planet.decorations.iter().enumerate() {
        let model = relative_model(assets, &deco.model);
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
        world_handles.push(engine.add_object(
            &object,
            blade_engine::Transform {
                position: deco.position.into(),
                orientation: deco.orientation.into(),
            },
            blade_engine::DynamicInput::Empty,
        ));
    }

    world_handles.extend(spawn_props(engine, &planet));
    let race_cfg = config::Race::default();
    let boost_pads = boost_pad::place(
        &planet.track,
        race_cfg.checkpoint_count as usize,
        boost_pad::PADS_PER_SECTOR,
        planet.track_width,
    );
    if let Some(model) = boost_pad::write_model(&generated, &boost_pads, planet_cfg, &planet.track)
    {
        let pads_object = blade_engine::config::Object {
            name: "boost-pads".to_string(),
            visuals: vec![blade_engine::config::Visual {
                model: relative_model(assets, &model),
                ..Default::default()
            }],
            // Visual only: the trigger is a footprint test, so no collider bumps.
            colliders: Vec::new(),
            additional_mass: None,
        };
        world_handles.push(engine.add_object(
            &pads_object,
            blade_engine::Transform::default(),
            blade_engine::DynamicInput::Empty,
        ));
    }
    if let Some(model) = gate_pylons::write_model(
        &generated,
        &planet.track,
        planet.track_width,
        planet_cfg,
        race_cfg.checkpoint_count,
    ) {
        let pylons = blade_engine::config::Object {
            name: "gate-pylons".to_string(),
            visuals: vec![blade_engine::config::Visual {
                model: relative_model(assets, &model),
                ..Default::default()
            }],
            // Cosmetic only: no colliders, so lap physics is untouched.
            colliders: Vec::new(),
            additional_mass: None,
        };
        world_handles.push(engine.add_object(
            &pylons,
            blade_engine::Transform::default(),
            blade_engine::DynamicInput::Empty,
        ));
    }
    log::info!(
        "laid {} boost pads (sector@sample: {})",
        boost_pads.len(),
        boost_pads
            .iter()
            .map(|pad| format!("{}@{}", pad.sector, pad.sample))
            .collect::<Vec<_>>()
            .join(" ")
    );
    log::info!(
        "spawned colliders in {:.1}ms",
        objects_started.elapsed().as_secs_f32() * 1e3
    );

    let spawn = vehicle::Vehicle::spawn_pose(&planet.track, vehicle::SPAWN_HOVER);
    let race = race::Race::new(&planet.track, race_cfg);
    (planet, planet_cfg, world_handles, spawn, race, boost_pads)
}

fn spawn_props(
    engine: &mut blade_engine::Engine,
    planet: &planet::GeneratedPlanet,
) -> Vec<blade_engine::ObjectHandle> {
    let mut handles = Vec::new();
    let start = &planet.track[0];
    let start_side = start.normal.cross(start.tangent).normalize_or_zero();
    let flag = blade_engine::config::Object {
        name: "start-flag".to_string(),
        visuals: vec![blade_engine::config::Visual {
            model: "models/flag-checkers.glb".to_string(),
            scale: 2.4,
            ..Default::default()
        }],
        colliders: vec![blade_engine::config::Collider {
            density: 1.0,
            friction: 0.4,
            restitution: 0.05,
            shape: blade_engine::config::Shape::Cuboid {
                half: mint::Vector3 {
                    x: 0.12,
                    y: 1.4,
                    z: 0.45,
                },
            },
            pos: [0.0; 3].into(),
            rot: [0.0; 3].into(),
        }],
        additional_mass: None,
    };
    handles.push(
        engine.add_object(
            &flag,
            blade_engine::Transform {
                // Keep the start marker off the racing line; a center-line pole
                // yaws a full-throttle launch into the weeds.
                position: (start.position
                    + start.normal * 0.2
                    + start_side * (planet.track_width * 0.48))
                    .into(),
                orientation: planet::surface_quat(start.normal, start.tangent).into(),
            },
            blade_engine::DynamicInput::Empty,
        ),
    );

    let pylon = blade_engine::config::Object {
        name: "pylon".to_string(),
        visuals: vec![blade_engine::config::Visual {
            model: "models/pylon.glb".to_string(),
            scale: 1.6,
            ..Default::default()
        }],
        colliders: vec![blade_engine::config::Collider {
            density: 1.0,
            friction: 0.4,
            restitution: 0.05,
            shape: blade_engine::config::Shape::Cylinder {
                half_height: 0.55,
                radius: 0.22,
            },
            pos: [0.0; 3].into(),
            rot: [0.0; 3].into(),
        }],
        additional_mass: None,
    };
    let stride = (planet.track.len() / 14).max(1);
    for sample in planet.track.iter().step_by(stride) {
        let side = sample.normal.cross(sample.tangent).normalize_or_zero();
        let offset = planet.track_width * 0.52;
        for sign in [-1.0, 1.0] {
            let pos = sample.position + side * (offset * sign) + sample.normal * 0.1;
            handles.push(engine.add_object(
                &pylon,
                blade_engine::Transform {
                    position: pos.into(),
                    orientation: planet::surface_quat(sample.normal, sample.tangent).into(),
                },
                blade_engine::DynamicInput::Empty,
            ));
        }
    }
    handles
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

fn format_time(seconds: f32) -> String {
    let m = (seconds / 60.0).floor() as u32;
    let s = seconds - m as f32 * 60.0;
    format!("{m}:{s:05.2}")
}

pub(crate) fn hash_u32(x: u32) -> u32 {
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
