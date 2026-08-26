# AGENTS.md

## Guidelines

- Do not edit files under `thirdparty/`.
- Always run Cargo commands from the workspace directory, e.g. `cd zintl && cargo check`.
- Use `cd zintl && cargo fmt` before finishing Rust changes.
- Use `swift format format <file>` before finishing Swift changes.
- Keep comments clear and short.
- Leave a comment upon test code. Make it short and understandable.
- Add a short comment to every test explaining the behavior or contract it verifies.
- Add SAFETY note to unsafe blocks.
- Don't wrtie a function in module scope directly unless the function is platform-specific code.
