//! Whether a session may be approved on the contributor's behalf.
//!
//! The one check that stands between an armed folder and an unattended
//! approval, from the connect-and-forget design
//! (`docs/superpowers/specs/2026-09-23-connect-and-forget-consent-design.md`,
//! "What automatic contribution requires"). It sits at the decision to
//! approve on someone's behalf, which comes before either upload branch --
//! the witness path and the local-redaction path -- so no route to an
//! unattended send goes round it. Three revisions of that spec put a gate on
//! a branch instead, and each missed a route.
//!
//! # Not enforced yet
//!
//! [`ENFORCED`] is `false`. The first requirement, a verified redaction
//! pipeline, cannot be met by anyone until the client checks the certified
//! pipeline version (K6), which waits on the shared allowlist (Z1) and the v2
//! witness certificate (#1005). Enforcing today would stop every armed folder
//! uploading. So for now the gate is evaluated and reported, and approvals go
//! ahead exactly as before; it is switched on together with moving
//! already-armed folders back to ask-first (K5), so that nobody's folder
//! stops silently.
//!
//! # What it checks
//!
//! Only what the client can know before the send, per contributor:
//!
//! - **R1** -- a prose pass is configured, and it is one that can be verified
//!   per session. Configuration alone never satisfies R1; until K6 exists
//!   nothing does, and the reason says which of the two is missing.
//! - **R3** -- the tenant is not one whose uploads need per-session admission
//!   evidence (#706), which automatic contribution removes the step for.
//! - **R7** -- a data-use scope has been chosen.
//!
//! R4 (provenance) is a property of what is claimed, R5 (the hold) is the
//! queue's `held_for_review`, and R6 (the void rule) is a property of the
//! grant rather than of a send; none is evaluated here.

use crate::config::ContributorConfig;

/// Whether an unmet requirement stops an unattended approval. See the module
/// docs for why this is off.
///
/// Before this is switched on, besides K5 and something able to pass R1:
/// a label-only health condition raised while `TickReport::gate_blocked` is
/// non-zero, with copy in every shell. The counter and the log line exist
/// today; without the health label, an enforced gate would hold armed work
/// with nothing in the app to say so.
pub const ENFORCED: bool = false;

/// A requirement the spec names, by its number there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
    R1Pipeline,
    R3Admission,
    R7Scope,
}

/// One requirement not met, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unmet {
    pub requirement: Requirement,
    pub reason: &'static str,
}

/// The gate's answer for a contributor at the start of a watcher pass.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GateVerdict {
    pub unmet: Vec<Unmet>,
    pub enforced: bool,
}

impl GateVerdict {
    /// Whether an unattended approval must not happen.
    pub fn blocks(&self) -> bool {
        self.enforced && !self.unmet.is_empty()
    }

    /// Whether an approval that goes ahead is one the gate would have
    /// refused. What report-only mode counts.
    pub fn would_refuse(&self) -> bool {
        !self.unmet.is_empty()
    }
}

pub const REASON_NO_PROSE_PASS: &str = "no-prose-pass-configured";
pub const REASON_PIPELINE_UNVERIFIED: &str = "pipeline-not-verified-per-session";
pub const REASON_ADMISSION_PER_SESSION: &str = "admission-evidence-is-per-session";
pub const REASON_NO_SCOPE: &str = "no-data-use-scope-chosen";

/// Whether this tenant's uploads need receipt-bound admission evidence: the
/// server's own rule, from the protocol crate, so the two cannot drift. A
/// tenant outside it is on the invite-free path and never passes through
/// admission.
fn needs_admission_evidence(tenant_id: &str) -> bool {
    trace_commons_protocol::admission::is_anchored_tenant(tenant_id)
}

