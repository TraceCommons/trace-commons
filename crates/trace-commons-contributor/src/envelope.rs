//! Envelope assembly and redaction pipeline.
//!
//! Maps a locally discovered `SessionTranscript` into a
//! `RawTraceContribution`, then runs it through the protocol crate's
//! deterministic (plus optional NEAR AI) redaction pipeline to produce a
//! `TraceContributionEnvelope` that is safe to submit off-machine.
//!
//! Fail-closed invariant: if the contributor config asks for a PII filter
//! backend and that backend cannot be constructed (missing settings, unknown
//! backend name), this module refuses to build a redactor rather than
//! silently falling back to deterministic-only redaction.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use uuid::Uuid;

use trace_commons_protocol::onboarding::user_subject_hash;
use trace_commons_protocol::privacy_filter_near_ai::NearAiPrivacyFilterAdapter;
use trace_commons_protocol::trace_contribution::{
    ConsentMetadata, ConsentScope, ContributorMetadata, DeterministicTraceRedactor,
    IronclawTraceMetadata, OutcomeMetadata, PrivacyFilterBackendTag, RawTraceContribution,
    RawTraceContributionEvent, ReplayMetadata, TRACE_CONTRIBUTION_POLICY_VERSION, TaskSuccess,
    TokenCounts, TraceAllowedUse, TraceChannel, TraceContributionEnvelope, TraceContributionError,
    TraceContributionEventType, TraceRedactor, ValueMetadata, run_privacy_filter_canary,
    synthetic_privacy_filter_canary_text, synthetic_privacy_filter_canary_values,
};

use crate::config::ContributorConfig;
use crate::source::{
    SessionEvent, SessionEventKind, SessionTranscript, preview_submission_id_for, session_hash,
    submission_id_for,
};

/// Envelopes larger than this are refused before submission (label-only
/// refusal; the oversized content itself is never logged).
///
/// Re-exported from the protocol crate rather than defined here: ingest's
/// body limit and account read-back ceiling derive from the same constant,
/// so the client cannot refuse at a size the server would have accepted (or
/// the reverse).
pub use trace_commons_protocol::trace_contribution::MAX_TRACE_ENVELOPE_BYTES as MAX_ENVELOPE_BYTES;

/// NEAR AI privacy-filter backend settings. Constructed from env by
/// `near_ai_settings_from_env`, or injected directly by callers/tests so
/// tests never have to touch process env (`set_var`/`remove_var` are
/// `unsafe` in edition 2024 and racy under parallel test execution).
/// `PartialEq` and the serde impls exist so the daemon can persist these in
/// its 0600 settings file: a service-managed daemon has no shell environment
/// to read them from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct NearAiSettings {
    pub api_key: String,
    pub base_url: Option<String>,
    pub model: Option<String>,
}

