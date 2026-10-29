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

/// A week is comparable when counted sessions, known and partial, are at
/// least this share of all; only unknown sessions count against it.
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

/// Weekly bars each Patterns card draws, oldest first, ending at the week on
/// screen. A week with no figure is a gap, never a zero bar, and no run of
/// weeks is counted. Owner decisions D1 and D10, open.
pub const PATTERN_BAR_WEEKS: usize = 6;

/// Whether any advice sentence, what-if card or context tip may be shown.
/// The engine computes what-if regardless; no surface reads it while this is
/// false. Owner decision D2, open.
pub const ADVICE_SHOWN: bool = false;

/// Whether the "Edit, failed command, edit" pattern is computed. It is
/// labelled inferred from the order of tool calls. Owner decision D9, open.
pub const EDIT_FAIL_EDIT_CARD: bool = true;

/// Whether Codex `last_token_usage` is accepted as a per-turn series. Owner
/// decision D11, settled (owner, 2026-10-20): the Codex reader keeps one turn
/// per increase of the running token total. Three views read the series:
/// the session drill-in's chart, the Sessions list (its `turns` and
/// `sparkline`) and the Models answers. A Codex session's `tokens`, in the
/// drill and the list, and the weekly token figures, goals, the glance and
/// Patterns keep reading the first-to-last observed change, so trimming a
/// series never moves a token figure. While false the counter pass stores
/// no Codex series (`insights_week::scan_session`) and no stored one is
/// served (`CounterRow::session_input_with`): every view reads a Codex
/// session as one saved before the per-turn reader.
pub const CODEX_PER_TURN_SERIES: bool = true;

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
/// Owner decision D5, settled by the owner on 2026-10-20: Insights shows
/// money, labelled. The proxy ledger's `cost_usd` is shown as a charge;
/// everything else is at API list prices from reviewed tables shipped in
/// the build, with no runtime fetch. It gates every money path: the ledger
/// fold takes no charge (`insights_route_tally::fold_into`), the answer
/// reads no stored charge (`insights_week::charge_map`), and the Docs cost
/// row carries no list-priced figure and no `near_ai_charge`
/// (`model_answers::cost_row`) while it is false.
pub const LEDGER_MONEY_IN_INSIGHTS: bool = true;

/// Whether the daemon setting `insights_ledger_feed` starts on. Owner
/// decision D3, settled by the owner on 2026-10-09: on by default, and the
/// setting remains so a contributor can turn it off. While it is off nothing
/// reads the proxy ledger for Insights: `insights_glance` answers
/// `enabled: false`, `inference_calls` carries no `tokens`, and
/// `usage_changed` is never published.
pub const LEDGER_FEED_DEFAULT_ON: bool = true;

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

