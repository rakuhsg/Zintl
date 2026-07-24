if (globalThis.Zintl !== undefined) {
  throw new Error("Zintl.window must be removed");
}
if (typeof app?.createWindow !== "function") {
  throw new Error("app.createWindow must be available");
}
if ("eventBus" in app) {
  throw new Error("app.eventBus must be removed");
}

app.commands = {
  appMenu: {
    items: [
      { title: "About Zintl", role: "about" },
      {
        title: "Quit Zintl",
        role: "quit",
        key: "q",
        modifiers: ["cmd"],
      },
    ],
  },
  menus: [
    {
      title: "File",
      items: [
        {
          id: "file.new",
          title: "New",
          key: "n",
          modifiers: ["cmd"],
        },
      ],
    },
  ],
};

globalThis.addEventListener("onload", (event) => {
  if (typeof event.windowId !== "number" || event.windowId <= 0) {
    throw new Error("onload must include a positive windowId");
  }
  console.log(`window loaded: ${event.windowId}`);
});

globalThis.addEventListener("willclose", (event) => {
  if (typeof event.windowId !== "number" || event.windowId <= 0) {
    throw new Error("willclose must include a positive windowId");
  }
  console.log(`window will close: ${event.windowId}`);
});

globalThis.addEventListener("click", (event) => {
  if (typeof event.commandId !== "string" || event.commandId.length === 0) {
    throw new Error("command click must include commandId");
  }
  console.log(`command clicked: ${event.commandId}`);
});

const firstWindow = await app.createWindow({
  bounds: { x: 80, y: 80, width: 640, height: 420 },
});
const secondWindow = await app.createWindow({
  size: { width: 520, height: 320 },
  position: { x: 120, y: 120 },
});

if (firstWindow.id <= 0 || secondWindow.id <= 0) {
  throw new Error("app.createWindow() must return positive window ids");
}

await firstWindow.setSize({ width: 720, height: 480 });
await firstWindow.setPosition({ x: 120, y: 120 });
await firstWindow.setBounds({ x: 160, y: 140, width: 760, height: 500 });
