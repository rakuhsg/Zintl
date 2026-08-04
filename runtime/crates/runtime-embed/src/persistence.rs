use crate::PermissionCodec;
use crate::filesystem::Directory;
use crate::{AuditCategory, AuditOutcome, RuntimeError, Services};
use runtime_filesystem::{ApprovedDirectory, DirectoryIdentity, FilesystemRights};
use runtime_permission::{
    PermissionEnvelope, PermissionImportPolicy, PermissionReplayCache, ResolvedPermissionScope,
    ScopeLocatorResolver, ScopeResolutionError, export_permission, import_permission,
};
use runtime_resource::ResourceHandle;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

const DIRECTORY_PERMISSION: &str = "zintl.permission.fs.directory";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeCodecError {
    Invalid,
    Unavailable,
}

/// Trusted platform boundary for bookmarks, portals, or exact local paths.
pub trait DirectoryScopeCodec: Send + Sync + 'static {
    /// Encodes a validated local directory locator into persistent opaque scope data.
    ///
    /// # Errors
    ///
    /// Rejects locators the platform cannot persist safely.
    fn encode(&self, local_path: &Path) -> Result<Vec<u8>, ScopeCodecError>;

    /// Resolves authenticated persistent scope data to a current local locator.
    ///
    /// # Errors
    ///
    /// Rejects invalid, stale, or unavailable platform scope data.
    fn resolve(&self, persistent_scope: &[u8]) -> Result<PathBuf, ScopeCodecError>;
}

/// Exact UTF-8 path codec for trusted CLI/headless hosts. GUI sandbox hosts
/// should supply bookmarks or a portal-backed implementation instead.
#[derive(Clone, Copy, Debug, Default)]
pub struct ExactPathScopeCodec;

impl DirectoryScopeCodec for ExactPathScopeCodec {
    fn encode(&self, local_path: &Path) -> Result<Vec<u8>, ScopeCodecError> {
        local_path
            .to_str()
            .map(|path| path.as_bytes().to_vec())
            .ok_or(ScopeCodecError::Invalid)
    }

    fn resolve(&self, persistent_scope: &[u8]) -> Result<PathBuf, ScopeCodecError> {
        let path = std::str::from_utf8(persistent_scope).map_err(|_| ScopeCodecError::Invalid)?;
        Ok(PathBuf::from(path))
    }
}

pub struct PersistenceConfiguration {
    pub(crate) issuer: String,
    pub(crate) audience: String,
    pub(crate) codec: Arc<dyn PermissionCodec>,
    pub(crate) scope_codec: Arc<dyn DirectoryScopeCodec>,
    pub(crate) maximum_replay_entries: usize,
}

impl PersistenceConfiguration {
    /// Creates an authenticated permission persistence boundary.
    ///
    /// # Errors
    ///
    /// Rejects empty/oversized issuer or audience and zero replay capacity.
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        codec: Arc<dyn PermissionCodec>,
        scope_codec: Arc<dyn DirectoryScopeCodec>,
        maximum_replay_entries: usize,
    ) -> Result<Self, RuntimeError> {
        let issuer = issuer.into();
        let audience = audience.into();
        if issuer.is_empty()
            || audience.is_empty()
            || issuer.len() > 255
            || audience.len() > 255
            || maximum_replay_entries == 0
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
        Ok(Self {
            issuer,
            audience,
            codec,
            scope_codec,
            maximum_replay_entries,
        })
    }
}

pub(crate) struct PersistenceState {
    configuration: PersistenceConfiguration,
    replay: Mutex<PermissionReplayCache>,
}

