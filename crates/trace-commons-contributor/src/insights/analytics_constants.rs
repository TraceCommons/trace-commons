//! Every constant the token analytics engine uses, in one place.
//!
//! Each is an open owner decision; a ruling changes one line here. Ratios are
//! integer fractions so no figure passes through floating point.

/// An integer fraction `num / den`, compared by cross-multiplication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratio {
    pub num: u64,
    pub den: u64,
}

impl Ratio {
    pub const fn new(num: u64, den: u64) -> Self {
        Self { num, den }
    }

    /// `value / of >= self`. False when `of` is zero.
    pub fn reached_by(self, value: u64, of: u64) -> bool {
        of != 0 && u128::from(value) * u128::from(self.den) >= u128::from(self.num) * u128::from(of)
    }

    /// `self > value / of`, the complement of [`Ratio::reached_by`]. False
    /// when `of` is zero, so an undefined ratio satisfies neither side.
    pub fn exceeds(self, value: u64, of: u64) -> bool {
        of != 0 && !self.reached_by(value, of)
    }
}

/// Context, in tokens, at which a session is in the long-context range.
/// Owner decision D10, open.
pub const LONG_CONTEXT_TOKENS: u64 = 200_000;

/// Summary size the what-if assumes a fresh session starts from. A core
/// constant, never a request field. Owner decision D10, open; shown nowhere
/// until owner decision D2.
pub const WHAT_IF_SUMMARY_TOKENS: u64 = 40_000;
/// Turns that must follow the long-context turn for a what-if to be computed.
/// Owner decision D10, open.
pub const WHAT_IF_MIN_TURNS_AFTER: usize = 3;

/// Pause after which a cache rewrite may be inferred. Owner decision D10, open.
pub const CACHE_GAP_SECS: u32 = 300;
/// Pause used instead when the last write declared a one-hour lifetime.
/// Owner decision D10, open.
pub const CACHE_GAP_ONE_HOUR_SECS: u32 = 3_600;
/// A rewrite needs this turn's cache read below this share of the last one.
/// Owner decision D10, open.
pub const CACHE_REWRITE_READ_RATIO: Ratio = Ratio::new(1, 2);
/// A rewrite needs this turn's cache write at or above this share of the last
/// turn's context. Owner decision D10, open.
pub const CACHE_REWRITE_WRITE_RATIO: Ratio = Ratio::new(1, 2);

/// Context shrank when it falls below this share of the last turn's.
/// Owner decision D10, open.
pub const SHRINK_RATIO: Ratio = Ratio::new(1, 2);
/// A shrink counts only from a context at least this large.
/// Owner decision D10, open.
pub const SHRINK_FLOOR_TOKENS: u64 = 50_000;

/// A retried tool call repeats one of this many calls before it.
/// Owner decision D10, open.
pub const RETRY_WINDOW_CALLS: usize = 5;
/// An edit, failed command, edit cycle spans at most this many calls.
/// Owner decision D10, open.
pub const EDIT_FAIL_EDIT_WINDOW_CALLS: usize = 6;

/// Result bytes per estimated token. Owner decision D10, open.
pub const RESULT_BYTES_PER_TOKEN: u64 = 4;
/// Result-size estimates are rounded to a multiple of this.
/// Owner decision D10, open.
pub const TOKEN_ESTIMATE_ROUNDING: u64 = 10_000;

/// A week is comparable when known sessions are at least this share of all.
/// Owner decision D10, open.
pub const COMPARABLE_COVERAGE: Ratio = Ratio::new(4, 5);
/// ...and it has at least this many sessions. Owner decision D10, open.
pub const COMPARABLE_MIN_SESSIONS: u32 = 5;
/// "Your best week" needs this many earlier comparable weeks.
/// Owner decision D10, open.
pub const BEST_WEEK_MIN_EARLIER_WEEKS: usize = 3;

