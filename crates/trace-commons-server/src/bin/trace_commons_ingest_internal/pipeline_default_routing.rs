// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pipeline default routing (spec 2026-10-10,
//! `docs/superpowers/specs/2026-10-10-pipeline-default-routing-design.md`):
//! an operator-armed mode that routes every tenant with no routing row to the
//! versioned pipeline.
//!
//! Off unless `TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING=all`. With the mode
//! absent nothing here runs: no loop, no cross-tenant read, and the scope of
//! the process is its receipts list, as before.
//!
//! With the mode on, the process starts unarmed and arms only when the
//! results directory holds a signed package and a full set of check results
//! (the material `pipeline.py s3-activate-api` posts) that verify against
//! this process's trust stores and name this process's code revision, are
//! ready now, and name a package this process can run (`load_arming`). Any
//! failure is a safe label in a WARN line and in `GET
//! /v1/admin/config-status`, and then no tenant is activated: uploads keep
//! their routing (legacy for a tenant with no row).
//!
//! An armed process activates a tenant through the same two functions the
//! admin routes call (`pipeline_activation::qualify_for_tenant` and
//! `activate_for_tenant`), under a fixed system actor
//! (`DEFAULT_ROUTING_ACTOR_REF`) and the reason code
//! `pipeline_default_routing`, always expecting that the tenant has no
//! routing row. The routing store compares that under its routing lock, so a
//! tenant whose routing an operator decided (legacy after a deactivate,
//! contained, pipeline) is refused with `pipeline_routing_state_changed` and
//! left as it is, and two concurrent first uploads write one activation.
//!
//! Scope. Two predicates replace the receipts-list-only check:
//! `pipeline_tenant_in_activation_scope` (the list, or the mode is on: who
//! may be activated) and `pipeline_tenant_served` (the list, or the
//! routed-tenant cache: whose uploads this process takes on the pipeline and
//! whose runs its worker drains). A tenant enters the cache only after its
//! bundles pass the start checks in this process
//! (`PipelineService::check_tenant_bundles`), so a process never serves a
//! bundle it has not checked.
//!
//! Log lines name a tenant by `tenant_storage_ref` only, and hold labels,
//! counts, and hashes; never a path, a tenant id, or a key id.

use super::*;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::pipeline_activation::{
    ActivateBody, PIPELINE_ADMIN_BODY_MAX_BYTES, PIPELINE_CHANGE_COMMITTED_AUDIT_FAILED_LABEL,
    PIPELINE_MAX_ATTESTATIONS, activate_for_tenant, qualify_for_tenant,
    route_infrastructure_profile, trust_and_revision,
};
use trace_commons_server::versioned_pipeline_activation::{
    NewReceiptRouting, PIPELINE_ROUTING_STATE_CHANGED_LABEL, PipelineActivationStore, RoutingState,
};
use trace_commons_server::versioned_pipeline_qualification::{
    PipelineCheckAttestation, ProductionDependencyProfile, QUALIFICATION_EVIDENCE_STALE_LABEL,
    QUALIFICATION_PROMOTION_NOT_READY_LABEL, SignedBundlePackage, evaluate_promotion,
    package_digests,
};

/// `off` (the default; unset or empty is `off`) or `all`.
pub(crate) const TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING: &str =
    "TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING";
/// The directory of the signed check-result set: `signed-package.json` and
/// one `<check_id>.attestation.json` for each check. Required with `all`.
pub(crate) const TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_RESULTS_DIR: &str =
    "TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_RESULTS_DIR";
/// Seconds between two passes of the loop, 5..=3600, default 60.
pub(crate) const TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_INTERVAL_SECONDS: &str =
    "TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_INTERVAL_SECONDS";
/// Tenants a pass routes, and tenants it re-qualifies, at most: 1..=1000,
/// default 50.
pub(crate) const TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_BATCH: &str =
    "TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_BATCH";

/// The signed package's file name in the results directory (what
/// `pipeline.py promote` writes, `PACKAGE_FILE`).
pub(crate) const DEFAULT_ROUTING_PACKAGE_FILE: &str = "signed-package.json";
/// The suffix of a signed check result's file name.
pub(crate) const DEFAULT_ROUTING_ATTESTATION_SUFFIX: &str = ".attestation.json";

const DEFAULT_INTERVAL_SECONDS: u64 = 60;
const DEFAULT_BATCH: i64 = 50;
const INTERVAL_SECONDS_RANGE: std::ops::RangeInclusive<u64> = 5..=3600;
const BATCH_RANGE: std::ops::RangeInclusive<i64> = 1..=1000;
/// How long the routed-tenant cache is used before it is read again.
pub(crate) const ROUTED_TENANT_CACHE_TTL: StdDuration = StdDuration::from_secs(30);
/// How long a tenant whose activation was refused is left alone.
pub(crate) const DEFAULT_ROUTING_BACKOFF: StdDuration = StdDuration::from_secs(600);
/// From this long before the armed set expires, each pass warns.
const EXPIRY_WARNING_SECONDS: i64 = 24 * 60 * 60;
/// The page size of the cache refresh's read of every routed tenant.
const ROUTED_PAGE: i64 = 1000;

/// The reason code of every routing change default routing makes.
pub(crate) const PIPELINE_DEFAULT_ROUTING_REASON_CODE: &str = "pipeline_default_routing";
/// The label the system actor's reference is derived from (the pin test
/// recomputes `DEFAULT_ROUTING_ACTOR_REF` from it).
#[cfg(test)]
pub(crate) const DEFAULT_ROUTING_ACTOR_LABEL: &str =
    "trace_commons.system.pipeline_default_routing";
/// The actor every change default routing makes is recorded under:
/// `principal_storage_ref(DEFAULT_ROUTING_ACTOR_LABEL)`, the canonical
/// `principal_sha256:` form of a principal reference, pinned by a test.
pub(crate) const DEFAULT_ROUTING_ACTOR_REF: &str =
    "principal_sha256:f181c8ce8ad8456d28933772fb616dd07c088b830c3a9e9df8b328652f59151b";

// Start refusals.
pub(crate) const PIPELINE_DEFAULT_ROUTING_MODE_INVALID_LABEL: &str =
    "pipeline_default_routing_mode_invalid";
pub(crate) const PIPELINE_DEFAULT_ROUTING_REQUIRES_PRODUCTION_RUNTIME_LABEL: &str =
    "pipeline_default_routing_requires_production_runtime";
pub(crate) const PIPELINE_DEFAULT_ROUTING_TEST_DEPENDENCIES_CONFLICT_LABEL: &str =
    "pipeline_default_routing_test_dependencies_conflict";
pub(crate) const PIPELINE_DEFAULT_ROUTING_RESULTS_DIR_MISSING_LABEL: &str =
    "pipeline_default_routing_results_dir_missing";
pub(crate) const PIPELINE_DEFAULT_ROUTING_CONFIG_INVALID_LABEL: &str =
    "pipeline_default_routing_config_invalid";
pub(crate) const PIPELINE_DEFAULT_ROUTING_ENUMERATION_UNAVAILABLE_LABEL: &str =
    "pipeline_default_routing_enumeration_unavailable";

