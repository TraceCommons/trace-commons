//! The witness surface's words and tones, in one place, for all three
//! shells.
//!
//! This module exists for the same reason [`crate::routing_copy`] does, and
//! for one more. The general reason: three shells render this surface, and a
//! word kept in three places is a privacy claim that silently stops matching
//! itself. The specific reason: the Windows shell's interop tests
//! (`NoWordingIsAuthoredInThisShell`,
//! `TheSettingsScreenAsksForTheRowRatherThanWritingIt`) fail on a hand
//! authored string literal, so on that shell there is nowhere else for these
//! words to live.
//!
//! GTK does not go through the C ABI -- it depends on
//! `trace-commons-contributor` directly -- so everything here is ordinary
//! public Rust, and `trace-commons-contributor-ffi` is a thin projection of
//! it. Nothing about this surface may exist only in the FFI crate; that is
//! how the three shells drift apart.
//!
//! # What a certificate says
//!
//! A witness certificate covers redaction mechanics and a residual-risk
//! verdict. **It does not say a trace is clean**, and no sentence in this
//! module may be read as saying so. The same rule governs the receipt count:
//! it is reported as `n of m`, never as the word "attested" and never as a
//! flag.

use serde::Serialize;

use crate::witness::status::{InferenceReceiptCount, WitnessLastResult, WitnessTrustState};

/// The card's heading.
pub const WITNESS_HEADING: &str = "Redaction witness";

/// What the card is for, in one sentence a contributor can act on.
pub const WITNESS_INTRO: &str = concat!(
    "A witness is a sealed machine that removes private material from a session for you, ",
    "instead of this app doing it here. Turning it on means sending the session to that ",
    "machine before anything is redacted, which is why it is checked against a measurement ",
    "you pin before a single byte leaves."
);

/// What the certificate proves, stated where a contributor reads it, so no
/// shell has to summarise it and no shell summarises it wrongly.
pub const WITNESS_CERTIFICATE_MEANS: &str = concat!(
    "A certificate records what the witness removed and the risk it judged was left. ",
    "It is not a statement that a session is clean."
);

/// Why the pin is a list and not a value.
pub const WITNESS_MEASUREMENTS_NOTE: &str = concat!(
    "More than one measurement can be pinned. An upgrade to the witness changes its ",
    "measurement, so the new one is added here before the change happens; a client that ",
    "holds only the old one will refuse the upgraded witness."
);

/// Field titles.
pub const WITNESS_URL_TITLE: &str = "Address";
pub const WITNESS_SIGNING_ADDRESS_TITLE: &str = "Signing key";
pub const WITNESS_MEASUREMENTS_TITLE: &str = "Pinned measurements";

/// Actions.
pub const WITNESS_CONFIGURE: &str = "Use this witness";
pub const WITNESS_CLEAR: &str = "Stop using a witness";

/// What clearing actually does. Not "off": the redaction still happens, on
/// this machine, and saying "off" would read as no redaction at all.
pub const WITNESS_CLEAR_NOTE: &str = concat!(
    "Sessions go back to being redacted on this machine, and are sent with this app's own ",
    "judgement of what was left rather than a certificate."
);

/// Changes take effect on the next upload, with no restart.
pub const WITNESS_APPLIES_AT_ONCE: &str = "Changes here apply to the next session sent.";

