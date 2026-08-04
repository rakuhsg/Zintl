//! Engine-neutral permission contracts and capability attenuation.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};

const PERMISSION_ENVELOPE_MAGIC: [u8; 4] = *b"ZPEM";
const PERMISSION_ENVELOPE_VERSION: u16 = 1;
const MAX_PERMISSION_TEXT_BYTES: usize = 255;
const MAX_SCOPE_LOCATOR_BYTES: usize = 64 * 1_024;
const MAX_SCOPE_IDENTITY_BYTES: usize = 1_024;
const MAX_OPENED_ENVELOPE_BYTES: usize = 128 * 1_024;

/// Untrusted request metadata; this value never grants authority by itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionRequest {
    pub request_id: u64,
    pub kind: String,
    pub requested_scope: Vec<u8>,
    pub requested_rights: u64,
}

/// A trusted embedder's proposal. Runtime validation is still required.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Deny,
    Allow {
        scope: Vec<u8>,
        rights: u64,
        quota: u64,
    },
}

/// Asynchronous resolver boundary with explicit invocation and cancellation.
pub trait PermissionResolver: Send + Sync + 'static {
    /// Starts an asynchronous decision without blocking the caller.
    ///
    /// # Errors
    ///
    /// Returns an error if the trusted callback cannot accept the request. A
    /// missing or failed resolver never grants authority.
    fn begin_request(&self, request: PermissionRequest) -> Result<(), PermissionCallbackError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionCallbackError {
    Unavailable,
    Rejected,
}

/// Trusted sealing/opening boundary for opaque exported permissions.
pub trait PermissionCodec: Send + Sync + 'static {
    /// Authenticates and seals an already validated envelope.
    ///
    /// # Errors
    ///
    /// Returns an error when sealing fails; callers must not emit a partial blob.
    fn seal(&self, authenticated_envelope: &[u8]) -> Result<Vec<u8>, PermissionCodecError>;

    /// Authenticates and opens an opaque exported blob.
    ///
    /// # Errors
    ///
    /// Returns an error for invalid, modified, or unsupported input.
    fn open(&self, blob: &[u8]) -> Result<Vec<u8>, PermissionCodecError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionCodecError {
    Invalid,
    Unavailable,
}

/// Trusted, versioned description sealed by an embedder-provided codec.
/// Runtime-local handles, paths, descriptors, and codec keys are never fields.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionEnvelope {
    issuer: String,
    audience: String,
    permission_kind: String,
    rights: u64,
    quota: u64,
    scope_locator: Vec<u8>,
    scope_identity: Vec<u8>,
    expires_at: u64,
    nonce: [u8; 16],
}

impl PermissionEnvelope {
    /// Constructs an exportable description after the grant and persistent
    /// scope locator have been validated by trusted embedding code.
    ///
    /// # Errors
    ///
    /// Rejects authority-free, unbounded, empty, or oversized fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        permission_kind: impl Into<String>,
        rights: u64,
        quota: u64,
        scope_locator: Vec<u8>,
        scope_identity: Vec<u8>,
        expires_at: u64,
        nonce: [u8; 16],
    ) -> Result<Self, PermissionPersistenceError> {
        let envelope = Self {
            issuer: issuer.into(),
            audience: audience.into(),
            permission_kind: permission_kind.into(),
            rights,
            quota,
            scope_locator,
            scope_identity,
            expires_at,
            nonce,
        };
        envelope.validate()?;
        Ok(envelope)
    }

    fn validate(&self) -> Result<(), PermissionPersistenceError> {
        for value in [&self.issuer, &self.audience, &self.permission_kind] {
            if value.is_empty() || value.len() > MAX_PERMISSION_TEXT_BYTES || value.contains('\0') {
                return Err(PermissionPersistenceError::InvalidEnvelope);
            }
        }
        if self.rights == 0
            || self.quota == 0
            || self.expires_at == 0
            || self.nonce == [0; 16]
            || self.scope_locator.is_empty()
            || self.scope_locator.len() > MAX_SCOPE_LOCATOR_BYTES
            || self.scope_identity.is_empty()
            || self.scope_identity.len() > MAX_SCOPE_IDENTITY_BYTES
        {
            return Err(PermissionPersistenceError::InvalidEnvelope);
        }
        Ok(())
    }
}

/// Trusted resolution result for one authenticated persistent scope locator.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedPermissionScope {
    pub permission_kind: String,
    pub stable_identity: Vec<u8>,
    pub local_locator: Vec<u8>,
}