impl PersistenceState {
    pub(crate) fn new(configuration: PersistenceConfiguration) -> Result<Self, RuntimeError> {
        let replay = PermissionReplayCache::new(configuration.maximum_replay_entries)
            .map_err(|_| RuntimeError::InvalidConfiguration)?;
        Ok(Self {
            configuration,
            replay: Mutex::new(replay),
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn export_directory(
        &self,
        services: &Services,
        handle: ResourceHandle,
        locator: &[u8],
        rights: u64,
        quota: u64,
        expires_at: u64,
        now: u64,
    ) -> Result<Vec<u8>, RuntimeError> {
        if expires_at <= now {
            return Err(RuntimeError::InvalidArgument);
        }
        let local_path = std::str::from_utf8(locator)
            .map(PathBuf::from)
            .map_err(|_| RuntimeError::InvalidPermission)?;
        let persistent_scope = self
            .configuration
            .scope_codec
            .encode(&local_path)
            .map_err(|_| RuntimeError::InvalidPermission)?;
        let identity = services
            .filesystem
            .directory_identity(handle)
            .map_err(RuntimeError::from_filesystem)?;
        let envelope = PermissionEnvelope::new(
            &self.configuration.issuer,
            &self.configuration.audience,
            DIRECTORY_PERMISSION,
            rights,
            quota,
            persistent_scope,
            identity.authenticated_bytes().to_vec(),
            expires_at,
            random_nonce()?,
        )
        .map_err(|_| RuntimeError::InvalidPermission)?;
        let blob = export_permission(self.configuration.codec.as_ref(), &envelope)
            .map_err(|_| RuntimeError::InvalidPermission)?;
        services.ensure_running()?;
        services.control.record(
            AuditCategory::PermissionPersistence,
            AuditOutcome::Succeeded,
            "permission.export",
            None,
        );
        Ok(blob)
    }

    pub(crate) fn import_directory(
        &self,
        services: &Arc<Services>,
        weak: Weak<Services>,
        blob: &[u8],
        requested_rights: FilesystemRights,
        requested_quota: u64,
        now: u64,
    ) -> Result<Directory, RuntimeError> {
        let policy = PermissionImportPolicy::new(
            &self.configuration.issuer,
            &self.configuration.audience,
            DIRECTORY_PERMISSION,
            requested_rights.bits(),
            requested_quota,
            now,
        )
        .map_err(|_| RuntimeError::InvalidPermission)?;
        let resolver = ImportResolver {
            services: services.clone(),
            scope_codec: self.configuration.scope_codec.clone(),
            rights: requested_rights,
        };
        let mut replay = self.replay.lock().map_err(|_| RuntimeError::Internal)?;
        let imported = import_permission(
            self.configuration.codec.as_ref(),
            &resolver,
            &mut replay,
            blob,
            &policy,
        )
        .map_err(|_| RuntimeError::InvalidPermission)?;
        let local_path = std::str::from_utf8(&imported.local_locator)
            .map(PathBuf::from)
            .map_err(|_| RuntimeError::InvalidPermission)?;
        let identity = DirectoryIdentity::from_authenticated_bytes(&imported.stable_identity)
            .map_err(RuntimeError::from_filesystem)?;
        let approved = ApprovedDirectory::from_trusted_approval(&local_path, requested_rights)
            .map_err(RuntimeError::from_filesystem)?;
        let handle = services
            .filesystem
            .open_imported_directory(&approved, identity)
            .map_err(RuntimeError::from_filesystem)?;
        services.track_resource(handle)?;
        if let Err(error) = services.ensure_running() {
            let _ = services.close_resource(handle);
            return Err(error);
        }
        services.control.record(
            AuditCategory::PermissionPersistence,
            AuditOutcome::Succeeded,
            "permission.import",
            None,
        );
        Ok(Directory::new(
            weak,
            handle,
            imported.local_locator,
            imported.rights,
            imported.quota,
        ))
    }
}

struct ImportResolver {
    services: Arc<Services>,
    scope_codec: Arc<dyn DirectoryScopeCodec>,
    rights: FilesystemRights,
}

impl ScopeLocatorResolver for ImportResolver {
    fn resolve(
        &self,
        permission_kind: &str,
        scope_locator: &[u8],
    ) -> Result<ResolvedPermissionScope, ScopeResolutionError> {
        if permission_kind != DIRECTORY_PERMISSION {
            return Err(ScopeResolutionError::Invalid);
        }
        let local_path = self
            .scope_codec
            .resolve(scope_locator)
            .map_err(|error| match error {
                ScopeCodecError::Invalid => ScopeResolutionError::Invalid,
                ScopeCodecError::Unavailable => ScopeResolutionError::Unavailable,
            })?;
        let approved = ApprovedDirectory::from_trusted_approval(&local_path, self.rights)
            .map_err(|_| ScopeResolutionError::Invalid)?;
        let handle = self
            .services
            .filesystem
            .open_approved_directory(&approved)
            .map_err(|_| ScopeResolutionError::Unavailable)?;
        let identity = self.services.filesystem.directory_identity(handle);
        let close = self.services.filesystem.close(handle);
        let identity = identity.map_err(|_| ScopeResolutionError::Unavailable)?;
        close.map_err(|_| ScopeResolutionError::Unavailable)?;
        let local_locator = local_path
            .to_str()
            .map(|path| path.as_bytes().to_vec())
            .ok_or(ScopeResolutionError::Invalid)?;
        Ok(ResolvedPermissionScope {
            permission_kind: DIRECTORY_PERMISSION.to_string(),
            stable_identity: identity.authenticated_bytes().to_vec(),
            local_locator,
        })
    }
}

fn random_nonce() -> Result<[u8; 16], RuntimeError> {
    let mut nonce = [0_u8; 16];
    getrandom::fill(&mut nonce).map_err(|_| RuntimeError::Internal)?;
    Ok(nonce)
}
