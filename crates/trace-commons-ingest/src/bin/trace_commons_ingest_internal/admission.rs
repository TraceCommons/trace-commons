// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use trace_commons_protocol::admission::{
    AdmissionBinding, AdmissionEvidence, AdmissionRefusal, EVIDENCE_HEADER, SIGNATURE_HEADER,
    hash_hex, is_anchored_tenant, is_hash,
};
use trace_commons_server::account_trust::{
    BoundedPolicy, TrustAccount, parse_bounded_policy, resolve_contribution_account,
};
use trace_commons_server::admission_evidence::{
    AdmissionProviderTrust, check_admission_evidence, verify_stored_admission_signature,
};
use trace_commons_server::admission_ledger::{
    AccountAdmissionReservation, AdmissionDecision, AdmissionLimits, AdmissionProcessingGuard,
    AdmissionReservation,
};

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

/// Account admission sends an upload with verified evidence and no
/// `source_session` to the V59 evidence ledger, so it cannot run without that
/// ledger's policy (`TRACE_COMMONS_ADMISSION_*`). Refused at startup rather
/// than per request, naming the missing control.
pub(super) fn require_legacy_evidence_policy(
    account_admission: bool,
    legacy_admission: bool,
) -> anyhow::Result<()> {
    if account_admission && !legacy_admission {
        anyhow::bail!("account_admission_requires_legacy_evidence_policy");
    }
    Ok(())
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
///
/// Every site passes its own fixed label, logged once at `warn` as
/// `admission_refusal`, so an operator can tell which check refused. The
/// label is a `&'static str` and nothing else is logged: no tenant, account,
/// principal, submission, header value or database error text (hash-only
/// logging). The response is unchanged: `403 admission_refused`.
fn denied_at(label: &'static str) -> (StatusCode, Json<ApiError>) {
    trace_commons_server::admission_ledger::log_admission_refusal(label);
    api_error(StatusCode::FORBIDDEN, AdmissionRefusal::Refused.label())
}

/// [`denied_at`], plus which evidence check failed (a label from
/// `ADMISSION_EVIDENCE_CHECK_LABELS`), on the same line.
fn denied_with(label: &'static str, check: &'static str) -> (StatusCode, Json<ApiError>) {
    tracing::warn!(
        admission_refusal = label,
        admission_check = check,
        "admission refused"
    );
    api_error(StatusCode::FORBIDDEN, AdmissionRefusal::Refused.label())
}

/// Which path is reading admission evidence headers, for the refusal label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderSite {
    /// `reserve_on_evidence_ledger`: a new upload with evidence.
    EvidenceLedger,
    /// `reserve_account`, replaying an existing V59 row.
    LegacyReplay,
    /// `reserve_account`, the account ledger with offered evidence.
    Account,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeaderFault {
    Count,
    Missing,
    NotText,
    TooLong,
}

fn header_refusal(site: HeaderSite, evidence: bool, fault: HeaderFault) -> &'static str {
    use HeaderFault::*;
    use HeaderSite::*;
    match (site, evidence, fault) {
        (EvidenceLedger, true, Count) => "evidence_ledger:evidence_header_count",
        (EvidenceLedger, true, Missing) => "evidence_ledger:evidence_header_missing",
        (EvidenceLedger, true, NotText) => "evidence_ledger:evidence_header_not_text",
        (EvidenceLedger, true, TooLong) => "evidence_ledger:evidence_header_too_long",
        (EvidenceLedger, false, Count) => "evidence_ledger:signature_header_count",
        (EvidenceLedger, false, Missing) => "evidence_ledger:signature_header_missing",
        (EvidenceLedger, false, NotText) => "evidence_ledger:signature_header_not_text",
        (EvidenceLedger, false, TooLong) => "evidence_ledger:signature_header_too_long",
        (LegacyReplay, true, Count) => "legacy_replay:evidence_header_count",
        (LegacyReplay, true, Missing) => "legacy_replay:evidence_header_missing",
        (LegacyReplay, true, NotText) => "legacy_replay:evidence_header_not_text",
        (LegacyReplay, true, TooLong) => "legacy_replay:evidence_header_too_long",
        (LegacyReplay, false, Count) => "legacy_replay:signature_header_count",
        (LegacyReplay, false, Missing) => "legacy_replay:signature_header_missing",
        (LegacyReplay, false, NotText) => "legacy_replay:signature_header_not_text",
        (LegacyReplay, false, TooLong) => "legacy_replay:signature_header_too_long",
        (Account, true, Count) => "account:evidence_header_count",
        (Account, true, Missing) => "account:evidence_header_missing",
        (Account, true, NotText) => "account:evidence_header_not_text",
        (Account, true, TooLong) => "account:evidence_header_too_long",
        (Account, false, Count) => "account:signature_header_count",
        (Account, false, Missing) => "account:signature_header_missing",
        (Account, false, NotText) => "account:signature_header_not_text",
        (Account, false, TooLong) => "account:signature_header_too_long",
    }
}

