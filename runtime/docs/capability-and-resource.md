# Capability and resource contracts

`CapabilityStore` is runtime-local, bounded, and empty by default. Root
capabilities can be minted only from `ValidatedGrant`; ordinary request data and
permission decisions are not handles. Derivation checks scope containment,
rights subset, non-increasing quota, and non-extending expiry. Capability IDs are
never reused, and revocation or expiry in any ancestor invalidates descendants.

`ResourceTable` is runtime-local and bounded. Its opaque handle contains only a
runtime owner, slot, and generation—never an OS descriptor. Every operation
checks owner, slot, generation, live state, semantic kind, and required rights
before invoking resource code. Close advances the generation, cleanup runs once,
and wraparound retires a slot. Dropping the table closes every live resource.

Both crates forbid unsafe code and have no JSC, Swift, reactor, or OS backend
dependency. Opened filesystem objects remain behind the resource trait;
filesystem methods do not become generic resource-table methods.
