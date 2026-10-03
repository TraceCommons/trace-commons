// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The admin routes an operator uses to qualify a bundle and to move a
//! tenant between the legacy path and the versioned pipeline (PR 5, P5-D11),
//! and the infrastructure profile the qualification and the activation gate
//! read (P5-D21).
//!
//! Every route starts with `authenticate_with_tenant_access_grant` and
//! `require_admin`, and takes its tenant and its actor from the credential
//! alone: `TenantAuth::tenant_id` and `TenantAuth::principal_ref`. A request
//! body names only a bundle, a reason, signed check results, or (for a
//! qualification) the signed package; each body refuses any other field
//! (`deny_unknown_fields`), so a body that names a tenant is refused before
//! anything is read.
//!
//! The activation gate (`PipelineActivationStore::activate_tenant` and
//! `rollback_bundle`) trusts four values from its caller. These routes are
//! its only caller, and each of the four comes from the server's state,
//! never from the request:
//!
//! - the `PromotionDecision`: `evaluate_promotion` at the time of the call,
//!   over the results `CheckResultTrustStore::verify_all` verified against
//!   the check trust store (`TRACE_COMMONS_PIPELINE_CHECK_TRUSTED_KEYS_PATH`);
//! - the `ActivationReadiness`: from the tenant's operational summary, read
//!   just before the call (activation only; a rollback reads none);
//! - the infrastructure profile: `infrastructure_profile_from_state`;
//! - the runtime code revision: the state's `pipeline_code_revision_hash`,
//!   which is `DEPLOYED_CODE_REVISION_HASH`.
//!
//! Before the store call a route also runs the startup checks of a tenant
//! bundle on the bundle it selects (`PipelineService::check_runnable_package`:
//! `main`'s gate configuration and the pipeline credit issuer), because the
//! gate holds no service. It also refuses a tenant that this process does not
//! list on its receipts list, and a bundle whose qualification records a
//! signing key that the package trust store no longer holds (`gate_inputs`).
//! A qualification goes through `qualify_bundle_attested` only, never the
//! bare `qualify_bundle`.
//!
//! Answers and log lines hold labels, counts, states, times, and hashes. A
//! refusal of these routes' own checks is a safe label
//! (`^[a-z0-9_]{1,64}$`, with a `:<check_id>` suffix for a refusal about one
//! check) built from no request field; a log line names the tenant by
//! `tenant_storage_ref` only. The checks these routes share with `main`'s
//! routes answer as `main`'s routes do, with `main`'s fixed texts, which are
//! not labels: a missing or unknown credential and `require_admin`
//! (`403 admin token required`), and a process with no pipeline runtime
//! (`404 pipeline runtime not configured`, from `require_pipeline_service`
//! and `require_pipeline_product`, before the revision or a trust store is
//! read). None of those texts holds a request field either.

use super::*;

use axum::extract::rejection::QueryRejection;
use trace_commons_gate_api::pipeline::Phase;
use trace_commons_server::versioned_pipeline::{
    PIPELINE_POLICY_INTERVENTION_BUSY_LABEL, PipelinePolicyInterventionRecord,
    PolicyOperationalStatus, is_bundle_id,
};
use trace_commons_server::versioned_pipeline_activation::{
    ActivationEvent, ActivationReadiness, ActivationRequest, LEGACY_DRAIN_REPORT_TIMEOUT_LABEL,
    LegacyDrainReport, PIPELINE_ROUTING_BUSY_LABEL, RoutingState, TenantRouting,
};
use trace_commons_server::versioned_pipeline_bundle::PIPELINE_BUNDLE_INVALID_LABEL;
use trace_commons_server::versioned_pipeline_qualification::{
    ACTIVATION_PROMOTION_NOT_READY_LABEL, BundleQualificationRecord,
    PACKAGE_RUNTIME_REVISION_UNKNOWN_LABEL, PACKAGE_SIGNER_UNTRUSTED_LABEL,
    PipelineCheckAttestation, ProductionAdapterKind, ProductionDependencyProfile,
    ProductionInfrastructureProfile, PromotionDecision, QUALIFICATION_PROMOTION_NOT_READY_LABEL,
    SignedBundlePackage, TrustedBundleKey, evaluate_promotion, is_safe_label,
};

/// The JSON file of the keys whose signatures make a bundle package trusted:
/// an array of `TrustedBundleKey` (`{"key_id", "public_key_base64url"}`),
/// the format `pipeline.py keygen` writes. Unset or empty: no package is
/// trusted, and the qualification and activation routes refuse with
/// `pipeline_trust_store_missing`.
pub(crate) const TRACE_COMMONS_PIPELINE_PACKAGE_TRUSTED_KEYS_PATH: &str =
    "TRACE_COMMONS_PIPELINE_PACKAGE_TRUSTED_KEYS_PATH";
/// The JSON file of the keys whose signatures make a check result count, in
/// the same format. No key may also be a package key
/// (`pipeline_trust_store_overlap`).
pub(crate) const TRACE_COMMONS_PIPELINE_CHECK_TRUSTED_KEYS_PATH: &str =
    "TRACE_COMMONS_PIPELINE_CHECK_TRUSTED_KEYS_PATH";

/// A trust store variable is set and its file is not a non-empty JSON array
/// of valid trusted keys with distinct ids: the start is refused.
pub(crate) const PIPELINE_TRUST_STORE_INVALID_LABEL: &str = "pipeline_trust_store_invalid";
/// A check key is also a package key (the same public key bytes, or the same
/// key id): the start is refused.
pub(crate) const PIPELINE_TRUST_STORE_OVERLAP_LABEL: &str = "pipeline_trust_store_overlap";
/// `503`: a qualification, activation, or rollback in a process without both
/// trust stores.
pub(crate) const PIPELINE_TRUST_STORE_MISSING_LABEL: &str = "pipeline_trust_store_missing";
/// `413`: more than `PIPELINE_MAX_ATTESTATIONS` attestations in one request.
pub(crate) const PIPELINE_EVIDENCE_TOO_LARGE_LABEL: &str = "pipeline_evidence_too_large";
/// A request body or query that does not parse as the route's shape (an
/// unknown field included); the status is the parser's.
pub(crate) const PIPELINE_REQUEST_INVALID_LABEL: &str = "pipeline_request_invalid";
/// `413`: a request body above the limit of these routes
/// (`PIPELINE_ADMIN_BODY_MAX_BYTES`, 1 MiB).
pub(crate) const PIPELINE_REQUEST_TOO_LARGE_LABEL: &str = "pipeline_request_too_large";
/// `409`: the drain report is asked for while the tenant's legacy records in
/// the database are not authoritative (`legacy_records_authoritative`).
pub(crate) const LEGACY_DRAIN_RECORDS_NOT_AUTHORITATIVE_LABEL: &str =
    "legacy_drain_records_not_authoritative";
/// `404`: a process without the database mirror holds no routing store.
pub(crate) const PIPELINE_ROUTING_STORE_MISSING_LABEL: &str = "pipeline_routing_store_missing";
/// `409`: an activation or a rollback of a tenant that this process does not
/// list on its receipts list (`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS`).
pub(crate) const PIPELINE_TENANT_NOT_IN_SCOPE_LABEL: &str = "pipeline_tenant_not_in_scope";
/// `404`: the tenant has no stored package of the bundle the request names
/// (the activation gate's own label for it). `409` with the same label: the
/// stored package no longer validates (a tampered package, BND-002).
const BUNDLE_PACKAGE_MISSING_LABEL: &str = "bundle_package_missing";

/// The most attestations one request may carry: a full qualification set is
/// one for each of the 22 entries of `PROMOTION_REQUIRED_CHECKS` (the 19
/// checks that `qualify` produces and the three promotion-only checks).
pub(crate) const PIPELINE_MAX_ATTESTATIONS: usize = 64;
/// The most routing events `GET /v1/admin/pipeline/routing` answers, newest
/// first.
pub(crate) const PIPELINE_ROUTING_EVENT_LIMIT: usize = 100;
/// The request body limit of the nine routes (fix round 1): 1 MiB, far above
/// a full qualification set (19 attestations and a signed package, about
/// 21 KB in the tests), and far below the router-wide `MAX_INGEST_BODY_BYTES`
/// that an envelope upload needs. Above it a route answers `413`
/// `pipeline_request_too_large`.
pub(crate) const PIPELINE_ADMIN_BODY_MAX_BYTES: usize = 1024 * 1024;

