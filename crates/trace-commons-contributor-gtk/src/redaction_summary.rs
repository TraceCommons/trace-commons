//! The preview sheet's removed-summary panel: one row per redaction family,
//! what that family IS, and how much of it left.
//!
//! Marking placeholders in the transcript answers *where*. It does not answer
//! "so I can right away see what doesn't go", because collecting the marks
//! means scrolling the whole body. This is the at-a-glance half, and it is
//! also the surface where `residual_secret_at` is finally stated correctly
//! rather than backwards.
//!
//! Three rules, and all three are about a vocabulary that is OPEN. The
//! redactor generates `secret:{pattern}`, `privacy_filter:{label}`,
//! `tool_sensitive_field:{action}` and `residual_secret_at:{schema_path}` at
//! redaction time, so no shell can hold a complete table of the labels:
//!
//! 1. Group by family -- the part before the first `:` -- so nine secret
//!    patterns are one `secret` row with its sub-labels on a detail line.
//! 2. Keep an unrecognised family, with a neutral description. Dropping one
//!    because this build has no words for it would understate what happened,
//!    which is the one direction this panel must not fail in.
//! 3. Never carry matched text. A sub-label is a schema-shaped identifier by
//!    construction -- the same property `log_residual_secret_locations`
//!    relies on -- and that is the only reason it is safe to render.

use crate::redaction_labels::{RESIDUAL_PREFIX, family, is_removal};
use std::collections::BTreeMap;

/// One family's row.
pub struct Row {
    /// The raw family, as the redactor spelled it. The key, never the text.
    pub family: String,
    /// The family as a person reads it.
    pub display: String,
    pub description: &'static str,
    pub occurrences: u32,
    /// Distinct values, summed over the family's sub-labels. Zero when the
    /// daemon reported none, which is how a shell tells "one value" apart
    /// from "not measured".
    pub distinct: u32,
    /// The sub-labels this row folded in, in a stable order, and empty when
    /// the family arrived bare. Kinds, never values.
    pub detail: Vec<String>,
}

/// What a family IS, or the neutral fallback.
///
/// Matching rather than a table lookup so an unrecognised family cannot be a
/// missing key: every input has an answer, and the answer for an unknown one
/// still puts it on screen.
fn describe(family: &str) -> &'static str {
    match family {
        "local_path" => crate::copy::REDACTION_CATEGORY_LOCAL_PATH,
        "secret" => crate::copy::REDACTION_CATEGORY_SECRET,
        "privacy_filter" => crate::copy::REDACTION_CATEGORY_PRIVACY_FILTER,
        "sensitive_field" => crate::copy::REDACTION_CATEGORY_SENSITIVE_FIELD,
        "tool_sensitive_field" => crate::copy::REDACTION_CATEGORY_TOOL_SENSITIVE_FIELD,
        RESIDUAL_PREFIX => crate::copy::REDACTION_CATEGORY_RESIDUAL,
        _ => crate::copy::REDACTION_CATEGORY_UNKNOWN,
    }
}

