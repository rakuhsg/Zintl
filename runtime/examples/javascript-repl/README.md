# Rust JavaScript REPL

This example embeds the pure-Rust Boa engine behind `runtime-embed`. It keeps a
single JavaScript realm alive and exposes a frozen, synchronous `Zintl` host API.
Filesystem access is deny-by-default.

Run it from `runtime`:

```console
cargo run -p javascript-repl
```

Request a directory from JavaScript:

```javascript
Zintl.requestDirectory("/absolute/path", "read,metadata")
```

The terminal displays the exact operation, rights, and directory, then asks
`Allow once? [y/N]`. Only an explicit `y` installs the directory in opaque Rust
host state. Terminal input stays on the REPL thread; a one-shot gate requires
the runtime permission request to match the displayed operation, directory, and
rights exactly. JavaScript receives no descriptor, resource ID, canonical path,
or permission blob.

```javascript
Zintl.readTextFile("notes.txt")
Zintl.stat("notes.txt")
Zintl.closeDirectory()
```

`writeTextFile` requires a grant containing both `write` and `truncate`. File
payloads and source input are limited to 1 MiB; recursion and loop iterations
are also bounded. `.help` prints the available APIs and `.exit` shuts the engine
down before the runtime.
