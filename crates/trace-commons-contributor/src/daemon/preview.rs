//! Real preview: redact one session without uploading, and report exactly
//! what would leave the machine.
//!
//! Before this module existed, the IPC `"preview"` arm reported
//! `entry.size_bytes` -- the raw session file's size on disk. Redaction
//! shrinks (and reshapes) the payload, so that number overstated what
//! actually gets sent and was the one figure backing a contributor's consent
//! decision. `build_preview` runs the *same* redaction path `submit_one`
//! uses (`build_redactor_with` + `build_raw_contribution` +
//! `redact_to_envelope`).
//!
//! Running the same *code* is not the same as producing the same
//! *artifact*, and this module used to claim it was ("so preview and upload
//! can never disagree"). It could not have been: preview kept nothing of
//! the envelope it built, and the uploader's guard re-hashed the raw
//! transcript only. Redaction-service output, privacy-filter
//! configuration, daemon settings, contributor config, and timestamps can
//! all move between the preview and the send while the raw session hash is
//! unchanged, and the guard stayed silent through every one of them --
//! because it verified the input, not the artifact.
//!
//! Two things close that, and they close different halves of it.
//!
//! [`input_fingerprint`] covers the *inputs*: the whole contributor config,
//! the privacy-filter backend and model, and the redaction ruleset version.
//! It is recorded on every approval, including approvals given without a
//! preview (armed auto-upload), and re-derived immediately before every
//! upload. A mismatch revokes the approval and re-offers the entry, the
//! same way a changed session hash does.
//!
//! The *artifact* is not re-derived and compared at all. It is stored.
//! [`build_preview`] hands back the envelope it built, the caller persists
//! it (`daemon::approved_envelope`), and the upload sends exactly those
//! bytes. An earlier round compared a digest instead, and that comparison
//! made the whole feature unusable with `pii_filter = "near-ai"`: an
//! LLM-backed filter does not return identical spans for identical text,
//! so every previewed entry was refused and re-offered forever. See
//! `daemon::approved_envelope` for the full account.
//!
//! **Preview is the one interface in this crate that deliberately carries
//! trace content** (`PreviewSummary::opening_prompt` over the socket, the
//! redacted body over the C ABI, and now the stored envelope at rest under
//! the 0700 state directory). A contributor cannot consent to sending
//! something they cannot see, so the exemption exists -- bounded to
//! post-redaction content, only for an entry the caller already asked
//! about, deleted as soon as the entry is resolved, and never onward into a
//! log line, an audit entry, a history record, notification text, or a
//! receipt. Everywhere else in this crate the no-trace-content rule is
//! absolute.

use anyhow::Result;
use chrono::Utc;
use sha2::{Digest, Sha256};

use crate::config::{ConfigStore, ContributorConfig};
use crate::envelope::{
    NearAiSettings, build_raw_contribution, build_redactor_with, envelope_size, redact_to_envelope,
};
use crate::source::{SessionRef, TraceSource};
use trace_commons_protocol::trace_contribution::{
    TraceContributionEnvelope, TraceContributionEventType,
};

/// Envelope fields that differ between two builds of the same content and
/// therefore cannot participate in an approval digest: freshly generated
/// ids and wall-clock stamps. Everything else -- every redacted event body,
/// every redaction count, the PII labels, the consent scopes, the policy
/// version, the trace card, the contributor metadata -- is in.
///
/// `timestamp` is on the list because an event with no timestamp of its own
/// inherits `now`. Event *content* is already covered by both this digest
/// and the raw session hash, so nothing about what is being sent escapes
/// scrutiny by stripping it; leaving it in would just make the digest
/// non-deterministic for sources that omit per-event times, and a digest
/// that spuriously mismatches would re-offer entries forever.
///
/// `redaction_hash` is the protocol crate's own SHA-256 over the events and
/// the redaction counts -- including each event's freshly generated
/// `event_id` -- so it moves whenever those do, for reasons that have
/// nothing to do with what was redacted. Both of its real inputs are
/// already in this digest directly, so stripping it loses no coverage.
///
/// A field wrongly *left out* of this list weakens the guard silently, so
/// `every_stripped_field_is_actually_volatile` pins each entry: a field
/// that turns out to be stable across two builds does not belong here.
const VOLATILE_ENVELOPE_FIELDS: &[&str] = &[
    "trace_id",
    "event_id",
    "revocation_handle",
    "created_at",
    "timestamp",
    "redaction_hash",
];

