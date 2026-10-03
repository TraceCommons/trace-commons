//! Shared contributor-facing history copy.

pub const HELD_ROW_BODY: &str = "Automated checks saw something that might be personal and \
     couldn't decide on their own. It has not been rejected, and it has not been shared with \
     anyone but the agent that inspects it.";

/// The label on a contribution whose status this build does not recognise.
///
/// Every shell shows this, and only this, for a status it has no label for:
/// it is never "Waiting to be scored", "Not in the commons" or the raw wire
/// token, each of which asserts a state the shell does not know. An
/// unrecognised status is also treated as terminal, so no shell offers
/// Withdraw on it.
///
/// Carried to the shells by `public_run_copy().contribution_status_unavailable`
/// (`tc_public_run_copy`) and by the disclosure bundle's
/// `history_ui.status_unavailable` (`tc_contributor_disclosure_copy_json`).
pub const STATUS_UNAVAILABLE: &str = "Status unavailable";

#[cfg(test)]
mod tests {
    use super::STATUS_UNAVAILABLE;

    /// An unrecognised status asserts nothing about where the contribution
    /// is: not waiting, not in the commons, not out of it.
    #[test]
    fn an_unrecognised_status_claims_no_state() {
        assert_eq!(STATUS_UNAVAILABLE, "Status unavailable");
        let lower = STATUS_UNAVAILABLE.to_lowercase();
        for claim in ["waiting", "scored", "commons", "withdrawn", "rejected"] {
            assert!(
                !lower.contains(claim),
                "{STATUS_UNAVAILABLE} claims {claim}"
            );
        }
    }
}