/// A lever candidate's figure must reach this. Owner decision D10, open.
pub const LEVER_FLOOR_TOKENS: u64 = 100_000;
/// ...and its ratio to its baseline median must reach this.
/// Owner decision D10, open.
pub const LEVER_RATIO: Ratio = Ratio::new(5, 4);
/// Comparable weeks the lever baseline is the median of.
/// Owner decision D10, open.
pub const LEVER_BASELINE_WEEKS: usize = 4;
/// Weeks a kind marked "Not useful" stays suppressed.
/// Owner decision D10, open.
pub const LEVER_SUPPRESSION_WEEKS: i64 = 4;
/// Dismissals after which a kind stays off until re-enabled.
/// Owner decision D10, open.
pub const LEVER_DISMISSALS_TO_DISABLE: usize = 3;

/// Weekly marks a goal shows: met, not met, or no figure, one per week. No
/// run of weeks is counted and no ring is drawn. Owner decision D1, open.
pub const WEEKLY_MARKS: usize = 6;

/// Whether any advice sentence, what-if card or context tip may be shown.
/// The engine computes what-if regardless; no surface reads it while this is
/// false. Owner decision D2, open.
pub const ADVICE_SHOWN: bool = false;

/// Whether the "Edit, failed command, edit" pattern is computed. It is
/// labelled inferred from the order of tool calls. Owner decision D9, open.
pub const EDIT_FAIL_EDIT_CARD: bool = true;

/// Whether Codex `last_token_usage` is accepted as a per-turn series. While
/// false, Codex counts only first-to-last observed deltas and its per-turn
/// views read "not recorded". Owner decision D11, open.
pub const CODEX_PER_TURN_SERIES: bool = false;

/// Whether Claude turns are counted once across snapshots by their keyed
/// message digest. While false, overlapping snapshots of one harness are
/// excluded as `reimport_overlap`. Owner decision D15, open.
pub const DEDUPE_TURNS_BY_MSG_KEY: bool = true;

/// Where the key behind every keyed digest (message, path and argument
/// digests) is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestKeyCustody {
    /// The OS keychain, separate from the store, so a copied store directory
    /// is not enough to confirm a guessed path or command.
    OsKeychain,
    /// A 0600 key file inside the store directory. Weaker; works where no
    /// keychain is available.
    KeyFileInStore,
}

/// Digest key custody. Owner decision D16, open; the recommended default is
/// the OS keychain.
pub const DIGEST_KEY_CUSTODY: DigestKeyCustody = DigestKeyCustody::OsKeychain;

/// Whether ledger `cost_usd` or list prices may reach an Insights figure.
/// Owner decision D5, open.
pub const LEDGER_MONEY_IN_INSIGHTS: bool = false;

/// Whether the daemon setting `insights_ledger_feed` starts on. While it is
/// off nothing reads the proxy ledger for Insights: `insights_glance` answers
/// `enabled: false`, `inference_calls` carries no `tokens`, and
/// `usage_changed` is never published. Owner decision D3, open.
pub const LEDGER_FEED_DEFAULT_ON: bool = false;

/// The context tip reads at most this many of the current session's newest
/// calls, so a side call (a title, a compaction, a subagent) cannot make it
/// flap. Owner decision D10, open.
pub const CONTEXT_TIP_RECENT_CALLS: usize = 5;
/// ...made within this many seconds. A session with no call this recent has
/// no tip. Owner decision D10, open.
pub const CONTEXT_TIP_WINDOW_SECS: i64 = 600;
/// The tip is lit when context reaches this share of the user's threshold.
/// Owner decision D10, open; shown nowhere until owner decision D2.
pub const CONTEXT_TIP_RATIO: Ratio = Ratio::new(9, 10);

