//! The default per-principal submission rate ingest admits, shared so the
//! contributor daemon paces itself under the number the server enforces.

/// Submissions one principal may start per [`SUBMIT_RATE_WINDOW_SECS`] by
/// default.
pub const SUBMIT_PER_PRINCIPAL_LIMIT: u32 = 30;

/// The fixed window, in seconds, [`SUBMIT_PER_PRINCIPAL_LIMIT`] is counted
/// over.
pub const SUBMIT_RATE_WINDOW_SECS: u64 = 60;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_rate_is_thirty_a_minute() {
        assert_eq!(SUBMIT_PER_PRINCIPAL_LIMIT, 30);
        assert_eq!(SUBMIT_RATE_WINDOW_SECS, 60);
    }
}