/// Whether the daemon setting `insights_counter_pass` starts on. While it is
/// off the watcher runs no counter pass, nothing is read for Insights, and
/// `insights_week` answers `enabled: false` without touching the store.
/// On (product owner, 2026-10-29: "counting is on by default, it must be on
/// always"), which settles owner decision D4: Insights reads the local
/// session files, nothing is uploaded for it. An install saved before then
/// is turned on once (`insights_counter_pass_defaulted_on`).
pub const COUNTER_PASS_DEFAULT_ON: bool = true;
/// Whether the weekly summary card starts on. It is an in-app card only (no
/// notification: owner decision D1), and it shows nothing without feed T.
/// Owner decisions D1 and D4, open.
pub const RECAP_CARD_DEFAULT_ON: bool = true;
/// Whether sessions in a folder that is Never are left out of the counter
/// pass, and their stored rows dropped: the folder's resolved rule, so a
/// global Never contribution override leaves every folder out. Owner
/// decision D4, settled in Ron's review of #1287.
pub const COUNTER_PASS_EXCLUDES_NEVER_FOLDERS: bool = true;
/// The counter store keeps at most this many sessions; past it the oldest go
/// first. Owner decision D4, open (a capped daemon-local store).
pub const COUNTER_STORE_MAX_SESSIONS: usize = 2_048;
/// ...and at most this many turns across every session's Claude Code
/// series. Codex per-turn series do not count here: they have their own
/// budget, below. Owner decision D4, open.
pub const COUNTER_STORE_MAX_TURNS: usize = 131_072;
/// ...and at most this many tool calls across every session's Claude Code
/// series; Codex's have their own budget. Owner decision D4, open.
pub const COUNTER_STORE_MAX_TOOL_CALLS: usize = 131_072;
/// The Codex per-turn series' own budget, in turns across every Codex row
/// (owner decision, 2026-10-20: a separate allowance for Codex store data).
/// Past it, or past the tool-call budget below, the oldest Codex rows lose
/// their per-turn series only, never the row: the row, its observed totals,
/// dates and facts stay, so the session still counts in every weekly
/// figure, while the Sessions drill and the Models answers read its
/// per-turn data as unknown. Nothing is evicted for this budget.
///
/// Sized for 13 weeks of heavy Codex use. Measured 2026-10-20 on the
/// owner's machine over `COUNTER_STORE_KEEP_WEEKS`: 2,517 Codex files,
/// 198,687 `token_count` events, 188,156 turns after the reader drops
/// repeats, about 166,700 once each session's own `MAX_SERIES_TURNS` cap
/// applies, and 160,756 tool calls. 196,608 (1.5 times the shared caps)
/// keeps about 18% headroom on turns and 22% on tool calls.
///
/// Size: serialized as the store's JSON, a turn is 185 bytes typical and at
/// most about 225 (every counter near `u32::MAX`, a sub-second time); a
/// tool call is 194 bytes typical and about 280 at most (a path digest and
/// extension). At both budgets the Codex series add at most about 44 MB
/// (turns) plus 55 MB (tool calls), about 99 MB, to the file, and about
/// 74 MB typical; in memory a record is about 100 bytes, about 40 MB, held
/// twice while a pass writes. The shared caps keep their own bound (about
/// 65 MB at most) beside it.
pub const COUNTER_STORE_MAX_CODEX_SERIES_TURNS: usize = 196_608;
/// ...and at most this many tool calls across every Codex row's series.
/// See `COUNTER_STORE_MAX_CODEX_SERIES_TURNS` for the sizing.
pub const COUNTER_STORE_MAX_CODEX_SERIES_TOOL_CALLS: usize = 196_608;
/// Automated sessions' own allowance (owner decision, 2026-10-20, option
/// (b)): a session started by a tool rather than the person (a Codex
/// session its harness spawned itself: the automatic approvals reviewer or
/// a delegated thread) does not count toward `COUNTER_STORE_MAX_SESSIONS`
/// or the shared turn and tool-call caps. At most this many are kept
/// besides, the oldest evicted first within the allowance, so they never
/// crowd the contributor's own sessions out. Their Codex series stay under
/// the Codex series budget above. A session that cannot be told apart
/// counts as the person's.
///
/// They stay in the overview's token totals, since they cost tokens, but
/// past the allowance the oldest are gone from those totals. Sized to keep
/// two busy weeks of them, several hundred a week on a heavy Codex user's
/// machine, so the current week and the one before it, the two the
/// overview's change compares, stay whole; older weeks may undercount
/// automated tokens. Half the shared cap, so the store grows by at most
/// half as many rows.
pub const COUNTER_STORE_MAX_AUTOMATED_SESSIONS: usize = 1_024;
/// Sessions last written longer ago than this many weeks are not read, and
/// their rows are dropped: enough for six weekly marks and a four-week
/// baseline. Owner decision D4, open.
pub const COUNTER_STORE_KEEP_WEEKS: i64 = 13;
/// One poll reads at most this many session files for the counter pass; the
/// rest wait for later polls, so a first pass over a large corpus never stalls
/// the watcher. Owner decision D4, settled 2026-10-20 (the per-poll budget
/// stays).
pub const COUNTER_PASS_MAX_READS_PER_TICK: usize = 16;
/// ...and at most this many bytes, except that the first file a poll reads
/// is always read, streamed and never held whole, so a file larger than this
/// is never starved. Owner decision D4, settled 2026-10-20.
pub const COUNTER_PASS_MAX_BYTES_PER_TICK: u64 = 64 * 1024 * 1024;
/// Session files are read line by line, so no file is skipped for its size
/// and memory does not grow with it: it is bounded by one line and by what
/// the scans keep, at most the series caps (`MAX_SERIES_TURNS`,
/// `MAX_SERIES_TOOL_CALLS`) for Claude Code (owner decision D4, settled
/// 2026-10-20). One line longer than this (an inlined image, say) is passed
/// over unread: the rest of the session is counted and it is marked
/// partial, never stored as unknown for it.
pub const COUNTER_PASS_MAX_LINE_BYTES: u64 = 16 * 1024 * 1024;