// Arming refusals (the process runs, unarmed).
pub(crate) const PIPELINE_DEFAULT_ROUTING_NOT_LOADED_LABEL: &str =
    "pipeline_default_routing_not_loaded";
pub(crate) const PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL: &str =
    "pipeline_default_routing_results_missing";
pub(crate) const PIPELINE_DEFAULT_ROUTING_RESULTS_INVALID_LABEL: &str =
    "pipeline_default_routing_results_invalid";
pub(crate) const PIPELINE_DEFAULT_ROUTING_REVISION_MISMATCH_LABEL: &str =
    "pipeline_default_routing_revision_mismatch";
pub(crate) const PIPELINE_DEFAULT_ROUTING_RESULTS_STALE_LABEL: &str =
    "pipeline_default_routing_results_stale";
pub(crate) const PIPELINE_DEFAULT_ROUTING_PACKAGE_MISMATCH_LABEL: &str =
    "pipeline_default_routing_package_mismatch";

// Pass warnings.
pub(crate) const PIPELINE_DEFAULT_ROUTING_RESULTS_EXPIRING_LABEL: &str =
    "pipeline_default_routing_results_expiring";
pub(crate) const PIPELINE_DEFAULT_ROUTING_REQUALIFY_BUNDLE_DIFFERS_LABEL: &str =
    "pipeline_default_routing_requalify_bundle_differs";
pub(crate) const PIPELINE_DEFAULT_ROUTING_TENANT_BUNDLE_CHECK_FAILED_LABEL: &str =
    "pipeline_default_routing_tenant_bundle_check_failed";
pub(crate) const PIPELINE_DEFAULT_ROUTING_ACTIVATION_REFUSED_LABEL: &str =
    "pipeline_default_routing_activation_refused";
pub(crate) const PIPELINE_DEFAULT_ROUTING_ENUMERATION_FAILED_LABEL: &str =
    "pipeline_default_routing_enumeration_failed";

/// The parsed configuration of the mode `all`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PipelineDefaultRoutingConfig {
    pub(crate) results_dir: PathBuf,
    pub(crate) interval: StdDuration,
    pub(crate) batch: i64,
}

/// Parses the four variables. `Ok(None)` for the mode `off` (unset or empty
/// included), whatever the other three say: the mode absent is today's
/// behaviour. Each refusal is a safe label.
pub(crate) fn parse_default_routing_config(
    mode: Option<&str>,
    results_dir: Option<&str>,
    interval_seconds: Option<&str>,
    batch: Option<&str>,
) -> anyhow::Result<Option<PipelineDefaultRoutingConfig>> {
    match mode.map(str::trim).unwrap_or("") {
        "" | "off" => return Ok(None),
        "all" => {}
        _ => anyhow::bail!(PIPELINE_DEFAULT_ROUTING_MODE_INVALID_LABEL),
    }
    let results_dir = results_dir
        .map(str::trim)
        .filter(|dir| !dir.is_empty())
        .ok_or_else(|| anyhow::anyhow!(PIPELINE_DEFAULT_ROUTING_RESULTS_DIR_MISSING_LABEL))?;
    let interval = match interval_seconds.map(str::trim).filter(|v| !v.is_empty()) {
        None => DEFAULT_INTERVAL_SECONDS,
        Some(value) => value
            .parse::<u64>()
            .ok()
            .filter(|seconds| INTERVAL_SECONDS_RANGE.contains(seconds))
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_DEFAULT_ROUTING_CONFIG_INVALID_LABEL))?,
    };
    let batch = match batch.map(str::trim).filter(|v| !v.is_empty()) {
        None => DEFAULT_BATCH,
        Some(value) => value
            .parse::<i64>()
            .ok()
            .filter(|batch| BATCH_RANGE.contains(batch))
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_DEFAULT_ROUTING_CONFIG_INVALID_LABEL))?,
    };
    Ok(Some(PipelineDefaultRoutingConfig {
        results_dir: PathBuf::from(results_dir),
        interval: StdDuration::from_secs(interval),
        batch,
    }))
}

/// `parse_default_routing_config` over the process environment.
pub(crate) fn default_routing_config_from_env()
-> anyhow::Result<Option<PipelineDefaultRoutingConfig>> {
    let var = |name: &str| std::env::var(name).ok();
    parse_default_routing_config(
        var(TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING).as_deref(),
        var(TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_RESULTS_DIR).as_deref(),
        var(TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_INTERVAL_SECONDS).as_deref(),
        var(TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_BATCH).as_deref(),
    )
}

/// What the start knows of the rest of the process when it checks the mode
/// `all` (`validate_default_routing_start`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct DefaultRoutingStartFacts {
    /// `TRACE_COMMONS_PIPELINE_RUNTIME=production` and a runtime assembled.
    pub(crate) production_runtime: bool,
    /// Both trust stores loaded.
    pub(crate) trust_stores: bool,
    /// The build holds a code revision.
    pub(crate) code_revision: bool,
    /// The routing, qualification, and product stores (a database).
    pub(crate) routing_store: bool,
    /// `TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES`.
    pub(crate) allow_test_dependencies: bool,
}

/// The start refusals of the mode `all`, each a safe label. Nothing is
/// checked without the mode.
pub(crate) fn validate_default_routing_start(
    config: Option<&PipelineDefaultRoutingConfig>,
    facts: DefaultRoutingStartFacts,
) -> anyhow::Result<()> {
    if config.is_none() {
        return Ok(());
    }
    anyhow::ensure!(
        facts.production_runtime,
        PIPELINE_DEFAULT_ROUTING_REQUIRES_PRODUCTION_RUNTIME_LABEL
    );
    anyhow::ensure!(
        facts.trust_stores,
        super::pipeline_activation::PIPELINE_TRUST_STORE_MISSING_LABEL
    );
    anyhow::ensure!(
        facts.code_revision,
        super::pipeline_activation::PIPELINE_CODE_REVISION_UNSET_LABEL
    );
    anyhow::ensure!(
        facts.routing_store,
        super::pipeline_activation::PIPELINE_ROUTING_STORE_MISSING_LABEL
    );
    anyhow::ensure!(
        !facts.allow_test_dependencies,
        PIPELINE_DEFAULT_ROUTING_TEST_DEPENDENCIES_CONFLICT_LABEL
    );
    Ok(())
}

/// The start's database check of the mode `all`: the runtime role may call
/// both V124 functions. A failed read refuses as a missing grant does.
pub(crate) async fn validate_default_routing_enumeration(
    activation: &PipelineActivationStore,
) -> anyhow::Result<()> {
    match activation.default_routing_enumeration_ready().await {
        Ok(true) => Ok(()),
        Ok(false) | Err(_) => anyhow::bail!(PIPELINE_DEFAULT_ROUTING_ENUMERATION_UNAVAILABLE_LABEL),
    }
}

/// A verified result set that names this process's revision.
#[derive(Debug, Clone)]
pub(crate) struct ArmedSet {
    pub(crate) signed_package: SignedBundlePackage,
    pub(crate) attestations: Vec<PipelineCheckAttestation>,
    pub(crate) bundle_id: String,
    /// The earliest `observed_at + maximum_age_seconds` of the set.
    pub(crate) expires_at: DateTime<Utc>,
    /// A digest of the package hash and the promotion's evidence hash: what
    /// a log line says to tell two sets apart.
    pub(crate) set_digest: String,
}

