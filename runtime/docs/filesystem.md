# Descriptor-relative filesystem

Filesystem access is empty by default. A trusted embedder first returns an
explicit allow decision containing an absolute directory locator and a bounded
rights set. The locator remains untrusted data: `open_approved_directory` starts
from an opened `/` descriptor and opens every normal component with
`O_DIRECTORY`, `O_NOFOLLOW`, and `O_CLOEXEC`. It stores the resulting descriptor
behind a runtime-scoped generational `ResourceHandle`; the descriptor and its
numeric value are never exposed.

Operation paths are UTF-8, NFC, relative, and bounded to 4096 bytes, 128
components, and 255 bytes per component. Empty components, `.`, `..`, absolute
syntax, NUL, and backslash are rejected. Each intermediate directory is opened
relative to the previous descriptor with no-follow flags and then type-checked.
Final file descriptors are also type-checked so FIFOs, devices, sockets, and
symlinks cannot turn a worker into an unbounded or authority-bearing channel.

Read, write, create-directory, list, metadata, remove, and rename operations
validate resource generation, kind, and explicit rights before a job is queued.
The canonical file path is `directory.openRelative(path, rights)`, which mints
an attenuated, runtime-local file resource. JavaScript then calls `read`,
`write`, `stat`, and `close` on that opaque resource; neither its descriptor nor
its table identity is observable. Opening runs away from the JS executor, and
file I/O runs in the bounded worker pool. Directory-relative convenience
operations retain the same validation and worker contracts.
The root descriptor is duplicated while the resource table is locked; blocking
work then runs in the fixed filesystem worker pool without that lock. Queue,
completion, input, read, listing-entry, and encoded-output limits are mandatory.
Cancellation is checked before the operation and during read/list loops.
Regular-file read/write loops preserve partial progress, treat zero read as EOF,
retry `EINTR` and transient would-block results with cancellation checks and a
hard retry budget, and reject zero-progress writes. This avoids both data loss
and an unbounded busy retry if an unexpected descriptor/backend violates the
regular-file assumption.

On macOS there is no `openat2(RESOLVE_BENEATH)` equivalent. Component-at-a-time
`openat` plus `O_NOFOLLOW` and descriptor type checks provide the deny-symlink
policy. `unlinkat` and `renameat` operate on the final directory entry rather
than following it; a concurrent replacement therefore cannot redirect those
operations outside the opened root. Errors exposed to untrusted callers are
mapped to stable categories rather than leaking a canonical path or descriptor.

Permission import uses the same open policy, then compares a fixed-width
device/file identity from the newly opened directory with identity protected by
the permission envelope. A match is required before a fresh runtime-local
resource is inserted. This trusted identity API is not registered in the JS
bootstrap and is never a raw descriptor.