/// Exactly one value of an admission header, as text, at most 8 KiB.
fn single_header<'a>(headers: &'a HeaderMap, name: &str, site: HeaderSite) -> ApiResult<&'a str> {
    let evidence = name == EVIDENCE_HEADER;
    let refuse = |fault| denied_at(header_refusal(site, evidence, fault));
    let values = headers.get_all(name);
    if values.iter().count() != 1 {
        return Err(refuse(HeaderFault::Count));
    }
    let value = values
        .iter()
        .next()
        .ok_or_else(|| refuse(HeaderFault::Missing))?
        .to_str()
        .map_err(|_| refuse(HeaderFault::NotText))?;
    if value.len() > 8192 {
        return Err(refuse(HeaderFault::TooLong));
    }
    Ok(value)
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
    if !is_anchored_tenant(tenant.tenant_id()) {
        return Ok(None);
    }
    let db = state
        .db_mirror
        .as_ref()
        .ok_or_else(|| denied_at("anchor_db_unconfigured"))?;
    let stored = db
        .get_near_provisioned_anchor(tenant.tenant_id(), &tenant.account_principal_ref())
        .await
        .map_err(|_| denied_at("db_error:anchor_lookup"))?
        .ok_or_else(|| denied_at("anchor_missing"))?;
    let stored = stored
        .strip_prefix("sha256:")
        .ok_or_else(|| denied_at("anchor_not_sha256_prefixed"))?;
    if !is_hash(stored) {
        return Err(denied_at("anchor_not_hash"));
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
/// [`check_admission_evidence`] has accepted a receipt bound to this
/// account's challenge; there is no third case, and in particular no case
/// that yields a `window`-kind reservation. Reverting that is an edit to this
/// function, and `an_unverified_request_never_binds_a_reservation` fails.
pub(super) fn evidence_binding(
    plan: EvidencePlan,
    verified: Option<(String, String)>,
) -> ApiResult<(Option<String>, Option<String>)> {
    match (plan, verified) {
        (EvidencePlan::Verify, Some((receipt, challenge))) => Ok((Some(receipt), Some(challenge))),
        _ => Err(denied_at("evidence_binding_unverified")),
    }
}

/// Where account admission sends a submission that has no prior V59 row,
/// decided from three facts alone so the decision is testable without a
/// database (the PostgreSQL matrix is `#[ignore]`d and runs only in CI's
/// `postgres-suites`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AccountRoute {
    /// A `source_session` is present: the account ledger. Offered evidence
    /// is verified there, and a fresh verified proof admits the upload
    /// without debiting the allowance.
    AccountLedger,
    /// No `source_session`, a new submission, and offered evidence: the V59
    /// evidence ledger, verified and bounded exactly as with account
    /// admission off. 0.12.x clients and the 2026-09-04 witness build never
    /// send `source_session`; owner decision of 2026-10-11.
    EvidenceLedger,
    /// No `source_session` otherwise: `422 source_session_invalid`, as before.
    RefuseMissingSource,
}

pub(super) fn account_route(
    source_present: bool,
    plan: EvidencePlan,
    account_replay: bool,
) -> AccountRoute {
    match (source_present, plan, account_replay) {
        (true, _, _) => AccountRoute::AccountLedger,
        (false, EvidencePlan::Verify, false) => AccountRoute::EvidenceLedger,
        (false, _, _) => AccountRoute::RefuseMissingSource,
    }
}

pub(super) struct Attempt {
    tenant_id: String,
    submission_id: Uuid,
    lease_id: Uuid,
    _guard: AdmissionProcessingGuard,
    processing: bool,
    completed: bool,
    /// Released by [`Attempt::finish_without_effect`]: the trailing
    /// `finish` has nothing left to do.
    released: bool,
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
        let transitioned = self.transition(db, "processing").await;
        if !transitioned.map_err(|_| denied_at("db_error:attempt_processing_transition"))? {
            return Err(denied_at("attempt_processing_transition_refused"));
        }
        self.processing = true;
        Ok(())
    }

    /// Releases the attempt, and gives back the cost bound or charge its
    /// reservation took, for an upload refused by a refusal that the caller
    /// KNOWS wrote nothing (#1237). Unlike `finish(false)`, this releases an
    /// attempt that is already `processing`; a `reserved` one is released as
    /// `finish(false)` releases it. A failure whose effects are not known
    /// must call `finish(false)` instead, which keeps a `processing`
    /// attempt's charge.
    ///
    /// A completed or already released attempt is left alone. A ledger that
    /// refuses the release is a refusal here, labelled; the attempt then
    /// stays as it was, as with `finish(false)`.
    pub async fn finish_without_effect(&mut self, db: &dyn Database) -> ApiResult<()> {
        if self.completed || self.released {
            return Ok(());
        }
        let next = if self.processing {
            "released_without_effect"
        } else {
            "released"
        };
        let transitioned = self.transition(db, next).await;
        if !transitioned.map_err(|_| denied_at("db_error:attempt_release_without_effect"))? {
            return Err(denied_at("attempt_release_without_effect_refused"));
        }
        self.released = true;
        Ok(())
    }

    async fn transition(&self, db: &dyn Database, next: &str) -> Result<bool, DatabaseError> {
        if let Some((account, principal)) = &self.account {
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
        }
    }

    pub async fn finish(&mut self, db: &dyn Database, success: bool) -> ApiResult<()> {
        if self.completed || self.released {
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
        let transitioned = self.transition(db, next).await;
        if !transitioned.map_err(|_| denied_at("db_error:attempt_finish_transition"))? {
            return Err(denied_at("attempt_finish_transition_refused"));
        }
        Ok(())
    }
}

