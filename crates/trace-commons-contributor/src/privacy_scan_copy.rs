//! The extra privacy scan's words, in one place, for every shell.
//!
//! The optional second scanner sends message text to NEAR AI before Trace
//! Commons receives it. The daemon refuses to use it until the contributor
//! has been shown this notice and `acknowledge_near_ai_notice` has recorded
//! that; until then it reports `near-ai-notice-not-acknowledged` and holds
//! every upload.
//!
//! Two surfaces render these sentences: onboarding's scan screen, where the
//! choice is first made, and the recovery prompt a shell offers when the
//! daemon reports the label after onboarding. Both must show the same
//! disclosure, so both read it from here.
//!
//! The onboarding sentences are the shared design spec's "### 4. Extra
//! privacy scan" copy, which the GTK and macOS scan screens already render.
//! The recovery title, detail, and action are the ones both shells already
//! draw for `near-ai-notice-not-acknowledged`.

/// The scan screen's heading.
pub const TITLE: &str = "Extra scrub before sending? (optional)";
/// Local scrubbing is not the thing being chosen; it always runs.
pub const LOCAL_ALWAYS: &str = "Local scrubbing removes secrets, keys, tokens and credentials by \
     pattern before anything leaves this machine. It runs either way.";
/// What the second scanner is and what it reads.
pub const OFFER: &str = "You can additionally send the message text of each trace — not tool \
     output, not file contents — through a second scanner run by NEAR AI, a third party, to \
     catch personal information the patterns miss: names, addresses, that kind of thing.";
/// Both halves of the disclosure. The cost (text really does leave the
/// machine to a third party) and the reassurance (an unreachable scanner
/// holds traces rather than sending them unscanned). Cutting either half
/// makes the screen dishonest in one direction, so they live in one string.
pub const DISCLOSURE: &str = concat!(
    "This means your message text is transmitted to NEAR AI before it reaches ",
    crate::app_name!(),
    ". If that scanner is unreachable, nothing is sent at all — traces wait rather than \
     going out unscanned."
);
/// The choice that keeps message text on this machine.
pub const LOCAL_ONLY: &str = "Local scrubbing only";
/// The choice that adds the second scanner.
pub const WITH_NEAR: &str = "Local scrubbing + NEAR AI scan";

/// The recovery prompt's heading, shown while the daemon holds uploads for
/// `near-ai-notice-not-acknowledged`.
pub const RECOVERY_TITLE: &str = "One thing to confirm.";
/// Why uploads are held, and what confirming does.
pub const RECOVERY_DETAIL: &str = "You chose the extra privacy scan, which sends message text to \
     NEAR AI. Confirm you're OK with that and contributions resume.";
/// The button that opens the notice.
pub const RECOVERY_ACTION: &str = "Review and confirm";
/// The button that records the acknowledgement, drawn only beside the full
/// notice.
pub const RECOVERY_CONFIRM: &str = "Confirm the NEAR AI scan";
/// Shown while the acknowledgement is being recorded.
pub const RECOVERY_WORKING: &str = "Recording your confirmation…";
/// The acknowledgement was not confirmed. The daemon still holds uploads,
/// so this says so rather than implying anything moved.
pub const RECOVERY_FAILED: &str = "Your confirmation wasn't recorded. Contributions stay paused \
     until it is; retry to confirm again.";

/// Every fixed word on the extra-privacy-scan surfaces.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct PrivacyScanCopy {
    pub title: &'static str,
    pub local_always: &'static str,
    pub offer: &'static str,
    pub disclosure: &'static str,
    pub local_only: &'static str,
    pub with_near: &'static str,
    pub recovery_title: &'static str,
    pub recovery_detail: &'static str,
    pub recovery_action: &'static str,
    pub recovery_confirm: &'static str,
    pub recovery_working: &'static str,
    pub recovery_failed: &'static str,
    pub recovery_cancel: &'static str,
}

/// The extra-privacy-scan surfaces' words.
#[must_use]
pub fn privacy_scan_copy() -> PrivacyScanCopy {
    PrivacyScanCopy {
        title: TITLE,
        local_always: LOCAL_ALWAYS,
        offer: OFFER,
        disclosure: DISCLOSURE,
        local_only: LOCAL_ONLY,
        with_near: WITH_NEAR,
        recovery_title: RECOVERY_TITLE,
        recovery_detail: RECOVERY_DETAIL,
        recovery_action: RECOVERY_ACTION,
        recovery_confirm: RECOVERY_CONFIRM,
        recovery_working: RECOVERY_WORKING,
        recovery_failed: RECOVERY_FAILED,
        recovery_cancel: crate::onboarding_copy::NOT_NOW,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The disclosure has two halves and both must stay: text really leaves
    /// the machine to a third party, and an unreachable scanner holds traces
    /// rather than sending them unscanned.
    #[test]
    fn the_disclosure_states_the_cost_and_the_fail_closed_reassurance() {
        let copy = privacy_scan_copy();
        assert!(copy.disclosure.contains("transmitted to NEAR AI"));
        assert!(copy.disclosure.contains("nothing is sent at all"));
        assert!(copy.offer.contains("not tool output, not file contents"));
        assert!(copy.local_always.contains("It runs either way."));
    }

    /// The recovery prompt names the label's consequence and its action
    /// matches the one the macOS and GTK shells draw for this label.
    #[test]
    fn the_recovery_prompt_matches_the_other_shells() {
        let copy = privacy_scan_copy();
        assert_eq!(copy.recovery_title, "One thing to confirm.");
        assert_eq!(copy.recovery_action, "Review and confirm");
        assert!(
            copy.recovery_detail
                .contains("sends message text to NEAR AI")
        );
        assert!(copy.recovery_detail.contains("contributions resume"));
    }

    /// A failed acknowledgement leaves the daemon holding uploads, so the
    /// failure sentence must not claim anything was sent or resumed.
    #[test]
    fn the_failure_sentence_claims_nothing_was_resumed() {
        let copy = privacy_scan_copy();
        assert!(copy.recovery_failed.contains("stay paused"));
        assert!(
            !copy
                .recovery_failed
                .to_lowercase()
                .contains("nothing was sent")
        );
        assert!(!copy.recovery_failed.contains("resumed"));
    }

    #[test]
    fn serialized_copy_carries_every_field() {
        let value = serde_json::to_value(privacy_scan_copy()).unwrap();
        for key in [
            "title",
            "local_always",
            "offer",
            "disclosure",
            "local_only",
            "with_near",
            "recovery_title",
            "recovery_detail",
            "recovery_action",
            "recovery_confirm",
            "recovery_working",
            "recovery_failed",
            "recovery_cancel",
        ] {
            assert!(
                value[key].as_str().is_some_and(|text| !text.is_empty()),
                "{key} is missing"
            );
        }
    }
}
