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

const removedCommandListener = () => {
  throw new Error("removed oncommandclick listener must not fire");
};
app.addEventListener("oncommandclick", removedCommandListener);
app.removeEventListener("oncommandclick", removedCommandListener);

app.addEventListener("oncommandclick", (event) => {
  if (typeof event.id !== "string" || event.id.length === 0) {
    throw new Error("oncommandclick must include a command id");
  }
  console.log(`command clicked: ${event.id}`);


    let commands = app.commands;
    commands.menus.push(
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
        }
    );
    app.commands = commands;
});

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
    if (event.windowId !== appWindow.id) {
      throw new Error(
        `click was dispatched to the wrong window: ${event.windowId}`,
      );
    }
    console.log(`window clicked: ${appWindow.id}`);
  });
}

if (firstWindow.id <= 0 || secondWindow.id <= 0) {
  throw new Error("app.createWindow() must return positive window ids");
}

await firstWindow.setSize({ width: 720, height: 480 });
await firstWindow.setPosition({ x: 120, y: 120 });
await firstWindow.setBounds({ x: 160, y: 140, width: 760, height: 500 });
