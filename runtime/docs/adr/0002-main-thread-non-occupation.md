# ADR-0002: Main-thread non-occupation

Status: Accepted

The public embedding API has no blocking poll or permanent run loop. Reactor
polling is confined to its dedicated thread. Completion notification schedules a
bounded drain on the host-selected JS executor, which need not be the main
thread. UI code may schedule only dialogs and view updates on MainActor.