/// Whether this process may activate a tenant now.
#[derive(Debug, Clone)]
pub(crate) enum Arming {
    Disarmed { label: String },
    Armed(Arc<ArmedSet>),
}

impl Arming {
    fn label(&self) -> Option<&str> {
        match self {
            Self::Disarmed { label } => Some(label),
            Self::Armed(_) => None,
        }
    }

    fn digest(&self) -> Option<&str> {
        match self {
            Self::Disarmed { .. } => None,
            Self::Armed(set) => Some(&set.set_digest),
        }
    }
}

/// Reads a file of at most `PIPELINE_ADMIN_BODY_MAX_BYTES`.
fn read_bounded(path: &Path) -> Result<Vec<u8>, &'static str> {
    use std::io::Read as _;
    let file =
        std::fs::File::open(path).map_err(|_| PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL)?;
    let mut bytes = Vec::new();
    file.take(PIPELINE_ADMIN_BODY_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL)?;
    if bytes.len() > PIPELINE_ADMIN_BODY_MAX_BYTES {
        return Err(PIPELINE_DEFAULT_ROUTING_RESULTS_INVALID_LABEL);
    }
    Ok(bytes)
}

/// Reads the results directory: the signed package and every
/// `*.attestation.json`, in file-name order, each bounded, at most
/// `PIPELINE_MAX_ATTESTATIONS` of them.
fn read_results_dir(
    dir: &Path,
) -> Result<(SignedBundlePackage, Vec<PipelineCheckAttestation>), &'static str> {
    let package_bytes = read_bounded(&dir.join(DEFAULT_ROUTING_PACKAGE_FILE))?;
    let signed_package: SignedBundlePackage = serde_json::from_slice(&package_bytes)
        .map_err(|_| PIPELINE_DEFAULT_ROUTING_RESULTS_INVALID_LABEL)?;
    let mut paths = std::fs::read_dir(dir)
        .map_err(|_| PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(DEFAULT_ROUTING_ATTESTATION_SUFFIX))
        })
        .collect::<Vec<_>>();
    if paths.len() > PIPELINE_MAX_ATTESTATIONS {
        return Err(PIPELINE_DEFAULT_ROUTING_RESULTS_INVALID_LABEL);
    }
    paths.sort();
    let attestations = paths
        .iter()
        .map(|path| {
            let bytes = read_bounded(path)?;
            serde_json::from_slice::<PipelineCheckAttestation>(&bytes)
                .map_err(|_| PIPELINE_DEFAULT_ROUTING_RESULTS_INVALID_LABEL)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((signed_package, attestations))
}

/// Verifies `signed_package` and `attestations` the way the admin routes
/// do, at `now`, for this process: the package against the package trust
/// store, the results against the check trust store, the promotion over
/// them (ready, and naming this process's revision and this package), the
/// package's dependency profile on this process's infrastructure, and the
/// startup checks of a tenant bundle on it. `Err` is a safe label.
pub(crate) fn verify_armed_set(
    state: &AppState,
    signed_package: SignedBundlePackage,
    attestations: Vec<PipelineCheckAttestation>,
    now: DateTime<Utc>,
) -> Result<ArmedSet, String> {
    let service = state
        .pipeline_service
        .as_deref()
        .ok_or_else(|| PIPELINE_DEFAULT_ROUTING_REQUIRES_PRODUCTION_RUNTIME_LABEL.to_string())?;
    let trust = trust_and_revision(state).map_err(|(_, error)| error.0.error)?;
    if attestations.is_empty() {
        return Err(PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL.to_string());
    }
    trust.package.verify(&signed_package)?;
    let verified = trust.check.verify_all(&attestations)?;
    let promotion = evaluate_promotion(&verified.evidence, now)?;
    if promotion.code_revision_hash.as_deref() != Some(trust.revision) {
        return Err(PIPELINE_DEFAULT_ROUTING_REVISION_MISMATCH_LABEL.to_string());
    }
    if !promotion.ready {
        let stale = promotion
            .safe_blockers
            .iter()
            .any(|blocker| blocker.starts_with(QUALIFICATION_EVIDENCE_STALE_LABEL));
        return Err(if stale {
            PIPELINE_DEFAULT_ROUTING_RESULTS_STALE_LABEL
        } else {
            QUALIFICATION_PROMOTION_NOT_READY_LABEL
        }
        .to_string());
    }
    let digests = package_digests(&signed_package.package)?;
    if !promotion
        .package
        .as_ref()
        .is_some_and(|package| package.is(&digests))
    {
        return Err(PIPELINE_DEFAULT_ROUTING_PACKAGE_MISMATCH_LABEL.to_string());
    }
    ProductionDependencyProfile::for_bundle(
        service,
        &signed_package.package,
        route_infrastructure_profile(state),
    )?;
    service.check_runnable_package(&signed_package.package, &state.pipeline_main_gate, true)?;
    let expires_at = verified
        .evidence
        .iter()
        .filter_map(|item| {
            i64::try_from(item.maximum_age_seconds)
                .ok()
                .and_then(chrono::Duration::try_seconds)
                .and_then(|age| item.check.observed_at.checked_add_signed(age))
        })
        .min()
        .ok_or_else(|| PIPELINE_DEFAULT_ROUTING_RESULTS_INVALID_LABEL.to_string())?;
    let set_digest = sha256_prefixed(&format!(
        "{}\n{}",
        signed_package.signature.package_hash, promotion.evidence_hash
    ));
    Ok(ArmedSet {
        bundle_id: signed_package.package.bundle_id.clone(),
        signed_package,
        attestations,
        expires_at,
        set_digest,
    })
}

/// `read_results_dir` then `verify_armed_set`: the arming of this process
/// from `dir` at `now`.
pub(crate) async fn load_arming(state: &AppState, dir: &Path, now: DateTime<Utc>) -> Arming {
    let dir = dir.to_path_buf();
    let read = tokio::task::spawn_blocking(move || read_results_dir(&dir))
        .await
        .unwrap_or(Err(PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL));
    let (signed_package, attestations) = match read {
        Ok(read) => read,
        Err(label) => {
            return Arming::Disarmed {
                label: label.to_string(),
            };
        }
    };
    match verify_armed_set(state, signed_package, attestations, now) {
        Ok(set) => Arming::Armed(Arc::new(set)),
        Err(label) => Arming::Disarmed {
            // A verifier's refusal is a safe label already; anything else
            // is not echoed.
            label: if super::pipeline_activation::is_refusal_label(&label) {
                label
            } else {
                PIPELINE_DEFAULT_ROUTING_RESULTS_INVALID_LABEL.to_string()
            },
        },
    }
}

/// What `default_route_tenant` did for one tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DefaultRouteOutcome {
    /// The process is not armed (or the armed set expired): nothing done.
    NotArmed,
    /// The tenant's last refusal is less than `DEFAULT_ROUTING_BACKOFF` old.
    BackedOff,
    /// The tenant had no routing row and is now routed to the pipeline.
    Activated,
    /// The tenant has a routing row (an operator's, or another process's
    /// activation): left as it is.
    AlreadyDecided,
    /// The qualification or the activation refused, with its label.
    Refused(String),
}

