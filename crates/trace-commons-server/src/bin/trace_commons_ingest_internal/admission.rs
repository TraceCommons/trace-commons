// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use trace_commons_protocol::admission::{
    AdmissionBinding, AdmissionEvidence, AdmissionRefusal, EVIDENCE_HEADER, SIGNATURE_HEADER,
    hash_hex, is_hash,
};
use trace_commons_server::account_trust::{
    BoundedPolicy, TrustAccount, parse_bounded_policy, resolve_contribution_account,
};
use trace_commons_server::admission_evidence::{
    AdmissionProviderTrust, verify_admission_evidence, verify_stored_admission_signature,
};
use trace_commons_server::admission_ledger::{
    AccountAdmissionReservation, AdmissionDecision, AdmissionLimits, AdmissionProcessingGuard,
    AdmissionReservation,
};
use trace_commons_server::trace_invite_registry::RESERVED_ACCOUNT_TENANT_PREFIXES;

#[derive(Clone)]
pub(super) struct AdmissionConfig {
    pub limits: AdmissionLimits,
    pub providers: AdmissionProviderTrust,
}

#[derive(Clone)]
pub(super) struct AccountAdmissionConfig {
    pub policy: BoundedPolicy,
    pub lease_seconds: i64,
    pub providers: Option<AdmissionProviderTrust>,
}

pub(super) fn account_config_from_env(
    durable_db: bool,
) -> anyhow::Result<Option<AccountAdmissionConfig>> {
    account_config_from_values(
        durable_db,
        |key| std::env::var(key),
        || {
            AdmissionProviderTrust::from_env(account_evidence_prefix(|key| {
                std::env::var_os(key).is_some()
            }))
        },
    )
}

fn account_evidence_prefix(present: impl Fn(&str) -> bool) -> &'static str {
    // A partial account policy must not silently borrow legacy values.
    if [
        "PROVIDER_SIGNERS",
        "GATEWAY_SIGNERS",
        "ACCEPTED_MODELS",
        "MIN_REQUEST_BYTES",
    ]
    .iter()
    .any(|suffix| {
        present(&format!(
            "TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_{suffix}"
        ))
    }) {
        "TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE"
    } else {
        "TRACE_COMMONS_ADMISSION"
    }
}

fn account_config_from_values(
    durable_db: bool,
    read: impl Fn(&str) -> Result<String, std::env::VarError>,
    providers: impl FnOnce() -> Result<
        AdmissionProviderTrust,
        trace_commons_server::admission_evidence::AdmissionEvidenceError,
    >,
) -> anyhow::Result<Option<AccountAdmissionConfig>> {
    match read("TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("false") | Ok("0") => return Ok(None),
        Ok("true") | Ok("1") => {}
        _ => anyhow::bail!("account_admission_activation_invalid"),
    }
    if !durable_db {
        anyhow::bail!("account_admission_requires_durable_database");
    }
    let raw = read("TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_JSON")
        .map_err(|_| anyhow::anyhow!("account_admission_policy_missing"))?;
    let version = read("TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_VERSION")
        .map_err(|_| anyhow::anyhow!("account_admission_policy_version_missing"))?;
    let policy = parse_bounded_policy(&raw, &[&version])
        .map_err(|_| anyhow::anyhow!("account_admission_policy_invalid"))?;
    let lease_seconds = read("TRACE_COMMONS_ACCOUNT_ADMISSION_LEASE_SECONDS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|v| (1..=86400).contains(v))
        .ok_or_else(|| anyhow::anyhow!("account_admission_lease_invalid"))?;
    let providers = Some(
        providers().map_err(|_| anyhow::anyhow!("account_admission_evidence_policy_invalid"))?,
    );
    Ok(Some(AccountAdmissionConfig {
        policy,
        lease_seconds,
        providers,
    }))
}

pub(super) fn config_from_env(
    witness: Option<&WitnessBypassConfig>,
    durable_db: bool,
) -> anyhow::Result<Option<AdmissionConfig>> {
    let Some(limits) = AdmissionLimits::from_env().map_err(anyhow::Error::msg)? else {
        return Ok(None);
    };
    if witness.is_none() || !durable_db {
        anyhow::bail!("admission_requires_witness_and_durable_database");
    }
    let providers = AdmissionProviderTrust::from_env("TRACE_COMMONS_ADMISSION")
        .map_err(|_| anyhow::anyhow!("admission_provider_policy_missing_or_invalid"))?;
    Ok(Some(AdmissionConfig { limits, providers }))
}

/// The one refusal every fail-closed path here answers with.
///
/// Spelled from the protocol's own constant rather than a literal. A client
/// separates a declined contribution from an expired credential by this
/// label, so a rename that moved only one side would put that back into
/// reading the status alone.
fn denied() -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::FORBIDDEN, AdmissionRefusal::Refused.label())
}