/// Whether a staging build lets an anchored tenant's upload through with no
/// admission ledger (staging spec S12: open sign-up, "skip ledger on
/// staging"). A staging build creates NEAR AI accounts without the ledger
/// (`near_provisioning`), so without this every account it creates is
/// refused at its first upload as `legacy_admission_unconfigured`.
///
/// Narrow on purpose: only a `nearai-` tenant (the NEAR AI login namespace;
/// a staging build refuses the wallet ceremony, so it creates no `near-`
/// tenant), only while the ledger is unconfigured (an operator who turns it
/// on gets the full path), and only after [`anchor`] has resolved the
/// tenant's provisioned device. Compiled to `false` in every build without
/// `near-ai-staging`, so a production build never takes it.
#[cfg(feature = "near-ai-staging")]
pub(super) fn staging_admits_without_ledger(tenant_id: &str, ledger_configured: bool) -> bool {
    const NEAR_AI_LOGIN_NAMESPACE: &str = "nearai-";
    debug_assert!(
        trace_commons_protocol::admission::ANCHOR_NAMESPACES.contains(&NEAR_AI_LOGIN_NAMESPACE)
    );
    !ledger_configured
        && tenant_id
            .strip_prefix(NEAR_AI_LOGIN_NAMESPACE)
            .is_some_and(is_hash)
}

/// See the `near-ai-staging` definition. Every other build requires the
/// ledger.
#[cfg(not(feature = "near-ai-staging"))]
pub(super) fn staging_admits_without_ledger(_tenant_id: &str, _ledger_configured: bool) -> bool {
    false
}

pub(super) async fn reserve(
    state: &AppState,
    tenant: &TenantCtx,
    headers: &HeaderMap,
    body: &[u8],
    envelope: &TraceContributionEnvelope,
) -> ApiResult<Option<Attempt>> {
    let submission = envelope.submission_id;
    // Coexistence (V81): account admission governs only the `near-`/`nearai-`
    // namespaces. A legacy `tenant-…` invite identity keeps the path it had
    // before the switch -- `anchor` below returns `None` for it -- whether or
    // not it has been linked to a NEAR account. Startup readiness
    // (`trace_account_admission_linkage_ready`) is what refuses an ambiguous
    // legacy identity; per request there is nothing further to decide.
    if let Some(config) = state
        .account_admission
        .as_ref()
        .filter(|_| is_anchored_tenant(tenant.tenant_id()))
    {
        return reserve_account(state, tenant, headers, body, envelope, config)
            .await
            .map(Some);
    }
    let Some(anchor) = anchor(state, tenant).await? else {
        return Ok(None);
    };
    // After `anchor`, never before it: the provisioned-device row it resolved
    // is what shows this server created the account, so a staging build
    // admits only a tenant its own NEAR AI sign-up provisioned.
    if staging_admits_without_ledger(tenant.tenant_id(), state.admission.is_some()) {
        tracing::info!(
            admission_path = "staging_open_sign_up",
            "admission skipped on a staging build"
        );
        return Ok(None);
    }
    state
        .admission
        .as_ref()
        .ok_or_else(|| denied_at("legacy_admission_unconfigured"))?;
    if !state.require_db_mirror_writes {
        return Err(denied_at("legacy_db_mirror_writes_off"));
    }
    let db = state
        .db_mirror
        .as_ref()
        .ok_or_else(|| denied_at("legacy_db_unconfigured"))?;
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
            .map_err(|_| denied_at("db_error:legacy_completed_lookup"))?
    {
        return Err(denied_at("legacy_no_evidence_headers"));
    }
    let guard = db
        .acquire_admission_processing_lock(tenant.tenant_id(), submission)
        .await
        .map_err(|_| denied_at("db_error:legacy_processing_lock"))?
        .ok_or_else(|| api_error(StatusCode::CONFLICT, AdmissionRefusal::InProgress.label()))?;
    reserve_on_evidence_ledger(state, tenant, headers, body, envelope, anchor, guard)
        .await
        .map(Some)
}