/// `(removed, still_present)`.
///
/// Two lists rather than one with a flag, because the second is not a
/// variation on the first: it renders under a different heading, in the
/// attention tone, and a caller that forgot to check a flag would print a
/// surviving secret under the word "Removed".
pub fn rows(
    occurrences: &BTreeMap<String, u32>,
    distinct: &BTreeMap<String, u32>,
) -> (Vec<Row>, Vec<Row>) {
    let mut grouped: Vec<Row> = Vec::new();
    for (label, count) in occurrences {
        let key = family(label);
        let sub = label.split_once(':').map(|(_, tail)| tail);
        let position = match grouped.iter().position(|r| r.family == key) {
            Some(at) => at,
            None => {
                grouped.push(Row {
                    family: key.to_string(),
                    display: key.replace('_', " "),
                    description: describe(key),
                    occurrences: 0,
                    distinct: 0,
                    detail: Vec::new(),
                });
                grouped.len() - 1
            }
        };
        let row = &mut grouped[position];
        row.occurrences += count;
        row.distinct += distinct.get(label).copied().unwrap_or(0);
        if let Some(sub) = sub {
            // A removal's sub-label is a pattern name and reads as words; a
            // survivor's is a SCHEMA PATH and must not be reworded, because
            // `events.3.tool_result` names a field and `events.3.tool
            // result` names nothing.
            row.detail.push(if is_removal(label) {
                sub.replace('_', " ")
            } else {
                sub.to_string()
            });
        }
    }
    for row in &mut grouped {
        row.detail.sort();
    }
    // Biggest first, which is what someone scanning the panel is looking
    // for, with ties on the family so two renders agree.
    grouped.sort_by(|a, b| {
        b.occurrences
            .cmp(&a.occurrences)
            .then_with(|| a.family.cmp(&b.family))
    });
    grouped.into_iter().partition(|row| is_removal(&row.family))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, u32)]) -> BTreeMap<String, u32> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn an_empty_map_produces_no_rows() {
        let (removed, still) = rows(&map(&[]), &map(&[]));
        assert!(removed.is_empty());
        assert!(still.is_empty());
    }

    #[test]
    fn one_family_becomes_one_row() {
        let (removed, _) = rows(&map(&[("local_path", 185)]), &map(&[("local_path", 12)]));
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].family, "local_path");
        assert_eq!(removed[0].display, "local path");
        assert_eq!(removed[0].occurrences, 185);
        assert_eq!(removed[0].distinct, 12);
        assert!(!removed[0].description.is_empty());
    }

    /// Nine secret patterns are one `secret` row, not nine rows.
    ///
    /// The distinct map is EMPTY here, and that is the real shape: distinct
    /// counts come from the placeholder map, and only `local_path` and
    /// `private_email` mint a placeholder (`apply_placeholder_regex` in
    /// `trace-commons-protocol`). A fixture feeding `distinct: {"secret": 3}`
    /// would assert correct behaviour on input that cannot occur.
    #[test]
    fn sub_labels_collapse_into_their_family() {
        let (removed, _) = rows(
            &map(&[
                ("secret:contextual_entropy", 3),
                ("secret:pem_private_key", 1),
                ("secret", 2),
            ]),
            &map(&[]),
        );
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].occurrences, 6);
        assert_eq!(removed[0].distinct, 0, "secrets mint no placeholder");
        assert_eq!(removed[0].detail, ["contextual entropy", "pem private key"]);
    }

    /// The shape a secrets-only session actually has: occurrences for two
    /// secret labels, and `redactions_distinct` as `{}` -- the key is always
    /// present on the wire (no `skip_serializing_if`) and always empty here.
    /// Nothing in the panel may render a distinct figure for it.
    #[test]
    fn a_secrets_only_session_carries_no_distinct_figure() {
        let (removed, _) = rows(
            &map(&[("secret", 1), ("secret:openai_api_key", 1)]),
            &map(&[]),
        );
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].distinct, 0);
        assert_eq!(
            crate::copy::redaction_row_counts(removed[0].occurrences, removed[0].distinct),
            "2",
            "a zero distinct count is an absence, never `(0 distinct)`"
        );
    }

    /// Distinct counts summed across a family, using the only two labels
    /// that can carry one.
    #[test]
    fn distinct_counts_sum_across_the_labels_that_mint_placeholders() {
        let (removed, _) = rows(
            &map(&[("local_path", 185), ("private_email", 4)]),
            &map(&[("local_path", 12), ("private_email", 2)]),
        );
        assert_eq!(
            removed
                .iter()
                .map(|r| (r.family.as_str(), r.occurrences, r.distinct))
                .collect::<Vec<_>>(),
            [("local_path", 185, 12), ("private_email", 4, 2)]
        );
    }

    /// A secret DETECTED AND NOT REMOVED. Putting it in `removed` would
    /// state the exact opposite of what happened.
    #[test]
    fn a_residual_survivor_is_reported_as_still_present() {
        let (removed, still) = rows(
            &map(&[
                ("local_path", 3),
                ("residual_secret_at:events.correction", 1),
            ]),
            &map(&[]),
        );
        assert_eq!(
            removed
                .iter()
                .map(|r| r.family.as_str())
                .collect::<Vec<_>>(),
            ["local_path"]
        );
        assert_eq!(
            still.iter().map(|r| r.family.as_str()).collect::<Vec<_>>(),
            ["residual_secret_at"]
        );
        assert_eq!(still[0].detail, ["events.correction"]);
    }

    /// Hiding a category this build has no words for would understate what
    /// happened, which is the one direction this panel must not fail in.
    #[test]
    fn an_unknown_family_is_kept_with_a_neutral_description() {
        let (removed, _) = rows(&map(&[("future_category", 4)]), &map(&[]));
        assert_eq!(removed.len(), 1);
        assert_eq!(
            removed[0].description,
            crate::copy::REDACTION_CATEGORY_UNKNOWN
        );
    }

    #[test]
    fn rows_are_ordered_by_occurrences_then_family() {
        let (removed, _) = rows(
            &map(&[("secret", 3), ("local_path", 185), ("email", 3)]),
            &map(&[]),
        );
        assert_eq!(
            removed
                .iter()
                .map(|r| r.family.as_str())
                .collect::<Vec<_>>(),
            ["local_path", "email", "secret"]
        );
    }

    /// The panel names kinds, never values.
    #[test]
    fn a_row_carries_no_matched_text() {
        let (removed, _) = rows(&map(&[("secret", 1)]), &map(&[]));
        assert!(removed[0].detail.is_empty());
    }

    /// A schema path is not prose and must not be reworded: `events.3.
    /// tool_result` names a field, and turning its underscore into a space
    /// would name nothing.
    #[test]
    fn a_survivor_site_is_reported_verbatim() {
        let (_, still) = rows(
            &map(&[("residual_secret_at:events.3.tool_result", 1)]),
            &map(&[]),
        );
        assert_eq!(still[0].detail, ["events.3.tool_result"]);
    }
}
