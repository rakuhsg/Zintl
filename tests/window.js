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

globalThis.addEventListener("unload", () => {
  offCommand();
});