/// The control-plane read audit surfaces of the three read routes, in the
/// form of their siblings' (`pipeline_operational_summary`,
/// `pipeline_forensic_trace`).
const ROUTING_READ_SURFACE: &str = "pipeline_routing";
const POLICY_INTERVENTIONS_READ_SURFACE: &str = "pipeline_policy_interventions";
const LEGACY_DRAIN_READ_SURFACE: &str = "pipeline_legacy_drain";

/// The body limit layer each of the nine routes is registered with
/// (`PIPELINE_ADMIN_BODY_MAX_BYTES`); the router-wide limit stays for every
/// other route.
pub(crate) fn pipeline_admin_body_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(PIPELINE_ADMIN_BODY_MAX_BYTES)
}

/// The provider label of the one artifact store that counts as production.
const GCS_PROVIDER_LABEL: &str = "gcs";
/// The provider label `GET /v1/admin/config-status` reports for a configured
/// store without a typed label (`trace_commons_config_status_response`).
const LOCAL_ENCRYPTED_PROVIDER_LABEL: &str = "local_encrypted";
/// The key wrapper kind of the one key wrapper that counts as production: the
/// Cloud KMS wrapper `build_selected_kek_wrapper_*` builds for
/// `TRACE_COMMONS_KEK_PROVIDER=gcp_cloud_kms`.
const GCP_CLOUD_KMS_WRAPPER_KIND: &str = "gcp_cloud_kms";

/// The two trust stores a process holds, each `None` when its variable is
/// unset.
pub(crate) struct PipelineTrustStores {
    pub(crate) package: Option<Arc<BundlePackageTrustStore>>,
    pub(crate) check: Option<Arc<CheckResultTrustStore>>,
}

/// The trusted keys in the JSON file at `path`; `None` when no path is given.
/// The file must be a non-empty JSON array of `TrustedBundleKey` with
/// distinct key ids; anything else, and a file that cannot be read, is
/// `pipeline_trust_store_invalid`. The error is the label alone: it never
/// names the path.
pub(crate) fn read_trusted_keys(
    path: Option<&Path>,
) -> anyhow::Result<Option<Vec<TrustedBundleKey>>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let invalid = || anyhow::anyhow!(PIPELINE_TRUST_STORE_INVALID_LABEL);
    let bytes = std::fs::read(path).map_err(|_| invalid())?;
    let keys: Vec<TrustedBundleKey> = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
    let mut key_ids = BTreeSet::new();
    if keys.is_empty() || !keys.iter().all(|key| key_ids.insert(key.key_id.as_str())) {
        return Err(invalid());
    }
    Ok(Some(keys))
}

/// `read_trusted_keys` of the file `variable` names: `None` when the variable
/// is unset, or set to the empty string or to blanks (final fix wave G25:
/// `optional_trimmed_env`, as every other optional setting is read, so an
/// empty variable refuses no start). A refusal names the variable, never its
/// value.
pub(crate) fn read_trusted_keys_from_env(
    variable: &'static str,
) -> anyhow::Result<Option<Vec<TrustedBundleKey>>> {
    let path = optional_trimmed_env(variable)?.map(PathBuf::from);
    read_trusted_keys(path.as_deref()).with_context(|| variable.to_string())
}

/// Builds the two trust stores from their keys: a key that a store refuses
/// (an id that is not an identifier, bytes that are not a 32-byte Ed25519
/// public key) is `pipeline_trust_store_invalid`, and a check key that is
/// also a package key is `pipeline_trust_store_overlap`. A key trusted to
/// sign packages is never trusted to vouch for check results, nor the
/// reverse (P5-D13; Task 8 review): the key files have one format and no
/// purpose marker, so a configuration mistake could put one key in both.
pub(crate) fn pipeline_trust_stores(
    package_keys: Option<Vec<TrustedBundleKey>>,
    check_keys: Option<Vec<TrustedBundleKey>>,
) -> anyhow::Result<PipelineTrustStores> {
    let invalid = |_| anyhow::anyhow!(PIPELINE_TRUST_STORE_INVALID_LABEL);
    let package = package_keys
        .clone()
        .map(BundlePackageTrustStore::new)
        .transpose()
        .map_err(invalid)?;
    let check = check_keys
        .clone()
        .map(CheckResultTrustStore::new)
        .transpose()
        .map_err(invalid)?;
    if let (Some(package_keys), Some(check_keys)) = (&package_keys, &check_keys) {
        anyhow::ensure!(
            !trusted_keys_overlap(package_keys, check_keys),
            PIPELINE_TRUST_STORE_OVERLAP_LABEL
        );
    }
    Ok(PipelineTrustStores {
        package: package.map(Arc::new),
        check: check.map(Arc::new),
    })
}

/// Whether a check key is also a package key: its public key bytes are a
/// package key's (under any id), or its id is a package key's. Run after
/// both stores accepted their keys, so every key decodes.
fn trusted_keys_overlap(package: &[TrustedBundleKey], check: &[TrustedBundleKey]) -> bool {
    let public_key = |key: &TrustedBundleKey| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(&key.public_key_base64url)
            .ok()
    };
    let package_ids = package
        .iter()
        .map(|key| key.key_id.as_str())
        .collect::<BTreeSet<_>>();
    let package_public_keys = package
        .iter()
        .filter_map(public_key)
        .collect::<BTreeSet<_>>();
    check.iter().any(|key| {
        package_ids.contains(key.key_id.as_str())
            || public_key(key).is_some_and(|bytes| package_public_keys.contains(&bytes))
    })
}

/// The two trust stores of this process, from
/// `TRACE_COMMONS_PIPELINE_PACKAGE_TRUSTED_KEYS_PATH` and
/// `TRACE_COMMONS_PIPELINE_CHECK_TRUSTED_KEYS_PATH`, read once at start.
pub(crate) fn pipeline_trust_stores_from_env() -> anyhow::Result<PipelineTrustStores> {
    pipeline_trust_stores(
        read_trusted_keys_from_env(TRACE_COMMONS_PIPELINE_PACKAGE_TRUSTED_KEYS_PATH)?,
        read_trusted_keys_from_env(TRACE_COMMONS_PIPELINE_CHECK_TRUSTED_KEYS_PATH)?,
    )
}

/// The start is refused: the build revision is set and is not a `sha256:`
/// digest.
pub(crate) const PIPELINE_CODE_REVISION_INVALID_LABEL: &str = "pipeline_code_revision_invalid";

