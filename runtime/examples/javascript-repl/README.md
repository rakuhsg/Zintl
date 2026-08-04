# Rust JavaScript REPL

This example runs Swift JavaScriptCore behind the engine-neutral Rust embedding
API. The JavaScript realm persists in interactive mode, while filesystem access
is deny-by-default and mediated by a Rust permission callback.

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
`await`. When stdin is a pipe, permission answers are read from `/dev/tty`, so
the script bytes cannot be consumed as an approval response.

```javascript
const directory = await Zintl.requestDirectory("/absolute/path", {
  read: true,
  metadata: true,
});
const bytes = await directory.readRelative("notes.txt", { maxBytes: 65536 });
console.log(bytes);
const file = await directory.openRelative("notes.txt", { read: true });
console.log(await file.readString({ maxBytes: 65536 }));
await file.close();
await directory.close();
```

The terminal displays the operation, requested rights and directory. Only an
explicit `y` grants attenuated authority. JavaScript receives opaque directory
and file objects, never a descriptor, resource ID, native pointer or permission
blob. `console.debug`, `log`, `info`, `warn` and `error` are forwarded through
the bounded engine event queue. `file.readString()` performs strict UTF-8
decoding and rejects malformed input rather than replacing invalid bytes.
