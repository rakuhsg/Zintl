//! Runtime-scoped generational virtual resource table.

#![forbid(unsafe_code)]

use std::any::Any;
use std::fmt;

/// Identity of the runtime that owns a resource table.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ResourceOwner(u64);

impl ResourceOwner {
    /// Constructs an owner identity from a core-generated non-zero value.
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }
}

/// Stable semantic resource kind, independent of an OS object type.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ResourceKind(String);

impl ResourceKind {
    /// Creates a non-empty namespaced resource kind.
    ///
    /// # Errors
    ///
    /// Rejects empty, overlong, or non-namespaced values.
    pub fn new(value: impl Into<String>) -> Result<Self, ResourceError> {
        let value = value.into();
        if value.is_empty() || value.len() > 128 || !value.contains('.') {
            return Err(ResourceError::InvalidKind);
        }
        Ok(Self(value))
    }
}

/// Resource-local rights. The table checks these before returning a resource.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceRights(u64);

impl ResourceRights {
    /// Creates a requested rights bitset. Authority exists only in a live slot.
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    #[must_use]
    pub const fn contains(self, requested: Self) -> bool {
        self.0 & requested.0 == requested.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Opaque transport representation. No field is an OS descriptor.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
pub struct ResourceHandle {
    owner: ResourceOwner,
    slot: u32,
    generation: u32,
}

impl fmt::Debug for ResourceHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ResourceHandle(<opaque>)")
    }
}

/// Minimal common resource behavior. Typed ops provide read/write semantics.
pub trait Resource: Any + Send + 'static {
    fn close(&mut self);
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

enum SlotState {
    Vacant,
    Live {
        kind: ResourceKind,
        rights: ResourceRights,
        resource: Box<dyn Resource>,
    },
    Retired,
}

struct Slot {
    generation: u32,
    state: SlotState,
}

/// A bounded resource table scoped to exactly one runtime owner.
pub struct ResourceTable {
    owner: ResourceOwner,
    max_resources: usize,
    live_count: usize,
    slots: Vec<Slot>,
}

impl ResourceTable {
    /// Creates an empty resource table.
    ///
    /// # Errors
    ///
    /// Rejects a zero resource limit or a limit larger than `u32` indexing.
    pub fn new(owner: ResourceOwner, max_resources: usize) -> Result<Self, ResourceError> {
        if max_resources == 0 || max_resources > u32::MAX as usize {
            return Err(ResourceError::QuotaExceeded);
        }
        Ok(Self {
            owner,
            max_resources,
            live_count: 0,
            slots: Vec::new(),
        })
    }

    /// Inserts a resource with explicit kind and non-empty rights.
    ///
    /// # Errors
    ///
    /// Rejects empty rights, table exhaustion, index exhaustion, or generation
    /// exhaustion. Ownership of a rejected resource remains with the caller.
    pub fn insert<R: Resource>(
        &mut self,
        kind: ResourceKind,
        rights: ResourceRights,
        resource: R,
    ) -> Result<ResourceHandle, ResourceError> {
        if rights.is_empty() {
            return Err(ResourceError::InvalidRights);
        }
        if self.live_count >= self.max_resources {
            return Err(ResourceError::QuotaExceeded);
        }

        let vacant = self
            .slots
            .iter()
            .position(|slot| matches!(slot.state, SlotState::Vacant));
        let slot_index = if let Some(index) = vacant {
            index
        } else {
            if self.slots.len() >= self.max_resources {
                return Err(ResourceError::QuotaExceeded);
            }
            self.slots.push(Slot {
                generation: 1,
                state: SlotState::Vacant,
            });
            self.slots.len() - 1
        };

        let slot = &mut self.slots[slot_index];
        slot.state = SlotState::Live {
            kind,
            rights,
            resource: Box::new(resource),
        };
        self.live_count += 1;
        Ok(ResourceHandle {
            owner: self.owner,
            slot: u32::try_from(slot_index).map_err(|_| ResourceError::IdentifierExhausted)?,
            generation: slot.generation,
        })
    }

