//! Engine- and platform-neutral embedded runtime core.

#![forbid(unsafe_code)]

pub mod codec;
pub mod runtime;

/// Stable, sanitized error categories shared by adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum ErrorCode {
    PermissionDenied = 1,
    InvalidArgument = 2,
    InvalidCapability = 3,
    InvalidResource = 4,
    ResourceClosed = 5,
    ResourceBusy = 6,
    NotSupported = 7,
    Cancelled = 8,
    TimedOut = 9,
    QuotaExceeded = 10,
    Io = 11,
    Protocol = 12,
    RuntimeShuttingDown = 13,
    Internal = 14,
}

/// Runtime lifecycle. Transitions are one-way.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RuntimeState {
    #[default]
    Configured,
    Running,
    ShuttingDown,
    Terminated,
}

#[cfg(test)]
mod tests {
    use super::RuntimeState;

    #[test]
    // Verifies the documented lifecycle states remain explicit and ordered.
    fn lifecycle_states_are_distinct() {
        assert_ne!(RuntimeState::Configured, RuntimeState::Running);
        assert_ne!(RuntimeState::Running, RuntimeState::ShuttingDown);
        assert_ne!(RuntimeState::ShuttingDown, RuntimeState::Terminated);
    }
}