/// A content-addressed digest of the redacted envelope a contributor was
/// shown, stable across two builds of the same envelope from the same
/// inputs *when the redaction step is deterministic*.
///
/// Nothing compares this against a rebuild any more, and nothing may start
/// doing so: with an LLM-backed privacy filter a rebuild legitimately
/// differs, and a comparison makes previewed entries permanently
/// unuploadable. Its two remaining jobs are both local:
///
/// * it is the marker that says "this entry was previewed, and the bytes
///   that were shown are on disk" (`QueueEntry::previewed_envelope_digest`);
/// * it is a self-consistency check on this crate's own storage -- the
///   uploader re-digests the file it read back and refuses if it is not the
///   one that was pinned, which catches a corrupted or crossed-over file
///   rather than a jittery filter.
///
/// It is also returned over IPC so an app can confirm the entry it approves
/// is the one it displayed.
pub fn envelope_digest(envelope: &TraceContributionEnvelope) -> Result<String> {
    let mut value = serde_json::to_value(envelope)
        .map_err(|_| anyhow::anyhow!("envelope-digest-serialize-failed"))?;
    strip_volatile(&mut value);
    // `serde_json::Value`'s map is a `BTreeMap` under this crate's feature
    // set, so serialization is key-ordered and the digest is stable.
    let canonical = serde_json::to_vec(&value)
        .map_err(|_| anyhow::anyhow!("envelope-digest-serialize-failed"))?;
    Ok(format!("sha256:{:x}", Sha256::digest(&canonical)))
}

fn strip_volatile(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for k in VOLATILE_ENVELOPE_FIELDS {
                map.remove(*k);
            }
            for v in map.values_mut() {
                strip_volatile(v);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items.iter_mut() {
                strip_volatile(v);
            }
        }
        _ => {}
    }
}

/// The redaction ruleset this build implements.
///
/// The deterministic redactor's patterns, the categories it accepts from a
/// privacy-filter backend, and the shape of the envelope it produces are
/// all compiled in. An in-place upgrade of the daemon binary between an
/// approval and the upload therefore changes what gets redacted with no
/// config change to show for it, and a fingerprint built from config alone
/// cannot see that. Previewed entries are covered anyway (their bytes are
/// stored), but an **armed auto-upload entry is covered by nothing else**,
/// so the version goes into the fingerprint.
///
/// Bump this whenever the redaction rules, the accepted filter categories,
/// or the envelope layout change in a way that alters output for unchanged
/// input. The crate version is hashed in alongside it, so an ordinary
/// release bump moves the fingerprint even if nobody remembers to touch
/// this constant; this exists for the case where the rules move without a
/// version bump.
pub const REDACTION_RULESET_VERSION: &str = "1";

