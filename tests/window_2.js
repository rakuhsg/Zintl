const win = await app.createWindow({});

globalThis.addEventListener("willclose", (event) => {
  if (event.windowId === win.id) {
    console.log(`Window will close ${event.windowId}`);
  }
});
