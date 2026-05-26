# AGENTS.md

## Project

Zintl is a GPU-first, framework-agnostic desktop app runtime for JavaScript and WebGPU.
See `DESIGN.md` for the product and architecture direction.

## Structure

- `runtime/Cargo.toml`: A runtime workspace root.
- `runtime/crates/zintl`: It's just placeholder for now.
- `runtime/crates/zintl-app`: native app entry point.
- `runtime/crates/zintl-deno`: Deno runtime for app.
- `runtime/crates/zintl-native`: platform message loop, main actor, and native bindings.
- `runtime/crates/zintl-render`: rendering with wgpu and vello.
- `platform/ZintlAppkitSupport`: Swift Package used by macOS AppKit FFI.
- `thirdparty/deno`: Deno 2.8.0 with a local patch applied to the WebGPU extension.
- `patches/deno`: A local patch files for deno.

## Commands

- Check/build all Rust crates: `cd zintl && cargo check`
- Run tests: `cd zintl && cargo test`
- Run the app on macOS: `cd zintl && cargo run -p zintl-app`
- Build AppKit support directly: `swift build -c release --package-path platform/ZintlAppkitSupport`

`zintl-app` builds the Swift AppKit support package from `build.rs` on macOS.

## Guidelines

- Keep changes small and aligned with `DESIGN.md`.
- Do not edit files under `thirdparty/`.
- Always run Cargo commands from the workspace directory, e.g. `cd zintl && cargo check`.
- Use `cd zintl && cargo fmt` before finishing Rust changes.
- Use `swift format format <file>` before finishing Swift changes.
- Keep comments clear and short.
- Add SAFETY note to unsafe blocks.
