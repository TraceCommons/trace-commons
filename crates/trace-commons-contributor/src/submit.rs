//! Submit pipeline: redact-and-upload sessions, then read back submission
//! status. Every outcome reason is a fixed label -- never a response body,
//! trace content, or raw path.

use std::time::Duration as StdDuration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use trace_commons_operator_client::{Client, Error as OcError};
use trace_commons_protocol::trace_contribution::{
    TraceContributionEnvelope, TraceSubmissionReceipt, TraceSubmissionStatusRequest,
    TraceSubmissionStatusUpdate,
};

use crate::config::{ConfigStore, ContributorConfig, Receipt, allowlist_for};
use crate::envelope::{
    MAX_ENVELOPE_BYTES, NearAiSettings, apply_granted_scopes, build_deterministic_preview_redactor,
    build_preview_raw_contribution, build_raw_contribution, build_redactor_with,
    canary_self_test_async, envelope_has_residual_secret, envelope_size, envelope_size_ok,
    near_ai_settings_from_env, parse_scope_names, parse_use_names, raw_contribution_size,
    raw_contribution_size_ok, redact_to_envelope,
};
use crate::identity::{
    DeviceIdentity, build_signed_claim_request, build_signed_claim_request_with_scopes,
};
use crate::issuer_client::{ClaimToken, IssuerClient};
use crate::source::{SessionRef, TraceSource};

/// Statuses that mean a session has already been accepted by the server;
/// re-encountering a receipt with one of these statuses short-circuits the
/// per-session flow instead of re-uploading.
pub(crate) const ALREADY_SUBMITTED_STATUSES: [&str; 3] = ["submitted", "accepted", "quarantined"];

#[derive(Debug)]
pub enum SubmitOutcome {
    Submitted {
        submission_id: Uuid,
        status: String,
    },
    AlreadySubmitted {
        submission_id: Uuid,
        /// The status this session already carries server-side, from the
        /// stored receipt. Without it, a re-run reports "already-submitted"
        /// and the contributor cannot tell whether the trace was accepted,
        /// quarantined, or merely delivered.
        prior_status: String,
    },
    SkippedParseFailure {
        reason_label: String,
    },
    Refused {
        reason_label: String,
        /// Opaque content hash identifying the local session without
        /// exposing its path or trace contents.
        session_ref: String,
        size_bytes: Option<usize>,
        limit_bytes: Option<usize>,
    }, // canary hit, fail-closed PII filter, too large
    Failed {
        reason_label: String,
    }, // network/auth after retries
}

pub struct SubmitOptions {
    pub dry_run: bool,
    pub pii_filter: Option<String>,
    /// Drop model reasoning from every session in this run before envelope
    /// construction. Reasoning is included by default.
    pub no_reasoning: bool,
    /// Suppress progress prose so stdout remains one machine-readable JSON
    /// document. Outcome data is still returned to the command renderer.
    pub machine_readable: bool,
    /// This run has no persisted contributor config. It must remain offline,
    /// use preview ids, and leave the contributor state directory untouched.
    pub unenrolled_preview: bool,
    /// Re-upload corrected envelopes for sessions whose local receipt is
    /// `quarantined`. Keeps the same content-addressed `submission_id` and
    /// asks the server to supersede the stored record (#214).
    pub remediate_quarantined: bool,
}

fn refused(reason_label: &str, session_ref: &str) -> SubmitOutcome {
    SubmitOutcome::Refused {
        reason_label: reason_label.to_string(),
        session_ref: session_ref.to_string(),
        size_bytes: None,
        limit_bytes: None,
    }
}

fn refused_for_size(session_ref: &str, size_bytes: usize) -> SubmitOutcome {
    SubmitOutcome::Refused {
        reason_label: "session-too-large".to_string(),
        session_ref: session_ref.to_string(),
        size_bytes: Some(size_bytes),
        limit_bytes: Some(MAX_ENVELOPE_BYTES),
    }
}

/// Whether a submit result must make the command exit non-zero. Only an
/// expected size finding is non-fatal during dry-run. Every known privacy or
/// pipeline refusal, and every future refusal label, fails closed.
pub fn outcomes_have_failure(outcomes: &[SubmitOutcome], dry_run: bool) -> bool {
    outcomes.iter().any(|outcome| match outcome {
        SubmitOutcome::Failed { .. } => true,
        SubmitOutcome::Refused { reason_label, .. } => match reason_label.as_str() {
            "session-too-large" => !dry_run,
            "pii-filter-unavailable"
            | "redaction-failed"
            | "secret-leak-detected"
            | "scopes-not-permitted" => true,
            _ => true,
        },
        _ => false,
    })
}

/// One entry in a `submit --manifest` file: an envelope id that reached the
/// server, for handing to an external collector (e.g. devfolio).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ManifestEntry {
    pub submission_id: Uuid,
    pub status: String,
}

/// Envelope ids that reached the server, for handing to an external
/// collector (e.g. devfolio). Includes freshly submitted and
/// already-submitted traces; skips refused/failed/skipped outcomes.
pub fn build_manifest(outcomes: &[SubmitOutcome]) -> Vec<ManifestEntry> {
    outcomes
        .iter()
        .filter_map(|o| match o {
            SubmitOutcome::Submitted {
                submission_id,
                status,
            } => Some(ManifestEntry {
                submission_id: *submission_id,
                status: status.clone(),
            }),
            SubmitOutcome::AlreadySubmitted {
                submission_id,
                prior_status,
            } => Some(ManifestEntry {
                submission_id: *submission_id,
                status: prior_status.clone(),
            }),
            SubmitOutcome::SkippedParseFailure { .. }
            | SubmitOutcome::Refused { .. }
            | SubmitOutcome::Failed { .. } => None,
        })
        .collect()
}

/// Redact-and-upload every selected session. Sessions are independent: one
/// session's failure never aborts the batch. The one exception is the
/// once-per-batch privacy-filter canary self-test, which is a fail-closed
/// precondition for the whole batch.
pub async fn submit_sessions(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    sessions: Vec<(Box<dyn TraceSource>, SessionRef)>,
    opts: &SubmitOptions,
) -> Result<Vec<SubmitOutcome>> {
    if opts.unenrolled_preview && !opts.dry_run {
        anyhow::bail!("unenrolled preview requires dry-run");
    }
    let mut ctx = SubmitContext::new(store, cfg, opts, near_ai_settings_from_env())?;
    let mut outcomes = Vec::with_capacity(sessions.len());
    for (source, session_ref) in sessions {
        outcomes.push(ctx.submit_one(source.as_ref(), &session_ref).await?);
    }
    Ok(outcomes)
}

/// A long-lived submit pipeline: everything `submit_sessions` used to hoist
/// across a batch -- device identity, issuer client, the minted claim, the
/// privacy-filter canary, and the receipts index -- held so it can be reused
/// across calls.
///
/// The CLI builds one per `submit` invocation and drops it. The daemon holds
/// one for the life of the process and feeds it a session at a time, so a
/// background upload takes byte-for-byte the same path as an interactive one
/// rather than a parallel reimplementation of it.
///
/// `near_ai` is supplied by the caller rather than read from the environment,
/// because a daemon started by a service manager inherits none of the user's
/// shell environment.
pub struct SubmitContext<'a> {
    store: &'a ConfigStore,
    cfg: &'a ContributorConfig,
    opts: &'a SubmitOptions,
    effective_cfg: ContributorConfig,
    device: Option<DeviceIdentity>,
    issuer: IssuerClient,
    claim: Option<ClaimToken>,
    canary_checked: bool,
    near_ai_notice_recorded: bool,
    near_ai: Option<NearAiSettings>,
    receipts: Vec<Receipt>,
    canary_runs: u32,
}

impl<'a> SubmitContext<'a> {
    pub fn new(
        store: &'a ConfigStore,
        cfg: &'a ContributorConfig,
        opts: &'a SubmitOptions,
        near_ai: Option<NearAiSettings>,
    ) -> Result<Self> {
        let effective_cfg = effective_config(cfg, opts);
        let device = if opts.unenrolled_preview {
            None
        } else if opts.dry_run {
            DeviceIdentity::load(store).context("loading device identity")?
        } else {
            Some(DeviceIdentity::load_or_generate(store).context("loading device identity")?)
        };
        let issuer = IssuerClient::new(allowlist_for(cfg.allowed_hosts.as_deref()))
            .context("building issuer client")?;
        // An unenrolled preview has no enrollment and therefore no submission
        // history it can truthfully replay. Ignore stale receipts from torn
        // local state and run the preview pipeline for every selected session.
        let receipts = if opts.unenrolled_preview {
            Vec::new()
        } else {
            store.load_receipts().context("loading receipts")?
        };
        Ok(Self {
            store,
            cfg,
            opts,
            effective_cfg,
            device,
            issuer,
            claim: None,
            canary_checked: false,
            near_ai_notice_recorded: false,
            near_ai,
            receipts,
            canary_runs: 0,
        })
    }

    /// Force the next `submit_one` to re-run the privacy-filter canary. A
    /// long-lived daemon must re-check the filter periodically rather than
    /// trusting a self-test from days ago.
    pub fn invalidate_canary(&mut self) {
        self.canary_checked = false;
    }

    /// Drop the cached claim, so the next upload mints a fresh one. Called
    /// when enrollment or consent may have changed underneath a running
    /// process.
    pub fn invalidate_claim(&mut self) {
        self.claim = None;
    }