/// This namespace is allocated only by verified NEAR provisioning. Both the
/// tenant and the principal come from authentication, never envelope attribution.
///
/// The `near-` prefix says only which namespace the tenant is in; the stored
/// row is the authorisation. It is reached by the authenticated tenant and the
/// authenticated principal together, and only through an unrevoked
/// `near`-origin device key on a linked principal of an open account, so a
/// tenant with no such row -- or one whose device has since been revoked --
/// has no anchor and is refused here.
///
/// The suffix is deliberately NOT compared against the anchor. V58 held them
/// equal, which made a contributor's tenant id computable offline from a NEAR
/// account name; V61 made `anchor_hash` a keyed blind index and the tenant id
/// random precisely so it no longer is (#716, #783). Reintroducing any
/// relationship between the two would undo that migration.
pub(super) async fn anchor(state: &AppState, tenant: &TenantCtx) -> ApiResult<Option<String>> {
    // Two namespaces, one lookup. `near-` is wallet provisioning; `nearai-` is
    // a NEAR AI login (#836), which anchors an account the contributor already
    // has because their receipts come from it. Both store the same row shape,
    // so everything below this test is unchanged -- what differs is the
    // preimage domain the anchor was computed under, which is what keeps a
    // value from one from ever being a value from the other.
    //
    // A tenant in neither namespace is not refused, it is `None`: this is the
    // legacy invite-free path. Global account cutover refuses unlinked identities.
    if !RESERVED_ACCOUNT_TENANT_PREFIXES
        .iter()
        .any(|prefix| tenant.tenant_id().strip_prefix(prefix).is_some_and(is_hash))
    {
        return Ok(None);
    }
    let db = state.db_mirror.as_ref().ok_or_else(denied)?;
    let stored = db
        .get_near_provisioned_anchor(tenant.tenant_id(), tenant.principal_ref())
        .await
        .map_err(|_| denied())?
        .ok_or_else(denied)?;
    let stored = stored.strip_prefix("sha256:").ok_or_else(denied)?;
    if !is_hash(stored) {
        return Err(denied());
    }
    Ok(Some(stored.to_string()))
}

/// What a request's admission headers demand, decided from the headers alone.
///
/// Pure on purpose. The refusal below is what closes the V59 trial window,
/// and the RLS matrix that exercises it end to end
/// (`admission_pg_tests::actual_postgres_challenge_witness_ingest_and_terminal_retry`)
/// needs an isolated PostgreSQL and is `#[ignore]`d, so CI never runs it. This
/// seam is what stands behind the refusal in a plain `cargo test --workspace`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EvidencePlan {
    /// At least one evidence header is present: verify it or refuse.
    Verify,
    /// No evidence headers at all.
    ///
    /// NEAR account bootstrap establishes identity, not invitation authority.
    /// Every new contribution on this path needs receipt-bound evidence; the
    /// historical trial-window budget must not authorize ordinary data.
    RefuseNewSubmission,
}

pub(super) fn evidence_plan(headers: &HeaderMap) -> EvidencePlan {
    if headers.contains_key(EVIDENCE_HEADER) || headers.contains_key(SIGNATURE_HEADER) {
        EvidencePlan::Verify
    } else {
        EvidencePlan::RefuseNewSubmission
    }
}

/// The evidence hashes a new submission's reservation carries, or a refusal.
///
/// This is the whole of the trial-window closure, in one pure function so it
/// can be tested without a database. `verified` is `Some` only after
/// [`verify_admission_evidence`] has accepted a receipt bound to this
/// account's challenge; there is no third case, and in particular no case
/// that yields a `window`-kind reservation. Reverting that is an edit to this
/// function, and `an_unverified_request_never_binds_a_reservation` fails.
pub(super) fn evidence_binding(
    plan: EvidencePlan,
    verified: Option<(String, String)>,
) -> ApiResult<(Option<String>, Option<String>)> {
    match (plan, verified) {
        (EvidencePlan::Verify, Some((receipt, challenge))) => Ok((Some(receipt), Some(challenge))),
        _ => Err(denied()),
    }
}

