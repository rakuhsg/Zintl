# Runtime state, scheduling, and reactor

The M3 core state is `Configured → Running → ShuttingDown → Terminated`.
Submissions require `Running`; shutdown rejects new work and queues one terminal
completion for each pending request. A request ID remains live until its
completion is drained, preventing reuse against an unsettled Promise. Complete,
cancel, timeout, and shutdown remove pending state only after the bounded
completion enqueue succeeds.

Shutdown completion order is sorted by request identity rather than depending
on randomized map iteration. Real concurrent C ABI tests verify cancel versus
complete has one winner, and notifier reentrancy verifies no core lock crosses
an embedder callback.

Completion polling is always non-blocking. The FFI reports empty or required
caller-buffer size without consuming data. The configured in-flight bound counts
pending and undrained completions together, so queue growth and total completion
bytes remain bounded. Notifiers run after core locks are released and may only
schedule a bounded drain on the host JS executor.

`KqueueReactor` owns its queue and attached sources through `OwnedFd`. The
backend uses level-triggered semantic read/write interests and maps kernel EOF
and error flags to portable flags. `udata` is a packed slot/generation value,
never a pointer. `KqueueDriver` moves the reactor to a named dedicated thread;
blocking `kevent` is therefore absent from caller, JS, and main executors.
Commands and delivered events are bounded, command submission wakes the poll,
and readiness backpressure relies on level-triggered redelivery.

`WorkerPool` has a fixed thread count and bounded job/completion queues. Jobs
receive a cancellation token and owned bytes only. Completion notification runs
after releasing the pool lock. Shutdown cancels queued work, wakes waiters, and
joins workers on the explicitly non-main shutdown executor.

Both completion count and completion payload bytes are configured. A worker
result that exceeds its byte bound is replaced with `QuotaExceeded` before it is
queued. Settled host callbacks continue consuming tracker capacity until their
terminal result is drained, preventing an undrained-result memory bypass.

`TimerQueue` uses absolute monotonic ticks, a bounded ordered set, generational
slots, and deterministic insertion sequence. Due timers are popped with an item
budget and enter the same runtime completion queue as every other request.
Cancellation and shutdown remove timer storage immediately, so stale heap nodes
cannot accumulate and a late wake cannot produce a second completion.