    /// How many times the privacy-filter canary has actually run. Used to
    /// assert the canary is not re-run once per session.
    pub fn canary_runs(&self) -> u32 {
        self.canary_runs
    }

    /// Redact and submit one session. Independent of every other session: a
    /// refusal or failure here never affects a later call. The single
    /// exception is a canary failure, which is a fail-closed precondition and
    /// returns `Err`.
    pub async fn submit_one(
        &mut self,
        source: &dyn TraceSource,
        session_ref: &SessionRef,
    ) -> Result<SubmitOutcome> {
        let opts = self.opts;
        let mut transcript = match source.load(session_ref) {
            Ok(t) => t,
            Err(_) => {
                return Ok(SubmitOutcome::SkippedParseFailure {
                    reason_label: "parse-failed".to_string(),
                });
            }
        };

        if opts.no_reasoning {
            crate::commands::strip_reasoning(&mut transcript);
        }

        // Take the most recent matching receipt, so a session that was
        // delivered and later accepted reports "accepted" rather than the
        // first status it ever had.
        let prior = self
            .receipts
            .iter()
            .filter(|r| {
                r.session_hash == transcript.session_hash
                    && ALREADY_SUBMITTED_STATUSES.contains(&r.status.as_str())
            })
            .max_by_key(|r| r.submitted_at);
        if let Some(prior) = prior {
            let remediating_quarantined =
                opts.remediate_quarantined && prior.status == "quarantined";
            if !remediating_quarantined {
                return Ok(SubmitOutcome::AlreadySubmitted {
                    submission_id: prior.submission_id,
                    prior_status: prior.status.clone(),
                });
            }
        }

        let redactor = if opts.unenrolled_preview {
            build_deterministic_preview_redactor(transcript.cwd.as_deref())
        } else {
            match build_redactor_with(
                &self.effective_cfg,
                transcript.cwd.as_deref(),
                self.near_ai.clone(),
            ) {
                Ok(r) => r,
                Err(_) => {
                    return Ok(refused("pii-filter-unavailable", &transcript.session_hash));
                }
            }
        };

        if !self.canary_checked {
            canary_self_test_async(&redactor)
                .await
                .context("privacy-filter-canary-failed")?;
            self.canary_checked = true;
            self.canary_runs += 1;
        }

        let now = Utc::now();
        let raw = if opts.unenrolled_preview {
            build_preview_raw_contribution(&transcript, &self.effective_cfg, now)
        } else {
            build_raw_contribution(&transcript, &self.effective_cfg, now)
        };
        // Skip sessions that already exceed the envelope limit before the
        // expensive redaction/privacy-filter pass; they would be refused for
        // size after redaction anyway (envelope_size_ok below is the
        // authoritative check).
        if raw_contribution_size_ok(&raw).is_err() {
            let size = raw_contribution_size(&raw).unwrap_or(MAX_ENVELOPE_BYTES + 1);
            return Ok(refused_for_size(&transcript.session_hash, size));
        }
        let mut envelope = match redact_to_envelope(&redactor, raw).await {
            Ok(e) => e,
            Err(_) => {
                return Ok(refused("redaction-failed", &transcript.session_hash));
            }
        };
        if !self.near_ai_notice_recorded
            && self.effective_cfg.pii_filter.as_deref() == Some("near-ai")
        {
            self.store
                .ensure_near_ai_notice_shown()
                .context("recording NEAR AI first-use notice")?;
            self.near_ai_notice_recorded = true;
        }

        let size = match envelope_size_ok(&envelope) {
            Ok(s) => s,
            Err(_) => {
                let size = envelope_size(&envelope).unwrap_or(MAX_ENVELOPE_BYTES + 1);
                return Ok(refused_for_size(&transcript.session_hash, size));
            }
        };

        if opts.dry_run {
            if let Some(outcome) =
                residual_secret_refusal(&redactor, &envelope, &transcript.session_hash)?
            {
                return Ok(outcome);
            }
            if !opts.machine_readable {
                if opts.unenrolled_preview {
                    println!(
                        "unenrolled-preview dry-run: preview_id={} bytes={size} \
                         deterministic-only",
                        envelope.submission_id
                    );
                } else {
                    println!(
                        "dry-run: submission_id={} bytes={size}",
                        envelope.submission_id
                    );
                }
            }
            return Ok(SubmitOutcome::Submitted {
                submission_id: envelope.submission_id,
                status: "dry-run".to_string(),
            });
        }

        if !self
            .claim
            .as_ref()
            .map(|c| c.is_fresh(now))
            .unwrap_or(false)
        {
            let device = self
                .device
                .as_ref()
                .context("device identity unavailable outside unenrolled preview")?;
            match mint_claim(&self.issuer, self.cfg, device, now).await {
                Ok(token) => self.claim = Some(token),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("consent scopes not permitted")
                        || msg.contains("allowed uses not permitted")
                    {
                        println!("hint: re-run login --scopes with a narrower selection");
                        return Ok(refused("scopes-not-permitted", &transcript.session_hash));
                    }
                    return Ok(SubmitOutcome::Failed {
                        reason_label: "claim-mint-failed".to_string(),
                    });
                }
            }
        }

        let token = self
            .claim
            .as_ref()
            .expect("a claim must be minted before applying granted scopes")
            .clone();
        stamp_granted_scopes(&mut envelope, &self.effective_cfg, &token);

        if let Some(outcome) =
            residual_secret_refusal(&redactor, &envelope, &transcript.session_hash)?
        {
            return Ok(outcome);
        }

        if envelope_size_ok(&envelope).is_err() {
            let size = envelope_size(&envelope).unwrap_or(MAX_ENVELOPE_BYTES + 1);
            return Ok(refused_for_size(&transcript.session_hash, size));
        }

        let device = self
            .device
            .as_ref()
            .context("device identity unavailable outside unenrolled preview")?;
        match upload_with_retry(
            self.cfg,
            &self.issuer,
            device,
            &mut self.claim,
            &mut envelope,
            &self.effective_cfg,
        )
        .await
        {
            Ok(receipt) => {
                let r = Receipt {
                    submission_id: envelope.submission_id,
                    session_hash: transcript.session_hash.clone(),
                    source: transcript.source.to_string(),
                    submitted_at: Utc::now(),
                    status: receipt.status.clone(),
                };
                match self.store.append_receipt(&r) {
                    Ok(()) => {
                        self.receipts.push(r);
                        Ok(SubmitOutcome::Submitted {
                            submission_id: envelope.submission_id,
                            status: receipt.status,
                        })
                    }
                    Err(_) => Ok(SubmitOutcome::Failed {
                        reason_label: "receipt-write-failed".to_string(),
                    }),
                }
            }
            Err(reason_label) if reason_label == "session-too-large" => {
                let size = envelope_size(&envelope).unwrap_or(MAX_ENVELOPE_BYTES + 1);
                Ok(refused_for_size(&transcript.session_hash, size))
            }
            Err(reason_label) => Ok(SubmitOutcome::Failed { reason_label }),
        }
    }
}

/// Read back submission status for every locally recorded receipt. Returns
/// an empty vec (no network calls) when there are no receipts yet.
pub async fn status(
    store: &ConfigStore,
    cfg: &ContributorConfig,
) -> Result<Vec<TraceSubmissionStatusUpdate>> {
    let receipts = store.load_receipts().context("loading receipts")?;
    if receipts.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = receipts.iter().map(|r| r.submission_id).collect();

    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;
    let issuer = IssuerClient::new(allowlist_for(cfg.allowed_hosts.as_deref()))
        .context("building issuer client")?;
    // Mint with an empty scopes/uses request rather than the submit path's
    // consent_scopes: the issuer resolves an empty request to the caller's
    // full grant ceiling, so status read-back works regardless of what
    // scopes were narrowed for submission since the last login.
    let token = mint_status_claim(&issuer, cfg, &device, Utc::now())
        .await
        .context("minting upload claim for status lookup")?;
    let client = build_ingest_client(cfg, &token).context("building ingest client")?;

    let mut updates = Vec::new();
    for chunk in ids.chunks(500) {
        let req = TraceSubmissionStatusRequest {
            submission_ids: chunk.to_vec(),
        };
        let mut chunk_updates: Vec<TraceSubmissionStatusUpdate> = client
            .call_json(
                Method::POST,
                "/v1/contributors/me/submission-status",
                &[],
                Some(&req),
            )
            .await
            .context("fetching submission status")?;
        updates.append(&mut chunk_updates);
    }
    Ok(updates)
}

#[derive(Debug, Clone, Serialize)]
struct CommunityProfilePutRequest<'a> {
    display_handle: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bio: Option<&'a str>,
}

/// The public profile as the server stores it.
#[derive(Debug, Clone, Deserialize)]
pub struct CommunityProfile {
    pub display_handle: String,
    pub bio: Option<String>,
    pub public_since: DateTime<Utc>,
}