/// The deployed code revision the routes read, from `value`
/// (`DEPLOYED_CODE_REVISION_HASH`, the build's
/// `TRACE_COMMONS_BUILD_CODE_REVISION_HASH`), checked once at start (final
/// fix wave G17). `None` for a binary built without the variable: the routes
/// then refuse with `bundle_runtime_revision_unknown`. A revision that is set
/// must be `sha256:` and 64 lowercase hex digits, the form `pipeline.py
/// revision` prints and the gate compares; anything else, the empty value of
/// a variable set to nothing included, refuses the start with
/// `pipeline_code_revision_invalid` (the label alone, never the value). A
/// malformed revision would otherwise start cleanly and first show as a
/// refusal after a full signed qualification.
pub(crate) fn deployed_code_revision(value: Option<&str>) -> anyhow::Result<Option<String>> {
    let is_digest = |revision: &str| {
        revision.strip_prefix("sha256:").is_some_and(|hex| {
            hex.len() == 64
                && hex
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    };
    match value {
        None => Ok(None),
        Some(revision) if is_digest(revision) => Ok(Some(revision.to_string())),
        Some(_) => Err(anyhow::anyhow!(PIPELINE_CODE_REVISION_INVALID_LABEL)),
    }
}

/// The infrastructure this process runs around a bundle (P5-D21), from the
/// fields `GET /v1/admin/config-status` reports. Nothing here reads a
/// request. Fail closed: only the production value of each row counts as
/// production, and an unknown value is not production.
///
/// - `authoritative_metadata`: `Production` with the database mirror and its
///   writes required, `Development` with a best-effort mirror, else `Missing`;
///   `best_effort_database_mirror` is the second case.
/// - `artifact_store`: by the configured store's provider label, as
///   config-status reports it (a store with no typed label is
///   `local_encrypted`): `gcs` is `Production`, any other label
///   (`file_system`, `local_encrypted`) `Development`, and no store, or a
///   store whose object IO is off (a remote provider this build does not
///   compile), `Missing`.
/// - `plaintext_fallback`: the configured store allows plaintext
///   compatibility, or no store is configured (the plain file store).
/// - `key_wrapper`: by the configured store's key wrapper kind:
///   `gcp_cloud_kms` is `Production`, any other wrapper (a local master key,
///   the dstack stub) `Development`, none `Missing`.
/// - `authentication`: `Production` with a signed-token verifier and managed
///   EdDSA tokens required, `Development` with a verifier alone, else
///   `Missing`.
/// - `static_bearer_authentication`: any static token is configured.
/// - `hs256_bridge_authentication`: the verifier holds more keys than EdDSA
///   keys (the two counts config-status reports); a verifier whose lock is
///   poisoned counts as holding one.
/// - `unversioned_policy_dependencies`: `false`; the qualification fails
///   closed on a dependency without a content hash
///   (`bundle_dependency_missing`).
/// - `live_external_payout_enabled`: the pipeline runtime holds a payout
///   whose settlement mode moves real money, `PipelineNearSettlementMode::Http`
///   (`TRACE_COMMONS_NEAR_SETTLEMENT_MODE=http`: the injected adapter pays).
///   A payout in the `Disabled` mode (nothing advances) or the `DryRun` mode
///   (an in-process adapter, no network and no funds) is not live, and a
///   runtime with no payout is not either (final fix wave G4).
pub(crate) fn infrastructure_profile_from_state(
    state: &AppState,
) -> ProductionInfrastructureProfile {
    let store = state.artifact_store.as_ref();
    let verifier = state.signed_token_verifier.as_ref();
    ProductionInfrastructureProfile {
        authoritative_metadata: match (state.db_mirror.is_some(), state.require_db_mirror_writes) {
            (true, true) => ProductionAdapterKind::Production,
            (true, false) => ProductionAdapterKind::Development,
            (false, _) => ProductionAdapterKind::Missing,
        },
        best_effort_database_mirror: state.db_mirror.is_some() && !state.require_db_mirror_writes,
        artifact_store: match store {
            Some(store) if store.object_io_enabled() => {
                if store
                    .provider_label()
                    .unwrap_or(LOCAL_ENCRYPTED_PROVIDER_LABEL)
                    == GCS_PROVIDER_LABEL
                {
                    ProductionAdapterKind::Production
                } else {
                    ProductionAdapterKind::Development
                }
            }
            _ => ProductionAdapterKind::Missing,
        },
        plaintext_fallback: store
            .is_none_or(ConfiguredTraceArtifactStore::plaintext_compatibility_allowed),
        key_wrapper: match store.and_then(ConfiguredTraceArtifactStore::kek_status) {
            Some(status) if status.kind == GCP_CLOUD_KMS_WRAPPER_KIND => {
                ProductionAdapterKind::Production
            }
            Some(_) => ProductionAdapterKind::Development,
            None => ProductionAdapterKind::Missing,
        },
        authentication: match (
            verifier.is_some(),
            state.require_managed_eddsa_signed_tokens,
        ) {
            (true, true) => ProductionAdapterKind::Production,
            (true, false) => ProductionAdapterKind::Development,
            (false, _) => ProductionAdapterKind::Missing,
        },
        static_bearer_authentication: !state.tokens.is_empty(),
        hs256_bridge_authentication: verifier.is_some_and(|verifier| {
            verifier.read().map_or(true, |verifier| {
                verifier.configured_key_count() > verifier.configured_eddsa_key_count()
            })
        }),
        unversioned_policy_dependencies: false,
        live_external_payout_enabled: state
            .pipeline_service
            .as_ref()
            .and_then(|service| service.payout_controls())
            .is_some_and(|controls| controls.settlement_mode == PipelineNearSettlementMode::Http),
    }
}

/// The profile the routes give the qualification and the gate:
/// `infrastructure_profile_from_state`, or, in a test build only, the test
/// state's override.
fn route_infrastructure_profile(state: &AppState) -> ProductionInfrastructureProfile {
    #[cfg(test)]
    if let Some(profile) = &state.pipeline_infrastructure_override {
        return profile.clone();
    }
    infrastructure_profile_from_state(state)
}

/// The gate driver's attempt ceiling when this process runs the in-process
/// gate driver (`TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED`; its
/// `max_attempts`), else `None`: the mode of the drain report.
fn gate_driver_max_attempts(state: &AppState) -> Option<i32> {
    state
        .perplexity_score_driver
        .as_ref()
        .map(|driver| driver.knobs.max_attempts)
}

/// The drain report's precondition (`legacy_drain_report`): the tenant's
/// legacy records in the database are authoritative. (1) The legacy path's
/// database writes are required, so a failed write fails the legacy
/// operation: the predicate `enforce_db_mirror_write_result` applies
/// (`TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES`, or account admission on). (2)
/// The tenant's review, credit, settlement, and NEAR outbox reads come from
/// the database (`db_reviewer_reads_for_tenant`), so the legacy workers act on
/// the rows the report counts.
fn legacy_records_authoritative(state: &AppState, tenant_id: &str) -> bool {
    (state.require_db_mirror_writes || state.account_admission.is_some())
        && state.db_reviewer_reads_for_tenant(tenant_id)
}

/// Whether `label` is a refusal label these routes answer: a safe label
/// (`^[a-z0-9_]{1,64}$`), or a safe label with one `:<check_id>` suffix whose
/// check id is a safe label too, the form `evaluate_promotion` gives a
/// refusal or a blocker about one check (final fix wave K2).
fn is_refusal_label(label: &str) -> bool {
    is_safe_label(label)
        || label
            .split_once(':')
            .is_some_and(|(label, check_id)| is_safe_label(label) && is_safe_label(check_id))
}

/// The refusal labels that are `503`, not `409`: a condition that passes by
/// itself, which the operator waits out and then repeats the request.
const RETRYABLE_REFUSAL_LABELS: &[&str] = &[
    // An admin operation that waited its lock timeout (5 s) behind a receipt,
    // a phase commit, or another admin operation (final fix wave G15).
    PIPELINE_ROUTING_BUSY_LABEL,
    PIPELINE_POLICY_INTERVENTION_BUSY_LABEL,
    // A drain report statement past its statement timeout (30 s, G27).
    LEGACY_DRAIN_REPORT_TIMEOUT_LABEL,
];

/// The one map from a store's or a check's error to an answer, shared by the
/// nine handlers: a refusal named by a refusal label (`is_refusal_label`) is
/// `409` with that label, or `503` for the labels of
/// `RETRYABLE_REFUSAL_LABELS`; a bundle the tenant has no policy row for is
/// `404` `bundle_package_missing`; anything else is the hash-only internal
/// error. No answer is built from a request field.
fn activation_error(error: DatabaseError) -> (StatusCode, Json<ApiError>) {
    match error {
        DatabaseError::Constraint(label) if RETRYABLE_REFUSAL_LABELS.contains(&label.as_str()) => {
            api_error(StatusCode::SERVICE_UNAVAILABLE, label)
        }
        DatabaseError::Constraint(label) if is_refusal_label(&label) => {
            api_error(StatusCode::CONFLICT, label)
        }
        DatabaseError::NotFound { .. } => {
            api_error(StatusCode::NOT_FOUND, BUNDLE_PACKAGE_MISSING_LABEL)
        }
        other => internal_error(other),
    }
}

/// `activation_error` of a check's refusal label.
fn refusal(label: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    activation_error(DatabaseError::Constraint(label.into()))
}

/// `activation_error`, and, when the refusal is `not_ready_label` (the
/// route's refusal of a promotion decision that is not ready), the
/// decision's blockers in the body's `blockers` (final fix wave K3): labels
/// only, each one a refusal label (`is_refusal_label`), so an operator sees
/// which check is missing, failed, or stale without reading the evidence.
fn promotion_refusal(
    error: DatabaseError,
    not_ready_label: &str,
    promotion: &PromotionDecision,
) -> (StatusCode, Json<ApiError>) {
    let not_ready = matches!(&error, DatabaseError::Constraint(label) if label == not_ready_label);
    let (status, Json(mut body)) = activation_error(error);
    if not_ready {
        body.blockers = Some(
            promotion
                .safe_blockers
                .iter()
                .filter(|blocker| is_refusal_label(blocker))
                .cloned()
                .collect(),
        );
    }
    (status, Json(body))
}

/// The parsed body, or its refusal: `413` `pipeline_request_too_large` above
/// the body limit, else the parser's status with `pipeline_request_invalid`.
/// The parser's own text, which can quote the request, is never answered.
fn request_body<T>(body: Result<Json<T>, JsonRejection>) -> ApiResult<T> {
    body.map(|Json(body)| body)
        .map_err(|rejection| request_refusal(rejection.status()))
}

/// `request_body` for a query string.
fn request_query<T>(query: Result<Query<T>, QueryRejection>) -> ApiResult<T> {
    query
        .map(|Query(query)| query)
        .map_err(|rejection| request_refusal(rejection.status()))
}

fn request_refusal(status: StatusCode) -> (StatusCode, Json<ApiError>) {
    if status == StatusCode::PAYLOAD_TOO_LARGE {
        api_error(status, PIPELINE_REQUEST_TOO_LARGE_LABEL)
    } else {
        api_error(status, PIPELINE_REQUEST_INVALID_LABEL)
    }
}

/// `422` `pipeline_request_invalid` for a bundle id that is not `sha256:`
/// and 64 lowercase hex digits (`is_bundle_id`, the rule `intervene_policy`
/// applies), before the id reaches any store call.
fn require_bundle_id(bundle_id: &str) -> ApiResult<()> {
    if !is_bundle_id(bundle_id) {
        return Err(api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            PIPELINE_REQUEST_INVALID_LABEL,
        ));
    }
    Ok(())
}

/// `413` `pipeline_evidence_too_large` above `PIPELINE_MAX_ATTESTATIONS`.
fn bound_attestations(attestations: &[PipelineCheckAttestation]) -> ApiResult<()> {
    if attestations.len() > PIPELINE_MAX_ATTESTATIONS {
        return Err(api_error(
            StatusCode::PAYLOAD_TOO_LARGE,
            PIPELINE_EVIDENCE_TOO_LARGE_LABEL,
        ));
    }
    Ok(())
}

fn require_activation_store(state: &AppState) -> ApiResult<&PipelineActivationStore> {
    state
        .pipeline_activation
        .as_deref()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, PIPELINE_ROUTING_STORE_MISSING_LABEL))
}