    /// Runs a typed operation after owner, generation, state, kind, and rights checks.
    ///
    /// # Errors
    ///
    /// Rejects forged, cross-runtime, stale, closed, wrong-kind, or
    /// insufficient-rights handles before invoking the operation.
    pub fn with_resource<T>(
        &mut self,
        handle: ResourceHandle,
        expected_kind: &ResourceKind,
        requested_rights: ResourceRights,
        operation: impl FnOnce(&mut dyn Resource) -> T,
    ) -> Result<T, ResourceError> {
        let slot = self.validated_slot_mut(handle)?;
        let SlotState::Live {
            kind,
            rights,
            resource,
        } = &mut slot.state
        else {
            return Err(ResourceError::ResourceClosed);
        };
        if kind != expected_kind {
            return Err(ResourceError::WrongKind);
        }
        if !rights.contains(requested_rights) {
            return Err(ResourceError::PermissionDenied);
        }
        Ok(operation(resource.as_mut()))
    }

    /// Closes a live resource exactly once and advances its slot generation.
    ///
    /// # Errors
    ///
    /// Rejects forged, cross-runtime, stale, or already-closed handles. A slot
    /// whose generation would wrap is permanently retired.
    pub fn close(&mut self, handle: ResourceHandle) -> Result<(), ResourceError> {
        {
            let slot = self.validated_slot_mut(handle)?;
            let old_state = std::mem::replace(&mut slot.state, SlotState::Retired);
            let SlotState::Live { mut resource, .. } = old_state else {
                return Err(ResourceError::ResourceClosed);
            };
            resource.close();
            match slot.generation.checked_add(1) {
                Some(next) => {
                    slot.generation = next;
                    slot.state = SlotState::Vacant;
                }
                None => slot.state = SlotState::Retired,
            }
        }
        self.live_count -= 1;
        Ok(())
    }

    #[must_use]
    pub const fn live_count(&self) -> usize {
        self.live_count
    }

    fn validated_slot_mut(&mut self, handle: ResourceHandle) -> Result<&mut Slot, ResourceError> {
        if handle.owner != self.owner {
            return Err(ResourceError::WrongRuntime);
        }
        let slot = self
            .slots
            .get_mut(handle.slot as usize)
            .ok_or(ResourceError::InvalidResource)?;
        if slot.generation != handle.generation {
            return Err(ResourceError::InvalidResource);
        }
        match slot.state {
            SlotState::Live { .. } => Ok(slot),
            SlotState::Vacant | SlotState::Retired => Err(ResourceError::ResourceClosed),
        }
    }
}