/// Evaluate the gate for a contributor's current configuration.
pub fn evaluate(cfg: Option<&ContributorConfig>, enforced: bool) -> GateVerdict {
    let mut unmet = Vec::new();
    let Some(cfg) = cfg else {
        // Not enrolled: nothing uploads at all, and the uploader says so. The
        // gate answers for the one requirement that is plainly absent.
        unmet.push(Unmet {
            requirement: Requirement::R7Scope,
            reason: REASON_NO_SCOPE,
        });
        return GateVerdict { unmet, enforced };
    };

    let prose_pass = cfg.witness.is_some() || cfg.pii_filter.is_some();
    unmet.push(Unmet {
        requirement: Requirement::R1Pipeline,
        reason: if prose_pass {
            REASON_PIPELINE_UNVERIFIED
        } else {
            REASON_NO_PROSE_PASS
        },
    });

    if needs_admission_evidence(&cfg.tenant_id) {
        unmet.push(Unmet {
            requirement: Requirement::R3Admission,
            reason: REASON_ADMISSION_PER_SESSION,
        });
    }

    if cfg.consent_scopes.is_empty() {
        unmet.push(Unmet {
            requirement: Requirement::R7Scope,
            reason: REASON_NO_SCOPE,
        });
    }

    GateVerdict { unmet, enforced }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(tenant: &str, scopes: &[&str], witness: bool, pii: Option<&str>) -> ContributorConfig {
        ContributorConfig {
            inference_receipt_endpoint: None,
            inference_receipt_check_attestation: false,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: "https://issuer.invalid".to_string(),
            ingest_url: "https://ingest.invalid".to_string(),
            audience: "aud".to_string(),
            tenant_id: tenant.to_string(),
            instance_id: "instance-1".to_string(),
            user_subject: "alice".to_string(),
            device_key_id: "sha256:aa".to_string(),
            consent_scopes: scopes.iter().map(|s| s.to_string()).collect(),
            pii_filter: pii.map(str::to_string),
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness: witness.then(|| {
                serde_json::from_value(serde_json::json!({
                    "url": "https://witness.example",
                    "signing_address": format!("0x{}", "ab".repeat(20)),
                    "expected_measurements": [format!("mrtd={}", "ab".repeat(48))],
                }))
                .unwrap()
            }),
        }
    }

    fn reasons(v: &GateVerdict) -> Vec<&'static str> {
        v.unmet.iter().map(|u| u.reason).collect()
    }

    /// Nobody passes today, which is the spec's own conclusion. Configuration
    /// alone never satisfies R1.
    #[test]
    fn no_contributor_passes_until_the_pipeline_is_verified_per_session() {
        let with_witness = evaluate(
            Some(&cfg("tenant-1", &["debugging_evaluation"], true, None)),
            true,
        );
        assert_eq!(reasons(&with_witness), [REASON_PIPELINE_UNVERIFIED]);
        let bare = evaluate(
            Some(&cfg("tenant-1", &["debugging_evaluation"], false, None)),
            true,
        );
        assert_eq!(reasons(&bare), [REASON_NO_PROSE_PASS]);
    }

    /// An invited tenant is outside admission; a wallet or NEAR AI-login
    /// tenant is inside it.
    #[test]
    fn admission_applies_to_the_near_namespaces_only() {
        let near = format!("near-{}", "a".repeat(64));
        let nearai = format!("nearai-{}", "b".repeat(64));
        for tenant in [near.as_str(), nearai.as_str()] {
            let v = evaluate(
                Some(&cfg(tenant, &["debugging_evaluation"], true, None)),
                true,
            );
            assert!(
                reasons(&v).contains(&REASON_ADMISSION_PER_SESSION),
                "{tenant}"
            );
        }
        // Neither an invited tenant nor a namespace prefix without a hash
        // after it: the server never admission-checks either.
        for tenant in ["tenant-1", "nearai-abc", "near-abc"] {
            let v = evaluate(
                Some(&cfg(tenant, &["debugging_evaluation"], true, None)),
                true,
            );
            assert!(
                !reasons(&v).contains(&REASON_ADMISSION_PER_SESSION),
                "{tenant}"
            );
        }
    }

    #[test]
    fn a_contributor_with_no_scope_chosen_does_not_pass() {
        let v = evaluate(Some(&cfg("tenant-1", &[], true, None)), true);
        assert!(reasons(&v).contains(&REASON_NO_SCOPE));
    }

    /// Report-only: the same unmet requirements, and nothing blocked.
    #[test]
    fn an_unenforced_gate_reports_and_does_not_block() {
        let v = evaluate(Some(&cfg("tenant-1", &[], false, None)), false);
        assert!(v.would_refuse());
        assert!(!v.blocks());
        let enforced = evaluate(Some(&cfg("tenant-1", &[], false, None)), true);
        assert!(enforced.blocks());
    }

    /// The shipped setting. A test, not only a constant, so that switching
    /// it on is a change someone makes on purpose with K5 beside it.
    #[test]
    // A constant assertion on purpose: its only job is to make turning
    // enforcement on a change that fails a test, so it is made deliberately.
    #[allow(clippy::assertions_on_constants)]
    fn the_gate_ships_unenforced() {
        assert!(!ENFORCED);
    }
}