/// The label of an internal error (an answer that is not a refusal label,
/// such as `internal_error`'s fixed text) in a WARN line and in an outcome.
pub(crate) const PIPELINE_DEFAULT_ROUTING_INTERNAL_ERROR_LABEL: &str =
    "pipeline_default_routing_internal_error";

/// `label` when it is a refusal label, else the internal error's label.
fn safe_refusal_label(label: String) -> String {
    if super::pipeline_activation::is_refusal_label(&label) {
        label
    } else {
        PIPELINE_DEFAULT_ROUTING_INTERNAL_ERROR_LABEL.to_string()
    }
}

/// A refusal of the qualification or the activation, as a safe label.
fn refused(label: String) -> DefaultRouteOutcome {
    DefaultRouteOutcome::Refused(safe_refusal_label(label))
}

/// The counts of one pass of the loop, reported by config-status.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub(crate) struct DefaultRoutingPassReport {
    pub(crate) activated: u64,
    pub(crate) refused: u64,
    pub(crate) skipped: u64,
    pub(crate) requalified: u64,
    pub(crate) duration_ms: u64,
}

impl DefaultRoutingPassReport {
    fn count(&mut self, outcome: &DefaultRouteOutcome) {
        match outcome {
            DefaultRouteOutcome::Activated => self.activated += 1,
            DefaultRouteOutcome::Refused(_) => self.refused += 1,
            DefaultRouteOutcome::NotArmed
            | DefaultRouteOutcome::BackedOff
            | DefaultRouteOutcome::AlreadyDecided => self.skipped += 1,
        }
    }
}

/// The tenants whose routing row is `pipeline` or `contained` and whose
/// bundles passed the start checks in this process.
#[derive(Debug, Default)]
struct RoutedTenantCache {
    tenants: BTreeSet<String>,
    refreshed_at: Option<tokio::time::Instant>,
}

/// The state of the mode `all` in one process. `AppState` holds it as
/// `pipeline_default_routing` (`None` without the mode).
#[derive(Debug)]
pub(crate) struct PipelineDefaultRouting {
    config: PipelineDefaultRoutingConfig,
    arming: std::sync::RwLock<Arming>,
    cache: std::sync::RwLock<RoutedTenantCache>,
    backoff: std::sync::Mutex<HashMap<String, tokio::time::Instant>>,
    tenant_locks: std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    route_cursor: std::sync::Mutex<Option<String>>,
    requalify_cursor: std::sync::Mutex<Option<String>>,
    last_pass: std::sync::Mutex<Option<DefaultRoutingPassReport>>,
    /// Where `load_arming` reads the clock: the wall clock, or a test's.
    #[cfg(test)]
    now_override: std::sync::Mutex<Option<DateTime<Utc>>>,
    /// Test builds only: the tenants the loop may act on. The suites share
    /// one database, and a loop that activated another suite's fixture
    /// tenant would change that suite's routing.
    #[cfg(test)]
    tenant_scope: std::sync::Mutex<Option<BTreeSet<String>>>,
}

/// The `config-status` fields of default routing.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub(crate) struct DefaultRoutingStatus {
    pub(crate) mode: &'static str,
    pub(crate) armed: bool,
    pub(crate) label: Option<String>,
    pub(crate) expires_in_seconds: Option<i64>,
    pub(crate) last_pass: Option<DefaultRoutingPassReport>,
    pub(crate) routed_tenant_count: usize,
}

impl DefaultRoutingStatus {
    /// The fields of a process without the mode.
    pub(crate) fn off() -> Self {
        Self {
            mode: "off",
            armed: false,
            label: None,
            expires_in_seconds: None,
            last_pass: None,
            routed_tenant_count: 0,
        }
    }
}

fn lock_poisoned<T>(error: std::sync::PoisonError<T>) -> T {
    error.into_inner()
}

impl PipelineDefaultRouting {
    pub(crate) fn new(config: PipelineDefaultRoutingConfig) -> Self {
        Self {
            config,
            arming: std::sync::RwLock::new(Arming::Disarmed {
                label: PIPELINE_DEFAULT_ROUTING_NOT_LOADED_LABEL.to_string(),
            }),
            cache: std::sync::RwLock::default(),
            backoff: std::sync::Mutex::default(),
            tenant_locks: std::sync::Mutex::default(),
            route_cursor: std::sync::Mutex::default(),
            requalify_cursor: std::sync::Mutex::default(),
            last_pass: std::sync::Mutex::default(),
            #[cfg(test)]
            now_override: std::sync::Mutex::default(),
            #[cfg(test)]
            tenant_scope: std::sync::Mutex::default(),
        }
    }

    pub(crate) fn config(&self) -> &PipelineDefaultRoutingConfig {
        &self.config
    }

    fn now(&self) -> DateTime<Utc> {
        #[cfg(test)]
        if let Some(now) = *self.now_override.lock().unwrap_or_else(lock_poisoned) {
            return now;
        }
        Utc::now()
    }

    /// Test builds only: the loop acts on `tenants` only.
    #[cfg(test)]
    pub(crate) fn restrict_loop_for_test(&self, tenants: BTreeSet<String>) {
        *self.tenant_scope.lock().unwrap_or_else(lock_poisoned) = Some(tenants);
    }

    /// Whether the loop may act on `tenant_id`: always, but in a test that
    /// restricted it.
    fn loop_may_act_on(&self, tenant_id: &str) -> bool {
        #[cfg(test)]
        if let Some(scope) = &*self.tenant_scope.lock().unwrap_or_else(lock_poisoned) {
            return scope.contains(tenant_id);
        }
        let _ = tenant_id;
        true
    }

    /// Test builds only: reads `now` in place of the wall clock.
    #[cfg(test)]
    pub(crate) fn set_now_for_test(&self, now: Option<DateTime<Utc>>) {
        *self.now_override.lock().unwrap_or_else(lock_poisoned) = now;
    }

    /// The armed set, when there is one and it has not expired.
    pub(crate) fn armed_set(&self) -> Option<Arc<ArmedSet>> {
        match &*self.arming.read().unwrap_or_else(lock_poisoned) {
            Arming::Armed(set) if self.now() < set.expires_at => Some(set.clone()),
            _ => None,
        }
    }

    pub(crate) fn is_armed(&self) -> bool {
        self.armed_set().is_some()
    }

    /// The refusal label of an unarmed process (`None` when armed).
    pub(crate) fn arming_label(&self) -> Option<String> {
        match &*self.arming.read().unwrap_or_else(lock_poisoned) {
            Arming::Armed(set) if self.now() >= set.expires_at => {
                Some(PIPELINE_DEFAULT_ROUTING_RESULTS_STALE_LABEL.to_string())
            }
            arming => arming.label().map(str::to_string),
        }
    }

