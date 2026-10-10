//! What an armed folder was told happens to its sessions, and what it would
//! be told now.
//!
//! K5 in the #991 split, and the first of the two notices the
//! connect-and-forget design (`docs/superpowers/specs/2026-09-23-connect-and-forget-consent-design.md`,
//! "The prior decision" and "When enforcement is switched on") puts on the
//! enforcement switch-on list: a folder armed under the old "will be
//! scrubbed" copy, whose wording becomes deterministic-only, is told what its
//! arming now means. Changing the words under an armed folder without saying
//! so would break the "nothing silently" property the design depends on.
//!
//! # Keyed on the wording, not on the disclosure alone
//!
//! The notice is owed when the words a folder would be shown now stop
//! claiming what the words it was armed under claimed. Two things decide the
//! words now, and they move separately:
//!
//! - the disclosure R1 allows (`automatic_gate::disclosure`, and the
//!   per-folder answer K6 adds beside it), and
//! - the arming copy each disclosure is given ([`project_arming_claim`]).
//!
//! Today every folder armed from a project's arming offer was shown
//! `project_copy::ARMING_BODY`, "Traces from this project will be scrubbed
//! and contributed", whatever the disclosure, and every shell still shows it.
//! So [`project_arming_claim`] answers [`ArmingClaim::ModelScrubbed`] for
//! both disclosures, and no notice is raised: nothing has been reworded yet,
//! so there is nothing to tell. When the arming copy becomes
//! disclosure-dependent (the "deterministic-only arming disclosure" on the
//! spec's Open list, and K13), the change is made in [`project_arming_claim`]
//! together with the shells' copy, and every folder armed under the old
//! wording whose disclosure is patterns-only gets its notice on the next
//! pass.
//!
//! It also happens without any copy change, per folder, from the folder's
//! own record ([`claim_in_force`], over K6's `project_disclosure`): a folder
//! armed under "will be scrubbed" that has sent a session on the
//! contributor's behalf without a certified full pipeline is told, once. The
//! record is kept by the upload pass, so the notice follows on the next
//! watcher pass.
//!
//! Nothing here reads certificates or the pipeline-version allowlist: that is
//! `automatic_gate`'s, and this module only consumes its [`Disclosure`].

use serde::{Deserialize, Serialize};

use super::automatic_gate::Disclosure;

/// What the words a folder was armed under claim about how its sessions are
/// scrubbed. Only the one distinction the K5 notice turns on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArmingClaim {
    /// The words say a model scrubs the sessions, or say "scrubbed" without
    /// saying by what: `project_copy::ARMING_BODY` ("will be scrubbed and
    /// contributed") and the `AUTO_SCRUB_*` grant wording.
    ModelScrubbed,
    /// The words say only the fixed patterns can be relied on: the
    /// `AUTO_PATTERNS_ONLY_*` grant wording.
    PatternsOnly,
}

impl ArmingClaim {
    /// Whether moving from `self` to `now` takes back a claim the contributor
    /// relied on when arming. Only a narrowing is owed a notice: a folder
    /// told "patterns only" that later earns the model-scrub wording has
    /// lost nothing it was promised.
    pub fn narrowed_to(self, now: ArmingClaim) -> bool {
        self == ArmingClaim::ModelScrubbed && now == ArmingClaim::PatternsOnly
    }
}

/// What a folder armed before its claim was recorded was told.
///
/// A folder armed by the contributor saw `project_copy::ARMING_BODY`, which
/// claims scrubbing. A folder armed by the Flow 1 grant saw the grant screen,
/// which has only ever been given `automatic_gate::disclosure`, and that
/// always answers `PatternsOnly`.
pub fn legacy_claim(armed_by_grant: bool) -> ArmingClaim {
    if armed_by_grant {
        ArmingClaim::PatternsOnly
    } else {
        ArmingClaim::ModelScrubbed
    }
}

/// What a project's arming offer says, for a folder with this disclosure.
///
/// Every shell shows `project_copy::ARMING_BODY` for every folder, which
/// says its sessions "will be scrubbed", so this is
/// [`ArmingClaim::ModelScrubbed`] whatever the disclosure. **Change this in
/// the same PR that makes the arming offer's copy depend on the disclosure**:
/// that change is what rewords already-armed folders, and this function is
/// what tells them.
pub fn project_arming_claim(disclosure: Disclosure) -> ArmingClaim {
    #[cfg(test)]
    if let Some(claim) = REWORDED_FOR_TEST.with(std::cell::Cell::get) {
        return match disclosure {
            Disclosure::ModelScrubbed => ArmingClaim::ModelScrubbed,
            Disclosure::PatternsOnly => claim,
        };
    }
    let _ = disclosure;
    ArmingClaim::ModelScrubbed
}

