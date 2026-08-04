# ADR-0003: Versioned payload codec

Status: Accepted

The initial protocol uses a small audited framing codec: four-byte magic,
big-endian fixed-width version/kind/IDs/status, a checked `u32` payload length,
and owned payload bytes. The fixed envelope has an explicit reserved field;
unknown non-zero fields and trailing bytes fail closed. Op-specific payloads are
versioned by their descriptors and validate text as strict UTF-8 where declared.

This was selected over a schema dependency for the initial narrow ABI because it
has no build script or native/network feature surface and can be implemented in
both Rust and Swift directly. Decoders check the configured cap and exact length
before copying. Unchecked casts or layout memcpy are prohibited. A richer schema
may be adopted only with a versioned envelope and compatibility ADR.