    /// Reads the results directory again and replaces the arming. Logs one
    /// line when the arming changed: a WARN with the label when it is not
    /// armed, an INFO with the set digest when it is.
    pub(crate) async fn rearm(&self, state: &AppState) {
        let arming = load_arming(state, &self.config.results_dir, self.now()).await;
        let mut current = self.arming.write().unwrap_or_else(lock_poisoned);
        let changed = current.label() != arming.label() || current.digest() != arming.digest();
        if changed {
            match &arming {
                Arming::Disarmed { label } => tracing::warn!(
                    label = %label,
                    "pipeline default routing is not armed: no tenant is activated"
                ),
                Arming::Armed(set) => tracing::info!(
                    set_digest = %set.set_digest,
                    "pipeline default routing is armed"
                ),
            }
        }
        *current = arming;
    }

    /// Whether this process serves `tenant_id` through the cache.
    pub(crate) fn serves(&self, tenant_id: &str) -> bool {
        self.cache
            .read()
            .unwrap_or_else(lock_poisoned)
            .tenants
            .contains(tenant_id)
    }

    /// The cached tenants, for the worker's list.
    pub(crate) fn routed_tenants(&self) -> BTreeSet<String> {
        self.cache
            .read()
            .unwrap_or_else(lock_poisoned)
            .tenants
            .clone()
    }

    /// Runs the start checks of `tenant_id`'s bundles in this process and,
    /// when they pass, admits it to the cache; a failure removes it and
    /// WARNs. Returns whether it was admitted.
    pub(crate) async fn admit(
        &self,
        state: &AppState,
        service: &PipelineService,
        tenant_id: &str,
    ) -> bool {
        let checked = service
            .check_tenant_bundles(tenant_id, &state.pipeline_main_gate, true)
            .await;
        let mut cache = self.cache.write().unwrap_or_else(lock_poisoned);
        match checked {
            Ok(()) => {
                cache.tenants.insert(tenant_id.to_string());
                true
            }
            Err(error) => {
                cache.tenants.remove(tenant_id);
                tracing::warn!(
                    tenant_storage_ref = %tenant_storage_ref(tenant_id),
                    error_hash = %safe_display_error_hash(&error),
                    "{PIPELINE_DEFAULT_ROUTING_TENANT_BUNDLE_CHECK_FAILED_LABEL}"
                );
                false
            }
        }
    }

    fn backed_off(&self, tenant_id: &str) -> bool {
        let mut backoff = self.backoff.lock().unwrap_or_else(lock_poisoned);
        match backoff.get(tenant_id) {
            Some(until) if tokio::time::Instant::now() < *until => true,
            Some(_) => {
                backoff.remove(tenant_id);
                false
            }
            None => false,
        }
    }

    fn back_off(&self, tenant_id: &str) {
        self.backoff.lock().unwrap_or_else(lock_poisoned).insert(
            tenant_id.to_string(),
            tokio::time::Instant::now() + DEFAULT_ROUTING_BACKOFF,
        );
    }

    fn clear_backoff(&self, tenant_id: &str) {
        self.backoff
            .lock()
            .unwrap_or_else(lock_poisoned)
            .remove(tenant_id);
    }

    fn tenant_lock(&self, tenant_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.tenant_locks
            .lock()
            .unwrap_or_else(lock_poisoned)
            .entry(tenant_id.to_string())
            .or_default()
            .clone()
    }

    fn release_tenant_lock(&self, tenant_id: &str, lock: Arc<tokio::sync::Mutex<()>>) {
        let mut locks = self.tenant_locks.lock().unwrap_or_else(lock_poisoned);
        // The map's own reference and this one: nobody else waits on it.
        if Arc::strong_count(&lock) <= 2 {
            locks.remove(tenant_id);
        }
    }

    /// Routes `tenant_id` to the pipeline when it has no routing row and this
    /// process is armed: the shared qualification, then the shared
    /// activation with the expectation "no routing row", both under the
    /// system actor. One caller at a time for a tenant in this process; the
    /// routing lock orders callers across processes.
    pub(crate) async fn default_route_tenant(
        &self,
        state: &AppState,
        tenant_id: &str,
    ) -> DefaultRouteOutcome {
        let Some(armed) = self.armed_set() else {
            return DefaultRouteOutcome::NotArmed;
        };
        if self.backed_off(tenant_id) {
            return DefaultRouteOutcome::BackedOff;
        }
        let lock = self.tenant_lock(tenant_id);
        let outcome = {
            let _guard = lock.lock().await;
            self.route_locked(state, tenant_id, &armed).await
        };
        self.release_tenant_lock(tenant_id, lock);
        match &outcome {
            DefaultRouteOutcome::Activated => self.clear_backoff(tenant_id),
            DefaultRouteOutcome::Refused(label) => {
                self.back_off(tenant_id);
                tracing::warn!(
                    tenant_storage_ref = %tenant_storage_ref(tenant_id),
                    label = %label,
                    "{PIPELINE_DEFAULT_ROUTING_ACTIVATION_REFUSED_LABEL}"
                );
            }
            _ => {}
        }
        outcome
    }

    async fn route_locked(
        &self,
        state: &AppState,
        tenant_id: &str,
        armed: &ArmedSet,
    ) -> DefaultRouteOutcome {
        let Some(activation) = state.pipeline_activation.as_deref() else {
            return DefaultRouteOutcome::Refused(
                super::pipeline_activation::PIPELINE_ROUTING_STORE_MISSING_LABEL.to_string(),
            );
        };
        // A row that is already there (an operator's decision, or a caller
        // that held this lock before) is left alone before anything is
        // written; the activation's own check under the routing lock is the
        // one that decides.
        match activation
            .routing_for_new_receipt(tenant_id, state.pipeline_code_revision_hash.as_deref())
            .await
        {
            Ok(read) if read.routing.is_some() => return DefaultRouteOutcome::AlreadyDecided,
            Ok(_) => {}
            Err(_) => {
                return DefaultRouteOutcome::Refused(
                    trace_commons_server::versioned_pipeline_activation::PIPELINE_ROUTING_UNAVAILABLE_LABEL
                        .to_string(),
                );
            }
        }
        let actor = system_audit_tenant(tenant_id, DEFAULT_ROUTING_ACTOR_REF);
        if let Err((_, error)) =
            qualify_for_tenant(state, &actor, &armed.signed_package, &armed.attestations).await
        {
            // A qualification that committed and whose audit row failed is
            // in place: the activation goes on.
            if error.0.error != PIPELINE_CHANGE_COMMITTED_AUDIT_FAILED_LABEL {
                return refused(error.0.error);
            }
        }
        let body = ActivateBody::for_tenant_with_no_row(
            armed.bundle_id.clone(),
            PIPELINE_DEFAULT_ROUTING_REASON_CODE,
            armed.attestations.clone(),
        );
        match activate_for_tenant(state, &actor, &body).await {
            Ok(_) => DefaultRouteOutcome::Activated,
            Err((_, error)) if error.0.error == PIPELINE_CHANGE_COMMITTED_AUDIT_FAILED_LABEL => {
                DefaultRouteOutcome::Activated
            }
            Err((_, error)) if error.0.error == PIPELINE_ROUTING_STATE_CHANGED_LABEL => {
                DefaultRouteOutcome::AlreadyDecided
            }
            Err((_, error)) => refused(error.0.error),
        }
    }