/// Read-only NEAR AI settings lookup from the process environment. Never
/// mutates the environment.
pub fn near_ai_settings_from_env() -> Option<NearAiSettings> {
    let api_key = std::env::var("TRACE_NEAR_AI_PRIVACY_API_KEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())?;
    let base_url = std::env::var("TRACE_NEAR_AI_PRIVACY_BASE_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let model = std::env::var("TRACE_NEAR_AI_PRIVACY_MODEL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    Some(NearAiSettings {
        api_key,
        base_url,
        model,
    })
}

/// Build a `DeterministicTraceRedactor` configured per `cfg`, with explicit
/// `near_ai` settings rather than reading the environment. Tests use this
/// entry point so they never touch process env.
///
/// Fail-closed: if `cfg.pii_filter == Some("near-ai")` and `near_ai` is
/// `None`, this refuses (never silently downgrades to deterministic-only).
/// Any other `pii_filter` value is refused as `"unknown-pii-filter"`.
///
/// Note: the underlying `DeterministicTraceRedactor::new` may additionally
/// attach an env-configured filter via `TRACE_PRIVACY_FILTER_BACKEND`
/// (protocol-crate contract, fail-closed on misconfig), so `cfg.pii_filter`
/// is not the sole filter source.
pub fn build_redactor_with(
    cfg: &ContributorConfig,
    transcript_cwd: Option<&str>,
    near_ai: Option<NearAiSettings>,
) -> Result<DeterministicTraceRedactor> {
    let known_path_prefixes = known_path_prefixes(transcript_cwd);
    let redactor = DeterministicTraceRedactor::new(known_path_prefixes)
        .map_err(|_| anyhow::anyhow!("redactor-config-error"))?;

    match cfg.pii_filter.as_deref() {
        None => Ok(redactor),
        Some("near-ai") => {
            let settings = near_ai
                .ok_or_else(|| anyhow::anyhow!("near-ai-privacy-filter-requires-settings"))?;
            let adapter = NearAiPrivacyFilterAdapter::new(
                settings
                    .base_url
                    .unwrap_or_else(|| "https://cloud-api.near.ai/v1".to_string()),
                settings
                    .model
                    .unwrap_or_else(|| "openai/privacy-filter".to_string()),
                settings.api_key,
                Duration::from_millis(10_000),
                1024 * 1024,
            )
            .map_err(|_| anyhow::anyhow!("near-ai-adapter-config-error"))?;
            Ok(redactor.with_privacy_filter(Arc::new(adapter), PrivacyFilterBackendTag::NearAi))
        }
        Some(_) => Err(anyhow::anyhow!("unknown-pii-filter")),
    }
}

/// Build an environment-independent deterministic redactor for an
/// unenrolled preview. This ignores both CLI/config filter selection and
/// inherited backend variables by construction.
pub fn build_deterministic_preview_redactor(
    transcript_cwd: Option<&str>,
) -> DeterministicTraceRedactor {
    DeterministicTraceRedactor::deterministic_only(known_path_prefixes(transcript_cwd))
}

fn known_path_prefixes(transcript_cwd: Option<&str>) -> Vec<String> {
    let mut known_path_prefixes = Vec::new();
    if let Some(home) = dirs::home_dir() {
        known_path_prefixes.push(home.to_string_lossy().into_owned());
    }
    if let Some(cwd) = transcript_cwd {
        known_path_prefixes.push(cwd.to_string());
    }
    known_path_prefixes
}

/// Production entry point: thin wrapper over `build_redactor_with` that
/// reads NEAR AI settings from the environment.
pub fn build_redactor(
    cfg: &ContributorConfig,
    transcript_cwd: Option<&str>,
) -> Result<DeterministicTraceRedactor> {
    build_redactor_with(cfg, transcript_cwd, near_ai_settings_from_env())
}

/// Run the synthetic privacy-filter canary text through `redactor` and
/// refuse if any canary value it is responsible for survives redaction.
///
/// The canary set (`synthetic_privacy_filter_canary_values`) includes a
/// value that is only secret-*shaped* for a PII filter backend to catch
/// (`tc_canary_secret_...`); it does not match any of the deterministic
/// pipeline's hardcoded secret-leak patterns (OpenAI/GitHub/AWS/provider
/// token prefixes, PEM headers) and is not path- or email-shaped, so a
/// deterministic-only redactor (no privacy filter attached) cannot strip it.
/// This check is scoped to the values the deterministic pass *is*
/// responsible for -- the private-email and local-path shaped canary
/// values -- so it can assert real behavior of a correctly-built redactor
/// without depending on a live PII filter backend. The submit-time
/// contract (a canary hit aborts the batch) is unaffected: whatever
/// redactor is actually configured (deterministic-only or with a privacy
/// filter attached) gets this same self-test run against it before
/// submission.
pub fn canary_self_test(redactor: &DeterministicTraceRedactor) -> Result<()> {
    let canary_text = synthetic_privacy_filter_canary_text();
    let (redacted, _report) = redactor.redact_text(&canary_text);

    for value in synthetic_privacy_filter_canary_values() {
        let deterministic_pass_owns_this_value = value.contains('@') || value.starts_with('/');
        if !deterministic_pass_owns_this_value {
            continue;
        }
        if redacted.contains(&value) {
            anyhow::bail!("privacy-filter-canary-failed");
        }
    }
    Ok(())
}

/// Once-per-batch precondition, extended to also exercise any privacy-filter
/// backend attached to `redactor` (NEAR AI, sidecar, ...).
///
/// `canary_self_test` only checks the deterministic pass (the email/path
/// shaped canary values it is responsible for). A well-formed no-op filter
/// -- one that returns 200 with an empty span list for everything -- passes
/// that check trivially, because the deterministic pass never depended on
/// the filter to strip anything. This function additionally runs the
/// protocol crate's `run_privacy_filter_canary` directly against the
/// attached adapter (if any) and fails closed when the canary's synthetic
/// secret/email/path values survive redaction through that adapter alone,
/// which is exactly the failure mode a no-op or broken filter exhibits.
pub async fn canary_self_test_async(redactor: &DeterministicTraceRedactor) -> Result<()> {
    canary_self_test(redactor)?;

    if let Some(adapter) = redactor.attached_privacy_filter() {
        let report = run_privacy_filter_canary(adapter.as_ref())
            .await
            .map_err(|_| anyhow::anyhow!("privacy-filter-canary-failed"))?;
        if !report.healthy {
            anyhow::bail!("privacy-filter-canary-failed");
        }
    }
    Ok(())
}

/// The one refusal from the redaction pipeline this crate reports under its
/// own name rather than the generic label.
///
/// It is the only pipeline refusal a contributor can act on: their
/// correction contains something credential-shaped, and what they should do
/// is remove it and rotate it. Every other failure is a condition of the
/// machine, not of anything they wrote, so it stays behind
/// `trace-redaction-failed`. The string is a fixed label and names neither
/// the text nor the match.
pub const REASON_CORRECTION_CREDENTIAL: &str = "correction-credential-detected";

/// Run `raw` through `redactor`, mapping any failure to a label-only error
/// (never trace content).
pub async fn redact_to_envelope(
    redactor: &DeterministicTraceRedactor,
    raw: RawTraceContribution,
) -> Result<TraceContributionEnvelope> {
    redactor.redact_trace(raw).await.map_err(|error| {
        // Matched on the pipeline's own label, so this cannot widen: any
        // reason but this exact one still collapses to the generic label
        // that every caller has always seen.
        match &error {
            TraceContributionError::RedactionFailed { reason }
                if reason == REASON_CORRECTION_CREDENTIAL =>
            {
                anyhow::anyhow!("{REASON_CORRECTION_CREDENTIAL}")
            }
            _ => anyhow::anyhow!("trace-redaction-failed"),
        }
    })
}

/// Re-scan the *finished* envelope with the secret detector and report
/// whether any secret shape survived redaction. Defense-in-depth: a
/// correctly-redacted envelope yields zero detector hits, but a survivor (a
/// detect-then-redact bug, or a non-string payload value the string-leaf
/// pass never visited) is still caught here and fails the session closed.
///
/// Unlike a whole-token diff, this only flags secret *shapes* (pattern
/// matches, PEM blocks, cue-gated entropy) -- ordinary prose and the
/// `{"record_type":...}` markers on Opaque events are not secret-shaped and
/// never trip it, so there are no prose false positives.
pub fn envelope_has_residual_secret(
    redactor: &DeterministicTraceRedactor,
    envelope: &TraceContributionEnvelope,
) -> Result<bool> {
    let json = serde_json::to_string(envelope)
        .map_err(|_| anyhow::anyhow!("envelope-serialize-failed"))?;
    let (_out, report) = redactor.redact_text(&json);
    Ok(report.blocked_secret_detected)
}

/// The site reported when the residual scan could not run at all.
///
/// Deliberately inside the `residual_secret_at:` family rather than a family
/// of its own. Every shell splits the count map exactly one way -- this
/// family is a survivor, everything else is a removal -- so a new family
/// would be silently summed into "removed by pattern", which is the
/// reassuring answer for the one case where nothing was verified. The braces
/// mark it as a placeholder rather than a path, matching how the scan itself
/// collapses a non-schema-shaped object key to `{}`.
pub const RESIDUAL_SCAN_UNAVAILABLE_SITE: &str = "{scan_unavailable}";

/// Residual-secret labels for a finished envelope, fail-closed.
///
/// On a scan the redactor could not complete (serialization failure, or an
/// envelope past the scan's node/depth budget) this reports a survivor at
/// [`RESIDUAL_SCAN_UNAVAILABLE_SITE`] rather than an empty map. That is the
/// point: an unscanned envelope must not render as a scanned-clean one on
/// the screen where somebody is deciding whether to send it.
///
/// Refusing the preview outright was the other candidate and is worse here:
/// it would deny a contributor any view of a large session, and a preview is
/// a disclosure surface, not a gate -- the submit path keeps its own
/// refusal in `envelope_has_residual_secret`.
pub fn residual_secret_labels_fail_closed(
    redactor: &DeterministicTraceRedactor,
    envelope: &TraceContributionEnvelope,
) -> BTreeMap<String, u32> {
    match trace_commons_protocol::trace_contribution::residual_secret_labels(redactor, envelope) {
        Ok(labels) => labels,
        Err(_) => BTreeMap::from([(
            format!(
                "{}{RESIDUAL_SCAN_UNAVAILABLE_SITE}",
                trace_commons_protocol::trace_contribution::RESIDUAL_SECRET_AT_PREFIX
            ),
            1,
        )]),
    }
}

/// Serialize `raw` and refuse (label-only) if the pre-redaction payload
/// already exceeds `MAX_ENVELOPE_BYTES`. The finished envelope carries the
/// same event content plus additional metadata (trace card, consent, etc.),
/// so an over-limit raw contribution cannot serialize to an in-limit
/// envelope. Checking here lets `submit` skip the expensive chunked,
/// networked privacy-filter pass on sessions that would be refused for size
/// anyway; `envelope_size_ok` remains the authoritative post-redaction guard.
pub fn raw_contribution_size_ok(raw: &RawTraceContribution) -> Result<usize> {
    let size = raw_contribution_size(raw)?;
    if size > MAX_ENVELOPE_BYTES {
        anyhow::bail!("session too large");
    }
    Ok(size)
}

/// Serialized size of a raw contribution before redaction.
pub fn raw_contribution_size(raw: &RawTraceContribution) -> Result<usize> {
    serde_json::to_vec(raw)
        .map(|bytes| bytes.len())
        .map_err(|_| anyhow::anyhow!("raw-serialize-failed"))
}

/// Serialize `envelope` and refuse (label-only) if it exceeds
/// `MAX_ENVELOPE_BYTES`. Returns the serialized byte size on success.
pub fn envelope_size_ok(envelope: &TraceContributionEnvelope) -> Result<usize> {
    let size = envelope_size(envelope)?;
    if size > MAX_ENVELOPE_BYTES {
        anyhow::bail!("session too large");
    }
    Ok(size)
}

/// Serialized size of a finished envelope before upload.
///
/// Drives the serializer into a counting sink rather than a `Vec<u8>`: the
/// only thing any caller wants here is the byte count, and a redacted
/// envelope can run to hundreds of megabytes for a large session, so
/// collecting a full serialized copy just to call `.len()` on it once was an
/// allocation purely in service of measuring it.
pub fn envelope_size(envelope: &TraceContributionEnvelope) -> Result<usize> {
    let mut counter = ByteCounter(0);
    serde_json::to_writer(&mut counter, envelope)
        .map_err(|_| anyhow::anyhow!("envelope-serialize-failed"))?;
    Ok(counter.0)
}

/// A `std::io::Write` sink that counts bytes written and discards them.
///
/// Used to measure a serializer's output size in O(1) memory instead of
/// collecting the bytes into a buffer only to read their length.
struct ByteCounter(usize);

impl std::io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What the contributor said about how the session went.
///
/// `OutcomeMetadata` has modelled an outcome since v1 and this client wrote
/// `default()` on every envelope, so `task_success` was `Unknown` on every
/// trace it has ever sent. Nothing in a transcript answers the question:
/// an agent that stops has not thereby succeeded, and a harness that records
/// no error has not thereby done what was asked. Only the person who asked
/// knows, so it is asked rather than inferred (issue #298).
///
/// Two states on purpose. A verdict is a judgement about the task, not text,
/// so it carries no PII and needs no consent decision -- which is exactly
/// what a free-text correction would need, and why that is not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContributorVerdict {
    Worked,
    Partly,
    Failed,
}

impl ContributorVerdict {
    /// Wire name, so a CLI flag and an IPC parameter cannot drift apart.
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "worked" => Some(Self::Worked),
            "partly" => Some(Self::Partly),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    /// The `TaskSuccess` this verdict maps to. Shared by `outcome` (below)
    /// and `apply_verdict`, which cannot delegate to `outcome` itself --
    /// assigning a whole `OutcomeMetadata` there would clobber the stored
    /// envelope's other outcome fields with defaults.
    fn task_success(self) -> TaskSuccess {
        match self {
            Self::Worked => TaskSuccess::Success,
            Self::Partly => TaskSuccess::Partial,
            Self::Failed => TaskSuccess::Failure,
        }
    }

    /// Writes `task_success` and nothing else.
    ///
    /// `user_feedback` is deliberately left `None`. It is a different
    /// question -- satisfaction rather than completion -- and the two
    /// genuinely diverge: a run can complete the task by a route the
    /// contributor dislikes, or fail at the task while doing the right
    /// thing. Setting both from one keystroke records a signal that was
    /// never given.
    ///
    /// `Partly` is what makes that concrete. It has no honest thumb, so any
    /// mapping onto `ThumbsUp`/`ThumbsDown` would have to invent one.
    ///
    /// This leaves `user_feedback` free for a real satisfaction control
    /// later, including `Correction` once there is a surface to collect one
    /// (the pipeline already handles `human_correction` -- credential
    /// detection only, since a correction is stored as written; what is
    /// missing is the UI, not the pipeline).
    fn outcome(self) -> OutcomeMetadata {
        OutcomeMetadata {
            task_success: self.task_success(),
            ..OutcomeMetadata::default()
        }
    }
}

/// Map a locally discovered transcript into a `RawTraceContribution` ready
/// for redaction. See the field-mapping table in the task brief for the
/// exact provenance of every field.
pub fn build_raw_contribution(
    t: &SessionTranscript,
    cfg: &ContributorConfig,
    now: DateTime<Utc>,
) -> RawTraceContribution {
    build_raw_contribution_with_id(t, cfg, now, submission_id_for(&t.session_hash), None, None)
}

/// The longest correction the client will accept, in characters.
///
/// A correction is an explanation of what a run got wrong, not a document.
/// The cap exists so a refusal happens at the keyboard, where the person can
/// see it and shorten what they wrote, rather than as an oversized-envelope
/// refusal after the fact. Characters rather than bytes so the limit means
/// the same thing in every script.
pub const MAX_CORRECTION_CHARS: usize = 2_000;

/// The same, carrying a verdict the contributor supplied for this session.
pub fn build_raw_contribution_with_verdict(
    t: &SessionTranscript,
    cfg: &ContributorConfig,
    now: DateTime<Utc>,
    verdict: Option<ContributorVerdict>,
) -> RawTraceContribution {
    build_raw_contribution_with_id(
        t,
        cfg,
        now,
        submission_id_for(&t.session_hash),
        verdict,
        None,
    )
}

/// The same again, carrying the contributor's written correction.
///
/// A correction has to be present when the redaction pipeline runs, which is
/// why it is a build-time input rather than a post-redaction stamp like
/// [`apply_verdict`]. Two things depend on it being there:
///
/// - credential detection, which refuses the whole submission on a High or
///   Critical match rather than masking it (`detect_correction_credentials`),
/// - `ConsentMetadata::correction_included`, which is derived from the
///   outcome this builds and is what enrols the envelope for the PII
///   backstop hold and floors its residual risk at Medium.
///
/// Stamping a correction onto an already-redacted envelope would skip both.
pub fn build_raw_contribution_with_correction(
    t: &SessionTranscript,
    cfg: &ContributorConfig,
    now: DateTime<Utc>,
    verdict: Option<ContributorVerdict>,
    correction: Option<&str>,
) -> RawTraceContribution {
    build_raw_contribution_with_id(
        t,
        cfg,
        now,
        submission_id_for(&t.session_hash),
        verdict,
        correction,
    )
}

/// Build the same raw contribution shape with a disjoint preview id.
pub fn build_preview_raw_contribution(
    t: &SessionTranscript,
    cfg: &ContributorConfig,
    now: DateTime<Utc>,
) -> RawTraceContribution {
    build_raw_contribution_with_id(
        t,
        cfg,
        now,
        preview_submission_id_for(&t.session_hash),
        None,
        None,
    )
}

fn build_raw_contribution_with_id(
    t: &SessionTranscript,
    cfg: &ContributorConfig,
    now: DateTime<Utc>,
    submission_id: Uuid,
    verdict: Option<ContributorVerdict>,
    correction: Option<&str>,
) -> RawTraceContribution {
    let mut feature_flags = BTreeMap::new();
    feature_flags.insert("agent".to_string(), t.source.to_string());
    feature_flags.insert(
        "agent_version".to_string(),
        t.agent_version
            .clone()
            .unwrap_or_else(|| "unknown".to_string()),
    );
    // The project basename is NOT sent, in any form.
    //
    // It used to ship in the clear as `project`, which was the leak in #207:
    // it is derived from the same working directory `cwd_hash` exists to
    // protect, and a repo or client name identifies on its own.
    //
    // It is not hashed either, and that was a deliberate choice rather than
    // an oversight. `session_hash` is unsalted SHA-256, and basenames are
    // dictionary-shaped -- `dotfiles`, `api`, `backend`, `monorepo` -- so a
    // wordlist inverts them in constant time. Worse, an unsalted digest is
    // stable across every contributor forever, so it preserves exactly the
    // cross-contributor linkage the cleartext field had (two people working
    // on identically named repos are still linkable) while reading as though
    // it were protected. Removing the evidence of a capability without
    // removing the capability is worse than leaving it visible.
    //
    // Nothing server-side reads this key -- the only `"project"` in
    // trace-commons-protocol is an unrelated issue-identity redaction rule --
    // so there is no consumer to preserve. Local `--project` scoping matches
    // on the in-memory field and never needed the serialized one.
    //
    // If per-project grouping is ever actually wanted, the answer is an HMAC
    // keyed by the device key, which keeps a contributor's own traces
    // groupable while destroying both the dictionary attack and the
    // cross-contributor linkage. Do not reintroduce a bare hash.
    feature_flags.insert(
        "cwd_hash".to_string(),
        t.cwd
            .as_ref()
            .map(|cwd| session_hash(cwd.as_bytes()))
            .unwrap_or_else(|| "unknown".to_string()),
    );

    let events: Vec<RawTraceContributionEvent> =
        raw_events_for_with_routing(&t.events, &t.routing, now);
    // Which tools a replay would have to stand up. The list was empty on every
    // envelope this client ever sent, which also zeroed the scorecard's
    // coverage term for a transcript that plainly covered tools.
    let required_tools: Vec<String> = events
        .iter()
        .filter(|event| event.event_type == TraceContributionEventType::ToolCall)
        .filter_map(|event| event.tool_name.clone())
        .collect::<BTreeSet<String>>()
        .into_iter()
        .collect();
    // Routing rows are attribution metadata, not mappable conversation
    // content -- a session with zero real events but one routing row is not
    // replayable just because `events` is non-empty. See
    // `compute_value_scorecard`'s matching guard in the protocol crate.
    let replayable = events
        .iter()
        .any(|event| event.event_type != TraceContributionEventType::RoutingDecision);
    // The declaration describes the payload above, rather than asserting a
    // constant. See `declared_content_presence`.
    let presence = declared_content_presence(&events);
    // Built before the consent block so the declaration can describe it. A
    // correction is its own content class, not message text: see
    // `ConsentMetadata::correction_included`. `false` unless the caller
    // supplied a correction, which only the approval path does.
    let mut outcome = verdict.map(ContributorVerdict::outcome).unwrap_or_default();
    // Whitespace is not a correction. Normalising here rather than at the
    // callers means every path -- the socket, the C ABI, a future CLI flag --
    // gets the same answer for "  ", and the declaration below cannot end up
    // true for an envelope carrying nothing a reader could use.
    outcome.human_correction = correction
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string);
    let correction_included = outcome
        .human_correction
        .as_ref()
        .is_some_and(|text| !text.is_empty());

    RawTraceContribution {
        trace_id: Uuid::new_v4(),
        submission_id,
        created_at: now,
        ironclaw: IronclawTraceMetadata {
            version: t
                .agent_version
                .clone()
                .unwrap_or_else(|| "unknown".to_string()),
            engine_version: None,
            feature_flags,
            channel: TraceChannel::Cli,
            model_name: t.model.clone(),
        },
        consent: ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: {
                let parsed = parse_scope_names(&cfg.consent_scopes);
                if parsed.is_empty() {
                    vec![ConsentScope::DebuggingEvaluation]
                } else {
                    parsed
                }
            },
            message_text_included: presence.message_text,
            tool_payloads_included: presence.tool_payloads,
            correction_included,
            routing_metadata_included: presence.routing_metadata,
            revocable: true,
        },
        contributor: ContributorMetadata {
            pseudonymous_contributor_id: Some(user_subject_hash(&cfg.user_subject)),
            tenant_scope_ref: Some(cfg.tenant_id.clone()),
            credit_account_ref: None,
            revocation_handle: Uuid::new_v4(),
        },
        events,
        outcome,
        replay: ReplayMetadata {
            // This said `false` unconditionally, and the scorecard reads it as
            // authoritative: sufficiency can only lower a score, never raise
            // one past this flag. So the one capture path that carries a
            // prompt, the arguments of every call and the results they
            // returned scored 0.20-weighted replayability of zero on every
            // envelope, while the web-history path -- carrying tool names and
            // nothing else -- scored full marks (issue #298).
            //
            // The flag's job is to let an emitter say a trace is KNOWN to be
            // unreplayable. A local transcript with events is not that, and
            // how much of a replay actually survived is measured from the
            // events by `replay_sufficiency`. State only what is true here and
            // let the measure do the rest.
            replayable,
            required_tools,
            tool_manifest_hashes: BTreeMap::new(),
            expected_assertions: vec![],
            replay_notes: vec![
                "Captured from a local agent transcript; environment and tool versions are not pinned."
                    .to_string(),
            ],
        },
        embedding_analysis: None,
        value: ValueMetadata::default(),
        conversation_id: t.conversation_id.clone(),
    }
}

