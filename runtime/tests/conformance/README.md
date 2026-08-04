# Conformance tests

Engine-neutral tests use a fake `JavaScriptEngineBackend`; the macOS backend
suite additionally evaluates Promise-based JavaScript through the statically
linked Swift/JSC adapter. Reactor conformance is generic over `Reactor`, with
real pipe/socketpair cases for kqueue.