/// Whether a Codex transcript records a contributor declining an edit in a
/// shape the Codex scan can match. While false, Codex `denied` is unknown
/// (`None`) in every mode, never a known zero.
///
/// Search of 2026-10-20 over 4,823 local Codex transcripts: no
/// `exec_approval_request` or `apply_patch_approval_request` event is
/// persisted, and no `apply_patch` denial was found. Five `exec_command`
/// rejections in four files were found, each a `response_item` of type
/// `function_call_output` whose output ends
/// `CreateProcess { message: "Rejected(\"rejected by user\")" }`, all in
/// `on-request` turns that carry no `approvals_reviewer` field. None was a
/// denial of an edit, and no denial appeared in a turn that names the
/// reviewer (`user` was seen only under `never`), so the reviewer value that
/// names the contributor cannot be pinned from a real transcript. Spec
/// section 1, "Codex markers are not confirmed".
pub const CODEX_DENIAL_CONFIRMED: bool = false;

/// The model label Codex's automatic reviewer declares in `turn_context`.
/// A session whose turns all declare it is the reviewer's, not the
/// contributor's: it keeps its token totals and is left out of every model
/// answer (spec section 2, "Reviewer sessions"). Search of 2026-10-20: 1,009
/// local Codex transcripts declare it in `turn_context`, and none of them
/// declares another model.
pub const CODEX_REVIEWER_MODEL: &str = "codex-auto-review";

/// Version of the task classifier's rules and thresholds. A bump that changes
/// only rules or thresholds re-classifies from the stored shape (spec
/// section 5).
pub const CLASSIFIER_VERSION: u32 = 1;
/// Version of the stored session facts. A bump that changes what a scan
/// collects re-reads each row once.
pub const SESSION_FACTS_VERSION: u32 = 1;

/// Sessions a (task type, model) cell needs before its figure is compared
/// (spec section 6).
pub const MODEL_ANSWER_MIN_SESSIONS: u32 = 5;
/// Edits a cell needs before its rejection figure is compared.
pub const MODEL_ANSWER_MIN_EDITS: u32 = 30;
/// Share of the contributor's task mix a model's qualifying task types must
/// carry before it is compared across task types.
pub const MODEL_ANSWER_MIN_MIX_COVERAGE: Ratio = Ratio::new(4, 5);
/// Resamples behind every named leader.
pub const MODEL_ANSWER_BOOTSTRAP_DRAWS: u32 = 1_000;
/// Share of draws in which the leader must lead the next model.
pub const MODEL_ANSWER_LEAD_SHARE: Ratio = Ratio::new(95, 100);
/// The fixed bootstrap seed, so the same rows always give the same answer.
pub const MODEL_ANSWER_BOOTSTRAP_SEED: u64 = 0x7472_6163_655f_6d61;
/// Weeks the Models answers cover, ending now: the counter store's
/// retention, whatever week is on screen.
pub const MODEL_ANSWER_WINDOW_WEEKS: i64 = COUNTER_STORE_KEEP_WEEKS;