pub(super) struct Attempt {
    tenant_id: String,
    submission_id: Uuid,
    lease_id: Uuid,
    _guard: AdmissionProcessingGuard,
    processing: bool,
    completed: bool,
    account: Option<(TrustAccount, String)>,
    pub(super) source_claim: Option<(Uuid, [u8; 32])>,
}
impl Attempt {
    pub fn is_completed(&self) -> bool {
        self.completed
    }

    pub async fn processing(&mut self, db: &dyn Database) -> ApiResult<()> {
        if self.processing || self.completed {
            return Ok(());
        }
        let transitioned = if let Some((account, principal)) = &self.account {
            db.transition_account_admission(
                &self.tenant_id,
                principal,
                account.account_id(),
                self.submission_id,
                self.lease_id,
                "processing",
            )
            .await
        } else {
            db.transition_submission_admission(
                &self.tenant_id,
                self.submission_id,
                self.lease_id,
                "processing",
            )
            .await
        };
        if !transitioned.map_err(|_| denied())? {
            return Err(denied());
        }
        self.processing = true;
        Ok(())
    }
    pub async fn finish(&mut self, db: &dyn Database, success: bool) -> ApiResult<()> {
        if self.completed {
            return Ok(());
        }
        if success {
            self.processing(db).await?;
        }
        let next = if success {
            "completed"
        } else if !self.processing {
            "released"
        } else {
            return Ok(());
        };
        let transitioned = if let Some((account, principal)) = &self.account {
            db.transition_account_admission(
                &self.tenant_id,
                principal,
                account.account_id(),
                self.submission_id,
                self.lease_id,
                next,
            )
            .await
        } else {
            db.transition_submission_admission(
                &self.tenant_id,
                self.submission_id,
                self.lease_id,
                next,
            )
            .await
        };
        if !transitioned.map_err(|_| denied())? {
            return Err(denied());
        }
        Ok(())
    }
}

pub(super) async fn reserve(
    state: &AppState,
    tenant: &TenantCtx,
    headers: &HeaderMap,
    body: &[u8],
    envelope: &TraceContributionEnvelope,
) -> ApiResult<Option<Attempt>> {
    let submission = envelope.submission_id;
    if let Some(config) = state.account_admission.as_ref() {
        return reserve_account(state, tenant, headers, body, envelope, config)
            .await
            .map(Some);
    }
    let Some(anchor) = anchor(state, tenant).await? else {
        return Ok(None);
    };
    let config = state.admission.as_ref().ok_or_else(denied)?;
    if !state.require_db_mirror_writes {
        return Err(denied());
    }
    let db = state.db_mirror.as_ref().ok_or_else(denied)?;
    let plan = evidence_plan(headers);
    let body_hash = hash_hex(body);
    // A terminal retry is a read of an already-admitted immutable request.
    // Short-lived evidence may now be expired or absent; no new work or
    // authorization is derived from those retry headers. The handler still
    // checks ownership before returning the existing receipt.
    //
    // Decided before the lock. Without evidence headers, a request that is not
    // already a completed retry has a fixed outcome, and every rejected probe
    // from a provisioned account would otherwise cost an advisory lock and a
    // pooled connection held for the rest of this function.
    if plan == EvidencePlan::RefuseNewSubmission
        && !db
            .lookup_completed_submission_admission(
                tenant.tenant_id(),
                &anchor,
                submission,
                &body_hash,
            )
            .await
            .map_err(|_| denied())?
    {
        return Err(denied());
    }
    let guard = db
        .acquire_admission_processing_lock(tenant.tenant_id(), submission)
        .await
        .map_err(|_| denied())?
        .ok_or_else(|| api_error(StatusCode::CONFLICT, AdmissionRefusal::InProgress.label()))?;
    // Re-read under the lock: the check above was advisory and unserialized.
    if db
        .lookup_completed_submission_admission(tenant.tenant_id(), &anchor, submission, &body_hash)
        .await
        .map_err(|_| denied())?
    {
        return Ok(Some(Attempt {
            tenant_id: tenant.tenant_id().into(),
            submission_id: submission,
            lease_id: Uuid::new_v4(),
            _guard: guard,
            processing: false,
            completed: true,
            account: None,
            source_claim: None,
        }));
    }
    let verified = match plan {
        EvidencePlan::Verify => {
            let read = |name: &str| -> ApiResult<&str> {
                let values = headers.get_all(name);
                if values.iter().count() != 1 {
                    return Err(denied());
                }
                let value = values
                    .iter()
                    .next()
                    .ok_or_else(denied)?
                    .to_str()
                    .map_err(|_| denied())?;
                if value.len() > 8192 {
                    return Err(denied());
                }
                Ok(value)
            };
            let evidence: AdmissionEvidence =
                serde_json::from_str(read(EVIDENCE_HEADER)?).map_err(|_| denied())?;
            let witness =
                verified_witness_for_submission(state, headers, body).ok_or_else(denied)?;
            let bypass = state.witness_bypass.as_ref().ok_or_else(denied)?;
            if !bypass.policy_version_allowed(witness.redaction_policy_version()) {
                return Err(denied());
            }
            verify_admission_evidence(
                &evidence,
                read(SIGNATURE_HEADER)?,
                &witness,
                bypass.pin(),
                &config.providers,
                &anchor,
                Utc::now().timestamp(),
            )
            .map_err(|_| denied())?;
            Some((evidence.receipt_sha256, evidence.challenge_sha256))
        }
        EvidencePlan::RefuseNewSubmission => None,
    };
    let (receipt_hash, challenge_hash) = evidence_binding(plan, verified)?;
    let reservation = AdmissionReservation {
        tenant_id: tenant.tenant_id().to_string(),
        anchor_hash: anchor,
        submission_id: submission,
        body_hash,
        receipt_hash,
        challenge_hash,
        lease_id: Uuid::new_v4(),
        limits: config.limits.clone(),
    };
    let decision = db
        .reserve_submission_admission(&reservation)
        .await
        .map_err(|_| denied())?;
    let completed = match decision {
        AdmissionDecision::Reserved => false,
        AdmissionDecision::Completed => true,
        AdmissionDecision::Busy => {
            return Err(api_error(
                StatusCode::CONFLICT,
                AdmissionRefusal::InProgress.label(),
            ));
        }
        AdmissionDecision::Conflict => {
            return Err(api_error(
                StatusCode::CONFLICT,
                AdmissionRefusal::IdentityConflict.label(),
            ));
        }
        AdmissionDecision::Exhausted => {
            return Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                AdmissionRefusal::LimitReached.label(),
            ));
        }
        AdmissionDecision::Refused => return Err(denied()),
    };
    Ok(Some(Attempt {
        tenant_id: reservation.tenant_id,
        submission_id: reservation.submission_id,
        lease_id: reservation.lease_id,
        _guard: guard,
        processing: false,
        completed,
        account: None,
        source_claim: None,
    }))
}

