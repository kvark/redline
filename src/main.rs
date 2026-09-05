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

#[cfg(all(target_arch = "wasm32", not(gles)))]
compile_error!(
    "wasm32 builds must set --cfg gles (see .cargo/config.toml); otherwise WebGL cannot link the shadow pipeline"
);

mod ai;
mod config;
mod control;
mod game;
mod glb;
mod planet;
mod race;
mod trace;
mod vehicle;

use game::{Game, QuitEvent};

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
    #[cfg(target_arch = "wasm32")]
    {
        console_error_panic_hook::set_once();
        let _ = console_log::init_with_level(log::Level::Info);
        mount_embedded_assets();
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::init();
    }

    let event_loop = winit::event_loop::EventLoop::new().unwrap();
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = App { game: None };
    event_loop.run_app(&mut app).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn mount_embedded_assets() {
    use include_dir::{Dir, include_dir};
    // Generated GLBs are created directly in Blade's VFS by planet::generate.
    // Embedding assets/generated here would duplicate megabytes and make every
    // native run invalidate the next WASM build.
    static MODELS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets/models");
    static SHADERS: Dir = include_dir!("$CARGO_MANIFEST_DIR/assets/shaders");
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets");
    fn walk(dir: &Dir, root: &std::path::Path) {
        for file in dir.files() {
            blade_engine::vfs::mount(root.join(file.path()), file.contents().to_vec());
        }
        for child in dir.dirs() {
            walk(child, root);
        }
    }
    walk(&MODELS, &root.join("models"));
    walk(&SHADERS, &root.join("shaders"));
    blade_engine::vfs::mount(
        root.join("vehicle.ron"),
        include_bytes!("../assets/vehicle.ron").to_vec(),
    );
}
