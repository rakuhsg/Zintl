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

await withTimeout(willClose, 30_000, "timed out waiting for willclose");

// didClose is intentionally not exposed to JS. Retry one operation until the
// native message loop has processed didClose and removed the Window.
await waitForWindowDoesNotExist();

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

async function waitForWindowDoesNotExist() {
  const deadline = Date.now() + 2_000;
  let lastError;

  while (Date.now() < deadline) {
    try {
      await appWindow.setSize({ width: 640, height: 420 });
    } catch (error) {
      const message = String(error?.message ?? error);
      if (message.includes(`window ${appWindow.id} does not exist`)) {
        console.log(`window removed after close: ${message}`);
        return;
      }
      lastError = error;
    }
    await delay(50);
  }

  if (lastError !== undefined) {
    throw lastError;
  }
  throw new Error("window remained usable after close");
}

function delay(ms) {
  return new Promise((resolve) => {
    globalThis.setTimeout(resolve, ms);
  });
}

function withTimeout(promise, ms, message) {
  let timeoutId;
  const timeout = new Promise((_, reject) => {
    timeoutId = globalThis.setTimeout(() => {
      reject(new Error(message));
    }, ms);
  });

  return Promise.race([promise, timeout]).finally(() => {
    globalThis.clearTimeout(timeoutId);
  });
}