/// Claim or update this contributor's public handle.
///
/// `login` can grant `public_attribution`, but until this existed nothing in
/// this CLI could use it: claiming a handle meant the operator-facing
/// `/profile` page and a workload token from the *other* enrollment path.
/// Since the server derives the principal from the authenticated request
/// rather than from anything in the body, a handle claimed through a
/// different credential lands on a different principal and never appears
/// beside this device's traces.
pub async fn set_profile(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    display_handle: &str,
    bio: Option<&str>,
) -> Result<CommunityProfile> {
    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;
    let issuer = IssuerClient::new(allowlist_for(cfg.allowed_hosts.as_deref()))
        .context("building issuer client")?;
    // Same empty-scope mint as `status`: the issuer resolves it to this
    // caller's full grant ceiling, so claiming a handle does not depend on
    // whichever scopes were narrowed for the last submission.
    let token = mint_status_claim(&issuer, cfg, &device, Utc::now())
        .await
        .context("minting upload claim for profile update")?;
    let client = build_ingest_client(cfg, &token).context("building ingest client")?;
    let req = CommunityProfilePutRequest {
        display_handle,
        bio,
    };
    client
        .call_json(Method::PUT, "/v1/community/profile", &[], Some(&req))
        .await
        .context("setting public profile")
}

/// Withdraw this contributor's public attribution.
///
/// The row goes at the next snapshot. This is the action `/about/privacy`
/// promises, so it belongs in the tool the contributor already has rather
/// than only in a page they may never have been given access to.
pub async fn clear_profile(store: &ConfigStore, cfg: &ContributorConfig) -> Result<()> {
    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;
    let issuer = IssuerClient::new(allowlist_for(cfg.allowed_hosts.as_deref()))
        .context("building issuer client")?;
    let token = mint_status_claim(&issuer, cfg, &device, Utc::now())
        .await
        .context("minting upload claim for profile withdrawal")?;
    let client = build_ingest_client(cfg, &token).context("building ingest client")?;
    client
        .call_raw::<()>(Method::DELETE, "/v1/community/profile", &[], None)
        .await
        .context("withdrawing public profile")?;
    Ok(())
}

/// Fetch a server-signed attestation of this contributor's own scores.
///
/// The returned value is a compact JWS the contributor hands to a collector
/// (a hackathon scorer, say). The collector verifies it against the ingest
/// attestation keyset rather than trusting a relayed list of submission ids,
/// which is forgeable by anyone who learns an id.
///
/// The endpoint takes no parameters: the principal comes from this call's
/// authentication, so there is nothing here that could request someone
/// else's scores.
pub async fn fetch_score_attestation(
    store: &ConfigStore,
    cfg: &ContributorConfig,
) -> Result<String> {
    let device = DeviceIdentity::load_or_generate(store).context("loading device identity")?;
    let issuer = IssuerClient::new(allowlist_for(cfg.allowed_hosts.as_deref()))
        .context("building issuer client")?;
    // Same empty-scope mint as `status`: the attestation is a read of scores
    // the server already holds, so it must not depend on whatever scopes were
    // narrowed for submission since the last login.
    let token = mint_status_claim(&issuer, cfg, &device, Utc::now())
        .await
        .context("minting upload claim for score attestation")?;
    let client = build_ingest_client(cfg, &token).context("building ingest client")?;

    #[derive(serde::Deserialize)]
    struct AttestationBody {
        attestation: String,
    }

    let body: AttestationBody = client
        .call_json(
            Method::GET,
            "/v1/contributors/me/score-attestation",
            &[],
            None::<&()>,
        )
        .await
        .context("fetching score attestation")?;
    Ok(body.attestation)
}

/// Re-scan a finished envelope for a residual secret shape. Returns
/// `Ok(Some(Refused))` (emitting the same `refusing session` warn every
/// caller relies on) when the redactor's re-scan still finds a secret shape
/// in the serialized envelope, else `Ok(None)`. This is the single seam both
/// the dry-run and real submit paths route through, so deleting either call
/// site removes the fail-closed guard entirely -- callers must `continue` on
/// `Some(_)`.
fn residual_secret_refusal(
    redactor: &trace_commons_protocol::trace_contribution::DeterministicTraceRedactor,
    envelope: &TraceContributionEnvelope,
    session_ref: &str,
) -> Result<Option<SubmitOutcome>> {
    if envelope_has_residual_secret(redactor, envelope)? {
        tracing::warn!("refusing session: secret survived redaction");
        return Ok(Some(refused("secret-leak-detected", session_ref)));
    }
    Ok(None)
}

/// `cfg` with `opts.pii_filter` overriding `cfg.pii_filter` when set.
fn effective_config(cfg: &ContributorConfig, opts: &SubmitOptions) -> ContributorConfig {
    let mut c = cfg.clone();
    if opts.unenrolled_preview {
        c.pii_filter = None;
    } else if opts.pii_filter.is_some() {
        c.pii_filter = opts.pii_filter.clone();
    }
    c
}

async fn mint_claim(
    issuer: &IssuerClient,
    cfg: &ContributorConfig,
    device: &DeviceIdentity,
    now: DateTime<Utc>,
) -> Result<ClaimToken> {
    let signed =
        build_signed_claim_request(cfg, device, now).context("building signed claim request")?;
    issuer.mint_claim(&cfg.issuer_url, &signed).await
}

/// Mint a claim for a status read-back: an empty consent_scopes/allowed_uses
/// request, which the issuer resolves to the caller's full grant ceiling
/// regardless of what was requested for submission.
async fn mint_status_claim(
    issuer: &IssuerClient,
    cfg: &ContributorConfig,
    device: &DeviceIdentity,
    now: DateTime<Utc>,
) -> Result<ClaimToken> {
    let signed = build_signed_claim_request_with_scopes(cfg, device, now, &[], &[])
        .context("building signed status claim request")?;
    issuer.mint_claim(&cfg.issuer_url, &signed).await
}

/// Stamp `envelope` with the granted consent scopes/uses from `token`,
/// falling back to the requested (`effective_cfg`) scopes/uses when the
/// issuer is old enough not to echo them back (empty `consent_scopes`).
/// Shared between the initial stamp before the first upload attempt and the
/// restamp after a claim re-mint, so both paths derive the grant the same
/// way.
fn stamp_granted_scopes(
    envelope: &mut TraceContributionEnvelope,
    effective_cfg: &ContributorConfig,
    token: &ClaimToken,
) {
    let (granted_scopes, granted_uses) = if token.consent_scopes.is_empty() {
        (
            parse_scope_names(&effective_cfg.consent_scopes),
            parse_use_names(&crate::consent::scopes_to_allowed_uses(
                &effective_cfg.consent_scopes,
            )),
        )
    } else {
        (
            parse_scope_names(&token.consent_scopes),
            parse_use_names(&token.allowed_uses),
        )
    };
    apply_granted_scopes(envelope, &granted_scopes, &granted_uses);
}

fn build_ingest_client(
    cfg: &ContributorConfig,
    token: &ClaimToken,
) -> std::result::Result<Client, OcError> {
    Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(&token.access_token)
    .host_allowlist(allowlist_for(cfg.allowed_hosts.as_deref()))
    .build()
}

/// Upload `envelope`, retrying transient transport failures up to 3 attempts
/// total (1s then 4s backoff) and, on a 401/403, re-minting the claim once
/// and retrying once more before giving up.
///
/// A re-mint can return narrower (or otherwise different) granted scopes
/// than the claim that was active when `envelope` was first stamped. To
/// avoid resending an envelope stamped with a stale grant, the envelope is
/// restamped with the new token's granted scopes/uses (via
/// `stamp_granted_scopes`, the same helper used before the first attempt)
/// and re-checked for size before the retry.
async fn upload_with_retry(
    cfg: &ContributorConfig,
    issuer: &IssuerClient,
    device: &DeviceIdentity,
    claim: &mut Option<ClaimToken>,
    envelope: &mut TraceContributionEnvelope,
    effective_cfg: &ContributorConfig,
) -> std::result::Result<TraceSubmissionReceipt, String> {
    let mut transport_attempts: u32 = 0;
    let mut remint_attempted = false;

    loop {
        let token = claim
            .as_ref()
            .expect("a claim must be minted before uploading")
            .clone();
        let client = match build_ingest_client(cfg, &token) {
            Ok(c) => c,
            Err(e) => return Err(e.kind().to_string()),
        };

        let result = client
            .call_json::<TraceContributionEnvelope, TraceSubmissionReceipt>(
                Method::POST,
                "/v1/traces",
                &[],
                Some(&*envelope),
            )
            .await;

        match result {
            Ok(receipt) => return Ok(receipt),
            Err(OcError::Transport { .. }) => {
                transport_attempts += 1;
                if transport_attempts >= 3 {
                    return Err("transport".to_string());
                }
                let delay_secs = if transport_attempts == 1 { 1 } else { 4 };
                tokio::time::sleep(StdDuration::from_secs(delay_secs)).await;
            }
            Err(e) if is_auth_failure(&e) => {
                if remint_attempted {
                    return Err("auth-failed".to_string());
                }
                remint_attempted = true;
                match mint_claim(issuer, cfg, device, Utc::now()).await {
                    Ok(new_token) => {
                        stamp_granted_scopes(envelope, effective_cfg, &new_token);
                        if envelope_size_ok(envelope).is_err() {
                            return Err("session-too-large".to_string());
                        }
                        *claim = Some(new_token);
                    }
                    Err(_) => return Err("auth-failed".to_string()),
                }
            }
            Err(e) => return Err(e.kind().to_string()),
        }
    }
}