/// Embedder boundary that resolves an authenticated opaque locator without
/// giving the permission core ambient filesystem access.
pub trait ScopeLocatorResolver: Send + Sync + 'static {
    /// Resolves a persistent locator and returns its current trusted identity.
    ///
    /// # Errors
    ///
    /// Fails when the locator is unavailable, changed, or has the wrong type.
    fn resolve(
        &self,
        permission_kind: &str,
        scope_locator: &[u8],
    ) -> Result<ResolvedPermissionScope, ScopeResolutionError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeResolutionError {
    Invalid,
    Unavailable,
}

/// Import policy supplied by the receiving application/runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionImportPolicy {
    issuer: String,
    audience: String,
    permission_kind: String,
    requested_rights: u64,
    requested_quota: u64,
    now: u64,
}

impl PermissionImportPolicy {
    /// Creates an exact issuer/audience/kind policy and an attenuated request.
    ///
    /// # Errors
    ///
    /// Rejects empty identity, rights, or quota fields.
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        permission_kind: impl Into<String>,
        requested_rights: u64,
        requested_quota: u64,
        now: u64,
    ) -> Result<Self, PermissionPersistenceError> {
        let policy = Self {
            issuer: issuer.into(),
            audience: audience.into(),
            permission_kind: permission_kind.into(),
            requested_rights,
            requested_quota,
            now,
        };
        if policy.requested_rights == 0
            || policy.requested_quota == 0
            || [&policy.issuer, &policy.audience, &policy.permission_kind]
                .iter()
                .any(|value| value.is_empty() || value.len() > MAX_PERMISSION_TEXT_BYTES)
        {
            return Err(PermissionPersistenceError::InvalidPolicy);
        }
        Ok(policy)
    }
}

/// Bounded runtime-local replay state. Nonces are consumed only after every
/// authentication, policy, attenuation, and scope-identity check succeeds.
#[derive(Debug)]
pub struct PermissionReplayCache {
    maximum_entries: usize,
    consumed: HashSet<[u8; 16]>,
}

impl PermissionReplayCache {
    /// Creates an empty replay cache with a hard entry limit.
    ///
    /// # Errors
    ///
    /// Rejects a zero limit.
    pub fn new(maximum_entries: usize) -> Result<Self, PermissionPersistenceError> {
        if maximum_entries == 0 {
            return Err(PermissionPersistenceError::ReplayStateFull);
        }
        Ok(Self {
            maximum_entries,
            consumed: HashSet::new(),
        })
    }

    fn consume(&mut self, nonce: [u8; 16]) -> Result<(), PermissionPersistenceError> {
        if self.consumed.contains(&nonce) {
            return Err(PermissionPersistenceError::Replay);
        }
        if self.consumed.len() >= self.maximum_entries {
            return Err(PermissionPersistenceError::ReplayStateFull);
        }
        self.consumed.insert(nonce);
        Ok(())
    }
}

/// Fully authenticated and attenuated import result. The local locator is
/// trusted input for the platform filesystem layer, but is never exposed to JS.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportedPermission {
    pub permission_kind: String,
    pub rights: u64,
    pub quota: u64,
    pub expires_at: u64,
    pub local_locator: Vec<u8>,
    pub stable_identity: Vec<u8>,
}

/// Seals a canonical versioned envelope with the embedder codec.
///
/// # Errors
///
/// Returns no blob if validation or codec sealing fails.
pub fn export_permission(
    codec: &dyn PermissionCodec,
    envelope: &PermissionEnvelope,
) -> Result<Vec<u8>, PermissionPersistenceError> {
    envelope.validate()?;
    codec
        .seal(&encode_permission_envelope(envelope)?)
        .map_err(PermissionPersistenceError::Codec)
}

