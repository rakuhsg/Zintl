const appWindow = await app.createWindow({
  bounds: {
    x: 120,
    y: 120,
    width: 520,
    height: 320,
  },
});

console.log(`window created: ${appWindow.id}`);
console.log("close the window to verify post-close operations reject");

const willClose = new Promise((resolve) => {
  const listener = (event) => {
    if (event.windowId !== appWindow.id) {
      return;
    }
    appWindow.removeEventListener("willclose", listener);
    console.log(`window will close: ${event.windowId}`);
    resolve();
  };
  appWindow.addEventListener("willclose", listener);
});

await willClose;

await assertWindowDoesNotExist(
  "setSize",
  appWindow.setSize({ width: 640, height: 420 }),
);
await assertWindowDoesNotExist(
  "setPosition",
  appWindow.setPosition({ x: 180, y: 180 }),
);
await assertWindowDoesNotExist(
  "setBounds",
  appWindow.setBounds({ x: 180, y: 180, width: 640, height: 420 }),
);
console.log("post-close window operations rejected without crashing");
Deno.exit(0);

async function assertWindowDoesNotExist(name, promise) {
  try {
    await promise;
  } catch (error) {
    const message = String(error?.message ?? error);
    if (!message.includes(`window ${appWindow.id} does not exist`)) {
      throw new Error(`${name} rejected with unexpected error: ${message}`);
    }
    console.log(`${name} rejected after close: ${message}`);
    return;
  }

  throw new Error(`${name} must reject after window close`);
}
