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
//! [`ENFORCED`] is `false`, and is switched on only when the conditions in the
//! spec's "When enforcement is switched on" all hold: #1020's account
//! admission check is enforced on every ingest replica, Z2 (#1005) is in
//! place, and a held-session health condition reaches every shell. Until then
//! the gate is evaluated and reported, and approvals go ahead exactly as
//! before.
//!
//! # What it checks
//!
//! The trust model (rev 8 of the spec) decides what gates arming and what
//! only decides the wording. Per contributor:
//!
//! - **R7** -- a data-use scope has been chosen.
//! - **R3** -- the tenant is not one whose uploads need per-session admission
//!   evidence (#706), which an armed folder never prepares. Kept until #1020's
//!   account check is enforced on the server, and withdrawn then: sending
//!   earlier would put sessions through the witness and classifier only for
//!   ingest to refuse them. The client cannot see that switch, so removing
//!   this check is a code change made when it happens, not a runtime test.
//!
//! **R1 is not a gate.** Full trust lets a folder with no verified model pass
//! send after deterministic redaction. What R1 still decides is what the
//! contributor is told: see [`disclosure`], which may claim a model scrubbed
//! a session only where a certified full pipeline ran.
//!
//! R4 (provenance) is a property of what is claimed, R5 (the hold) is the
//! queue's `held_for_review`, and R6 (the void rule) is a property of the
//! grant rather than of a send; none is evaluated here.

use crate::config::ContributorConfig;

/// Whether an unmet requirement stops an unattended approval. See the module
/// docs for why this is off.
///
/// Among the conditions for switching this on: a label-only health
/// condition raised while `TickReport::gate_blocked` is non-zero, with copy
/// in every shell. The counter and the log line exist today; without the
/// health label, an enforced gate would hold armed work with nothing in the
/// app to say so.
pub const ENFORCED: bool = false;

/// A requirement the spec names, by its number there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Requirement {
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

/// What an armed folder may be told happens to its sessions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disclosure {
    /// A certified full pipeline ran on every automatic session, so the
    /// `AUTO_SCRUB_*` wording, which says a model removes what it
    /// recognises, is true.
    ModelScrubbed,
    /// Only the fixed patterns can be relied on. The wording for this is not
    /// yet written (the spec's Open list).
    PatternsOnly,
}

/// R1, as the choice of disclosure: "trust relaxes what may be sent, never
/// what may be said".
///
/// Always [`Disclosure::PatternsOnly`] for now. A configured witness or
/// filter is not enough -- R1 needs the certified pipeline version checked
/// per session against the published allowlist (K6), which waits on #1005.
/// Configuration presence never earns the model-scrub wording.
pub fn disclosure(_cfg: Option<&ContributorConfig>) -> Disclosure {
    Disclosure::PatternsOnly
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

    /// R1 is not a gate: an invited contributor with a scope chosen passes
    /// with or without a prose pass. What they are told is decided apart.
    #[test]
    fn an_invitee_with_a_scope_passes_whatever_their_prose_pass() {
        for (witness, pii) in [(false, None), (true, None), (false, Some("near-ai"))] {
            let v = evaluate(
                Some(&cfg("tenant-1", &["debugging_evaluation"], witness, pii)),
                true,
            );
            assert!(v.unmet.is_empty(), "{:?}", reasons(&v));
            assert!(!v.blocks());
        }
    }

    /// Configuration never earns the model-scrub wording; only a per-session
    /// check of the certified pipeline would, and there is none yet.
    #[test]
    fn no_configuration_earns_the_model_scrub_disclosure() {
        assert_eq!(disclosure(None), Disclosure::PatternsOnly);
        for (witness, pii) in [(false, None), (true, None), (true, Some("near-ai"))] {
            assert_eq!(
                disclosure(Some(&cfg(
                    "tenant-1",
                    &["debugging_evaluation"],
                    witness,
                    pii
                ))),
                Disclosure::PatternsOnly
            );
        }
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
    /// it on is a change someone makes on purpose, with the spec's switch-on
    /// conditions beside it.
    #[test]
    // A constant assertion on purpose: its only job is to make turning
    // enforcement on a change that fails a test, so it is made deliberately.
    #[allow(clippy::assertions_on_constants)]
    fn the_gate_ships_unenforced() {
        assert!(!ENFORCED);
    }
}