/// The claim in force for one armed folder, from its own disclosure (K6's
/// `automatic_gate::project_disclosure`), for the K5 sweep.
///
/// A folder armed under "will be scrubbed" is told its arming now means
/// patterns-only exactly when its own record contradicts that wording: it has
/// sent at least one session on the contributor's behalf since it was armed,
/// and not every such session had a certified full pipeline. A folder whose
/// every automatic send was certified keeps the scrub claim, and so does one
/// that has sent nothing yet, which is judged by the arming offer's words
/// alone ([`project_arming_claim`]). Arming again resets the record, so a
/// re-armed folder is judged afresh.
pub fn claim_in_force(policy: &super::policy::ProjectPolicy, project_key: &str) -> ArmingClaim {
    match super::automatic_gate::project_disclosure(policy, project_key) {
        Disclosure::ModelScrubbed => ArmingClaim::ModelScrubbed,
        // Evidence: at least one session went out on the contributor's
        // behalf since arming, and not every one had a certified full
        // pipeline (`project_disclosure` answers `ModelScrubbed` otherwise).
        // "Will be scrubbed" was not true of that session.
        Disclosure::PatternsOnly if sent_automatically(policy, project_key) => {
            ArmingClaim::PatternsOnly
        }
        // Nothing sent since arming: no evidence either way, so the words in
        // force are whatever the arming offer says for this disclosure.
        disclosure @ Disclosure::PatternsOnly => project_arming_claim(disclosure),
    }
}

/// Whether any session has gone out from this armed folder on the
/// contributor's behalf since it was last armed: K6's tally, read as a count.
fn sent_automatically(policy: &super::policy::ProjectPolicy, project_key: &str) -> bool {
    policy.projects.get(project_key).is_some_and(|entry| {
        let tally = entry.automatic_redaction;
        tally.certified_full_pipeline > 0 || tally.not_certified > 0
    })
}

/// What the Flow 1 grant screen says, for this disclosure:
/// `consent_copy::automatic_grant_copy` carries exactly one of the two
/// wordings, the one the disclosure names.
pub fn grant_arming_claim(disclosure: Disclosure) -> ArmingClaim {
    match disclosure {
        Disclosure::ModelScrubbed => ArmingClaim::ModelScrubbed,
        Disclosure::PatternsOnly => ArmingClaim::PatternsOnly,
    }
}

// Lets a test play the rewording that the arming-copy change will make,
// before it is made. Thread-local for the reason `watcher`'s
// `ENFORCE_GATE_FOR_TEST` is; compiled out of every non-test build.
#[cfg(test)]
thread_local! {
    static REWORDED_FOR_TEST: std::cell::Cell<Option<ArmingClaim>> = const { std::cell::Cell::new(None) };
}

