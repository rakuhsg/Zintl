# Builtin operation catalog

The implementation namespace and stable IDs are frozen as follows. Every entry
uses operation version 1 and schema version 1.

| Stable ID | Name | Execution |
| ---: | --- | --- |
| 2 | `zintl.builtin.timer.sleep` | runtime |
| 16 | `zintl.builtin.fs.request-directory` | host executor |
| 17 | `zintl.builtin.fs.read-file` | filesystem worker |
| 18 | `zintl.builtin.fs.write-file` | filesystem worker |
| 19 | `zintl.builtin.fs.create-directory` | filesystem worker |
| 20 | `zintl.builtin.fs.list-directory` | filesystem worker |
| 21 | `zintl.builtin.fs.metadata` | filesystem worker |
| 22 | `zintl.builtin.fs.remove-file` | filesystem worker |
| 23 | `zintl.builtin.fs.remove-directory` | filesystem worker |
| 24 | `zintl.builtin.fs.rename` | filesystem worker |
| 25 | `zintl.builtin.fs.open-relative` | filesystem worker |
| 32 | `zintl.builtin.resource.close` | runtime |

`runtime-ops::builtin::register_all` installs this entire catalog before freeze.
Each descriptor has a mandatory permission kind, input/output byte limits,
execution class, and monotonic timeout. Runtime and worker handlers may complete
asynchronously, but they cannot replace descriptor policy.

Builtin and custom registrations share the same immutable `OpRegistry` and
`prepare_dispatch` path. Lookup/version, input quota, authorization, execution
selection, output quota, cancellation, timeout, and completion therefore have
the same error and ordering contracts. The implementation prefix cannot be
registered through the custom API or dispatched as an unknown host callback.

Catalog snapshot tests make ID/name changes explicit. Such a change requires a
new operation or schema version rather than silently reinterpreting an existing
ID.