/// Consent to carry inference content is independent of observing a proxy's
/// ledger, configuring a witness, and acknowledging the extra privacy scan.
pub const WITNESS_INFERENCE_HEADING: &str = "Include captured inference evidence";
pub const WITNESS_INFERENCE_DISCLOSURE: &str = concat!(
    "When a contribution uses a witness, this allows the final model call's exact request ",
    "and response to be sent to that remote witness before redaction. These may include ",
    "prompts, conversation history, tool results, and secrets. The witness checks the ",
    "evidence and removes these attached bodies from the contribution it returns. ",
    "Looking up a receipt with NEAR AI can also reveal which call you are contributing."
);
pub const WITNESS_INFERENCE_CAPTURE_NOTE: &str = concat!(
    "IronWire capture must be configured separately. When capture is enabled, request ",
    "and response bodies are stored on this machine. Turning this permission off stops ",
    "including bodies in future contributions; it does not turn off IronWire capture ",
    "or delete bodies already stored. Work already in progress may still finish."
);
pub const WITNESS_INFERENCE_SCOPE_NOTE: &str = concat!(
    "This permission does not connect an agent to NEAR AI, fund inference, or prove a ",
    "receipt was verified. A supported desktop app asks separately before sending a ",
    "session for witness review. This permission alone does not make it ready to send."
);
pub const WITNESS_INFERENCE_ENABLE: &str = "Review permission";
pub const WITNESS_INFERENCE_DISABLE: &str = "Stop including inference bodies";
pub const WITNESS_INFERENCE_CONFIRM: &str = "Allow sending captured bodies";
pub const WITNESS_INFERENCE_CANCEL: &str = "Not now";
pub const WITNESS_INFERENCE_ENABLED: &str =
    "Permission saved. Captured bodies may be included when a contribution uses a witness.";
pub const WITNESS_INFERENCE_DISABLED: &str = "Captured inference bodies are not included.";
pub const WITNESS_INFERENCE_SAVE_FAILED: &str =
    "Couldn't confirm this permission was saved. Reload settings to check before continuing.";

/// How a witness sentence is painted.
///
/// Five values, and the fifth is the reason this is not
/// [`crate::routing_copy::StateTone`]. A configured witness with nothing
/// pinned sends **nothing at all**, and neither of the two tones that could
/// otherwise carry it is honest: `Attention` is the tone of "something here
/// needs fixing before this can work", which reads as a degraded but
/// functioning setup, and `Neutral` reads as off. A refusal is neither.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WitnessTone {
    /// Says nothing either way. No witness is configured, which is a
    /// supported mode and not a fault.
    Neutral,
    /// Configured, and no answer has arrived yet.
    Held,
    /// Configured, pinned, and working.
    Clear,
    /// Something on this machine needs fixing, but sessions still go out.
    Attention,
    /// Nothing is being sent at all until this is resolved.
    Refused,
}

impl WitnessTone {
    /// The stable integer this tone travels as across the C ABI.
    ///
    /// # Why these are not the routing tone's numbers
    ///
    /// The obvious choice was to reuse `TC_ROUTING_TONE_NEUTRAL`..`ATTENTION`
    /// (0..=3) and give `Refused` the next value. That is the dangerous
    /// choice. The existing consumers of the routing tone spell out their
    /// arms and map anything else to *neutral* -- Windows'
    /// `RoutingSurface.FromAbiTone` does exactly this -- so a witness tone
    /// fed into a routing mapper would render a refusal as "nothing to say".
    /// Silently degrading a refusal to neutral is the precise failure this
    /// whole surface exists to prevent, and picking overlapping numbers
    /// makes it a one-line mistake.
    ///
    /// A disjoint range makes that mistake loud instead: a witness tone
    /// passed to a routing mapper is unrecognised for EVERY value, not just
    /// the new one, so the surface is visibly wrong the first time anyone
    /// looks at it rather than wrong only in the case nobody tested.
    ///
    /// Extending the routing enum was the other option and is rejected for
    /// the same reason in reverse: it would widen the domain of an ABI value
    /// every current consumer already switches on, and every one of them
    /// would keep compiling.
    pub fn abi_code(self) -> i32 {
        match self {
            Self::Neutral => 10,
            Self::Held => 11,
            Self::Clear => 12,
            Self::Attention => 13,
            Self::Refused => 14,
        }
    }
}

