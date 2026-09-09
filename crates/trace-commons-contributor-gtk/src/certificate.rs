//! Which queue rows a witness certificate is held for, and how that reads.
//!
//! **One fact, two readings.** A contributor without an invite is being told
//! the session is a candidate for submission; one with an invite is being
//! told it is cryptographically attested. The underlying fact is identical
//! -- the daemon holds a witness certificate over the bytes this row was
//! pinned to, which is true after either witness route -- so the wire says
//! it once, on `holds_certificate`, and the reading follows
//! `admission_evidence_required`, which this shell already holds.
//!
//! **That flag is passed through, never negated.** It is true for a
//! contributor who signed up through NEAR -- no invite, building a case for
//! submission -- and false for one enrolled on an invite. So true selects the
//! CANDIDATE reading, which looks backwards until you know which way the flag
//! points. Every `!` written here would be a chance to swap the two readings.
//!
//! **This is not [`crate::attestation`], and the difference is the whole
//! point.** The mark answers *does this session carry a checkable copy of
//! the model call that produced it?* This answers *is a certificate held
//! over what was reviewed?* Holds-a-certificate, is-attestable and
//! was-attested are three different facts about a session, and reading the
//! mark to build this list would be reading the wrong one.
//!
//! Nothing here decides anything. Both sentences and the pick between them
//! come from `trace_commons_contributor::private_inference_copy`, which
//! macOS and Windows reach across the C ABI. There is no branch on the flag
//! in this shell and there must not be: three shells choosing for themselves
//! is three chances to promise an uninvited contributor an attestation that
//! has not happened.

use crate::copy;
use crate::model::QueueEntry;

/// What one row of the certificate-held list says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CertificateView {
    /// The row sentence, in the reading this contributor gets.
    pub line: &'static str,
}

/// The rows a certificate is held for, in the order the queue gave them.
///
/// Filtering rather than annotating: the list *is* the held ones. A row with
/// no certificate is absent, not present and qualified.
///
/// Takes no invite status, deliberately. Which rows are in the list is the
/// same question for both audiences -- only the words differ. A filter that
/// accepted the reading would be a filter that could one day depend on it,
/// and then an invited and an uninvited contributor would be looking at
/// different sessions rather than the same ones described differently.
#[must_use]
pub fn held(entries: &[QueueEntry]) -> Vec<&QueueEntry> {
    entries.iter().filter(|e| e.holds_certificate).collect()
}

/// The heading, in this contributor's reading.
#[must_use]
pub fn title(evidence_admitted: bool) -> &'static str {
    view_through(evidence_admitted, &SHARED).title
}

/// What the list says with nothing in it.
///
/// Never the empty string. On this shell every queue field decodes with
/// `#[serde(default)]`, so a `holds_certificate` that did not arrive renders
/// an empty list rather than an error -- and a blank panel is
/// indistinguishable from having no certificates.
#[must_use]
pub fn empty() -> &'static str {
    view_through(false, &SHARED).empty
}

/// One row's sentence.
#[must_use]
pub fn view(evidence_admitted: bool) -> CertificateView {
    CertificateView {
        line: view_through(evidence_admitted, &SHARED).row,
    }
}

/// The three lookups this list is built from, behind function pointers.
///
/// **A seam for the tests, not a policy choice**, exactly as
/// `AttestationTable` is one: a test can hand `view_through` a table
/// answering the INVERSE of the real one and require the output to follow
/// it. Code that decided anything for itself would keep agreeing with the
/// real table and could not agree with an inverted one by coincidence.
struct CertificateTable {
    row: fn(bool) -> &'static str,
    title: fn(bool) -> &'static str,
    empty: fn() -> &'static str,
}

/// The one table this shell ships: the shared crate's.
const SHARED: CertificateTable = CertificateTable {
    row: copy::certificate_row_line,
    title: copy::certificate_list_title,
    empty: copy::certificate_list_empty,
};

/// What one table answers for one reading.
struct Resolved {
    row: &'static str,
    title: &'static str,
    empty: &'static str,
}