fn is_auth_failure(e: &OcError) -> bool {
    match e {
        OcError::ServerLabel { status, .. } | OcError::HttpFailure { status, .. } => {
            status.as_u16() == 401 || status.as_u16() == 403
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};
    use std::sync::{Arc, Mutex};

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    /// Same as `spawn`, but returns a URL addressed via `localhost` instead
    /// of the literal `127.0.0.1`, so tests can put the issuer and ingest
    /// endpoints on distinct allowlist-checkable host strings while both
    /// still resolve to the same loopback listener.
    async fn spawn_as_localhost(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://localhost:{port}")
    }

    fn stub_issuer() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                    "consent_scopes": ["debugging_evaluation", "model_training"],
                    "allowed_uses": ["debugging", "evaluation", "model_training", "aggregate_analytics"],
                }))
            }),
        )
    }

    fn stub_issuer_refuses_scopes() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "consent scopes not permitted"})),
                )
            }),
        )
    }

    fn stub_issuer_refuses_uses() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "allowed uses not permitted"})),
                )
            }),
        )
    }

    fn stub_ingest(received: Arc<Mutex<Vec<serde_json::Value>>>) -> Router {
        stub_ingest_status(received, "accepted")
    }

    fn stub_ingest_status(
        received: Arc<Mutex<Vec<serde_json::Value>>>,
        status: &'static str,
    ) -> Router {
        Router::new().route(
            "/v1/traces",
            post(
                move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                    let received = received.clone();
                    async move {
                        assert_eq!(
                            headers.get("authorization").unwrap(),
                            "Bearer stub-claim-jwt"
                        );
                        received.lock().unwrap().push(body);
                        Json(serde_json::json!({
                            "status": status,
                            "credit_points_pending": 0.0,
                            "explanation": []
                        }))
                    }
                },
            ),
        )
    }

    fn fixture_selection() -> Vec<(
        Box<dyn crate::source::TraceSource>,
        crate::source::SessionRef,
    )> {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = crate::source::claude_code::ClaudeCodeSource::new(root.clone());
        let r = src.discover().unwrap().remove(0);
        vec![(
            Box::new(crate::source::claude_code::ClaudeCodeSource::new(root))
                as Box<dyn crate::source::TraceSource>,
            r,
        )]
    }

    fn write_test_trajectory(path: &std::path::Path, content: &str) {
        let body = serde_json::json!([
            {"role": "meta", "source": "submit-test"},
            {
                "role": "user",
                "content": content,
                "timestamp": "2026-07-31T12:00:00Z"
            }
        ]);
        std::fs::write(path, serde_json::to_vec(&body).unwrap()).unwrap();
    }

    fn trajectory_selection(
        root: &std::path::Path,
    ) -> Vec<(
        Box<dyn crate::source::TraceSource>,
        crate::source::SessionRef,
    )> {
        let mut refs = crate::source::trajectory::TrajectorySource::new(root.to_path_buf())
            .discover()
            .unwrap();
        refs.sort_by(|a, b| a.path.cmp(&b.path));
        refs.into_iter()
            .map(|session_ref| {
                (
                    Box::new(crate::source::trajectory::TrajectorySource::new(
                        root.to_path_buf(),
                    )) as Box<dyn crate::source::TraceSource>,
                    session_ref,
                )
            })
            .collect()
    }

    async fn narrow_boundary_envelope(
        trajectory_path: &std::path::Path,
        content_len: usize,
        cfg: &ContributorConfig,
        narrow_token: &ClaimToken,
    ) -> TraceContributionEnvelope {
        write_test_trajectory(trajectory_path, &"x".repeat(content_len));
        let source =
            crate::source::trajectory::TrajectorySource::new(trajectory_path.to_path_buf());
        let session_ref = source.discover().unwrap().remove(0);
        let transcript = source.load(&session_ref).unwrap();
        let redactor = build_redactor_with(cfg, transcript.cwd.as_deref(), None).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-07-31T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let raw = build_raw_contribution(&transcript, cfg, now);
        assert!(raw_contribution_size_ok(&raw).is_ok());
        let mut envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        stamp_granted_scopes(&mut envelope, cfg, narrow_token);
        envelope
    }

    fn cfg_for(
        issuer: &str,
        ingest: &str,
        device_key_id: &str,
    ) -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: issuer.into(),
            ingest_url: ingest.into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device_key_id.into(),
            consent_scopes: vec!["debugging_evaluation".into(), "model_training".into()],
            pii_filter: None,
            allowed_hosts: None,
        }
    }

    async fn outcome_for_fixture(
        cfg: &crate::config::ContributorConfig,
        unenrolled_preview: bool,
    ) -> trace_commons_protocol::trace_contribution::TraceContributionEnvelope {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let source = crate::source::claude_code::ClaudeCodeSource::new(root);
        let session_ref = source.discover().unwrap().remove(0);
        let transcript = source.load(&session_ref).unwrap();
        let redactor = if unenrolled_preview {
            build_deterministic_preview_redactor(transcript.cwd.as_deref())
        } else {
            build_redactor_with(cfg, transcript.cwd.as_deref(), None).unwrap()
        };
        let now = DateTime::parse_from_rfc3339("2026-07-31T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let raw = if unenrolled_preview {
            build_preview_raw_contribution(&transcript, cfg, now)
        } else {
            build_raw_contribution(&transcript, cfg, now)
        };
        redact_to_envelope(&redactor, raw).await.unwrap()
    }

    #[tokio::test]
    async fn unenrolled_and_enrolled_previews_have_full_outcome_parity() {
        let preview_cfg = crate::commands::unenrolled_preview_config();
        let enrolled_cfg = crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: trace_commons_protocol::onboarding::derive_user_tenant_id(
                "instance-1",
                "alice",
            ),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: "sha256:enrolled".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        };
        assert_eq!(preview_cfg.tenant_id.len(), enrolled_cfg.tenant_id.len());
        assert_eq!(preview_cfg.tenant_id.len(), 71);

        let preview = outcome_for_fixture(&preview_cfg, true).await;
        let enrolled = outcome_for_fixture(&enrolled_cfg, false).await;

        assert_eq!(
            envelope_size(&preview).unwrap(),
            envelope_size(&enrolled).unwrap(),
            "canonical-width placeholder identity must preserve serialized size"
        );
        assert_eq!(
            envelope_size_ok(&preview).is_ok(),
            envelope_size_ok(&enrolled).is_ok(),
            "placeholder identity must not change the size decision"
        );
        assert_eq!(
            preview.consent, enrolled.consent,
            "consent must agree without rewriting either fixture"
        );
        assert_eq!(
            preview.privacy.redaction_pipeline_version,
            enrolled.privacy.redaction_pipeline_version
        );
        assert_eq!(
            preview.privacy.redaction_counts,
            enrolled.privacy.redaction_counts
        );
        assert_eq!(
            preview.privacy.privacy_filter_summary,
            enrolled.privacy.privacy_filter_summary
        );
        assert_eq!(
            preview.privacy.pii_labels_present,
            enrolled.privacy.pii_labels_present
        );
        assert_eq!(
            preview.privacy.residual_pii_risk,
            enrolled.privacy.residual_pii_risk
        );
        assert_eq!(preview.privacy.warnings, enrolled.privacy.warnings);
        // The redaction hash commits to each envelope's deliberately disjoint
        // preview/submission id, so equality would erase the namespace fix.
        for hash in [
            &preview.privacy.redaction_hash,
            &enrolled.privacy.redaction_hash,
        ] {
            assert!(hash.starts_with("sha256:"));
            assert_eq!(hash.len(), 71);
        }
        assert_eq!(
            preview.trace_card.consent_scope,
            enrolled.trace_card.consent_scope
        );
        assert_eq!(
            preview.trace_card.redaction_pipeline_version,
            enrolled.trace_card.redaction_pipeline_version
        );
        assert_eq!(
            preview.trace_card.source_channel,
            enrolled.trace_card.source_channel
        );
        assert_eq!(
            preview.trace_card.tool_categories,
            enrolled.trace_card.tool_categories
        );
        assert_eq!(
            preview.trace_card.allowed_uses,
            enrolled.trace_card.allowed_uses
        );
        assert_eq!(
            preview.trace_card.retention_policy,
            enrolled.trace_card.retention_policy
        );
        assert!(Uuid::parse_str(&preview.trace_card.revocation_handle).is_ok());
        assert!(Uuid::parse_str(&enrolled.trace_card.revocation_handle).is_ok());
        let residual_scanner =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::deterministic_only(
                Vec::new(),
            );
        assert_eq!(
            envelope_has_residual_secret(&residual_scanner, &preview).unwrap(),
            envelope_has_residual_secret(&residual_scanner, &enrolled).unwrap(),
            "placeholder identity must not change the residual-secret result"
        );
        assert_eq!(preview.submission_id.get_version_num(), 8);
        assert_eq!(enrolled.submission_id.get_version_num(), 5);
        assert_ne!(preview.submission_id, enrolled.submission_id);
    }

    #[test]
    fn only_size_refusal_is_non_fatal_in_dry_run() {
        assert!(!outcomes_have_failure(
            &[refused("session-too-large", "sha256:test")],
            true
        ));
        assert!(outcomes_have_failure(
            &[refused("session-too-large", "sha256:test")],
            false
        ));
        for reason in [
            "pii-filter-unavailable",
            "redaction-failed",
            "secret-leak-detected",
            "scopes-not-permitted",
            "future-refusal",
        ] {
            assert!(
                outcomes_have_failure(&[refused(reason, "sha256:test")], true),
                "dry-run suppressed {reason}"
            );
            assert!(
                outcomes_have_failure(&[refused(reason, "sha256:test")], false),
                "real submit suppressed {reason}"
            );
        }
        assert!(outcomes_have_failure(
            &[SubmitOutcome::Failed {
                reason_label: "transport".into(),
            }],
            true
        ));
    }

    /// Drives the real submit path twice and inspects what actually reached
    /// the wire.
    ///
    /// The unit test on `strip_reasoning` alone is not enough: deleting the
    /// call site in `submit_sessions` would leave it green while every
    /// submission silently carried reasoning. `--no-reasoning` is a privacy
    /// control, so its failure mode has to be caught at the boundary it
    /// actually protects.
    #[test]
    fn already_submitted_preserves_the_prior_status() {
        // A re-run reports already-submitted for every session it has seen.
        // Reporting only that is what made three re-submitted traces look
        // like three failures to a contributor and to the collector reading
        // the manifest: nothing told them the traces had been ACCEPTED the
        // first time. The prior status is in the receipt; carry it through.
        let id = Uuid::new_v4();
        let outcomes = vec![SubmitOutcome::AlreadySubmitted {
            submission_id: id,
            prior_status: "accepted".to_string(),
        }];
        let manifest = build_manifest(&outcomes);
        assert_eq!(manifest.len(), 1);
        assert_eq!(manifest[0].submission_id, id);
        assert_eq!(
            manifest[0].status, "accepted",
            "the manifest must carry the real server status, not the literal \"already-submitted\""
        );
    }

    #[tokio::test]
    async fn no_reasoning_controls_what_reaches_the_wire() {
        async fn run(no_reasoning: bool) -> serde_json::Value {
            let received = Arc::new(Mutex::new(Vec::new()));
            let issuer = spawn(stub_issuer()).await;
            let ingest = spawn(stub_ingest(received.clone())).await;
            let dir = tempfile::tempdir().unwrap();
            let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
            let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
            let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
            let opts = SubmitOptions {
                dry_run: false,
                pii_filter: None,
                no_reasoning,
                machine_readable: false,
                unenrolled_preview: false,
                remediate_quarantined: false,
            };
            submit_sessions(&store, &cfg, fixture_selection(), &opts)
                .await
                .unwrap();
            let guard = received.lock().unwrap();
            guard[0].clone()
        }

        fn reasoning_events(envelope: &serde_json::Value) -> usize {
            envelope["events"]
                .as_array()
                .map(|events| {
                    events
                        .iter()
                        .filter(|e| e["event_type"] == "reasoning")
                        .count()
                })
                .unwrap_or(0)
        }

        // The committed fixture contains a thinking block, so the default
        // path must carry reasoning. If this ever reaches zero the fixture
        // stopped exercising the feature and the opt-out assertion below
        // would pass vacuously.
        let with = run(false).await;
        assert!(
            reasoning_events(&with) > 0,
            "reasoning must reach the wire by default"
        );

        let without = run(true).await;
        assert_eq!(
            reasoning_events(&without),
            0,
            "--no-reasoning must strip reasoning before upload"
        );
    }

    #[tokio::test]
    async fn submit_context_reuses_the_canary_across_sessions() {
        // The canary is a per-batch precondition, not a per-session one. A
        // daemon holding one context for weeks must not pay for -- or fail
        // on -- a fresh self-test per trace.
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(
            "http://issuer.invalid",
            "http://ingest.invalid",
            &device.device_key_id,
        );
        let opts = SubmitOptions {
            dry_run: true,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: true,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let selection = fixture_selection();
        let (source, session_ref) = &selection[0];

        let first = ctx.submit_one(source.as_ref(), session_ref).await.unwrap();
        let second = ctx.submit_one(source.as_ref(), session_ref).await.unwrap();

        assert!(
            matches!(first, SubmitOutcome::Submitted { .. }),
            "got {first:?}"
        );
        assert!(
            matches!(second, SubmitOutcome::Submitted { .. }),
            "got {second:?}"
        );
        assert_eq!(ctx.canary_runs(), 1, "canary must not re-run per session");
    }

    #[tokio::test]
    async fn submit_context_reruns_the_canary_after_invalidation() {
        // A long-lived daemon re-checks the privacy filter periodically.
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(
            "http://issuer.invalid",
            "http://ingest.invalid",
            &device.device_key_id,
        );
        let opts = SubmitOptions {
            dry_run: true,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: true,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let selection = fixture_selection();
        let (source, session_ref) = &selection[0];

        ctx.submit_one(source.as_ref(), session_ref).await.unwrap();
        ctx.invalidate_canary();
        ctx.submit_one(source.as_ref(), session_ref).await.unwrap();

        assert_eq!(ctx.canary_runs(), 2);
    }

    #[tokio::test]
    async fn submits_fixture_session_and_is_idempotent_on_rerun() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        {
            // Scope the guard: `let sent = &received.lock().unwrap()[0]` would
            // extend the MutexGuard to the end of the test and self-deadlock
            // on the re-lock after the second run.
            let received_guard = received.lock().unwrap();
            assert_eq!(received_guard.len(), 1);
            let sent = &received_guard[0];
            assert_eq!(sent["schema_version"], "ironclaw.trace_contribution.v1");
            assert!(
                !serde_json::to_string(sent)
                    .unwrap()
                    .contains("sk-fake-fixture-secret-1234")
            );
            assert_eq!(
                sent["consent"]["scopes"],
                serde_json::json!(["debugging_evaluation", "model_training"])
            );
        }

        // Second run: receipt short-circuits, no second upload.
        let outcomes2 = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(
            outcomes2[0],
            SubmitOutcome::AlreadySubmitted { .. }
        ));
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn remediate_quarantined_reuploads_under_same_submission_id() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest_status(received.clone(), "quarantined")).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(
            &outcomes[0],
            SubmitOutcome::Submitted { status, .. } if status == "quarantined"
        ));
        assert_eq!(received.lock().unwrap().len(), 1);

        // Default re-run still short-circuits on the quarantined receipt.
        let blocked = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(
            &blocked[0],
            SubmitOutcome::AlreadySubmitted {
                prior_status,
                ..
            } if prior_status == "quarantined"
        ));
        assert_eq!(received.lock().unwrap().len(), 1);

        // Opt-in remediation rebuilds and re-uploads the same submission_id.
        let remediate = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: true,
        };
        let outcomes2 = submit_sessions(&store, &cfg, fixture_selection(), &remediate)
            .await
            .unwrap();
        assert!(matches!(
            &outcomes2[0],
            SubmitOutcome::Submitted { status, .. } if status == "quarantined"
        ));
        let received_guard = received.lock().unwrap();
        assert_eq!(received_guard.len(), 2);
        assert_eq!(
            received_guard[0]["submission_id"], received_guard[1]["submission_id"],
            "remediation must keep the content-addressed submission_id"
        );
    }

    /// The residual-secret guard is a re-scan of the finished envelope with
    /// the secret detector. A survivor (a detect-then-redact bug, or a
    /// non-string payload value the string-leaf pass never visited) leaves a
    /// recognizable secret shape in the serialized envelope and trips the
    /// guard; a clean envelope does not. This exercises the helper directly:
    /// forcing a real survivor through the (now-strong) redaction pipeline is
    /// impractical, so we plant a detector-recognized secret shape
    /// (`sk-ant-...`) into a finished envelope and assert the guard catches
    /// it, plus that an unmodified redacted envelope is clean. The full
    /// submit path's clean-session Submitted behavior is covered by
    /// `submits_fixture_session_and_is_idempotent_on_rerun` against the
    /// original fixture (whose Opaque record-type markers and normal prose
    /// are not secret-shaped and never trip the guard).
    #[tokio::test]
    async fn residual_secret_guard_flags_survivor_and_passes_clean_envelope() {
        use crate::envelope::{
            build_raw_contribution, envelope_has_residual_secret, redact_to_envelope,
        };
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = crate::source::claude_code::ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        let transcript = src.load(&r).unwrap();

        let cfg = cfg_for(
            "https://issuer.example",
            "https://ingest.example",
            "sha256:00",
        );
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let raw = build_raw_contribution(&transcript, &cfg, Utc::now());
        let mut envelope = redact_to_envelope(&redactor, raw).await.unwrap();

        // A properly-redacted envelope has no residual secret shape.
        assert!(!envelope_has_residual_secret(&redactor, &envelope).unwrap());

        // Plant a detector-recognized secret shape into the finished
        // envelope, simulating a value that survived redaction. The re-scan
        // must catch it and the session must fail closed.
        if let Some(first) = envelope.events.first_mut() {
            first.redacted_content =
                Some("leftover sk-ant-EXPOSEDsecret0123456789abcdefghij here".to_string());
        }
        assert!(envelope_has_residual_secret(&redactor, &envelope).unwrap());
    }

    /// The `model` field (`IronclawTraceMetadata::model_name`) is copied
    /// verbatim from the transcript into the envelope and is never routed
    /// through the per-field redaction pass (only `content` and
    /// `structured_payload` are). The whole-envelope residual-secret rescan
    /// (`residual_secret_refusal`, called from both submit-path call sites)
    /// is the only thing standing between a secret-shaped literal placed
    /// there and delivery to ingest. This drives the *real* `submit_sessions`
    /// entrypoint end to end with a fixture whose `model` field is a
    /// recognized secret shape (`sk-ant-...`), so it fails if either call
    /// site is ever deleted: without the guard, this session would upload
    /// (`Submitted`, 1 delivery) instead of refusing.
    #[tokio::test]
    async fn submit_sessions_refuses_session_with_secret_in_unredacted_model_field() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        // A minimal transcript whose assistant message carries a
        // detector-recognized secret shape in `model`, a field the per-field
        // redaction pass never scans.
        let fixture_root = tempfile::tempdir().unwrap();
        let project_dir = fixture_root.path().join("-tmp-secret-model-proj");
        std::fs::create_dir_all(&project_dir).unwrap();
        let jsonl = concat!(
            r#"{"type":"user","message":{"role":"user","content":"hello"},"cwd":"/tmp/secret-model-proj","timestamp":"2026-07-01T10:00:00Z","version":"2.0.1","sessionId":"22222222-2222-2222-2222-222222222222","uuid":"a1"}"#,
            "\n",
            r#"{"type":"assistant","message":{"role":"assistant","model":"sk-ant-EXPOSEDsecret0123456789abcdefghij","content":[{"type":"text","text":"ok"}],"usage":{"input_tokens":1,"output_tokens":1}},"cwd":"/tmp/secret-model-proj","timestamp":"2026-07-01T10:00:05Z","version":"2.0.1","uuid":"a2"}"#,
            "\n",
        );
        std::fs::write(
            project_dir.join("22222222-2222-2222-2222-222222222222.jsonl"),
            jsonl,
        )
        .unwrap();

        let src =
            crate::source::claude_code::ClaudeCodeSource::new(fixture_root.path().to_path_buf());
        let session_ref = src.discover().unwrap().remove(0);
        let selection: Vec<(
            Box<dyn crate::source::TraceSource>,
            crate::source::SessionRef,
        )> = vec![(
            Box::new(crate::source::claude_code::ClaudeCodeSource::new(
                fixture_root.path().to_path_buf(),
            )) as Box<dyn crate::source::TraceSource>,
            session_ref,
        )];

        let outcomes = submit_sessions(&store, &cfg, selection, &opts)
            .await
            .unwrap();
        match &outcomes[0] {
            SubmitOutcome::Refused { reason_label, .. } => {
                assert_eq!(reason_label, "secret-leak-detected");
            }
            other => panic!("expected Refused(secret-leak-detected), got {other:?}"),
        }
        assert_eq!(
            received.lock().unwrap().len(),
            0,
            "a session with a residual secret must never reach ingest"
        );
        assert!(store.load_receipts().unwrap().is_empty());
    }

    #[tokio::test]
    async fn dry_run_uploads_nothing_and_writes_no_receipt() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: true,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };
        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_filter_construction_does_not_write_notice_marker() {
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(Arc::new(Mutex::new(Vec::new())))).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        assert!(!store.dir().join("near-ai-notice-shown").exists());

        let opts = SubmitOptions {
            dry_run: true,
            pii_filter: Some("near-ai".to_string()),
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };
        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(
            &outcomes[0],
            SubmitOutcome::Refused { reason_label, .. }
                if reason_label == "pii-filter-unavailable"
        ));
        assert!(!store.dir().join("near-ai-notice-shown").exists());
    }

    #[tokio::test]
    async fn receipt_append_failure_preserves_prior_outcomes_and_finishes_batch() {
        let trajectory_dir = tempfile::tempdir().unwrap();
        write_test_trajectory(&trajectory_dir.path().join("a.json"), "first session");
        write_test_trajectory(&trajectory_dir.path().join("b.json"), "second session");

        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let receipt_path = store.dir().join("receipts.jsonl");
        let post_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let received = Arc::new(Mutex::new(Vec::new()));
        let ingest = spawn(Router::new().route(
            "/v1/traces",
            post({
                let post_calls = post_calls.clone();
                let received = received.clone();
                move |Json(body): Json<serde_json::Value>| {
                    let post_calls = post_calls.clone();
                    let received = received.clone();
                    let receipt_path = receipt_path.clone();
                    async move {
                        received.lock().unwrap().push(body);
                        let call = post_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        if call == 1 {
                            std::fs::remove_file(&receipt_path).unwrap();
                            std::fs::create_dir(&receipt_path).unwrap();
                        }
                        Json(serde_json::json!({
                            "status": "accepted",
                            "credit_points_pending": 0.0,
                            "explanation": []
                        }))
                    }
                }
            }),
        ))
        .await;
        let issuer = spawn(stub_issuer()).await;
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(
            &store,
            &cfg,
            trajectory_selection(trajectory_dir.path()),
            &opts,
        )
        .await
        .unwrap();

        assert_eq!(outcomes.len(), 2);
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        assert!(matches!(
            &outcomes[1],
            SubmitOutcome::Failed { reason_label } if reason_label == "receipt-write-failed"
        ));
        assert_eq!(received.lock().unwrap().len(), 2);
    }

    /// Grants strictly less than requested: config asks for
    /// debugging_evaluation + model_training, issuer grants only
    /// debugging_evaluation.
    fn stub_issuer_narrows_grant() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                    "consent_scopes": ["debugging_evaluation"],
                    "allowed_uses": ["debugging", "evaluation", "aggregate_analytics"],
                }))
            }),
        )
    }

    #[tokio::test]
    async fn envelope_is_stamped_with_narrowed_grant_when_server_grants_less() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_narrows_grant()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        // cfg_for requests debugging_evaluation + model_training; the stub
        // issuer grants only debugging_evaluation. The envelope must carry
        // the granted (narrower) set, never the requested one.
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        let received_guard = received.lock().unwrap();
        assert_eq!(received_guard.len(), 1);
        let sent = &received_guard[0];
        assert_eq!(
            sent["consent"]["scopes"],
            serde_json::json!(["debugging_evaluation"])
        );
        let allowed_uses = sent["trace_card"]["allowed_uses"].as_array().unwrap();
        assert!(
            !allowed_uses
                .iter()
                .any(|u| u == &serde_json::json!("model_training"))
        );
    }

    /// An issuer that predates the consent_scopes/allowed_uses echo: the
    /// claim response omits both fields entirely.
    fn stub_issuer_omits_scope_echo() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                }))
            }),
        )
    }

    #[tokio::test]
    async fn envelope_is_stamped_with_requested_scopes_when_issuer_omits_echo() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_omits_scope_echo()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        // cfg_for requests debugging_evaluation + model_training; the stub
        // issuer's claim response has no consent_scopes/allowed_uses fields
        // at all, so the fallback must stamp the requested set verbatim.
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        let received_guard = received.lock().unwrap();
        assert_eq!(received_guard.len(), 1);
        let sent = &received_guard[0];
        assert_eq!(
            sent["consent"]["scopes"],
            serde_json::json!(["debugging_evaluation", "model_training"])
        );
    }

    #[tokio::test]
    async fn scope_refusal_from_issuer_yields_refused_outcome_with_no_deliveries() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_refuses_scopes()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        match &outcomes[0] {
            SubmitOutcome::Refused { reason_label, .. } => {
                assert_eq!(reason_label, "scopes-not-permitted");
            }
            other => panic!("expected Refused(scopes-not-permitted), got {other:?}"),
        }
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }

    #[tokio::test]
    async fn uses_refusal_from_issuer_yields_refused_outcome_with_no_deliveries() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_refuses_uses()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        match &outcomes[0] {
            SubmitOutcome::Refused { reason_label, .. } => {
                assert_eq!(reason_label, "scopes-not-permitted");
            }
            other => panic!("expected Refused(scopes-not-permitted), got {other:?}"),
        }
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }

    /// Mints `["debugging_evaluation", "model_training"]` on the first call
    /// and the narrower `["debugging_evaluation"]` on every call after —
    /// simulating a grant narrowed between the first and second mint.
    fn stub_issuer_narrows_on_remint(mint_calls: Arc<std::sync::atomic::AtomicUsize>) -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(move || {
                let mint_calls = mint_calls.clone();
                async move {
                    let n = mint_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if n == 0 {
                        Json(serde_json::json!({
                            "access_token": "stub-claim-jwt",
                            "token_type": "Bearer",
                            "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                            "expires_in": 300,
                            "consent_scopes": ["debugging_evaluation", "model_training"],
                            "allowed_uses": ["debugging", "evaluation", "model_training", "aggregate_analytics"],
                        }))
                    } else {
                        Json(serde_json::json!({
                            "access_token": "stub-claim-jwt",
                            "token_type": "Bearer",
                            "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                            "expires_in": 300,
                            "consent_scopes": ["debugging_evaluation"],
                            "allowed_uses": ["debugging", "evaluation", "aggregate_analytics"],
                        }))
                    }
                }
            }),
        )
    }

    fn stub_issuer_widens_on_remint(mint_calls: Arc<std::sync::atomic::AtomicUsize>) -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(move || {
                let mint_calls = mint_calls.clone();
                async move {
                    let n = mint_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let (consent_scopes, allowed_uses) = if n == 0 {
                        (
                            serde_json::json!(["debugging_evaluation"]),
                            serde_json::json!(["debugging", "evaluation", "aggregate_analytics"]),
                        )
                    } else {
                        (
                            serde_json::json!([
                                "debugging_evaluation",
                                "benchmark_only",
                                "ranking_training",
                                "model_training",
                                "public_attribution"
                            ]),
                            serde_json::json!([
                                "debugging",
                                "evaluation",
                                "benchmark_generation",
                                "ranking_model_training",
                                "model_training",
                                "aggregate_analytics"
                            ]),
                        )
                    };
                    Json(serde_json::json!({
                        "access_token": "stub-claim-jwt",
                        "token_type": "Bearer",
                        "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                        "expires_in": 300,
                        "consent_scopes": consent_scopes,
                        "allowed_uses": allowed_uses,
                    }))
                }
            }),
        )
    }

    /// Refuses the first POST with 401 (forcing a claim re-mint + retry) and
    /// accepts every POST after, recording every received body so the test
    /// can inspect what the *retried* request actually carried.
    fn stub_ingest_401_then_200(
        received: Arc<Mutex<Vec<serde_json::Value>>>,
        post_calls: Arc<std::sync::atomic::AtomicUsize>,
    ) -> Router {
        use axum::response::IntoResponse;
        Router::new().route(
            "/v1/traces",
            post(move |Json(body): Json<serde_json::Value>| {
                let received = received.clone();
                let post_calls = post_calls.clone();
                async move {
                    let n = post_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    received.lock().unwrap().push(body);
                    if n == 0 {
                        axum::http::StatusCode::UNAUTHORIZED.into_response()
                    } else {
                        Json(serde_json::json!({
                            "status": "accepted",
                            "credit_points_pending": 0.0,
                            "explanation": []
                        }))
                        .into_response()
                    }
                }
            }),
        )
    }

    #[tokio::test]
    async fn envelope_is_restamped_after_claim_remint_on_auth_failure() {
        use std::sync::atomic::AtomicUsize;

        let mint_calls = Arc::new(AtomicUsize::new(0));
        let post_calls = Arc::new(AtomicUsize::new(0));
        let received = Arc::new(Mutex::new(Vec::new()));

        let issuer = spawn(stub_issuer_narrows_on_remint(mint_calls.clone())).await;
        let ingest = spawn(stub_ingest_401_then_200(
            received.clone(),
            post_calls.clone(),
        ))
        .await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(
            matches!(outcomes[0], SubmitOutcome::Submitted { .. }),
            "expected Submitted after remint+retry, got {:?}",
            outcomes[0]
        );
        assert_eq!(mint_calls.load(std::sync::atomic::Ordering::SeqCst), 2);

        let received_guard = received.lock().unwrap();
        assert_eq!(
            received_guard.len(),
            2,
            "the 401 attempt and the successful retry must both reach ingest"
        );
        // The envelope actually delivered on the second (200) POST must carry
        // the NEW token's narrower grant, not the original wider one it was
        // first stamped with.
        let restamped = &received_guard[1];
        assert_eq!(
            restamped["consent"]["scopes"],
            serde_json::json!(["debugging_evaluation"]),
            "retried envelope must be restamped with the re-minted (narrower) scopes: {restamped}"
        );
        let allowed_uses = restamped["trace_card"]["allowed_uses"].as_array().unwrap();
        assert!(
            !allowed_uses
                .iter()
                .any(|u| u == &serde_json::json!("model_training")),
            "retried envelope must not retain model_training from the stale claim: {restamped}"
        );
    }

    #[tokio::test]
    async fn post_remint_size_overflow_is_a_structured_refusal() {
        use std::sync::atomic::AtomicUsize;

        let trajectory_dir = tempfile::tempdir().unwrap();
        let trajectory_path = trajectory_dir.path().join("boundary.json");
        let base_content_len = 1_496_000usize;
        write_test_trajectory(&trajectory_path, &"x".repeat(base_content_len));

        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let mint_calls = Arc::new(AtomicUsize::new(0));
        let post_calls = Arc::new(AtomicUsize::new(0));
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_widens_on_remint(mint_calls.clone())).await;
        let ingest = spawn(stub_ingest_401_then_200(
            received.clone(),
            post_calls.clone(),
        ))
        .await;
        let mut cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        cfg.consent_scopes = vec!["debugging_evaluation".to_string()];
        let narrow_token = ClaimToken {
            access_token: "narrow".to_string(),
            expires_at: Utc::now() + chrono::Duration::seconds(300),
            consent_scopes: vec!["debugging_evaluation".to_string()],
            allowed_uses: vec![
                "debugging".to_string(),
                "evaluation".to_string(),
                "aggregate_analytics".to_string(),
            ],
        };
        let wide_token = ClaimToken {
            access_token: "wide".to_string(),
            expires_at: Utc::now() + chrono::Duration::seconds(300),
            consent_scopes: crate::consent::VALID_SCOPES
                .iter()
                .map(|scope| scope.to_string())
                .collect(),
            allowed_uses: crate::consent::scopes_to_allowed_uses(
                &crate::consent::VALID_SCOPES
                    .iter()
                    .map(|scope| scope.to_string())
                    .collect::<Vec<_>>(),
            ),
        };

        let initial =
            narrow_boundary_envelope(&trajectory_path, base_content_len, &cfg, &narrow_token).await;
        let target_size = MAX_ENVELOPE_BYTES - 64;
        let initial_size = envelope_size(&initial).unwrap();
        let calibrated_len = if initial_size <= target_size {
            base_content_len + (target_size - initial_size)
        } else {
            base_content_len - (initial_size - target_size)
        };
        let narrow =
            narrow_boundary_envelope(&trajectory_path, calibrated_len, &cfg, &narrow_token).await;
        let narrow_size = envelope_size(&narrow).unwrap();
        let mut wide = narrow.clone();
        stamp_granted_scopes(&mut wide, &cfg, &wide_token);
        let wide_size = envelope_size(&wide).unwrap();
        assert_eq!(narrow_size, target_size);
        assert!(wide_size > MAX_ENVELOPE_BYTES);

        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };
        let outcomes = submit_sessions(
            &store,
            &cfg,
            trajectory_selection(trajectory_dir.path()),
            &opts,
        )
        .await
        .unwrap();

        match &outcomes[0] {
            SubmitOutcome::Refused {
                reason_label,
                size_bytes,
                limit_bytes,
                ..
            } => {
                assert_eq!(reason_label, "session-too-large");
                assert!(size_bytes.unwrap() > MAX_ENVELOPE_BYTES);
                assert_eq!(*limit_bytes, Some(MAX_ENVELOPE_BYTES));
            }
            other => panic!("expected structured size refusal, got {other:?}"),
        }
        assert_eq!(mint_calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(post_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    /// Records every claim-request body it receives (as raw JSON) before
    /// responding with a fixed claim, so tests can inspect what scopes/uses
    /// were actually requested.
    fn stub_issuer_recording_requests(received: Arc<Mutex<Vec<serde_json::Value>>>) -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(move |body: String| {
                let received = received.clone();
                async move {
                    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
                    received.lock().unwrap().push(parsed);
                    Json(serde_json::json!({
                        "access_token": "stub-claim-jwt",
                        "token_type": "Bearer",
                        "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                        "expires_in": 300,
                        "consent_scopes": ["debugging_evaluation", "model_training"],
                        "allowed_uses": ["debugging", "evaluation", "model_training", "aggregate_analytics"],
                    }))
                }
            }),
        )
    }

    fn stub_submission_status_ingest() -> Router {
        Router::new().route(
            "/v1/contributors/me/submission-status",
            post(|Json(req): Json<serde_json::Value>| async move {
                let ids = req["submission_ids"].as_array().unwrap();
                let updates: Vec<serde_json::Value> = ids
                    .iter()
                    .map(|id| {
                        serde_json::json!({
                            "submission_id": id,
                            "trace_id": id,
                            "status": "accepted",
                            "credit_points_pending": 0.0,
                        })
                    })
                    .collect();
                Json(updates)
            }),
        )
    }

    #[tokio::test]
    async fn status_mints_claim_with_empty_scopes_and_uses() {
        let claim_requests = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_recording_requests(claim_requests.clone())).await;
        let ingest = spawn(stub_submission_status_ingest()).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);

        // Seed a receipt so status() actually mints a claim and calls out.
        store
            .append_receipt(&crate::config::Receipt {
                submission_id: Uuid::new_v4(),
                session_hash: "sha256:test".to_string(),
                source: "claude-code".to_string(),
                submitted_at: Utc::now(),
                status: "submitted".to_string(),
            })
            .unwrap();

        let updates = status(&store, &cfg).await.unwrap();
        assert_eq!(updates.len(), 1);

        let requests = claim_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        let req = &requests[0];
        assert_eq!(
            req["consent_scopes"],
            serde_json::json!([]),
            "status claim request must not request the submit-path's scopes: {req}"
        );
        assert_eq!(
            req["allowed_uses"],
            serde_json::json!([]),
            "status claim request must not request the submit-path's uses: {req}"
        );
    }

    /// Records the method and body of every /v1/community/profile call.
    fn stub_community_profile_ingest(seen: Arc<Mutex<Vec<(String, String)>>>) -> Router {
        Router::new().route(
            "/v1/community/profile",
            axum::routing::put({
                let seen = seen.clone();
                move |body: String| {
                    let seen = seen.clone();
                    async move {
                        seen.lock().unwrap().push(("PUT".to_string(), body));
                        Json(serde_json::json!({
                            "display_handle": "stub_handle",
                            "handle_normalized": "stub_handle",
                            "bio": null,
                            "public_since": chrono::Utc::now(),
                            "last_updated_at": chrono::Utc::now(),
                            "update_count": 0,
                        }))
                    }
                }
            })
            .delete(move |body: String| {
                let seen = seen.clone();
                async move {
                    seen.lock().unwrap().push(("DELETE".to_string(), body));
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        )
    }

    #[tokio::test]
    async fn set_profile_mints_an_empty_scope_claim() {
        // Same property `status` relies on: an empty request resolves to the
        // caller's full grant ceiling, so claiming a handle does not depend
        // on whichever scopes were narrowed for the last submission.
        let claim_requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_recording_requests(claim_requests.clone())).await;
        let ingest = spawn(stub_community_profile_ingest(seen.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);

        let profile = set_profile(&store, &cfg, "stub_handle", None)
            .await
            .unwrap();
        assert_eq!(profile.display_handle, "stub_handle");

        let requests = claim_requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["consent_scopes"], serde_json::json!([]));
        assert_eq!(requests[0]["allowed_uses"], serde_json::json!([]));

        let calls = seen.lock().unwrap();
        assert_eq!(calls[0].0, "PUT");
        let body: serde_json::Value = serde_json::from_str(&calls[0].1).unwrap();
        assert_eq!(body["display_handle"], "stub_handle");
        // Omitting the key is NOT a way to preserve an existing bio: the
        // server deserializes missing and null identically to None and then
        // upserts `bio = excluded.bio`, so either form clears it. An earlier
        // version of this test asserted the opposite. The protection against
        // clearing a bio by accident lives in the command layer, which
        // requires --bio or --no-bio; this only pins the wire shape.
        assert!(
            body.get("bio").is_none(),
            "bio must be omitted from the body when not set: {body}"
        );
    }

    #[tokio::test]
    async fn clear_profile_sends_a_bodyless_delete() {
        let claim_requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer_recording_requests(claim_requests.clone())).await;
        let ingest = spawn(stub_community_profile_ingest(seen.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);

        clear_profile(&store, &cfg).await.unwrap();

        let calls = seen.lock().unwrap();
        assert_eq!(calls[0].0, "DELETE");
        assert!(
            calls[0].1.is_empty(),
            "withdrawal must not send a JSON body: {:?}",
            calls[0].1
        );
    }

    #[tokio::test]
    async fn upload_refuses_ingest_host_off_allowlist_before_any_request() {
        let received = Arc::new(Mutex::new(Vec::new()));
        // Issuer stays on the literal `127.0.0.1` host (allowed); ingest is
        // addressed via `localhost` (not on the allowlist), so the claim
        // mints fine but the ingest client must refuse to even build.
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn_as_localhost(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let mut cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        cfg.allowed_hosts = Some("127.0.0.1".to_string());
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        match &outcomes[0] {
            SubmitOutcome::Failed { reason_label } => {
                assert_eq!(reason_label, "host-not-allowed");
            }
            other => panic!("expected Failed(host-not-allowed), got {other:?}"),
        }
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }

    #[test]
    fn build_manifest_includes_only_delivered_ids() {
        let u1 = Uuid::new_v4();
        let u2 = Uuid::new_v4();
        let outcomes = vec![
            SubmitOutcome::Submitted {
                submission_id: u1,
                status: "submitted".to_string(),
            },
            SubmitOutcome::AlreadySubmitted {
                submission_id: u2,
                prior_status: "quarantined".to_string(),
            },
            refused("secret-leak-detected", "sha256:test"),
            SubmitOutcome::Failed {
                reason_label: "claim-mint-failed".to_string(),
            },
            SubmitOutcome::SkippedParseFailure {
                reason_label: "parse-failed".to_string(),
            },
        ];

        let manifest = build_manifest(&outcomes);

        assert_eq!(manifest.len(), 2);
        assert_eq!(manifest[0].submission_id, u1);
        assert_eq!(manifest[0].status, "submitted");
        assert_eq!(manifest[1].submission_id, u2);
        // Previously the literal "already-submitted". A collector reading the
        // manifest could not distinguish an accepted trace from a quarantined
        // one, so a contributor's re-run looked like a batch of failures.
        assert_eq!(manifest[1].status, "quarantined");
    }

    #[tokio::test]
    async fn submit_sessions_outcomes_round_trip_through_manifest_file() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: false,
            unenrolled_preview: false,
            remediate_quarantined: false,
        };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts)
            .await
            .unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));

        let entries = build_manifest(&outcomes);
        let manifest_path = tempfile::NamedTempFile::new().unwrap();
        let json = serde_json::to_string_pretty(&entries).unwrap();
        std::fs::write(manifest_path.path(), json).unwrap();

        let read_back = std::fs::read_to_string(manifest_path.path()).unwrap();
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&read_back).unwrap();
        assert_eq!(parsed.len(), 1);
        let SubmitOutcome::Submitted { submission_id, .. } = &outcomes[0] else {
            unreachable!()
        };
        assert_eq!(
            parsed[0]["submission_id"],
            serde_json::Value::String(submission_id.to_string())
        );
        assert_eq!(parsed[0]["status"], "accepted");
    }
}

