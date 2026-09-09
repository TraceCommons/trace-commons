//! Whether a queue row's session carries proof of the model call that made
//! it.
//!
//! **This is not eligibility, and the difference is a presence rule.**
//! [`crate::eligibility`] answers *may I send this?* and correctly says
//! nothing at all when nobody is asking -- an invited contributor was never
//! admitted on evidence, so there is no admission question on their rows.
//! The mark answers *does this session carry a checkable copy of its last
//! model call?*, which is a fact about the trace rather than a permission,
//! and every contributor has that question all of the time. So the wire
//! carries `attestation` on EVERY entry and this module renders it on every
//! row, the invited contributor's included.
//!
//! Nothing here decides anything. The sentence, the tone and the reason all
//! come from `trace_commons_contributor::private_inference_copy`, which
//! macOS and Windows reach across the C ABI as `tc_attestation_*`. This
//! module reads two fields off the entry and hands the labels to those three
//! functions. There is no `match` on a mark name in this shell, and there
//! must not be: a second table is a second chance to tell a contributor
//! their work carries no proof when it does.
//!
//! **There is no control here.** The mark describes the trace and offers
//! nothing to press; whether a row may send stays [`crate::eligibility`]'s
//! answer. That asymmetry is why [`AttestationView`] has three fields where
//! `EligibilityView` has four.

use crate::copy;
use crate::model::QueueEntry;

/// What one row says about the proof its session carries.
///
/// Built as a unit from a single label so the sentence and the colour cannot
/// answer differently.
///
/// Not an `Option`, unlike `EligibilityView`: there is always an answer, and
/// a build that cannot read the label still has the `unknown` sentence to
/// say so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttestationView {
    /// The mark sentence. Always non-empty -- an unread mark has its own.
    pub state_line: &'static str,
    /// How firmly the sentence reads. `Attention` is the one mark with
    /// something to do about it; a permanently unattested session is
    /// deliberately not `Refused`, because nothing was refused and the
    /// contributor did nothing wrong.
    pub tone: copy::PrivateInferenceTone,
    /// The reason detail, or the empty string, which this shell draws as no
    /// line at all rather than as a guess.
    pub reason_line: &'static str,
}

/// The attestation surface for one entry. Never absent.
///
/// **The reason branches on ITS OWN key's presence, never on the mark**, and
/// this is the rule most easily got wrong. `attested` carries no reason;
/// both unattested marks always carry one; `unknown` carries one only
/// sometimes -- absent when the row was never evaluated, present
/// (`receipt_unavailable`) when a send was refused because the receipt
/// service could not be reached, which is a retraction rather than a
/// refusal. Suppressing the reason on `unknown` would drop the only signal
/// telling a contributor it may work later.
#[must_use]
pub fn view(entry: &QueueEntry) -> AttestationView {
    view_through(entry, &SHARED)
}

/// The three lookups one row's mark is built from, behind function
/// pointers.
///
/// **A seam for the tests, not a policy choice**, for the same reason
/// `EligibilityTable` is one: a test can hand `view_through` a table
/// answering the INVERSE of the real one and require the output to follow
/// it. Code that decided anything for itself would keep agreeing with the
/// real table, and could not agree with the inverse by coincidence.
struct AttestationTable {
    state_line: fn(&str) -> &'static str,
    state_tone: fn(&str) -> copy::PrivateInferenceTone,
    reason_line: fn(&str) -> &'static str,
}

/// The one table this shell ships: the shared crate's, which macOS and
/// Windows reach across the C ABI as `tc_attestation_*`.
const SHARED: AttestationTable = AttestationTable {
    state_line: copy::attestation_state_line,
    state_tone: copy::attestation_state_tone,
    reason_line: copy::attestation_reason_line,
};

