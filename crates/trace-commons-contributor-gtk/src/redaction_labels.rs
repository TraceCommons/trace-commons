//! Reading the daemon's redaction count map, which does not mean what its
//! heading says it means.
//!
//! `DeterministicTraceRedactor` sets `redaction_counts` to the WHOLE
//! redaction report. Most of that report is what you would expect -- one
//! entry per pattern that fired, counting values it took out -- but it also
//! carries `residual_secret_at:{path}`, which `note_residual_secret_location`
//! increments when a secret was **detected and NOT removed**: a credential
//! inside a correction the contributor wrote, which is preserved on purpose,
//! or a field the typed redaction traversal never visits, which is a real
//! gap.
//!
//! Every shell renders that map under the heading "Removed by pattern", so a
//! session carrying a surviving secret has been reporting it as a thing that
//! was taken out -- the exact opposite of what happened, on the one screen
//! where somebody is deciding whether to send it.
//!
//! This shell had it worst of the three. `humanize_redaction_kind` maps any
//! label containing `secret` onto the word "secrets", and
//! `residual_secret_at:...` contains `secret` -- so a survivor was not merely
//! mislabelled here, it was SUMMED INTO the removed-secret count and became
//! indistinguishable from a secret that really had been taken out.
//!
//! Both halves of the fix matter and neither is optional:
//!
//! * a survivor must not be counted as a removal, and
//! * a survivor must still be SHOWN. Filtering it out of the figure and
//!   saying nothing else would trade a wrong statement for silence about a
//!   secret that is still in the payload, which on a consent surface is not
//!   an improvement.

use std::collections::BTreeMap;

/// The label family marking a secret that was found and left in place.
pub const RESIDUAL_PREFIX: &str = "residual_secret_at";

/// The part of a label before its first `:`.
///
/// The count vocabulary is namespaced and OPEN -- `secret:{pattern_name}`,
/// `privacy_filter:{label}` and `tool_sensitive_field:{action}` are generated
/// at redaction time -- so nothing here may assume a closed set of labels.
/// Families are the only stable thing to reason about.
pub fn family(label: &str) -> &str {
    label.split_once(':').map_or(label, |(head, _)| head)
}

/// Whether a label counts something that actually left the payload.
pub fn is_removal(label: &str) -> bool {
    family(label) != RESIDUAL_PREFIX
}

/// Total occurrences removed. Never includes survivors.
pub fn removed_total(counts: &BTreeMap<String, u32>) -> u32 {
    counts
        .iter()
        .filter(|(label, _)| is_removal(label))
        .map(|(_, n)| n)
        .sum()
}

/// How many places a secret was found and left in what would be sent.
/// Sites, not secrets: one site can hold more than one value.
pub fn survivor_total(counts: &BTreeMap<String, u32>) -> u32 {
    counts
        .iter()
        .filter(|(label, _)| !is_removal(label))
        .map(|(_, n)| n)
        .sum()
}

/// Where secrets were found and left in the payload, ordered for a stable
/// rendering.
///
/// The sites are schema-shaped identifiers -- `events.3.correction`, not a
/// filesystem path and not transcript text. The redactor guarantees that
/// where these labels are minted, and it is what makes them safe to show.
pub fn survivor_sites(counts: &BTreeMap<String, u32>) -> Vec<(String, u32)> {
    let prefix = format!("{RESIDUAL_PREFIX}:");
    let mut sites: Vec<(String, u32)> = counts
        .iter()
        .filter(|(label, _)| !is_removal(label))
        .map(|(label, n)| {
            let site = label
                .strip_prefix(prefix.as_str())
                .unwrap_or("")
                .to_string();
            (site, *n)
        })
        .collect();
    sites.sort();
    sites
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, u32)]) -> BTreeMap<String, u32> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn a_family_is_the_label_before_its_colon() {
        assert_eq!(family("secret:contextual_entropy"), "secret");
        assert_eq!(family("local_path"), "local_path");
        assert_eq!(
            family("residual_secret_at:events.3.correction"),
            RESIDUAL_PREFIX
        );
        assert_eq!(family(""), "");
    }

    #[test]
    fn an_ordinary_label_is_a_removal() {
        assert!(is_removal("local_path"));
        assert!(is_removal("secret"));
        assert!(is_removal("secret:pem_private_key"));
        assert!(is_removal("privacy_filter:person_name"));
    }

    /// The whole point of the module. `residual_secret_at` counts a secret
    /// that was DETECTED AND LEFT IN.
    #[test]
    fn a_residual_survivor_is_not_a_removal() {
        assert!(!is_removal("residual_secret_at:events.correction"));
    }

    /// The defect this module exists to fix, stated as a test: this shell's
    /// `humanize_redaction_kind` matches any label containing `secret`, so
    /// before the filter a survivor was summed into the removed-secret count.
    #[test]
    fn a_survivor_is_not_summed_into_the_removed_secret_count() {
        let counts = map(&[("secret", 1), ("residual_secret_at:events.correction", 1)]);
        assert_eq!(removed_total(&counts), 1);
        assert_eq!(survivor_total(&counts), 1);
    }

    #[test]
    fn removed_total_excludes_survivors() {
        let counts = map(&[
            ("local_path", 185),
            ("secret", 3),
            ("residual_secret_at:events.correction", 1),
        ]);
        assert_eq!(removed_total(&counts), 188);
    }

    /// A session that removed nothing and left a secret in reports zero
    /// removals, which is what puts the card in the tone that asks somebody
    /// to look.
    #[test]
    fn a_session_with_only_a_survivor_removed_nothing() {
        let counts = map(&[("residual_secret_at:events.x", 1)]);
        assert_eq!(removed_total(&counts), 0);
        assert_eq!(survivor_total(&counts), 1);
    }

    #[test]
    fn survivor_sites_are_reported_in_a_stable_order() {
        let counts = map(&[
            ("local_path", 3),
            ("residual_secret_at:events.9.correction", 2),
            ("residual_secret_at:events.1.tool_result", 1),
        ]);
        assert_eq!(
            survivor_sites(&counts),
            vec![
                ("events.1.tool_result".to_string(), 1),
                ("events.9.correction".to_string(), 2),
            ]
        );
    }

    #[test]
    fn a_session_with_no_survivors_reports_none() {
        let counts = map(&[("local_path", 3)]);
        assert_eq!(survivor_total(&counts), 0);
        assert!(survivor_sites(&counts).is_empty());
    }

    /// A bare `residual_secret_at` with no site still counts. It should never
    /// be minted, but dropping it would be the one failure direction that
    /// matters: silence about a surviving secret.
    #[test]
    fn a_survivor_with_no_site_is_still_counted() {
        let counts = map(&[("residual_secret_at", 1)]);
        assert_eq!(survivor_total(&counts), 1);
        assert_eq!(removed_total(&counts), 0);
        assert_eq!(survivor_sites(&counts), vec![(String::new(), 1)]);
    }
}