async fn reserve_account(
    state: &AppState,
    tenant: &TenantCtx,
    headers: &HeaderMap,
    body: &[u8],
    envelope: &TraceContributionEnvelope,
    config: &AccountAdmissionConfig,
) -> ApiResult<Attempt> {
    let submission = envelope.submission_id;
    if !state.require_db_mirror_writes {
        return Err(denied());
    }
    let db = state.db_mirror.as_ref().ok_or_else(denied)?;
    let account =
        resolve_contribution_account(db.as_ref(), tenant.tenant_id(), tenant.principal_ref())
            .await
            .map_err(|refusal| match refusal {
                trace_commons_server::account_trust::TrustRefusal::Unlinked => api_error(
                    StatusCode::FORBIDDEN,
                    AdmissionRefusal::AccountIdentityUnlinked.label(),
                ),
                _ => denied(),
            })?;
    if let Some(existing) = tenant
        .read_submission_record(&state.root, submission)
        .map_err(internal_error)?
    {
        if !tenant.can_access_submission(&existing) {
            return Err(api_error(
                StatusCode::CONFLICT,
                "admission_identity_conflict",
            ));
        }
    }
    let guard = db
        .acquire_admission_processing_lock(tenant.tenant_id(), submission)
        .await
        .map_err(|_| denied())?
        .ok_or_else(|| api_error(StatusCode::CONFLICT, AdmissionRefusal::InProgress.label()))?;
    if db
        .get_trace_withdrawal(tenant.tenant_id(), submission)
        .await
        .map_err(|_| denied())?
        .is_some()
    {
        return Err(api_error(StatusCode::CONFLICT, "source_session_withdrawn"));
    }
    // The shared guard serializes both admission modes. Read *after* taking
    // it: a legacy worker may have completed while this request waited.
    let legacy_anchor = anchor(state, tenant).await?.ok_or_else(denied)?;
    if let Some(legacy) = db
        .legacy_admission_record(tenant.tenant_id(), submission)
        .await
        .map_err(|_| denied())?
    {
        if legacy.anchor_hash != legacy_anchor || legacy.body_hash != hash_hex(body) {
            return Err(api_error(
                StatusCode::CONFLICT,
                AdmissionRefusal::IdentityConflict.label(),
            ));
        }
        if legacy.status == "completed" {
            // The submit handler still checks receipt ownership.
            return Ok(Attempt {
                tenant_id: tenant.tenant_id().into(),
                submission_id: submission,
                lease_id: Uuid::new_v4(),
                _guard: guard,
                processing: false,
                completed: true,
                account: None,
                source_claim: None,
            });
        }
        // A replay uses the original V59 authority. Offered evidence cannot
        // replace that authority: verify its original signed binding without
        // imposing the now-expired first-use time window. A headerless replay
        // derives no new proof and is bound by the stored anchor/body instead.
        if evidence_plan(headers) == EvidencePlan::Verify {
            let read = |name: &str| -> ApiResult<&str> {
                let values = headers.get_all(name);
                if values.iter().count() != 1 {
                    return Err(denied());
                }
                let value = values
                    .iter()
                    .next()
                    .ok_or_else(denied)?
                    .to_str()
                    .map_err(|_| denied())?;
                if value.len() > 8192 {
                    return Err(denied());
                }
                Ok(value)
            };
            let evidence: AdmissionEvidence =
                serde_json::from_str(read(EVIDENCE_HEADER)?).map_err(|_| denied())?;
            let pin = state
                .witness_capture_pin
                .as_ref()
                .or_else(|| state.witness_bypass.as_ref().map(WitnessBypassConfig::pin))
                .ok_or_else(denied)?;
            verify_stored_admission_signature(&evidence, read(SIGNATURE_HEADER)?, pin)
                .map_err(|_| denied())?;
            if evidence.account_anchor_sha256 != legacy_anchor
                || legacy.receipt_hash.as_deref() != Some(evidence.receipt_sha256.as_str())
                || legacy.challenge_hash.as_deref() != Some(evidence.challenge_sha256.as_str())
                || evidence.artifact_sha256 != hash_hex(body)
            {
                return Err(denied());
            }
        }
        let lease = Uuid::new_v4();
        let decision = db
            .resume_legacy_admission(
                tenant.tenant_id(),
                &legacy_anchor,
                submission,
                &hash_hex(body),
                lease,
                config.lease_seconds,
            )
            .await
            .map_err(|_| denied())?;
        let completed = match decision {
            AdmissionDecision::Reserved => false,
            AdmissionDecision::Completed => true,
            AdmissionDecision::Busy => {
                return Err(api_error(
                    StatusCode::CONFLICT,
                    AdmissionRefusal::InProgress.label(),
                ));
            }
            AdmissionDecision::Conflict => {
                return Err(api_error(
                    StatusCode::CONFLICT,
                    AdmissionRefusal::IdentityConflict.label(),
                ));
            }
            AdmissionDecision::Exhausted => {
                return Err(api_error(
                    StatusCode::TOO_MANY_REQUESTS,
                    AdmissionRefusal::LimitReached.label(),
                ));
            }
            AdmissionDecision::Refused => return Err(denied()),
        };
        return Ok(Attempt {
            tenant_id: tenant.tenant_id().into(),
            submission_id: submission,
            lease_id: lease,
            _guard: guard,
            processing: false,
            completed,
            account: None,
            source_claim: None,
        });
    }
    // Under the shared processing guard, only an existing exact binding may
    // relax first-use evidence time limits. Reserve rechecks live identity,
    // lease state and budget atomically after proof validation.
    let body_hash = hash_hex(body);
    let account_replay = if let Some(prior) = db
        .account_admission_record(tenant.tenant_id(), submission)
        .await
        .map_err(|_| denied())?
    {
        if prior.account_id != account.account_id() || prior.body_hash != body_hash {
            return Err(api_error(
                StatusCode::CONFLICT,
                AdmissionRefusal::IdentityConflict.label(),
            ));
        }
        true
    } else {
        false
    };
    // Only an exact existing V59 identity can omit source metadata. That
    // branch above retains its original submission-level withdrawal guarantee.
    let source = envelope
        .source_session
        .as_ref()
        .ok_or_else(|| api_error(StatusCode::UNPROCESSABLE_ENTITY, "source_session_invalid"))?;
    let source = canonical_source_session(source)
        .map_err(|_| api_error(StatusCode::UNPROCESSABLE_ENTITY, "source_session_invalid"))?;
    let digest = session_digest(&source);
    let source_status = db
        .claim_trace_source_session(
            tenant.tenant_id(),
            account.account_id(),
            &digest,
            submission,
        )
        .await
        .map_err(|error| match error {
            DatabaseError::Query(ref label) if label == "TraceSourceSessionConflict" => {
                api_error(StatusCode::CONFLICT, "source_session_conflict")
            }
            _ => api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "source_session_unavailable",
            ),
        })?;
    if source_status == StorageTraceSourceSessionStatus::Withdrawn {
        return Err(api_error(StatusCode::CONFLICT, "source_session_withdrawn"));
    }
    let plan = evidence_plan(headers);
    if plan == EvidencePlan::Verify {
        // An offered legacy proof is always checked, even though account trust
        // is the new authority. A partial or malformed header cannot be ignored.
        let read = |name: &str| -> ApiResult<&str> {
            let values = headers.get_all(name);
            if values.iter().count() != 1 {
                return Err(denied());
            }
            let value = values
                .iter()
                .next()
                .ok_or_else(denied)?
                .to_str()
                .map_err(|_| denied())?;
            if value.len() > 8192 {
                return Err(denied());
            }
            Ok(value)
        };
        let evidence: AdmissionEvidence =
            serde_json::from_str(read(EVIDENCE_HEADER)?).map_err(|_| denied())?;
        if account_replay {
            let pin = state
                .witness_capture_pin
                .as_ref()
                .or_else(|| state.witness_bypass.as_ref().map(WitnessBypassConfig::pin))
                .ok_or_else(denied)?;
            verify_stored_admission_signature(&evidence, read(SIGNATURE_HEADER)?, pin)
                .map_err(|_| denied())?;
            if evidence.account_anchor_sha256 != legacy_anchor
                || evidence.artifact_sha256 != body_hash
            {
                return Err(denied());
            }
        } else {
            let providers = config.providers.as_ref().ok_or_else(denied)?;
            let witness =
                verified_witness_for_submission(state, headers, body).ok_or_else(denied)?;
            let bypass = state.witness_bypass.as_ref().ok_or_else(denied)?;
            if !bypass.policy_version_allowed(witness.redaction_policy_version()) {
                return Err(denied());
            }
            verify_admission_evidence(
                &evidence,
                read(SIGNATURE_HEADER)?,
                &witness,
                bypass.pin(),
                providers,
                &legacy_anchor,
                Utc::now().timestamp(),
            )
            .map_err(|_| denied())?;
        }
    }
    let reservation = AccountAdmissionReservation {
        account: account.clone(),
        principal_ref: tenant.principal_ref().into(),
        expected_trust_version: None,
        submission_id: submission,
        body_hash,
        lease_id: Uuid::new_v4(),
        policy: config.policy.clone(),
        lease_seconds: config.lease_seconds,
    };
    let decision = db
        .reserve_account_admission(&reservation)
        .await
        .map_err(|_| denied())?;
    let completed = match decision.decision {
        AdmissionDecision::Reserved => false,
        AdmissionDecision::Completed => true,
        AdmissionDecision::Busy => {
            return Err(api_error(
                StatusCode::CONFLICT,
                AdmissionRefusal::InProgress.label(),
            ));
        }
        AdmissionDecision::Conflict => {
            return Err(api_error(
                StatusCode::CONFLICT,
                AdmissionRefusal::IdentityConflict.label(),
            ));
        }
        AdmissionDecision::Exhausted => {
            return Err(api_error_with_retry(
                StatusCode::TOO_MANY_REQUESTS,
                AdmissionRefusal::AccountLimitReached.label(),
                decision.retry_after_seconds,
            ));
        }
        AdmissionDecision::Refused => return Err(denied()),
    };
    Ok(Attempt {
        tenant_id: tenant.tenant_id().into(),
        submission_id: submission,
        lease_id: reservation.lease_id,
        _guard: guard,
        processing: false,
        completed,
        account: Some((account.clone(), tenant.principal_ref().into())),
        source_claim: Some((account.account_id(), digest)),
    })
}