    /// One pass of the loop: re-arm, route a batch of tenants with no row,
    /// re-qualify a batch of tenants this mode routed whose active bundle has
    /// no qualification on this revision, and refresh the cache when its TTL
    /// ran out. Returns the pass's counts, which config-status reports.
    pub(crate) async fn run_pass(&self, state: &AppState) -> DefaultRoutingPassReport {
        let started = std::time::Instant::now();
        let mut report = DefaultRoutingPassReport::default();
        self.rearm(state).await;
        if let Some(armed) = self.armed_set() {
            let left = armed.expires_at - self.now();
            if left.num_seconds() <= EXPIRY_WARNING_SECONDS {
                tracing::warn!(
                    expires_in_seconds = left.num_seconds(),
                    "{PIPELINE_DEFAULT_ROUTING_RESULTS_EXPIRING_LABEL}"
                );
            }
            self.route_batch(state, &mut report).await;
            self.requalify_batch(state, &armed, &mut report).await;
        }
        self.refresh_cache_if_due(state).await;
        report.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        *self.last_pass.lock().unwrap_or_else(lock_poisoned) = Some(report.clone());
        report
    }

    async fn route_batch(&self, state: &AppState, report: &mut DefaultRoutingPassReport) {
        let Some(activation) = state.pipeline_activation.as_deref() else {
            return;
        };
        let after = self
            .route_cursor
            .lock()
            .unwrap_or_else(lock_poisoned)
            .clone();
        let page = match activation
            .unrouted_tenants(after.as_deref(), self.config.batch)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(
                    error_hash = %safe_display_error_hash(&error),
                    "{PIPELINE_DEFAULT_ROUTING_ENUMERATION_FAILED_LABEL}"
                );
                return;
            }
        };
        // A short page is the end of the list: the next pass starts over.
        let next = (page.len() as i64 >= self.config.batch)
            .then(|| page.last().cloned())
            .flatten();
        for tenant_id in page
            .iter()
            .filter(|tenant_id| self.loop_may_act_on(tenant_id))
        {
            let outcome = self.default_route_tenant(state, tenant_id).await;
            report.count(&outcome);
        }
        *self.route_cursor.lock().unwrap_or_else(lock_poisoned) = next;
    }

    async fn requalify_batch(
        &self,
        state: &AppState,
        armed: &ArmedSet,
        report: &mut DefaultRoutingPassReport,
    ) {
        let (Some(activation), Some(revision)) = (
            state.pipeline_activation.as_deref(),
            state.pipeline_code_revision_hash.as_deref(),
        ) else {
            return;
        };
        let after = self
            .requalify_cursor
            .lock()
            .unwrap_or_else(lock_poisoned)
            .clone();
        let page = match activation
            .routed_tenants(after.as_deref(), self.config.batch)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                tracing::warn!(
                    error_hash = %safe_display_error_hash(&error),
                    "{PIPELINE_DEFAULT_ROUTING_ENUMERATION_FAILED_LABEL}"
                );
                return;
            }
        };
        let next = (page.len() as i64 >= self.config.batch)
            .then(|| page.last().map(|(tenant_id, _)| tenant_id.clone()))
            .flatten();
        for (tenant_id, _) in page
            .iter()
            .filter(|(tenant_id, _)| self.loop_may_act_on(tenant_id))
        {
            let Ok(view) = activation.routing_view(tenant_id, 1, Some(revision)).await else {
                continue;
            };
            // Owner decision (PR body): only a tenant whose routing row this
            // mode wrote is re-qualified. A tenant an operator activated by
            // hand, or whose row an operator changed since (a contain), keeps
            // the admin routes' behaviour.
            let ours = view.routing.as_ref().is_some_and(|routing| {
                routing.actor_principal_ref == DEFAULT_ROUTING_ACTOR_REF
                    && routing.reason_code == PIPELINE_DEFAULT_ROUTING_REASON_CODE
            });
            if !ours || view.active_bundle_qualified_on_revision != Some(false) {
                continue;
            }
            if view.active_bundle_id.as_deref() != Some(armed.bundle_id.as_str()) {
                tracing::warn!(
                    tenant_storage_ref = %tenant_storage_ref(tenant_id),
                    "{PIPELINE_DEFAULT_ROUTING_REQUALIFY_BUNDLE_DIFFERS_LABEL}"
                );
                report.skipped += 1;
                continue;
            }
            let actor = system_audit_tenant(tenant_id, DEFAULT_ROUTING_ACTOR_REF);
            match qualify_for_tenant(state, &actor, &armed.signed_package, &armed.attestations)
                .await
            {
                Ok(_) => report.requalified += 1,
                Err((_, error))
                    if error.0.error == PIPELINE_CHANGE_COMMITTED_AUDIT_FAILED_LABEL =>
                {
                    report.requalified += 1;
                }
                Err((_, error)) => {
                    report.refused += 1;
                    tracing::warn!(
                        tenant_storage_ref = %tenant_storage_ref(tenant_id),
                        label = %safe_refusal_label(error.0.error),
                        "{PIPELINE_DEFAULT_ROUTING_ACTIVATION_REFUSED_LABEL}"
                    );
                }
            }
        }
        *self.requalify_cursor.lock().unwrap_or_else(lock_poisoned) = next;
    }

    async fn refresh_cache_if_due(&self, state: &AppState) {
        let due = self
            .cache
            .read()
            .unwrap_or_else(lock_poisoned)
            .refreshed_at
            .is_none_or(|at| at.elapsed() >= ROUTED_TENANT_CACHE_TTL);
        if due {
            self.refresh_cache(state).await;
        }
    }

    /// Reads every `pipeline` or `contained` tenant and rebuilds the cache:
    /// a tenant new to it is admitted only once its bundles pass the start
    /// checks here; a tenant no longer routed (deactivated to legacy) drops
    /// out. A read that fails keeps the cache as it is.
    pub(crate) async fn refresh_cache(&self, state: &AppState) {
        let (Some(activation), Some(service)) = (
            state.pipeline_activation.as_deref(),
            state.pipeline_service.as_deref(),
        ) else {
            return;
        };
        let mut routed = BTreeSet::new();
        let mut after: Option<String> = None;
        loop {
            let page = match activation
                .routed_tenants(after.as_deref(), ROUTED_PAGE)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    tracing::warn!(
                        error_hash = %safe_display_error_hash(&error),
                        "{PIPELINE_DEFAULT_ROUTING_ENUMERATION_FAILED_LABEL}"
                    );
                    return;
                }
            };
            let full = page.len() as i64 >= ROUTED_PAGE;
            after = page.last().map(|(tenant_id, _)| tenant_id.clone());
            routed.extend(
                page.into_iter()
                    .map(|(tenant_id, _)| tenant_id)
                    .filter(|tenant_id| self.loop_may_act_on(tenant_id)),
            );
            if !full {
                break;
            }
        }
        let known = self.routed_tenants();
        let mut admitted = BTreeSet::new();
        for tenant_id in routed {
            if known.contains(&tenant_id) {
                admitted.insert(tenant_id);
                continue;
            }
            match service
                .check_tenant_bundles(&tenant_id, &state.pipeline_main_gate, true)
                .await
            {
                Ok(()) => {
                    admitted.insert(tenant_id);
                }
                Err(error) => tracing::warn!(
                    tenant_storage_ref = %tenant_storage_ref(&tenant_id),
                    error_hash = %safe_display_error_hash(&error),
                    "{PIPELINE_DEFAULT_ROUTING_TENANT_BUNDLE_CHECK_FAILED_LABEL}"
                ),
            }
        }
        let mut cache = self.cache.write().unwrap_or_else(lock_poisoned);
        cache.tenants = admitted;
        cache.refreshed_at = Some(tokio::time::Instant::now());
    }

    /// The config-status fields.
    pub(crate) fn status(&self) -> DefaultRoutingStatus {
        let armed = self.armed_set();
        DefaultRoutingStatus {
            mode: "all",
            armed: armed.is_some(),
            label: self.arming_label(),
            expires_in_seconds: armed.map(|set| (set.expires_at - self.now()).num_seconds()),
            last_pass: self.last_pass.lock().unwrap_or_else(lock_poisoned).clone(),
            routed_tenant_count: self
                .cache
                .read()
                .unwrap_or_else(lock_poisoned)
                .tenants
                .len(),
        }
    }
}

