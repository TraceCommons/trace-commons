//! Whether a queue row may offer to send the session it stands for.
//!
//! **Show every session. Offer only the eligible ones.** An ineligible row
//! is present, unoffered, and carries its reason. Hiding it would be its own
//! dishonesty -- it makes the app look as though it had not noticed files
//! the contributor knows it can see -- and it is not what the shells were
//! asked for.
//!
//! Nothing here decides anything. Every one of the four answers a row needs
//! -- the sentence, the tone, the reason line, and whether the send control
//! may be drawn -- comes from `trace_commons_contributor::
//! private_inference_copy`, which macOS and Windows reach across the C ABI
//! as `tc_eligibility_*`. This module reads one field off the entry and
//! hands the label to those four functions. There is no `match` on a state
//! name in this shell, and there must not be: a second table is a second
//! chance to draw `Submit` beside a session the server will refuse, which is
//! the whole defect this surface exists to remove.
//!
//! The one decision that IS made here is the absent-field one, and it is not
//! about eligibility at all.

use crate::copy;
use crate::model::QueueEntry;

/// What one row shows and what it may offer.
///
/// Built as a unit from a single label so the sentence, the colour and the
/// button cannot answer differently.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EligibilityView {
    /// The state sentence. Always non-empty -- an unread state has its own.
    pub state_line: &'static str,
    /// How firmly the sentence reads. `Attention` is
    /// `ineligible_configuration` alone; a permanent ineligibility is
    /// deliberately not `Refused`, because nothing was refused.
    pub tone: copy::PrivateInferenceTone,
    /// The reason detail, or the empty string, which this shell draws as no
    /// line at all rather than as a guess.
    pub reason_line: &'static str,
    /// The one control this row may offer.
    pub control: copy::ContributionControl,
}

/// The eligibility surface for one entry, or `None` when the entry has no
/// eligibility question.
///
/// **`None` IS THE ABSENT KEY, NEVER `unknown`.** A daemon sends no
/// `eligibility` field at all when the contributor was invited rather than
/// admitted on evidence, or when the config could not be read. Those rows
/// have no eligibility question, and must render exactly as they did before
/// this surface existed -- no sentence, no reason, and the send control
/// offered as it always was. `unknown` is a different thing entirely: a real
/// state that arrives on the wire, gets its own sentence, and offers
/// nothing. Collapsing the two would put "has not been worked out" on every
/// card an invited contributor owns.
///
/// `#[serde(default)]` on `QueueEntry::eligibility` is what keeps them
/// apart, so the tests go through the real deserializer.
#[must_use]
pub fn view(entry: &QueueEntry) -> Option<EligibilityView> {
    let state = entry.eligibility.as_deref()?;
    Some(EligibilityView {
        state_line: copy::eligibility_state_line(state),
        tone: copy::eligibility_state_tone(state),
        // An absent reason is the ordinary case on an `eligible` row --
        // there is nothing to explain -- and it answers the same empty
        // string an unfamiliar one does. Not `?`: a missing reason must not
        // take the state sentence down with it.
        reason_line: copy::eligibility_reason_line(
            entry.eligibility_reason.as_deref().unwrap_or_default(),
        ),
        control: copy::eligibility_control(state),
    })
}

