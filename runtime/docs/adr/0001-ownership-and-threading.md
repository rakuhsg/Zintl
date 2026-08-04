# ADR-0001: Ownership and threading

Status: Accepted

Swift owns JSC objects on an embedder-provided serial executor. Rust owns policy,
request, completion, and resource state. A dedicated reactor thread may block;
bounded filesystem workers are separate. Cross-thread delivery uses owned bytes,
IDs, a bounded queue, and a notifier. No Rust lock is held while calling an
embedder callback. This prevents cross-thread JSC access and makes ownership and
shutdown auditable.

