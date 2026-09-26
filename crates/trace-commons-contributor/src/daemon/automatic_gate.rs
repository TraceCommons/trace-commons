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
//! [`ENFORCED`] is `false`, and is switched on only when the switch-on
//! conditions in rev 8 of the spec ("When enforcement is switched on") hold:
//! among them, Z2 (#1005) is in place, a held-session health condition
//! reaches every shell, and the client reads the contribution-status answer
//! that feeds R3. Until then the gate is evaluated and reported, and
//! approvals go ahead exactly as before.
//!
//! # What it checks
//!
//! The trust model (rev 8, #1025) decides what gates arming and what only
//! decides the wording. Per contributor:
//!
//! - **R7** -- a data-use scope has been chosen.
//! - **R3** -- the tenant is not one whose uploads need per-session admission
//!   evidence (#706), which an armed folder never prepares. **A runtime check,
//!   never removed in code.** It is built to stop applying only while the
//!   configured ingest says it admits by account instead
//!   ([`AccountAdmission`]), which #1020 reports on
//!   `/v1/account/contribution-status` (see [`AccountAdmission::from_status`]).
//!   **This build has only the decision half.** The status fetch that feeds
//!   it lands in a follow-up; until then the watcher calls [`evaluate`],
//!   which passes [`AccountAdmission::NotAdvertised`], so R3 always applies.
//!   Account admission is per ingest replica and off whenever its environment
//!   variable is missing, so it can revert on any redeploy; a client that had
//!   dropped R3 for good would then send every armed session through the
//!   witness and classifier only for ingest to refuse it. Anything short of
//!   an affirmative answer -- no answer, a non-200 or unparseable status
//!   response, an older ingest, a different commons, `legacy_evidence`, a
//!   spent allowance -- keeps R3.
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
/// condition, set and cleared only from full passes (where
/// `TickReport::gate_blocked` is `Some`), with copy in every shell. An
/// event-driven pass sees only changed paths, so it can neither raise nor
/// clear it. The count and the log line exist today; without the health
/// label, an enforced gate would hold armed work with nothing in the app to
/// say so.
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
pub const REASON_ACCOUNT_ALLOWANCE_SPENT: &str = "account-allowance-spent";

/// Whether this tenant's uploads need receipt-bound admission evidence: the
/// server's own rule, from the protocol crate, so the two cannot drift. A
/// tenant outside it is on the invite-free path and never passes through
/// admission.
fn needs_admission_evidence(tenant_id: &str) -> bool {
    trace_commons_protocol::admission::is_anchored_tenant(tenant_id)
}

/// What the configured ingest last said about how it admits contributions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AccountAdmission {
    /// It admits by account (#1020), so no per-session evidence is needed.
    Advertised,
    /// No affirmative answer. The default, and the only safe one: R3 applies.
    #[default]
    NotAdvertised,
    /// It admits by account, but this account's allowance for the period is
    /// spent (`bounded` with `ready: false`), so every send would be refused
    /// until it resets. R3 still holds -- the gate fails closed exactly as
    /// for [`Self::NotAdvertised`] -- but under its own reason, because no
    /// per-session evidence would help: the remedy is to wait.
    AllowanceSpent,
}

impl AccountAdmission {
    /// From what `/v1/account/contribution-status` returns (#1020): its
    /// `authority` label and its `ready` flag.
    ///
    /// Account admission only when the authority is `bounded` or `invited`
    /// **and** `ready` is true. A `bounded` account whose allowance is spent
    /// answers `ready: false`, and every send it made would be refused, so it
    /// keeps R3, as [`Self::AllowanceSpent`]. `legacy_evidence`, an
    /// `invited` answer that is not ready (which #1020 never gives), and any
    /// label this build does not know keep R3 as [`Self::NotAdvertised`].
    ///
    /// Only for a 200 whose body parses. The endpoint's own failures are not
    /// answers: #1020 returns 403 (`account_identity_unlinked`, or a missing
    /// DB mirror) rather than `legacy_evidence` when it cannot resolve the
    /// account, so the fetcher maps any non-200, transport error or parse
    /// failure to `NotAdvertised` without calling this.
    ///
    /// One answer comes from one ingest replica, so it is provisional: the
    /// caller re-reads it on every full pass and drops back to
    /// `NotAdvertised` on any admission refusal from ingest. Nothing here
    /// persists a lift.
    pub fn from_status(authority: &str, ready: bool) -> Self {
        match authority {
            "bounded" | "invited" if ready => Self::Advertised,
            "bounded" => Self::AllowanceSpent,
            _ => Self::NotAdvertised,
        }
    }
}

