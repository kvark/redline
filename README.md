# Redline

Race around a procedurally generated planet. Mars is the first circuit.

Play in the browser: <https://kvark.github.io/redline/>

## Run

Models are stored in Git LFS:

```sh
git lfs install
git lfs pull
cargo run --release
```

Optional ray-traced lighting (needs RT hardware):

```sh
REDLINE_RT=1 cargo run --release
```

Headless smoke test (used by Linux CI):

```sh
# debug — catches assertion failures that --release strips
xvfb-run -a cargo run -- --smoke 20
xvfb-run -a cargo run --release -- --smoke 20
```

## Web / WASM

Requires a WebGL2 browser. Assets are embedded at compile time.

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli
cargo build --release --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir dist/pkg \
    target/wasm32-unknown-unknown/release/redline.wasm
cp web/index.html dist/index.html
python3 -m http.server --directory dist
```

Pushes to `main` build the WASM target and deploy it to GitHub Pages.

Controls: `W/↑` throttle, `S/↓` brake, `A/D` steer, `R` respawn, `Space` jump, `,/.` roll, `Esc` quit.

Kenney models are CC0; licenses are in `assets/licenses/`. Generated planet meshes go to `assets/generated/` and are not committed.
