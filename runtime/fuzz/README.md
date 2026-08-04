# Runtime fuzz targets

- `engine_event_decoder`: arbitrary bytes for the canonical `ZJE1` decoder.
- `path_parser`: arbitrary filesystem paths.
- `resource_operations`: generated capability/resource operation sequences.

Run from the repository root with `cargo fuzz`, for example:

```sh
cargo +nightly fuzz run engine_event_decoder --fuzz-dir runtime/fuzz
```
