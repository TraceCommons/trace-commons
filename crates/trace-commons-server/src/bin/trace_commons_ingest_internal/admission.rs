// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use trace_commons_protocol::admission::{
    AdmissionBinding, AdmissionEvidence, EVIDENCE_HEADER, SIGNATURE_HEADER, hash_hex, is_hash,
};
use trace_commons_server::admission_evidence::{AdmissionProviderTrust, verify_admission_evidence};
use trace_commons_server::admission_ledger::{
    AdmissionDecision, AdmissionLimits, AdmissionProcessingGuard, AdmissionReservation,
};

#[derive(Clone)]
pub(super) struct AdmissionConfig {
    pub limits: AdmissionLimits,
    pub providers: AdmissionProviderTrust,
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

fn denied() -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::FORBIDDEN, "admission_refused")
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
    if !tenant
        .tenant_id()
        .strip_prefix("near-")
        .is_some_and(is_hash)
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
}
impl Attempt {
    pub fn is_completed(&self) -> bool {
        self.completed
    }

    pub async fn processing(&mut self, db: &dyn Database) -> ApiResult<()> {
        if self.processing || self.completed {
            return Ok(());
        }
        if !db
            .transition_submission_admission(
                &self.tenant_id,
                self.submission_id,
                self.lease_id,
                "processing",
            )
            .await
            .map_err(|_| denied())?
        {
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
        if !db
            .transition_submission_admission(
                &self.tenant_id,
                self.submission_id,
                self.lease_id,
                next,
            )
            .await
            .map_err(|_| denied())?
        {
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
    submission: Uuid,
) -> ApiResult<Option<Attempt>> {
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
        .ok_or_else(|| api_error(StatusCode::CONFLICT, "admission_in_progress"))?;
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
            return Err(api_error(StatusCode::CONFLICT, "admission_in_progress"));
        }
        AdmissionDecision::Conflict => {
            return Err(api_error(
                StatusCode::CONFLICT,
                "admission_identity_conflict",
            ));
        }
        AdmissionDecision::Exhausted => {
            return Err(api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "admission_limit_reached",
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
    }))
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