/// A fingerprint of everything outside the session file that determines the
/// envelope: the whole contributor config (consent scopes, PII filter
/// selection, tenant/instance/subject/device identity, audience, endpoints,
/// host allowlist), the presence and identity of the NEAR AI
/// privacy-filter backend, and the redaction ruleset/build this daemon is
/// running.
///
/// Cheap -- no redaction pass, no network -- so it is recorded on every
/// approval and re-derived before every upload, including for entries that
/// were never previewed and for armed auto-upload.
///
/// The NEAR AI **API key is deliberately never hashed in**, and that is a
/// decision rather than an omission: rotating a credential does not change
/// what the filter does to a trace, so hashing it in would revoke every
/// standing approval on a routine rotation for no consent-relevant reason
/// -- and hashing a live secret into a value this crate writes to disk is
/// not something it does. The base URL and model *are* hashed in, because
/// both change what comes back.
///
/// One envelope-determining input is still outside this: `SubmitOptions`.
/// See the note at the daemon's construction site in `daemon::drain_approved`.
pub fn input_fingerprint(cfg: &ContributorConfig, near_ai: Option<&NearAiSettings>) -> String {
    let mut h = Sha256::new();
    // Serialized rather than field-by-field so a field added to
    // `ContributorConfig` later is covered without anyone remembering to
    // come back here.
    h.update(serde_json::to_vec(cfg).unwrap_or_default().as_slice());
    h.update(b"\x00redactor\x00");
    h.update(REDACTION_RULESET_VERSION.as_bytes());
    h.update(b"\x00");
    h.update(env!("CARGO_PKG_VERSION").as_bytes());
    h.update(b"\x00near_ai\x00");
    match near_ai {
        None => h.update(b"absent"),
        Some(s) => {
            h.update(b"present\x00");
            h.update(s.base_url.as_deref().unwrap_or("").as_bytes());
            h.update(b"\x00");
            h.update(s.model.as_deref().unwrap_or("").as_bytes());
        }
    }
    format!("sha256:{:x}", h.finalize())
}

/// The fixed reason label an entry is re-offered under when it was pinned
/// to a previewed envelope but those bytes are missing or unusable at
/// upload time.
///
/// Fail-closed: the daemon does **not** quietly rebuild the envelope in
/// that case. Rebuilding is exactly what stored bytes exist to avoid, and a
/// silent rebuild would send something the contributor never saw.
pub const REASON_APPROVED_ENVELOPE_UNAVAILABLE: &str = "approved-envelope-unavailable";

/// The fixed reason label an entry is re-offered under when an
/// envelope-determining input changed between approval and upload.
pub const REASON_INPUTS_CHANGED: &str = "approval-inputs-changed";

/// What preview reports to the contributor before they consent to upload.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PreviewSummary {
    pub would_send_bytes: usize,
    pub raw_session_bytes: u64,
    pub event_count: usize,
    pub opening_prompt: String,
    pub redactions: std::collections::BTreeMap<String, u32>,
    pub pii_labels_present: Vec<String>,
    /// The consent scopes this device **requests**, taken from the local
    /// config, which is what `build_raw_contribution` stamps onto the
    /// envelope here.
    ///
    /// These are not necessarily the scopes an upload ends up carrying. An
    /// actual submission mints an upload claim first, and
    /// `submit::stamp_granted_scopes` then overwrites the envelope with the
    /// **granted** set the issuer echoed back -- falling back to the
    /// requested set only when the issuer is old enough not to echo one.
    /// Preview cannot show the granted set without minting a claim, which
    /// it deliberately does not do (preview is a local operation and must
    /// work offline).
    ///
    /// So this field is an upper bound on what an upload will claim, never
    /// an under-statement: the issuer can only narrow the request, never
    /// widen it. A consumer rendering this to a contributor should say
    /// "requested", not "will be sent as". Separately, an entry's approval
    /// is pinned to exactly this requested set
    /// (`QueueEntry::approved_scopes`), so a local widening between preview
    /// and upload revokes the approval rather than riding along with it.
    pub consent_scopes: Vec<String>,
    pub residual_risk: String,
    /// Digest of the redacted envelope this summary describes, and a
    /// fingerprint of the inputs that produced it. Hashes, never content.
    ///
    /// The digest is recorded on the queue entry alongside the envelope
    /// itself, and identifies the file the upload will send. It is not
    /// compared against a rebuild -- see [`envelope_digest`] and the module
    /// doc for why a comparison cannot work here.
    pub envelope_digest: String,
    pub input_fingerprint: String,
}

