const win = await Zintl.window.create({});

app.eventBus.subscribe("window.willClose", (e) => {
    console.log(`Window will close ${e.windowId}`);
});

win.onWillClose((e) => {
    console.log("hello, world");
});
