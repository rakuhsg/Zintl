# Conformance tests

`reactor_api::run_readable_conformance` is backend-neutral and is executed by
the kqueue fixture with a real pipe. Future readiness backends attach their own
platform source, trigger readability, and call the same function.

`RuntimeJSC.EngineConformanceHarness` accepts only the engine-neutral
`JavaScriptEngineAdapter` protocol. JavaScriptCore runs the shared synchronous
evaluation, Promise/microtask, structured rejection, and no-ambient-authority
cases. Future engine adapters conform to the protocol and run the same report.

Backend-specific lifecycle, stale token, EOF/error, wake, and close-race cases
remain alongside each backend because their fixtures own platform resources.
