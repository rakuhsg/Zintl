# Rust JavaScript REPL

This example runs Swift JavaScriptCore behind the engine-neutral Rust embedding
API. The JavaScript realm persists in interactive mode. The host mounts its
current directory as the `fs` virtual filesystem and mediates each read through
an application-owned Rust `Authority`.

Run interactively from `runtime`:

```console
cargo run -p javascript-repl
```

The interactive prompt evaluates one complete line at a time. For a multiline
script, pipe the complete source to the binary:

```console
cat a.js | ./target/debug/javascript-repl
```

Batch input is read completely before evaluation and supports top-level
`await`. When stdin is a pipe, authorization answers are read from `/dev/tty`, so
the script bytes cannot be consumed as an approval response.

```javascript
const text = await Zintl.readFile("fs://notes.txt", "utf8");
console.log(text);
const bytes = await Zintl.readFile("fs://image.bin");
console.log(bytes);
```

The terminal displays the VFS, operation and relative path. Only an explicit
`y` grants access. JavaScript cannot provide an absolute OS path, and the
runtime does not cache the application's decision. `console.debug`, `log`,
`info`, `warn` and `error` are forwarded through the bounded engine event queue.
The `utf8` mode performs strict decoding and rejects malformed input.
