const WIDTH = 640;
const HEIGHT = 480;
const WINDOW_READY_DELAY_MS = 250;
const frameLimit = Deno.args[0] === undefined ? Infinity : Number(Deno.args[0]);

if (frameLimit <= 0 || Number.isNaN(frameLimit)) {
  throw new Error("Frame limit must be a positive number");
}

const libName = "libwebgpu_byow_macos.dylib";

const libPath = new URL(`./.build/debug/${libName}`, import.meta.url);
const dylib = Deno.dlopen(libPath, {
  byow_create_window: {
    parameters: ["u32", "u32"],
    result: "pointer",
  },
  byow_poll_events: {
    parameters: [],
    result: "void",
  },
  byow_close_window: {
    parameters: [],
    result: "void",
  },
} as const);

const adapter = await navigator.gpu.requestAdapter();
if (adapter === null) {
  dylib.close();
  throw new Error("No WebGPU adapter available");
}

const handle = dylib.symbols.byow_create_window(WIDTH, HEIGHT);

if (handle === null) {
  dylib.close();
  throw new Error("Failed to create window");
}

const surface = new Deno.UnsafeWindowSurface({
  system: "core-animation",
  windowHandle: handle,
  displayHandle: null,
  width: WIDTH,
  height: HEIGHT,
});

const context = surface.getContext("webgpu") as GPUCanvasContext | null;
if (context === null) {
  dylib.symbols.byow_close_window();
  dylib.close();
  throw new Error("Failed to create WebGPU context");
}

const device = await adapter.requestDevice();
const format = navigator.gpu.getPreferredCanvasFormat();

context.configure({
  device,
  format,
  alphaMode: "opaque",
});

// Waiting for the window to show.
const readyAt = performance.now() + WINDOW_READY_DELAY_MS;
do {
  dylib.symbols.byow_poll_events();
  await new Promise((resolve) => setTimeout(resolve, 16));
} while (performance.now() < readyAt);

let frame = 0;

try {
  while (frame < frameLimit) {
    dylib.symbols.byow_poll_events();

    const texture = context.getCurrentTexture();
    const view = texture.createView();
    const t = frame / 60;
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginRenderPass({
      colorAttachments: [{
        view,
        clearValue: {
          r: 0.5 + 0.5 * Math.sin(t),
          g: 0.5 + 0.5 * Math.sin(t + 2.1),
          b: 0.5 + 0.5 * Math.sin(t + 4.2),
          a: 1,
        },
        loadOp: "clear",
        storeOp: "store",
      }],
    });
    pass.end();

    device.queue.submit([encoder.finish()]);
    surface.present();

    frame++;
    await new Promise((resolve) => setTimeout(resolve, 16));
  }
} finally {
  context.unconfigure();
  dylib.symbols.byow_close_window();
  dylib.close();
  device.destroy();
}