/// The two trust stores and the deployed revision, each from the state:
/// `503` `pipeline_trust_store_missing` without both stores, `409`
/// `bundle_runtime_revision_unknown` without the revision.
struct TrustAndRevision<'a> {
    package: &'a BundlePackageTrustStore,
    check: &'a CheckResultTrustStore,
    revision: &'a str,
}

fn trust_and_revision(state: &AppState) -> ApiResult<TrustAndRevision<'_>> {
    let (Some(package), Some(check)) = (
        state.pipeline_package_trust.as_deref(),
        state.pipeline_check_trust.as_deref(),
    ) else {
        return Err(api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            PIPELINE_TRUST_STORE_MISSING_LABEL,
        ));
    };
    let revision = state
        .pipeline_code_revision_hash
        .as_deref()
        .ok_or_else(|| api_error(StatusCode::CONFLICT, PACKAGE_RUNTIME_REVISION_UNKNOWN_LABEL))?;
    Ok(TrustAndRevision {
        package,
        check,
        revision,
    })
}

/// Logs one successful action: the tenant's storage reference, the action
/// label, and the evidence hash of what it recorded. Never the package, a
/// token, or the tenant id.
fn log_action(tenant: &TenantAuth, action: &'static str, evidence_hash: &str) {
    tracing::info!(
        tenant_storage_ref = %tenant_storage_ref(&tenant.tenant_id),
        action,
        evidence_hash,
        "pipeline admin action recorded"
    );
}

/// `POST /v1/admin/pipeline/qualifications`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct QualifyBody {
    signed_package: SignedBundlePackage,
    attestations: Vec<PipelineCheckAttestation>,
}

/// `POST /v1/admin/pipeline/activate` and `POST /v1/admin/pipeline/rollback`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ActivateBody {
    bundle_id: String,
    reason_code: String,
    attestations: Vec<PipelineCheckAttestation>,
}

/// `POST /v1/admin/pipeline/contain` and `POST /v1/admin/pipeline/deactivate`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReasonBody {
    reason_code: String,
}

/// `POST /v1/admin/pipeline/policy-interventions`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InterventionBody {
    bundle_id: String,
    phase: Phase,
    action: String,
    reason_code: String,
}

/// `GET /v1/admin/pipeline/policy-interventions?bundle_id=...`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InterventionQuery {
    bundle_id: String,
}

/// `GET /v1/admin/pipeline/routing`: the tenant's routing state (`None` with
/// no routing row), its active bundle, and its newest routing events.
#[derive(Debug, Serialize)]
pub(crate) struct PipelineRoutingView {
    routing_state: Option<RoutingState>,
    active_bundle_id: Option<String>,
    events: Vec<ActivationEvent>,
}

/// `GET /v1/admin/pipeline/policy-interventions`: oldest first.
#[derive(Debug, Serialize)]
pub(crate) struct PipelinePolicyInterventions {
    interventions: Vec<PipelinePolicyInterventionRecord>,
}

/// The gate's inputs that a route supplies, each from the server's state.
struct GateInputs<'a> {
    promotion: PromotionDecision,
    dependencies: ProductionDependencyProfile,
    revision: &'a str,
}

impl GateInputs<'_> {
    fn request<'r>(
        &'r self,
        tenant: &'r TenantAuth,
        body: &'r ActivateBody,
    ) -> ActivationRequest<'r> {
        ActivationRequest {
            tenant_id: &tenant.tenant_id,
            bundle_id: &body.bundle_id,
            actor_principal_ref: &tenant.principal_ref,
            reason_code: &body.reason_code,
            promotion: &self.promotion,
            runtime_code_revision_hash: self.revision,
            dependencies: &self.dependencies,
        }
    }
}

/// The activation and the rollback, in order, up to the store call: the
/// bundle id's shape and the attestation count; the scope of this process
/// (`409 pipeline_tenant_not_in_scope` for a tenant that is not on its
/// receipts list, final fix wave G8: every new upload of an activated tenant
/// is `503 pipeline_tenant_not_served` on a process that does not list it);
/// the two trust stores and the revision; the attestations verified against
/// the check store; the promotion evaluated over them now (an `Err` is
/// refused here, a decision that is not ready by the gate); the tenant's
/// stored package (`404` when none); the package trust store (final fix wave
/// G11, below); the startup checks of a tenant bundle on it; and its
/// dependency profile with this process's infrastructure. Nothing in it comes
/// from the request but the bundle id and the attestations.
///
/// The package trust store. A stored package carries no signature: the
/// qualification verified it and recorded the id of the key that signed it
/// (`signing_key_id`). So the check here is that the package trust store of
/// this process still holds the key id that the bundle's qualification on
/// the deployed revision recorded; a key that an operator removed from the
/// store stops activating and rolling back to every bundle it signed, with
/// the label the package verification gives an unknown signer
/// (`bundle_package_signer_untrusted`). A bundle with no qualification on
/// this revision is left to the gate, which refuses it with its own labels.
async fn gate_inputs<'a>(
    state: &'a AppState,
    service: &PipelineService,
    tenant: &TenantAuth,
    body: &ActivateBody,
) -> ApiResult<GateInputs<'a>> {
    require_bundle_id(&body.bundle_id)?;
    bound_attestations(&body.attestations)?;
    if !state.tenant_rollout_gates.enabled_for(
        TraceTenantRolloutFeature::PipelineReceipts,
        false,
        &tenant.tenant_id,
    ) {
        return Err(api_error(
            StatusCode::CONFLICT,
            PIPELINE_TENANT_NOT_IN_SCOPE_LABEL,
        ));
    }
    let trust = trust_and_revision(state)?;
    let verified = trust
        .check
        .verify_all(&body.attestations)
        .map_err(refusal)?;
    let promotion = evaluate_promotion(&verified.evidence, Utc::now()).map_err(refusal)?;
    let package = service
        .store()
        .load_bundle(&tenant.tenant_id, &body.bundle_id)
        .await
        .map_err(|error| match error {
            // A stored package that no longer validates (a tampered package,
            // BND-002) is refused with the gate's label for it, as the gate
            // refuses it (`stored_package_or_refusal`), never as an internal
            // error (final fix wave K4).
            DatabaseError::Serialization(label) if label == PIPELINE_BUNDLE_INVALID_LABEL => {
                refusal(BUNDLE_PACKAGE_MISSING_LABEL)
            }
            other => activation_error(other),
        })?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, BUNDLE_PACKAGE_MISSING_LABEL))?;
    let qualification = state
        .pipeline_qualification
        .as_deref()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, PIPELINE_ROUTING_STORE_MISSING_LABEL))?
        .qualification(&tenant.tenant_id, &body.bundle_id, trust.revision)
        .await
        .map_err(activation_error)?;
    if qualification.is_some_and(|record| !trust.package.holds_key_id(&record.signing_key_id)) {
        return Err(refusal(PACKAGE_SIGNER_UNTRUSTED_LABEL));
    }
    service
        .check_runnable_package(&package, &state.pipeline_main_gate, true)
        .map_err(refusal)?;
    let dependencies = ProductionDependencyProfile::for_bundle(
        service,
        &package,
        route_infrastructure_profile(state),
    )
    .map_err(refusal)?;
    Ok(GateInputs {
        promotion,
        dependencies,
        revision: trust.revision,
    })
}

