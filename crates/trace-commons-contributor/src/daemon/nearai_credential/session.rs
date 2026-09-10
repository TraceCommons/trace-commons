//! Serializes refreshes and credential publication for each daemon directory.
//!
//! Lock order: refresh (optional), commit, then daemon settings. Never hold
//! commit across an await. Forget only needs commit, so a slow Cloud response
//! cannot delay local removal.

use crate::daemon::ipc::DaemonShared;
use crate::daemon::nearai_credential::api::CloudApi;
use crate::daemon::settings::{DaemonSettings, NearAiSession};
use anyhow::{Result, anyhow};
use chrono::Utc;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub(crate) struct Coordination {
    pub commit: CredentialFileLock,
    pub storage: CredentialFileLock,
    refresh: tokio::sync::Mutex<()>,
}

/// Also serializes the FFI pre-start writer and a daemon in another process.
/// Lock files remain in place: unlinking one would split locks across inodes.
/// On Unix, std's OpenOptions opens with O_CLOEXEC: spawned proxy processes
/// cannot retain this descriptor after exec and keep the file lock alive.
pub(crate) struct CredentialFileLock {
    path: PathBuf,
    local: Mutex<()>,
}

pub(crate) struct CredentialFileGuard<'a> {
    file: std::fs::File,
    _local: std::sync::MutexGuard<'a, ()>,
}

impl CredentialFileLock {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            local: Mutex::new(()),
        }
    }

    pub(crate) fn lock(&self) -> Result<CredentialFileGuard<'_>> {
        let local = self
            .local
            .lock()
            .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&self.path)
            .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
        file.lock()
            .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
        Ok(CredentialFileGuard {
            file,
            _local: local,
        })
    }
}

impl Drop for CredentialFileGuard<'_> {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

pub(crate) fn coordination(dir: &Path) -> Result<Arc<Coordination>> {
    static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Weak<Coordination>>>> = OnceLock::new();
    let mut registry = REGISTRY
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
    registry.retain(|_, value| value.strong_count() > 0);
    if let Some(existing) = registry.get(dir).and_then(Weak::upgrade) {
        return Ok(existing);
    }
    let value = Arc::new(Coordination {
        commit: CredentialFileLock::new(dir.join(".cloud-credential-commit.lock")),
        storage: CredentialFileLock::new(dir.join(".cloud-credential-storage.lock")),
        refresh: tokio::sync::Mutex::new(()),
    });
    registry.insert(dir.to_path_buf(), Arc::downgrade(&value));
    Ok(value)
}

/// Read a bound runtime snapshot while the caller holds this directory's
/// commit guard. OS reads never occur here: matching metadata proves that the
/// already resolved memory credentials belong to the current disk record.
pub(crate) fn runtime_snapshot(
    shared: &DaemonShared,
    _commit: &CredentialFileGuard<'_>,
) -> Result<DaemonSettings> {
    let mut settings = DaemonSettings::load(&shared.store)?;
    let memory = shared
        .settings
        .lock()
        .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
    if memory.cloud_storage_unavailable {
        return Err(anyhow!("near_ai_credential_storage_unavailable"));
    }
    if settings.cloud_credentials.is_some() {
        crate::daemon::cloud_credential_lifecycle::ensure_current(&settings, &memory)?;
        settings.near_ai_inference = memory.near_ai_inference.clone();
        settings.near_ai_session = memory.near_ai_session.clone();
    }
    Ok(settings)
}

/// A stale caller is refused rather than switched onto another connection.
/// Rotation reaches disk and memory before its access token leaves this call.
pub(crate) struct SessionAccess {
    pub access_token: String,
    pub retained: NearAiSession,
}

pub(crate) async fn exchange(
    shared: &DaemonShared,
    api: &CloudApi,
    expected: &NearAiSession,
) -> Result<SessionAccess> {
    let locks = coordination(shared.store.dir())?;
    let _refresh = locks.refresh.lock().await;
    let expected_settings = {
        let _commit = locks
            .commit
            .lock()
            .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
        let settings = runtime_snapshot(shared, &_commit)?;
        if settings.near_ai_session.as_ref() != Some(expected) {
            return Err(anyhow!("near_ai_credential_session_changed"));
        }
        settings
    };
    let refreshed = api.refresh_session(&expected.refresh_token).await?;
    let session = NearAiSession {
        refresh_token: refreshed.session.refresh_token,
        refresh_token_expires_at: Some(refreshed.refresh_token_expires_at),
        stored_at: Utc::now(),
    };
    let store = shared.store.clone();
    let memory = Arc::clone(&shared.settings);
    let retained = session.clone();
    tokio::task::spawn_blocking(move || {
        crate::daemon::cloud_credential_lifecycle::native(&store)?.replace(
            &expected_settings,
            expected_settings.near_ai_inference.clone(),
            Some(retained),
            || Ok(()),
            |stored| {
                let mut memory = memory
                    .lock()
                    .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
                memory.near_ai_session = stored.near_ai_session.clone();
                memory.cloud_credentials = stored.cloud_credentials.clone();
                memory.cloud_storage_unavailable = false;
                Ok(())
            },
        )
    })
    .await
    .map_err(|_| anyhow!("near_ai_credential_unavailable"))??;
    Ok(SessionAccess {
        access_token: refreshed.session.access_token,
        retained: session,
    })
}