/// Authenticates, validates, attenuates, resolves, and consumes one permission.
/// No capability or filesystem resource is minted by this function.
///
/// # Errors
///
/// Rejects malformed/modified blobs, policy mismatch, expiry, replay,
/// authority increase, and changed scope identity.
pub fn import_permission(
    codec: &dyn PermissionCodec,
    resolver: &dyn ScopeLocatorResolver,
    replay: &mut PermissionReplayCache,
    blob: &[u8],
    policy: &PermissionImportPolicy,
) -> Result<ImportedPermission, PermissionPersistenceError> {
    let opened = codec
        .open(blob)
        .map_err(PermissionPersistenceError::Codec)?;
    if opened.len() > MAX_OPENED_ENVELOPE_BYTES {
        return Err(PermissionPersistenceError::InvalidEnvelope);
    }
    let envelope = decode_permission_envelope(&opened)?;
    if envelope.issuer != policy.issuer {
        return Err(PermissionPersistenceError::WrongIssuer);
    }
    if envelope.audience != policy.audience {
        return Err(PermissionPersistenceError::WrongAudience);
    }
    if envelope.permission_kind != policy.permission_kind {
        return Err(PermissionPersistenceError::WrongKind);
    }
    if policy.now >= envelope.expires_at {
        return Err(PermissionPersistenceError::Expired);
    }
    if envelope.rights & policy.requested_rights != policy.requested_rights
        || policy.requested_quota > envelope.quota
    {
        return Err(PermissionPersistenceError::Escalation);
    }
    let local_scope = resolver
        .resolve(&envelope.permission_kind, &envelope.scope_locator)
        .map_err(PermissionPersistenceError::Scope)?;
    if local_scope.permission_kind != envelope.permission_kind
        || local_scope.stable_identity != envelope.scope_identity
        || local_scope.local_locator.is_empty()
    {
        return Err(PermissionPersistenceError::ScopeMismatch);
    }
    replay.consume(envelope.nonce)?;
    Ok(ImportedPermission {
        permission_kind: envelope.permission_kind,
        rights: policy.requested_rights,
        quota: policy.requested_quota,
        expires_at: envelope.expires_at,
        local_locator: local_scope.local_locator,
        stable_identity: local_scope.stable_identity,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionPersistenceError {
    InvalidEnvelope,
    InvalidPolicy,
    Codec(PermissionCodecError),
    Scope(ScopeResolutionError),
    ScopeMismatch,
    WrongIssuer,
    WrongAudience,
    WrongKind,
    Expired,
    Replay,
    ReplayStateFull,
    Escalation,
}

fn encode_permission_envelope(
    envelope: &PermissionEnvelope,
) -> Result<Vec<u8>, PermissionPersistenceError> {
    let mut output = Vec::new();
    output.extend_from_slice(&PERMISSION_ENVELOPE_MAGIC);
    output.extend_from_slice(&PERMISSION_ENVELOPE_VERSION.to_be_bytes());
    push_sized(&mut output, envelope.issuer.as_bytes())?;
    push_sized(&mut output, envelope.audience.as_bytes())?;
    push_sized(&mut output, envelope.permission_kind.as_bytes())?;
    output.extend_from_slice(&envelope.rights.to_be_bytes());
    output.extend_from_slice(&envelope.quota.to_be_bytes());
    output.extend_from_slice(&envelope.expires_at.to_be_bytes());
    output.extend_from_slice(&envelope.nonce);
    push_sized(&mut output, &envelope.scope_locator)?;
    push_sized(&mut output, &envelope.scope_identity)?;
    if output.len() > MAX_OPENED_ENVELOPE_BYTES {
        return Err(PermissionPersistenceError::InvalidEnvelope);
    }
    Ok(output)
}

fn push_sized(output: &mut Vec<u8>, value: &[u8]) -> Result<(), PermissionPersistenceError> {
    let length =
        u32::try_from(value.len()).map_err(|_| PermissionPersistenceError::InvalidEnvelope)?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

fn decode_permission_envelope(
    input: &[u8],
) -> Result<PermissionEnvelope, PermissionPersistenceError> {
    let mut cursor = EnvelopeCursor::new(input);
    if cursor.take(4)? != PERMISSION_ENVELOPE_MAGIC
        || cursor.read_u16()? != PERMISSION_ENVELOPE_VERSION
    {
        return Err(PermissionPersistenceError::InvalidEnvelope);
    }
    let issuer = cursor.read_text(MAX_PERMISSION_TEXT_BYTES)?;
    let audience = cursor.read_text(MAX_PERMISSION_TEXT_BYTES)?;
    let permission_kind = cursor.read_text(MAX_PERMISSION_TEXT_BYTES)?;
    let rights = cursor.read_u64()?;
    let quota = cursor.read_u64()?;
    let expires_at = cursor.read_u64()?;
    let nonce: [u8; 16] = cursor
        .take(16)?
        .try_into()
        .map_err(|_| PermissionPersistenceError::InvalidEnvelope)?;
    let scope_locator = cursor.read_bytes(MAX_SCOPE_LOCATOR_BYTES)?;
    let scope_identity = cursor.read_bytes(MAX_SCOPE_IDENTITY_BYTES)?;
    if !cursor.is_finished() {
        return Err(PermissionPersistenceError::InvalidEnvelope);
    }
    PermissionEnvelope::new(
        issuer,
        audience,
        permission_kind,
        rights,
        quota,
        scope_locator,
        scope_identity,
        expires_at,
        nonce,
    )
}

struct EnvelopeCursor<'a> {
    input: &'a [u8],
    offset: usize,
}

impl<'a> EnvelopeCursor<'a> {
    const fn new(input: &'a [u8]) -> Self {
        Self { input, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], PermissionPersistenceError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(PermissionPersistenceError::InvalidEnvelope)?;
        let value = self
            .input
            .get(self.offset..end)
            .ok_or(PermissionPersistenceError::InvalidEnvelope)?;
        self.offset = end;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16, PermissionPersistenceError> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(
            |_| PermissionPersistenceError::InvalidEnvelope,
        )?))
    }

    fn read_u32(&mut self) -> Result<u32, PermissionPersistenceError> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(
            |_| PermissionPersistenceError::InvalidEnvelope,
        )?))
    }

    fn read_u64(&mut self) -> Result<u64, PermissionPersistenceError> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().map_err(
            |_| PermissionPersistenceError::InvalidEnvelope,
        )?))
    }

    fn read_bytes(&mut self, maximum: usize) -> Result<Vec<u8>, PermissionPersistenceError> {
        let length = usize::try_from(self.read_u32()?)
            .map_err(|_| PermissionPersistenceError::InvalidEnvelope)?;
        if length > maximum {
            return Err(PermissionPersistenceError::InvalidEnvelope);
        }
        Ok(self.take(length)?.to_vec())
    }

    fn read_text(&mut self, maximum: usize) -> Result<String, PermissionPersistenceError> {
        String::from_utf8(self.read_bytes(maximum)?)
            .map_err(|_| PermissionPersistenceError::InvalidEnvelope)
    }

    const fn is_finished(&self) -> bool {
        self.offset == self.input.len()
    }
}

