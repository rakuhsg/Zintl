# ADR-0005: Filesystem path policy

Status: Accepted for implementation in M5

Directory capability scope is enforced by descriptor-relative component walking.
Filesystem ops accept relative paths only; NUL, empty components, absolute
syntax, parent traversal, platform separators, ambiguous normalization, and
oversize input are rejected. Symlinks are denied during traversal. Permission
request locators are untrusted data and are opened only after approval. Any
platform primitive limitation that weakens this policy requires a new ADR and
user confirmation before implementation.

