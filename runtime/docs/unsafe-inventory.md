# Unsafe inventory

Unsafe code is denied except in the two platform/FFI boundary crates below.

## reactor-kqueue

Unsafe blocks wrap `kqueue`, `kevent`, `fcntl`, `dup` and transfer successful
descriptor results into `OwnedFd`. Each block documents descriptor and pointer
validity. Native descriptors never leave the internal backend through the
embedding API.

## runtime-jsc

Unsafe blocks call the statically linked Swift C ABI and dereference the
notifier user-data pointer while its owning box is live. The opaque engine
pointer stays private, is uniquely freed after synchronous Swift shutdown, and
is never exposed to JavaScript or public Rust callers.
