# PermissionReplDemo

This is a macOS AppKit host application demonstrating the embedding API. It is
an independent Swift package under `runtime/examples`; no AppKit, CryptoKit, UI,
or Demo persistence dependency enters the production runtime crates/packages.

## Build and run

Build the Rust static library first, then the Demo:

```sh
cargo build --manifest-path runtime/Cargo.toml -p runtime-ffi
swift build --package-path runtime/examples/PermissionReplDemo
swift run --package-path runtime/examples/PermissionReplDemo PermissionReplDemo
```

Run the Demo-specific tamper/identity tests with:

```sh
swift test --package-path runtime/examples/PermissionReplDemo
```

The JavaScriptCore executor is the non-main serial queue
`dev.zintl.demo.javascript`. Only AppKit alert/window work runs on `MainActor`.
The permission callback awaits a sheet continuation and never blocks the main
run loop. Filesystem jobs and Promise drains remain on runtime worker/JS
executors. Runtime integration tests assert the executor is non-main; the Demo's
pending label and Cancel/Shutdown/Recreate controls provide a manual UI
responsiveness check while a dialog or Promise is pending.

## Permission flow

`Zintl.requestDirectory` displays the full, unshortened JS-requested directory,
operation origin, and every requested right. The choices are Deny, Allow Once,
and Save and Allow. No directory picker is shown, no parent/root substitution is
performed, and rights are not widened. Closing the window, Cancel, or Shutdown
ends every visible sheet and settles its continuation once with deny/cancel
semantics before runtime teardown.

Save and Allow performs one exact-scope host reopen after the JS grant, exports
an authenticated permission, and stores only the opaque blob in the Demo's
Application Support directory. AES-GCM uses a separate mode-0600 application
key. The scope locator is a macOS security-scoped bookmark; import checks expiry,
issuer/audience, nonce replay, rights attenuation, bookmark identity, and the
directory identity reopened by Rust. On the next launch a valid permission is
installed as `savedDirectory`; tampered, expired, replaced, or otherwise invalid
data reports that permission must be requested again. Blobs, keys, identities,
virtual handles, native pointers, and raw descriptors never appear in REPL
history, alerts, or audit output.

## Samples

Create `/tmp/zintl-demo/sample.txt`, then run the initial read-only sample. A
write attempt is rejected because the capability has no write/create rights:

```js
Zintl.requestDirectory('/tmp/zintl-demo', { read: true })
  .then(async directory => {
    const file = await directory.openRelative('sample.txt', { read: true });
    const bytes = await file.read({ maxBytes: 65536 });
    await file.close();
    const write = await directory.openRelative(
      'denied.txt', { write: true, create: true })
      .then(() => 'unexpected', error => error.code);
    return [Array.from(bytes), write];
  });
```

Traversal and symlink escape are rejected:

```js
Zintl.requestDirectory('/tmp/zintl-demo', { read: true })
  .then(directory => directory.openRelative('../outside', { read: true }))
  .catch(error => error.code);
```

After a saved permission imports, read through `savedDirectory`. The Custom Op
Sample button loads a concise Swift-registered async byte reversal:

```js
savedDirectory.openRelative('sample.txt', { read: true })
  .then(async file => {
    try { return await file.read({ maxBytes: 65536 }); }
    finally { await file.close(); }
  });
Zintl.invoke('dev.zintl.demo.reverse', new Uint8Array([90, 105, 110, 116, 108]))
  .then(bytes => Array.from(bytes));
```

The shared runtime security suite covers forged receivers, hidden native bridge
globals, outside scope, `..`, symlinks, write denial, callback shutdown races,
tampered export, non-main execution, and exactly-once settlement. The Demo adds
only the UI choice, bookmark/AES-GCM adapters, app storage, and controls.
