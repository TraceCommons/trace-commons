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
/// `QueueEntry::eligibility` being an `Option` is what keeps them apart, so
/// the tests go through the real deserializer.
#[must_use]
pub fn view(entry: &QueueEntry) -> Option<EligibilityView> {
    view_through(entry, &SHARED)
}

/// The four lookups one entry's row is built from, behind function
/// pointers.
///
/// **A seam for the tests, not a policy choice.** `SHARED` is the only
/// table this shell ever ships, and `view` is the only caller that names
/// it. What the indirection buys is a mutation proof with teeth: a test can
/// hand `view_through` a table answering the INVERSE of the real one and
/// require the output to follow it. Code that decided anything for itself
/// would keep agreeing with the real table and fail, and -- the part a
/// same-answer fake cannot check -- it could not agree with the inverse by
/// coincidence either.
struct EligibilityTable {
    state_line: fn(&str) -> &'static str,
    state_tone: fn(&str) -> copy::PrivateInferenceTone,
    reason_line: fn(&str) -> &'static str,
    control: fn(&str) -> copy::ContributionControl,
}

/// The one table this shell ships: the shared crate's, which macOS and
/// Windows reach across the C ABI as `tc_eligibility_*`.
const SHARED: EligibilityTable = EligibilityTable {
    state_line: copy::eligibility_state_line,
    state_tone: copy::eligibility_state_tone,
    reason_line: copy::eligibility_reason_line,
    control: copy::eligibility_control,
};