/// Identity of the runtime that owns a capability store.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RuntimeId(u64);

impl RuntimeId {
    /// Constructs an identity from a core-generated non-zero value.
    ///
    /// Zero is reserved so a zero-filled or forged transport value is invalid.
    #[must_use]
    pub const fn new(value: u64) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }
}

/// Rights known to the permission core. A request bitset is not authority until
/// it is stored in a runtime-minted capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rights(u64);

impl Rights {
    pub const DIRECTORY_OPEN: Self = Self(1 << 0);
    pub const DIRECTORY_READ: Self = Self(1 << 1);
    pub const DIRECTORY_WRITE: Self = Self(1 << 2);
    pub const DIRECTORY_CREATE: Self = Self(1 << 3);
    pub const DIRECTORY_TRUNCATE: Self = Self(1 << 4);
    pub const DIRECTORY_METADATA: Self = Self(1 << 5);
    pub const DIRECTORY_ENUMERATE: Self = Self(1 << 6);
    pub const CUSTOM_INVOKE: Self = Self(1 << 16);

    /// Combines requested rights. This does not mint authority.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Returns whether `self` includes every bit in `requested`.
    #[must_use]
    pub const fn contains(self, requested: Self) -> bool {
        self.0 & requested.0 == requested.0
    }

    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Engine-neutral capability scope. Directory identity is opaque and never an
/// OS descriptor or path; components represent attenuation below that root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Scope {
    Directory {
        root_identity: u64,
        components: Vec<Vec<u8>>,
    },
    Custom {
        namespace: String,
    },
}

impl Scope {
    fn is_within(&self, parent: &Self) -> bool {
        match (self, parent) {
            (
                Self::Directory {
                    root_identity: child_root,
                    components: child_components,
                },
                Self::Directory {
                    root_identity: parent_root,
                    components: parent_components,
                },
            ) => {
                child_root == parent_root
                    && child_components
                        .as_slice()
                        .starts_with(parent_components.as_slice())
            }
            (Self::Custom { namespace: child }, Self::Custom { namespace: parent }) => {
                child == parent
            }
            _ => false,
        }
    }
}

/// Validated trusted grant input. Construction rejects authority-free grants.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedGrant {
    scope: Scope,
    rights: Rights,
    quota: u64,
    expires_at: Option<u64>,
}