/// `GET /v1/admin/pipeline/routing`: the routing state, the active bundle,
/// and the newest events, read at one instant
/// (`PipelineActivationStore::routing_view`). Needs the routing store, not a
/// runtime. Records the read, as the operational summary does.
pub(crate) async fn pipeline_routing_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<PipelineRoutingView>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let view = require_activation_store(state.as_ref())?
        .routing_view(&tenant.tenant_id, PIPELINE_ROUTING_EVENT_LIMIT)
        .await
        .map_err(activation_error)?;
    append_control_plane_read_audit(
        state.as_ref(),
        &tenant,
        ROUTING_READ_SURFACE,
        view.events.len(),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(PipelineRoutingView {
        routing_state: view.routing.map(|routing| routing.routing_state),
        active_bundle_id: view.active_bundle_id,
        events: view.events,
    }))
}

/// `POST /v1/admin/pipeline/qualifications`: records a qualification of the
/// signed package for the tenant on this process's code revision, through
/// `qualify_bundle_attested` only. It verifies the package against the
/// package trust store and the attestations against the check trust store,
/// and builds the qualification's metadata itself.
pub(crate) async fn pipeline_qualify_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<QualifyBody>, JsonRejection>,
) -> ApiResult<Json<BundleQualificationRecord>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let service = require_pipeline_service(state.as_ref())?;
    let body = request_body(body)?;
    bound_attestations(&body.attestations)?;
    let trust = trust_and_revision(state.as_ref())?;
    let qualifications = state
        .pipeline_qualification
        .as_deref()
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, PIPELINE_ROUTING_STORE_MISSING_LABEL))?;
    // The package's signature first, before any work on the package (fix
    // round 1); `qualify_bundle_attested` verifies it again.
    trust
        .package
        .verify(&body.signed_package)
        .map_err(refusal)?;
    let dependencies = ProductionDependencyProfile::for_bundle(
        service,
        &body.signed_package.package,
        route_infrastructure_profile(state.as_ref()),
    )
    .map_err(refusal)?;
    let record = match qualifications
        .qualify_bundle_attested(
            &tenant.tenant_id,
            &body.signed_package,
            trust.package,
            trust.check,
            &dependencies,
            &body.attestations,
            trust.revision,
        )
        .await
    {
        Ok(record) => record,
        Err(DatabaseError::Constraint(label))
            if label == QUALIFICATION_PROMOTION_NOT_READY_LABEL =>
        {
            // The decision the qualification refused, evaluated again over
            // the same verified results for its blockers (K3).
            let error = DatabaseError::Constraint(label);
            return Err(
                match trust
                    .check
                    .verify_all(&body.attestations)
                    .ok()
                    .and_then(|verified| evaluate_promotion(&verified.evidence, Utc::now()).ok())
                {
                    Some(promotion) => promotion_refusal(
                        error,
                        QUALIFICATION_PROMOTION_NOT_READY_LABEL,
                        &promotion,
                    ),
                    None => activation_error(error),
                },
            );
        }
        Err(error) => return Err(activation_error(error)),
    };
    log_action(&tenant, "qualify", &record.metadata.evidence_hash);
    Ok(Json(record))
}

/// `POST /v1/admin/pipeline/activate`: routes the tenant's new receipts to
/// the pipeline with the bundle, when the tenant's readiness passes and every
/// term of the qualified activation gate holds.
pub(crate) async fn pipeline_activate_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<ActivateBody>, JsonRejection>,
) -> ApiResult<Json<TenantRouting>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let service = require_pipeline_service(state.as_ref())?;
    let body = request_body(body)?;
    let inputs = gate_inputs(state.as_ref(), service, &tenant, &body).await?;
    let summary = require_pipeline_product(state.as_ref())?
        .operational_summary(&tenant.tenant_id)
        .await
        .map_err(activation_error)?;
    let readiness = ActivationReadiness::from_operational_summary(&summary);
    let routing = require_activation_store(state.as_ref())?
        .activate_tenant(inputs.request(&tenant, &body), &readiness)
        .await
        .map_err(|error| {
            promotion_refusal(
                error,
                ACTIVATION_PROMOTION_NOT_READY_LABEL,
                &inputs.promotion,
            )
        })?;
    log_action(&tenant, "activate", &routing.evidence_hash);
    Ok(Json(routing))
}

/// `POST /v1/admin/pipeline/rollback`: routes a `pipeline` or `contained`
/// tenant's new receipts to the pipeline with a bundle it selected before.
/// The same inputs as the activation, without the readiness.
pub(crate) async fn pipeline_rollback_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<ActivateBody>, JsonRejection>,
) -> ApiResult<Json<TenantRouting>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let service = require_pipeline_service(state.as_ref())?;
    let body = request_body(body)?;
    let inputs = gate_inputs(state.as_ref(), service, &tenant, &body).await?;
    let routing = require_activation_store(state.as_ref())?
        .rollback_bundle(inputs.request(&tenant, &body))
        .await
        .map_err(|error| {
            promotion_refusal(
                error,
                ACTIVATION_PROMOTION_NOT_READY_LABEL,
                &inputs.promotion,
            )
        })?;
    log_action(&tenant, "rollback", &routing.evidence_hash);
    Ok(Json(routing))
}

/// `POST /v1/admin/pipeline/contain`: stops the tenant's new receipts.
pub(crate) async fn pipeline_contain_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<ReasonBody>, JsonRejection>,
) -> ApiResult<Json<TenantRouting>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    require_pipeline_service(state.as_ref())?;
    let body = request_body(body)?;
    let routing = require_activation_store(state.as_ref())?
        .contain(&tenant.tenant_id, &tenant.principal_ref, &body.reason_code)
        .await
        .map_err(activation_error)?;
    log_action(&tenant, "contain", &routing.evidence_hash);
    Ok(Json(routing))
}

/// `POST /v1/admin/pipeline/deactivate`: returns the tenant to the legacy
/// path.
pub(crate) async fn pipeline_deactivate_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<ReasonBody>, JsonRejection>,
) -> ApiResult<Json<TenantRouting>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    require_pipeline_service(state.as_ref())?;
    let body = request_body(body)?;
    let routing = require_activation_store(state.as_ref())?
        .deactivate(&tenant.tenant_id, &tenant.principal_ref, &body.reason_code)
        .await
        .map_err(activation_error)?;
    log_action(&tenant, "deactivate", &routing.evidence_hash);
    Ok(Json(routing))
}

/// `POST /v1/admin/pipeline/policy-interventions`: suspends or resumes one
/// policy of one of the tenant's bundles (`terminate` is
/// `policy_intervention_not_supported`).
pub(crate) async fn pipeline_policy_intervention_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Result<Json<InterventionBody>, JsonRejection>,
) -> ApiResult<Json<PipelinePolicyInterventionRecord>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let service = require_pipeline_service(state.as_ref())?;
    let body = request_body(body)?;
    let record = service
        .store()
        .intervene_policy(
            &tenant.tenant_id,
            &body.bundle_id,
            body.phase,
            &body.action,
            &tenant.principal_ref,
            &body.reason_code,
        )
        .await
        .map_err(activation_error)?;
    // The label comes from the recorded transition, not from the request.
    let action = match record.resulting_status {
        PolicyOperationalStatus::Suspended => "policy_suspend",
        PolicyOperationalStatus::Runnable => "policy_resume",
        PolicyOperationalStatus::Terminated => "policy_terminate",
    };
    log_action(&tenant, action, &record.evidence_hash);
    Ok(Json(record))
}

