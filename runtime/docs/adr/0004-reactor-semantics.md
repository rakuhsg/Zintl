# ADR-0004: Reactor semantics

Status: Accepted

The initial reactor is readiness-based, level-triggered, and reports semantic
readable, writable, error, and hangup flags for opaque generational tokens.
Wakeups are coalescible and do not correspond one-to-one with work. Core owns
fairness and rearming policy. Completion-based platforms may implement a future
operation-driver contract instead of emulating readiness.

