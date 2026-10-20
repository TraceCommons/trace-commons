//! Refreshing the signed provider-catalog document while the daemon runs.
//!
//! `docs/UPDATES.md` §2: catalog exist so that a changed `anthropic-beta` flag
//! is a minutes-long fix rather than a five-ecosystem release. That only holds
//! if a *running* daemon picks it up — a document that requires a restart to
//! take effect has the same latency as a release, which is the problem it was
//! built to solve.
//!
//! Two properties this must keep, both from `docs/TRUST.md` I2:
//!
//! - The URL is **pinned**, not configurable. A redirectable catalog source
//!   would be a supply-chain hole, and the schema deliberately cannot express a
//!   host so that a compromised *signing key* still cannot move traffic.
//! - Verification happens before parse, and a failure leaves the previous
//!   document in force. Fail-closed onto known-good values, never open.

use std::sync::Arc;
use std::time::Duration;

use crate::state::AppState;
use ironwire_catalog::{CatalogStore, SignedCatalog};
use ironwire_core::config::PathsConfig;

/// Where the signed document lives. Pinned for the same reason the release
/// manifest is.
const CATALOG_URL: &str = "https://ironwire.dev/releases/catalog.json";

/// How often a running daemon looks for a newer document.
///
/// Six hours, not six minutes. This is a fallback path for a provider changing
/// something under us; polling harder would put load on a static file for no
/// benefit, and a user who needs it *now* can restart. The first check is
/// deliberately delayed so it never competes with the first request.
const REFRESH_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Wait before the first check, so startup is never slowed by a network call.
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(60);

/// Start the background refresh, unless update checks are switched off.
///
/// Governed by the same `updates.check` switch as the release check: both are
/// requests IronWire makes that are not the user's own work, and someone who
/// turned one off meant both.
pub(crate) fn spawn_refresh(
    state: AppState,
    paths: &PathsConfig,
    enabled: bool,
) -> Option<tokio::task::JoinHandle<()>> {
    if !enabled {
        return None;
    }
    let path = paths.catalog_file();
    Some(tokio::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            match fetch_and_apply(&state, &path).await {
                Ok(Some(serial)) => {
                    tracing::info!(serial, "applied a newer provider-catalog document");
                }
                Ok(None) => tracing::debug!("provider catalog unchanged"),
                // Never fatal: the compiled-in defaults, or the document
                // already loaded, stay in force.
                Err(error) => tracing::debug!(%error, "provider-catalog refresh skipped"),
            }
            tokio::time::sleep(REFRESH_INTERVAL).await;
        }
    }))
}

/// Fetch, verify, and install — returning the new serial when one was applied.
async fn fetch_and_apply(
    state: &AppState,
    path: &std::path::Path,
) -> Result<Option<u64>, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()?;
    let body = client
        .get(CATALOG_URL)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let signed: SignedCatalog = serde_json::from_slice(&body)?;

    // Start from the store in force, so the rollback guard on `serial` applies
    // and a replayed older document is refused. There is deliberately no way to
    // read the serial before verifying: an unverified number is not evidence.
    let mut next = (*state.catalog()).clone();
    let before = next.serial();
    next.apply(&signed)?;
    let serial = next.serial();
    if serial == before {
        return Ok(None);
    }

    // Persist only after it verified, so a bad document cannot poison the
    // cache that the next startup reads.
    if let Err(error) = CatalogStore::persist(&signed, path) {
        tracing::warn!(%error, "could not cache the catalog document");
    }
    state.set_catalog(Arc::new(next));
    Ok(Some(serial))
}