impl ValidatedGrant {
    /// Creates a grant only after the trusted decision and scope were validated.
    ///
    /// # Errors
    ///
    /// Rejects empty rights or zero quota.
    pub fn new(
        scope: Scope,
        rights: Rights,
        quota: u64,
        expires_at: Option<u64>,
    ) -> Result<Self, CapabilityError> {
        if rights.is_empty() || quota == 0 {
            return Err(CapabilityError::InvalidGrant);
        }
        Ok(Self {
            scope,
            rights,
            quota,
            expires_at,
        })
    }
}

/// Unforgeable in safe Rust because its fields and constructor are private.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct CapabilityHandle {
    runtime_id: RuntimeId,
    capability_id: u64,
}

#[derive(Clone, Debug)]
struct Capability {
    parent: Option<u64>,
    scope: Scope,
    rights: Rights,
    quota: u64,
    expires_at: Option<u64>,
    revoked: bool,
}

/// Runtime-local capability authority. IDs are never reused.
#[derive(Debug)]
pub struct CapabilityStore {
    runtime_id: RuntimeId,
    next_id: u64,
    max_capabilities: usize,
    capabilities: HashMap<u64, Capability>,
}

impl CapabilityStore {
    /// Creates an empty, deny-by-default capability store.
    ///
    /// # Errors
    ///
    /// Rejects a zero capacity.
    pub fn new(runtime_id: RuntimeId, max_capabilities: usize) -> Result<Self, CapabilityError> {
        if max_capabilities == 0 {
            return Err(CapabilityError::QuotaExceeded);
        }
        Ok(Self {
            runtime_id,
            next_id: 1,
            max_capabilities,
            capabilities: HashMap::new(),
        })
    }

    /// Mints authority from an already validated trusted grant.
    ///
    /// # Errors
    ///
    /// Rejects capacity exhaustion or capability-ID exhaustion.
    pub fn mint_validated(
        &mut self,
        grant: ValidatedGrant,
    ) -> Result<CapabilityHandle, CapabilityError> {
        if self.capabilities.len() >= self.max_capabilities {
            return Err(CapabilityError::QuotaExceeded);
        }
        let capability_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(CapabilityError::IdentifierExhausted)?;
        self.capabilities.insert(
            capability_id,
            Capability {
                parent: None,
                scope: grant.scope,
                rights: grant.rights,
                quota: grant.quota,
                expires_at: grant.expires_at,
                revoked: false,
            },
        );
        Ok(CapabilityHandle {
            runtime_id: self.runtime_id,
            capability_id,
        })
    }

    /// Derives a capability with monotonically reduced authority.
    ///
    /// # Errors
    ///
    /// Rejects unknown, cross-runtime, revoked, or expired parents and any
    /// increase in scope, rights, quota, or expiry.
    pub fn derive(
        &mut self,
        parent: CapabilityHandle,
        scope: Scope,
        rights: Rights,
        quota: u64,
        expires_at: Option<u64>,
        now: u64,
    ) -> Result<CapabilityHandle, CapabilityError> {
        self.authorize(parent, rights, &scope, quota, expires_at, now)?;
        if self.capabilities.len() >= self.max_capabilities {
            return Err(CapabilityError::QuotaExceeded);
        }
        let capability_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(CapabilityError::IdentifierExhausted)?;
        self.capabilities.insert(
            capability_id,
            Capability {
                parent: Some(parent.capability_id),
                scope,
                rights,
                quota,
                expires_at,
                revoked: false,
            },
        );
        Ok(CapabilityHandle {
            runtime_id: self.runtime_id,
            capability_id,
        })
    }

    /// Checks that a capability includes all requested authority.
    ///
    /// # Errors
    ///
    /// Rejects cross-runtime, unknown, revoked, expired, or insufficient
    /// capabilities without consulting the target resource.
    pub fn authorize(
        &self,
        handle: CapabilityHandle,
        requested_rights: Rights,
        requested_scope: &Scope,
        requested_quota: u64,
        requested_expiry: Option<u64>,
        now: u64,
    ) -> Result<(), CapabilityError> {
        if handle.runtime_id != self.runtime_id {
            return Err(CapabilityError::WrongRuntime);
        }
        let capability = self
            .capabilities
            .get(&handle.capability_id)
            .ok_or(CapabilityError::InvalidCapability)?;
        self.validate_chain(handle.capability_id, now)?;
        if !capability.rights.contains(requested_rights)
            || !requested_scope.is_within(&capability.scope)
            || requested_quota > capability.quota
            || !expiry_is_at_most(requested_expiry, capability.expires_at)
        {
            return Err(CapabilityError::Escalation);
        }
        Ok(())
    }

