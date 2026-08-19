# Redline

Race around a procedurally generated planet. Mars is the first circuit.

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
xvfb-run -a cargo run --release -- --smoke 20
```

Controls: `W/↑` throttle, `S/↓` brake, `A/D` steer, `R` respawn, `Space` jump, `,/.` roll, `Esc` quit.

Kenney models are CC0; licenses are in `assets/licenses/`. Generated planet meshes go to `assets/generated/` and are not committed.
