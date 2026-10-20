//! Notify-only update checking shared with the CLI.
use crate::state::AppState;
use ironwire_core::config::PathsConfig;
use ironwire_update::{CheckedAt, InstallMethod, Manifest, UpdateStatus};
const MANIFEST_URL: &str = "https://ironwire.dev/releases/manifest.json";

pub fn install_method() -> InstallMethod {
    std::env::current_exe()
        .map(|exe| InstallMethod::detect(&exe))
        .unwrap_or(InstallMethod::Unmanaged)
}

/// Fetch the manifest and evaluate it, caching the result.
pub async fn check_now(paths: &PathsConfig, install: InstallMethod) -> UpdateStatus {
    let Some(manifest) = fetch_manifest().await else {
        return UpdateStatus::Unknown;
    };
    let status = ironwire_update::evaluate(env!("CARGO_PKG_VERSION"), &manifest, install);
    let checked = CheckedAt {
        at: chrono::Utc::now(),
        status: status.clone(),
    };
    if let Err(error) = ironwire_update::save_cache(&paths.update_cache_file(), &checked) {
        tracing::debug!(%error, "could not cache the update check");
    }
    status
}

async fn fetch_manifest() -> Option<Manifest> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        // No install id, no per-check identifier, nothing about the user's
        // work: version and platform only (`docs/TRUST.md` §7).
        .user_agent(format!(
            "ironwire/{} ({})",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS
        ))
        .build()
        .ok()?;
    client
        .get(MANIFEST_URL)
        .send()
        .await
        .ok()?
        .json::<Manifest>()
        .await
        .ok()
}

/// Start the daemon's background check.
///
/// Rate-limited to once a day, honours the kill switch, and never blocks
/// startup: a proxy that will not start because a release server is down would
/// be a worse failure than being out of date.
pub(crate) fn spawn_check(
    state: AppState,
    paths: &PathsConfig,
    enabled: bool,
) -> Option<tokio::task::JoinHandle<()>> {
    let cache_path = paths.update_cache_file();
    let cached = ironwire_update::load_cache(&cache_path);

    // Whatever the last check concluded is worth showing immediately, even if
    // it is too soon to check again.
    if let Some(cached) = &cached {
        state.set_update_status(cached.status.clone());
    }
    if !ironwire_update::should_check(enabled, cached.as_ref(), chrono::Utc::now()) {
        return None;
    }

    let paths = paths.clone();
    let install = install_method();
    Some(tokio::spawn(async move {
        let status = check_now(&paths, install).await;
        if status.is_actionable() {
            tracing::info!(
                ?status,
                "a newer IronWire is available (run `ironwire update`)"
            );
        }
        state.set_update_status(status);
    }))
}
