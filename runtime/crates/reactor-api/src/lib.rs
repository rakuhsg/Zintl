//! Backend-neutral reactor contracts.

#![forbid(unsafe_code)]

use std::time::Instant;

/// Opaque core-owned source identity. This is not an OS handle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceRef(u64);

impl SourceRef {
    /// Creates a source identity allocated by runtime core.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the core-owned identity for backend tokenization. It is not an OS handle.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Portable readiness requested by runtime core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Interest {
    pub readable: bool,
    pub writable: bool,
}

/// Portable event returned by a reactor backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ReactorEvent {
    pub source: SourceRef,
    pub flags: EventFlags,
}

/// Portable semantic event flags. Values are not OS event constants.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventFlags(u8);

impl EventFlags {
    pub const READABLE: Self = Self(1 << 0);
    pub const WRITABLE: Self = Self(1 << 1);
    pub const ERROR: Self = Self(1 << 2);
    pub const HANGUP: Self = Self(1 << 3);

    #[must_use]
    pub const fn empty() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns whether every flag in `other` is present.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// Backend-neutral reactor failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReactorError {
    InvalidRegistration,
    Unsupported,
    Backend,
}

/// Readiness reactor contract. `poll` is called only by a dedicated reactor thread.
pub trait Reactor: Send + 'static {
    type Registration: Send;

    /// Registers a source and interest.
    ///
    /// # Errors
    ///
    /// Returns an error when the source or interest is invalid or the backend
    /// cannot install a registration.
    fn register(
        &mut self,
        source: SourceRef,
        interest: Interest,
    ) -> Result<Self::Registration, ReactorError>;

    /// Changes interest for a live registration.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale registration or backend failure.
    fn reregister(
        &mut self,
        registration: &Self::Registration,
        interest: Interest,
    ) -> Result<(), ReactorError>;

    /// Removes a live registration.
    ///
    /// # Errors
    ///
    /// Returns an error for a stale registration or backend failure.
    fn deregister(&mut self, registration: Self::Registration) -> Result<(), ReactorError>;

    /// Polls into `output` until readiness, wakeup, or the optional deadline.
    ///
    /// # Errors
    ///
    /// Returns a sanitized backend error. Implementations handle interruption
    /// according to the reactor contract rather than exposing an OS errno.
    fn poll(
        &mut self,
        deadline: Option<Instant>,
        output: &mut Vec<ReactorEvent>,
    ) -> Result<(), ReactorError>;

    /// Wakes a pending poll. Wakeups may be coalesced.
    ///
    /// # Errors
    ///
    /// Returns an error when the backend wake mechanism is unavailable.
    fn wake(&self) -> Result<(), ReactorError>;
}

/// Reusable semantic conformance case for readiness backends. Platform test
/// fixtures attach `source` and make it readable in `trigger` before polling.
///
/// # Errors
///
/// Returns a stable contract error for backend failures or missing readiness.
pub fn run_readable_conformance<R: Reactor>(
    reactor: &mut R,
    source: SourceRef,
    trigger: impl FnOnce() -> Result<(), ReactorError>,
    deadline: Instant,
) -> Result<(), ReactorConformanceError> {
    let registration = reactor
        .register(
            source,
            Interest {
                readable: true,
                writable: false,
            },
        )
        .map_err(ReactorConformanceError::Backend)?;
    trigger().map_err(ReactorConformanceError::Backend)?;
    let mut events = Vec::new();
    reactor
        .poll(Some(deadline), &mut events)
        .map_err(ReactorConformanceError::Backend)?;
    let observed = events
        .iter()
        .any(|event| event.source == source && event.flags.contains(EventFlags::READABLE));
    reactor
        .deregister(registration)
        .map_err(ReactorConformanceError::Backend)?;
    if observed {
        Ok(())
    } else {
        Err(ReactorConformanceError::MissingReadableEvent)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReactorConformanceError {
    Backend(ReactorError),
    MissingReadableEvent,
}
