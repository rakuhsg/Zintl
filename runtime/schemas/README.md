# Protocol schemas

Wire payloads are versioned, bounded, and engine-neutral. Runtime request and
completion envelopes use the codec accepted in ADR-0003. Unknown versions and
fields with unsafe lengths are rejected; unchecked struct casts are not a
codec.

Filesystem worker results are bounded internal payloads. Reads return raw bytes.
Metadata is `kind: u8` (`1` file, `2` directory) followed by a big-endian `u64`
length. Directory listings are a big-endian `u32` count followed by repeated
big-endian `u16` UTF-8 NFC name lengths and name bytes. M8 wraps these internal
forms in the stable builtin-op schemas.

The schema-version-1 builtin catalog and stable IDs are recorded in
`docs/builtin-ops.md`. Timer sleep input is exactly one big-endian `u32`
millisecond delay. Filesystem path fields use bounded UTF-8 NFC components and
never contain a native path or descriptor after a directory resource is opened.