/// Parse wire names to typed scopes; unknown names are skipped (they were
/// validated at login/claim time) — never panics.
pub fn parse_scope_names(names: &[String]) -> Vec<ConsentScope> {
    names
        .iter()
        .filter_map(|name| match name.as_str() {
            "debugging_evaluation" => Some(ConsentScope::DebuggingEvaluation),
            "benchmark_only" => Some(ConsentScope::BenchmarkOnly),
            "ranking_training" => Some(ConsentScope::RankingTraining),
            "model_training" => Some(ConsentScope::ModelTraining),
            "public_attribution" => Some(ConsentScope::PublicAttribution),
            _ => None,
        })
        .collect()
}

/// Same for allowed uses (wire names -> TraceAllowedUse, unknown skipped).
pub fn parse_use_names(names: &[String]) -> Vec<TraceAllowedUse> {
    names
        .iter()
        .filter_map(|name| match name.as_str() {
            "debugging" => Some(TraceAllowedUse::Debugging),
            "evaluation" => Some(TraceAllowedUse::Evaluation),
            "benchmark_generation" => Some(TraceAllowedUse::BenchmarkGeneration),
            "ranking_model_training" => Some(TraceAllowedUse::RankingModelTraining),
            "model_training" => Some(TraceAllowedUse::ModelTraining),
            "aggregate_analytics" => Some(TraceAllowedUse::AggregateAnalytics),
            _ => None,
        })
        .collect()
}

/// Overwrite the envelope's consent metadata and trace card with the
/// claim-granted set. Called after redaction, before size check/upload.
///
/// Moved to `trace-commons-protocol` and re-exported here so no caller moved.
/// The redaction witness has to apply the grants before it serialises and
/// digests the envelope -- a grant stamped afterwards is a byte change the
/// certificate does not cover -- and it cannot reach this crate to do it.
pub use trace_commons_protocol::trace_contribution::apply_granted_scopes;

/// Stamp the contributor's verdict onto an already-redacted envelope.
///
/// The daemon path cannot supply a verdict at build time: the envelope is
/// built for the preview, before the contributor has answered, and the
/// upload sends those stored bytes rather than rebuilding. So the verdict is
/// applied here, the same post-redaction mutation `apply_granted_scopes`
/// performs.
///
/// Writes `task_success` only. `user_feedback` is a different question --
/// satisfaction rather than completion -- and is deliberately left alone;
/// see `ContributorVerdict::outcome`.
pub fn apply_verdict(envelope: &mut TraceContributionEnvelope, verdict: ContributorVerdict) {
    envelope.outcome.task_success = verdict.task_success();
}

/// Whether the built events actually carry message text / tool payloads.
///
/// `docs/trace-spec.md` defines these consent booleans as a FACTUAL
/// DECLARATION of what the envelope contains, not a preference and not a
/// default. This client hardcoded both to `true` on every envelope, which is
/// wrong in both directions:
///
/// - It over-declares. A trace with no tool calls declared
///   `tool_payloads_included: true`, which puts it at Medium residual risk and
///   quarantines it on a default deployment -- for content it never carried.
/// - It ignores what the trace is. The declaration is supposed to describe the
///   payload, and a constant cannot.
///
/// Derived from the events as built, after any content gating, so the
/// declaration and the payload cannot disagree.
///
/// A struct rather than a tuple of three bools: the call site at the envelope
/// builder reads them apart, and three positional bools is exactly the shape
/// that silently swaps two of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DeclaredPresence {
    message_text: bool,
    tool_payloads: bool,
    routing_metadata: bool,
}

fn declared_content_presence(events: &[RawTraceContributionEvent]) -> DeclaredPresence {
    let mut presence = DeclaredPresence::default();

    for event in events {
        let has_content = event
            .content
            .as_ref()
            .is_some_and(|content| !content.is_empty());
        if has_content {
            match event.event_type {
                TraceContributionEventType::UserMessage
                | TraceContributionEventType::AssistantMessage
                | TraceContributionEventType::Reasoning
                | TraceContributionEventType::RoutingDecision
                | TraceContributionEventType::Feedback => presence.message_text = true,
                TraceContributionEventType::ToolCall
                | TraceContributionEventType::ToolResult
                | TraceContributionEventType::HttpExchange => presence.tool_payloads = true,
            }
        }
        // A structured payload is tool-call content regardless of event kind.
        // A bare `tool_name` deliberately does NOT count: the name is metadata
        // about which tool ran, not the payload the flag declares. Neither
        // does the capture path's withheld-payload marker -- an object whose
        // keys are all fixed marker names and whose values are all booleans
        // or nulls. Any other non-blank key IS content, because a key is as
        // free-form as the string beside it. All of that is the same rule the
        // server half applies, from the same function, because the two
        // derivations are required to agree.
        //
        // Must agree with `derive_envelope_content_presence` in the protocol
        // crate. If the client declares honestly and the server then corrects
        // that declaration upward, the contributor is penalised for telling
        // the truth. `the_two_content_derivations_agree` pins this.
        //
        // That upward-correction mechanism does not exist for
        // `routing_metadata`: `reconcile_consent_declarations` has no arm for
        // it, deliberately, because nothing consumes
        // `consent.routing_metadata_included` as a protective gate the way it
        // consumes the other two flags. Concordance is still worth keeping
        // here (a client and server that disagree about what an envelope
        // contains is a bug either way), it just is not backstopped by a
        // server-side correction the way the other two flags are.
        if trace_commons_protocol::trace_contribution::payload_carries_readable_content(
            &event.structured_payload,
        ) {
            match event.event_type {
                TraceContributionEventType::RoutingDecision => presence.routing_metadata = true,
                _ => presence.tool_payloads = true,
            }
        }
    }

    presence
}

/// Map a whole transcript, so a result can name the call it answers.
///
/// Pairing needs state across events, which is why this is not simply a `map`
/// over `raw_event_for`: the call ids the adapters now carry are matched here
/// into `parent_event_id`. Without it, array order is the only sequence signal
/// an envelope has -- the finding in issue #298 -- and order is not something
/// a consumer can verify.
fn raw_events_for(events: &[SessionEvent], now: DateTime<Utc>) -> Vec<RawTraceContributionEvent> {
    let mut mapped: Vec<RawTraceContributionEvent> = Vec::with_capacity(events.len());
    // Where each call landed, so a result can both point at it and take from
    // it what the transcript recorded only once.
    let mut call_slots: BTreeMap<&str, usize> = BTreeMap::new();

    for event in events {
        let mut raw = raw_event_for(event, now);
        match (&event.kind, event.tool_call_id.as_deref()) {
            (SessionEventKind::ToolCall, Some(call_id)) => {
                call_slots.insert(call_id, mapped.len());
            }
            (SessionEventKind::ToolResult, Some(call_id)) => {
                if let Some(&slot) = call_slots.get(call_id) {
                    raw.parent_event_id = Some(mapped[slot].event_id);
                    // A result record names the call it answers and not the
                    // tool that ran it -- Claude Code's `tool_result` block
                    // carries only `tool_use_id`, and Codex's output only
                    // `call_id` -- so every adapter had to leave this `None`.
                    // The pairing supplies it, which is what gives the result
                    // a `tool_category` and stops a consumer having to walk
                    // `parent_event_id` to learn which tool it is holding.
                    if raw.tool_name.is_none() {
                        raw.tool_name = mapped[slot].tool_name.clone();
                    }
                    // The verdict travels the other way. `is_error` sits on
                    // the result, but "which tool failed" is a question about
                    // the call, and a consumer scanning calls should not have
                    // to join to results to answer it. The capture path
                    // already sets `success` on both halves; this makes the
                    // local path agree. Never overwrite a verdict the call
                    // already carries.
                    if mapped[slot].success.is_none() {
                        mapped[slot].success = raw.success;
                    }
                }
            }
            _ => {}
        }
        mapped.push(raw);
    }

    mapped
}

