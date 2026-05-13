# AGENTS.md

## Project

Zintl is a GPU-first, framework-agnostic desktop app runtime for JavaScript and WebGPU.
See `DESIGN.md` for the product and architecture direction.

## Structure

- `zintl/Cargo.toml`: Rust workspace root.
- `zintl/crates/zintl`: public/core crate placeholder.
- `zintl/crates/zintl-app`: native app entry point.
- `zintl/crates/zintl-native`: platform message loop, main actor, and native bindings.
- `zintl/crates/zintl-render`: rendering crate placeholder.
- `platform/ZintlAppkitSupport`: Swift Package used by macOS AppKit FFI.

## Commands

- Check/build all Rust crates: `cd zintl && cargo check`
- Run tests: `cd zintl && cargo test`
- Run the app on macOS: `cd zintl && cargo run -p zintl-app`
- Build AppKit support directly: `swift build -c release --package-path platform/ZintlAppkitSupport`

`zintl-app` builds the Swift AppKit support package from `build.rs` on macOS.

## Guidelines

- Keep changes small and aligned with `DESIGN.md`.
- Always run Cargo commands from the workspace directory, e.g. `cd zintl && cargo check`.
- Use `cd zintl && cargo fmt` before finishing Rust changes.
- Keep comments clear and short.
- Add SAFETY note to unsafe blocks.
