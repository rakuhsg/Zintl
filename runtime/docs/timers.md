# Monotonic timers

`TimerQueue` is bounded by runtime configuration and stores only request IDs,
absolute monotonic ticks, opaque generational timer identities, and an insertion
sequence. A `BTreeSet` provides deadline order and FIFO order for identical
deadlines while allowing cancellation to remove storage immediately; cancelled
timers cannot accumulate stale heap entries.

The queue accepts no duplicate or zero request identity. Pop operations require
a non-zero item budget. Cancellation advances the slot generation, elapsed and
cancelled timers are removed exactly once, and shutdown returns every remaining
request in deterministic deadline/FIFO order before permanently rejecting new
timers. Core tests supply explicit fake ticks, so ordering and boundary behavior
do not depend on wall time or test sleeps.

The C ABI submits and fires timers using one caller-provided monotonic clock
origin. `RuntimeJSC` uses `DispatchTime.uptimeNanoseconds` and a bounded number of
cooperative Swift sleep tasks only as wake signals. Rust remains the source of
truth: on wake it decides which timers are due and transactionally enqueues
their normal completion envelopes. The notifier schedules bounded JSC drain
turns and never calls JavaScriptCore from the wake task.

The frozen JS API exposes `Zintl.sleep(milliseconds)`. Delays must be safe,
non-negative integer milliseconds no greater than 24 hours. Shutdown cancels
the wake task and the Rust timer request; late wakeups and completions cannot
settle the Promise again.