/// Read one entry through a table.
///
/// Every field is a lookup. There is no `match` on a mark name and nothing
/// here that would still be true if the table said something else.
///
/// An absent `attestation` is handed on as the empty string rather than
/// branched on: a daemon predating the field answers the `unknown` sentence,
/// which is what a build with no evidence should say, and never an
/// unattested mark.
fn view_through(entry: &QueueEntry, table: &AttestationTable) -> AttestationView {
    let mark = entry.attestation.as_deref().unwrap_or_default();
    AttestationView {
        state_line: (table.state_line)(mark),
        tone: (table.state_tone)(mark),
        // `attestation_reason`, never `eligibility_reason`. The two take the
        // same thirteen labels and answer different sentences, so reading
        // the wrong field is a mistake no type can catch.
        reason_line: (table.reason_line)(entry.attestation_reason.as_deref().unwrap_or_default()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The source of this module and of the view that renders it.
    ///
    /// Read at compile time so a sweep cannot pass over a file that moved.
    const SOURCE: &str = include_str!("attestation.rs");
    const QUEUE_SOURCE: &str = include_str!("ui/queue.rs");

    /// The marks the daemon emits.
    const MARKS: [&str; 4] = [
        "attested",
        "unattested_permanent",
        "unattested_configuration",
        "unknown",
    ];

    /// The thirteen reason labels, shared with eligibility.
    const REASONS: [&str; 13] = [
        "no_inference_call",
        "capture_off",
        "digest_absent",
        "upstream_id_absent",
        "digest_mismatch",
        "reference_malformed",
        "bodies_unreadable",
        "body_not_utf8",
        "body_too_large",
        "evidence_capture_off",
        "marker_absent",
        "request_malformed",
        "receipt_unavailable",
    ];

    /// Build an entry the way the daemon does: through the real
    /// deserializer, from real wire JSON.
    ///
    /// Never a struct literal. `#[serde(default)]` on an `Option` IS what
    /// tells an absent key apart from a present one, and a hand-built value
    /// tests the mechanism's absence.
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
        serde_json::from_value(base).expect("the daemon's shape deserializes")
    }

    // -- (a) the field decodes at all -----------------------------------

    /// **Both fields come off the wire.** Nothing read them before this
    /// slice, so this is the test that fails first without it.
    #[test]
    fn the_wire_fields_decode() {
        let entry = wire(serde_json::json!({
            "attestation": "unknown",
            "attestation_reason": "receipt_unavailable",
        }));
        assert_eq!(entry.attestation.as_deref(), Some("unknown"));
        assert_eq!(
            entry.attestation_reason.as_deref(),
            Some("receipt_unavailable")
        );
    }

    /// A daemon predating the fields degrades to the `unknown` sentence and
    /// **never to an unattested mark**: a build with no label has no
    /// evidence about anybody's session.
    #[test]
    fn an_absent_mark_reads_as_unknown_and_never_as_unattested() {
        let entry = wire(serde_json::json!({}));
        assert!(entry.attestation.is_none());
        let absent = view(&entry);
        assert_eq!(
            absent.state_line,
            view(&wire(serde_json::json!({ "attestation": "unknown" }))).state_line,
            "an absent mark says what `unknown` says"
        );
        for unattested in ["unattested_permanent", "unattested_configuration"] {
            assert_ne!(
                absent.state_line,
                view(&wire(serde_json::json!({ "attestation": unattested }))).state_line,
                "an absent mark must never borrow an unattested sentence"
            );
        }
        assert!(absent.reason_line.is_empty());
    }

    // -- (b) the case the feature exists for ----------------------------

    /// **An invited contributor's row carries the mark.** They have no
    /// eligibility question -- `eligibility::view` answers `None` -- and the
    /// mark is stated anyway, because it is a fact about the trace and not a
    /// permission. A shell that copied eligibility's absent-key rule would
    /// withhold from exactly the people this surface was added for.
    #[test]
    fn the_mark_renders_for_an_invited_contributor() {
        let entry = wire(serde_json::json!({
            "attestation": "unattested_permanent",
            "attestation_reason": "capture_off",
        }));
        assert!(
            entry.eligibility.is_none(),
            "an invited contributor's row carries no eligibility field"
        );
        assert!(
            crate::eligibility::view(&entry).is_none(),
            "and so draws no eligibility surface"
        );
        let mark = view(&entry);
        assert!(!mark.state_line.trim().is_empty(), "the mark is stated");
        assert!(!mark.reason_line.trim().is_empty(), "with its reason");
        assert!(
            crate::eligibility::offers_send(&entry),
            "and the send control they always had is untouched"
        );
    }

    // -- (c) four marks, four answers -----------------------------------

    /// Every mark the daemon emits renders a sentence of its own, and the
    /// tones are the shared table's.
    #[test]
    fn every_mark_renders_its_own_sentence_and_tone() {
        let mut seen: Vec<&'static str> = Vec::new();
        for mark in MARKS {
            let view = view(&wire(serde_json::json!({ "attestation": mark })));
            assert!(
                !view.state_line.trim().is_empty(),
                "{mark} renders a sentence"
            );
            assert!(
                !seen.contains(&view.state_line),
                "{mark} borrows another mark's sentence"
            );
            seen.push(view.state_line);
        }

        let tone = |mark: &str| view(&wire(serde_json::json!({ "attestation": mark }))).tone;
        assert_eq!(tone("attested"), copy::PrivateInferenceTone::Clear);
        assert_eq!(
            tone("unattested_configuration"),
            copy::PrivateInferenceTone::Attention,
            "the one mark with something to do about it"
        );
        assert_eq!(
            tone("unattested_permanent"),
            copy::PrivateInferenceTone::Neutral,
            "nothing was refused: ordinary work done when nothing kept copies"
        );
        assert_eq!(tone("unknown"), copy::PrivateInferenceTone::Neutral);
        assert_ne!(
            tone("unattested_permanent"),
            copy::PrivateInferenceTone::Refused,
            "painting a contributor's history as a failure is not this app's call"
        );
    }

    /// **A mark this build has never heard of degrades to `unknown`**, never
    /// to an unattested one, and reads `Neutral`.
    #[test]
    fn an_unrecognised_mark_degrades_to_unknown() {
        let unknown = view(&wire(serde_json::json!({ "attestation": "unknown" })));
        for strange in ["", "attested_maybe", "ATTESTED", "unattested", "42"] {
            let view = view(&wire(serde_json::json!({ "attestation": strange })));
            assert_eq!(
                view.state_line, unknown.state_line,
                "an unread mark says what `unknown` says: {strange}"
            );
            assert_eq!(view.tone, copy::PrivateInferenceTone::Neutral);
        }
    }

    // -- (d) the reason follows its own key -----------------------------

    /// **`unknown` shows its reason when the key is there and draws no
    /// reason line when it is not.** Both rows carry the same mark, so a
    /// shell that branched on the mark would answer them alike and drop the
    /// only signal saying a receipt may be fetchable later.
    #[test]
    fn unknown_shows_a_reason_only_when_the_key_is_there() {
        let never_evaluated = view(&wire(serde_json::json!({ "attestation": "unknown" })));
        assert_eq!(
            never_evaluated.reason_line, "",
            "a row nobody evaluated draws no reason line"
        );

        let retracted = view(&wire(serde_json::json!({
            "attestation": "unknown",
            "attestation_reason": "receipt_unavailable",
        })));
        assert!(
            !retracted.reason_line.trim().is_empty(),
            "a refused send with an unreachable receipt says so"
        );
        assert_eq!(
            retracted.state_line, never_evaluated.state_line,
            "the two differ in the reason alone, which is the point"
        );
    }

    /// `attested` carries no reason, and both unattested marks carry theirs.
    #[test]
    fn the_reason_line_is_read_off_its_own_key_not_off_the_mark() {
        assert_eq!(
            view(&wire(serde_json::json!({ "attestation": "attested" }))).reason_line,
            "",
            "there is nothing to explain about an attested session"
        );
        for (mark, reason) in [
            ("unattested_permanent", "no_inference_call"),
            ("unattested_configuration", "evidence_capture_off"),
        ] {
            let view = view(&wire(serde_json::json!({
                "attestation": mark,
                "attestation_reason": reason,
            })));
            assert!(
                !view.reason_line.trim().is_empty(),
                "{mark} explains itself"
            );
        }
    }

    /// The reason comes from `attestation_reason`. An `eligibility_reason`
    /// on the same row is a different question's answer and must not leak
    /// into this line.
    #[test]
    fn the_eligibility_reason_is_not_borrowed() {
        let entry = wire(serde_json::json!({
            "attestation": "unknown",
            "eligibility": "ineligible_permanent",
            "eligibility_reason": "no_inference_call",
        }));
        assert_eq!(
            view(&entry).reason_line,
            "",
            "no `attestation_reason` key means no reason line, whatever eligibility said"
        );
    }

    /// **Shared labels, separate tables.** All thirteen render, and none of
    /// them tells the contributor their session cannot be sent -- which is
    /// what the eligibility sentences say, and which is false for an invited
    /// contributor whose session sends perfectly well and merely arrives
    /// without a copy of its call.
    ///
    /// A handful of the pairs are word-for-word identical, because a few
    /// reasons carry no sendability clause to drop; that is the shared
    /// crate's decision and not this shell's. What must hold here is that
    /// the tables are two, that they disagree somewhere, and that no
    /// refusal wording reaches this surface.
    #[test]
    fn no_reason_sentence_claims_the_session_cannot_be_sent() {
        let mut differs = 0;
        for reason in REASONS {
            let mark = view(&wire(serde_json::json!({
                "attestation": "unattested_permanent",
                "attestation_reason": reason,
            })));
            assert!(
                !mark.reason_line.trim().is_empty(),
                "{reason} renders as an attestation reason"
            );
            for refusal in ["cannot be sent", "unsendable", "will be refused"] {
                assert!(
                    !mark.reason_line.contains(refusal),
                    "{reason} carries eligibility's refusal wording: {refusal}"
                );
            }
            if mark.reason_line != copy::eligibility_reason_line(reason) {
                differs += 1;
            }
        }
        assert!(
            differs > 0,
            "the two reason tables must not be one table under two names"
        );
    }

    /// An unfamiliar reason says nothing and costs the mark sentence
    /// nothing.
    #[test]
    fn an_unrecognised_reason_says_nothing_and_costs_no_mark_sentence() {
        let entry = wire(serde_json::json!({
            "attestation": "unattested_configuration",
            "attestation_reason": "a_reason_from_the_future",
        }));
        let view = view(&entry);
        assert_eq!(
            view.reason_line, "",
            "a reason nobody established is not shown"
        );
        assert_eq!(
            view.state_line,
            super::view(&wire(
                serde_json::json!({ "attestation": "unattested_configuration" })
            ))
            .state_line,
            "and it does not take the mark sentence down with it"
        );
        assert_eq!(view.tone, copy::PrivateInferenceTone::Attention);
    }

    // -- (e) nothing is decided here ------------------------------------

    /// The production half of this module names no mark and no reason.
    #[test]
    fn no_attestation_decision_is_made_in_this_shell() {
        let production = SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("the module has a production half");
        for label in MARKS.iter().chain(REASONS.iter()) {
            assert!(
                !production.contains(&format!("\"{label}\"")),
                "the label is branched on in this shell: {label}"
            );
        }
        assert!(
            !production.contains("match mark"),
            "the mark must be looked up, never matched on"
        );
    }

    /// The queue row asks this module and spells no mark of its own -- and
    /// **draws no control from it**. The mark describes the trace; there is
    /// nothing to press.
    #[test]
    fn the_row_renders_the_mark_and_offers_no_control() {
        // Every row builds one of these, unconditionally -- which is what
        // makes "on every queue row" true rather than a claim about the
        // rows somebody happened to look at.
        let row_start = QUEUE_SOURCE
            .find("fn row(app: &Rc<App>, entry: &QueueEntry, index: usize)")
            .expect("the row builder exists");
        let row_body = &QUEUE_SOURCE[row_start..];
        let row_body = &row_body[..row_body.find("\nfn ").unwrap_or(row_body.len())];
        assert!(
            row_body.contains("manifest_block(app, entry, index, state)"),
            "every row must build the manifest block the mark lives in"
        );

        let start = QUEUE_SOURCE
            .find("fn manifest_block(")
            .expect("the manifest block exists");
        let body = &QUEUE_SOURCE[start..];
        let body = &body[..body.find("\nfn ").unwrap_or(body.len())];
        assert!(
            body.contains("crate::attestation::view(entry)"),
            "the row must ask this module for the mark"
        );
        // Unconditionally: no `if let`, no `Some(...)`, nothing that could
        // leave a row unmarked. The eligibility line beside it is the one
        // that is allowed to be absent.
        assert!(
            !body.contains("if let Some(mark)"),
            "the mark is always present and must not be drawn behind an option"
        );
        for label in MARKS.iter().chain(REASONS.iter()) {
            assert!(
                !body.contains(&format!("\"{label}\"")),
                "the row branches on a label of its own: {label}"
            );
        }

        let mark_start = body
            .find("crate::attestation::view(entry)")
            .expect("the mark is rendered");
        let mark_block = &body[mark_start..];
        let mark_block = &mark_block[..mark_block
            .find("block.append(&facts)")
            .unwrap_or(mark_block.len())];
        assert!(
            !mark_block.contains("Button"),
            "the mark offers nothing to press"
        );
    }

    // -- (f) the inverse table ------------------------------------------

    fn inverse_state_line(_mark: &str) -> &'static str {
        "INVERSE STATE"
    }

    fn inverse_tone(_mark: &str) -> copy::PrivateInferenceTone {
        copy::PrivateInferenceTone::Refused
    }

    /// The inverse of the real reason table: it answers a sentence exactly
    /// where the real one answers the empty string, and the empty string
    /// where the real one answers a sentence.
    fn inverse_reason_line(label: &str) -> &'static str {
        if copy::attestation_reason_line(label).is_empty() {
            "INVERSE REASON"
        } else {
            ""
        }
    }

    fn inverse_table() -> AttestationTable {
        AttestationTable {
            state_line: inverse_state_line,
            state_tone: inverse_tone,
            reason_line: inverse_reason_line,
        }
    }

    /// **Every field follows the table it is given.** A hand-written
    /// expectation that happens to match today is a second implementation
    /// wearing a test's clothes; only a table answering the inverse can say
    /// the lookup is what produced the answer.
    #[test]
    fn every_field_follows_the_table_it_is_given() {
        let table = inverse_table();
        for mark in MARKS.iter().chain(["a_mark_from_the_future"].iter()) {
            let entry = wire(serde_json::json!({ "attestation": mark }));
            let view = view_through(&entry, &table);
            assert_eq!(view.state_line, "INVERSE STATE", "{mark}");
            assert_eq!(view.tone, copy::PrivateInferenceTone::Refused, "{mark}");
            assert_ne!(
                view.state_line,
                super::view(&entry).state_line,
                "the inverse must not agree with the real table: {mark}"
            );
        }

        // A reason the real table knows: the inverse blanks it.
        let known = wire(serde_json::json!({
            "attestation": "unattested_permanent",
            "attestation_reason": "no_inference_call",
        }));
        assert_eq!(view_through(&known, &table).reason_line, "");
        assert!(!super::view(&known).reason_line.is_empty());

        // A reason it does not: the inverse speaks where the real one is
        // silent, which proves the empty string is a lookup and not a
        // hard-coded blank.
        let strange = wire(serde_json::json!({
            "attestation": "unknown",
            "attestation_reason": "a_reason_from_the_future",
        }));
        assert_eq!(view_through(&strange, &table).reason_line, "INVERSE REASON");
        assert_eq!(super::view(&strange).reason_line, "");
    }

    /// No table can make an absent mark into anything but a lookup of the
    /// empty string, and no table is consulted with the wrong key: an entry
    /// whose only reason is an `eligibility_reason` gets the inverse's
    /// answer for the empty label, not for `no_call`.
    #[test]
    fn the_reason_key_is_the_attestation_one_under_any_table() {
        let table = inverse_table();
        let entry = wire(serde_json::json!({
            "attestation": "unknown",
            "eligibility_reason": "no_inference_call",
        }));
        assert_eq!(
            view_through(&entry, &table).reason_line,
            "INVERSE REASON",
            "the empty label is what was looked up, so `eligibility_reason` was not read"
        );
    }
}