pub(super) async fn account_status_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<axum::response::Response> {
    let mut response = if let Some(config) = state.account_admission.as_ref() {
        if !RESERVED_ACCOUNT_TENANT_PREFIXES
            .iter()
            .any(|prefix| ctx.tenant_id.strip_prefix(prefix).is_some_and(is_hash))
        {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                AdmissionRefusal::AccountIdentityUnlinked.label(),
            ));
        }
        let db = state.db_mirror.as_ref().ok_or_else(denied)?;
        let mut resolved = None;
        for principal in ctx.principal_set.to_vec() {
            if let Ok(account) =
                resolve_contribution_account(db.as_ref(), &ctx.tenant_id, &principal).await
            {
                if account.account_id() == ctx.account_id.as_uuid() {
                    resolved = Some((account, principal));
                    break;
                }
            }
        }
        let (account, principal) = resolved.ok_or_else(denied)?;
        let status = db
            .account_admission_status(&account, &principal, &config.policy)
            .await
            .map_err(|_| denied())?
            .ok_or_else(denied)?;
        Json(serde_json::json!({
            "authority":status.authority,
            "policy_version":status.policy_version,
            "ready":status.ready,
            "refusal_label":if status.ready { None } else { Some(AdmissionRefusal::AccountLimitReached.label()) },
            "retry_after_seconds":status.retry_after_seconds,
        })).into_response()
    } else {
        Json(serde_json::json!({"authority":"legacy_evidence","policy_version":null,"ready":false,"refusal_label":null,"retry_after_seconds":null})).into_response()
    };
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    Ok(response)
}

