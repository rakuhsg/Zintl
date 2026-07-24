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

const firstWindow = await app.createWindow({
  bounds: { x: 80, y: 80, width: 640, height: 420 },
});
if (typeof firstWindow.addEventListener !== "function") {
  throw new Error("ZintlWindow.addEventListener must be available");
}
installWindowListeners(firstWindow);

const secondWindow = await app.createWindow({
  size: { width: 520, height: 320 },
  position: { x: 120, y: 120 },
});
installWindowListeners(secondWindow);

function installWindowListeners(appWindow) {
  appWindow.addEventListener("onload", (event) => {
    if (event.windowId !== appWindow.id) {
      throw new Error(
        `onload was dispatched to the wrong window: ${event.windowId}`,
      );
    }
    console.log(`window loaded: ${event.windowId}`);
  });

  appWindow.addEventListener("willclose", (event) => {
    if (event.windowId !== appWindow.id) {
      throw new Error(
        `willclose was dispatched to the wrong window: ${event.windowId}`,
      );
    }
    console.log(`window will close: ${event.windowId}`);
  });

  appWindow.addEventListener("click", (event) => {
    if (typeof event.commandId !== "string" || event.commandId.length === 0) {
      throw new Error("command click must include commandId");
    }
    if (event.windowId !== appWindow.id) {
      throw new Error(
        `click was dispatched to the wrong window: ${event.windowId}`,
      );
    }
    console.log(
      `command clicked in window ${appWindow.id}: ${event.commandId}`,
    );
  });
}

if (firstWindow.id <= 0 || secondWindow.id <= 0) {
  throw new Error("app.createWindow() must return positive window ids");
}

await firstWindow.setSize({ width: 720, height: 480 });
await firstWindow.setPosition({ x: 120, y: 120 });
await firstWindow.setBounds({ x: 160, y: 140, width: 760, height: 500 });