/// A glance whose ledger last answered longer ago than this is stale, and a
/// shell hides the card rather than show an old figure. Owner decision D10,
/// open.
pub const LEDGER_GLANCE_STALE_SECS: i64 = 600;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_context_threshold() {
        assert_eq!(LONG_CONTEXT_TOKENS, 200_000);
    }

    #[test]
    fn what_if_summary_size() {
        assert_eq!(WHAT_IF_SUMMARY_TOKENS, 40_000);
        assert_eq!(WHAT_IF_MIN_TURNS_AFTER, 3);
    }

    #[test]
    fn cache_gaps() {
        assert_eq!(CACHE_GAP_SECS, 300);
        assert_eq!(CACHE_GAP_ONE_HOUR_SECS, 3_600);
    }

    #[test]
    fn half_ratios() {
        for ratio in [
            CACHE_REWRITE_READ_RATIO,
            CACHE_REWRITE_WRITE_RATIO,
            SHRINK_RATIO,
        ] {
            assert_eq!(ratio, Ratio::new(1, 2));
        }
    }

    #[test]
    fn shrink_floor() {
        assert_eq!(SHRINK_FLOOR_TOKENS, 50_000);
    }

    #[test]
    fn call_windows() {
        assert_eq!(RETRY_WINDOW_CALLS, 5);
        assert_eq!(EDIT_FAIL_EDIT_WINDOW_CALLS, 6);
    }

    #[test]
    fn token_estimate_from_result_size() {
        assert_eq!(RESULT_BYTES_PER_TOKEN, 4);
        assert_eq!(TOKEN_ESTIMATE_ROUNDING, 10_000);
    }

    #[test]
    fn comparable_week_floor() {
        assert_eq!(COMPARABLE_COVERAGE, Ratio::new(4, 5));
        assert_eq!(COMPARABLE_MIN_SESSIONS, 5);
        assert_eq!(BEST_WEEK_MIN_EARLIER_WEEKS, 3);
    }

    #[test]
    fn lever_constants() {
        assert_eq!(LEVER_FLOOR_TOKENS, 100_000);
        assert_eq!(LEVER_RATIO, Ratio::new(5, 4));
        assert_eq!(LEVER_BASELINE_WEEKS, 4);
        assert_eq!(LEVER_SUPPRESSION_WEEKS, 4);
        assert_eq!(LEVER_DISMISSALS_TO_DISABLE, 3);
    }

    #[test]
    fn weekly_marks_not_streaks() {
        assert_eq!(WEEKLY_MARKS, 6);
    }

    #[test]
    fn held_and_default_decisions() {
        const { assert!(!ADVICE_SHOWN) };
        const { assert!(EDIT_FAIL_EDIT_CARD) };
        const { assert!(!CODEX_PER_TURN_SERIES) };
        const { assert!(DEDUPE_TURNS_BY_MSG_KEY) };
        const { assert!(!LEDGER_MONEY_IN_INSIGHTS) };
        assert_eq!(DIGEST_KEY_CUSTODY, DigestKeyCustody::OsKeychain);
    }

    #[test]
    fn ledger_glance_constants() {
        const { assert!(!LEDGER_FEED_DEFAULT_ON) };
        assert_eq!(CONTEXT_TIP_RECENT_CALLS, 5);
        assert_eq!(CONTEXT_TIP_WINDOW_SECS, 600);
        assert_eq!(CONTEXT_TIP_RATIO, Ratio::new(9, 10));
        assert_eq!(LEDGER_GLANCE_STALE_SECS, 600);
    }

    #[test]
    fn ratio_comparisons_are_exact() {
        let half = Ratio::new(1, 2);
        assert!(half.reached_by(1, 2));
        assert!(!half.reached_by(49, 100));
        assert!(half.exceeds(49, 100));
        assert!(!half.exceeds(1, 2));
        assert!(COMPARABLE_COVERAGE.reached_by(4, 5));
        assert!(!COMPARABLE_COVERAGE.reached_by(79, 100));
        assert!(LEVER_RATIO.reached_by(125, 100));
        assert!(!LEVER_RATIO.reached_by(124, 100));
        // No overflow at the extremes.
        assert!(half.reached_by(u64::MAX, u64::MAX));
    }

    #[test]
    fn a_zero_denominator_reaches_nothing() {
        assert!(!Ratio::new(1, 2).reached_by(5, 0));
        assert!(!Ratio::new(1, 2).exceeds(5, 0));
    }
}
