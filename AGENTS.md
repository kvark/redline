# Redline contributor guide

Redline is a Rust/Blade vehicle game with one codebase for native Vulkan and
browser WebGL2. `src/game.rs` owns setup and the frame loop, `src/vehicle.rs`
builds the Rapier wheel/suspension graph, `src/planet.rs` generates the planet
and track, and `src/ai.rs` drives opponents. Models use Git LFS.

## Prerequisites

- Rust 1.92 or newer, including `rustfmt`, `clippy`, and the
  `wasm32-unknown-unknown` target.
- `git lfs pull` after cloning.
- Native runtime: a Vulkan driver. On headless Linux, install Xvfb, `libvulkan1`,
  Mesa Vulkan drivers, and the XKB libraries used in `.github/workflows/check.yaml`.
- Web packaging: `wasm-bindgen-cli` at the version in `Cargo.lock`. Do not assume
  the newest CLI is compatible with the locked Rust crate.

## Fast checks

Run these for normal code changes:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --target wasm32-unknown-unknown
```

`.cargo/config.toml` supplies Blade's required `--cfg gles` for the WASM target.
Do not replace its target rustflags: without that cfg, WebGL cannot link all
pipelines even if `cargo check` succeeds.

## Native runtime tests

Use release mode for playtesting because every vehicle is a Rapier joint graph:

```sh
cargo run --release
REDLINE_RT=1 cargo run --release  # optional ray-traced path
```

For CI-style startup coverage, run both profiles. `--smoke N` renders N frames
and exits; it is more useful than a compile-only check because it loads models,
creates shaders, generates the planet, and steps physics.

```sh
xvfb-run -a cargo run -- --smoke 20
xvfb-run -a cargo run --release -- --smoke 20
```

If a headless machine has multiple Vulkan ICDs, select lavapipe explicitly:

```sh
VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.x86_64.json \
  xvfb-run -a cargo run --release -- --smoke 20
```

Vehicle work must also run deterministic scripted traces. They use a fixed
10 ms simulation step and write CSV, which makes regressions visible without
subjective driving:

```sh
xvfb-run -a cargo run --release -- --script accel --seconds 8
xvfb-run -a cargo run --release -- --script steer --seconds 8
xvfb-run -a cargo run --release -- --script offroad --seconds 12
xvfb-run -a cargo run --release -- --script lap --seconds 20 \
  --record /tmp/redline-lap.csv
```

Inspect the logged summary and CSV. In particular, compare `min_upright`,
`max_off`, `yaw_flips`, `recoveries`, and `wheel_error`; the lap trace should
stay on the road without recovery, and `wheel_error` should remain near zero.
An `accel` trace intentionally follows a geodesic rather than steering along
the curving track, so its final off-track distance is not a handling failure.

## Web build and runtime test

A WASM compile check is necessary but does not exercise WebGL, JavaScript glue,
embedded assets, canvas sizing, or browser color space. Package and serve it:

```sh
rustup target add wasm32-unknown-unknown
VERSION=$(awk '/name = "wasm-bindgen"/{getline; if ($1=="version") {gsub(/"/,"",$3); print $3; exit}}' Cargo.lock)
cargo install wasm-bindgen-cli --version "$VERSION" --locked
cargo build --release --target wasm32-unknown-unknown
mkdir -p dist/pkg
wasm-bindgen --target web --no-typescript --out-dir dist/pkg \
  target/wasm32-unknown-unknown/release/redline.wasm
cp web/index.html dist/index.html
python3 -m http.server 8000 --directory dist
```

Open <http://127.0.0.1:8000/> in a WebGL2 browser. Do not open `dist/index.html`
as a `file:` URL. Verify that “Loading Redline…” disappears, the canvas fills
the viewport at the device pixel ratio, there are no console errors, shadows
and terrain midtones are visible, all four wheels remain on each opponent, and
keyboard controls continue after focus/resize changes.

For the repeatable headless Linux check, use the repository script. It builds
and packages WASM, serves it, waits for real asset/shader initialization,
validates that boot completed and the canvas is sensibly sized, then captures a
nontrivial PNG through Chrome DevTools:

```sh
scripts/test-web.sh
# Optional overrides:
REDLINE_BROWSER=google-chrome REDLINE_WEB_SCREENSHOT=/tmp/redline-web.png \
  scripts/test-web.sh
```

The script needs Chromium/Chrome, Node 22+, Python 3, and curl. Open the
screenshot and compare the starting road, car, and HUD with a native capture at
the same size. A successful script is runtime coverage, but visually inspect
lighting and model placement because a renderer can produce a valid but wrong
frame.

## Generated assets and web caveats

`assets/generated/` is ignored. Native generation writes GLBs there; WASM
generation mounts the encoded bytes into Blade's virtual filesystem. Do not
commit generated GLBs. Only `assets/models`, `assets/shaders`, and `vehicle.ron`
are embedded into WASM; this deliberately excludes native generated output.
Rebuild the WASM binary after changing an embedded model, shader, texture, or
RON file.

Keep raster changes compatible with WGSL-to-GLSL/WebGL2. Native-only success
does not prove the web shaders compile. Lighting/exposure intended to match both
platforms belongs in the shared raster shader or raster configuration; Blade's
`Engine::set_average_luminosity` currently affects only the ray tracer.