/// Redact one session without uploading and describe exactly what would be
/// sent. Same redaction path the uploader uses.
///
/// Returns the summary, the redacted body for display, **and the envelope
/// itself**. The envelope is the artifact the upload will send verbatim:
/// the caller persists it (`daemon::approved_envelope`) so nothing has to
/// rebuild it, which is what makes preview and upload agree under a
/// non-deterministic privacy filter.
///
/// `store` is accepted for signature parity with the other entry points that
/// build a redactor/envelope (`submit_one`); preview does not itself read or
/// write through it -- everything it needs comes from `cfg` and the already
/// -resolved `source`/`session_ref`.
pub async fn build_preview(
    _store: &ConfigStore,
    cfg: &ContributorConfig,
    near_ai: Option<NearAiSettings>,
    source: &dyn TraceSource,
    session_ref: &SessionRef,
) -> Result<(PreviewSummary, String, TraceContributionEnvelope)> {
    let transcript = source.load(session_ref)?;
    let raw_session_bytes = session_ref.size_bytes;

    let fingerprint = input_fingerprint(cfg, near_ai.as_ref());
    let redactor = build_redactor_with(cfg, transcript.cwd.as_deref(), near_ai)
        .map_err(|_| anyhow::anyhow!("pii-filter-unavailable"))?;
    let raw = build_raw_contribution(&transcript, cfg, Utc::now());
    let envelope = redact_to_envelope(&redactor, raw).await?;
    // Digested here, at exactly the point `submit_loaded` takes over the
    // envelope it is about to send: after redaction, before the granted
    // scopes the issuer echoes back are stamped on. This identifies the
    // stored artifact; it is not re-derived from a second build anywhere.
    let digest = envelope_digest(&envelope)?;
    let would_send_bytes = envelope_size(&envelope)?;

    let event_count = envelope.events.len();
    let opening_prompt = envelope
        .events
        .iter()
        .find(|e| e.event_type == TraceContributionEventType::UserMessage)
        .and_then(|e| e.redacted_content.clone())
        .unwrap_or_default();
    let opening_prompt = truncate_chars(&opening_prompt, 200);

    let redactions = envelope.privacy.redaction_counts.clone();
    let pii_labels_present = envelope.privacy.pii_labels_present.clone();
    let consent_scopes = envelope.consent.scopes.iter().map(wire_name).collect();
    let residual_risk = serde_json::to_value(envelope.privacy.residual_pii_risk)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "pattern-based".to_string());

    let body = serde_json::to_string_pretty(&envelope.events)
        .map_err(|_| anyhow::anyhow!("preview-body-serialize-failed"))?;

    Ok((
        PreviewSummary {
            would_send_bytes,
            raw_session_bytes,
            event_count,
            opening_prompt,
            redactions,
            pii_labels_present,
            consent_scopes,
            residual_risk,
            envelope_digest: digest,
            input_fingerprint: fingerprint,
        },
        body,
        envelope,
    ))
}

