# zpd-appkit example

This example installs an application menu and a custom **Example** menu, then
creates and shows an AppKit window with an `NSTextField` label, editable text
field, and `NSButton` laid out using Auto Layout. Selecting **Example > Print
Hello** prints a greeting; clicking **Save** prints the text entered by the user.

Run it from the repository root on macOS:

```sh
cargo run --manifest-path platform/Cargo.toml -p zpd-appkit-example
```

## Main-thread scheduler

The `scheduler` example sends closures from a worker thread through a shared
queue. `RunLoopScheduler` wakes AppKit, and `ApplicationDelegate::perform`
drains and runs the queued closures on the process main thread.

```sh
cargo run --manifest-path platform/Cargo.toml -p zpd-appkit-example --example scheduler
```
