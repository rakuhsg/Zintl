const win = await app.createWindow({});

win.addEventListener("click", (e) => {
  console.log("click event");
});

win.addEventListener("willclose", (event) => {
  if (event.windowId === win.id) {
    console.log(`Window will close ${event.windowId}`);
  }
});