/// Read one entry through a table.
///
/// Every field is a lookup. There is no `match` on a state name, no
/// `if state == ...`, and nothing here that would still be true if the
/// table said something else -- which is exactly what the inverse-table
/// test checks.
///
/// The ONE decision this shell makes is the first line, and it is not
/// about eligibility: an absent field is not a state, so there is nothing
/// to look up and no row to draw. No table can change that, and none
/// should.
fn view_through(entry: &QueueEntry, table: &EligibilityTable) -> Option<EligibilityView> {
    let state = entry.eligibility.as_deref()?;
    Some(EligibilityView {
        state_line: (table.state_line)(state),
        tone: (table.state_tone)(state),
        // An absent reason is the ordinary case on an `eligible` row --
        // there is nothing to explain -- and it answers the same empty
        // string an unfamiliar one does. Not `?`: a missing reason must not
        // take the state sentence down with it.
        reason_line: (table.reason_line)(entry.eligibility_reason.as_deref().unwrap_or_default()),
        control: (table.control)(state),
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
            assert!(
                !offers_send(&entry),
                "{unread:?} must offer no send control"
            );
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
            assert!(
                SOURCE.contains(asked),
                "the shared crate is not asked: {asked}"
            );
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

    // -- the inverse table ----------------------------------------------
    //
    // A fake that AGREES with the real table proves nothing: a shell that
    // ignored the table entirely and decided for itself would pass. These
    // fakes answer the inverse at every state, so agreeing with them is
    // only possible by actually reading them.

    /// `Contribute` for everything the real table refuses, and `None` for
    /// the one state it offers.
    fn inverse_control(state: &str) -> copy::ContributionControl {
        match copy::eligibility_control(state) {
            copy::ContributionControl::Contribute => copy::ContributionControl::None,
            copy::ContributionControl::None => copy::ContributionControl::Contribute,
        }
    }

    /// `Attention` everywhere the real table is not, and `Neutral` where it
    /// is -- so the one actionable state becomes the unremarkable one.
    fn inverse_tone(state: &str) -> copy::PrivateInferenceTone {
        match copy::eligibility_state_tone(state) {
            copy::PrivateInferenceTone::Attention => copy::PrivateInferenceTone::Neutral,
            _ => copy::PrivateInferenceTone::Attention,
        }
    }

    /// A sentence no real state has.
    fn inverse_state_line(_state: &str) -> &'static str {
        "INVERSE-STATE"
    }

    /// A non-empty answer where the real table says nothing, and the empty
    /// string where it speaks.
    fn inverse_reason_line(label: &str) -> &'static str {
        if copy::eligibility_reason_line(label).is_empty() {
            "INVERSE-REASON"
        } else {
            ""
        }
    }

    fn inverse_table() -> EligibilityTable {
        EligibilityTable {
            state_line: inverse_state_line,
            state_tone: inverse_tone,
            reason_line: inverse_reason_line,
            control: inverse_control,
        }
    }

    /// **The table is read, not re-decided.**
    ///
    /// Handed a table answering the inverse of the shared one at every
    /// state, every field of the view follows the fake. A shell that made
    /// any of these four decisions itself would keep agreeing with the real
    /// table here and fail -- and, unlike a fake that happens to match, it
    /// cannot agree with this one by coincidence.
    #[test]
    fn every_field_follows_the_table_it_is_given() {
        use trace_commons_contributor::daemon::contribution_eligibility::{
            ALL_REASONS, ALL_STATES,
        };
        let inverse = inverse_table();
        for state in ALL_STATES {
            let entry = wire(serde_json::json!({ "eligibility": state }));
            let real = view(&entry).expect("renders");
            let faked = view_through(&entry, &inverse).expect("renders");

            assert_eq!(faked.state_line, "INVERSE-STATE", "{state}: sentence");
            assert_ne!(
                faked.state_line, real.state_line,
                "{state}: the sentence ignored the table"
            );
            assert_eq!(faked.tone, inverse_tone(state), "{state}: tone");
            assert_ne!(faked.tone, real.tone, "{state}: the tone ignored the table");
            assert_eq!(faked.control, inverse_control(state), "{state}: control");
            assert_ne!(
                faked.control, real.control,
                "{state}: the control ignored the table"
            );
        }
        // And the reason line, which is keyed on the other field.
        for reason in ALL_REASONS {
            let entry = wire(serde_json::json!({
                "eligibility": "ineligible_permanent",
                "eligibility_reason": reason,
            }));
            let real = view(&entry).expect("renders");
            let faked = view_through(&entry, &inverse).expect("renders");
            assert_eq!(faked.reason_line, "", "{reason}: reason");
            assert_ne!(
                faked.reason_line, real.reason_line,
                "{reason}: the reason line ignored the table"
            );
        }
        // An unfamiliar reason: the real table says nothing, the inverse
        // speaks, and the view must say what it was told.
        let unfamiliar = wire(serde_json::json!({
            "eligibility": "ineligible_permanent",
            "eligibility_reason": "a_reason_from_a_later_daemon",
        }));
        assert_eq!(view(&unfamiliar).expect("renders").reason_line, "");
        assert_eq!(
            view_through(&unfamiliar, &inverse)
                .expect("renders")
                .reason_line,
            "INVERSE-REASON"
        );
    }

    /// The absent-key rule is this shell's own and NO table may overturn
    /// it.
    ///
    /// It is not an eligibility decision -- it is the decision that there
    /// is no eligibility question to answer -- so the inverse table, which
    /// changes every real answer, must not change this one.
    #[test]
    fn no_table_can_make_the_absent_key_into_a_state() {
        let absent = wire(serde_json::json!({}));
        assert!(view(&absent).is_none());
        assert!(
            view_through(&absent, &inverse_table()).is_none(),
            "a table was allowed to invent a row for an entry that has no eligibility question"
        );
    }

    // -- removed on the row, disarmed in the sheet ----------------------

    /// **A queue row REMOVES the control; the preview sheet DISARMS it.**
    ///
    /// The same table, rendered two ways, because the two surfaces differ
    /// in whether the control was already there. A row is drawn fresh, so
    /// nothing vanishes under anyone -- there was never a button there for
    /// this session. The sheet's `Contribute` is already on screen under
    /// the contributor's cursor while they read the sentence saying why the
    /// session cannot be sent, and a primary control that disappears
    /// mid-read is its own small confusion.
    #[test]
    fn the_row_removes_the_control_and_the_sheet_only_disarms_it() {
        let row = QUEUE_SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests");
        let sheet = PREVIEW_SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests");

        // The row: the send control is appended only inside the gate, so an
        // ineligible row never builds one.
        assert!(
            row.contains("if crate::eligibility::offers_send(entry) {"),
            "the row does not gate its send control"
        );
        assert_eq!(
            row.matches("actions.append(&submit)").count(),
            1,
            "the row appends its send control somewhere other than the gate"
        );
        assert!(
            !row.contains("submit.set_sensitive("),
            "the row disarms its send control instead of removing it; a row is drawn fresh \
             and should simply not build a button it may not offer"
        );

        // The sheet: sensitivity is the lever, and the button is never
        // removed, hidden, or unparented.
        let sync = sheet
            .find("fn sync_contribute(&self)")
            .expect("the sheet has one place its control is decided");
        let sync_end = sheet[sync..].find("\n    }").expect("it closes") + sync;
        let body = &sheet[sync..sync_end];
        assert!(
            body.contains("self.contribute.set_sensitive("),
            "the sheet does not disarm its control: {body}"
        );
        assert!(
            !body.contains("self.contribute.set_visible(")
                && !body.contains("self.contribute.unparent("),
            "the sheet hides or removes a primary control the contributor is already \
             looking at: {body}"
        );
        assert!(
            !sheet.contains("self.contribute.set_visible("),
            "the sheet hides its send control somewhere outside sync_contribute"
        );
    }

    /// Source with every run of whitespace removed.
    ///
    /// A sweep that matches raw source is a sweep rustfmt can break by
    /// wrapping an expression across lines -- which it did to
    /// `self.app.entries.borrow()` the first time this test was written.
    /// What these assertions are about is which calls the code makes, and
    /// that survives reformatting; the line breaks do not.
    fn squashed(source: &str) -> String {
        source.chars().filter(|c| !c.is_whitespace()).collect()
    }

    // -- staleness ------------------------------------------------------

    /// **The sheet's gate reads the live queue, not the copy it opened
    /// with.**
    ///
    /// `Sheet::pending` is cloned when the sheet opens. That is right for
    /// the transcript and wrong for the gate: a submit-time failure writes
    /// its reason back into the row (Decision 1), so a row can be
    /// downgraded while a sheet sits open on it. Nothing undoes the gate --
    /// the gate goes on reading a copy that stopped being true, and
    /// recomputing from it faithfully recomputes the same wrong answer
    /// forever. This is the defect macOS hit; GTK had it too.
    #[test]
    fn the_sheets_gate_reads_the_live_queue_not_its_snapshot() {
        let sheet = PREVIEW_SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests");

        // The resolver exists and looks the entry up by id.
        let resolver = sheet
            .find("fn current_live(&self)")
            .expect("the sheet resolves its entry against the live queue");
        let resolver_end = sheet[resolver..].find("\n    }").expect("it closes") + resolver;
        let body = &sheet[resolver..resolver_end];
        let squashed_body = squashed(body);
        assert!(
            squashed_body.contains("self.app.entries.borrow()"),
            "current_live does not read the live queue: {body}"
        );
        assert!(
            squashed_body.contains("e.entry_id==held.entry_id"),
            "current_live does not resolve by id: {body}"
        );

        // The gate uses it, and does NOT use the snapshot.
        let sync = sheet
            .find("fn sync_contribute(&self)")
            .expect("the gate exists");
        let sync_end = sheet[sync..].find("\n    }").expect("it closes") + sync;
        let gate = &sheet[sync..sync_end];
        let squashed_gate = squashed(gate);
        assert!(
            squashed_gate.contains("self.current_live()"),
            "the gate reads the sheet's snapshot: {gate}"
        );
        assert!(
            !squashed_gate.contains("self.current()."),
            "the gate still reads the snapshot beside the live row: {gate}"
        );
    }

    /// And the PRESS is guarded, not only the draw.
    ///
    /// Nothing tells the sheet the queue changed -- it holds no
    /// subscription -- so the gate runs only at the moments listed beside
    /// it. Between two of them a row can be downgraded under an armed
    /// button. What is offered is decided at draw time; what is SENT is
    /// decided here, and only the second is load-bearing.
    #[test]
    fn the_press_is_checked_against_the_live_queue_too() {
        let sheet = PREVIEW_SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests");
        let approve = sheet
            .find("fn approve_current(self: &Rc<Self>)")
            .expect("the approval exists");
        let approve_end = sheet[approve..].find("\n    }").expect("it closes") + approve;
        let body = &sheet[approve..approve_end];
        let squashed_body = squashed(body);
        assert!(
            squashed_body.contains("self.current_live()"),
            "the press is not re-checked against the live queue: {body}"
        );
        assert!(
            squashed_body.contains("crate::eligibility::offers_send"),
            "the press does not consult the shared answer: {body}"
        );
        // The guard precedes the call it guards.
        let guard = squashed_body
            .find("self.current_live()")
            .expect("the guard is present");
        let call = squashed_body
            .find("self.app.call(")
            .expect("the approve call is present");
        assert!(
            guard < call,
            "the guard runs after the call it is meant to prevent"
        );
    }

    /// The resolver reads the queue rather than assuming the worst: a row
    /// that has BECOME eligible arms the control, the same as one that
    /// stopped being eligible disarms it.
    ///
    /// Both directions matter. A resolver that only ever downgraded would
    /// be a different bug -- a contributor whose session became sendable
    /// would be told forever that it was not.
    #[test]
    fn the_resolver_moves_in_both_directions() {
        let downgraded = wire(serde_json::json!({ "eligibility": "ineligible_permanent" }));
        let upgraded = wire(serde_json::json!({ "eligibility": "eligible" }));
        assert!(!offers_send(&downgraded));
        assert!(offers_send(&upgraded));

        let sheet = PREVIEW_SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests");
        let resolver = sheet
            .find("fn current_live(&self)")
            .expect("the resolver exists");
        let resolver_end = sheet[resolver..].find("\n    }").expect("it closes") + resolver;
        let body = &sheet[resolver..resolver_end];
        // No eligibility reasoning in the resolver at all: it returns a row
        // and the shared table judges it. A resolver that took the worst of
        // the two rows would be deciding.
        let squashed_body = squashed(body);
        assert!(
            !squashed_body.contains("offers_send") && !squashed_body.contains("eligibility"),
            "the resolver judges the row instead of returning it: {body}"
        );
    }

    // -- both group controls, or neither --------------------------------

    /// **Neither group control is built when nothing in the group can be
    /// sent**, and both are built from the same answer.
    ///
    /// `Submit all as...` is a second route to the same `approve`; a live
    /// one beside an absent button would offer exactly what the missing
    /// button withheld.
    ///
    /// Not built rather than built-and-disabled. The folder now SAYS what
    /// it is withholding, so a dead control has nothing left to
    /// communicate.
    #[test]
    fn neither_group_control_is_built_when_nothing_can_be_sent() {
        let row = squashed(
            QUEUE_SOURCE
                .split("#[cfg(test)]")
                .next()
                .expect("production code above the tests"),
        );
        for control in ["submit_all", "submit_all_as"] {
            assert!(
                row.contains(&format!("ifsendable{{bar.append(&{control});}}")),
                "{control} is not gated on the group having something to send"
            );
            assert!(
                !row.contains(&format!("{control}.set_sensitive(")),
                "{control} is drawn and disabled; it should not be drawn at all"
            );
        }
        // Both read ONE computed answer. Two computations are two chances
        // to disagree.
        assert_eq!(
            row.matches("letsendable=!group.eligible.is_empty()")
                .count(),
            1,
            "the two controls do not share one computed answer"
        );
    }

    /// The folder says what its submit will leave behind, in the shared
    /// crate's words, before the press.
    #[test]
    fn the_folder_says_what_its_submit_leaves_behind() {
        let row = squashed(
            QUEUE_SOURCE
                .split("#[cfg(test)]")
                .next()
                .expect("production code above the tests"),
        );
        assert!(
            row.contains("copy::group_withheld_line("),
            "the folder authors its own withheld sentence instead of asking for one"
        );
        assert!(
            row.contains(".saturating_sub("),
            "the withheld count can underflow"
        );
        // Zero draws nothing, which is also the invited contributor's whole
        // case: no eligibility field, nothing withheld, no line.
        assert_eq!(copy::group_withheld_line(0), "");
        assert!(!copy::group_withheld_line(1).is_empty());
        // It counts and does not explain -- the reason lives on the row.
        let line = copy::group_withheld_line(3).to_lowercase();
        assert!(line.contains('3'), "{line}");
        for reason_word in ["attested", "eligib", "setting", "capture"] {
            assert!(
                !line.contains(reason_word),
                "the withheld line reaches for a reason: {line}"
            );
        }
    }

    /// **A bulk control may not reach `approve` around `submit_group`.**
    ///
    /// The guarantee is unchanged and only the enforcing layer moved: a
    /// project-wide `approve` now admits eligible entries alone, so
    /// `submit_group` makes the one call and the daemon does the filtering.
    /// What must stay true is that no second bulk path builds that call
    /// itself.
    #[test]
    fn a_group_submit_never_sends_by_project_id_directly() {
        let production = QUEUE_SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests");
        let group = production
            .find("fn submit_group(")
            .expect("the group submit exists");
        let group_end = production[group..]
            .find("\n/// The head of a folder")
            .expect("submit_group ends")
            + group;
        let inside = &production[group..group_end];
        assert!(
            inside.contains("ApproveTarget::Project"),
            "submit_group no longer builds the project-wide call"
        );

        // Everything but `submit_group` and `approve_params`. The latter is
        // the enum's serializer -- it TRANSLATES the variant it is handed
        // and chooses nothing -- so its arm is not a call site.
        let serializer = production
            .find("pub(crate) fn approve_params(")
            .expect("the serializer exists");
        let mut outside = production[..group].to_string() + &production[group_end..];
        let serializer_arm = "ApproveTarget::Project(key) => serde_json::json!";
        assert!(
            production[serializer..].contains(serializer_arm),
            "the serializer no longer holds the project arm; this exclusion is now hiding \
             a real call site"
        );
        outside = outside.replace(serializer_arm, "");
        assert!(
            !outside.contains("ApproveTarget::Project"),
            "a bulk control builds the project-wide call outside submit_group, which is \
             the one place the rule is enforced"
        );
        assert_eq!(
            production.matches("submit_group(").count(),
            3,
            "expected the definition plus both bulk handlers to reach submit_group"
        );
        // The fan-out went with the constraint that forced it.
        for removed in ["submit_each_and_toast", "struct FanOut"] {
            assert!(
                !production.contains(removed),
                "{removed} outlived the daemon-side filter that replaced it"
            );
        }
    }

    /// The split itself: eligible rows are collected, ineligible ones are
    /// counted, and rows from another project are neither.
    ///
    /// Exercised through the same deserializer the daemon feeds, so an
    /// entry with no eligibility field counts as sendable -- the invited
    /// contributor's group takes the unchanged one-call arm.
    #[test]
    fn a_group_splits_its_rows_on_what_may_be_sent() {
        let rows = [
            (Some("eligible"), "p1", true),
            (Some("ineligible_permanent"), "p1", false),
            (Some("ineligible_configuration"), "p1", false),
            (Some("unknown"), "p1", false),
            (Some("a_state_from_a_later_daemon"), "p1", false),
            (None, "p1", true),
        ];
        let mut eligible = 0;
        let mut ineligible = 0;
        for (state, project, expected) in rows {
            let mut json = serde_json::json!({
                "entry_id": "e",
                "state": "pending",
                "project_id": project,
            });
            if let Some(state) = state {
                json["eligibility"] = serde_json::Value::from(state);
            }
            let entry = entry_from_wire(json);
            assert_eq!(offers_send(&entry), expected, "{state:?}");
            if expected {
                eligible += 1;
            } else {
                ineligible += 1;
            }
        }
        assert_eq!((eligible, ineligible), (2, 4));
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
        let body_end = production[gate..].find('}').expect("the gate closes") + gate;
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
