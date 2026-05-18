const window = Zintl.window.create({
  bounds: {
    x: 80,
    y: 80,
    width: 640,
    height: 420,
  },
  commands: {
    appMenu: {
      items: [
        {
          title: "About Zintl",
          role: "about",
        },
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
          {
            id: "file.close",
            title: "Close",
            key: "w",
            modifiers: ["cmd"],
          },
        ],
      },
      {
        title: "View",
        items: [
          {
            id: "view.zoom-in",
            title: "Zoom In",
            key: "+",
            modifiers: ["cmd"],
          },
          {
            id: "view.zoom-out",
            title: "Zoom Out",
            key: "-",
            modifiers: ["cmd"],
          },
        ],
      },
    ],
  },
});

if (typeof window.id !== "number" || window.id <= 0) {
  throw new Error("Zintl.window.create() must return a window handle with a positive id");
}

if (
  typeof app !== "object" ||
  typeof app.eventBus?.subscribe !== "function" ||
  typeof app.eventBus?.poll !== "function"
) {
  throw new Error("app.eventBus must expose subscribe() and poll()");
}

const initialEvent = app.eventBus.poll();
if (initialEvent !== undefined) {
  assertAppEvent(initialEvent);
}

const offEventBusCreated = app.eventBus.subscribe("window.created", (event) => {
  assertAppEvent(event);
  if (event.type !== "window.created") {
    throw new Error(`expected window.created app event, got ${event.type}`);
  }
  if (event.windowId !== window.id) {
    throw new Error(`expected app event bus created for window ${window.id}, got ${event.windowId}`);
  }

  console.log(`app event bus created: ${event.windowId}`);
});

const offEventBusWillClose = app.eventBus.subscribe("window.willClose", (event) => {
  assertAppEvent(event);
  if (event.type !== "window.willClose") {
    throw new Error(`expected window.willClose app event, got ${event.type}`);
  }
  if (event.windowId !== window.id) {
    throw new Error(
      `expected app event bus willClose for window ${window.id}, got ${event.windowId}`,
    );
  }

  console.log(`app event bus will close: ${event.windowId}`);
});

const offUnsubscribedCommand = app.eventBus.subscribe("window.command", () => {
  throw new Error("unsubscribed app.eventBus listener must not be called");
});
if (typeof offUnsubscribedCommand !== "function") {
  throw new Error("app.eventBus.subscribe() must return an unsubscribe function");
}
offUnsubscribedCommand();

const offCreated = window.onCreated((event) => {
  if (event.windowId !== window.id) {
    throw new Error(`expected created for window ${window.id}, got ${event.windowId}`);
  }

  console.log(`window created: ${event.windowId}`);
});

const offWillClose = window.onWillClose((event) => {
  if (event.windowId !== window.id) {
    throw new Error(`expected willClose for window ${window.id}, got ${event.windowId}`);
  }

  console.log(`window will close: ${event.windowId}`);
});

window.setSize({ width: 720, height: 480 });
window.setPosition({ x: 120, y: 120 });
window.setBounds({ x: 160, y: 140, width: 760, height: 500 });
window.setCommands({
  appMenu: {
    items: [
      {
        title: "About Zintl",
        role: "about",
      },
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
        {
          id: "file.close",
          title: "Close",
          key: "w",
          modifiers: ["cmd"],
        },
      ],
    },
    {
      title: "Window",
      items: [
        {
          id: "window.toggle-inspector",
          title: "Toggle Inspector",
          key: "i",
          modifiers: ["cmd", "alt"],
        },
      ],
    },
  ],
});

const offCommand = window.onCommand((event) => {
  if (event.windowId !== window.id) {
    throw new Error(`expected command for window ${window.id}, got ${event.windowId}`);
  }

  console.log(`window command: ${event.commandId}`);
});

const offEventBusCommand = app.eventBus.subscribe("window.command", (event) => {
  assertAppEvent(event);
  if (event.type !== "window.command") {
    throw new Error(`expected window.command app event, got ${event.type}`);
  }
  if (event.windowId !== window.id) {
    throw new Error(`expected app event bus command for window ${window.id}, got ${event.windowId}`);
  }
  if (typeof event.commandId !== "string" || event.commandId.length === 0) {
    throw new Error("window.command app event must include a non-empty commandId");
  }

  console.log(`app event bus command: ${event.commandId}`);
});

function assertAppEvent(event) {
  if (event == null || typeof event !== "object") {
    throw new Error("app event must be an object");
  }
  if (
    event.type !== "window.command" &&
    event.type !== "window.created" &&
    event.type !== "window.willClose"
  ) {
    throw new Error(`unexpected app event type: ${event.type}`);
  }
  if (typeof event.windowId !== "number" || event.windowId <= 0) {
    throw new Error("app event must include a positive windowId");
  }
}

globalThis.addEventListener("unload", () => {
  offCreated();
  offWillClose();
  offCommand();
  offEventBusCreated();
  offEventBusWillClose();
  offEventBusCommand();
});
