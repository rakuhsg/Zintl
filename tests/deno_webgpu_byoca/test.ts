const WIDTH = 640;
const HEIGHT = 480;

const libName = Deno.build.os === "darwin"
  ? "libwebgpu_byow_macos.dylib"
  : Deno.build.os === "windows"
  ? "webgpu_byow_macos.dll"
  : "libwebgpu_byow_macos.so";

const libPath = new URL(`./target/debug/${libName}`, import.meta.url);
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

const surface = new Deno.UnsafeWindowSurface({
  system: "core-animation",
  windowHandle: dylib.symbols.byow_create_window(WIDTH, HEIGHT),
  displayHandle: null,
  width: WIDTH,
  height: HEIGHT,
});

const device = await adapter.requestDevice();
const context = surface.getContext("webgpu");
const format = navigator.gpu.getPreferredCanvasFormat();

context.configure({
  device,
  format,
  alphaMode: "opaque",
});

let frame = 0;

try {
  while (true) {
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
  surface.getContext("webgpu").unconfigure();
  dylib.symbols.byow_close_window();
  dylib.close();
  device.destroy();
}