/// Extensions that make an edited file doc-class.
pub const DOC_EXTENSIONS: [&str; 4] = ["md", "mdx", "rst", "adoc"];
/// Directories under which a `.txt` file is doc-class; elsewhere it is
/// config-class.
pub const DOC_DIRS: [&str; 2] = ["docs", "doc"];
/// Directories that make every file under them test-class.
pub const TEST_DIRS: [&str; 4] = ["tests", "test", "__tests__", "spec"];
/// Marks inside a file name that make it test-class.
pub const TEST_NAME_MARKS: [&str; 3] = ["_test.", ".test.", ".spec."];
/// A file-name prefix that makes it test-class.
pub const TEST_NAME_PREFIX: &str = "test_";
/// Extensions that make an edited file config-class. A default taken, not an
/// owner ruling; `CMakeLists.txt` and `requirements.txt` fall here by `txt`.
pub const CONFIG_EXTENSIONS: [&str; 11] = [
    "txt", "json", "toml", "yaml", "yml", "ini", "cfg", "conf", "lock", "xml", "plist",
];
/// Files renamed or moved at which a session is a refactor.
pub const REFACTOR_MIN_RENAMES: u32 = 3;
/// Edited files at which balanced line counts make a session a refactor.
pub const REFACTOR_MIN_FILES: u32 = 5;
/// Net lines (`|added - removed|`) within this share of lines changed
/// (`added + removed`) count as balanced.
pub const REFACTOR_NET_LINES: Ratio = Ratio::new(1, 10);

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
        assert_eq!(PATTERN_BAR_WEEKS, 6);
    }

    #[test]
    fn held_and_default_decisions() {
        const { assert!(!ADVICE_SHOWN) };
        const { assert!(EDIT_FAIL_EDIT_CARD) };
        const { assert!(CODEX_PER_TURN_SERIES) };
        const { assert!(DEDUPE_TURNS_BY_MSG_KEY) };
        const { assert!(LEDGER_MONEY_IN_INSIGHTS) };
        assert_eq!(DIGEST_KEY_CUSTODY, DigestKeyCustody::OsKeychain);
    }

    #[test]
    fn ledger_glance_constants() {
        const { assert!(LEDGER_FEED_DEFAULT_ON) };
        assert_eq!(CONTEXT_TIP_RECENT_CALLS, 5);
        assert_eq!(CONTEXT_TIP_WINDOW_SECS, 600);
        assert_eq!(CONTEXT_TIP_RATIO, Ratio::new(9, 10));
        assert_eq!(LEDGER_GLANCE_STALE_SECS, 600);
    }

    #[test]
    fn counter_pass_constants() {
        const { assert!(COUNTER_PASS_DEFAULT_ON) };
        const { assert!(COUNTER_PASS_EXCLUDES_NEVER_FOLDERS) };
        const { assert!(RECAP_CARD_DEFAULT_ON) };
        assert_eq!(COUNTER_STORE_MAX_SESSIONS, 2_048);
        assert_eq!(COUNTER_STORE_MAX_TURNS, 131_072);
        assert_eq!(COUNTER_STORE_MAX_TOOL_CALLS, 131_072);
        assert_eq!(COUNTER_STORE_MAX_CODEX_SERIES_TURNS, 196_608);
        assert_eq!(COUNTER_STORE_MAX_CODEX_SERIES_TOOL_CALLS, 196_608);
        // The newest Codex session always keeps its series: one session's
        // own caps fit inside the budget.
        const {
            assert!(
                COUNTER_STORE_MAX_CODEX_SERIES_TURNS
                    >= trace_commons_protocol::insights_usage_series::MAX_SERIES_TURNS
            )
        };
        const {
            assert!(
                COUNTER_STORE_MAX_CODEX_SERIES_TOOL_CALLS
                    >= trace_commons_protocol::insights_usage_series::MAX_SERIES_TOOL_CALLS
            )
        };
        assert_eq!(COUNTER_STORE_MAX_AUTOMATED_SESSIONS, 1_024);
        const { assert!(COUNTER_STORE_MAX_AUTOMATED_SESSIONS < COUNTER_STORE_MAX_SESSIONS) };
        assert_eq!(COUNTER_STORE_KEEP_WEEKS, 13);
        assert_eq!(COUNTER_PASS_MAX_READS_PER_TICK, 16);
        assert_eq!(COUNTER_PASS_MAX_BYTES_PER_TICK, 64 * 1024 * 1024);
        assert_eq!(COUNTER_PASS_MAX_LINE_BYTES, 16 * 1024 * 1024);
        // The kept weeks cover the six weekly marks plus the lever's baseline.
        const { assert!(COUNTER_STORE_KEEP_WEEKS as usize > WEEKLY_MARKS + LEVER_BASELINE_WEEKS) };
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
    fn codex_denial_follows_the_confirmed_marker() {
        // false: the 2026-10-20 search found exec rejections only in turns
        // that carry no `approvals_reviewer`, and none of an edit; Codex
        // `denied` is always None.
        const { assert!(!CODEX_DENIAL_CONFIRMED) };
        assert_eq!(CODEX_REVIEWER_MODEL, "codex-auto-review");
    }

    #[test]
    fn task_classifier_constants() {
        assert_eq!(CLASSIFIER_VERSION, 1);
        assert_eq!(SESSION_FACTS_VERSION, 1);
        assert_eq!(DOC_EXTENSIONS, ["md", "mdx", "rst", "adoc"]);
        assert_eq!(DOC_DIRS, ["docs", "doc"]);
        assert_eq!(TEST_DIRS, ["tests", "test", "__tests__", "spec"]);
        assert_eq!(TEST_NAME_MARKS, ["_test.", ".test.", ".spec."]);
        assert_eq!(TEST_NAME_PREFIX, "test_");
        assert_eq!(
            CONFIG_EXTENSIONS,
            [
                "txt", "json", "toml", "yaml", "yml", "ini", "cfg", "conf", "lock", "xml", "plist"
            ]
        );
        assert_eq!(REFACTOR_MIN_RENAMES, 3);
        assert_eq!(REFACTOR_MIN_FILES, 5);
        assert_eq!(REFACTOR_NET_LINES, Ratio::new(1, 10));
    }

    #[test]
    fn a_zero_denominator_reaches_nothing() {
        assert!(!Ratio::new(1, 2).reached_by(5, 0));
        assert!(!Ratio::new(1, 2).exceeds(5, 0));
    }
}
