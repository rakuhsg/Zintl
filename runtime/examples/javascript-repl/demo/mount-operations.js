(async () => {
  const directory = "mount://project/.zintl-repl-mount-demo";
  const original = `${directory}/hello.txt`;
  const renamed = `${directory}/renamed.txt`;

  // Clean up leftovers from an interrupted previous run.
  await Zintl.removeFile(original).catch(() => {});
  await Zintl.removeFile(renamed).catch(() => {});
  await Zintl.removeDirectory(directory).catch(() => {});

  await Zintl.mkdir(directory);
  await Zintl.writeFile(original, "Hello from Zintl 👋");
  const beforeRename = await Zintl.readFile(original, "utf8");

  await Zintl.rename(original, renamed);
  const afterRename = await Zintl.readFile(renamed, "utf8");

  await Zintl.removeFile(renamed);
  await Zintl.removeDirectory(directory);

  return { beforeRename, afterRename, cleanedUp: true };
})()
