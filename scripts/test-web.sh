#!/usr/bin/env bash
set -euo pipefail

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_dir"

browser=${REDLINE_BROWSER:-chromium}
serve_port=${REDLINE_WEB_PORT:-8765}
debug_port=$((serve_port + 1))
screenshot=${REDLINE_WEB_SCREENSHOT:-/tmp/redline-web.png}

locked_bindgen=$(awk '/name = "wasm-bindgen"/{getline; if ($1=="version") {gsub(/"/,"",$3); print $3; exit}}' Cargo.lock)
installed_bindgen=$(wasm-bindgen --version | awk '{print $2}')
if [[ "$locked_bindgen" != "$installed_bindgen" ]]; then
  echo "wasm-bindgen-cli $locked_bindgen is required (found $installed_bindgen)" >&2
  echo "cargo install -f wasm-bindgen-cli --version $locked_bindgen --locked" >&2
  exit 1
fi

cargo build --release --target wasm32-unknown-unknown
mkdir -p dist/pkg
wasm-bindgen --target web --no-typescript --out-dir dist/pkg \
  target/wasm32-unknown-unknown/release/redline.wasm
cp web/index.html dist/index.html

profile_dir=$(mktemp -d /tmp/redline-chromium.XXXXXX)
server_pid=
browser_pid=
cleanup() {
  [[ -z "$browser_pid" ]] || kill "$browser_pid" 2>/dev/null || true
  [[ -z "$server_pid" ]] || kill "$server_pid" 2>/dev/null || true
  case "$profile_dir" in
    /tmp/redline-chromium.*) rm -rf -- "$profile_dir" ;;
  esac
}
trap cleanup EXIT

python3 -m http.server "$serve_port" --directory dist >"$profile_dir/server.log" 2>&1 &
server_pid=$!
"$browser" \
  --headless \
  --no-sandbox \
  --disable-background-networking \
  --disable-dev-shm-usage \
  --enable-webgl \
  --enable-unsafe-swiftshader \
  --use-angle=swiftshader \
  --window-size=1280,720 \
  --user-data-dir="$profile_dir/profile" \
  --remote-debugging-port="$debug_port" \
  "http://127.0.0.1:$serve_port/" >"$profile_dir/chromium.log" 2>&1 &
browser_pid=$!

devtools="http://127.0.0.1:$debug_port/json"
for _ in $(seq 1 80); do
  if curl --fail --silent "$devtools" >/dev/null; then
    break
  fi
  sleep 0.25
done
curl --fail --silent "$devtools" >/dev/null

# Asset cooking and shader creation use real worker time. Chromium's
# --virtual-time-budget can expire while the canvas is still on its first black
# frame, so deliberately wait on the wall clock before capturing through CDP.
sleep 12
node scripts/capture-web.mjs "$devtools" "$screenshot"