/// The sentence for a witness state.
///
/// One sentence per state, because the states are one per condition. In
/// particular [`WitnessTrustState::Absent`] and
/// [`WitnessTrustState::RefusingUnpinned`] get sentences that cannot be
/// mistaken for each other: the first says redaction is happening here, the
/// second says nothing is being sent.
#[must_use]
pub fn witness_state_line(state: WitnessTrustState) -> &'static str {
    match state {
        WitnessTrustState::Absent => concat!(
            "Not in use. Sessions are redacted on this machine before they are sent, ",
            "which is the normal arrangement."
        ),
        WitnessTrustState::Pinned => concat!(
            "In use. Each session is redacted by the pinned witness, which signs a record ",
            "of what it removed and the risk it judged was left."
        ),
        WitnessTrustState::RefusingUnpinned => concat!(
            "Nothing is being sent. A witness is set but no measurement is pinned, and this ",
            "app will not hand a session to a machine it cannot check. Pin the measurement ",
            "this witness reports."
        ),
        WitnessTrustState::RefusingPinMalformed => concat!(
            "Nothing is being sent. The pinned measurement could not be read, so this app ",
            "cannot check the witness. Check what was entered against what the witness ",
            "reports."
        ),
        WitnessTrustState::RefusingInferenceReceiptsMissing => concat!(
            "Nothing is being sent. The witness requires a receipt for each model call, and ",
            "these sessions carried none."
        ),
        WitnessTrustState::NotEnrolled => {
            "Not set up yet. Join an instance first, and a witness can be chosen after that."
        }
        WitnessTrustState::SettingsUnreadable => concat!(
            "Nothing is being sent. This app could not read its own settings, so it cannot ",
            "say what would happen to a session."
        ),
    }
}

/// The tone [`witness_state_line`]'s sentence is painted in.
///
/// ONE BRANCH TABLE, NOT TWO. This takes what the sentence takes, so the two
/// stay in step by construction, and no shell may recover the tone by
/// comparing the rendered sentence against anything.
#[must_use]
pub fn witness_state_tone(state: WitnessTrustState) -> WitnessTone {
    match state {
        WitnessTrustState::Absent => WitnessTone::Neutral,
        WitnessTrustState::Pinned => WitnessTone::Clear,
        // Nothing about a witness is being declined here; the device has no
        // account yet. Painting it as a refusal would accuse a setup that has
        // simply not happened.
        WitnessTrustState::NotEnrolled => WitnessTone::Neutral,
        WitnessTrustState::RefusingUnpinned
        | WitnessTrustState::RefusingPinMalformed
        | WitnessTrustState::RefusingInferenceReceiptsMissing
        | WitnessTrustState::SettingsUnreadable => WitnessTone::Refused,
    }
}

/// How many measurement sets are pinned, as a sentence.
///
/// Two shells rendered this as a bare numeral and both declined to write a
/// sentence for it, which is the right instinct and the reason this function
/// exists: a number with no words around it on a privacy surface is a shell
/// authoring wording by omission.
///
/// The zero case says only that nothing is pinned. It does NOT repeat the
/// outage -- [`witness_state_line`] already leads with "Nothing is being
/// sent." for that state, and a card that says it twice reads as two
/// separate faults.
#[must_use]
pub fn witness_pinned_count_line(count: usize) -> String {
    match count {
        0 => "No measurement is pinned.".to_string(),
        1 => "One measurement is pinned.".to_string(),
        n => format!("{n} measurements are pinned."),
    }
}

/// How many of a session's model calls carried a receipt, as a sentence.
///
/// ALWAYS THE PAIR. There is no sentence here that says "attested", and none
/// that reduces the count to a yes or a no: a flag would be false on nearly
/// every real session while reading as an accusation, and true on a session
/// with one model call in it while reading as a guarantee.
#[must_use]
pub fn witness_n_of_m_line(count: InferenceReceiptCount) -> String {
    if count.m == 1 {
        return format!("{} of 1 model call carried a receipt.", count.n);
    }
    format!("{} of {} model calls carried a receipt.", count.n, count.m)
}

