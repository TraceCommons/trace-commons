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

#[derive(Default)]
pub(crate) struct Coordination {
    pub commit: Mutex<()>,
    refresh: tokio::sync::Mutex<()>,
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
    let value = Arc::new(Coordination::default());
    registry.insert(dir.to_path_buf(), Arc::downgrade(&value));
    Ok(value)
}

fn current_settings(shared: &DaemonShared, expected: &NearAiSession) -> Result<DaemonSettings> {
    let settings = DaemonSettings::load(&shared.store)?;
    if settings.near_ai_session.as_ref() != Some(expected) {
        return Err(anyhow!("near_ai_credential_session_changed"));
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
    {
        let _commit = locks
            .commit
            .lock()
            .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
        current_settings(shared, expected)?;
    }
    let refreshed = api.refresh_session(&expected.refresh_token).await?;
    let _commit = locks
        .commit
        .lock()
        .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
    let mut disk = current_settings(shared, expected)?;
    let mut memory = shared
        .settings
        .lock()
        .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
    let session = NearAiSession {
        refresh_token: refreshed.session.refresh_token,
        refresh_token_expires_at: Some(refreshed.refresh_token_expires_at),
        stored_at: Utc::now(),
    };
    disk.near_ai_session = Some(session.clone());
    disk.save(&shared.store)?;
    memory.near_ai_session = Some(session.clone());
    Ok(SessionAccess {
        access_token: refreshed.session.access_token,
        retained: session,
    })
}