    /// Revokes a capability; descendants fail through parent-chain validation.
    ///
    /// # Errors
    ///
    /// Rejects cross-runtime and unknown handles.
    pub fn revoke(&mut self, handle: CapabilityHandle) -> Result<(), CapabilityError> {
        if handle.runtime_id != self.runtime_id {
            return Err(CapabilityError::WrongRuntime);
        }
        let capability = self
            .capabilities
            .get_mut(&handle.capability_id)
            .ok_or(CapabilityError::InvalidCapability)?;
        capability.revoked = true;
        Ok(())
    }

    fn validate_chain(&self, mut capability_id: u64, now: u64) -> Result<(), CapabilityError> {
        loop {
            let capability = self
                .capabilities
                .get(&capability_id)
                .ok_or(CapabilityError::InvalidCapability)?;
            if capability.revoked {
                return Err(CapabilityError::Revoked);
            }
            if capability.expires_at.is_some_and(|expiry| now >= expiry) {
                return Err(CapabilityError::Expired);
            }
            match capability.parent {
                Some(parent) => capability_id = parent,
                None => return Ok(()),
            }
        }
    }
}

fn expiry_is_at_most(child: Option<u64>, parent: Option<u64>) -> bool {
    match (child, parent) {
        (Some(child), Some(parent)) => child <= parent,
        (_, None) => true,
        (None, Some(_)) => false,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityError {
    InvalidGrant,
    InvalidCapability,
    WrongRuntime,
    Escalation,
    Revoked,
    Expired,
    QuotaExceeded,
    IdentifierExhausted,
}

#[cfg(test)]
mod tests {
    use super::{
        CapabilityError, CapabilityStore, PermissionCodec, PermissionCodecError,
        PermissionEnvelope, PermissionImportPolicy, PermissionPersistenceError,
        PermissionReplayCache, ResolvedPermissionScope, Rights, RuntimeId, Scope,
        ScopeLocatorResolver, ScopeResolutionError, ValidatedGrant, export_permission,
        import_permission,
    };
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TestCodec {
        next_blob: AtomicU64,
        opened: Mutex<HashMap<Vec<u8>, Vec<u8>>>,
    }

    impl TestCodec {
        fn new() -> Self {
            Self {
                next_blob: AtomicU64::new(1),
                opened: Mutex::new(HashMap::new()),
            }
        }
    }

    impl PermissionCodec for TestCodec {
        fn seal(&self, authenticated_envelope: &[u8]) -> Result<Vec<u8>, PermissionCodecError> {
            let mut blob = b"opaque-test-blob".to_vec();
            blob.extend_from_slice(&self.next_blob.fetch_add(1, Ordering::Relaxed).to_be_bytes());
            self.opened
                .lock()
                .map_err(|_| PermissionCodecError::Unavailable)?
                .insert(blob.clone(), authenticated_envelope.to_vec());
            Ok(blob)
        }

        fn open(&self, blob: &[u8]) -> Result<Vec<u8>, PermissionCodecError> {
            self.opened
                .lock()
                .map_err(|_| PermissionCodecError::Unavailable)?
                .get(blob)
                .cloned()
                .ok_or(PermissionCodecError::Invalid)
        }
    }

    struct TestResolver {
        identity: Vec<u8>,
    }

    impl ScopeLocatorResolver for TestResolver {
        fn resolve(
            &self,
            permission_kind: &str,
            scope_locator: &[u8],
        ) -> Result<ResolvedPermissionScope, ScopeResolutionError> {
            if scope_locator != b"bookmark" {
                return Err(ScopeResolutionError::Invalid);
            }
            Ok(ResolvedPermissionScope {
                permission_kind: permission_kind.to_owned(),
                stable_identity: self.identity.clone(),
                local_locator: b"/trusted/resolved/path".to_vec(),
            })
        }
    }

    fn persistent_envelope() -> PermissionEnvelope {
        PermissionEnvelope::new(
            "com.example.issuer",
            "com.example.application",
            "zintl.permission.fs.directory",
            0b11,
            100,
            b"bookmark".to_vec(),
            b"device-and-file-identity".to_vec(),
            50,
            [7; 16],
        )
        .expect("valid envelope")
    }

    fn import_policy(rights: u64, now: u64) -> PermissionImportPolicy {
        PermissionImportPolicy::new(
            "com.example.issuer",
            "com.example.application",
            "zintl.permission.fs.directory",
            rights,
            50,
            now,
        )
        .expect("valid policy")
    }

    fn resolver() -> TestResolver {
        TestResolver {
            identity: b"device-and-file-identity".to_vec(),
        }
    }

    fn directory_scope(components: &[&[u8]]) -> Scope {
        Scope::Directory {
            root_identity: 7,
            components: components.iter().map(|item| item.to_vec()).collect(),
        }
    }

    fn store(id: u64) -> CapabilityStore {
        CapabilityStore::new(RuntimeId::new(id).expect("non-zero runtime ID"), 16)
            .expect("valid store")
    }

    fn read_grant() -> ValidatedGrant {
        ValidatedGrant::new(
            directory_scope(&[]),
            Rights::DIRECTORY_OPEN.union(Rights::DIRECTORY_READ),
            100,
            Some(50),
        )
        .expect("valid grant")
    }

    #[test]
    // Verifies an empty store has no ambient authority.
    fn empty_store_denies_unknown_capability() {
        let mut source = store(1);
        let handle = source.mint_validated(read_grant()).expect("minted");
        let empty = store(2);
        assert_eq!(
            empty.authorize(
                handle,
                Rights::DIRECTORY_READ,
                &directory_scope(&[]),
                1,
                Some(10),
                0
            ),
            Err(CapabilityError::WrongRuntime)
        );
    }

    #[test]
    // Verifies derived capabilities cannot gain a right absent from the parent.
    fn derivation_rejects_rights_escalation() {
        let mut capabilities = store(1);
        let parent = capabilities.mint_validated(read_grant()).expect("minted");
        assert_eq!(
            capabilities.derive(
                parent,
                directory_scope(&[]),
                Rights::DIRECTORY_READ.union(Rights::DIRECTORY_WRITE),
                10,
                Some(20),
                0
            ),
            Err(CapabilityError::Escalation)
        );
    }

    #[test]
    // Verifies directory scope can narrow to descendants but never move or widen.
    fn derivation_rejects_scope_escalation() {
        let mut capabilities = store(1);
        let grant = ValidatedGrant::new(
            directory_scope(&[b"allowed"]),
            Rights::DIRECTORY_READ,
            10,
            None,
        )
        .expect("grant");
        let parent = capabilities.mint_validated(grant).expect("minted");
        assert!(
            capabilities
                .derive(
                    parent,
                    directory_scope(&[b"allowed", b"child"]),
                    Rights::DIRECTORY_READ,
                    5,
                    None,
                    0
                )
                .is_ok()
        );
        assert_eq!(
            capabilities.derive(
                parent,
                directory_scope(&[]),
                Rights::DIRECTORY_READ,
                5,
                None,
                0
            ),
            Err(CapabilityError::Escalation)
        );
    }

    #[test]
    // Verifies quota and expiry are monotonically reduced by derivation.
    fn derivation_rejects_quota_or_expiry_increase() {
        let mut capabilities = store(1);
        let parent = capabilities.mint_validated(read_grant()).expect("minted");
        assert_eq!(
            capabilities.derive(
                parent,
                directory_scope(&[]),
                Rights::DIRECTORY_READ,
                101,
                Some(20),
                0
            ),
            Err(CapabilityError::Escalation)
        );
        assert_eq!(
            capabilities.derive(
                parent,
                directory_scope(&[]),
                Rights::DIRECTORY_READ,
                10,
                Some(51),
                0
            ),
            Err(CapabilityError::Escalation)
        );
    }

    #[test]
    // Verifies revoking a parent invalidates every descendant.
    fn parent_revocation_reaches_child() {
        let mut capabilities = store(1);
        let parent = capabilities.mint_validated(read_grant()).expect("minted");
        let child = capabilities
            .derive(
                parent,
                directory_scope(&[b"child"]),
                Rights::DIRECTORY_READ,
                10,
                Some(20),
                0,
            )
            .expect("derived");
        capabilities.revoke(parent).expect("revoked");
        assert_eq!(
            capabilities.authorize(
                child,
                Rights::DIRECTORY_READ,
                &directory_scope(&[b"child"]),
                1,
                Some(10),
                0
            ),
            Err(CapabilityError::Revoked)
        );
    }

    #[test]
    // Verifies an expired capability cannot authorize an operation.
    fn expired_capability_is_rejected() {
        let mut capabilities = store(1);
        let handle = capabilities.mint_validated(read_grant()).expect("minted");
        assert_eq!(
            capabilities.authorize(
                handle,
                Rights::DIRECTORY_READ,
                &directory_scope(&[]),
                1,
                Some(49),
                50
            ),
            Err(CapabilityError::Expired)
        );
    }

    #[test]
    // Verifies export is opaque and import can only attenuate an authenticated directory grant.
    fn permission_round_trip_is_opaque_and_attenuated() {
        let codec = TestCodec::new();
        let blob = export_permission(&codec, &persistent_envelope()).expect("sealed");
        assert!(!blob.windows(8).any(|window| window == b"bookmark"));
        assert!(!blob.windows(7).any(|window| window == b"example"));
        let mut replay = PermissionReplayCache::new(4).expect("replay cache");
        let imported = import_permission(
            &codec,
            &resolver(),
            &mut replay,
            &blob,
            &import_policy(0b01, 10),
        )
        .expect("imported");
        assert_eq!(imported.rights, 0b01);
        assert_eq!(imported.quota, 50);
        assert_eq!(imported.local_locator, b"/trusted/resolved/path");
        assert_eq!(
            import_permission(
                &codec,
                &resolver(),
                &mut replay,
                &blob,
                &import_policy(0b01, 10)
            ),
            Err(PermissionPersistenceError::Replay)
        );
    }

    #[test]
    // Verifies modified opaque bytes, authority increase, and expiry fail before scope resolution.
    fn permission_import_rejects_tamper_escalation_and_expiry() {
        let codec = TestCodec::new();
        let blob = export_permission(&codec, &persistent_envelope()).expect("sealed");
        let mut tampered = blob.clone();
        tampered[0] ^= 1;
        let mut replay = PermissionReplayCache::new(4).expect("replay cache");
        assert_eq!(
            import_permission(
                &codec,
                &resolver(),
                &mut replay,
                &tampered,
                &import_policy(0b01, 10)
            ),
            Err(PermissionPersistenceError::Codec(
                PermissionCodecError::Invalid
            ))
        );
        assert_eq!(
            import_permission(
                &codec,
                &resolver(),
                &mut replay,
                &blob,
                &import_policy(0b100, 10)
            ),
            Err(PermissionPersistenceError::Escalation)
        );
        assert_eq!(
            import_permission(
                &codec,
                &resolver(),
                &mut replay,
                &blob,
                &import_policy(0b01, 50)
            ),
            Err(PermissionPersistenceError::Expired)
        );
    }

    #[test]
    // Verifies issuer, audience, kind, and resolved identity are exact authenticated matches.
    fn permission_import_rejects_policy_or_scope_substitution() {
        let codec = TestCodec::new();
        let blob = export_permission(&codec, &persistent_envelope()).expect("sealed");
        let policies = [
            (
                PermissionImportPolicy::new(
                    "wrong.issuer",
                    "com.example.application",
                    "zintl.permission.fs.directory",
                    1,
                    50,
                    1,
                )
                .expect("policy"),
                PermissionPersistenceError::WrongIssuer,
            ),
            (
                PermissionImportPolicy::new(
                    "com.example.issuer",
                    "wrong.audience",
                    "zintl.permission.fs.directory",
                    1,
                    50,
                    1,
                )
                .expect("policy"),
                PermissionPersistenceError::WrongAudience,
            ),
            (
                PermissionImportPolicy::new(
                    "com.example.issuer",
                    "com.example.application",
                    "wrong.kind",
                    1,
                    50,
                    1,
                )
                .expect("policy"),
                PermissionPersistenceError::WrongKind,
            ),
        ];
        for (policy, expected) in policies {
            let mut replay = PermissionReplayCache::new(1).expect("replay cache");
            assert_eq!(
                import_permission(&codec, &resolver(), &mut replay, &blob, &policy),
                Err(expected)
            );
        }

        let wrong_identity = TestResolver {
            identity: b"replacement-identity".to_vec(),
        };
        let mut replay = PermissionReplayCache::new(1).expect("replay cache");
        assert_eq!(
            import_permission(
                &codec,
                &wrong_identity,
                &mut replay,
                &blob,
                &import_policy(1, 1)
            ),
            Err(PermissionPersistenceError::ScopeMismatch)
        );
        assert!(
            import_permission(
                &codec,
                &resolver(),
                &mut replay,
                &blob,
                &import_policy(1, 1)
            )
            .is_ok(),
            "a failed resolution must not consume the nonce or mint a grant"
        );
    }
}