pub(super) async fn challenge_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<axum::response::Response> {
    use rand::RngCore as _;
    let tenant = authenticate_ctx(&state, &headers)?;
    let key = submit_principal_rate_limit_key(
        tenant.tenant_id(),
        tenant.safe_auth_method(),
        tenant.principal_ref(),
    );
    if !ACCOUNT_RATE_LIMITER.check(&format!("admission-challenge:{key}"), 10) {
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"));
    }
    let config = state.admission.as_ref().ok_or_else(denied)?;
    let anchor = anchor(&state, &tenant).await?.ok_or_else(denied)?;
    let mut nonce = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| denied())?;
    // The native proxy's deliberately bounded ephemeral binding is at most 15m.
    let expires = Utc::now() + Duration::seconds(config.limits.challenge_ttl_seconds.min(900));
    let binding = AdmissionBinding {
        account_anchor_sha256: anchor.clone(),
        nonce_hex: hex::encode(nonce),
        expires_at: expires.timestamp(),
    };
    state
        .db_mirror
        .as_ref()
        .ok_or_else(denied)?
        .issue_admission_challenge(
            tenant.tenant_id(),
            &anchor,
            &binding.digest().map_err(|_| denied())?,
            expires,
        )
        .await
        .map_err(|_| denied())?;
    let mut response = Json(serde_json::json!({"binding":binding.encode().map_err(|_| denied())?,"expires_at":expires.timestamp()})).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        axum::http::header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderName, HeaderValue};

    #[test]
    fn account_configuration_is_default_off_and_requires_independent_evidence_policy() {
        assert_eq!(
            account_evidence_prefix(|_| false),
            "TRACE_COMMONS_ADMISSION"
        );
        assert_eq!(
            account_evidence_prefix(|key| key == "TRACE_COMMONS_ADMISSION_MIN_REQUEST_BYTES"),
            "TRACE_COMMONS_ADMISSION"
        );
        assert_eq!(
            account_evidence_prefix(
                |key| key == "TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE_PROVIDER_SIGNERS"
            ),
            "TRACE_COMMONS_ACCOUNT_ADMISSION_EVIDENCE"
        );
        let valid_policy = r#"{"version":"test","processing_cost_bound":1,"bounded_allowance":10,"period":{"mode":"lifetime"},"growth_rule":"none"}"#;
        let parse = |enabled: Option<&str>,
                     durable,
                     policy: Option<&str>,
                     version: Option<&str>,
                     lease: Option<&str>,
                     evidence| {
            account_config_from_values(
                durable,
                |key| {
                    let value = match key {
                        "TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED" => enabled,
                        "TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_JSON" => policy,
                        "TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_VERSION" => version,
                        "TRACE_COMMONS_ACCOUNT_ADMISSION_LEASE_SECONDS" => lease,
                        _ => panic!("unexpected config lookup"),
                    };
                    value
                        .map(str::to_owned)
                        .ok_or(std::env::VarError::NotPresent)
                },
                || {
                    if evidence {
                        AdmissionProviderTrust::new(Vec::new(), ["11".repeat(32)], Vec::new(), 1)
                    } else {
                        Err(trace_commons_server::admission_evidence::AdmissionEvidenceError)
                    }
                },
            )
        };
        for off in [None, Some("0"), Some("false")] {
            assert!(
                parse(off, false, None, None, None, false)
                    .unwrap()
                    .is_none()
            );
        }
        assert!(
            parse(
                Some("yes"),
                true,
                Some(valid_policy),
                Some("test"),
                Some("1"),
                true
            )
            .is_err()
        );
        assert!(
            parse(
                Some("true"),
                false,
                Some(valid_policy),
                Some("test"),
                Some("1"),
                true
            )
            .is_err()
        );
        assert!(parse(Some("true"), true, None, Some("test"), Some("1"), true).is_err());
        assert!(
            parse(
                Some("true"),
                true,
                Some(valid_policy),
                Some("other"),
                Some("1"),
                true
            )
            .is_err()
        );
        for lease in [None, Some("0"), Some("86401"), Some("invalid")] {
            assert!(
                parse(
                    Some("true"),
                    true,
                    Some(valid_policy),
                    Some("test"),
                    lease,
                    true
                )
                .is_err()
            );
        }
        assert!(
            parse(
                Some("true"),
                true,
                Some(valid_policy),
                Some("test"),
                Some("1"),
                false
            )
            .is_err()
        );
        let configured = parse(
            Some("true"),
            true,
            Some(valid_policy),
            Some("test"),
            Some("1"),
            true,
        )
        .unwrap()
        .unwrap();
        assert!(
            configured.providers.is_some(),
            "account evidence does not depend on legacy minimum-byte environment setting"
        );
    }

    #[test]
    fn account_limit_wire_contract_only_advertises_configured_reset() {
        let (code, Json(fixed)) = api_error_with_retry(
            StatusCode::TOO_MANY_REQUESTS,
            AdmissionRefusal::AccountLimitReached.label(),
            Some(7),
        );
        assert_eq!(code, StatusCode::TOO_MANY_REQUESTS);
        let fixed = serde_json::to_value(fixed).unwrap();
        assert_eq!(fixed["error"], "account_limit_reached");
        assert_eq!(fixed["retry_after_seconds"], 7);
        let (_, Json(lifetime)) = api_error_with_retry(
            StatusCode::TOO_MANY_REQUESTS,
            AdmissionRefusal::AccountLimitReached.label(),
            None,
        );
        assert!(
            serde_json::to_value(lifetime)
                .unwrap()
                .get("retry_after_seconds")
                .is_none()
        );
    }

    /// The V59 trial window is closed, not narrowed: with no evidence headers
    /// there is no reachable path that admits a new submission.
    ///
    /// This runs in a plain `cargo test --workspace`. The RLS matrix that
    /// proves the same thing end to end is `#[ignore]`d for want of an
    /// isolated PostgreSQL, so without this the refusal could be reverted
    /// with `main` staying green.
    #[test]
    fn a_request_with_no_evidence_headers_refuses_a_new_submission() {
        assert_eq!(
            evidence_plan(&HeaderMap::new()),
            EvidencePlan::RefuseNewSubmission
        );
        for name in [EVIDENCE_HEADER, SIGNATURE_HEADER] {
            let mut headers = HeaderMap::new();
            headers.insert(
                HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_static("offered"),
            );
            assert_eq!(
                evidence_plan(&headers),
                EvidencePlan::Verify,
                "{name} alone must still enter the verify branch, which then \
                 fails on the header it is missing"
            );
        }
    }

    /// A NEAR account that bootstrapped its identity has no admission
    /// authority of its own. Before this, an evidence-less request fell
    /// through to the V59 trial-window budget and uploaded ordinary data
    /// under it; the only end-to-end test of that is `#[ignore]`d.
    #[test]
    fn an_unverified_request_never_binds_a_reservation() {
        let receipt = "a".repeat(64);
        let challenge = "b".repeat(64);
        assert!(evidence_binding(EvidencePlan::RefuseNewSubmission, None).is_err());
        assert!(
            evidence_binding(
                EvidencePlan::RefuseNewSubmission,
                Some((receipt.clone(), challenge.clone()))
            )
            .is_err()
        );
        assert!(evidence_binding(EvidencePlan::Verify, None).is_err());
        assert_eq!(
            evidence_binding(
                EvidencePlan::Verify,
                Some((receipt.clone(), challenge.clone()))
            )
            .map_err(|_| ())
            .unwrap(),
            (Some(receipt), Some(challenge)),
            "positive control: verified evidence does bind"
        );
    }
}