/// Machine-readable form of a submit run, for callers driving this CLI
/// programmatically (an MCP server, CI, a hackathon collector).
///
/// Every outcome is represented, including the ones `build_manifest` drops:
/// a caller automating submission needs to know a session was refused and
/// why, not merely that it is absent from the manifest.
pub fn outcomes_to_json(
    outcomes: &[SubmitOutcome],
    unenrolled_preview: bool,
    notices: &[&str],
) -> serde_json::Value {
    let entries: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|o| {
            let mut entry = match o {
                SubmitOutcome::Submitted {
                    submission_id,
                    status,
                } if unenrolled_preview => serde_json::json!({
                    "outcome": "previewed",
                    "preview_id": submission_id,
                    "status": status,
                }),
                SubmitOutcome::Submitted {
                    submission_id,
                    status,
                } => serde_json::json!({
                    "outcome": "submitted",
                    "submission_id": submission_id,
                    "status": status,
                }),
                SubmitOutcome::AlreadySubmitted {
                    submission_id,
                    prior_status,
                } => serde_json::json!({
                    "outcome": "already-submitted",
                    "submission_id": submission_id,
                    "status": prior_status,
                }),
                SubmitOutcome::SkippedParseFailure { reason_label } => serde_json::json!({
                    "outcome": "skipped",
                    "reason": reason_label,
                }),
                SubmitOutcome::Refused {
                    reason_label,
                    session_ref,
                    size_bytes,
                    limit_bytes,
                } => serde_json::json!({
                    "outcome": "refused",
                    "reason": reason_label,
                    "session_ref": session_ref,
                    "size_bytes": size_bytes,
                    "limit_bytes": limit_bytes,
                }),
                SubmitOutcome::Failed { reason_label } => serde_json::json!({
                    "outcome": "failed",
                    "reason": reason_label,
                }),
            };
            entry["unenrolled_preview"] = serde_json::Value::Bool(unenrolled_preview);
            entry
        })
        .collect();
    serde_json::json!({
        "schema_version": "trace_commons.submit_result.v1",
        "unenrolled_preview": unenrolled_preview,
        "notices": notices,
        "results": entries,
    })
}

