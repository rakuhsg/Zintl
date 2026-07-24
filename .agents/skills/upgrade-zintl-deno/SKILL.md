---
name: upgrade-zintl-deno
description: Upgrade Zintl's vendored Deno release and adapt the local runtime integration. Use when asked to bump `thirdparty/deno.rev` or migrate Zintl between Deno versions, including updating tracked version references, porting and validating `patches/deno` in a clean temporary clone, updating `runtime/zintl-deno` APIs and pinned crates, refreshing `runtime/Cargo.lock`, and running the required Rust and TypeScript checks.
---

# Upgrade Zintl Deno

Upgrade the pinned Deno release without weakening or dropping Zintl's local
WebGPU behavior. Work from a clean target-version checkout and treat successful
compilation and tests as requirements, not substitutes for patch validation.

## Prepare

1. Work from the Zintl repository root.
2. Read `AGENTS.md` and relevant parts of `DESIGN.md`.
3. Inspect `git status --short`, `thirdparty/deno.rev`,
   `setup-deno.sh`, `patches/deno/`, `runtime/zintl-deno/Cargo.toml`,
   and the current `thirdparty/deno` status.
4. Preserve unrelated user changes. Do not reset or overwrite a dirty Deno
   checkout. Confirm its modifications are only the currently tracked patches
   before replacing it.
5. Normalize the requested release to a tag such as `v2.9.4`.

## Update and port the patches

1. Fetch the requested tag in `thirdparty/deno` when the local clone is usable.
   Request network approval when required.
2. Run:

   ```bash
   .agents/skills/upgrade-zintl-deno/scripts/verify-deno-patches.sh --keep vX.Y.Z
   ```

   The script clones the exact tag into a unique temporary directory, applies
   every `patches/deno/*.patch` in order, checks whitespace, and retains the
   checkout for inspection.
3. If a patch fails, use the retained clean checkout to port the behavior:

   - Inspect the failing hunk and the corresponding old-version implementation.
   - Adapt to upstream API changes while preserving the patch's intent.
   - Keep unsafe blocks accompanied by a short `SAFETY` comment.
   - Generate the candidate diff with `git diff -- <affected-files>`.
   - Update the tracked patch with `apply_patch`; do not overwrite it through
     shell redirection.
   - If the repository-level `git diff --check` flags a blank context line in
     the patch file, remove only its trailing context-marker space and confirm
     that `git apply --check` still accepts the patch.
   - Rerun the verifier from a fresh target-version checkout.

4. Verify the regenerated patch has target-version blob IDs and hunk positions,
   applies with `git apply --check`, and leaves `git diff --check` clean.
5. Replace the ignored `thirdparty/deno` working tree only after validation.
   Move the old tree to a uniquely named directory under `/private/tmp` first,
   then move the retained patched checkout into place. Report the backup path.

## Update tracked version references

1. Change `thirdparty/deno.rev` to the target tag.
2. Change `.github/workflows/deno-check.yml` to install the target release.
3. Change the Deno version documented in `AGENTS.md`.
4. Run `git grep` for the old version and inspect every result. Do not alter
   unrelated crate versions in lockfiles merely because their number matches.
5. If Deno's `rust-toolchain.toml` changed, align Zintl's root
   `rust-toolchain.toml` so ordinary commands use the required compiler.

## Upgrade `zintl-deno`

1. Compare direct pins in `runtime/zintl-deno/Cargo.toml` with the target Deno
   workspace dependencies in `thirdparty/deno/Cargo.toml`. Align exact versions
   such as `deno_ast`, `deno_error`, and `sys_traits`.
2. Run `cargo check` from `runtime/` to resolve dependencies and refresh
   `runtime/Cargo.lock`.
3. Fix target-version API errors narrowly. Search the target Deno checkout for
   current call sites and examples before inventing adapters.
4. Preserve Zintl's architecture and snapshots. Do not disable features or drop
   extensions merely to make compilation succeed.
5. Format Rust changes from `runtime/` with `cargo fmt --all`.

## Validate

Run all applicable checks:

```bash
.agents/skills/upgrade-zintl-deno/scripts/verify-deno-patches.sh vX.Y.Z
cd runtime
cargo fmt --all -- --check
cargo check --locked
cargo test --locked
```

Also run `deno check libs/*.ts` when files under `libs/` changed. Confirm:

- `thirdparty/deno.rev` and the local Deno `HEAD` identify the requested tag.
- The local Deno diff contains exactly the intended patch changes.
- A fresh target checkout accepts all patches.
- `git diff --check` succeeds in both repositories.
- No tracked reference still identifies the old Deno release.
- The Cargo lockfile contains the target Deno crate versions.

Report changed files, patch verification, check and test results, upstream-only
warnings, and any retained temporary backup.