/// Serde's wire name for a `Serialize` value that serializes to a bare
/// string (every enum used here does, via `#[serde(rename_all =
/// "snake_case")]`).
fn wire_name<T: serde::Serialize>(value: T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

/// Truncate to at most `max_chars` characters, always on a char boundary.
fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::claude_code::ClaudeCodeSource;

    fn sample_cfg(store: &ConfigStore) -> ContributorConfig {
        let device = crate::identity::DeviceIdentity::load_or_generate(store).unwrap();
        ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        }
    }

    /// A session with a planted secret, so redaction has something to do.
    fn fixture_session() -> (tempfile::TempDir, ClaudeCodeSource, SessionRef) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let project = root.join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("11111111-1111-1111-1111-111111111111.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\
             \"content\":\"deploy with key sk-fake-fixture-secret-1234\"},\
             \"cwd\":\"/Users/testuser/code/myproj\",\
             \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
             \"sessionId\":\"11111111-1111-1111-1111-111111111111\",\
             \"uuid\":\"a1\"}\n",
        )
        .unwrap();
        let src = ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        (dir, src, r)
    }

    #[tokio::test]
    async fn preview_reports_the_redacted_size_not_the_raw_size() {
        // The defect this task exists to fix: the old code returned the raw
        // file size. Measured, the redacted envelope is substantially LARGER
        // than the session file it came from -- envelope metadata dominates --
        // so the old number understated what actually leaves the machine.
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body, _envelope) =
            build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert!(summary.raw_session_bytes > 0);
        assert!(summary.would_send_bytes > 0);
        assert_ne!(
            summary.would_send_bytes as u64, summary.raw_session_bytes,
            "a redacted envelope is not the same size as the raw session file"
        );
    }

    #[tokio::test]
    async fn preview_reports_what_redaction_actually_removed() {
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body, _envelope) =
            build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        let total: u32 = summary.redactions.values().sum();
        assert!(
            total > 0,
            "planted secret should appear in the counts: {:?}",
            summary.redactions
        );
    }

    #[tokio::test]
    async fn preview_body_does_not_contain_the_planted_secret() {
        // The whole point of showing a body is that it is the redacted one.
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (_summary, body, _envelope) =
            build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert!(
            !body.contains("sk-fake-fixture-secret-1234"),
            "secret survived into the preview body"
        );
    }

    #[tokio::test]
    async fn preview_carries_an_opening_prompt_and_an_event_count() {
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body, _envelope) =
            build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert_eq!(summary.event_count, 1);
        assert!(!summary.opening_prompt.is_empty());
        assert!(
            !summary
                .opening_prompt
                .contains("sk-fake-fixture-secret-1234"),
            "the opening prompt must be the redacted one"
        );
    }

    #[tokio::test]
    async fn preview_opening_prompt_is_truncated() {
        // 200 chars, so a huge first message cannot dominate a queue row.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let project = root.join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project).unwrap();
        let long = "x".repeat(500);
        std::fs::write(
            project.join("22222222-2222-2222-2222-222222222222.jsonl"),
            format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{long}\"}},\
                 \"cwd\":\"/Users/testuser/code/myproj\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"22222222-2222-2222-2222-222222222222\",\"uuid\":\"a1\"}}\n"
            ),
        )
        .unwrap();
        let src = ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body, _envelope) =
            build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert!(summary.opening_prompt.chars().count() <= 200);
    }

    #[tokio::test]
    async fn two_builds_of_the_same_envelope_digest_identically() {
        // The guard is only usable if the digest is stable: a
        // non-deterministic one would re-offer every entry forever.
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (a, _, _) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        let (b, _, _) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert_eq!(a.envelope_digest, b.envelope_digest);
        assert!(a.envelope_digest.starts_with("sha256:"));
    }

    #[tokio::test]
    async fn every_stripped_field_is_actually_volatile() {
        // A field on the strip list that is in fact stable is coverage
        // quietly given away: it would be excluded from the guard for no
        // reason. Build the same envelope twice and require that each
        // stripped field genuinely differs somewhere in the document (or,
        // for a field the schema does not currently emit, is absent from
        // both).
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let mut t = src.load(&r).unwrap();
        // A source that records no per-event time is what makes
        // `timestamp` volatile: those events inherit `now`. The claude-code
        // fixture does record times, so clear them here -- otherwise this
        // test would "prove" `timestamp` is stable and demand it be removed
        // from the list, which would make the digest non-deterministic for
        // every source that omits them.
        for e in t.events.iter_mut() {
            e.timestamp = None;
        }
        let redactor = build_redactor_with(&cfg, t.cwd.as_deref(), None).unwrap();
        let a = redact_to_envelope(&redactor, build_raw_contribution(&t, &cfg, Utc::now()))
            .await
            .unwrap();
        let b = redact_to_envelope(&redactor, build_raw_contribution(&t, &cfg, Utc::now()))
            .await
            .unwrap();
        let va = serde_json::to_value(&a).unwrap();
        let vb = serde_json::to_value(&b).unwrap();
        for field in VOLATILE_ENVELOPE_FIELDS {
            let ka = collect_field(&va, field);
            let kb = collect_field(&vb, field);
            assert!(
                ka.is_empty() || ka != kb,
                "{field} is stable across two builds and does not belong on \
                 the volatile list"
            );
        }
    }

    /// Every value stored under `field`, anywhere in the document.
    fn collect_field(value: &serde_json::Value, field: &str) -> Vec<serde_json::Value> {
        let mut out = Vec::new();
        fn walk(v: &serde_json::Value, field: &str, out: &mut Vec<serde_json::Value>) {
            match v {
                serde_json::Value::Object(map) => {
                    if let Some(found) = map.get(field) {
                        out.push(found.clone());
                    }
                    for child in map.values() {
                        walk(child, field, out);
                    }
                }
                serde_json::Value::Array(items) => {
                    for child in items {
                        walk(child, field, out);
                    }
                }
                _ => {}
            }
        }
        walk(value, field, &mut out);
        out
    }

    #[test]
    fn the_input_fingerprint_moves_with_every_envelope_determining_input() {
        let (_sd, store) = crate::config::tests_support::temp_store();
        let base = sample_cfg(&store);
        let baseline = input_fingerprint(&base, None);
        assert_eq!(baseline, input_fingerprint(&base, None), "must be stable");

        for mutate in [
            (|c: &mut ContributorConfig| c.tenant_id = "other-tenant".into())
                as fn(&mut ContributorConfig),
            |c: &mut ContributorConfig| c.consent_scopes = vec!["model_training".into()],
            |c: &mut ContributorConfig| c.pii_filter = Some("near-ai".into()),
            |c: &mut ContributorConfig| c.user_subject = "bob".into(),
            |c: &mut ContributorConfig| c.ingest_url = "http://elsewhere.invalid".into(),
            |c: &mut ContributorConfig| c.device_key_id = "sha256:zz".into(),
        ] {
            let mut cfg = base.clone();
            mutate(&mut cfg);
            assert_ne!(
                baseline,
                input_fingerprint(&cfg, None),
                "an envelope-determining config change must move the fingerprint"
            );
        }

        // Attaching a privacy-filter backend changes it; so does pointing
        // that backend at a different service or model.
        let near = NearAiSettings {
            api_key: "secret-key".into(),
            base_url: Some("https://filter.invalid".into()),
            model: Some("m1".into()),
        };
        assert_ne!(baseline, input_fingerprint(&base, Some(&near)));
        let other_model = NearAiSettings {
            model: Some("m2".into()),
            ..near.clone()
        };
        assert_ne!(
            input_fingerprint(&base, Some(&near)),
            input_fingerprint(&base, Some(&other_model))
        );
        // But rotating the credential does not: it changes nothing about
        // what the filter does, and a secret is not hashed into a value
        // that lands on disk.
        let rotated = NearAiSettings {
            api_key: "a-different-secret".into(),
            ..near.clone()
        };
        assert_eq!(
            input_fingerprint(&base, Some(&near)),
            input_fingerprint(&base, Some(&rotated))
        );
    }

    #[tokio::test]
    async fn the_digest_moves_when_the_envelope_does() {
        // Same session bytes, different envelope-determining config: the
        // digest must not be blind to it.
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (a, _, _) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();

        let mut widened = cfg.clone();
        widened.consent_scopes = vec!["model_training".into()];
        let (b, _, _) = build_preview(&store, &widened, None, &src, &r)
            .await
            .unwrap();
        assert_ne!(a.envelope_digest, b.envelope_digest);
        assert_ne!(a.input_fingerprint, b.input_fingerprint);
    }
}