/// The sentence for what the last submission did about the witness.
#[must_use]
pub fn witness_last_result_line(result: &WitnessLastResult) -> String {
    match result {
        WitnessLastResult::NotObserved => {
            "Nothing has been sent since this app started.".to_string()
        }
        WitnessLastResult::LocalRedaction => {
            "Last sent: redacted on this machine, with no certificate.".to_string()
        }
        WitnessLastResult::Certified { n_of_m } => {
            let mut line = concat!(
                "Last sent: redacted by the witness, which signed a record of what it ",
                "removed and the risk it judged was left."
            )
            .to_string();
            if let Some(count) = n_of_m {
                line.push(' ');
                line.push_str(&witness_n_of_m_line(*count));
            }
            line
        }
        // The label is deliberately absent from the sentence. It is a fixed
        // operator string, not wording, and a contributor reading "nothing
        // was sent" needs to know that before they need to know which check
        // said so. A shell that wants to show it has it separately, from
        // the status surface.
        WitnessLastResult::Refused {
            certificate_obtained,
            ..
        } => {
            if *certificate_obtained {
                concat!(
                    "Last send was refused. The witness answered, but its certificate did not ",
                    "match what it returned, so nothing was sent."
                )
                .to_string()
            } else {
                "Last send was refused. The witness could not be used, so nothing was sent."
                    .to_string()
            }
        }
    }
}

/// The tone [`witness_last_result_line`]'s sentence is painted in.
#[must_use]
pub fn witness_last_result_tone(result: &WitnessLastResult) -> WitnessTone {
    match result {
        WitnessLastResult::NotObserved => WitnessTone::Held,
        // Not `Clear`. Local redaction is the normal arrangement and claims
        // nothing beyond itself; painting it as reassuring would put the
        // same tone on it as on a certified send.
        WitnessLastResult::LocalRedaction => WitnessTone::Neutral,
        WitnessLastResult::Certified { .. } => WitnessTone::Clear,
        WitnessLastResult::Refused { .. } => WitnessTone::Refused,
    }
}

/// Explicit remote review disclosure, shared by every native shell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WitnessReviewCopy {
    pub heading: &'static str,
    pub disclosure: &'static str,
    pub action: &'static str,
    pub confirm: &'static str,
    pub cancel: &'static str,
    pub working: &'static str,
    pub failed: &'static str,
    pub immutable: &'static str,
}

/// Persistent next steps, without inferring acceptance or funding from local setup.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FirstContributionCopy {
    pub heading: &'static str,
    pub start: &'static str,
    pub review: &'static str,
    pub follow_up: &'static str,
    pub agent_setup: &'static str,
}

/// Fixed wallet words used by the core state machine and all native adapters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WalletCopy {
    pub heading: &'static str,
    pub disclosure: &'static str,
    pub commons: &'static str,
    pub account: &'static str,
    pub check: &'static str,
    pub start: &'static str,
    pub cancel: &'static str,
    pub available: &'static str,
    pub unavailable: &'static str,
    pub opening: &'static str,
    pub waiting: &'static str,
    pub failed: &'static str,
    pub cancelled: &'static str,
    pub refused_glyph: &'static str,
    pub refused_tone: &'static str,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AdmissionCopy {
    pub heading: &'static str,
    pub disclosure: &'static str,
    pub prerequisite: &'static str,
    pub backend: &'static str,
    pub confirm: &'static str,
    pub cancel: &'static str,
    pub permission: &'static str,
    pub working: &'static str,
    pub ready: &'static str,
    pub failed: &'static str,
    pub refused_glyph: &'static str,
    pub refused_tone: &'static str,
}

