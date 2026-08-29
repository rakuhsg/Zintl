use crate::view::Context;

/// An event family supplied by a UI implementation.
pub trait Event: Clone + 'static {
    type Kind: Copy + Eq + 'static;

    /// Returns the kind used to select an element's handler.
    fn kind(&self) -> Self::Kind;
}

/// Identifies the event handlers owned by one mounted [`crate::element::Element`].
///
/// The identifier is opaque and generational. A route becomes invalid when its
/// element is unmounted, even if its storage slot is later reused.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EventRouteId {
    slot: u32,
    generation: u32,
}

impl EventRouteId {
    /// Encodes this route for transport through a platform backend.
    pub fn into_raw(self) -> u64 {
        u64::from(self.generation) << 32 | u64::from(self.slot)
    }

    /// Decodes a route previously produced by [`Self::into_raw`].
    ///
    /// Decoding does not make a stale route valid; dispatch still checks its
    /// generation against the composer's live route table.
    pub fn from_raw(raw: u64) -> Self {
        Self {
            slot: raw as u32,
            generation: (raw >> 32) as u32,
        }
    }
}

type EventHandler<E> = Box<dyn for<'a> FnMut(&mut Context<'a>, E)>;

#[doc(hidden)]
pub struct EventHandlers<E: Event> {
    handlers: Vec<(E::Kind, EventHandler<E>)>,
}

impl<E: Event> EventHandlers<E> {
    pub(crate) fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.handlers.is_empty()
    }

    pub(crate) fn insert(&mut self, kind: E::Kind, handler: EventHandler<E>) {
        if let Some((_, current)) = self
            .handlers
            .iter_mut()
            .find(|(current, _)| *current == kind)
        {
            *current = handler;
        } else {
            self.handlers.push((kind, handler));
        }
    }

    fn dispatch(&mut self, cx: &mut Context<'_>, event: E) -> bool {
        let kind = event.kind();
        let Some((_, handler)) = self
            .handlers
            .iter_mut()
            .find(|(current, _)| *current == kind)
        else {
            return false;
        };
        handler(cx, event);
        true
    }
}

struct EventRouteSlot<E: Event> {
    generation: u32,
    handlers: Option<EventHandlers<E>>,
}

pub(crate) struct EventRouter<E: Event> {
    slots: Vec<EventRouteSlot<E>>,
    free: Vec<u32>,
}

impl<E: Event> EventRouter<E> {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    pub(crate) fn insert(&mut self, handlers: EventHandlers<E>) -> EventRouteId {
        debug_assert!(!handlers.is_empty());
        if let Some(slot) = self.free.pop() {
            let entry = &mut self.slots[slot as usize];
            debug_assert!(entry.handlers.is_none());
            entry.handlers = Some(handlers);
            return EventRouteId {
                slot,
                generation: entry.generation,
            };
        }

        let slot = self.slots.len() as u32;
        self.slots.push(EventRouteSlot {
            generation: 0,
            handlers: Some(handlers),
        });
        EventRouteId {
            slot,
            generation: 0,
        }
    }

    pub(crate) fn replace(&mut self, id: EventRouteId, handlers: EventHandlers<E>) -> bool {
        let Some(slot) = self.slots.get_mut(id.slot as usize) else {
            return false;
        };
        if slot.generation != id.generation || slot.handlers.is_none() {
            return false;
        }
        slot.handlers = Some(handlers);
        true
    }

    pub(crate) fn remove(&mut self, id: EventRouteId) -> bool {
        let Some(slot) = self.slots.get_mut(id.slot as usize) else {
            return false;
        };
        if slot.generation != id.generation || slot.handlers.take().is_none() {
            return false;
        }
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.slot);
        true
    }

    pub(crate) fn dispatch(&mut self, id: EventRouteId, cx: &mut Context<'_>, event: E) -> bool {
        let Some(slot) = self.slots.get_mut(id.slot as usize) else {
            return false;
        };
        if slot.generation != id.generation {
            return false;
        }
        slot.handlers
            .as_mut()
            .is_some_and(|handlers| handlers.dispatch(cx, event))
    }
}