/// [`raw_events_for`] plus one `RoutingDecision` per joined inference hop.
///
/// The routing events are appended rather than interleaved. A hop is not a
/// step in the transcript -- it is how a step was served -- and giving it a
/// position in the sequence would assert an ordering the ledger cannot
/// support: rows are timestamped by the proxy, session events by the harness,
/// and the two clocks are not the same clock.
fn raw_events_for_with_routing(
    events: &[SessionEvent],
    routing: &[crate::routing::RoutedExchange],
    now: DateTime<Utc>,
) -> Vec<RawTraceContributionEvent> {
    let mut mapped = raw_events_for(events, now);
    mapped.extend(routing.iter().map(raw_routing_event_for));
    mapped
}

/// One inference hop as an envelope event.
///
/// Numbers go in the typed fields the event already carries. Only the labels
/// with no typed home -- backend, rung, attempts, the requested/served model
/// pair, the cache token split -- go in `structured_payload`, and that is what
/// `routing_metadata` declares. `content` stays empty: nothing here is text
/// from the session, and a routing event that carried prose would declare
/// `message_text` and mean something else entirely.
fn raw_routing_event_for(row: &crate::routing::RoutedExchange) -> RawTraceContributionEvent {
    RawTraceContributionEvent {
        event_id: Uuid::new_v4(),
        parent_event_id: None,
        event_type: TraceContributionEventType::RoutingDecision,
        timestamp: row.started_at,
        content: None,
        structured_payload: serde_json::json!({
            "backend": row.backend,
            "facade": row.facade,
            "rung": row.rung,
            "attempts": row.attempts,
            "requested_model": row.requested_model,
            "served_model": row.served_model,
            "cache_read_tokens": row.cache_read_tokens,
            "cache_write_tokens": row.cache_write_tokens,
            "status": row.status,
        }),
        tool_name: None,
        tool_call_id: None,
        latency_ms: row.total_ms.and_then(|ms| u64::try_from(ms).ok()),
        token_counts: match (row.input_tokens, row.output_tokens) {
            (Some(input), Some(output)) => Some(TokenCounts {
                input_tokens: u32::try_from(input).unwrap_or(u32::MAX),
                output_tokens: u32::try_from(output).unwrap_or(u32::MAX),
            }),
            // `None`, never a fabricated zero: an unreported count that summed
            // as zero would understate what the session actually consumed.
            _ => None,
        },
        cost_usd: row.cost_usd.and_then(|usd| {
            trace_commons_protocol::trace_contribution::Decimal::try_from(usd).ok()
        }),
        success: None,
        failure_modes: Vec::new(),
    }
}

