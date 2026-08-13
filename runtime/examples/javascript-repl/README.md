# Rust JavaScript REPL

This example runs JavaScriptCore or V8 through `ZjsHost` on the main-thread
`MessageLoopIo`. The JavaScript realm persists in interactive mode. A separate
IO thread mounts the current directory as `project` and owns every real
filesystem descriptor. The V8 backend is built directly from `rusty_v8` and
does not build or depend on Zintl's Deno workspace.

Run interactively from `runtime`:

```console
cargo run -p javascript-repl
```

Select an engine when starting the REPL:

```console
cargo run -p javascript-repl -- --engine jsc
cargo run -p javascript-repl -- --engine v8
```

The default is JavaScriptCore on macOS and V8 on other Unix platforms.
JavaScriptCore is only available on macOS.

The interactive prompt evaluates one complete line at a time. For a multiline
script, pipe the complete source to the binary:

```console
cat a.js | ./target/debug/javascript-repl
```

Each interactive input line is evaluated as one task. Piped input is evaluated
as one complete script, so multiline async demos work.

```javascript
const text = await Zintl.readFile("mount://project/notes.txt", "utf8");
console.log(text);
const bytes = await Zintl.readFile("mount://project/image.bin");
console.log(bytes);
```

Run the mount mutation demo from `runtime`:

```console
cargo run -p javascript-repl < examples/javascript-repl/demo/mount-operations.js
```

The demo uses `Zintl.mkdir`, `writeFile`, `readFile`, `rename`,
`removeFile`, and `removeDirectory`, then removes every entry it created.

The IO service resolves the mount name and relative path with capability-based
fd-relative operations. JavaScript receives logical mount/file IDs only and
cannot provide an absolute OS path. `console.debug`, `log`, `info`, `warn` and
`error` are forwarded through the bounded engine event queue. The `utf8` mode
performs strict decoding and rejects malformed input.