/// Whether `tenant_id` is on this process's receipts list
/// (`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS`).
fn on_receipts_list(state: &AppState, tenant_id: &str) -> bool {
    state.tenant_rollout_gates.enabled_for(
        TraceTenantRolloutFeature::PipelineReceipts,
        false,
        tenant_id,
    )
}

/// Whether this process may activate (or roll back) `tenant_id`: it is on
/// the receipts list, or the mode `all` is configured. The scope check of
/// the activation gate (`pipeline_tenant_not_in_scope`).
pub(crate) fn pipeline_tenant_in_activation_scope(state: &AppState, tenant_id: &str) -> bool {
    on_receipts_list(state, tenant_id) || state.pipeline_default_routing.is_some()
}

/// Whether this process serves `tenant_id`'s pipeline receipts and drains
/// its runs: it is on the receipts list, or default routing admitted it to
/// the routed-tenant cache (its bundles passed the start checks here).
pub(crate) fn pipeline_tenant_served(state: &AppState, tenant_id: &str) -> bool {
    on_receipts_list(state, tenant_id)
        || state
            .pipeline_default_routing
            .as_deref()
            .is_some_and(|routing| routing.serves(tenant_id))
}

/// The upload path's step before the route decision: when this process is
/// armed, the upload is not a remediation, and the tenant has no routing
/// row, routes the tenant to the pipeline first, so a new signup's first
/// upload is a pipeline receipt. Returns the routing the decision uses:
/// `routing` when nothing changed, the routing read again after an
/// activation (or after another process's), or `None` when a read failed,
/// so that `decide_upload_route` reads it and answers its own refusal.
/// A refusal never fails the upload: it goes on with its current routing.
pub(crate) async fn route_before_first_upload(
    state: &AppState,
    tenant_id: &str,
    remediating: bool,
    routing: Option<NewReceiptRouting>,
) -> Option<NewReceiptRouting> {
    let Some(default_routing) = state.pipeline_default_routing.as_deref() else {
        return routing;
    };
    if remediating || default_routing.serves(tenant_id) || !default_routing.is_armed() {
        return routing;
    }
    let (Some(activation), Some(service)) = (
        state.pipeline_activation.as_deref(),
        state.pipeline_service.as_deref(),
    ) else {
        return routing;
    };
    let revision = state.pipeline_code_revision_hash.as_deref();
    let routing = match routing {
        Some(routing) => routing,
        None => match activation
            .routing_for_new_receipt(tenant_id, revision)
            .await
        {
            Ok(routing) => routing,
            Err(_) => return None,
        },
    };
    if routing.routing.is_some() {
        return Some(routing);
    }
    match default_routing.default_route_tenant(state, tenant_id).await {
        DefaultRouteOutcome::Activated | DefaultRouteOutcome::AlreadyDecided => {}
        _ => return Some(routing),
    }
    let routing = match activation
        .routing_for_new_receipt(tenant_id, revision)
        .await
    {
        Ok(routing) => routing,
        Err(_) => return None,
    };
    // Another process may have activated the tenant: admit it here too
    // before the decision, so the upload is not refused as not served.
    let routed = routing.routing.as_ref().is_some_and(|row| {
        matches!(
            row.routing_state,
            RoutingState::Pipeline | RoutingState::Contained
        )
    });
    if routed && !default_routing.serves(tenant_id) {
        default_routing.admit(state, service, tenant_id).await;
    }
    Some(routing)
}

/// The loop of the mode `all`, beside the worker: one `run_pass` every
/// `interval`, until `stop` fires.
pub(crate) struct DefaultRoutingLoopHandle {
    pub(crate) stop: tokio::sync::watch::Sender<bool>,
    pub(crate) join: tokio::task::JoinHandle<()>,
}

/// Starts the loop when the mode is configured.
pub(crate) fn spawn_default_routing_loop(state: Arc<AppState>) -> Option<DefaultRoutingLoopHandle> {
    let routing = state.pipeline_default_routing.clone()?;
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    let join = tokio::spawn(async move {
        while !*stop_rx.borrow() {
            routing.run_pass(&state).await;
            tokio::select! {
                _ = tokio::time::sleep(routing.config().interval) => {},
                _ = stop_rx.changed() => {},
            }
        }
    });
    Some(DefaultRoutingLoopHandle {
        stop: stop_tx,
        join,
    })
}

/// The start's first look, before the listener opens: the arming and the
/// cache, within `limit`. A tenant the refresh does not reach is admitted on
/// a later pass; nothing here refuses the start.
pub(crate) async fn prepare_default_routing(state: &AppState, limit: StdDuration) {
    let Some(routing) = state.pipeline_default_routing.as_deref() else {
        return;
    };
    let prepared = tokio::time::timeout(limit, async {
        routing.rearm(state).await;
        routing.refresh_cache(state).await;
    })
    .await;
    if prepared.is_err() {
        tracing::warn!(
            timed_out = true,
            "{}",
            super::pipeline_activation::PIPELINE_QUALIFICATION_START_CHECK_INCOMPLETE_LABEL
        );
    }
}

/// Counters of the worker's last pass, read by `GET /v1/pipeline/readiness`.
#[derive(Debug, Default)]
pub(crate) struct PipelineWorkerPassStats {
    pub(crate) tenant_count: AtomicU64,
    pub(crate) duration_ms: AtomicU64,
}