#[cfg(test)]
mod json_output_tests {
    use super::*;

    #[test]
    fn every_outcome_kind_is_represented() {
        let id = Uuid::new_v4();
        let out = outcomes_to_json(
            &[
                SubmitOutcome::Submitted {
                    submission_id: id,
                    status: "accepted".to_string(),
                },
                SubmitOutcome::AlreadySubmitted {
                    submission_id: id,
                    prior_status: "quarantined".to_string(),
                },
                refused("secret-leak-detected", "sha256:test"),
                SubmitOutcome::Failed {
                    reason_label: "claim-mint-failed".to_string(),
                },
                SubmitOutcome::SkippedParseFailure {
                    reason_label: "parse-failed".to_string(),
                },
            ],
            false,
            &[],
        );

        let results = out["results"].as_array().unwrap();
        // A caller automating submission must be able to see a refusal. The
        // manifest deliberately omits these, so JSON output cannot reuse it.
        assert_eq!(results.len(), 5, "no outcome may be silently dropped");
        assert_eq!(results[0]["outcome"], "submitted");
        assert_eq!(results[1]["outcome"], "already-submitted");
        assert_eq!(
            results[1]["status"], "quarantined",
            "the real prior status must survive into JSON"
        );
        assert_eq!(results[2]["outcome"], "refused");
        assert_eq!(results[2]["reason"], "secret-leak-detected");
        assert_eq!(results[2]["session_ref"], "sha256:test");
        assert_eq!(results[3]["outcome"], "failed");
        assert_eq!(results[4]["outcome"], "skipped");
    }

    #[test]
    fn reasons_stay_labels_and_never_carry_content() {
        // Reason labels are fixed strings by construction. Pinning it here
        // stops a future change from surfacing a response body or path to a
        // caller that logs this output.
        let out = outcomes_to_json(
            &[refused_for_size("sha256:test", 1_600_000)],
            true,
            &["preview notice"],
        );
        let reason = out["results"][0]["reason"].as_str().unwrap();
        assert!(!reason.contains('/'), "a label must not look like a path");
        assert!(reason.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
        assert_eq!(out["unenrolled_preview"], true);
        assert_eq!(out["results"][0]["unenrolled_preview"], true);
        assert_eq!(out["notices"][0], "preview notice");
        assert_eq!(out["results"][0]["session_ref"], "sha256:test");
        assert_eq!(out["results"][0]["size_bytes"], 1_600_000);
        assert_eq!(
            out["results"][0]["limit_bytes"],
            crate::envelope::MAX_ENVELOPE_BYTES
        );
    }
}