impl Drop for ResourceTable {
    fn drop(&mut self) {
        for slot in &mut self.slots {
            if let SlotState::Live { resource, .. } = &mut slot.state {
                resource.close();
            }
            slot.state = SlotState::Retired;
        }
        self.live_count = 0;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceError {
    InvalidKind,
    InvalidRights,
    InvalidResource,
    WrongRuntime,
    WrongKind,
    PermissionDenied,
    ResourceClosed,
    QuotaExceeded,
    IdentifierExhausted,
}

#[cfg(test)]
mod tests {
    use super::{
        Resource, ResourceError, ResourceKind, ResourceOwner, ResourceRights, ResourceTable,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakeResource {
        close_count: Arc<AtomicUsize>,
    }

    impl Resource for FakeResource {
        fn close(&mut self) {
            self.close_count.fetch_add(1, Ordering::SeqCst);
        }

        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    }

    fn kind(value: &str) -> ResourceKind {
        ResourceKind::new(value).expect("valid kind")
    }

    fn table(owner: u64, max: usize) -> ResourceTable {
        ResourceTable::new(ResourceOwner::new(owner).expect("non-zero owner"), max)
            .expect("valid table")
    }

    fn insert(table: &mut ResourceTable, closes: &Arc<AtomicUsize>) -> super::ResourceHandle {
        table
            .insert(
                kind("fs.file"),
                ResourceRights::from_bits(0b01),
                FakeResource {
                    close_count: closes.clone(),
                },
            )
            .expect("inserted")
    }

    #[test]
    // Verifies a handle minted by one runtime is rejected by another.
    fn rejects_cross_runtime_handle() {
        let closes = Arc::new(AtomicUsize::new(0));
        let mut first = table(1, 1);
        let handle = insert(&mut first, &closes);
        let mut second = table(2, 1);
        assert_eq!(second.close(handle), Err(ResourceError::WrongRuntime));
    }

    #[test]
    // Verifies a closed handle remains stale after its slot is reused.
    fn rejects_stale_handle_after_slot_reuse() {
        let closes = Arc::new(AtomicUsize::new(0));
        let mut resources = table(1, 1);
        let stale = insert(&mut resources, &closes);
        resources.close(stale).expect("closed");
        let current = insert(&mut resources, &closes);
        assert_ne!(stale, current);
        assert_eq!(resources.close(stale), Err(ResourceError::InvalidResource));
        resources.close(current).expect("current closed");
    }

    #[test]
    // Verifies explicit close runs resource cleanup exactly once.
    fn double_close_is_rejected_without_double_cleanup() {
        let closes = Arc::new(AtomicUsize::new(0));
        let mut resources = table(1, 1);
        let handle = insert(&mut resources, &closes);
        resources.close(handle).expect("closed");
        assert_eq!(resources.close(handle), Err(ResourceError::InvalidResource));
        assert_eq!(closes.load(Ordering::SeqCst), 1);
    }

    #[test]
    // Verifies kind and rights are checked before a resource operation executes.
    fn checks_kind_and_rights_before_operation() {
        let closes = Arc::new(AtomicUsize::new(0));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut resources = table(1, 1);
        let handle = insert(&mut resources, &closes);
        let result = resources.with_resource(
            handle,
            &kind("fs.directory"),
            ResourceRights::from_bits(0b01),
            |_| calls.fetch_add(1, Ordering::SeqCst),
        );
        assert_eq!(result, Err(ResourceError::WrongKind));
        let result = resources.with_resource(
            handle,
            &kind("fs.file"),
            ResourceRights::from_bits(0b10),
            |_| calls.fetch_add(1, Ordering::SeqCst),
        );
        assert_eq!(result, Err(ResourceError::PermissionDenied));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    // Verifies the configured table limit fails closed without consuming input.
    fn rejects_resource_exhaustion() {
        let closes = Arc::new(AtomicUsize::new(0));
        let mut resources = table(1, 1);
        let _ = insert(&mut resources, &closes);
        assert_eq!(
            resources.insert(
                kind("fs.file"),
                ResourceRights::from_bits(1),
                FakeResource {
                    close_count: closes.clone()
                }
            ),
            Err(ResourceError::QuotaExceeded)
        );
    }

    #[test]
    // Verifies dropping the table closes every still-live resource once.
    fn table_drop_closes_live_resources() {
        let closes = Arc::new(AtomicUsize::new(0));
        {
            let mut resources = table(1, 2);
            let _ = insert(&mut resources, &closes);
            let _ = insert(&mut resources, &closes);
        }
        assert_eq!(closes.load(Ordering::SeqCst), 2);
    }

    #[test]
    // Verifies long generated insert/use/close sequences preserve bounds and exactly-once cleanup.
    fn generated_operation_sequences_preserve_resource_invariants() {
        let closes = Arc::new(AtomicUsize::new(0));
        let mut resources = table(1, 8);
        let mut handles = Vec::new();
        let mut successful_inserts = 0;
        let mut state = 0x9e37_79b9_u64;
        for _ in 0..10_000 {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            match state % 3 {
                0 => {
                    if let Ok(handle) = resources.insert(
                        kind("fs.file"),
                        ResourceRights::from_bits(1),
                        FakeResource {
                            close_count: closes.clone(),
                        },
                    ) {
                        handles.push(handle);
                        successful_inserts += 1;
                    }
                }
                1 => {
                    let index = usize::from(state.to_le_bytes()[0]) % handles.len().max(1);
                    if let Some(handle) = handles.get(index) {
                        let _ = resources.with_resource(
                            *handle,
                            &kind("fs.file"),
                            ResourceRights::from_bits(1),
                            |_| (),
                        );
                    }
                }
                _ => {
                    let index = usize::from(state.to_le_bytes()[0]) % handles.len().max(1);
                    if let Some(handle) = handles.get(index) {
                        let _ = resources.close(*handle);
                    }
                }
            }
            assert!(resources.live_count() <= 8);
            assert!(closes.load(Ordering::SeqCst) <= successful_inserts);
        }
        drop(resources);
        assert_eq!(closes.load(Ordering::SeqCst), successful_inserts);
    }
}
