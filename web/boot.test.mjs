// Run with: node --experimental-vm-modules --test web/boot.test.mjs
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import { createContext, SourceTextModule, SyntheticModule } from "node:vm";

const html = await readFile(new URL("./index.html", import.meta.url), "utf8");
const script = html.match(/<script type="module">([\s\S]*?)<\/script>/)?.[1];
assert.ok(script, "The deployed page must contain the startup module");

// Execute the actual inline module so tests cover the page shipped by Pages.
async function runBoot(options = {}) {
  const events = [];
  const boot = {
    textContent: "Loading Redline…",
    style: {},
    role: "status",
    remove() {
      events.push("remove");
    },
    setAttribute(name, value) {
      assert.equal(name, "role");
      this.role = value;
    },
  };
  const context = createContext({
    document: {
      getElementById(id) {
        assert.equal(id, "boot");
        return options.missingBoot ? null : boot;
      },
      createElement(tag) {
        assert.equal(tag, "canvas");
        return {
          getContext(type, attributes) {
            events.push("webgl");
            assert.equal(type, "webgl2");
            assert.equal(attributes.antialias, false);
            if (options.contextError) throw options.contextError;
            if (options.webgl === false) return null;
            return {
              getExtension(name) {
                assert.equal(name, "WEBGL_lose_context");
                return options.noLoseContext
                  ? null
                  : { loseContext: () => events.push("release") };
              },
            };
          },
        };
      },
    },
  });
  const module = new SourceTextModule(script, {
    context,
    async importModuleDynamically(specifier) {
      events.push("import");
      assert.equal(specifier, "./pkg/redline.js");
      if (options.importError) throw options.importError;
      const pkg = new SyntheticModule(
        ["default"],
        function () {
          this.setExport("default", async () => {
            events.push("init");
            if (Object.hasOwn(options, "initError")) throw options.initError;
          });
        },
        { context },
      );
      await pkg.link(() => {});
      await pkg.evaluate();
      return pkg;
    },
  });
  await module.link(() => {
    assert.fail("Startup dependencies must load inside the guarded dynamic import");
  });
  let error;
  try {
    await module.evaluate();
  } catch (err) {
    error = err;
  }
  return { boot, events, error };
}

test("successful startup releases the probe before importing and starting the game", async () => {
  const { boot, events, error } = await runBoot();
  assert.equal(error, undefined);
  assert.deepEqual(events, ["webgl", "release", "import", "init", "remove"]);
  assert.equal(boot.role, "status");
});

test("WebGL 2 unavailable: show recovery instructions without downloading the game", async () => {
  const { boot, events, error } = await runBoot({ webgl: false });
  assert.match(error.message, /WebGL 2 is unavailable/);
  assert.deepEqual(events, ["webgl"]);
  assert.match(boot.textContent, /hardware acceleration/);
  assert.match(boot.textContent, /another browser or device/);
  assert.equal(boot.role, "alert");
  assert.equal(boot.style.pointerEvents, "auto");
});

test("a blocked context that throws gets the same WebGL recovery instructions", async () => {
  const { boot, events, error } = await runBoot({
    contextError: new Error("Context creation blocked"),
  });
  assert.match(error.message, /WebGL 2 is unavailable/);
  assert.match(boot.textContent, /hardware acceleration/);
  assert.deepEqual(events, ["webgl"]);
});

test("the context-loss extension is optional", async () => {
  const { events, error } = await runBoot({ noLoseContext: true });
  assert.equal(error, undefined);
  assert.deepEqual(events, ["webgl", "import", "init", "remove"]);
});

test("a failed JavaScript module download replaces the loading screen", async () => {
  const failure = new TypeError("Failed to fetch dynamically imported module");
  const { boot, events, error } = await runBoot({ importError: failure });
  assert.equal(error, failure);
  assert.deepEqual(events, ["webgl", "release", "import"]);
  assert.match(boot.textContent, /Redline couldn't start/);
  assert.match(boot.textContent, /Failed to fetch dynamically imported module/);
  assert.match(boot.textContent, /Reload the page/);
  assert.equal(boot.role, "alert");
});

test("a WASM initialization failure stays visible and is still reported", async () => {
  const failure = new Error("WASM download failed");
  const { boot, events, error } = await runBoot({ initError: failure });
  assert.equal(error, failure);
  assert.ok(!events.includes("remove"));
  assert.match(boot.textContent, /WASM download failed/);
  assert.equal(boot.role, "alert");
});

for (const wrap of [String, (message) => new Error(message)]) {
  test(`winit's expected control-flow ${wrap === String ? "string" : "error"} completes startup`, async () => {
    const { events, error } = await runBoot({
      initError: wrap(
        "Using exceptions for control flow, don't mind me. This isn't actually an error!",
      ),
    });
    assert.equal(error, undefined);
    assert.equal(events.at(-1), "remove");
  });
}

test("other errors mentioning control flow are not swallowed", async () => {
  const failure = new Error("Unexpected failure: Using exceptions for control flow,");
  const { boot, events, error } = await runBoot({ initError: failure });
  assert.equal(error, failure);
  assert.ok(!events.includes("remove"));
  assert.match(boot.textContent, /Unexpected failure/);
});

test("a non-Error rejection is shown as text", async () => {
  const { boot, error } = await runBoot({ initError: null });
  assert.equal(error, null);
  assert.match(boot.textContent, /\n\nnull\n\n/);
});

test("missing boot markup does not mask the original initialization error", async () => {
  const failure = new Error("WASM failed");
  const { error } = await runBoot({ missingBoot: true, initError: failure });
  assert.equal(error, failure);
});