/// Read one reading through a table. Every field is a lookup.
fn view_through(evidence_admitted: bool, table: &CertificateTable) -> Resolved {
    Resolved {
        row: (table.row)(evidence_admitted),
        title: (table.title)(evidence_admitted),
        empty: (table.empty)(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The queue draws this section, asks this module for it, and spells no
    /// sentence of its own.
    ///
    /// The same source guard `eligibility.rs` keeps over the same file, for
    /// the same reason: a card that reached past this module would be a
    /// second table, and a second table is a second chance to tell a
    /// contributor with no invite that their session is attested.
    ///
    /// **The empty state is checked here too.** A filtered section that
    /// renders nothing when nothing matches is indistinguishable from a
    /// section that failed to load -- and on this shell a `holds_certificate`
    /// that never arrived decodes to `false` on every row, which produces
    /// exactly that. The contributor has to be told the category exists.
    #[test]
    fn the_queue_asks_this_module_for_the_section_and_writes_no_sentence() {
        const QUEUE_SOURCE: &str = include_str!("ui/queue.rs");

        for call in [
            "crate::certificate::held(",
            "crate::certificate::title(",
            "crate::certificate::empty(",
            "crate::certificate::view(",
        ] {
            assert!(
                QUEUE_SOURCE.contains(call),
                "the queue does not ask for {call}"
            );
        }

        // Prose is stripped before the sweep. A guard that fails source for
        // naming a sentence in a comment teaches the next reader to delete
        // the comment, and the comments here are what say why the section is
        // drawn even when empty.
        let production: String = QUEUE_SOURCE
            .split("#[cfg(test)]")
            .next()
            .expect("production code above the tests")
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for authored in ["put forward", "signed proof", "witness certificate"] {
            assert!(
                !production.contains(authored),
                "{authored:?} is written in the queue rather than coming from the shared copy"
            );
        }
    }

    fn entry(id: &str, holds: bool) -> QueueEntry {
        QueueEntry {
            entry_id: id.into(),
            holds_certificate: holds,
            ..Default::default()
        }
    }

    /// The list is the held ones. Both directions, because a filter that
    /// only ever returns everything passes any test that checks the held
    /// rows are present.
    #[test]
    fn the_list_is_exactly_the_rows_a_certificate_is_held_for() {
        let entries = vec![entry("a", true), entry("b", false), entry("c", true)];
        let ids: Vec<&str> = held(&entries).iter().map(|e| e.entry_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "c"]);
        assert!(held(&[entry("b", false)]).is_empty());
    }

    /// Nothing in this shell decides which reading a contributor gets.
    ///
    /// The table is inverted -- candidate and attested swapped. Code that
    /// branched on `invited` itself would keep agreeing with the real
    /// sentences and could not follow an inverted table by coincidence.
    #[test]
    fn every_sentence_follows_the_table_and_not_this_shell() {
        const INVERSE: CertificateTable = CertificateTable {
            row: |evidence_admitted| {
                if evidence_admitted {
                    "ROW-WHEN-FLAG-SET"
                } else {
                    "ROW-WHEN-FLAG-CLEAR"
                }
            },
            title: |evidence_admitted| {
                if evidence_admitted {
                    "TITLE-WHEN-FLAG-SET"
                } else {
                    "TITLE-WHEN-FLAG-CLEAR"
                }
            },
            empty: || "EMPTY",
        };
        assert_eq!(view_through(true, &INVERSE).row, "ROW-WHEN-FLAG-SET");
        assert_eq!(view_through(false, &INVERSE).row, "ROW-WHEN-FLAG-CLEAR");
        assert_eq!(view_through(true, &INVERSE).title, "TITLE-WHEN-FLAG-SET");
        assert_eq!(view_through(false, &INVERSE).empty, "EMPTY");
    }

    /// The two readings reach the surface, and differ.
    #[test]
    fn the_two_readings_are_different_sentences() {
        assert_ne!(view(true).line, view(false).line);
        assert_ne!(title(true), title(false));
        assert!(
            !empty().is_empty(),
            "an empty list would say nothing at all"
        );
    }

    /// **The key must actually decode.**
    ///
    /// Every field on `QueueEntry` is `#[serde(default)]`, so a daemon that
    /// stopped sending `holds_certificate`, or a key renamed on one side
    /// only, yields `false` on every row -- an empty list, which is exactly
    /// what a contributor with no certificates sees. Nothing else in this
    /// shell would fail. This pins that a row carrying the key differs
    /// observably from one without it.
    #[test]
    fn a_decoded_entry_carrying_the_key_differs_from_one_without_it() {
        let with: QueueEntry = serde_json::from_str(r#"{"entry_id":"a","holds_certificate":true}"#)
            .expect("an entry carrying the key decodes");
        let without: QueueEntry =
            serde_json::from_str(r#"{"entry_id":"a"}"#).expect("an older entry still decodes");

        assert!(with.holds_certificate, "the key decoded into nothing");
        assert!(!without.holds_certificate);
        assert_eq!(held(std::slice::from_ref(&with)).len(), 1);
        assert!(
            held(std::slice::from_ref(&without)).is_empty(),
            "an entry without the key must not appear in the list"
        );
    }
}
