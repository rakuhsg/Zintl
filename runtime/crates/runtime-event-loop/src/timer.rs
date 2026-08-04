//! Bounded deterministic monotonic timer scheduling.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::time::Instant;

/// Opaque generational identity for one scheduled timer.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct TimerHandle {
    slot: u32,
    generation: u32,
}

impl fmt::Debug for TimerHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TimerHandle(<opaque>)")
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct TimerKey {
    deadline_tick: u64,
    sequence: u64,
    slot: u32,
    generation: u32,
}

#[derive(Clone, Copy, Debug)]
struct LiveTimer {
    request_id: u64,
    key: TimerKey,
}

#[derive(Clone, Copy, Debug)]
enum SlotState {
    Vacant,
    Live(LiveTimer),
    Retired,
}

#[derive(Clone, Copy, Debug)]
struct TimerSlot {
    generation: u32,
    state: SlotState,
}

/// Stable event produced when a deadline becomes due.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TimerFired {
    pub request_id: u64,
    pub deadline_tick: u64,
}

/// A bounded one-shot timer queue driven exclusively by monotonic ticks.
pub struct TimerQueue {
    max_timers: usize,
    live_count: usize,
    next_sequence: u64,
    slots: Vec<TimerSlot>,
    ordered: BTreeSet<TimerKey>,
    by_request: HashMap<u64, TimerHandle>,
    shutting_down: bool,
}

impl TimerQueue {
    /// Creates an empty queue.
    ///
    /// # Errors
    ///
    /// Rejects zero capacity or capacity beyond opaque slot indexing.
    pub fn new(max_timers: usize) -> Result<Self, TimerError> {
        if max_timers == 0 || max_timers > u32::MAX as usize {
            return Err(TimerError::QuotaExceeded);
        }
        Ok(Self {
            max_timers,
            live_count: 0,
            next_sequence: 1,
            slots: Vec::new(),
            ordered: BTreeSet::new(),
            by_request: HashMap::new(),
            shutting_down: false,
        })
    }

    /// Schedules one request at an absolute monotonic tick.
    ///
    /// # Errors
    ///
    /// Rejects zero/duplicate request IDs, capacity or identity exhaustion,
    /// and scheduling after shutdown.
    pub fn schedule_at(
        &mut self,
        request_id: u64,
        deadline_tick: u64,
    ) -> Result<TimerHandle, TimerError> {
        if self.shutting_down {
            return Err(TimerError::ShuttingDown);
        }
        if request_id == 0 {
            return Err(TimerError::InvalidRequest);
        }
        if self.by_request.contains_key(&request_id) {
            return Err(TimerError::DuplicateRequest);
        }
        if self.live_count >= self.max_timers {
            return Err(TimerError::QuotaExceeded);
        }
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(TimerError::IdentifierExhausted)?;
        let vacant = self
            .slots
            .iter()
            .position(|slot| matches!(slot.state, SlotState::Vacant));
        let slot_index = if let Some(index) = vacant {
            index
        } else {
            if self.slots.len() >= self.max_timers {
                return Err(TimerError::QuotaExceeded);
            }
            self.slots.push(TimerSlot {
                generation: 1,
                state: SlotState::Vacant,
            });
            self.slots.len() - 1
        };
        let slot_number = u32::try_from(slot_index).map_err(|_| TimerError::IdentifierExhausted)?;
        let slot = &mut self.slots[slot_index];
        let handle = TimerHandle {
            slot: slot_number,
            generation: slot.generation,
        };
        let key = TimerKey {
            deadline_tick,
            sequence,
            slot: slot_number,
            generation: slot.generation,
        };
        slot.state = SlotState::Live(LiveTimer { request_id, key });
        let inserted_order = self.ordered.insert(key);
        let previous_request = self.by_request.insert(request_id, handle);
        debug_assert!(inserted_order && previous_request.is_none());
        self.live_count += 1;
        Ok(handle)
    }

    /// Cancels a live timer exactly once.
    ///
    /// # Errors
    ///
    /// Rejects stale, already-fired, cancelled, or forged handles.
    pub fn cancel(&mut self, handle: TimerHandle) -> Result<u64, TimerError> {
        let timer = self.remove_live(handle)?;
        Ok(timer.request_id)
    }

    /// Cancels a request without exposing its timer identity.
    ///
    /// # Errors
    ///
    /// Rejects an unknown or already-settled request.
    pub fn cancel_request(&mut self, request_id: u64) -> Result<(), TimerError> {
        let handle = *self
            .by_request
            .get(&request_id)
            .ok_or(TimerError::InvalidTimer)?;
        self.cancel(handle).map(|_| ())
    }

    /// Removes at most `maximum` due timers in deterministic deadline/FIFO order.
    ///
    /// # Errors
    ///
    /// Rejects a zero item budget.
    pub fn pop_due(
        &mut self,
        now_tick: u64,
        maximum: usize,
    ) -> Result<Vec<TimerFired>, TimerError> {
        if maximum == 0 {
            return Err(TimerError::InvalidBudget);
        }
        let mut fired = Vec::with_capacity(maximum.min(self.live_count));
        while fired.len() < maximum {
            let Some(key) = self.ordered.first().copied() else {
                break;
            };
            if key.deadline_tick > now_tick {
                break;
            }
            let handle = TimerHandle {
                slot: key.slot,
                generation: key.generation,
            };
            let timer = self.remove_live(handle)?;
            fired.push(TimerFired {
                request_id: timer.request_id,
                deadline_tick: key.deadline_tick,
            });
        }
        Ok(fired)
    }