/// Whether a row may draw a control that sends this session.
///
/// An entry with no eligibility field answers `true`: it is an invited
/// contributor's row, and it behaves as it always did. Every state but
/// `eligible` -- including one this build has never heard of -- answers
/// `false`.
#[must_use]
pub fn offers_send(entry: &QueueEntry) -> bool {
    match view(entry) {
        None => true,
        Some(view) => view.control == copy::ContributionControl::Contribute,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The source of this module and of the two views that render it.
    ///
    /// Read at compile time so a sweep cannot pass over a file that moved.
    const SOURCE: &str = include_str!("eligibility.rs");
    const QUEUE_SOURCE: &str = include_str!("ui/queue.rs");
    const PREVIEW_SOURCE: &str = include_str!("ui/preview.rs");

    /// Build an entry the way the daemon does: through the real
    /// deserializer, from real wire JSON.
    ///
    /// Never a struct literal. `#[serde(default)]` on an `Option` IS the
    /// mechanism that keeps an absent key apart from `unknown`, and a
    /// hand-built value tests the mechanism's absence.
    fn entry_from_wire(json: serde_json::Value) -> QueueEntry {
        serde_json::from_value(json).expect("the daemon's shape deserializes")
    }

    fn wire(extra: serde_json::Value) -> QueueEntry {
        let mut base = serde_json::json!({
            "entry_id": "e1",
            "session_hash": "abc",
            "source": "claude-code",
            "project_label": "a project",
        });
        let object = base.as_object_mut().expect("an object");
        for (key, value) in extra.as_object().expect("an object") {
            object.insert(key.clone(), value.clone());
        }
        entry_from_wire(base)
    }

    // -- (b) the absent key is not a state ------------------------------

    /// **An entry with NO `eligibility` key has no eligibility question.**
    ///
    /// It is the invited contributor's row, and the field is absent
    /// entirely rather than null or `unknown`. Read as `unknown` it would
    /// put "has not been worked out" under every card they own and take
    /// their send control away with it.
    ///
    /// Goes through `serde_json::from_value`, not a struct literal:
    /// `#[serde(default)]` is what does this, and a hand-built `QueueEntry`
    /// would prove nothing about it.
    #[test]
    fn an_absent_eligibility_key_is_not_a_state() {
        let entry = wire(serde_json::json!({}));
        assert!(
            entry.eligibility.is_none(),
            "the absent key must deserialize to None"
        );
        assert!(
            view(&entry).is_none(),
            "an entry with no eligibility field must render no eligibility surface"
        );
        assert!(
            offers_send(&entry),
            "an invited contributor's row keeps the send control it always had"
        );
    }

    /// And `unknown` on the wire is a real state, told apart from the
    /// absent key by more than its own name: it draws a sentence and takes
    /// the control away, which the absent key does neither of.
    #[test]
    fn an_unknown_state_on_the_wire_is_not_the_absent_key() {
        let unknown = wire(serde_json::json!({ "eligibility": "unknown" }));
        let absent = wire(serde_json::json!({}));

        assert_eq!(unknown.eligibility.as_deref(), Some("unknown"));
        let view = view(&unknown).expect("`unknown` is a state and renders as one");
        assert!(!view.state_line.trim().is_empty());
        assert!(!offers_send(&unknown), "`unknown` offers no send control");

        assert!(super::view(&absent).is_none());
        assert_ne!(
            super::view(&absent).is_none(),
            super::view(&unknown).is_none(),
            "the absent key and `unknown` must not render the same"
        );
    }

    /// A `null` on the wire is the absent key, not a state. The daemon does
    /// not send one, and a shell that read it as a state would be inventing
    /// an answer out of a hole.
    #[test]
    fn a_null_eligibility_is_the_absent_key() {
        let entry = wire(serde_json::json!({ "eligibility": serde_json::Value::Null }));
        assert!(entry.eligibility.is_none());
        assert!(view(&entry).is_none());
        assert!(offers_send(&entry));
    }

    // -- (c) an unrecognised state borrows nothing ----------------------

    /// **A state this build has never heard of borrows no other state's
    /// sentence**, least of all an ineligibility -- which would tell a
    /// contributor their own finished work is unsendable on no evidence at
    /// all -- and offers no send control.
    #[test]
    fn an_unrecognised_state_borrows_no_other_states_sentence() {
        let known: Vec<&'static str> = [
            "eligible",
            "ineligible_permanent",
            "ineligible_configuration",
        ]
        .into_iter()
        .map(|state| {
            view(&wire(serde_json::json!({ "eligibility": state })))
                .expect("a known state renders")
                .state_line
        })
        .collect();

        let unread_line = view(&wire(serde_json::json!({ "eligibility": "unknown" })))
            .expect("`unknown` renders")
            .state_line;

        for unread in ["", "a_state_from_a_later_daemon", "ELIGIBLE", "eligible "] {
            let entry = wire(serde_json::json!({ "eligibility": unread }));
            let view = view(&entry).expect("a state that arrived renders");
            assert!(
                !view.state_line.trim().is_empty(),
                "{unread:?} must still say something"
            );
            for borrowed in &known {
                assert_ne!(
                    view.state_line, *borrowed,
                    "{unread:?} borrowed a known state's sentence"
                );
            }
            assert_eq!(
                view.state_line, unread_line,
                "{unread:?} must read as unevaluated"
            );
            assert_eq!(view.control, copy::ContributionControl::None, "{unread:?}");
            assert!(!offers_send(&entry), "{unread:?} must offer no send control");
        }
    }

    /// An unfamiliar reason renders as nothing rather than as a guess, and
    /// never takes the state sentence down with it.
    #[test]
    fn an_unrecognised_reason_says_nothing_and_costs_no_state_sentence() {
        for unread in ["", "a_reason_from_a_later_daemon", "NO_CALL"] {
            let entry = wire(serde_json::json!({
                "eligibility": "ineligible_permanent",
                "eligibility_reason": unread,
            }));
            let view = view(&entry).expect("the state still renders");
            assert_eq!(view.reason_line, "", "{unread:?} guessed at a reason");
            assert!(!view.state_line.trim().is_empty(), "{unread:?}");
        }
        // And an `eligible` row, which carries no reason at all.
        let eligible = wire(serde_json::json!({ "eligibility": "eligible" }));
        let view = view(&eligible).expect("eligible renders");
        assert_eq!(view.reason_line, "");
        assert!(!view.state_line.trim().is_empty());
        assert_eq!(view.control, copy::ContributionControl::Contribute);
    }

    // -- the contract's own tables, asked ------------------------------

    /// Every state and reason the daemon can emit reaches this shell with a
    /// sentence, read from the daemon's own pinned arrays rather than typed
    /// again here.
    #[test]
    fn every_state_and_reason_the_daemon_emits_renders() {
        use trace_commons_contributor::daemon::contribution_eligibility::{
            ALL_REASONS, ALL_STATES,
        };
        for state in ALL_STATES {
            let entry = wire(serde_json::json!({ "eligibility": state }));
            let view = view(&entry).expect("a daemon state renders");
            assert!(!view.state_line.trim().is_empty(), "{state}");
        }
        for reason in ALL_REASONS {
            let entry = wire(serde_json::json!({
                "eligibility": "ineligible_permanent",
                "eligibility_reason": reason,
            }));
            let view = view(&entry).expect("renders");
            assert!(!view.reason_line.trim().is_empty(), "{reason}");
        }
    }

    /// The send control is offered for `eligible` and for nothing else --
    /// the safety property of this surface in one assertion.
    #[test]
    fn only_an_eligible_entry_is_offered() {
        use trace_commons_contributor::daemon::contribution_eligibility::ALL_STATES;
        for state in ALL_STATES {
            let entry = wire(serde_json::json!({ "eligibility": state }));
            assert_eq!(offers_send(&entry), state == "eligible", "{state}");
        }
    }

    /// Tone follows the shared table: `Attention` for the configuration
    /// state alone, and a permanent ineligibility is never `Refused` --
    /// nothing was refused.
    #[test]
    fn the_tone_is_the_shared_tables_and_permanent_is_not_a_refusal() {
        use trace_commons_contributor::daemon::contribution_eligibility::ALL_STATES;
        for state in ALL_STATES {
            let entry = wire(serde_json::json!({ "eligibility": state }));
            let view = view(&entry).expect("renders");
            assert_eq!(
                view.tone,
                copy::eligibility_state_tone(state),
                "{state}: the tone was re-derived rather than asked for"
            );
            if state == "ineligible_configuration" {
                assert_eq!(view.tone, copy::PrivateInferenceTone::Attention, "{state}");
            } else {
                assert_ne!(view.tone, copy::PrivateInferenceTone::Attention, "{state}");
            }
        }
        let permanent = wire(serde_json::json!({ "eligibility": "ineligible_permanent" }));
        assert_ne!(
            view(&permanent).expect("renders").tone,
            copy::PrivateInferenceTone::Refused,
            "a permanent ineligibility is not a refusal: nothing was refused"
        );
    }

    // -- (a) the decision is asked for, never made here ------------------

    /// **This shell holds no eligibility table.** The sentence, the tone,
    /// the reason and the control are all read from the shared crate, which
    /// macOS and Windows reach across the C ABI. A `match` on a state name
    /// here would be a second table, and the arm it would get wrong is the
    /// unread one.
    #[test]
    fn no_eligibility_decision_is_made_in_this_shell() {
        for asked in [
            "copy::eligibility_state_line(state)",
            "copy::eligibility_state_tone(state)",
            "copy::eligibility_control(state)",
            "copy::eligibility_reason_line(",
        ] {
            assert!(SOURCE.contains(asked), "the shared crate is not asked: {asked}");
        }
        // The state and reason labels are never spelled in this shell
        // outside its own tests -- a literal here is a table forming.
        let production = SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("the module has production code above its tests");
        for spelled in [
            "\"eligible\"",
            "\"ineligible_permanent\"",
            "\"ineligible_configuration\"",
            "\"unknown\"",
            "\"no_inference_call\"",
            "\"capture_off\"",
        ] {
            assert!(
                !production.contains(spelled),
                "an eligibility label is spelled in this shell: {spelled}"
            );
        }
    }

    /// The two views ask this module rather than reading the field, and
    /// neither spells a state name.
    #[test]
    fn the_views_ask_this_module_and_spell_no_state() {
        assert!(
            QUEUE_SOURCE.contains("crate::eligibility::view(entry)"),
            "the card does not ask for the eligibility view"
        );
        assert!(
            QUEUE_SOURCE.contains("crate::eligibility::offers_send(entry)"),
            "the card does not gate its send control on the shared answer"
        );
        assert!(
            PREVIEW_SOURCE.contains("crate::eligibility::view"),
            "the sheet does not ask for the eligibility view"
        );
        for source in [QUEUE_SOURCE, PREVIEW_SOURCE] {
            let production = source
                .split("#[cfg(test)]")
                .next()
                .expect("production code above the tests");
            for spelled in [
                "\"ineligible_permanent\"",
                "\"ineligible_configuration\"",
                "\"no_inference_call\"",
            ] {
                assert!(
                    !production.contains(spelled),
                    "an eligibility label is spelled in a view: {spelled}"
                );
            }
        }
    }

    // -- the rendering rule ---------------------------------------------

    /// **Every session is shown; only the eligible ones are offered.**
    ///
    /// The card gates the SEND control and nothing else -- no arm of it
    /// skips a row, hides one, or drops `Look inside`. A contributor's own
    /// work stays on screen whatever the daemon says about sending it.
    #[test]
    fn an_ineligible_row_is_unoffered_and_never_hidden() {
        let production = QUEUE_SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests");
        // The gate wraps the send control alone. `Look inside` and `Not
        // this one` are appended unconditionally, on the lines above it.
        let gate = production
            .find("if crate::eligibility::offers_send(entry) {")
            .expect("the send control is gated");
        let body_end = production[gate..]
            .find('}')
            .expect("the gate closes")
            + gate;
        let gated = &production[gate..body_end];
        assert!(gated.contains("actions.append(&submit)"), "{gated}");
        for ungated in ["actions.append(&skip)", "actions.append(&look)"] {
            assert!(
                !gated.contains(ungated),
                "the gate swallowed a control that is not the send control: {ungated}"
            );
            assert!(
                production.contains(ungated),
                "{ungated} must still be drawn on every row"
            );
        }
        // And the row itself is built and returned whatever the state is:
        // no arm of `row` returns early on eligibility.
        let row = production
            .find("fn row(app: &Rc<App>, entry: &QueueEntry, index: usize)")
            .expect("the row builder exists");
        let row_body = &production[row..];
        let row_body = &row_body[..row_body.find("\nfn ").unwrap_or(row_body.len())];
        assert!(
            !row_body.contains("eligibility"),
            "the row builder branches on eligibility; it must build every row alike"
        );
    }
}
