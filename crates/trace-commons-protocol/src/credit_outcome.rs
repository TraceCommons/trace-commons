//! Why a decided trace earned no credit, as the contributor surfaces see it.
//!
//! The server's submission-status read-back carries a trace's final credit
//! in `credit_points_final`. When that figure is 0 because the trace was
//! decided without an award, `credit_outcome_reason` carries one of the
//! labels below, so a client can say why instead of showing the trace as
//! still pending.
//!
//! The set is closed and label-only: no identifier, score or operator
//! configuration ever rides on it. The server folds its own, finer
//! withheld-reason labels onto this set, and sends [`OTHER`] for one it
//! does not map, never the raw label. A client that meets a label it does
//! not know treats it as [`OTHER`].

/// The trace's uses do not include the one the award requires (a tenant
/// or submission allowed-use mismatch).
pub const POLICY_MISMATCH: &str = "policy_mismatch";

/// The gate judged the trace too close to what the commons already holds.
pub const NOVELTY_FAILED: &str = "novelty_failed";

/// The gate judged the trace below its information floor.
pub const PERPLEXITY_FAILED: &str = "perplexity_failed";

/// The same content was already sent under this account.
pub const DUPLICATE: &str = "duplicate";

/// The trace was not in the commons when the award was decided.
pub const SUBMISSION_NOT_ACCEPTED: &str = "submission_not_accepted";

/// The trace was too long for the gate to read all of it, and the part it
/// read did not pass.
pub const INSUFFICIENT_COVERAGE: &str = "insufficient_coverage";

/// This server did not issue the award (its credit issuer or gate is not
/// set up to, or its credit check failed).
pub const CREDIT_NOT_ISSUED: &str = "credit_not_issued";

/// A reason this build has no label for.
pub const OTHER: &str = "other";

/// Every label the server may send, in a fixed order.
pub const CREDIT_OUTCOME_REASONS: [&str; 8] = [
    POLICY_MISMATCH,
    NOVELTY_FAILED,
    PERPLEXITY_FAILED,
    DUPLICATE,
    SUBMISSION_NOT_ACCEPTED,
    INSUFFICIENT_COVERAGE,
    CREDIT_NOT_ISSUED,
    OTHER,
];

/// Whether `label` is one of [`CREDIT_OUTCOME_REASONS`].
#[must_use]
pub fn is_known_reason(label: &str) -> bool {
    CREDIT_OUTCOME_REASONS.contains(&label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_are_distinct_snake_case_tokens() {
        let mut seen = std::collections::BTreeSet::new();
        for label in CREDIT_OUTCOME_REASONS {
            assert!(
                label.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{label} is not snake_case"
            );
            assert!(seen.insert(label), "{label} is listed twice");
        }
    }

    #[test]
    fn an_unlisted_label_is_not_known() {
        assert!(is_known_reason(DUPLICATE));
        assert!(!is_known_reason("skipped_duplicate"));
        assert!(!is_known_reason(""));
    }
}