    /// Returns the next absolute monotonic deadline without blocking.
    #[must_use]
    pub fn next_deadline(&self) -> Option<u64> {
        self.ordered.first().map(|key| key.deadline_tick)
    }

    /// Rejects future scheduling and returns every pending request exactly once
    /// in deterministic deadline/FIFO order.
    pub fn shutdown(&mut self) -> Vec<u64> {
        self.shutting_down = true;
        let keys = std::mem::take(&mut self.ordered);
        let mut cancelled = Vec::with_capacity(keys.len());
        for key in keys {
            let handle = TimerHandle {
                slot: key.slot,
                generation: key.generation,
            };
            if let Ok(timer) = self.remove_live_without_order(handle) {
                cancelled.push(timer.request_id);
            }
        }
        cancelled
    }

    #[must_use]
    pub const fn live_count(&self) -> usize {
        self.live_count
    }

    fn remove_live(&mut self, handle: TimerHandle) -> Result<LiveTimer, TimerError> {
        let timer = self.remove_live_without_order(handle)?;
        let removed = self.ordered.remove(&timer.key);
        debug_assert!(removed);
        Ok(timer)
    }

    fn remove_live_without_order(&mut self, handle: TimerHandle) -> Result<LiveTimer, TimerError> {
        let slot = self
            .slots
            .get_mut(handle.slot as usize)
            .ok_or(TimerError::InvalidTimer)?;
        if slot.generation != handle.generation {
            return Err(TimerError::InvalidTimer);
        }
        let SlotState::Live(timer) = slot.state else {
            return Err(TimerError::InvalidTimer);
        };
        slot.state = SlotState::Retired;
        match slot.generation.checked_add(1) {
            Some(next) => {
                slot.generation = next;
                slot.state = SlotState::Vacant;
            }
            None => slot.state = SlotState::Retired,
        }
        self.by_request.remove(&timer.request_id);
        self.live_count -= 1;
        Ok(timer)
    }
}

/// Process-local monotonic clock origin for production reactor integration.
pub struct MonotonicClock {
    origin: Instant,
}

impl MonotonicClock {
    #[must_use]
    pub fn new() -> Self {
        Self {
            origin: Instant::now(),
        }
    }

    /// Nanoseconds since this clock was created, saturating at `u64::MAX`.
    #[must_use]
    pub fn now_tick(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TimerError {
    InvalidRequest,
    DuplicateRequest,
    InvalidTimer,
    InvalidBudget,
    QuotaExceeded,
    IdentifierExhausted,
    ShuttingDown,
}

#[cfg(test)]
mod tests {
    use super::{TimerError, TimerQueue};

    #[test]
    // Verifies equal deadlines fire in insertion order and earlier deadlines win globally.
    fn deadlines_have_deterministic_order() {
        let mut timers = TimerQueue::new(4).expect("queue");
        timers.schedule_at(1, 20).expect("timer one");
        timers.schedule_at(2, 10).expect("timer two");
        timers.schedule_at(3, 20).expect("timer three");
        assert_eq!(timers.next_deadline(), Some(10));
        assert_eq!(
            timers
                .pop_due(20, 4)
                .expect("due")
                .iter()
                .map(|event| event.request_id)
                .collect::<Vec<_>>(),
            vec![2, 1, 3]
        );
    }

    #[test]
    // Verifies a drain item budget yields while preserving the next deadline.
    fn due_drain_is_bounded() {
        let mut timers = TimerQueue::new(3).expect("queue");
        timers.schedule_at(1, 1).expect("first");
        timers.schedule_at(2, 1).expect("second");
        timers.schedule_at(3, 1).expect("third");
        assert_eq!(timers.pop_due(1, 2).expect("first drain").len(), 2);
        assert_eq!(timers.live_count(), 1);
        assert_eq!(timers.next_deadline(), Some(1));
        assert_eq!(timers.pop_due(1, 2).expect("second drain").len(), 1);
    }

    #[test]
    // Verifies cancellation is exactly once and slot reuse rejects stale identities.
    fn cancellation_rejects_stale_timer() {
        let mut timers = TimerQueue::new(1).expect("queue");
        let stale = timers.schedule_at(1, 10).expect("timer");
        assert_eq!(timers.cancel(stale), Ok(1));
        let current = timers.schedule_at(2, 20).expect("reused slot");
        assert_eq!(timers.cancel(stale), Err(TimerError::InvalidTimer));
        assert_eq!(timers.cancel(current), Ok(2));
    }

    #[test]
    // Verifies duplicate requests and capacity exhaustion fail before allocation grows.
    fn request_and_capacity_limits_are_enforced() {
        let mut timers = TimerQueue::new(1).expect("queue");
        timers.schedule_at(1, 10).expect("first");
        assert_eq!(timers.schedule_at(1, 20), Err(TimerError::DuplicateRequest));
        assert_eq!(timers.schedule_at(2, 20), Err(TimerError::QuotaExceeded));
        assert_eq!(timers.pop_due(10, 0), Err(TimerError::InvalidBudget));
    }

    #[test]
    // Verifies shutdown settles every request once and permanently rejects new timers.
    fn shutdown_is_deterministic_and_terminal() {
        let mut timers = TimerQueue::new(3).expect("queue");
        timers.schedule_at(1, 20).expect("first");
        timers.schedule_at(2, 10).expect("second");
        timers.schedule_at(3, 20).expect("third");
        assert_eq!(timers.shutdown(), vec![2, 1, 3]);
        assert!(timers.shutdown().is_empty());
        assert_eq!(timers.live_count(), 0);
        assert_eq!(timers.schedule_at(4, 30), Err(TimerError::ShuttingDown));
    }
}