/// Evaluate the gate for a contributor's current configuration, with no
/// answer from ingest about account admission, so R3 applies to the anchored
/// namespaces.
pub fn evaluate(cfg: Option<&ContributorConfig>, enforced: bool) -> GateVerdict {
    evaluate_with(cfg, enforced, AccountAdmission::NotAdvertised)
}

/// Evaluate the gate, with what the configured ingest last said about
/// account admission.
pub fn evaluate_with(
    cfg: Option<&ContributorConfig>,
    enforced: bool,
    account_admission: AccountAdmission,
) -> GateVerdict {
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
        let reason = match account_admission {
            AccountAdmission::Advertised => None,
            AccountAdmission::NotAdvertised => Some(REASON_ADMISSION_PER_SESSION),
            AccountAdmission::AllowanceSpent => Some(REASON_ACCOUNT_ALLOWANCE_SPENT),
        };
        if let Some(reason) = reason {
            unmet.push(Unmet {
                requirement: Requirement::R3Admission,
                reason,
            });
        }
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

    /// R3 is lifted only by an affirmative answer from ingest, and returns
    /// the moment that answer does: a replica with #1020 switched off says
    /// `legacy_evidence`, and an unknown label is not a yes.
    #[test]
    fn r3_follows_what_ingest_says_at_runtime() {
        let nearai = format!("nearai-{}", "b".repeat(64));
        let cfg = cfg(&nearai, &["debugging_evaluation"], false, None);
        let with = |a| reasons(&evaluate_with(Some(&cfg), true, a));
        assert!(with(AccountAdmission::NotAdvertised).contains(&REASON_ADMISSION_PER_SESSION));
        assert!(with(AccountAdmission::Advertised).is_empty());
        assert_eq!(
            reasons(&evaluate(Some(&cfg), true)),
            with(AccountAdmission::NotAdvertised)
        );

        use AccountAdmission::{Advertised, AllowanceSpent, NotAdvertised};
        for (label, ready, expected) in [
            ("bounded", true, Advertised),
            ("invited", true, Advertised),
            // An exhausted allowance: ingest would refuse every send.
            ("bounded", false, AllowanceSpent),
            ("invited", false, NotAdvertised),
            ("legacy_evidence", true, NotAdvertised),
            ("", true, NotAdvertised),
            ("Bounded", true, NotAdvertised),
            ("something-newer", true, NotAdvertised),
        ] {
            assert_eq!(
                AccountAdmission::from_status(label, ready),
                expected,
                "{label} ready={ready}"
            );
        }
        assert_eq!(AccountAdmission::default(), AccountAdmission::NotAdvertised);
    }

    /// A spent allowance still holds, but says so: it is not missing
    /// per-session evidence, and nothing the contributor could prepare
    /// would help.
    #[test]
    fn a_spent_allowance_holds_under_its_own_reason() {
        assert_eq!(
            AccountAdmission::from_status("bounded", false),
            AccountAdmission::AllowanceSpent
        );
        let nearai = format!("nearai-{}", "b".repeat(64));
        let anchored = cfg(&nearai, &["debugging_evaluation"], false, None);
        let v = evaluate_with(Some(&anchored), true, AccountAdmission::AllowanceSpent);
        assert_eq!(
            v.unmet,
            vec![Unmet {
                requirement: Requirement::R3Admission,
                reason: REASON_ACCOUNT_ALLOWANCE_SPENT,
            }]
        );
        assert!(v.blocks(), "fails closed, as before");
        // Outside the anchored namespaces admission never applies.
        let invited = cfg("tenant-1", &["debugging_evaluation"], false, None);
        assert!(
            evaluate_with(Some(&invited), true, AccountAdmission::AllowanceSpent)
                .unmet
                .is_empty()
        );
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