/// `GET /v1/admin/pipeline/policy-interventions?bundle_id=...`: the
/// interventions on one of the tenant's bundles, oldest first. A bundle id
/// that is not one is `422` before any read. Records the read.
pub(crate) async fn pipeline_policy_interventions_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    query: Result<Query<InterventionQuery>, QueryRejection>,
) -> ApiResult<Json<PipelinePolicyInterventions>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let service = require_pipeline_service(state.as_ref())?;
    let query = request_query(query)?;
    require_bundle_id(&query.bundle_id)?;
    let interventions = service
        .store()
        .list_policy_interventions(&tenant.tenant_id, &query.bundle_id)
        .await
        .map_err(activation_error)?;
    append_control_plane_read_audit(
        state.as_ref(),
        &tenant,
        POLICY_INTERVENTIONS_READ_SURFACE,
        interventions.len(),
    )
    .await
    .map_err(internal_error)?;
    Ok(Json(PipelinePolicyInterventions { interventions }))
}

/// `GET /v1/admin/pipeline/legacy-drain`: what the legacy path still owes
/// the tenant (`legacy_drain_report`), in the mode of this process's gate
/// driver and with its legal-hold retention policies. Refused with `409`
/// `legacy_drain_records_not_authoritative` unless the tenant's legacy
/// records in the database are authoritative. Needs the routing store, not a
/// runtime. Records the read.
pub(crate) async fn pipeline_legacy_drain_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<LegacyDrainReport>> {
    let tenant = authenticate_with_tenant_access_grant(state.as_ref(), &headers).await?;
    require_admin(&tenant)?;
    let store = require_activation_store(state.as_ref())?;
    if !legacy_records_authoritative(state.as_ref(), &tenant.tenant_id) {
        return Err(api_error(
            StatusCode::CONFLICT,
            LEGACY_DRAIN_RECORDS_NOT_AUTHORITATIVE_LABEL,
        ));
    }
    let held_retention_policy_ids = state
        .legal_hold_retention_policy_ids
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    let report = store
        .legacy_drain_report(
            &tenant.tenant_id,
            gate_driver_max_attempts(state.as_ref()),
            &held_retention_policy_ids,
        )
        .await
        .map_err(activation_error)?;
    // One report.
    append_control_plane_read_audit(state.as_ref(), &tenant, LEGACY_DRAIN_READ_SURFACE, 1)
        .await
        .map_err(internal_error)?;
    Ok(Json(report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_server::versioned_pipeline_qualification::ProductionAdapterKind;

    /// A trusted key with `key_id` and the 32 public key bytes `seed`
    /// repeated: the shape the loader reads, not a real key pair.
    fn key(key_id: &str, seed: u8) -> TrustedBundleKey {
        TrustedBundleKey {
            key_id: key_id.to_string(),
            public_key_base64url: base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode([seed; 32]),
        }
    }

    fn written(dir: &tempfile::TempDir, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).expect("write the trust store file");
        path
    }

    /// A verifier with an HS256 secret when `hs256` and one EdDSA key; the
    /// key is only counted here, never parsed.
    fn verifier(hs256: bool) -> SharedTraceCommonsSignedTokenVerifier {
        shared_signed_token_verifier(TraceCommonsSignedTokenVerifier {
            default_secret: hs256.then(|| SecretString::from("hs256-bridge-secret".to_string())),
            keyed_secrets: BTreeMap::new(),
            default_eddsa_public_key: Some(TraceCommonsSignedEddsaPublicKey {
                pem: "eddsa-public-key-counted-only".to_string(),
                not_before: None,
                not_after: None,
            }),
            keyed_eddsa_public_keys: BTreeMap::new(),
            managed_eddsa_key_ids: BTreeSet::new(),
            managed_eddsa_keyset_last_refreshed_at: None,
            managed_eddsa_keyset_last_refresh_failed_at: None,
            issuer: None,
            audience: None,
            revoked_jtis: BTreeSet::new(),
            max_ttl_seconds: None,
            require_jti: false,
        })
    }

    fn kek(kind: &str) -> KekWrapperStatus {
        KekWrapperStatus {
            kind: kind.to_string(),
            key_ref_hash: "sha256:kek-ref".to_string(),
            is_production_trust_boundary: kind != "local_master_key",
        }
    }

    /// One case for each row of the brief's table, each changing one field
    /// of a state whose every row reads its production value.
    #[tokio::test]
    async fn the_infrastructure_profile_follows_the_configuration() {
        use ProductionAdapterKind::{Development, Missing, Production};

        let dir = tempfile::tempdir().unwrap();
        let service_paying_in =
            |mode| crate::tests::qualified_test_service_with_payout(&dir, Some(mode));
        let unpaid = crate::tests::qualified_test_service_with_payout(&dir, None).await;
        let paid = service_paying_in(PipelineNearSettlementMode::Http).await;
        let dry_run = service_paying_in(PipelineNearSettlementMode::DryRun).await;
        let disabled = service_paying_in(PipelineNearSettlementMode::Disabled).await;
        assert!(!unpaid.payout_enabled() && paid.payout_enabled());
        assert!(dry_run.payout_enabled() && disabled.payout_enabled());
        let unused_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let backend: Arc<dyn Database> = Arc::new(
            PgBackend::new(&DatabaseConfig::from_postgres_url(
                &format!("postgres://nobody@127.0.0.1:{unused_port}/none"),
                1,
            ))
            .await
            .unwrap(),
        );
        let local_store = || {
            let crypto = SecretsCrypto::new(SecretString::from(
                trace_commons_server::secrets::keychain::generate_master_key_hex(),
            ))
            .expect("test crypto");
            ConfiguredTraceArtifactStore::legacy(Arc::new(LocalEncryptedTraceArtifactStore::new(
                dir.path(),
                crypto,
            )))
        };
        let gcs = ConfiguredTraceArtifactStore {
            provider_label: Some("gcs"),
            plaintext_compatibility_allowed: false,
            kek_status: Some(kek("gcp_cloud_kms")),
            ..local_store()
        };
        let mut production_state = (*crate::tests::test_state(dir.path().to_path_buf())).clone();
        production_state.db_mirror = Some(backend);
        production_state.require_db_mirror_writes = true;
        production_state.artifact_store = Some(gcs.clone());
        production_state.signed_token_verifier = Some(verifier(false));
        production_state.require_managed_eddsa_signed_tokens = true;
        production_state.tokens = Arc::new(BTreeMap::new());
        production_state.pipeline_service = Some(unpaid);
        let production = ProductionInfrastructureProfile {
            authoritative_metadata: Production,
            artifact_store: Production,
            key_wrapper: Production,
            authentication: Production,
            plaintext_fallback: false,
            best_effort_database_mirror: false,
            static_bearer_authentication: false,
            hs256_bridge_authentication: false,
            unversioned_policy_dependencies: false,
            live_external_payout_enabled: false,
        };
        assert_eq!(
            infrastructure_profile_from_state(&production_state),
            production
        );
        let with = |change: &dyn Fn(&mut AppState)| {
            let mut state = production_state.clone();
            change(&mut state);
            infrastructure_profile_from_state(&state)
        };
        let store = |change: &dyn Fn(&mut ConfiguredTraceArtifactStore)| {
            let mut store = gcs.clone();
            change(&mut store);
            move |state: &mut AppState| state.artifact_store = Some(store.clone())
        };
        let cases: Vec<(
            &str,
            ProductionInfrastructureProfile,
            ProductionInfrastructureProfile,
        )> = vec![
            (
                "authoritative_metadata: a database mirror whose writes are best effort",
                with(&|state| state.require_db_mirror_writes = false),
                ProductionInfrastructureProfile {
                    authoritative_metadata: Development,
                    best_effort_database_mirror: true,
                    ..production.clone()
                },
            ),
            (
                "authoritative_metadata: no database mirror",
                with(&|state| state.db_mirror = None),
                ProductionInfrastructureProfile {
                    authoritative_metadata: Missing,
                    ..production.clone()
                },
            ),
            (
                "artifact_store: a file-system store",
                with(&store(&|store| store.provider_label = Some("file_system"))),
                ProductionInfrastructureProfile {
                    artifact_store: Development,
                    ..production.clone()
                },
            ),
            (
                "artifact_store: the local encrypted store (no typed label)",
                with(&store(&|store| store.provider_label = None)),
                ProductionInfrastructureProfile {
                    artifact_store: Development,
                    ..production.clone()
                },
            ),
            (
                "artifact_store: a remote store compiled out (object IO disabled)",
                with(&store(&|store| store.object_io_enabled = false)),
                ProductionInfrastructureProfile {
                    artifact_store: Missing,
                    ..production.clone()
                },
            ),
            (
                "artifact_store: no artifact store (the plain file store)",
                with(&|state| state.artifact_store = None),
                ProductionInfrastructureProfile {
                    artifact_store: Missing,
                    key_wrapper: Missing,
                    plaintext_fallback: true,
                    ..production.clone()
                },
            ),
            (
                "plaintext_fallback: a store that allows plaintext compatibility",
                with(&store(&|store| {
                    store.plaintext_compatibility_allowed = true
                })),
                ProductionInfrastructureProfile {
                    plaintext_fallback: true,
                    ..production.clone()
                },
            ),
            (
                "key_wrapper: a local master key",
                with(&store(&|store| {
                    store.kek_status = Some(kek("local_master_key"))
                })),
                ProductionInfrastructureProfile {
                    key_wrapper: Development,
                    ..production.clone()
                },
            ),
            (
                "key_wrapper: the dstack stub",
                with(&store(&|store| store.kek_status = Some(kek("dstack_kek")))),
                ProductionInfrastructureProfile {
                    key_wrapper: Development,
                    ..production.clone()
                },
            ),
            (
                "key_wrapper: no key wrapper",
                with(&store(&|store| store.kek_status = None)),
                ProductionInfrastructureProfile {
                    key_wrapper: Missing,
                    ..production.clone()
                },
            ),
            (
                "authentication: a verifier without the managed EdDSA requirement",
                with(&|state| state.require_managed_eddsa_signed_tokens = false),
                ProductionInfrastructureProfile {
                    authentication: Development,
                    ..production.clone()
                },
            ),
            (
                "authentication: no signed-token verifier",
                with(&|state| state.signed_token_verifier = None),
                ProductionInfrastructureProfile {
                    authentication: Missing,
                    ..production.clone()
                },
            ),
            (
                "static_bearer_authentication: a static token",
                with(&|state| {
                    let mut tokens = BTreeMap::new();
                    insert_token(&mut tokens, "tenant-a", "token-a", TokenRole::Admin);
                    state.tokens = Arc::new(tokens);
                }),
                ProductionInfrastructureProfile {
                    static_bearer_authentication: true,
                    ..production.clone()
                },
            ),
            (
                "hs256_bridge_authentication: an HS256 secret beside the EdDSA key",
                with(&|state| state.signed_token_verifier = Some(verifier(true))),
                ProductionInfrastructureProfile {
                    hs256_bridge_authentication: true,
                    ..production.clone()
                },
            ),
            (
                "live_external_payout_enabled: a runtime whose payout pays (settlement mode http)",
                with(&|state| state.pipeline_service = Some(paid.clone())),
                ProductionInfrastructureProfile {
                    live_external_payout_enabled: true,
                    ..production.clone()
                },
            ),
            // Final fix wave (G4): a payout that moves no money is not live.
            (
                "live_external_payout_enabled: a payout in the dry_run settlement mode",
                with(&|state| state.pipeline_service = Some(dry_run.clone())),
                production.clone(),
            ),
            (
                "live_external_payout_enabled: a payout in the disabled settlement mode",
                with(&|state| state.pipeline_service = Some(disabled.clone())),
                production.clone(),
            ),
            (
                "live_external_payout_enabled: no runtime",
                with(&|state| state.pipeline_service = None),
                production.clone(),
            ),
        ];
        for (case, actual, expected) in cases {
            assert_eq!(actual, expected, "{case}");
        }
    }

    /// The loader reads a JSON array of `TrustedBundleKey`; an empty array, a
    /// file that is not JSON, a duplicate key id, a key that is not 32 bytes,
    /// and a file that cannot be read fail with `pipeline_trust_store_invalid`
    /// (the label only, never the path); an unset variable gives `None`.
    #[test]
    fn a_trust_store_file_is_a_list_of_trusted_keys() {
        let dir = tempfile::tempdir().unwrap();
        let keys = vec![key("release_key_1", 1), key("release_key_2", 2)];
        let path = written(&dir, "keys.json", &serde_json::to_vec(&keys).unwrap());
        assert_eq!(
            read_trusted_keys(Some(path.as_path())).unwrap(),
            Some(keys.clone())
        );
        let stores = pipeline_trust_stores(Some(keys.clone()), None).unwrap();
        assert!(stores.package.is_some() && stores.check.is_none());
        let stores = pipeline_trust_stores(None, Some(keys)).unwrap();
        assert!(stores.package.is_none() && stores.check.is_some());

        assert_eq!(read_trusted_keys(None).unwrap(), None);
        assert_eq!(
            read_trusted_keys_from_env("TRACE_COMMONS_PIPELINE_TEST_UNSET_TRUSTED_KEYS_PATH")
                .unwrap(),
            None,
            "an unset variable"
        );
        let stores = pipeline_trust_stores(None, None).unwrap();
        assert!(stores.package.is_none() && stores.check.is_none());

        let duplicate =
            serde_json::to_vec(&[key("release_key_1", 1), key("release_key_1", 3)]).unwrap();
        let short = serde_json::json!([{
            "key_id": "release_key_1",
            "public_key_base64url": base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([1u8; 31]),
        }]);
        for (case, contents) in [
            ("an empty array", b"[]".to_vec()),
            ("a file that is not JSON", b"not json".to_vec()),
            ("an object, not an array", b"{}".to_vec()),
            ("a duplicate key id", duplicate),
            (
                "a key that is not 32 bytes",
                serde_json::to_vec(&short).unwrap(),
            ),
        ] {
            let path = written(&dir, "invalid.json", &contents);
            let refused = read_trusted_keys(Some(path.as_path()))
                .and_then(|keys| pipeline_trust_stores(keys, None).map(|_| ()))
                .err()
                .unwrap_or_else(|| panic!("{case} is refused"));
            assert_eq!(
                refused.to_string(),
                PIPELINE_TRUST_STORE_INVALID_LABEL,
                "{case}"
            );
        }
        let missing = dir.path().join("missing.json");
        let refused =
            read_trusted_keys(Some(missing.as_path())).expect_err("a missing file is refused");
        assert_eq!(refused.to_string(), PIPELINE_TRUST_STORE_INVALID_LABEL);
        assert!(!format!("{refused:#}").contains("missing.json"), "no path");
    }

    /// The two stores never share a key: a check key whose public key bytes
    /// are a package key's (under any id), or whose id is a package key's,
    /// refuses the start with `pipeline_trust_store_overlap`.
    #[test]
    fn a_check_key_that_is_also_a_package_key_refuses_the_start() {
        let package = vec![key("package_key", 1), key("package_key_2", 2)];
        for (case, check) in [
            ("the same key under another id", vec![key("check_key", 2)]),
            (
                "another key under a package key's id",
                vec![key("package_key", 3)],
            ),
        ] {
            let refused = pipeline_trust_stores(Some(package.clone()), Some(check))
                .err()
                .unwrap_or_else(|| panic!("{case} is refused"));
            assert_eq!(
                refused.to_string(),
                PIPELINE_TRUST_STORE_OVERLAP_LABEL,
                "{case}"
            );
        }
        let stores = pipeline_trust_stores(Some(package), Some(vec![key("check_key", 4)]))
            .expect("two separate key sets");
        assert!(stores.package.is_some() && stores.check.is_some());
    }

    /// Every label this module answers, logs, or records is a safe label: each
    /// refusal, the start refusals, the runtime assembly's new refusal, the
    /// read audit surfaces, and the action labels of the log lines.
    #[test]
    fn the_route_labels_are_safe_labels() {
        for label in [
            PIPELINE_TRUST_STORE_INVALID_LABEL,
            PIPELINE_TRUST_STORE_OVERLAP_LABEL,
            PIPELINE_TRUST_STORE_MISSING_LABEL,
            PIPELINE_CODE_REVISION_INVALID_LABEL,
            PIPELINE_EVIDENCE_TOO_LARGE_LABEL,
            PIPELINE_REQUEST_INVALID_LABEL,
            PIPELINE_REQUEST_TOO_LARGE_LABEL,
            LEGACY_DRAIN_RECORDS_NOT_AUTHORITATIVE_LABEL,
            PIPELINE_ROUTING_STORE_MISSING_LABEL,
            PIPELINE_TENANT_NOT_IN_SCOPE_LABEL,
            PIPELINE_ROUTING_BUSY_LABEL,
            PIPELINE_POLICY_INTERVENTION_BUSY_LABEL,
            LEGACY_DRAIN_REPORT_TIMEOUT_LABEL,
            BUNDLE_PACKAGE_MISSING_LABEL,
            PACKAGE_RUNTIME_REVISION_UNKNOWN_LABEL,
            ROUTING_READ_SURFACE,
            POLICY_INTERVENTIONS_READ_SURFACE,
            LEGACY_DRAIN_READ_SURFACE,
            "pipeline_unqualified_routing_not_allowed_when_required",
            "pipeline_unqualified_routing_with_production_runtime",
            "pipeline_unqualified_routing_allowed",
            "qualify",
            "activate",
            "rollback",
            "contain",
            "deactivate",
            "policy_suspend",
            "policy_resume",
            "policy_terminate",
        ] {
            assert!(is_safe_label(label), "{label}");
        }
        assert_eq!(PIPELINE_MAX_ATTESTATIONS, 64);
    }

    /// Final fix wave (K2): a refusal label with a `:<check_id>` suffix, as
    /// `evaluate_promotion` gives for one check (`qualification_evidence_invalid:<check_id>`),
    /// is answered `409` with that label, never `500`; the check id must be a
    /// safe label too. Any other text is still the internal error.
    #[test]
    fn a_refusal_label_with_a_check_id_is_answered_as_a_refusal() {
        for label in [
            "qualification_evidence_invalid:pipeline_restore_drill",
            "qualification_evidence_stale:pipeline_crash_matrix",
        ] {
            let (status, Json(body)) =
                activation_error(DatabaseError::Constraint(label.to_string()));
            assert_eq!((status, body.error.as_str()), (StatusCode::CONFLICT, label));
        }
        let too_long_check_id = format!("qualification_evidence_invalid:{}", "a".repeat(65));
        for unsafe_label in [
            "Not A Label",
            "qualification_evidence_invalid:Pipeline-Check",
            "qualification_evidence_invalid:",
            ":pipeline_restore_drill",
            "qualification_evidence_invalid:pipeline:drill",
            "qualification_evidence_invalid :pipeline_restore_drill",
            too_long_check_id.as_str(),
        ] {
            let (status, _) = activation_error(DatabaseError::Constraint(unsafe_label.to_string()));
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{unsafe_label}");
        }
    }

    /// Final fix wave (G15): an admin operation that waited out its lock
    /// timeout is `503` with its busy label, a condition an operator retries,
    /// not a `409` refusal.
    #[test]
    fn a_busy_admin_lock_is_service_unavailable() {
        for label in ["pipeline_routing_busy", "policy_intervention_busy"] {
            let (status, Json(body)) =
                activation_error(DatabaseError::Constraint(label.to_string()));
            assert_eq!(
                (status, body.error.as_str()),
                (StatusCode::SERVICE_UNAVAILABLE, label)
            );
        }
    }

    /// Final fix wave (G27): a drain report that ran past its statement
    /// timeout is `503 legacy_drain_report_timeout`.
    #[test]
    fn a_drain_report_past_its_statement_timeout_is_service_unavailable() {
        let (status, Json(body)) = activation_error(DatabaseError::Constraint(
            "legacy_drain_report_timeout".to_string(),
        ));
        assert_eq!(
            (status, body.error.as_str()),
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "legacy_drain_report_timeout"
            )
        );
    }

    /// Final fix wave (G25): a trust store variable set to the empty string,
    /// or to blanks, is unset, as `optional_trimmed_env` reads every other
    /// optional setting: it refuses no start, also in a binary that holds no
    /// pipeline runtime.
    #[test]
    fn an_empty_trust_store_variable_is_unset() {
        const VARIABLE: &str = "TRACE_COMMONS_PIPELINE_TEST_EMPTY_TRUSTED_KEYS_PATH";
        for value in ["", "   "] {
            // SAFETY: a variable no other test reads; set and removed here.
            unsafe { std::env::set_var(VARIABLE, value) };
            let read = read_trusted_keys_from_env(VARIABLE);
            // SAFETY: as above.
            unsafe { std::env::remove_var(VARIABLE) };
            assert_eq!(
                read.unwrap_or_else(|error| panic!("{value:?} refused the start: {error}")),
                None,
                "{value:?}"
            );
        }
    }

    /// Final fix wave (G17): a build revision that is set must be a `sha256:`
    /// digest of 64 lowercase hex digits, or the start is refused with
    /// `pipeline_code_revision_invalid` (the label only, never the value). An
    /// unset revision is `None`, and the routes refuse with
    /// `bundle_runtime_revision_unknown`.
    #[test]
    fn a_build_revision_that_is_set_must_be_a_digest() {
        let digest = format!("sha256:{}", "a1".repeat(32));
        assert_eq!(deployed_code_revision(None).unwrap(), None);
        assert_eq!(
            deployed_code_revision(Some(&digest)).unwrap(),
            Some(digest.clone())
        );
        let upper = digest.to_uppercase();
        let short = &digest[..digest.len() - 1];
        let padded = format!(" {digest}");
        for malformed in ["", "  ", "not-a-revision", upper.as_str(), short, &padded] {
            let refused = deployed_code_revision(Some(malformed))
                .err()
                .unwrap_or_else(|| panic!("{malformed:?} is refused"));
            assert_eq!(refused.to_string(), "pipeline_code_revision_invalid");
            if !malformed.trim().is_empty() {
                assert!(
                    !format!("{refused:#}").contains(malformed.trim()),
                    "the refusal never holds the value"
                );
            }
        }
    }

    /// Final fix wave (G17): config-status reports the start inputs of the
    /// qualification and activation routes as booleans: whether the build
    /// revision is set and whether each trust store is loaded, never the
    /// revision, a key id, or a key.
    #[tokio::test]
    async fn config_status_reports_the_revision_and_the_trust_stores() {
        let dir = tempfile::tempdir().unwrap();
        let base = crate::tests::test_state(dir.path().to_path_buf());
        let fields = [
            "pipeline_code_revision_configured",
            "pipeline_package_trust_store_loaded",
            "pipeline_check_trust_store_loaded",
        ];
        let off = serde_json::to_value(trace_commons_config_status_response(&base)).unwrap();
        for field in fields {
            assert_eq!(off[field], serde_json::json!(false), "{field}");
        }
        let revision = format!("sha256:{}", "b2".repeat(32));
        let mut on = (*base).clone();
        on.pipeline_code_revision_hash = Some(revision.clone());
        on.pipeline_package_trust = Some(Arc::new(
            BundlePackageTrustStore::new([key("status_package_key", 7)]).unwrap(),
        ));
        on.pipeline_check_trust = Some(Arc::new(
            CheckResultTrustStore::new([key("status_check_key", 8)]).unwrap(),
        ));
        let on = serde_json::to_value(trace_commons_config_status_response(&on)).unwrap();
        for field in fields {
            assert_eq!(on[field], serde_json::json!(true), "{field}");
        }
        let text = on.to_string();
        for secret in [
            revision.as_str(),
            "status_package_key",
            "status_check_key",
            &key("status_package_key", 7).public_key_base64url,
        ] {
            assert!(!text.contains(secret), "config-status holds no {secret}");
        }
    }

    /// Final fix wave (G9): config-status reports whether this process routes
    /// a listed tenant with no routing row to the pipeline (unqualified
    /// routing, a setting for tests).
    #[tokio::test]
    async fn config_status_reports_unqualified_routing() {
        let dir = tempfile::tempdir().unwrap();
        let base = crate::tests::test_state(dir.path().to_path_buf());
        for allowed in [false, true] {
            let mut state = (*base).clone();
            state.pipeline_unqualified_routing = allowed;
            let status =
                serde_json::to_value(trace_commons_config_status_response(&state)).unwrap();
            assert_eq!(
                status["pipeline_unqualified_routing_allowed"],
                serde_json::json!(allowed)
            );
        }
    }
}