/// Make [`project_arming_claim`] answer `claim` for a patterns-only folder on
/// this thread, as it will once the arming copy is disclosure-dependent.
#[cfg(test)]
pub(crate) fn reword_patterns_only_for_test(claim: Option<ArmingClaim>) {
    REWORDED_FOR_TEST.with(|c| c.set(claim));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Today's arming offer says "will be scrubbed" for every folder, so
    /// the claim does not move with the disclosure and nothing is reworded.
    #[test]
    fn todays_arming_offer_claims_scrubbing_whatever_the_disclosure() {
        for d in [Disclosure::PatternsOnly, Disclosure::ModelScrubbed] {
            assert_eq!(project_arming_claim(d), ArmingClaim::ModelScrubbed);
        }
        assert_eq!(
            crate::project_copy::ARMING_BODY
                .split_whitespace()
                .take(7)
                .collect::<Vec<_>>()
                .join(" "),
            "Traces from this project will be scrubbed",
            "if the arming offer stops saying this, project_arming_claim must change with it"
        );
    }

    #[test]
    fn the_grant_screen_claims_what_its_disclosure_names() {
        assert_eq!(
            grant_arming_claim(Disclosure::PatternsOnly),
            ArmingClaim::PatternsOnly
        );
        assert_eq!(
            grant_arming_claim(Disclosure::ModelScrubbed),
            ArmingClaim::ModelScrubbed
        );
    }

    #[test]
    fn only_a_narrowing_is_owed_a_notice() {
        use ArmingClaim::{ModelScrubbed, PatternsOnly};
        assert!(ModelScrubbed.narrowed_to(PatternsOnly));
        assert!(!PatternsOnly.narrowed_to(ModelScrubbed));
        assert!(!ModelScrubbed.narrowed_to(ModelScrubbed));
        assert!(!PatternsOnly.narrowed_to(PatternsOnly));
    }

    fn armed(key: &str) -> super::super::policy::ProjectPolicy {
        let mut p = super::super::policy::ProjectPolicy::new();
        p.set_mode(
            key,
            super::super::policy::ProjectMode::AutoUpload,
            "2026-09-27T00:00:00Z".parse().unwrap(),
        )
        .unwrap();
        p
    }

    fn sweep(p: &mut super::super::policy::ProjectPolicy) -> Vec<String> {
        let keys: Vec<String> = p.projects.keys().cloned().collect();
        let claims: std::collections::BTreeMap<String, ArmingClaim> = keys
            .iter()
            .map(|k| (k.clone(), claim_in_force(p, k)))
            .collect();
        p.sweep_arming_claims(
            |k| claims.get(k).copied().unwrap_or(ArmingClaim::ModelScrubbed),
            "2026-09-27T01:00:00Z".parse().unwrap(),
        )
    }

    /// K5 on the folder's own disclosure: one uncertified automatic send
    /// under "will be scrubbed" gets exactly one notice.
    #[test]
    fn an_uncertified_automatic_send_rewords_the_folder_once() {
        use super::super::automatic_gate::SessionRedaction;
        let mut p = armed("/w/api");
        assert!(
            sweep(&mut p).is_empty(),
            "nothing sent, nothing contradicted"
        );
        p.record_automatic_redaction("/w/api", SessionRedaction::CertifiedFullPipeline);
        p.record_automatic_redaction("/w/api", SessionRedaction::NotCertified);
        assert_eq!(sweep(&mut p), vec!["api".to_string()]);
        p.record_automatic_redaction("/w/api", SessionRedaction::NotCertified);
        assert!(sweep(&mut p).is_empty(), "told once");
        assert_eq!(p.arming_rewordings.len(), 1);
    }

    #[test]
    fn a_folder_whose_every_automatic_send_was_certified_is_not_reworded() {
        use super::super::automatic_gate::SessionRedaction;
        let mut p = armed("/w/api");
        for _ in 0..3 {
            p.record_automatic_redaction("/w/api", SessionRedaction::CertifiedFullPipeline);
        }
        assert_eq!(claim_in_force(&p, "/w/api"), ArmingClaim::ModelScrubbed);
        assert!(sweep(&mut p).is_empty());
        assert!(p.arming_rewordings.is_empty());
    }

    /// Re-arming answers the notice and starts the record again: nothing is
    /// said until a new uncertified send, and then it is said once more.
    #[test]
    fn a_rearm_resets_the_rewording() {
        use super::super::automatic_gate::SessionRedaction;
        let mut p = armed("/w/api");
        p.record_automatic_redaction("/w/api", SessionRedaction::NotCertified);
        assert_eq!(sweep(&mut p).len(), 1);

        p.set_mode(
            "/w/api",
            super::super::policy::ProjectMode::AutoUpload,
            "2026-09-27T02:00:00Z".parse().unwrap(),
        )
        .unwrap();
        assert!(p.record_arming_claim("/w/api", ArmingClaim::ModelScrubbed));
        assert!(p.arming_rewordings.is_empty(), "re-arming answers it");
        assert!(sweep(&mut p).is_empty(), "the record starts from nothing");

        p.record_automatic_redaction("/w/api", SessionRedaction::NotCertified);
        assert_eq!(sweep(&mut p).len(), 1);
        assert_eq!(p.arming_rewordings.len(), 1);
    }

    #[test]
    fn a_folder_armed_before_claims_were_recorded_was_told_what_its_screen_said() {
        assert_eq!(legacy_claim(false), ArmingClaim::ModelScrubbed);
        assert_eq!(legacy_claim(true), ArmingClaim::PatternsOnly);
    }
}
