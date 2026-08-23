# zintl-desktop-build

Add the crate to the application's build dependencies:

```toml
[dependencies]
zintl-desktop = { path = "path/to/zintl/ui/zintl-desktop" }

[build-dependencies]
zintl-desktop-build = { path = "path/to/zintl/ui/zintl-desktop-build" }
```

Then configure the application binary from `build.rs`:

```rust
fn main() {
    zintl_desktop_build::configure();
}
```
