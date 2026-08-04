# Permission persistence

Permission export/import is a trusted embedding API and is not installed into
JavaScript. `PermissionCodec` owns authenticated encryption/sealing; codec keys
never enter Rust, a runtime object, an export envelope, or JS. The runtime never
accepts an envelope directly from unsigned JSON/base64. It parses only bytes
successfully returned by the configured codec's `open` call.

The canonical binary envelope starts with `ZPEM` and version 1, then authenticates
length-prefixed issuer, audience, permission kind, rights, quota, expiry, a
16-byte nonce, opaque scope locator, and stable scope identity. All fields and
the total envelope have hard byte limits. Runtime-local capability/resource IDs,
descriptors, native pointers, canonical paths, and codec secrets are absent.
The codec contract requires confidentiality as well as integrity so the sealed
blob does not reveal the inner locator or identity.

`PermissionPersistenceConfiguration` supplies an exact application issuer and
audience, the codec, and scope-locator encode/resolve callbacks. A macOS embedder
may implement the locator with a security-scoped bookmark; that choice is not a
core dependency. Import checks codec authentication, version, issuer, audience,
kind, expiry, requested-rights subset, and non-increasing quota before resolving
the locator. The resolver must return the authenticated stable identity and a
local directory locator.

Directory import then opens the locator again with the ordinary component-wise
no-symlink policy and compares the opened directory's device/file identity with
the authenticated identity before inserting a fresh runtime-local resource.
Identity mismatch, wrong type, missing path, or symlink leaves no new resource.
The old resource/descriptor is never serialized or reused.

Replay state is runtime-local and bounded. An import atomically reserves its
nonce before reopening, rejects concurrent/previous use, commits only after a
successful reopen, and releases the reservation on failure. This makes a failed
scope resolution or open retryable without allowing two successful imports in
one runtime. Applications that require process- or account-wide replay policy
must implement equivalent durable policy in their codec.