fn raw_event_for(e: &SessionEvent, now: DateTime<Utc>) -> RawTraceContributionEvent {
    let (event_type, content, structured_payload) = match e.kind {
        SessionEventKind::User => (
            TraceContributionEventType::UserMessage,
            e.content.clone(),
            e.structured.clone(),
        ),
        SessionEventKind::Assistant => (
            TraceContributionEventType::AssistantMessage,
            e.content.clone(),
            e.structured.clone(),
        ),
        SessionEventKind::Reasoning => (
            TraceContributionEventType::Reasoning,
            e.content.clone(),
            e.structured.clone(),
        ),
        // The adapters hand over the tool's argument object itself. Name it,
        // rather than shipping a bare blob: `replay_sufficiency` looks for
        // arguments under a known key, so a payload of `{"file_path": ...}`
        // read as "no arguments recorded" even though the arguments were all
        // there. Absent arguments stay `Null` rather than becoming
        // `{"arguments": null}`, which would claim an empty payload exists.
        SessionEventKind::ToolCall => (
            TraceContributionEventType::ToolCall,
            e.content.clone(),
            if e.structured.is_null() {
                Value::Null
            } else {
                serde_json::json!({ "arguments": e.structured.clone() })
            },
        ),
        SessionEventKind::ToolResult => (
            TraceContributionEventType::ToolResult,
            e.content.clone(),
            e.structured.clone(),
        ),
        // There is no generic/opaque event type in the v1 schema; map to
        // ToolResult with no content. The `structured_payload`'s
        // `{"record_type": ...}` marker (set by the source adapter)
        // preserves provenance without carrying any record content.
        SessionEventKind::Opaque => (
            TraceContributionEventType::ToolResult,
            None,
            e.structured.clone(),
        ),
    };

    RawTraceContributionEvent {
        event_id: Uuid::new_v4(),
        // Set by `raw_events_for`, which is the only caller and the only
        // place that can see more than one event at a time.
        parent_event_id: None,
        event_type,
        timestamp: e.timestamp.unwrap_or(now),
        content,
        structured_payload,
        tool_name: e.tool_name.clone(),
        tool_call_id: e.tool_call_id.clone(),
        latency_ms: None,
        token_counts: e
            .token_counts
            .map(|(input_tokens, output_tokens)| TokenCounts {
                input_tokens,
                output_tokens,
            }),
        // What the step would have cost at the provider's published list
        // price -- not a bill, and not money the contributor was charged;
        // most sessions run under a subscription. `None` wherever the
        // transcript does not say enough to price it honestly, which is
        // every source that reports no model or an incomplete usage report,
        // and every model absent from the price table. Never a zero: a
        // fabricated zero would silently understate. See `crate::pricing`.
        cost_usd: match (e.token_counts, e.served_by.as_ref()) {
            (Some((input_tokens, output_tokens)), Some(served)) => crate::pricing::list_price_usd(
                &served.model,
                &crate::pricing::TokenUsage {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens: served.cache_read_tokens,
                    cache_write_5m_tokens: served.cache_write_5m_tokens,
                    cache_write_1h_tokens: served.cache_write_1h_tokens,
                },
            ),
            _ => None,
        },
        success: e.success,
        failure_modes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{TraceSource, claude_code::ClaudeCodeSource};

    fn fixture_transcript() -> crate::source::SessionTranscript {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = ClaudeCodeSource::new(root);
        let refs = src.discover().unwrap();
        src.load(&refs[0]).unwrap()
    }

    fn sample_routed_exchange() -> crate::routing::RoutedExchange {
        crate::routing::RoutedExchange {
            id: None,
            started_at: chrono::Utc::now(),
            client_session_id: Some("s-1".to_string()),
            total_ms: Some(1200),
            facade: "anthropic".to_string(),
            backend: "claude-sub".to_string(),
            requested_model: Some("claude-opus-4-6".to_string()),
            served_model: Some("claude-opus-4-6".to_string()),
            rung: "same_model".to_string(),
            attempts: 1,
            input_tokens: Some(1000),
            cache_read_tokens: Some(500),
            cache_write_tokens: None,
            output_tokens: Some(200),
            cost_usd: Some(0.02),
            status: 200,
        }
    }

    fn test_config() -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: "sha256:00".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: None,
        }
    }

    /// The contributor's serialised envelope is not sensitive to
    /// `serde_json::Map` ordering, so gaining `trace-commons-attestation` --
    /// and with it `dcap-qvl`, whose mandatory `std` feature turns on
    /// `serde_json/preserve_order` -- moves no digest.
    ///
    /// # Why this was checked, and why it is not a blocker
    ///
    /// `preserve_order` swaps `serde_json::Map` from a `BTreeMap` to an
    /// insertion-ordered `IndexMap`, and cargo unifies features across a
    /// build, so one dependency anywhere can silently reorder every
    /// `Value::Object` in every crate. That is not hypothetical: adding
    /// `dcap-qvl` on a branch enabled it and moved a golden envelope digest in
    /// a crate the branch never touched.
    ///
    /// It is not a blocker here, for a reason worth recording so the next
    /// person does not have to re-derive it. **The feature was already on in
    /// this crate's graph before the witness work**, reached through the
    /// existing `cfg(not(windows))` dev-dependency on `trace-commons-server`
    /// and from there `dcap-qvl` -- `cargo tree -p trace-commons-contributor
    /// -e features -i serde_json` shows it. So this crate's suite, including
    /// everything downstream of `redaction_hash`, has been running under an
    /// `IndexMap` and passing all along. The only build graph the new
    /// dependency changes is the standalone `--no-default-features` one, which
    /// is a `cargo check` job rather than a test job, and
    /// `trace_commons_protocol::canonical_json` plus the
    /// `serde_json preserve_order guard` CI job exist for exactly that case.
    ///
    /// What this test pins is the invariant those rest on: every path whose
    /// bytes are hashed routes through `canonicalize`, so the ordering of the
    /// backing map cannot be observed in a digest. Under a `BTreeMap` the
    /// assertion is true for free; under `preserve_order` it is real work, and
    /// it is the version that runs in this crate's graph.
    /// A residual scan that cannot cover the envelope must report an
    /// explicit unchecked survivor, never an empty map.
    ///
    /// An empty map is indistinguishable from "scanned, nothing survived",
    /// and it would render on the consent surface as the reassuring answer
    /// for the one case where nothing was actually verified. The label goes
    /// into the `residual_secret_at:` family on purpose: every shell splits
    /// the count map on exactly that prefix, so a family of its own would be
    /// summed into "removed by pattern".
    #[tokio::test]
    async fn a_residual_scan_that_cannot_run_reports_unchecked_not_clean() {
        let transcript = fixture_transcript();
        let cfg = test_config();
        let raw = build_raw_contribution(&transcript, &cfg, chrono::Utc::now());
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let mut envelope = redact_to_envelope(&redactor, raw).await.unwrap();

        // Baseline: this envelope scans clean, so the assertion below cannot
        // pass merely because the fixture carries a real survivor.
        assert!(residual_secret_labels_fail_closed(&redactor, &envelope).is_empty());

        // Nest past the scan's depth budget. Mutating after redaction is the
        // point: this models an envelope the scan cannot traverse, not one
        // the redactor mishandled.
        let mut deep = Value::Null;
        for _ in 0..256 {
            deep = Value::Array(vec![deep]);
        }
        envelope.replay.expected_assertions.push(deep);

        let labels = residual_secret_labels_fail_closed(&redactor, &envelope);
        let expected = format!(
            "{}{RESIDUAL_SCAN_UNAVAILABLE_SITE}",
            trace_commons_protocol::trace_contribution::RESIDUAL_SECRET_AT_PREFIX
        );
        assert_eq!(
            labels.get(expected.as_str()),
            Some(&1),
            "an unscannable envelope must report an unchecked survivor; got {labels:?}"
        );
    }

    #[tokio::test]
    async fn an_envelope_serialises_key_sorted_whatever_map_backs_this_build() {
        use trace_commons_protocol::trace_contribution::{
            RawTraceContributionEvent, TraceContributionEventType,
        };

        let transcript = fixture_transcript();
        let cfg = test_config();
        let mut raw = build_raw_contribution(&transcript, &cfg, chrono::Utc::now());
        // Keys deliberately out of sorted order in the source. Under an
        // `IndexMap` an uncanonicalized payload would serialise in exactly
        // this order.
        raw.events.push(RawTraceContributionEvent {
            event_id: uuid::Uuid::new_v4(),
            parent_event_id: None,
            event_type: TraceContributionEventType::ToolResult,
            timestamp: chrono::Utc::now(),
            content: None,
            structured_payload: serde_json::json!({
                "zeta": 1,
                "alpha": {"omega": 2, "beta": 3},
                "middle": "value",
            }),
            tool_name: Some("Bash".to_string()),
            tool_call_id: Some("call-1".to_string()),
            latency_ms: None,
            token_counts: None,
            cost_usd: None,
            success: Some(true),
            failure_modes: Vec::new(),
        });

        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::new(vec![
                "/Users/testuser".into(),
            ])
            .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        let bytes = serde_json::to_vec(&envelope).unwrap();
        let rendered = String::from_utf8(bytes.clone()).unwrap();

        // Sorted order, not insertion order. Positions rather than a
        // substring match, so the assertion cannot pass on a payload that
        // happens to contain the keys somewhere.
        let alpha = rendered.find("\"alpha\"").expect("the payload survived");
        let middle = rendered.find("\"middle\"").expect("the payload survived");
        let zeta = rendered.find("\"zeta\"").expect("the payload survived");
        assert!(
            alpha < middle && middle < zeta,
            "an event payload reached the wire in insertion order, so a digest \
             over it depends on which map backs the build"
        );
        // And nested objects too -- `canonicalize` recurses, and a shallow
        // sort would leave a nested map ordering-dependent.
        let beta = rendered
            .find("\"beta\"")
            .expect("the nested payload survived");
        let omega = rendered
            .find("\"omega\"")
            .expect("the nested payload survived");
        assert!(beta < omega, "a nested payload was not canonicalized");

        // The digest the envelope carries is over those same canonical bytes,
        // so it is stable across backing maps. Re-serialising and re-hashing
        // must reproduce it exactly.
        let reparsed: trace_commons_protocol::trace_contribution::TraceContributionEnvelope =
            serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            serde_json::to_vec(&reparsed).unwrap(),
            bytes,
            "a round trip moved bytes, so the digest depends on the backing map"
        );
        assert_eq!(
            reparsed.privacy.redaction_hash,
            envelope.privacy.redaction_hash
        );
    }

    #[tokio::test]
    async fn envelope_has_schema_version_and_no_local_paths_or_secrets() {
        let t = fixture_transcript();
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        assert_eq!(
            raw.submission_id,
            crate::source::submission_id_for(&t.session_hash)
        );
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::new(vec![
                "/Users/testuser".into(),
            ])
            .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        assert_eq!(
            envelope.schema_version,
            trace_commons_protocol::trace_contribution::TRACE_CONTRIBUTION_SCHEMA_VERSION
        );
        let json = serde_json::to_string(&envelope).unwrap();
        // The fixture's fake secret value must not survive redaction.
        assert!(!json.contains("sk-fake-fixture-secret-1234"));
        // The full local path prefix must not survive.
        assert!(!json.contains("/Users/testuser"));
        // The project basename must not survive at all -- not in the clear,
        // and not as a digest either. See the note at the feature-flag block
        // for why a bare hash of a dictionary-shaped basename was rejected.
        assert!(!json.contains("myproj"));
        assert!(!json.contains(&session_hash("myproj".as_bytes())));
        // The agent tag does survive.
        assert!(json.contains("claude-code"));
    }

    #[test]
    fn canary_self_test_passes_for_deterministic_redactor() {
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        canary_self_test(&redactor).unwrap();
    }

    #[test]
    fn near_ai_filter_fails_closed_without_key() {
        let mut cfg = test_config();
        cfg.pii_filter = Some("near-ai".into());
        // No settings injected: must refuse, never downgrade to deterministic-only.
        assert!(build_redactor_with(&cfg, None, None).is_err());
    }

    #[test]
    fn unknown_pii_filter_fails_closed() {
        let mut cfg = test_config();
        cfg.pii_filter = Some("bogus".into());
        // DeterministicTraceRedactor is not Debug, so unwrap_err() is
        // unavailable; match on the error branch instead.
        match build_redactor_with(&cfg, None, None) {
            Err(err) => assert!(err.to_string().contains("unknown-pii-filter")),
            Ok(_) => panic!("unknown pii_filter must fail closed"),
        }
    }

    /// Spans back every occurrence of `bob@example.com` (used by
    /// `near_ai_filter_redacts_via_mock_endpoint`) and every canary email
    /// value the redaction canary sends through (used by the canary tests
    /// below), so one realistic classifier stub covers both. Matches how a
    /// real NEAR AI privacy-filter deployment would flag these categories --
    /// unlike a no-op filter, which always returns an empty span list.
    fn realistic_classify_router() -> axum::Router {
        use axum::{Json, Router, routing::post};
        Router::new().route(
            "/privacy/classify",
            post(|Json(req): Json<serde_json::Value>| async move {
                let input = req["input"].as_str().unwrap_or_default().to_string();
                let targets: &[(&str, &str)] = &[
                    ("bob@example.com", "private_email"),
                    ("trace-canary.person@example.invalid", "private_email"),
                    ("tc_canary_secret_0123456789abcdef", "secret"),
                    ("/tmp/trace_canary_private/path.txt", "private_url"),
                ];
                let mut spans = Vec::new();
                for (needle, category) in targets {
                    if let Some(start) = input.find(needle) {
                        spans.push(serde_json::json!({
                            "category": category,
                            "start": start,
                            "end": start + needle.len(),
                            "score": 0.99
                        }));
                    }
                }
                Json(serde_json::json!({"data": [{"spans": spans}]}))
            }),
        )
    }

    /// Always returns 200 with an empty span list, regardless of input --
    /// the shape a well-formed but non-functional ("no-op") privacy filter
    /// takes. It must not be able to pass the batch canary.
    fn noop_classify_router() -> axum::Router {
        use axum::{Json, Router, routing::post};
        Router::new().route(
            "/privacy/classify",
            post(|Json(_req): Json<serde_json::Value>| async move {
                Json(serde_json::json!({"data": [{"spans": []}]}))
            }),
        )
    }

    async fn spawn_near_ai_mock(router: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        base
    }

    fn near_ai_redactor(base_url: String) -> DeterministicTraceRedactor {
        let mut cfg = test_config();
        cfg.pii_filter = Some("near-ai".into());
        build_redactor_with(
            &cfg,
            Some("/Users/testuser/code/myproj"),
            Some(NearAiSettings {
                api_key: "test-key".into(),
                base_url: Some(base_url),
                model: None,
            }),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn near_ai_filter_redacts_via_mock_endpoint() {
        let base = spawn_near_ai_mock(realistic_classify_router()).await;

        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::User,
            timestamp: None,
            content: Some("please email bob@example.com about this".into()),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        });
        let cfg = test_config();
        let redactor = near_ai_redactor(base);
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(!json.contains("bob@example.com"));
    }

    #[tokio::test]
    async fn canary_self_test_async_passes_for_realistic_near_ai_filter() {
        let base = spawn_near_ai_mock(realistic_classify_router()).await;
        let redactor = near_ai_redactor(base);
        canary_self_test_async(&redactor).await.unwrap();
    }

    #[tokio::test]
    async fn canary_self_test_async_fails_closed_for_noop_near_ai_filter() {
        // A no-op filter (always empty spans) must not be able to pass the
        // batch canary just because the deterministic pass alone happens to
        // strip the values it is responsible for.
        let base = spawn_near_ai_mock(noop_classify_router()).await;
        let redactor = near_ai_redactor(base);
        let err = canary_self_test_async(&redactor).await.unwrap_err();
        assert!(err.to_string().contains("privacy-filter-canary-failed"));
    }

    #[tokio::test]
    async fn canary_self_test_async_is_noop_without_attached_filter() {
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        canary_self_test_async(&redactor).await.unwrap();
    }

    #[tokio::test]
    async fn granted_scopes_overwrite_consent_and_trace_card() {
        let t = fixture_transcript();
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let mut envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        let scopes = vec![
            ConsentScope::DebuggingEvaluation,
            ConsentScope::ModelTraining,
        ];
        let uses = vec![
            trace_commons_protocol::trace_contribution::TraceAllowedUse::Debugging,
            trace_commons_protocol::trace_contribution::TraceAllowedUse::ModelTraining,
        ];
        apply_granted_scopes(&mut envelope, &scopes, &uses);
        assert_eq!(envelope.consent.scopes, scopes);
        assert_eq!(envelope.trace_card.allowed_uses, uses);
        assert_eq!(
            envelope.trace_card.consent_scope,
            ConsentScope::DebuggingEvaluation
        );
    }

    #[tokio::test]
    async fn oversized_envelope_is_refused() {
        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some("x".repeat(MAX_ENVELOPE_BYTES + 1)),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        });
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        assert!(envelope_size_ok(&envelope).is_err());
    }

    /// A whole hackathon-scale coding session must submit. Sized from the
    /// real refusal that motivated the raise: a 42 MB session redacted to a
    /// 2.8 MB envelope and was refused against a 1.5 MB cap. 4 MB of event
    /// content stands in for that envelope -- comfortably over the old cap,
    /// comfortably under the new one.
    #[tokio::test]
    async fn a_hackathon_scale_session_envelope_is_accepted() {
        const HACKATHON_ENVELOPE_CONTENT_BYTES: usize = 4_000_000;
        // Compile-time: the fixture must sit under the cap, or this would
        // silently become a second copy of the refusal test.
        const _: () = assert!(HACKATHON_ENVELOPE_CONTENT_BYTES < MAX_ENVELOPE_BYTES);
        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some("x".repeat(HACKATHON_ENVELOPE_CONTENT_BYTES)),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        });
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        assert!(
            raw_contribution_size_ok(&raw).is_ok(),
            "pre-redaction guard must not refuse a hackathon-scale session"
        );
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        let size = envelope_size_ok(&envelope)
            .expect("a hackathon-scale session envelope must be accepted");
        assert!(size > 1_500_000, "fixture must exceed the pre-raise cap");
    }

    /// `envelope_size` now serializes into a counting `std::io::Write` sink
    /// rather than a `Vec<u8>`, so it never holds a second full copy of the
    /// envelope just to call `.len()` on it. The byte count must be
    /// unchanged: this pins it against `serde_json::to_vec(...).len()` --
    /// the ground truth the old implementation computed directly -- on a
    /// non-trivial envelope (real fixture content plus a multi-megabyte
    /// event, so a bug that dropped or double-counted a chunk would not
    /// hide inside a tiny fixture).
    #[tokio::test]
    async fn envelope_size_matches_a_plain_to_vec_len() {
        const CONTENT_BYTES: usize = 3_000_000;
        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some("y".repeat(CONTENT_BYTES)),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        });
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();

        let ground_truth = serde_json::to_vec(&envelope).unwrap().len();
        assert!(
            ground_truth > CONTENT_BYTES,
            "fixture must actually be non-trivial: {ground_truth}"
        );
        assert_eq!(envelope_size(&envelope).unwrap(), ground_truth);
    }

    #[test]
    fn oversized_raw_is_refused_before_redaction() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some("x".repeat(MAX_ENVELOPE_BYTES + 1)),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        });
        let big = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        assert!(raw_contribution_size_ok(&big).is_err());

        let small = build_raw_contribution(&fixture_transcript(), &cfg, chrono::Utc::now());
        assert!(raw_contribution_size_ok(&small).is_ok());
    }

    /// The declaration must describe the payload, not assert a constant.
    ///
    /// This is the case the hardcoded `true` got wrong in the direction that
    /// costs contributors traces: a transcript with no tool calls declared
    /// `tool_payloads_included: true`, which lands it at Medium residual risk
    /// and quarantines it on a default deployment -- for tool payloads it does
    /// not carry.
    #[test]
    fn transcript_without_tool_calls_does_not_declare_tool_payloads() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::User,
                timestamp: None,
                content: Some("what does this function do?".to_string()),
                structured: serde_json::Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: None,
                success: None,
            },
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::Assistant,
                timestamp: None,
                content: Some("it parses the config".to_string()),
                structured: serde_json::Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: None,
                success: None,
            },
        ];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert!(
            raw.consent.message_text_included,
            "the transcript carries message text, so it must be declared"
        );
        assert!(
            !raw.consent.tool_payloads_included,
            "no event carries a tool payload, so it must not be declared"
        );
    }

    /// A tool name is metadata about which tool ran, not the payload. Counting
    /// it would re-introduce the over-declaration this fixes, since the
    /// recorded-trace path keeps tool names for structure even when it strips
    /// payloads for privacy.
    #[test]
    fn a_bare_tool_name_is_not_a_tool_payload() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::ToolCall,
            timestamp: None,
            content: None,
            structured: serde_json::Value::Null,
            tool_name: Some("read_file".to_string()),
            token_counts: None,
            tool_call_id: None,
            success: None,
        }];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert!(
            !raw.consent.tool_payloads_included,
            "a tool name without a payload must not declare tool payloads"
        );
        assert!(
            !raw.consent.message_text_included,
            "a tool call is not message text"
        );
    }

    /// A tool call that does carry a payload must declare it -- the fix must
    /// not under-declare, which would misrepresent the envelope in the
    /// direction that matters for consent.
    #[test]
    fn tool_payloads_are_declared_when_present() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::ToolCall,
            timestamp: None,
            content: None,
            structured: serde_json::json!({"path": "src/main.rs"}),
            tool_name: Some("read_file".to_string()),
            token_counts: None,
            tool_call_id: None,
            success: None,
        }];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert!(raw.consent.tool_payloads_included);
    }

    /// A structure-only trace declares neither, which is what lets it stay out
    /// of quarantine on a default deployment.
    #[test]
    fn content_free_transcript_declares_neither() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some(String::new()),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        }];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert!(!raw.consent.message_text_included);
        assert!(!raw.consent.tool_payloads_included);
    }

    #[test]
    fn reasoning_events_map_to_the_reasoning_event_type() {
        let event = crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::Reasoning,
            timestamp: None,
            content: Some("weighing two approaches".to_string()),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
            tool_call_id: None,
            success: None,
        };
        let raw = super::raw_event_for(&event, chrono::Utc::now());
        assert_eq!(
            raw.event_type,
            trace_commons_protocol::trace_contribution::TraceContributionEventType::Reasoning
        );
        assert_eq!(raw.content.as_deref(), Some("weighing two approaches"));
    }

    #[test]
    fn trajectory_session_builds_an_envelope_with_reasoning_and_no_cwd_leak() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.json");
        let mut f = std::fs::File::create(&path).unwrap();
        // `cwd` is nested one level below the "secretproj" marker. Neither
        // the full cwd path (and therefore the "secretproj" marker) nor the
        // project basename ("workdir") may appear in the serialized output.
        // The cwd crosses only as `cwd_hash`; the basename does not cross at
        // all, in any form.
        f.write_all(
            br#"[
              {"role":"meta","source":"openhands","cwd":"/home/dev/secretproj/workdir","model":"gpt-5"},
              {"role":"user","content":"fix the bug","timestamp":"2026-07-10T12:00:00Z"},
              {"role":"reasoning","content":"the guard clause is inverted","timestamp":"2026-07-10T12:00:01Z"},
              {"role":"assistant","content":"Fixed it.","timestamp":"2026-07-10T12:00:02Z"}
            ]"#,
        )
        .unwrap();

        let src = crate::source::trajectory::TrajectorySource::new(path);
        let r = &crate::source::TraceSource::discover(&src).unwrap()[0];
        let t = crate::source::TraceSource::load(&src, r).unwrap();

        assert_eq!(t.source.as_ref(), "openhands");

        let cfg = test_config();
        let raw = super::build_raw_contribution(&t, &cfg, chrono::Utc::now());
        assert!(
            raw.events.iter().any(|e| {
                e.event_type
                == trace_commons_protocol::trace_contribution::TraceContributionEventType::Reasoning
            }),
            "reasoning must survive into the raw contribution"
        );

        let serialized = serde_json::to_string(&raw).unwrap();
        assert!(
            !serialized.contains("secretproj"),
            "cwd must never be serialized"
        );
        assert!(
            !serialized.contains("workdir"),
            "project basename must never be serialized in the clear"
        );
        assert!(
            !serialized.contains(&session_hash("workdir".as_bytes())),
            "the basename must not cross as a digest either -- an unsalted \
             hash of a dictionary-shaped name is reversible and is a stable \
             cross-contributor identifier"
        );
        assert!(
            !raw.ironclaw.feature_flags.contains_key("project"),
            "the cleartext project flag must be gone"
        );
        assert!(
            !raw.ironclaw.feature_flags.contains_key("project_hash"),
            "and it must not have been replaced by a hashed one"
        );
    }

    /// A result names the call it answers.
    ///
    /// Array order was the only sequence signal an envelope carried, and a
    /// consumer cannot verify order. The adapters read these ids all along.
    #[test]
    fn a_result_names_the_call_it_answers() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolCall,
                timestamp: None,
                content: None,
                structured: serde_json::json!({"file_path": "cfg.toml"}),
                tool_name: Some("Read".to_string()),
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: None,
            },
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolResult,
                timestamp: None,
                content: Some("port = 8080".to_string()),
                structured: serde_json::Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: Some(true),
            },
        ];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert_eq!(
            raw.events[1].parent_event_id,
            Some(raw.events[0].event_id),
            "the result must point at the call"
        );
        assert_eq!(raw.events[1].success, Some(true));
    }

    /// An unpaired result gets no parent rather than the nearest call.
    #[test]
    fn a_result_with_no_matching_call_is_left_unparented() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolCall,
                timestamp: None,
                content: None,
                structured: serde_json::json!({"file_path": "cfg.toml"}),
                tool_name: Some("Read".to_string()),
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: None,
            },
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolResult,
                timestamp: None,
                content: Some("port = 8080".to_string()),
                structured: serde_json::Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: Some("tu_other".to_string()),
                success: None,
            },
        ];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert_eq!(
            raw.events[1].parent_event_id, None,
            "guessing a parent is worse than admitting there is none"
        );
    }

    /// The arguments an adapter captured have to be recognisable as arguments.
    ///
    /// The adapters hand over the tool's argument object itself, so the payload
    /// was a bare blob like `{"file_path": ...}`. Replay sufficiency looks for
    /// arguments under a known key, so a fully-populated call read as carrying
    /// no arguments at all.
    #[test]
    fn tool_call_arguments_are_named_in_the_payload() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::ToolCall,
            timestamp: None,
            content: None,
            structured: serde_json::json!({"file_path": "cfg.toml"}),
            tool_name: Some("Read".to_string()),
            token_counts: None,
            tool_call_id: None,
            success: None,
        }];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert_eq!(
            raw.events[0].structured_payload,
            serde_json::json!({"arguments": {"file_path": "cfg.toml"}}),
        );
    }

    /// A call with no recorded arguments stays null rather than claiming an
    /// empty payload exists.
    #[test]
    fn a_call_with_no_arguments_names_nothing() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::ToolCall,
            timestamp: None,
            content: None,
            structured: serde_json::Value::Null,
            tool_name: Some("Read".to_string()),
            token_counts: None,
            tool_call_id: None,
            success: None,
        }];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert!(raw.events[0].structured_payload.is_null());
        assert!(
            !raw.consent.tool_payloads_included,
            "an absent payload must not be declared as one"
        );
    }

    /// This path declared every transcript unreplayable, and the scorecard
    /// reads that flag as authoritative -- so the one capture path carrying a
    /// prompt, arguments and results scored zero replayability while the
    /// web-history path carrying tool names alone scored full marks.
    #[test]
    fn a_transcript_with_events_is_not_declared_unreplayable() {
        let cfg = test_config();
        let t = fixture_transcript();

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert!(!raw.events.is_empty(), "the fixture has events");
        assert!(
            raw.replay.replayable,
            "a local transcript is not KNOWN to be unreplayable; how much of a              replay survived is for the sufficiency measure to say"
        );
        assert_eq!(
            raw.replay.required_tools,
            vec!["Read".to_string()],
            "the tools a replay would have to stand up, derived from the calls"
        );
    }

    /// An empty transcript is genuinely unreplayable, and says so.
    #[test]
    fn an_empty_transcript_is_still_declared_unreplayable() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = Vec::new();

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert!(!raw.replay.replayable);
        assert!(raw.replay.required_tools.is_empty());
    }

    /// End to end, the finding in issue #298 the other way round: a locally
    /// captured transcript carries a prompt, the arguments of its call and
    /// the result that came back, so it must score as replayable. It scored
    /// exactly zero before -- on all three counts -- while a web-history
    /// envelope carrying tool names and nothing else scored 1.0.
    #[tokio::test]
    async fn a_local_transcript_now_scores_as_replayable() {
        use trace_commons_protocol::trace_contribution::compute_value_scorecard;

        let cfg = test_config();
        let t = fixture_transcript();
        let redactor = build_deterministic_preview_redactor(t.cwd.as_deref());
        let envelope = redact_to_envelope(
            &redactor,
            build_raw_contribution(&t, &cfg, chrono::Utc::now()),
        )
        .await
        .expect("redaction succeeds");

        let scored = compute_value_scorecard(&envelope);
        assert_eq!(
            scored.replayability, 1.0,
            "a prompt, arguments on every call and a result per call is \
             everything replay needs: {:?}",
            scored.explanation
        );
        assert!(
            scored.coverage_bonus > 0.0,
            "a transcript that calls tools covers tools"
        );
    }

    /// A correction reaches the outcome AND the declaration, which is the
    /// pair that matters: `correction_included` is what makes unredacted
    /// contributor prose visible to residual-risk derivation and to the PII
    /// backstop hold. An envelope carrying the text with the flag false is
    /// exactly the gap S5 exists to avoid reopening.
    #[test]
    fn a_correction_reaches_the_outcome_and_the_declaration() {
        let cfg = test_config();
        let t = fixture_transcript();

        let corrected = build_raw_contribution_with_correction(
            &t,
            &cfg,
            chrono::Utc::now(),
            Some(ContributorVerdict::Failed),
            Some("it edited /Users/x/proj/config.toml instead of the staging one"),
        );
        assert_eq!(
            corrected.outcome.human_correction.as_deref(),
            Some("it edited /Users/x/proj/config.toml instead of the staging one"),
            "stored as written -- the path is the point"
        );
        assert!(corrected.consent.correction_included);
    }

    /// Whitespace is not a correction, and the declaration must not claim it
    /// is: normalised in the builder so every caller gets the same answer.
    #[test]
    fn a_blank_correction_declares_nothing() {
        let cfg = test_config();
        let t = fixture_transcript();

        for blank in [None, Some(""), Some("   \n\t ")] {
            let raw = build_raw_contribution_with_correction(
                &t,
                &cfg,
                chrono::Utc::now(),
                Some(ContributorVerdict::Failed),
                blank,
            );
            assert_eq!(raw.outcome.human_correction, None, "blank: {blank:?}");
            assert!(!raw.consent.correction_included, "blank: {blank:?}");
        }
    }

    /// The credential refusal keeps its own label all the way out of
    /// `redact_to_envelope`, because it is the one pipeline refusal a
    /// contributor can act on -- remove it, and rotate it. Everything else
    /// stays behind the generic label.
    #[tokio::test]
    async fn a_credential_in_a_correction_refuses_under_its_own_label() {
        let cfg = test_config();
        let t = fixture_transcript();
        let redactor = build_deterministic_preview_redactor(t.cwd.as_deref());

        let raw = build_raw_contribution_with_correction(
            &t,
            &cfg,
            chrono::Utc::now(),
            Some(ContributorVerdict::Failed),
            Some("it used the key AKIAIOSFODNN7EXAMPLE and blew up"),
        );
        let err = redact_to_envelope(&redactor, raw)
            .await
            .expect_err("a credential in a correction refuses the submission");
        assert_eq!(err.to_string(), REASON_CORRECTION_CREDENTIAL);
    }

    /// `task_success` was `Unknown` on every envelope this client has ever
    /// sent, because `OutcomeMetadata::default()` was written unconditionally.
    /// Nothing in a transcript answers the question -- an agent that stops has
    /// not thereby succeeded -- so the contributor is asked.
    #[test]
    fn a_verdict_reaches_the_outcome() {
        use trace_commons_protocol::trace_contribution::{TaskSuccess, UserFeedback};

        let cfg = test_config();
        let t = fixture_transcript();

        let worked = build_raw_contribution_with_verdict(
            &t,
            &cfg,
            chrono::Utc::now(),
            Some(ContributorVerdict::Worked),
        );
        assert_eq!(worked.outcome.task_success, TaskSuccess::Success);

        let partly = build_raw_contribution_with_verdict(
            &t,
            &cfg,
            chrono::Utc::now(),
            Some(ContributorVerdict::Partly),
        );
        assert_eq!(partly.outcome.task_success, TaskSuccess::Partial);

        let failed = build_raw_contribution_with_verdict(
            &t,
            &cfg,
            chrono::Utc::now(),
            Some(ContributorVerdict::Failed),
        );
        assert_eq!(failed.outcome.task_success, TaskSuccess::Failure);

        // A verdict answers "did the task complete", and nothing else. It
        // must not also assert satisfaction: the contributor was asked one
        // question, so the envelope carries one fact. `Partly` is what makes
        // this concrete -- it has no honest thumb.
        for outcome in [&worked.outcome, &partly.outcome, &failed.outcome] {
            assert_eq!(
                outcome.user_feedback,
                UserFeedback::None,
                "a verdict must not assert a satisfaction signal"
            );
        }
    }

    /// No verdict means unknown, not success. Silence is not a claim.
    #[test]
    fn no_verdict_leaves_the_outcome_unknown() {
        use trace_commons_protocol::trace_contribution::{TaskSuccess, UserFeedback};

        let cfg = test_config();
        let t = fixture_transcript();
        let raw = build_raw_contribution_with_verdict(&t, &cfg, chrono::Utc::now(), None);

        assert_eq!(raw.outcome.task_success, TaskSuccess::Unknown);
        assert_eq!(raw.outcome.user_feedback, UserFeedback::None);
        assert_eq!(
            raw.outcome,
            trace_commons_protocol::trace_contribution::OutcomeMetadata::default(),
            "the no-verdict envelope must be byte-identical to the old behaviour"
        );
    }

    /// A verdict is a judgement, not text. It must not move either content
    /// declaration, or a two-button answer would start gating consent.
    #[test]
    fn a_verdict_declares_no_content() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events.clear();

        let raw = build_raw_contribution_with_verdict(
            &t,
            &cfg,
            chrono::Utc::now(),
            Some(ContributorVerdict::Failed),
        );

        assert!(!raw.consent.message_text_included);
        assert!(!raw.consent.tool_payloads_included);
    }

    /// A failed verdict is what makes a trace score as difficult, which is
    /// the whole reason to collect it: `difficulty` is 0.65 for a failure
    /// against 0.35 otherwise.
    #[tokio::test]
    async fn a_failed_verdict_scores_as_difficult() {
        use trace_commons_protocol::trace_contribution::compute_value_scorecard;

        let cfg = test_config();
        let t = fixture_transcript();
        let redactor = build_deterministic_preview_redactor(t.cwd.as_deref());

        let unknown = redact_to_envelope(
            &redactor,
            build_raw_contribution_with_verdict(&t, &cfg, chrono::Utc::now(), None),
        )
        .await
        .expect("redaction succeeds");
        let failed = redact_to_envelope(
            &redactor,
            build_raw_contribution_with_verdict(
                &t,
                &cfg,
                chrono::Utc::now(),
                Some(ContributorVerdict::Failed),
            ),
        )
        .await
        .expect("redaction succeeds");

        assert!(
            compute_value_scorecard(&failed).difficulty
                > compute_value_scorecard(&unknown).difficulty,
            "a known failure is worth more than an unknown outcome"
        );
    }

    #[test]
    fn verdict_names_are_the_wire_names() {
        assert_eq!(
            ContributorVerdict::parse("worked"),
            Some(ContributorVerdict::Worked)
        );
        assert_eq!(
            ContributorVerdict::parse("partly"),
            Some(ContributorVerdict::Partly)
        );
        assert_eq!(
            ContributorVerdict::parse("failed"),
            Some(ContributorVerdict::Failed)
        );
        // A typo must not silently become "unknown": the caller refuses.
        assert_eq!(ContributorVerdict::parse("Worked"), None);
        assert_eq!(ContributorVerdict::parse("success"), None);
        assert_eq!(ContributorVerdict::parse("partial"), None);
    }

    /// The client half must apply the same marker rule as the server, or the
    /// contributor is penalised for declaring honestly: the server corrects
    /// flags upward only, so a client that under-declares gets corrected and
    /// a client that over-declares is believed.
    #[test]
    fn a_marker_payload_is_not_declared_as_a_tool_payload() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::ToolCall,
            timestamp: None,
            content: None,
            structured: serde_json::json!({"has_result": true, "has_error": false}),
            tool_name: Some("read_file".to_string()),
            token_counts: None,
            tool_call_id: None,
            success: None,
        }];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert!(
            !raw.consent.tool_payloads_included,
            "a payload of booleans carries nothing to declare"
        );
    }

    /// A payload can carry its content in the KEY. Values were the only
    /// thing the declaration inspected, so `{"someone@example.com": true}`
    /// declared nothing, took the Low-risk acceptance path, and skipped the
    /// server-side PII backstop -- which does classify keys.
    #[test]
    fn a_content_bearing_key_is_declared_as_a_tool_payload() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![crate::source::SessionEvent {
            served_by: None,
            kind: crate::source::SessionEventKind::ToolCall,
            timestamp: None,
            content: None,
            structured: serde_json::json!({"someone@example.com": true}),
            tool_name: Some("read_file".to_string()),
            token_counts: None,
            tool_call_id: None,
            success: None,
        }];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert!(
            raw.consent.tool_payloads_included,
            "a key is as free-form as the string beside it, and must be \
             declared"
        );
    }

    /// Both halves agree on the same envelope. Pinned because the server
    /// derivation is the one that can overrule this one.
    ///
    /// `fixture_transcript()` is a real captured claude-code session and
    /// carries no `RoutingDecision` events on its own, so a
    /// [`crate::routing::RoutedExchange`] is attached to `t.routing` and the
    /// envelope is built by the real path (`build_raw_contribution`, which
    /// now calls `raw_events_for_with_routing`) rather than by mutating
    /// `raw.events` after the builder has already run. That is the gap this
    /// test used to leave open: it pinned the derivation logic against a
    /// hypothetical event shape, not against anything the real builder
    /// emits.
    #[tokio::test]
    async fn the_two_content_derivations_agree() {
        use trace_commons_protocol::trace_contribution::derive_envelope_content_presence;

        let cfg = test_config();
        let mut t = fixture_transcript();
        t.routing = vec![sample_routed_exchange()];
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let declared = declared_content_presence(&raw.events);

        // The comparison above is worth nothing if `build_raw_contribution`
        // never actually writes the derived presence into the consent block
        // it returns -- a hardcoded `false` there would leave `declared`
        // alone and this test would stay green. Assert the builder's output
        // matches what it should have derived.
        assert_eq!(
            (
                raw.consent.message_text_included,
                raw.consent.tool_payloads_included,
                raw.consent.routing_metadata_included,
            ),
            (
                declared.message_text,
                declared.tool_payloads,
                declared.routing_metadata,
            ),
            "build_raw_contribution must write the derived presence into the \
             consent block, not a hardcoded value"
        );

        let redactor = build_deterministic_preview_redactor(t.cwd.as_deref());
        let envelope = redact_to_envelope(&redactor, raw)
            .await
            .expect("redaction succeeds");
        let presence = derive_envelope_content_presence(&envelope);

        assert!(
            declared.routing_metadata && presence.routing_metadata,
            "the fixture must actually exercise routing_metadata on both \
             sides, or this test proves nothing about it"
        );
        assert_eq!(
            (
                declared.message_text,
                declared.tool_payloads,
                declared.routing_metadata,
            ),
            (
                presence.message_text,
                presence.tool_payloads,
                presence.routing_metadata,
            ),
            "the client declaration and the server derivation must not disagree"
        );
    }

    /// A session event carries what the step would have cost at the
    /// provider's list price, worked out here by hand from the published
    /// rates rather than by calling the function under test.
    ///
    /// The fixture's first assistant record is served by `claude-fable-5`
    /// and reports 100 input, 25 output, 1000 cache-read, and 500
    /// cache-creation tokens split 200 (5m) / 300 (1h):
    ///
    /// ```text
    ///   100 x $10        = $0.00100
    ///    25 x $50        = $0.00125
    ///  1000 x $1         = $0.00100
    ///   200 x $12.50     = $0.00250
    ///   300 x $20        = $0.00600   per million tokens
    ///                       -------
    ///                       $0.01175
    /// ```
    #[test]
    fn a_priced_step_carries_what_it_would_have_cost() {
        let t = fixture_transcript();
        let events = raw_events_for(&t.events, chrono::Utc::now());
        assert_eq!(
            events[2].cost_usd,
            Some(<trace_commons_protocol::trace_contribution::Decimal as std::str::FromStr>::from_str(
                "0.01175"
            )
            .unwrap())
        );
    }

    /// The step after it reports tokens but no cache report, so it cannot be
    /// priced. The field is absent, not zero -- a zero here would read as a
    /// step that cost nothing, and would silently understate any total built
    /// by summing these.
    #[test]
    fn an_unpriceable_step_carries_no_cost_rather_than_a_zero() {
        let t = fixture_transcript();
        let events = raw_events_for(&t.events, chrono::Utc::now());
        assert!(events[5].token_counts.is_some());
        assert_eq!(events[5].cost_usd, None);
        assert_ne!(
            events[5].cost_usd,
            Some(trace_commons_protocol::trace_contribution::Decimal::ZERO)
        );
    }

    /// A step whose model is not in the price table is not priced at some
    /// other model's rate. Nothing else about the event changes.
    #[test]
    fn a_step_served_by_an_unlisted_model_is_not_priced() {
        let event = crate::source::SessionEvent {
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some("hi".to_string()),
            structured: Value::Null,
            tool_name: None,
            token_counts: Some((1_000_000, 1_000_000)),
            tool_call_id: None,
            success: None,
            served_by: Some(crate::source::ServedBy {
                model: "some-other-vendors-model".to_string(),
                cache_read_tokens: 0,
                cache_write_5m_tokens: 0,
                cache_write_1h_tokens: 0,
            }),
        };
        let listed = crate::source::SessionEvent {
            served_by: Some(crate::source::ServedBy {
                model: "claude-opus-5".to_string(),
                cache_read_tokens: 0,
                cache_write_5m_tokens: 0,
                cache_write_1h_tokens: 0,
            }),
            ..event.clone()
        };
        let now = chrono::Utc::now();
        // The listed model prices at $5 + $25 per million, which is what
        // makes the `None` above a refusal rather than a dead code path.
        assert_eq!(
            raw_events_for(&[listed], now)[0].cost_usd,
            Some(<trace_commons_protocol::trace_contribution::Decimal as std::str::FromStr>::from_str("30").unwrap())
        );
        assert_eq!(raw_events_for(&[event], now)[0].cost_usd, None);
    }

    /// Token counts alone are not enough to price a step, and a source that
    /// reports them without a model or a cache report -- every adapter but
    /// Claude Code today -- leaves the cost absent rather than guessing.
    #[test]
    fn token_counts_without_a_usage_report_are_not_priced() {
        let event = crate::source::SessionEvent {
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some("hi".to_string()),
            structured: Value::Null,
            tool_name: None,
            token_counts: Some((1_000_000, 1_000_000)),
            tool_call_id: None,
            success: None,
            served_by: None,
        };
        let mapped = raw_events_for(&[event], chrono::Utc::now());
        assert_eq!(
            mapped[0].token_counts.as_ref().map(|t| t.input_tokens),
            Some(1_000_000)
        );
        assert_eq!(mapped[0].cost_usd, None);
    }

    #[test]
    fn a_routing_row_becomes_an_event_carrying_its_numbers_in_typed_fields() {
        // The numbers go in the typed fields the event already has. Only the
        // labels with no typed home go in `structured_payload`, and that is
        // what the routing_metadata presence class exists to declare.
        let events =
            raw_events_for_with_routing(&[], &[sample_routed_exchange()], chrono::Utc::now());
        let routing: Vec<_> = events
            .iter()
            .filter(|e| e.event_type == TraceContributionEventType::RoutingDecision)
            .collect();
        assert_eq!(routing.len(), 1);
        let e = routing[0];
        assert_eq!(e.latency_ms, Some(1200));
        assert!(e.cost_usd.is_some());
        assert_eq!(e.token_counts.as_ref().map(|t| t.input_tokens), Some(1000));
        assert!(
            e.content.is_none(),
            "the overlay is numbers and labels, never text -- Some(\"\") would \
             also satisfy the old is_empty() check and is not the same claim"
        );
        assert_eq!(e.structured_payload["backend"], "claude-sub");
    }

    #[test]
    fn routing_events_do_not_declare_a_tool_payload() {
        // The whole point of task 1, asserted where the events are actually
        // built.
        let events =
            raw_events_for_with_routing(&[], &[sample_routed_exchange()], chrono::Utc::now());
        let presence = declared_content_presence(&events);
        assert!(presence.routing_metadata);
        assert!(!presence.tool_payloads);
    }

    /// A comparable projection of a mapped event: everything but `event_id`,
    /// with `parent_event_id` rewritten as the parent's position in the
    /// vector instead of its (randomly generated) uuid. `event_id` is
    /// `Uuid::new_v4()` at every call, so two otherwise-identical runs of
    /// `raw_events_for`/`raw_events_for_with_routing` can never compare equal
    /// on the raw structs -- this is what lets the comparison look at
    /// everything that actually describes the event.
    ///
    /// This hand-lists every field of `RawTraceContributionEvent` except
    /// `event_id`, so it is complete as of this writing -- but it is a list,
    /// not a projection derived from the struct. A field added to
    /// `RawTraceContributionEvent` later does not fail to compile here; it
    /// just never enters the comparison, and tests built on this function
    /// silently stop covering it. Update this tuple (and its type) when the
    /// struct gains a field.
    fn normalize_for_comparison(
        events: &[RawTraceContributionEvent],
    ) -> Vec<(
        Option<usize>,
        TraceContributionEventType,
        chrono::DateTime<chrono::Utc>,
        Option<String>,
        serde_json::Value,
        Option<String>,
        Option<String>,
        Option<u64>,
        Option<trace_commons_protocol::trace_contribution::TokenCounts>,
        Option<trace_commons_protocol::trace_contribution::Decimal>,
        Option<bool>,
        Vec<trace_commons_protocol::trace_contribution::TraceFailureMode>,
    )> {
        let index_of: std::collections::HashMap<uuid::Uuid, usize> = events
            .iter()
            .enumerate()
            .map(|(i, e)| (e.event_id, i))
            .collect();
        events
            .iter()
            .map(|e| {
                (
                    e.parent_event_id.and_then(|id| index_of.get(&id).copied()),
                    e.event_type,
                    e.timestamp,
                    e.content.clone(),
                    e.structured_payload.clone(),
                    e.tool_name.clone(),
                    e.tool_call_id.clone(),
                    e.latency_ms,
                    e.token_counts.clone(),
                    e.cost_usd,
                    e.success,
                    e.failure_modes.clone(),
                )
            })
            .collect()
    }

    #[test]
    fn a_session_with_no_routing_rows_produces_exactly_what_it_did_before() {
        // A real fixture's events, not `&[]` -- an empty slice in, an empty
        // slice out proves nothing about whether routing rows change
        // anything, since there is nothing there for them to change.
        let events = fixture_transcript().events;
        assert!(
            !events.is_empty(),
            "fixture sanity check: the comparison below needs real events"
        );
        let now = chrono::Utc::now();
        let before = raw_events_for(&events, now);
        let after = raw_events_for_with_routing(&events, &[], now);
        assert_eq!(
            normalize_for_comparison(&before),
            normalize_for_comparison(&after),
            "an empty routing slice must produce exactly what raw_events_for \
             produced on its own"
        );
    }

    /// A result that names its call should also name its tool.
    ///
    /// The transcript records the tool name once, on the call; a result record
    /// carries only the id. Every adapter therefore left `tool_name` empty on
    /// results, which left them with no `tool_category` and made a consumer
    /// walk `parent_event_id` just to learn which tool it was holding.
    #[test]
    fn a_paired_result_inherits_the_tool_that_ran() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolCall,
                timestamp: None,
                content: None,
                structured: serde_json::json!({"file_path": "cfg.toml"}),
                tool_name: Some("Read".to_string()),
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: None,
            },
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolResult,
                timestamp: None,
                content: Some("port = 8080".to_string()),
                structured: serde_json::Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: Some(true),
            },
        ];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert_eq!(raw.events[1].tool_name.as_deref(), Some("Read"));
    }

    /// An unpaired result stays anonymous rather than borrowing the nearest
    /// call's name.
    #[test]
    fn an_unpaired_result_names_no_tool() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolCall,
                timestamp: None,
                content: None,
                structured: serde_json::json!({"file_path": "cfg.toml"}),
                tool_name: Some("Read".to_string()),
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: None,
            },
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolResult,
                timestamp: None,
                content: Some("port = 8080".to_string()),
                structured: serde_json::Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: Some("tu_other".to_string()),
                success: Some(true),
            },
        ];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert_eq!(raw.events[1].tool_name, None);
        assert_eq!(
            raw.events[0].success, None,
            "an unpaired verdict must not be attributed to a call"
        );
    }

    /// "Which tool failed" is a question about the call, so the result's
    /// verdict travels back to it. The capture path already sets `success` on
    /// both halves; this makes the local path agree.
    #[test]
    fn a_failed_result_marks_the_call_that_failed() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolCall,
                timestamp: None,
                content: None,
                structured: serde_json::json!({"file_path": "cfg.toml"}),
                tool_name: Some("Read".to_string()),
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: None,
            },
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolResult,
                timestamp: None,
                content: Some("permission denied".to_string()),
                structured: serde_json::Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: Some(false),
            },
        ];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert_eq!(raw.events[0].success, Some(false));
        assert_eq!(raw.events[1].success, Some(false));
    }

    /// A verdict the call already carries is never overwritten by its result.
    #[test]
    fn a_calls_own_verdict_wins() {
        let cfg = test_config();
        let mut t = fixture_transcript();
        t.events = vec![
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolCall,
                timestamp: None,
                content: None,
                structured: serde_json::json!({"file_path": "cfg.toml"}),
                tool_name: Some("Read".to_string()),
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: Some(false),
            },
            crate::source::SessionEvent {
                served_by: None,
                kind: crate::source::SessionEventKind::ToolResult,
                timestamp: None,
                content: Some("ok".to_string()),
                structured: serde_json::Value::Null,
                tool_name: None,
                token_counts: None,
                tool_call_id: Some("tu_1".to_string()),
                success: Some(true),
            },
        ];

        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());

        assert_eq!(raw.events[0].success, Some(false));
    }
}
