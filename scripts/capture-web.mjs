import fs from "node:fs";

const [endpoint, output] = process.argv.slice(2);
if (!endpoint || !output) {
  throw new Error("usage: node scripts/capture-web.mjs <devtools-json-url> <output.png>");
}

const targets = await (await fetch(endpoint)).json();
const page = targets.find(
  (target) => target.type === "page" && target.url.startsWith("http"),
);
if (!page) {
  throw new Error("Chromium has no HTTP page target");
}

const socket = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => {
  socket.addEventListener("open", resolve, { once: true });
  socket.addEventListener("error", reject, { once: true });
});

let nextId = 0;
const pending = new Map();
socket.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);
  const resolve = pending.get(message.id);
  if (resolve) {
    pending.delete(message.id);
    resolve(message.result);
  }
});
function command(method, params = {}) {
  return new Promise((resolve) => {
    const id = ++nextId;
    pending.set(id, resolve);
    socket.send(JSON.stringify({ id, method, params }));
  });
}

const evaluated = await command("Runtime.evaluate", {
  expression: `JSON.stringify({
    boot: document.getElementById("boot")?.textContent ?? null,
    canvases: [...document.querySelectorAll("canvas")].map((canvas) => ({
      width: canvas.width,
      height: canvas.height,
      cssWidth: canvas.clientWidth,
      cssHeight: canvas.clientHeight,
    })),
  })`,
  returnByValue: true,
});
const state = JSON.parse(evaluated.result.value);
if (state.boot !== null) {
  throw new Error(`web boot did not finish: ${state.boot}`);
}
if (
  state.canvases.length !== 1 ||
  state.canvases[0].width < 600 ||
  state.canvases[0].height < 300
) {
  throw new Error(`unexpected canvas state: ${JSON.stringify(state.canvases)}`);
}

const captured = await command("Page.captureScreenshot", { format: "png" });
const png = Buffer.from(captured.data, "base64");
if (png.length < 10_000) {
  throw new Error(`screenshot is suspiciously small (${png.length} bytes)`);
}
fs.writeFileSync(output, png);
socket.close();
console.log(`web smoke passed: ${JSON.stringify(state.canvases[0])}`);
console.log(`screenshot: ${output} (${png.length} bytes)`);
