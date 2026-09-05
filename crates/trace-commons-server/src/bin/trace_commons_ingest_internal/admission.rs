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
    let signers = std::env::var("TRACE_COMMONS_ADMISSION_PROVIDER_SIGNERS")
        .map_err(|_| anyhow::anyhow!("admission_provider_trust_missing"))?;
    let providers = AdmissionProviderTrust::new(signers.split(',').map(|s| s.trim().to_string()))
        .map_err(|_| anyhow::anyhow!("admission_provider_trust_invalid"))?;
    Ok(Some(AdmissionConfig { limits, providers }))
}

fn denied() -> (StatusCode, Json<ApiError>) {
    api_error(StatusCode::FORBIDDEN, "admission_refused")
}

/// This namespace is allocated only by verified NEAR provisioning. Both the
/// tenant and the principal come from authentication, never envelope attribution.
pub(super) async fn anchor(state: &AppState, tenant: &TenantCtx) -> ApiResult<Option<String>> {
    let Some(candidate) = tenant
        .tenant_id()
        .strip_prefix("near-")
        .filter(|s| is_hash(s))
    else {
        return Ok(None);
    };
    let db = state.db_mirror.as_ref().ok_or_else(denied)?;
    let stored = db
        .get_near_provisioned_anchor(tenant.tenant_id(), tenant.principal_ref())
        .await
        .map_err(|_| denied())?
        .ok_or_else(denied)?;
    let stored = stored.strip_prefix("sha256:").ok_or_else(denied)?;
    if stored != candidate {
        return Err(denied());
    }
    Ok(Some(stored.to_string()))
}

pub(super) struct Attempt {
    reservation: AdmissionReservation,
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
                &self.reservation.tenant_id,
                self.reservation.submission_id,
                self.reservation.lease_id,
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
                &self.reservation.tenant_id,
                self.reservation.submission_id,
                self.reservation.lease_id,
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
    let guard = db
        .acquire_admission_processing_lock(tenant.tenant_id(), submission)
        .await
        .map_err(|_| denied())?
        .ok_or_else(|| api_error(StatusCode::CONFLICT, "admission_in_progress"))?;
    // A terminal retry is a read of an already-admitted immutable request.
    // Short-lived evidence may now be expired or absent; no new work or
    // authorization is derived from those retry headers. The handler still
    // checks ownership before returning the existing receipt.
    let body_hash = hash_hex(body);
    if db
        .lookup_completed_submission_admission(tenant.tenant_id(), &anchor, submission, &body_hash)
        .await
        .map_err(|_| denied())?
    {
        return Ok(Some(Attempt {
            reservation: AdmissionReservation {
                tenant_id: tenant.tenant_id().into(),
                anchor_hash: anchor,
                submission_id: submission,
                body_hash,
                receipt_hash: None,
                challenge_hash: None,
                lease_id: Uuid::new_v4(),
                limits: config.limits.clone(),
            },
            _guard: guard,
            processing: false,
            completed: true,
        }));
    }
    let (receipt_hash, challenge_hash) = if headers.contains_key(EVIDENCE_HEADER)
        || headers.contains_key(SIGNATURE_HEADER)
    {
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
        let witness = verified_witness_for_submission(state, headers, body).ok_or_else(denied)?;
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
        (
            Some(evidence.receipt_sha256),
            Some(evidence.challenge_sha256),
        )
    } else {
        (None, None)
    };
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
        reservation,
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