/// Every fixed word on the witness surface, in one value.
///
/// ONE CALL, NOT ONE PER STRING, for the reason [`crate::routing_copy`]
/// gives: a shell handed the words one at a time takes some of them and
/// writes the rest, and a hand-written word here is a privacy claim that
/// stops matching what the other shells print.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WitnessCopy {
    pub heading: &'static str,
    pub intro: &'static str,
    pub certificate_means: &'static str,
    pub measurements_note: &'static str,
    pub url_title: &'static str,
    pub signing_address_title: &'static str,
    pub measurements_title: &'static str,
    pub configure: &'static str,
    pub clear: &'static str,
    pub clear_note: &'static str,
    pub applies_at_once: &'static str,
    pub inference_heading: &'static str,
    pub inference_disclosure: &'static str,
    pub inference_capture_note: &'static str,
    pub inference_scope_note: &'static str,
    pub inference_enable: &'static str,
    pub inference_disable: &'static str,
    pub inference_confirm: &'static str,
    pub inference_cancel: &'static str,
    pub inference_enabled: &'static str,
    pub inference_disabled: &'static str,
    pub inference_save_failed: &'static str,
    pub review: WitnessReviewCopy,
    pub onboarding: FirstContributionCopy,
    pub wallet: WalletCopy,
    pub admission: AdmissionCopy,
}

