# AGENTS.md

## Project

Zintl is a GPU-first, framework-agnostic desktop app runtime for JavaScript and WebGPU.
See `DESIGN.md` for the product and architecture direction.

## Structure

- `Cargo.toml`: Rust workspace root.
- `crates/zintl`: public/core crate placeholder.
- `crates/zintl-app`: native app entry point.
- `crates/zintl-native`: platform message loop, main actor, and native bindings.
- `crates/zintl-render`: rendering crate placeholder.
- `platform/ZintlAppkitSupport`: Swift Package used by macOS AppKit FFI.

## Commands

- Check/build all Rust crates: `cargo check`
- Run tests: `cargo test`
- Run the app on macOS: `cargo run -p zintl-app`
- Build AppKit support directly: `swift build -c release --package-path platform/ZintlAppkitSupport`

`zintl-app` builds the Swift AppKit support package from `build.rs` on macOS.

## Guidelines

- Keep changes small and aligned with `DESIGN.md`.
- Use `cargo fmt` before finishing Rust changes.
- Keep comments clear and short.
- Add SAFETY note to unsafe blocks.