impl PipelineWorkerPassStats {
    pub(crate) fn record(&self, tenant_count: usize, duration: StdDuration) {
        self.tenant_count.store(
            u64::try_from(tenant_count).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
        self.duration_ms.store(
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn the_actor_ref_is_the_principal_ref_of_its_label() {
        assert_eq!(
            DEFAULT_ROUTING_ACTOR_REF,
            principal_storage_ref(DEFAULT_ROUTING_ACTOR_LABEL)
        );
        assert!(DEFAULT_ROUTING_ACTOR_REF.starts_with("principal_sha256:"));
        assert!(
            trace_commons_server::versioned_pipeline_qualification::is_safe_label(
                PIPELINE_DEFAULT_ROUTING_REASON_CODE
            )
        );
    }

    #[test]
    fn the_mode_is_off_unless_it_says_all() {
        for mode in [None, Some(""), Some("off"), Some("  off ")] {
            assert_eq!(
                parse_default_routing_config(mode, None, Some("x"), Some("y")).unwrap(),
                None,
                "{mode:?} is off, whatever the other variables say"
            );
        }
        let config = parse_default_routing_config(Some("all"), Some("/r"), None, None)
            .unwrap()
            .expect("all is on");
        assert_eq!(config.results_dir, PathBuf::from("/r"));
        assert_eq!(config.interval, StdDuration::from_secs(60));
        assert_eq!(config.batch, 50);
    }

    fn refusal(
        mode: Option<&str>,
        dir: Option<&str>,
        interval: Option<&str>,
        batch: Option<&str>,
    ) -> String {
        parse_default_routing_config(mode, dir, interval, batch)
            .expect_err("refused")
            .to_string()
    }

    #[test]
    fn each_bad_variable_refuses_the_start_with_its_label() {
        assert_eq!(
            refusal(Some("some"), Some("/r"), None, None),
            PIPELINE_DEFAULT_ROUTING_MODE_INVALID_LABEL
        );
        assert_eq!(
            refusal(Some("ALL"), Some("/r"), None, None),
            PIPELINE_DEFAULT_ROUTING_MODE_INVALID_LABEL
        );
        for dir in [None, Some(""), Some("  ")] {
            assert_eq!(
                refusal(Some("all"), dir, None, None),
                PIPELINE_DEFAULT_ROUTING_RESULTS_DIR_MISSING_LABEL
            );
        }
        for interval in ["4", "3601", "x", "-1"] {
            assert_eq!(
                refusal(Some("all"), Some("/r"), Some(interval), None),
                PIPELINE_DEFAULT_ROUTING_CONFIG_INVALID_LABEL
            );
        }
        for batch in ["0", "1001", "x"] {
            assert_eq!(
                refusal(Some("all"), Some("/r"), None, Some(batch)),
                PIPELINE_DEFAULT_ROUTING_CONFIG_INVALID_LABEL
            );
        }
        let edges = parse_default_routing_config(Some("all"), Some("/r"), Some("5"), Some("1000"))
            .unwrap()
            .unwrap();
        assert_eq!(edges.interval, StdDuration::from_secs(5));
        assert_eq!(edges.batch, 1000);
    }

    fn facts() -> DefaultRoutingStartFacts {
        DefaultRoutingStartFacts {
            production_runtime: true,
            trust_stores: true,
            code_revision: true,
            routing_store: true,
            allow_test_dependencies: false,
        }
    }

    #[test]
    fn the_start_refuses_the_mode_without_what_it_needs() {
        let config = parse_default_routing_config(Some("all"), Some("/r"), None, None)
            .unwrap()
            .unwrap();
        let refused = |facts: DefaultRoutingStartFacts| {
            validate_default_routing_start(Some(&config), facts)
                .expect_err("refused")
                .to_string()
        };
        validate_default_routing_start(Some(&config), facts()).expect("everything is in place");
        assert_eq!(
            refused(DefaultRoutingStartFacts {
                production_runtime: false,
                ..facts()
            }),
            PIPELINE_DEFAULT_ROUTING_REQUIRES_PRODUCTION_RUNTIME_LABEL
        );
        assert_eq!(
            refused(DefaultRoutingStartFacts {
                trust_stores: false,
                ..facts()
            }),
            super::super::pipeline_activation::PIPELINE_TRUST_STORE_MISSING_LABEL
        );
        assert_eq!(
            refused(DefaultRoutingStartFacts {
                code_revision: false,
                ..facts()
            }),
            super::super::pipeline_activation::PIPELINE_CODE_REVISION_UNSET_LABEL
        );
        assert_eq!(
            refused(DefaultRoutingStartFacts {
                routing_store: false,
                ..facts()
            }),
            super::super::pipeline_activation::PIPELINE_ROUTING_STORE_MISSING_LABEL
        );
        assert_eq!(
            refused(DefaultRoutingStartFacts {
                allow_test_dependencies: true,
                ..facts()
            }),
            PIPELINE_DEFAULT_ROUTING_TEST_DEPENDENCIES_CONFLICT_LABEL
        );
        // Without the mode nothing is checked.
        validate_default_routing_start(
            None,
            DefaultRoutingStartFacts {
                production_runtime: false,
                trust_stores: false,
                code_revision: false,
                routing_store: false,
                allow_test_dependencies: true,
            },
        )
        .expect("the mode off checks nothing");
    }

    #[tokio::test]
    async fn a_refused_tenant_is_backed_off_and_a_success_clears_it() {
        let routing = PipelineDefaultRouting::new(PipelineDefaultRoutingConfig {
            results_dir: PathBuf::from("/nonexistent"),
            interval: StdDuration::from_secs(60),
            batch: 50,
        });
        assert!(!routing.backed_off("tenant-a"));
        routing.back_off("tenant-a");
        assert!(routing.backed_off("tenant-a"));
        assert!(!routing.backed_off("tenant-b"), "per tenant");
        routing.clear_backoff("tenant-a");
        assert!(!routing.backed_off("tenant-a"));
        routing.back_off("tenant-a");
        tokio::time::pause();
        tokio::time::advance(DEFAULT_ROUTING_BACKOFF + StdDuration::from_secs(1)).await;
        assert!(!routing.backed_off("tenant-a"), "the backoff ends");
    }

    #[test]
    fn a_new_process_is_not_armed_and_says_why() {
        let routing = PipelineDefaultRouting::new(PipelineDefaultRoutingConfig {
            results_dir: PathBuf::from("/nonexistent"),
            interval: StdDuration::from_secs(60),
            batch: 50,
        });
        assert!(!routing.is_armed());
        let status = routing.status();
        assert_eq!(status.mode, "all");
        assert!(!status.armed);
        assert_eq!(
            status.label.as_deref(),
            Some(PIPELINE_DEFAULT_ROUTING_NOT_LOADED_LABEL)
        );
        assert_eq!(status.expires_in_seconds, None);
    }

    #[test]
    fn a_missing_or_unparsable_results_dir_has_its_label() {
        let missing = tempfile::tempdir().unwrap();
        let gone = missing.path().join("absent");
        assert_eq!(
            read_results_dir(&gone).err(),
            Some(PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL)
        );
        std::fs::write(
            missing.path().join(DEFAULT_ROUTING_PACKAGE_FILE),
            b"{not json",
        )
        .unwrap();
        assert_eq!(
            read_results_dir(missing.path()).err(),
            Some(PIPELINE_DEFAULT_ROUTING_RESULTS_INVALID_LABEL)
        );
        std::fs::write(
            missing.path().join(DEFAULT_ROUTING_PACKAGE_FILE),
            vec![b' '; PIPELINE_ADMIN_BODY_MAX_BYTES + 1],
        )
        .unwrap();
        assert_eq!(
            read_results_dir(missing.path()).err(),
            Some(PIPELINE_DEFAULT_ROUTING_RESULTS_INVALID_LABEL),
            "too large"
        );
    }
}