/// The V59 evidence ledger, entered with the shared processing guard already
/// held. Account admission reaches it too, for an upload with verified
/// evidence and no `source_session` (owner decision of 2026-10-11), so that
/// upload is admitted exactly as it is with account admission off.
async fn reserve_on_evidence_ledger(
    state: &AppState,
    tenant: &TenantCtx,
    headers: &HeaderMap,
    body: &[u8],
    envelope: &TraceContributionEnvelope,
    anchor: String,
    guard: AdmissionProcessingGuard,
) -> ApiResult<Attempt> {
    let submission = envelope.submission_id;
    let config = state
        .admission
        .as_ref()
        .ok_or_else(|| denied_at("evidence_ledger_unconfigured"))?;
    let db = state
        .db_mirror
        .as_ref()
        .ok_or_else(|| denied_at("evidence_ledger_db_unconfigured"))?;
    let plan = evidence_plan(headers);
    let body_hash = hash_hex(body);
    // Re-read under the lock: any check before it was advisory and unserialized.
    if db
        .lookup_completed_submission_admission(tenant.tenant_id(), &anchor, submission, &body_hash)
        .await
        .map_err(|_| denied_at("db_error:evidence_ledger_completed_lookup"))?
    {
        return Ok(Attempt {
            tenant_id: tenant.tenant_id().into(),
            submission_id: submission,
            lease_id: Uuid::new_v4(),
            _guard: guard,
            released: false,
            processing: false,
            completed: true,
            account: None,
            source_claim: None,
        });
    }
    let verified = match plan {
        EvidencePlan::Verify => {
            let read = |name: &str| single_header(headers, name, HeaderSite::EvidenceLedger);
            let evidence: AdmissionEvidence = serde_json::from_str(read(EVIDENCE_HEADER)?)
                .map_err(|_| denied_at("evidence_ledger:evidence_json_invalid"))?;
            let witness = verified_witness_for_submission(state, headers, body)
                .ok_or_else(|| denied_at("evidence_ledger:witness_unverified"))?;
            let bypass = state
                .witness_bypass
                .as_ref()
                .ok_or_else(|| denied_at("evidence_ledger:witness_bypass_unconfigured"))?;
            if !bypass.policy_version_allowed(witness.redaction_policy_version()) {
                return Err(denied_at("evidence_ledger:policy_version_not_allowed"));
            }
            check_admission_evidence(
                &evidence,
                read(SIGNATURE_HEADER)?,
                &witness,
                bypass.pin(),
                &config.providers,
                &anchor,
                Utc::now().timestamp(),
            )
            .map_err(|check| denied_with("evidence_ledger:evidence_unverified", check))?;
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
        .map_err(|_| denied_at("db_error:evidence_ledger_reserve"))?;
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
        AdmissionDecision::Refused => return Err(denied_at("evidence_ledger_refused")),
    };
    Ok(Attempt {
        tenant_id: reservation.tenant_id,
        submission_id: reservation.submission_id,
        lease_id: reservation.lease_id,
        _guard: guard,
        released: false,
        processing: false,
        completed,
        account: None,
        source_claim: None,
    })
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
        return Err(denied_at("account_db_mirror_writes_off"));
    }
    let db = state
        .db_mirror
        .as_ref()
        .ok_or_else(|| denied_at("account_db_unconfigured"))?;
    // The device principal for a device-key claim, not the claim's own
    // signed-claim principal: provisioning keys the account, the anchor and
    // the ledger's live-device check by it (`TenantCtx::account_principal_ref`).
    let principal = tenant.account_principal_ref();
    let account = resolve_contribution_account(db.as_ref(), tenant.tenant_id(), &principal)
        .await
        .map_err(|refusal| match refusal {
            trace_commons_server::account_trust::TrustRefusal::Unlinked => api_error(
                StatusCode::FORBIDDEN,
                AdmissionRefusal::AccountIdentityUnlinked.label(),
            ),
            trace_commons_server::account_trust::TrustRefusal::Refused => {
                denied_at("account_trust_refused")
            }
            trace_commons_server::account_trust::TrustRefusal::Unavailable => {
                denied_at("account_trust_unavailable")
            }
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
        .map_err(|_| denied_at("db_error:account_processing_lock"))?
        .ok_or_else(|| api_error(StatusCode::CONFLICT, AdmissionRefusal::InProgress.label()))?;
    if db
        .get_trace_withdrawal(tenant.tenant_id(), submission)
        .await
        .map_err(|_| denied_at("db_error:account_withdrawal_lookup"))?
        .is_some()
    {
        // A submission-level tombstone, with or without a source session.
        return Err(api_error(StatusCode::CONFLICT, "submission_withdrawn"));
    }
    // The shared guard serializes both admission modes. Read *after* taking
    // it: a legacy worker may have completed while this request waited.
    let legacy_anchor = anchor(state, tenant)
        .await?
        .ok_or_else(|| denied_at("account_anchor_none"))?;
    if let Some(legacy) = db
        .legacy_admission_record(tenant.tenant_id(), submission)
        .await
        .map_err(|_| denied_at("db_error:legacy_record_lookup"))?
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
                released: false,
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
            let read = |name: &str| single_header(headers, name, HeaderSite::LegacyReplay);
            let evidence: AdmissionEvidence = serde_json::from_str(read(EVIDENCE_HEADER)?)
                .map_err(|_| denied_at("legacy_replay:evidence_json_invalid"))?;
            let pin = state
                .witness_capture_pin
                .as_ref()
                .or_else(|| state.witness_bypass.as_ref().map(WitnessBypassConfig::pin))
                .ok_or_else(|| denied_at("legacy_replay:witness_pin_unconfigured"))?;
            verify_stored_admission_signature(&evidence, read(SIGNATURE_HEADER)?, pin)
                .map_err(|_| denied_at("legacy_replay:stored_signature_invalid"))?;
            if evidence.account_anchor_sha256 != legacy_anchor
                || legacy.receipt_hash.as_deref() != Some(evidence.receipt_sha256.as_str())
                || legacy.challenge_hash.as_deref() != Some(evidence.challenge_sha256.as_str())
                || evidence.artifact_sha256 != hash_hex(body)
            {
                return Err(denied_at("legacy_replay:binding_mismatch"));
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
            .map_err(|_| denied_at("db_error:legacy_resume"))?;
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
            AdmissionDecision::Refused => return Err(denied_at("legacy_resume_refused")),
        };
        return Ok(Attempt {
            tenant_id: tenant.tenant_id().into(),
            submission_id: submission,
            lease_id: lease,
            _guard: guard,
            released: false,
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
        .map_err(|_| denied_at("db_error:account_record_lookup"))?
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
    // An exact existing V59 identity can omit source metadata (the branch
    // above), and so can a new upload offering evidence, which goes to the
    // V59 ledger. Both keep the submission-level withdrawal check above.
    let plan = evidence_plan(headers);
    match account_route(envelope.source_session.is_some(), plan, account_replay) {
        AccountRoute::AccountLedger => {}
        AccountRoute::EvidenceLedger => {
            return reserve_on_evidence_ledger(
                state,
                tenant,
                headers,
                body,
                envelope,
                legacy_anchor,
                guard,
            )
            .await;
        }
        AccountRoute::RefuseMissingSource => {
            return Err(api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "source_session_invalid",
            ));
        }
    }
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
    // Set only by a fresh, full verification below. A replay's stored-signature
    // check does not re-verify the provider, model or freshness, so it never
    // sets this; the ledger keeps whatever kind the first reservation recorded.
    let mut attested = false;
    if plan == EvidencePlan::Verify {
        // An offered legacy proof is always checked, even though account trust
        // is the new authority. A partial or malformed header cannot be ignored.
        let read = |name: &str| single_header(headers, name, HeaderSite::Account);
        let evidence: AdmissionEvidence = serde_json::from_str(read(EVIDENCE_HEADER)?)
            .map_err(|_| denied_at("account:evidence_json_invalid"))?;
        if account_replay {
            let pin = state
                .witness_capture_pin
                .as_ref()
                .or_else(|| state.witness_bypass.as_ref().map(WitnessBypassConfig::pin))
                .ok_or_else(|| denied_at("account_replay:witness_pin_unconfigured"))?;
            verify_stored_admission_signature(&evidence, read(SIGNATURE_HEADER)?, pin)
                .map_err(|_| denied_at("account_replay:stored_signature_invalid"))?;
            if evidence.account_anchor_sha256 != legacy_anchor
                || evidence.artifact_sha256 != body_hash
            {
                return Err(denied_at("account_replay:binding_mismatch"));
            }
        } else {
            let providers = config
                .providers
                .as_ref()
                .ok_or_else(|| denied_at("account:evidence_providers_unconfigured"))?;
            let witness = verified_witness_for_submission(state, headers, body)
                .ok_or_else(|| denied_at("account:witness_unverified"))?;
            let bypass = state
                .witness_bypass
                .as_ref()
                .ok_or_else(|| denied_at("account:witness_bypass_unconfigured"))?;
            if !bypass.policy_version_allowed(witness.redaction_policy_version()) {
                return Err(denied_at("account:policy_version_not_allowed"));
            }
            check_admission_evidence(
                &evidence,
                read(SIGNATURE_HEADER)?,
                &witness,
                bypass.pin(),
                providers,
                &legacy_anchor,
                Utc::now().timestamp(),
            )
            .map_err(|check| denied_with("account:evidence_unverified", check))?;
            attested = true;
        }
    }
    let reservation = AccountAdmissionReservation {
        account: account.clone(),
        principal_ref: principal.clone(),
        expected_trust_version: None,
        submission_id: submission,
        body_hash,
        lease_id: Uuid::new_v4(),
        policy: config.policy.clone(),
        lease_seconds: config.lease_seconds,
        attested,
    };
    let decision = db
        .reserve_account_admission(&reservation)
        .await
        .map_err(|_| denied_at("db_error:account_reserve"))?;
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
            // Earned-trust shadow calibration only: counts a refusal the
            // candidate growth policy would have admitted. It cannot change
            // this refusal, and any failure in it is ignored.
            if let Some(spend) = decision.period_spend {
                super::account_trust_growth_routes::observe_shadow_would_admit(
                    state, &account, spend,
                )
                .await;
            }
            return Err(api_error_with_retry(
                StatusCode::TOO_MANY_REQUESTS,
                AdmissionRefusal::AccountLimitReached.label(),
                decision.retry_after_seconds,
            ));
        }
        AdmissionDecision::Refused => return Err(denied_at("account_ledger_refused")),
    };
    Ok(Attempt {
        tenant_id: tenant.tenant_id().into(),
        submission_id: submission,
        lease_id: reservation.lease_id,
        _guard: guard,
        released: false,
        processing: false,
        completed,
        account: Some((account.clone(), principal)),
        source_claim: Some((account.account_id(), digest)),
    })
}

pub(super) async fn account_status_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<axum::response::Response> {
    let mut response = if let Some(config) = state.account_admission.as_ref() {
        if !is_anchored_tenant(&ctx.tenant_id) {
            return Err(api_error(
                StatusCode::FORBIDDEN,
                AdmissionRefusal::AccountIdentityUnlinked.label(),
            ));
        }
        let db = state
            .db_mirror
            .as_ref()
            .ok_or_else(|| denied_at("status_db_unconfigured"))?;
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
        let (account, principal) =
            resolved.ok_or_else(|| denied_at("status_account_unresolved"))?;
        let status = db
            .account_admission_status(&account, &principal, &config.policy)
            .await
            .map_err(|_| denied_at("db_error:status_lookup"))?
            .ok_or_else(|| denied_at("status_missing"))?;
        let mut body = serde_json::json!({
            "authority":status.authority,
            "policy_version":status.policy_version,
            "ready":status.ready,
            "refusal_label":if status.ready { None } else { Some(AdmissionRefusal::AccountLimitReached.label()) },
            "retry_after_seconds":status.retry_after_seconds,
        });
        // Spec rule 7: under tiered-v1, the account's own growth view beside
        // `authority:"bounded"`. Absent otherwise, so `none` answers as before.
        if let (Some(growth), Some(policy), Some(fields)) = (
            status.growth.as_ref(),
            state.account_trust_applied_policy.as_deref(),
            body.as_object_mut(),
        ) {
            let view =
                trace_commons_server::account_trust_growth::growth_status_view(policy, growth);
            if let Ok(serde_json::Value::Object(extra)) = serde_json::to_value(view) {
                fields.extend(extra);
            }
        }
        Json(body).into_response()
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
    if !ACCOUNT_RATE_LIMITER.check_principal(&format!("admission-challenge:{key}"), 10) {
        return Err(api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited"));
    }
    let config = state
        .admission
        .as_ref()
        .ok_or_else(|| denied_at("challenge_unconfigured"))?;
    let anchor = anchor(&state, &tenant)
        .await?
        .ok_or_else(|| denied_at("challenge_anchor_none"))?;
    let mut nonce = [0u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| denied_at("challenge_nonce_unavailable"))?;
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
        .ok_or_else(|| denied_at("challenge_db_unconfigured"))?
        .issue_admission_challenge(
            tenant.tenant_id(),
            &anchor,
            &binding
                .digest()
                .map_err(|_| denied_at("challenge_digest_invalid"))?,
            expires,
        )
        .await
        .map_err(|_| denied_at("db_error:challenge_issue"))?;
    let mut response = Json(serde_json::json!({"binding":binding.encode().map_err(|_| denied_at("challenge_encode_invalid"))?,"expires_at":expires.timestamp()})).into_response();
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

    const NEAR_AI_TENANT: &str =
        "nearai-0000000000000000000000000000000000000000000000000000000000000000";
    const NEAR_WALLET_TENANT: &str =
        "near-0000000000000000000000000000000000000000000000000000000000000000";

    /// A production build never skips admission, for any tenant, ledger or
    /// not. The staging twin below is the only build where it can.
    #[cfg(not(feature = "near-ai-staging"))]
    #[test]
    fn a_production_build_never_admits_without_the_ledger() {
        for tenant in [NEAR_AI_TENANT, NEAR_WALLET_TENANT, "tenant-a", ""] {
            for ledger_configured in [false, true] {
                assert!(
                    !staging_admits_without_ledger(tenant, ledger_configured),
                    "{tenant} {ledger_configured}"
                );
            }
        }
    }

    /// A staging build skips admission only for a NEAR AI login tenant and
    /// only while the ledger is unconfigured.
    #[cfg(feature = "near-ai-staging")]
    #[test]
    fn a_staging_build_admits_only_near_ai_tenants_without_the_ledger() {
        assert!(staging_admits_without_ledger(NEAR_AI_TENANT, false));
        assert!(!staging_admits_without_ledger(NEAR_AI_TENANT, true));
        for tenant in [
            NEAR_WALLET_TENANT,
            "tenant-a",
            "nearai-not-a-hash",
            "NEARAI-0000000000000000000000000000000000000000000000000000000000000000",
            "",
        ] {
            for ledger_configured in [false, true] {
                assert!(
                    !staging_admits_without_ledger(tenant, ledger_configured),
                    "{tenant} {ledger_configured}"
                );
            }
        }
    }

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

    /// Owner decision of 2026-10-11: a sourceless upload offering evidence
    /// takes the V59 ledger; without evidence it is still refused 422, and a
    /// replay of an existing account row is never re-routed.
    #[test]
    fn a_sourceless_upload_reaches_the_evidence_ledger_only_with_offered_evidence() {
        use AccountRoute::*;
        use EvidencePlan::*;
        for plan in [Verify, RefuseNewSubmission] {
            for replay in [false, true] {
                assert_eq!(account_route(true, plan, replay), AccountLedger);
            }
        }
        assert_eq!(account_route(false, Verify, false), EvidenceLedger);
        assert_eq!(
            account_route(false, RefuseNewSubmission, false),
            RefuseMissingSource
        );
        assert_eq!(account_route(false, Verify, true), RefuseMissingSource);
        assert_eq!(
            account_route(false, RefuseNewSubmission, true),
            RefuseMissingSource
        );
    }

    #[test]
    fn account_admission_refuses_to_start_without_the_legacy_evidence_policy() {
        assert_eq!(
            require_legacy_evidence_policy(true, false)
                .unwrap_err()
                .to_string(),
            "account_admission_requires_legacy_evidence_policy"
        );
        assert!(require_legacy_evidence_policy(true, true).is_ok());
        assert!(require_legacy_evidence_policy(false, false).is_ok());
        assert!(require_legacy_evidence_policy(false, true).is_ok());
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

#[cfg(test)]
mod refusal_label_tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every label an `admission_refused` here can log, one per site.
    const ADMISSION_REFUSAL_LABELS: &[&str] = &[
        "evidence_ledger:evidence_header_count",
        "evidence_ledger:evidence_header_missing",
        "evidence_ledger:evidence_header_not_text",
        "evidence_ledger:evidence_header_too_long",
        "evidence_ledger:signature_header_count",
        "evidence_ledger:signature_header_missing",
        "evidence_ledger:signature_header_not_text",
        "evidence_ledger:signature_header_too_long",
        "legacy_replay:evidence_header_count",
        "legacy_replay:evidence_header_missing",
        "legacy_replay:evidence_header_not_text",
        "legacy_replay:evidence_header_too_long",
        "legacy_replay:signature_header_count",
        "legacy_replay:signature_header_missing",
        "legacy_replay:signature_header_not_text",
        "legacy_replay:signature_header_too_long",
        "account:evidence_header_count",
        "account:evidence_header_missing",
        "account:evidence_header_not_text",
        "account:evidence_header_too_long",
        "account:signature_header_count",
        "account:signature_header_missing",
        "account:signature_header_not_text",
        "account:signature_header_too_long",
        "anchor_db_unconfigured",
        "db_error:anchor_lookup",
        "anchor_missing",
        "anchor_not_sha256_prefixed",
        "anchor_not_hash",
        "evidence_binding_unverified",
        "db_error:attempt_processing_transition",
        "attempt_processing_transition_refused",
        "db_error:attempt_finish_transition",
        "attempt_finish_transition_refused",
        "db_error:attempt_release_without_effect",
        "attempt_release_without_effect_refused",
        "legacy_admission_unconfigured",
        "legacy_db_mirror_writes_off",
        "legacy_db_unconfigured",
        "db_error:legacy_completed_lookup",
        "legacy_no_evidence_headers",
        "db_error:legacy_processing_lock",
        "evidence_ledger_unconfigured",
        "evidence_ledger_db_unconfigured",
        "db_error:evidence_ledger_completed_lookup",
        "evidence_ledger:evidence_json_invalid",
        "evidence_ledger:witness_unverified",
        "evidence_ledger:witness_bypass_unconfigured",
        "evidence_ledger:policy_version_not_allowed",
        "evidence_ledger:evidence_unverified",
        "db_error:evidence_ledger_reserve",
        "evidence_ledger_refused",
        "account_db_mirror_writes_off",
        "account_db_unconfigured",
        "account_trust_refused",
        "account_trust_unavailable",
        "db_error:account_processing_lock",
        "db_error:account_withdrawal_lookup",
        "account_anchor_none",
        "db_error:legacy_record_lookup",
        "legacy_replay:evidence_json_invalid",
        "legacy_replay:witness_pin_unconfigured",
        "legacy_replay:stored_signature_invalid",
        "legacy_replay:binding_mismatch",
        "db_error:legacy_resume",
        "legacy_resume_refused",
        "db_error:account_record_lookup",
        "account:evidence_json_invalid",
        "account_replay:witness_pin_unconfigured",
        "account_replay:stored_signature_invalid",
        "account_replay:binding_mismatch",
        "account:evidence_providers_unconfigured",
        "account:witness_unverified",
        "account:witness_bypass_unconfigured",
        "account:policy_version_not_allowed",
        "account:evidence_unverified",
        "db_error:account_reserve",
        "account_ledger_refused",
        "status_db_unconfigured",
        "status_account_unresolved",
        "db_error:status_lookup",
        "status_missing",
        "challenge_unconfigured",
        "challenge_anchor_none",
        "challenge_nonce_unavailable",
        "challenge_db_unconfigured",
        "challenge_digest_invalid",
        "db_error:challenge_issue",
        "challenge_encode_invalid",
    ];

    /// Labels at `denied_at("...")` / `denied_with("...", ..)` call sites in
    /// this file's production code. Read from the source so that a new site
    /// reusing a label, or one missing from the list, fails here.
    fn call_site_labels() -> Vec<String> {
        let source = include_str!("admission.rs");
        let production = source
            .split(concat!("#[cfg(test)]\nmod ", "tests {"))
            .next()
            .unwrap();
        let mut found = Vec::new();
        for needle in [concat!("denied_at", "(\""), concat!("denied_with", "(\"")] {
            let mut rest = production;
            while let Some(at) = rest.find(needle) {
                let tail = &rest[at + needle.len()..];
                let end = tail.find('"').unwrap();
                found.push(tail[..end].to_string());
                rest = &tail[end..];
            }
        }
        found
    }

    fn header_labels() -> Vec<&'static str> {
        let mut labels = Vec::new();
        for site in [
            HeaderSite::EvidenceLedger,
            HeaderSite::LegacyReplay,
            HeaderSite::Account,
        ] {
            for evidence in [true, false] {
                for fault in [
                    HeaderFault::Count,
                    HeaderFault::Missing,
                    HeaderFault::NotText,
                    HeaderFault::TooLong,
                ] {
                    labels.push(header_refusal(site, evidence, fault));
                }
            }
        }
        labels
    }

    #[test]
    fn every_admission_refusal_site_has_its_own_static_label() {
        let listed: BTreeSet<&str> = ADMISSION_REFUSAL_LABELS.iter().copied().collect();
        assert_eq!(
            listed.len(),
            ADMISSION_REFUSAL_LABELS.len(),
            "duplicate label"
        );
        let mut seen = BTreeSet::new();
        for label in call_site_labels() {
            assert!(listed.contains(label.as_str()), "{label} is not listed");
            assert!(seen.insert(label.clone()), "{label} names two sites");
        }
        for label in header_labels() {
            assert!(listed.contains(label), "{label} is not listed");
            assert!(seen.insert(label.to_string()), "{label} names two sites");
        }
        assert_eq!(seen.len(), listed.len(), "a listed label has no site");
        // Labels only: nothing that could be an identifier or free text.
        for label in ADMISSION_REFUSAL_LABELS {
            assert!(
                label.bytes().all(|b| b.is_ascii_lowercase()
                    || b.is_ascii_digit()
                    || b == b'_'
                    || b == b':'),
                "{label}"
            );
        }
        // At least the sites this was written for.
        assert!(call_site_labels().len() >= 60);
    }

    #[test]
    fn evidence_check_labels_are_unique_and_static() {
        use trace_commons_server::admission_evidence::ADMISSION_EVIDENCE_CHECK_LABELS;
        let set: BTreeSet<&str> = ADMISSION_EVIDENCE_CHECK_LABELS.iter().copied().collect();
        assert_eq!(set.len(), ADMISSION_EVIDENCE_CHECK_LABELS.len());
        for label in ADMISSION_EVIDENCE_CHECK_LABELS {
            assert!(!ADMISSION_REFUSAL_LABELS.contains(label), "{label}");
        }
    }

    /// The label is for the operator. The client still gets the one
    /// refusal it has always got.
    #[test]
    fn a_labelled_refusal_answers_403_admission_refused() {
        for (status, Json(body)) in [
            denied_at("anchor_missing"),
            denied_with("account:evidence_unverified", "evidence_expired"),
        ] {
            assert_eq!(status, StatusCode::FORBIDDEN);
            assert_eq!(body.error, "admission_refused");
            assert_eq!(body.error, AdmissionRefusal::Refused.label());
            assert!(body.retry_after_seconds.is_none());
        }
        let headers = HeaderMap::new();
        let (status, Json(body)) =
            single_header(&headers, EVIDENCE_HEADER, HeaderSite::Account).unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.error, "admission_refused");
    }
}