/// The witness surface's fixed words.
#[must_use]
pub fn witness_copy() -> WitnessCopy {
    WitnessCopy {
        heading: WITNESS_HEADING,
        intro: WITNESS_INTRO,
        certificate_means: WITNESS_CERTIFICATE_MEANS,
        measurements_note: WITNESS_MEASUREMENTS_NOTE,
        url_title: WITNESS_URL_TITLE,
        signing_address_title: WITNESS_SIGNING_ADDRESS_TITLE,
        measurements_title: WITNESS_MEASUREMENTS_TITLE,
        configure: WITNESS_CONFIGURE,
        clear: WITNESS_CLEAR,
        clear_note: WITNESS_CLEAR_NOTE,
        applies_at_once: WITNESS_APPLIES_AT_ONCE,
        inference_heading: WITNESS_INFERENCE_HEADING,
        inference_disclosure: WITNESS_INFERENCE_DISCLOSURE,
        inference_capture_note: WITNESS_INFERENCE_CAPTURE_NOTE,
        inference_scope_note: WITNESS_INFERENCE_SCOPE_NOTE,
        inference_enable: WITNESS_INFERENCE_ENABLE,
        inference_disable: WITNESS_INFERENCE_DISABLE,
        inference_confirm: WITNESS_INFERENCE_CONFIRM,
        inference_cancel: WITNESS_INFERENCE_CANCEL,
        inference_enabled: WITNESS_INFERENCE_ENABLED,
        inference_disabled: WITNESS_INFERENCE_DISABLED,
        inference_save_failed: WITNESS_INFERENCE_SAVE_FAILED,
        review: WitnessReviewCopy {
            heading: "Review with your configured witness",
            disclosure: "This sends this session, including its unredacted conversation and any correction you include, to your configured remote witness before you approve a contribution. It may contain prompts, tool results, personal data, or secrets. Captured inference bodies are included only with the separate saved permission. You can inspect the returned redacted contribution before deciding whether to send it. Cancelling afterwards cannot recall a session already sent to the witness.",
            action: "Prepare witness review",
            confirm: "Send this session for review",
            cancel: "Not now",
            working: "Preparing your witness review. The session may already have left this device.",
            failed: "The witness review could not be confirmed. The session may already have reached the witness. No contribution has been approved here. Try again only if you want to send another review request.",
            immutable: "This certified review is fixed. Outcome and correction edits are unavailable for this review.",
        },
        wallet: WalletCopy {
            heading: "Join with a NEAR account",
            disclosure: "Check whether your commons accepts new accounts. Connecting proves control of your account and this device; it does not fund inference or enable capture.",
            commons: "Commons HTTPS address",
            account: "Your NEAR account",
            check: "Check availability",
            start: "Continue in wallet",
            cancel: "Cancel connection",
            available: "This commons supports wallet signup.",
            unavailable: "Wallet signup is unavailable for this commons. You can still use an invite.",
            opening: "Opening a wallet connection…",
            waiting: "Finish signing in your wallet. Keep this window open.",
            failed: "The wallet connection could not be confirmed. Cancel and try again.",
            cancelled: "Connection cancelled.",
            refused_glyph: "⊘",
            refused_tone: "refused",
        },
        admission: AdmissionCopy {
            heading: "Prepare next NEAR inference",
            disclosure: "For new inference evidence, this adds an account-bound challenge to the next request in this session. Use your own funded NEAR AI backend, then continue the agent task and return here to review. You can separately choose witness review of eligible existing history, subject to server limits.",
            prerequisite: "IronWire must already route this agent to that backend and capture request bodies. Inference-body evidence also needs your separate permission in Settings.",
            backend: "NEAR AI backend name",
            confirm: "Prepare session",
            cancel: "Cancel",
            permission: "Review inference-body permission",
            working: "Preparing this session…",
            ready: "Ready. Continue this session in your agent, then review the updated session.",
            failed: "This session could not be prepared. Check your supported agent, backend, and capture settings, then try again.",
            refused_glyph: "⊘",
            refused_tone: "refused",
        },
        onboarding: FirstContributionCopy {
            heading: "Your first contribution",
            start: "Start with an existing session you can share, or complete a new task in a supported agent. Choose its session folder in Settings, then return here to review. Setup alone does not mean a contribution was accepted.",
            review: "Open a waiting session with Look inside. A configured witness asks separately before the session leaves this device for review. Check the returned contribution before sending it. The server may allow limited initial submissions from eligible existing history; this screen does not show a remaining allowance.",
            follow_up: "Open History to follow the server's recorded result. Upload, acceptance, and credit are separate steps. Points are not a spendable NEAR AI balance.",
            agent_setup: "To generate new NEAR AI inference evidence, configure your selected agent using your own funded provider account and model settings. IronWire capture and sending captured bodies each require separate setup. Existing-history review is a separate choice; this app does not create a funded provider account.",
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY_STATE: [WitnessTrustState; 7] = [
        WitnessTrustState::Absent,
        WitnessTrustState::Pinned,
        WitnessTrustState::RefusingUnpinned,
        WitnessTrustState::RefusingPinMalformed,
        WitnessTrustState::RefusingInferenceReceiptsMissing,
        WitnessTrustState::NotEnrolled,
        WitnessTrustState::SettingsUnreadable,
    ];

    #[test]
    fn every_state_has_its_own_sentence() {
        let mut seen: Vec<&str> = Vec::new();
        for state in EVERY_STATE {
            let line = witness_state_line(state);
            assert!(!line.is_empty());
            assert!(
                !seen.contains(&line),
                "{state:?} reuses another state's sentence"
            );
            seen.push(line);
        }
    }

    #[test]
    fn absent_and_unpinned_do_not_read_alike() {
        let absent = witness_state_line(WitnessTrustState::Absent);
        let unpinned = witness_state_line(WitnessTrustState::RefusingUnpinned);
        assert_ne!(absent, unpinned);
        // The one fact a contributor needs from the refusal is that nothing
        // is going out; the one fact they need from the absent state is that
        // redaction still happens here.
        assert!(
            unpinned.starts_with("Nothing is being sent."),
            "the unpinned sentence must lead with the outage: {unpinned}"
        );
        assert!(
            !absent.contains("Nothing is being sent"),
            "no witness is not an outage: {absent}"
        );
    }

    #[test]
    fn a_refusal_is_painted_as_a_refusal_and_never_as_attention() {
        for state in EVERY_STATE {
            let tone = witness_state_tone(state);
            if state.is_refusing() {
                assert_eq!(
                    tone,
                    WitnessTone::Refused,
                    "{state:?} sends nothing at all; Attention would read as degraded but \
                     working, and Neutral would read as off"
                );
            } else {
                assert_ne!(tone, WitnessTone::Refused, "{state:?} is not a refusal");
            }
        }
        assert_eq!(
            witness_state_tone(WitnessTrustState::Absent),
            WitnessTone::Neutral
        );
        assert_eq!(
            witness_state_tone(WitnessTrustState::Pinned),
            WitnessTone::Clear
        );
    }

    #[test]
    fn the_tone_numbering_shares_nothing_with_the_routing_one() {
        assert_eq!(WitnessTone::Neutral.abi_code(), 10);
        assert_eq!(WitnessTone::Held.abi_code(), 11);
        assert_eq!(WitnessTone::Clear.abi_code(), 12);
        assert_eq!(WitnessTone::Attention.abi_code(), 13);
        assert_eq!(WitnessTone::Refused.abi_code(), 14);

        // The routing tone's own numbers, restated rather than imported,
        // because the point is that the two sets must not meet. A shell
        // that cross-wires the two mappers must be wrong for every value
        // and not just for the refusal, which is the value a routing mapper
        // would quietly turn into "nothing to say".
        let routing = [0, 1, 2, 3];
        for tone in [
            WitnessTone::Neutral,
            WitnessTone::Held,
            WitnessTone::Clear,
            WitnessTone::Attention,
            WitnessTone::Refused,
        ] {
            assert!(
                !routing.contains(&tone.abi_code()),
                "{tone:?} collides with a routing tone value"
            );
        }
    }

    #[test]
    fn no_sentence_says_a_session_is_clean_or_attested() {
        let mut lines: Vec<String> = EVERY_STATE
            .iter()
            .map(|s| witness_state_line(*s).to_string())
            .collect();
        lines.push(WITNESS_INTRO.to_string());
        lines.push(WITNESS_CERTIFICATE_MEANS.to_string());
        lines.push(WITNESS_CLEAR_NOTE.to_string());
        lines.push(witness_n_of_m_line(InferenceReceiptCount { n: 3, m: 7 }));
        for count in [0usize, 1, 2] {
            lines.push(witness_pinned_count_line(count));
        }
        for result in [
            WitnessLastResult::NotObserved,
            WitnessLastResult::LocalRedaction,
            WitnessLastResult::Certified {
                n_of_m: Some(InferenceReceiptCount { n: 3, m: 7 }),
            },
            WitnessLastResult::Refused {
                label: "witness_quote_unverified".into(),
                certificate_obtained: false,
            },
        ] {
            lines.push(witness_last_result_line(&result));
        }

        for line in lines {
            let lowered = line.to_lowercase();
            assert!(
                !lowered.contains("attested") && !lowered.contains("attests"),
                "a surface must report n of m, never the word attested: {line}"
            );
            // "not a statement that a session is clean" is the one place the
            // word may appear, and only in that denial.
            if lowered.contains("clean") {
                assert!(
                    lowered.contains("not a statement that a session is clean"),
                    "a certificate never says a session is clean: {line}"
                );
            }
        }
    }

    #[test]
    fn the_pinned_count_is_a_sentence_and_never_a_bare_numeral() {
        assert_eq!(witness_pinned_count_line(0), "No measurement is pinned.");
        assert_eq!(witness_pinned_count_line(1), "One measurement is pinned.");
        assert_eq!(witness_pinned_count_line(2), "2 measurements are pinned.");
        assert_eq!(witness_pinned_count_line(11), "11 measurements are pinned.");
        for count in [0usize, 1, 2, 11] {
            let line = witness_pinned_count_line(count);
            assert!(line.ends_with('.'), "{line} is not a sentence");
            assert!(
                line.split_whitespace().count() > 1,
                "{line} is a bare numeral, which is a shell writing wording by omission"
            );
        }
        // The zero case must not repeat the outage: the state line already
        // leads with it, and a card saying it twice reads as two faults.
        assert!(
            !witness_pinned_count_line(0).contains("Nothing is being sent"),
            "the state line already says this"
        );
    }

    #[test]
    fn the_receipt_count_is_always_a_pair() {
        assert_eq!(
            witness_n_of_m_line(InferenceReceiptCount { n: 3, m: 7 }),
            "3 of 7 model calls carried a receipt."
        );
        assert_eq!(
            witness_n_of_m_line(InferenceReceiptCount { n: 0, m: 1 }),
            "0 of 1 model call carried a receipt."
        );
        assert_eq!(
            witness_n_of_m_line(InferenceReceiptCount { n: 0, m: 0 }),
            "0 of 0 model calls carried a receipt."
        );
    }

    #[test]
    fn a_certified_send_carries_the_count_into_its_sentence() {
        let with = witness_last_result_line(&WitnessLastResult::Certified {
            n_of_m: Some(InferenceReceiptCount { n: 3, m: 7 }),
        });
        let without = witness_last_result_line(&WitnessLastResult::Certified { n_of_m: None });
        assert!(with.contains("3 of 7 model calls carried a receipt."));
        assert!(!without.contains("receipt"));
        assert!(with.starts_with(&without));
    }

    #[test]
    fn local_redaction_and_no_send_yet_are_different_sentences_and_tones() {
        let local = witness_last_result_line(&WitnessLastResult::LocalRedaction);
        let never = witness_last_result_line(&WitnessLastResult::NotObserved);
        assert_ne!(local, never);
        assert_ne!(
            witness_last_result_tone(&WitnessLastResult::LocalRedaction),
            witness_last_result_tone(&WitnessLastResult::NotObserved)
        );
        assert_ne!(
            witness_last_result_tone(&WitnessLastResult::LocalRedaction),
            WitnessTone::Clear,
            "local redaction claims nothing beyond itself and must not wear the same tone \
             as a certified send"
        );
    }

    #[test]
    fn a_refused_send_reads_as_nothing_sent_in_both_shapes() {
        for obtained in [true, false] {
            let line = witness_last_result_line(&WitnessLastResult::Refused {
                label: "witness_certificate_mismatched".into(),
                certificate_obtained: obtained,
            });
            assert!(
                line.contains("nothing was sent"),
                "a refusal must say nothing was sent: {line}"
            );
            assert!(
                !line.contains("witness_certificate_mismatched"),
                "an operator label is not wording: {line}"
            );
            assert_eq!(
                witness_last_result_tone(&WitnessLastResult::Refused {
                    label: "witness_certificate_mismatched".into(),
                    certificate_obtained: obtained,
                }),
                WitnessTone::Refused
            );
        }
        let obtained = witness_last_result_line(&WitnessLastResult::Refused {
            label: "witness_certificate_mismatched".into(),
            certificate_obtained: true,
        });
        let never = witness_last_result_line(&WitnessLastResult::Refused {
            label: "witness_attestation_unavailable".into(),
            certificate_obtained: false,
        });
        assert_ne!(
            obtained, never,
            "a witness that answered with a certificate that does not hold is a different \
             fact from one that never answered"
        );
    }

    #[test]
    fn the_copy_call_carries_every_fixed_word() {
        let copy = witness_copy();
        let json = serde_json::to_value(&copy).unwrap();
        let object = json.as_object().unwrap();
        assert_eq!(
            object.len(),
            26,
            "a field added to WitnessCopy must be counted here, or a shell can be handed \
             a word this test has never seen"
        );
        for (key, value) in object {
            assert!(
                value.as_str().is_some_and(|text| !text.is_empty())
                    || value.as_object().is_some_and(|fields| fields
                        .values()
                        .all(|text| text.as_str().is_some_and(|text| !text.is_empty()))),
                "{key} is empty, so a shell renders a blank and writes its own"
            );
        }
    }
}
