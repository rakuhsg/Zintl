# Fuzz targets

The standalone `cargo-fuzz` package is excluded from the stable runtime
workspace and contains three bounded targets:

- `ffi_op_decoder`: arbitrary request/completion bytes plus valid-pointer C ABI
  submission, capped at 64 KiB;
- `path_parser`: UTF-8 path separator, normalization, NUL, and length cases;
- `resource_operations`: at most 4096 insert/use/close operations in a 32-slot
  generational table.

CI runs each target with explicit time, input, timeout, and RSS limits. Crashing
inputs must become regression fixtures under `tests/fixtures/` before a fix is
merged. Run locally with, for example:

```sh
cargo +nightly fuzz run ffi_op_decoder --fuzz-dir runtime/fuzz -- \
  -max_total_time=60 -max_len=65536 -timeout=5 -rss_limit_mb=2048
```
