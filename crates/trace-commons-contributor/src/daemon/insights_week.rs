//! The Insights counter pass (feed T) and `insights_week`.
//!
//! Gated through the `insights_counter_pass` setting, on by default (product
//! owner, 2026-10-29, settling owner decision D4; an install saved before
//! then is turned on once). While it is off -- after unenroll, or an agent
//! turning it off -- the watcher runs no pass, nothing is read for Insights,
//! and `insights_week` answers `enabled: false` without opening the store or
//! the key.
//!
//! # The pass
//!
//! On a full watcher poll, after the session has been quiet for the same
//! `quiescence_secs` the watcher waits before offering it, the pass reads the
//! Claude Code and Codex sessions the watcher already finds and stores their
//! counter rows. The poll does not run the pass: it hands an owned job (the
//! sessions' paths, sizes and last writes, the Never folders, the settings
//! it needs and the time) to [`submit_counter_pass`] and moves on, and the
//! pass reads on a thread of the runtime's blocking pool. One pass runs at a
//! time; a poll that arrives while one runs replaces the one job waiting
//! behind it, so the newest sessions are read next and a long pass leaves
//! one follow-up, not a backlog. Shutdown, the setting going off, or a
//! `clear` abandons a pass between files and before its write, and a pass
//! made before a `clear` writes nothing and keeps no key.
//!
//! The rows are Claude per-turn rows, with the extractor saved analysis uses
//! (`insights::turn_series`), and Codex's first-to-last observed change
//! beside its per-turn series (`insights::codex_series`; owner decision D11,
//! settled 2026-10-20). Weekly figures read Codex's observed change only;
//! the Sessions drill and the Models answers read the series. Each row also
//! carries the session's facts (`insights::session_facts`): per-model
//! counts of edits, declined, undone and interrupted calls, the task shape
//! and inferred task, active time, and fixed labels, never a path, a
//! command or a text. The store holds no list price: the Models answers
//! and `insights_week`'s `work` (`insights_week_work`) price stored tokens
//! when they are built. Its one stored money figure is the
//! actual NEAR AI charge per session, folded from the proxy ledger's
//! `cost_usd` into the session's route tally (owner decision D5, settled
//! 2026-10-20; `insights_route_tally`), in integer USD micros, because the
//! ledger keeps a call for 24 hours only. A row made before facts were kept, or under
//! an older `SESSION_FACTS_VERSION`, is read once more, within the per-poll
//! budget; a row whose task rules are older is re-classified from its
//! stored shape when answered, not read again. Sessions in a folder that is Never, by its
//! own rule or a global Never override, are not read, and rows already
//! stored for one are dropped (owner decision D4). Nor do they keep a route
//! tally: the pass names them from their file names, and both the pass and
//! the ledger fold drop their tallies (`NeverSessions`). A Claude session's
//! delegated transcripts (its sub-agents', workflow ones included) are not
//! read as part of it; a session that has any is marked truncated, so it
//! counts as partial coverage and never as complete.
//!
//! Those sub-agent transcripts are read on their own (product owner,
//! 2026-10-29), after the person's sessions and within the same per-poll
//! budget, into `CounterStore::spawned`: rows apart from the person's, under
//! their own allowance (`COUNTER_STORE_MAX_SPAWNED_SESSIONS`), with no
//! harness session (a sub-agent records its parent's session ID), no goals
//! and no `placed_at`. Only Benchmarks' sub-agent population reads them.
//! The parent's own transcript holds only its own model calls (on the
//! owner's machine, 2026-10-29: 1 message ID shared between 34 parents and
//! their sub-agents), so a week's tokens and spend, which stay the person's
//! sessions alone, never count a sub-agent's call twice, and leave its
//! calls out. A sub-agent of a session passed over for its managed-profile
//! copy is passed over with it; one in a Never folder is not read.
//!
//! A pass reads a bounded number of files and bytes, newest first, so a first
//! pass over a large corpus catches up across polls. The first file a pass
//! reads is always read, so a file larger than the byte budget is never
//! starved (owner decision D4, settled 2026-10-20: stream, no file-size cap,
//! the per-poll budget stays). Each file is read line by line, never held
//! whole, so no file is skipped for its size and memory does not grow with
//! it: it is bounded by one line and by what the scans keep, at most the
//! series caps for Claude Code. One line over `COUNTER_PASS_MAX_LINE_BYTES`
//! (an inlined image, say) is passed over unread and the session is marked
//! partial; the lines around it are still counted. A file unchanged in size and time since its
//! row was made is not read again; a file seen again with new bytes replaces
//! its row. A row stored as unknown by the earlier whole-file read, which
//! skipped files over 16 MiB, is read once more.
//!
//! # The store
//!
//! One daemon-local file, capped by sessions, turns and tool calls, oldest
//! going first, and limited to recent weeks, so it keeps the newest
//! sessions. The turn and tool-call caps count Claude Code's per-turn series
//! only. Codex per-turn series have their own allowance
//! (`COUNTER_STORE_MAX_CODEX_SERIES_TURNS` and `..._TOOL_CALLS`; owner
//! decision, 2026-10-20): past it the oldest Codex rows lose their series
//! only, marked `series_trimmed`, and keep the row, `observed`, dates and
//! facts, so every weekly figure still counts them. A trimmed row is not
//! evicted and not read again while its file is unchanged; the Sessions
//! answers draw it as a row read before the per-turn reader, and the Models
//! answers count it as unknown facts, never as zero.
//!
//! Automated sessions (owner decision, 2026-10-20, option (b)): a Codex
//! session whose first `session_meta` names a `subagent` source was started
//! by its harness (the automatic approvals reviewer, or a thread an agent
//! spawned), not by the person, and its row is classified `automated`
//! when read. Nothing else is guessed: a session that cannot be told, a
//! spawned thread carrying a person-started `session_meta` after its own,
//! and every Claude Code session (whose delegated transcripts are never
//! read as sessions) are unknown, and unknown counts as the person's.
//! Automated rows stay out of the shared session, turn and tool-call caps
//! and are kept under their own allowance
//! (`COUNTER_STORE_MAX_AUTOMATED_SESSIONS`), oldest evicted first within
//! it; their Codex series stay under the Codex series budget. The weekly
//! token figures count them (they cost tokens; `insights_week` also says
//! how many are stored, `automated_stored`), and past the allowance the
//! oldest are gone from them. The Models answers leave them out, counted
//! in `left_out.automated`; the Patterns cards and the Sessions feed leave
//! them out with no switch to bring them back. A Codex row stored before
//! the classification is read once more, within the per-poll budget; until
//! every such row this poll finds is read, the eviction bounds below are
//! not applied and an eviction of the person's is not marked, and marks
//! written before the allowance are dropped, so a session of the person's
//! that automated rows pushed out is read again.
//!
//! A session the caps evict is marked by its keyed digest, size
//! and last write. A session with no row whose last write is no later than
//! the newest mark of the shared caps, or, with the person's rows at the
//! session cap, earlier than every one of them, is not read: the caps would
//! evict it again. An automated session is not read again while its own
//! mark matches its unchanged file. An evicted file is
//! read again once it changes, since a write moves its last write past the
//! mark. A session older than the newest mark is not read even when rows
//! leave for another reason, such as a Never folder, until that mark ages
//! out of the kept weeks. Rows are keyed by a keyed digest
//! of the harness and the session's address, and carry a keyed digest of the
//! folder's project key (owner decision D7, open), never the key itself or a
//! path. Each row also keeps its transcript's earliest and latest recorded
//! event, by the rule a saved snapshot's time evidence uses, for the Sessions
//! drill's date and span; a row stored before those were kept is read once
//! more. The key is the daemon's own, in the OS keychain behind the
//! `DigestKeyStore` trait (owner decision D16, open); tests use an in-memory
//! store and never reach a keychain. The store and its key are removed on
//! unenroll.
//!
//! The person's own labels on their sessions (task and outcome,
//! `insights::session_labels`; Insights PM request, 2026-10-21), set through
//! `insights_session_label`, live in their own file beside the store
//! ([`LabelFile`]): they are the one thing here that cannot be read again
//! from the sessions on disk, and the store is begun again whenever it
//! cannot be read. Each is keyed by its row's keyed session digest, which a
//! row made again when its file grows keeps, so a label outlives the row's
//! new reference. A label goes with its row: when the row ages out of the
//! kept weeks, the caps evict it, or its folder is Never (no label is set on
//! a row in a Never folder, and the next pass drops one whose folder became
//! Never). A store begun again keeps every label while its rows come back
//! (`CounterPass::keep_labels`). Unenroll removes the labels file with the
//! store. Labels are never sent anywhere: the Sessions rows carry them back
//! to the shell, and the Models answers read them.
//!
//! Anyone holding both the store and the key could confirm a guessed path;
//! a store copied without the key reveals none.
//!
//! # What crosses the socket
//!
//! The week's rollup: counts, dates, coverage reasons, declared model labels
//! and fixed labels. Each session row also carries its first recorded
//! event's time (owner question Q1, default taken) and, while the ledger
//! feed is on and a proxy ledger is declared, its route tally as fixed
//! labels and counts, joined here by the keyed harness-session digest
//! (owner decision D15, extended to feed T). Never a session or project
//! digest, a path, a message or session ID, a backend or model name, a price
//! or a title. Each row also carries its Sessions row's short `t{seq}`
//! name (the reference the Sessions methods take adds the store's tag) and
//! resolved folder display label, plus the latest recorded event time when
//! available (activity, not completion). Nothing here
//! is logged except fixed labels.
//!
//! The Sessions tab reads the same rows through `insights_sessions` and
//! `insights_session_drill` (`daemon::insights_sessions`): each listed row
//! carries an opaque reference (`t` and the order the row was made in, the
//! engine's own `session_ref`, then `.` and the store's tag, so a reference
//! from a store begun again names nothing), its harness, first event time,
//! turn count
//! and its folder's display label, resolved at answer time from the folder
//! digest and never stored; the drill is the saved-snapshot drill's shape.
//! Still never a digest, a path or a key.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Datelike, Duration, FixedOffset, NaiveDate, Offset, Utc, Weekday};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_usage_series::{
    DigestKey, DigestKeyError, DigestKeyStore, InMemoryDigestKeyStore, KeyedDigest, UsageSeries,
    harness_session_digest, key_fingerprint, project_digest, session_key,
};

use super::inference_map::Speakers;
use super::insights_route_tally::{
    FoldCursor, FoldSummary, RouteTally, fold_into, fold_plan, prune, routing_value, session_charge,
};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use crate::config::ConfigStore;
use crate::insights::SourceFormat;
use crate::insights::analytics_constants::PATTERN_BAR_WEEKS;
use crate::insights::analytics_constants::{
    CODEX_PER_TURN_SERIES, COUNTER_PASS_EXCLUDES_NEVER_FOLDERS, COUNTER_PASS_MAX_BYTES_PER_TICK,
    COUNTER_PASS_MAX_LINE_BYTES, COUNTER_PASS_MAX_READS_PER_TICK, COUNTER_STORE_KEEP_WEEKS,
    COUNTER_STORE_MAX_AUTOMATED_SESSIONS, COUNTER_STORE_MAX_CODEX_SERIES_TOOL_CALLS,
    COUNTER_STORE_MAX_CODEX_SERIES_TURNS, COUNTER_STORE_MAX_SESSIONS,
    COUNTER_STORE_MAX_SPAWNED_SESSIONS, COUNTER_STORE_MAX_SPAWNED_TOOL_CALLS,
    COUNTER_STORE_MAX_SPAWNED_TURNS, COUNTER_STORE_MAX_TOOL_CALLS, COUNTER_STORE_MAX_TURNS,
    DIGEST_KEY_CUSTODY, DigestKeyCustody, LEDGER_MONEY_IN_INSIGHTS, SESSION_FACTS_VERSION,
};
use crate::insights::codex_series::{CodexTurnSeriesEvidence, CodexTurnSeriesScan};
use crate::insights::goals::WeekFigures;
use crate::insights::model_answers::ModelRoute;
use crate::insights::session_facts::{ClaudeFactsScan, InferredTask, SessionFacts};
use crate::insights::session_identity::HarnessSessionScan;
use crate::insights::session_labels::{LabelledTask, SessionLabel, SessionOutcome};
use crate::insights::slash_goals::{ClaudeGoal, ClaudeGoalScan, GoalStatus, WeekGoal, WeekGoals};
use crate::insights::task_classifier::{current_task, reclassify};
use crate::insights::time_evidence::RecordedTimeScan;
use crate::insights::turn_series::ClaudeTurnSeriesScan;
use crate::insights::usage::NativeTokenCounts;
use crate::insights::usage_evidence::{CodexUsageScan, PersistedUsageEvidence};
use crate::insights::week_glance::{counter_overview, dated_weeks};
use crate::insights::week_patterns::{week_figures, week_patterns};
use crate::insights::week_rollup::{
    AnalyticsSource, CodexObserved, Feed, SessionBody, SessionInput, UnknownReason, WeekRollup,
    change_vs_last_week, comparable, local_week_start, week_rollup,
};
use crate::source::{SOURCE_CLAUDE_CODE, SOURCE_CODEX};

/// The feed label every answer carries, so a shell can say which feed it
/// shows.
pub const FEED_COUNTER_PASS: &str = "counter_pass";
/// The store, in the daemon directory.
pub const COUNTER_ROWS_FILE: &str = "insights-counter-rows.json";
/// The person's own labels on their sessions, beside the store
/// ([`LabelFile`]).
pub const SESSION_LABELS_FILE: &str = "insights-session-labels.json";
/// The labels file's schema. Any other name is read all the same, by the
/// fields this build knows.
const SESSION_LABELS_SCHEMA: &str = "trace_commons.insights_session_labels.v1";
/// Where the key lives under the key-file custody (owner decision D16).
pub const COUNTER_KEY_DIR: &str = "insights-counter-key";
/// v2 adds each row's keyed harness-session digest (owner decision D15,
/// extended to feed T).
pub const COUNTER_STORE_SCHEMA: &str = "trace_commons.insights_counter_rows.v2";
/// The schema before it. A v1 store still loads and is upgraded in place:
/// its rows keep their counters, each one whose session is still found is
/// read once more for its session digest, a row whose session is gone keeps
/// none until it ages out, and the next pass writes the store back as v2.
const COUNTER_STORE_SCHEMA_V1: &str = "trace_commons.insights_counter_rows.v1";
/// `iso_week` not a `YYYY-Www` string naming a real ISO week.
pub const ERR_ISO_WEEK_INVALID: &str = "iso-week-invalid";
/// The digest key could not be read; nothing is shown in its place.
pub const UNREADABLE_KEY: &str = "key_unavailable";
/// The store could not be read; nothing is shown in its place.
pub const UNREADABLE_STORE: &str = "store_unreadable";
/// The folder rules could not be read, so the Never folders are unknown;
/// nothing is shown in their place.
pub const UNREADABLE_POLICY: &str = "policy_unreadable";
/// `routing_unavailable`: the contributor turned the `insights_ledger_feed`
/// setting off (on by default, owner decision D3, settled 2026-10-09), so
/// no row carries a route.
pub const ROUTING_LEDGER_FEED_OFF: &str = "ledger_feed_off";
/// `routing_unavailable`: no proxy ledger is declared, so no row carries a
/// route.
pub const ROUTING_NO_LEDGER: &str = "no_ledger";
/// The store could not be written; the rows stay as they were.
pub const STORE_WRITE_FAILED: &str = "store_write_failed";
/// The store could not be removed on unenroll.
pub const ERR_CLEAR_FAILED: &str = "insights-counter-clear-failed";

/// One stored session's counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum RowBody {
    Claude {
        /// Declared labels the turns' `model_label_ix` index into.
        model_labels: Vec<String>,
        series: UsageSeries,
    },
    Codex {
        observed: Option<CodexObserved>,
        /// The per-turn series (owner decision D11, settled 2026-10-20).
        /// `None` when the per-turn reader could not read the transcript,
        /// or the row was made before it ran; `observed` stands either way.
        #[serde(default)]
        series: Option<CodexTurnSeriesEvidence>,
        /// Whether `series` was dropped by the Codex series budget
        /// (`COUNTER_STORE_MAX_CODEX_SERIES_TURNS`), oldest first: the row
        /// and `observed` stay, and it is not read again for it. Left out of
        /// the file while false.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        series_trimmed: bool,
    },
    Unknown {
        source: AnalyticsSource,
        reason: UnknownReason,
    },
}

/// Whether a stored session was started by a tool rather than by the
/// person (owner decision, 2026-10-20, option (b)). Told from the
/// transcript when the row is read, never guessed: `Unknown` when it cannot
/// be told, and `Unknown` counts as the person's, so the contributor's own
/// work is never set aside by a guess.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Automation {
    #[default]
    Unknown,
    /// The transcript names how it was started, and not by its harness.
    Person,
    /// Started by its harness: a Codex session whose first `session_meta`
    /// names a `subagent` source (the automatic approvals reviewer, or a
    /// thread an agent spawned).
    Automated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterRow {
    /// The observation the row was made from, so an unchanged file is not
    /// read again.
    size_bytes: u64,
    modified_at: DateTime<Utc>,
    /// Discovery's `started_at`, which places a session with no dated turns.
    /// For Claude Code and Codex that is the session file's last write, not
    /// its first event (discovery reads no file), so it never crosses as
    /// `started_at`: `first_event_at` does. Never the time it was counted.
    placed_at: Option<DateTime<Utc>>,
    /// The transcript's first recorded event: the first line's top-level
    /// `timestamp`, the rule the Claude Code and Codex sources date a queue
    /// entry by. `None` when no line carries one, or the file could not be
    /// read. The one time a row's `started_at` crosses as (owner question
    /// Q1, default taken).
    #[serde(default)]
    first_event_at: Option<DateTime<Utc>>,
    /// Keyed digest of the folder's project key (owner decision D7, open).
    project: KeyedDigest,
    /// Order the row was made in, for the rollup's overlap rule.
    seq: u64,
    body: RowBody,
    /// Keyed digest of the one session ID the transcript records, under the
    /// counter key, so a proxy ledger row can be joined to this row inside
    /// the daemon (owner decision D15, extended to feed T: the keyed
    /// harness-session digest under the counter key is feed T's join key).
    /// `None` when the file records none, more than one, or could not be
    /// read. Stored only; never on the wire (only the tally it joins to
    /// crosses, as counts), and never the engine's
    /// `SessionInput::harness_session`, so the overlap rule is unchanged.
    #[serde(default)]
    harness_session: Option<KeyedDigest>,
    /// Whether `harness_session` (and `first_event_at` with it) was looked
    /// for when the row was read. A v1
    /// row lacks it and is read once more, even when its file is unchanged;
    /// a v2 row that found none is not, so an ambiguous file is not read on
    /// every pass.
    #[serde(default)]
    harness_session_read: bool,
    /// The transcript's earliest and latest recorded event, by the rule a
    /// saved snapshot's time evidence uses
    /// (`insights::time_evidence::extract_recorded_time_evidence`), so the
    /// Sessions drill's date and span match a saved snapshot's for the same
    /// bytes. `None` when no record carries a valid time, or the file could
    /// not be read or parsed. Never on the wire except as the drill's date
    /// and span, and as `insights_sessions`' `started_at`.
    #[serde(default)]
    earliest_event_at: Option<DateTime<Utc>>,
    #[serde(default)]
    latest_event_at: Option<DateTime<Utc>>,
    /// Whether the two times above were looked for when the row was read. A
    /// row made before they were kept lacks it and is read once more, within
    /// the per-poll budget, even when its file is unchanged.
    #[serde(default)]
    event_span_read: bool,
    /// Whether the row was made by the line-by-line read, which skips no
    /// file for its size. A row stored as unknown with no usage counters
    /// before it may be a file the earlier whole-file read skipped for being
    /// over 16 MiB, so it is read once more, within the per-poll budget,
    /// even when its file is unchanged. Any other row is not.
    #[serde(default)]
    streamed: bool,
    /// The session's facts beyond its tokens: counts and fixed labels, each
    /// model by its index into the body's declared labels. `None` when the
    /// facts scan could not read the transcript, the row is unknown, or it
    /// was made before facts were kept. Never a path, a text, a command or
    /// a price. Left out of the file while `None`, as are the three fields
    /// after it at their defaults, so a row with none of them is written
    /// as an older daemon wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    facts: Option<SessionFacts>,
    /// The `SESSION_FACTS_VERSION` the row was read under, set on every row
    /// a pass makes, `facts` or not, so a session the scan cannot read is
    /// not read on every poll. A row under another version (`0`: made
    /// before facts were kept) is read once more, within the per-poll
    /// budget, even when its file is unchanged.
    #[serde(default, skip_serializing_if = "is_zero")]
    facts_version: u32,
    /// Whether the session was started by a tool (`Automation`). An
    /// automated row is kept under its own allowance
    /// (`COUNTER_STORE_MAX_AUTOMATED_SESSIONS`), outside the shared caps,
    /// and the Models answers, the Patterns cards and the Sessions feed
    /// leave it out; the weekly token figures count it.
    #[serde(default, skip_serializing_if = "Automation::is_unknown")]
    automation: Automation,
    /// Whether `automation` was looked for when the row was read. A Codex
    /// row made before it lacks it and is read once more, within the
    /// per-poll budget, even when its file is unchanged. Claude Code has no
    /// marker, so its rows are never read again for it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    automation_read: bool,
    /// Why the whole-file analysis Analyze & save runs on the session's
    /// file (`insights::analyze_file`) would refuse it, as the pass found
    /// the file. `None` when it would take it, or the row was made before
    /// this was looked for: such a file is refused, if at all, when Analyze
    /// & save runs. Left out of the file while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    not_analyzable: Option<NotAnalyzable>,
    /// The goals set with `/goal` in a Claude session, each as its latest
    /// `goal_status` record left it: status, checks, tokens, time and when
    /// (`insights::slash_goals`). Never the goal's condition or the reason
    /// Claude gave. Read with the facts, so a row made before goals were
    /// kept is read again once (`SESSION_FACTS_VERSION`). Left out of the
    /// file while empty. Codex goals are not stored: they are read from
    /// Codex's own database when the week is answered.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    goals: Vec<ClaudeGoal>,
}

/// Why Analyze & save cannot analyze a listed session's file: the core's
/// whole-file analysis refuses it, so every attempt would fail the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotAnalyzable {
    /// Larger than the analysis reads (`insights::MAX_SOURCE_BYTES`).
    TooLarge,
    /// A Codex file that does not record exactly one `session_meta`.
    NotOneSession,
    /// A file Analyze & save does not find or read: a session in a NEAR
    /// AI managed profile, or a compressed Codex rollout.
    InsightsOnly,
}

impl NotAnalyzable {
    /// Why a session file of `bytes` bytes, with `session_metas` Codex
    /// session records when it is Codex's, would be refused.
    fn of(
        source: AnalyticsSource,
        insights_only: bool,
        bytes: Option<u64>,
        session_metas: Option<usize>,
    ) -> Option<Self> {
        if insights_only {
            Some(NotAnalyzable::InsightsOnly)
        } else if bytes.is_some_and(|bytes| bytes > crate::insights::MAX_SOURCE_BYTES) {
            Some(NotAnalyzable::TooLarge)
        } else if source == AnalyticsSource::Codex && session_metas.is_some_and(|count| count != 1)
        {
            Some(NotAnalyzable::NotOneSession)
        } else {
            None
        }
    }
}

/// For `skip_serializing_if`.
fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// For `skip_serializing_if`.
fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}

impl Automation {
    /// For `skip_serializing_if`.
    fn is_unknown(&self) -> bool {
        *self == Automation::Unknown
    }
}

impl CounterRow {
    /// Kept under the automated allowance, outside the shared caps. An
    /// unknown row is the person's.
    fn is_automated(&self) -> bool {
        self.automation == Automation::Automated
    }

    /// A Codex row made before `automation` was looked for.
    fn awaiting_classification(&self) -> bool {
        !self.automation_read
            && matches!(
                self.body,
                RowBody::Codex { .. }
                    | RowBody::Unknown {
                        source: AnalyticsSource::Codex,
                        ..
                    }
            )
    }

    /// The row's short name: the engine's own `session_ref`, `t` and the
    /// order the row was made in, as the week's rows carry it. Never a
    /// digest or a path. A row read again when its file grew is made again
    /// under a new one. The order begins again with the store, so the
    /// Sessions methods name a row by [`Self::listed_reference`] instead.
    fn reference(&self) -> String {
        format!("t{}", self.seq)
    }

    /// The opaque reference the Sessions methods list and take: the short
    /// name, `.`, and the tag of the store it was answered from
    /// (`store_tag_of`), as the paging cursor names it. A store begun again
    /// numbers its rows from `t0` again, so without the tag a reference
    /// answered before would name another session now; with it, the
    /// reference is refused as unknown ([`find_listed`]).
    fn listed_reference(&self, store_tag: u64) -> String {
        format!("{}.{store_tag}", self.reference())
    }

    /// The task the rules infer from the stored facts, re-run when they
    /// were read under older rules, as the Models answers read it; `None`
    /// with no facts.
    fn inferred_task(&self) -> Option<InferredTask> {
        self.facts.as_ref().map(current_task)
    }

    /// The time a Sessions row is listed and labelled by: its earliest
    /// recorded event, the drill's date; for a row made before that was
    /// kept, its first event.
    fn listed_at(&self) -> Option<DateTime<Utc>> {
        self.earliest_event_at.or(self.first_event_at)
    }

    /// Turns against the shared cap: Claude Code's series only. A Codex
    /// series counts against its own budget (`codex_series_turns`).
    fn turns(&self) -> usize {
        match &self.body {
            RowBody::Claude { series, .. } => series.turns.len(),
            _ => 0,
        }
    }

    /// Tool calls against the shared cap: Claude Code's series only.
    fn tool_calls(&self) -> usize {
        match &self.body {
            RowBody::Claude { series, .. } => series.tool_calls.len(),
            _ => 0,
        }
    }

    /// Turns against the Codex series budget.
    fn codex_series_turns(&self) -> usize {
        match &self.body {
            RowBody::Codex {
                series: Some(evidence),
                ..
            } => evidence.series.turns.len(),
            _ => 0,
        }
    }

    /// Tool calls against the Codex series budget.
    fn codex_series_tool_calls(&self) -> usize {
        match &self.body {
            RowBody::Codex {
                series: Some(evidence),
                ..
            } => evidence.series.tool_calls.len(),
            _ => 0,
        }
    }

    fn session_input(&self) -> SessionInput {
        self.session_input_with(CODEX_PER_TURN_SERIES)
    }

    /// The per-turn series the Sessions list draws: Claude Code's, or
    /// Codex's while `codex_per_turn` (`CODEX_PER_TURN_SERIES`, owner
    /// decision D11) holds.
    fn listed_series(&self, codex_per_turn: bool) -> Option<&UsageSeries> {
        match &self.body {
            RowBody::Claude { series, .. } => Some(series),
            RowBody::Codex {
                series: Some(evidence),
                ..
            } if codex_per_turn => Some(&evidence.series),
            _ => None,
        }
    }

    /// `session_input`, with `codex_per_turn` for `CODEX_PER_TURN_SERIES`
    /// (owner decision D11): held, a stored Codex series is not served, and
    /// the session reads as one saved before the per-turn reader.
    fn session_input_with(&self, codex_per_turn: bool) -> SessionInput {
        SessionInput {
            // The order the row was made in, so the engine can tell the
            // sessions apart. Never a digest; every wire form drops it.
            session_ref: self.reference(),
            import_seq: self.seq,
            placed_at: self.placed_at,
            // Feed T rows are one per session address, and the overlap rule
            // does not run here: no session ID is needed.
            harness_session: None,
            automated: self.is_automated(),
            body: match &self.body {
                RowBody::Claude {
                    model_labels,
                    series,
                } => SessionBody::Claude {
                    series: series.clone(),
                    model_labels: model_labels.clone(),
                },
                RowBody::Codex {
                    observed, series, ..
                } => SessionBody::Codex {
                    observed: *observed,
                    series: series.clone().filter(|_| codex_per_turn),
                },
                RowBody::Unknown { source, reason } => SessionBody::Unknown {
                    source: Some(*source),
                    reason: *reason,
                },
            },
            // Facts read under older task rules are re-classified from
            // their stored shape here, at answer time, not read again.
            facts: self.facts.clone().map(|mut facts| {
                reclassify(&mut facts);
                facts
            }),
        }
    }
}

/// The tag a `insights_sessions` cursor carries: the first four bytes of
/// the key's fingerprint, exclusive-or the store's generation, as a number
/// of at most ten digits (no digest-like run crosses). A new key (a new
/// device) or a store begun again under the same key (an unreadable file, a
/// clear) is a new tag, so a cursor from before is refused. A store written
/// before generations reads as generation `0`, so its tag, and the cursors
/// already answered from it, are unchanged until it is begun again.
fn store_tag_of(key: &DigestKey, store: &CounterStore) -> u64 {
    let bytes = key_fingerprint(key).0;
    let key_part = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    u64::from(key_part ^ store.generation)
}

/// A store's generation: random and never `0`, the value a store written
/// before generations reads as.
fn new_generation() -> u32 {
    (uuid::Uuid::new_v4().as_u128() as u32).max(1)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterStore {
    schema: String,
    /// Names the key every digest here was made under, without revealing it.
    key_fingerprint: KeyedDigest,
    next_seq: u64,
    rows: BTreeMap<KeyedDigest, CounterRow>,
    /// Each session's route tally, keyed by its harness-session digest
    /// under this store's key (owner question Q3 and decision D15 extended,
    /// defaults taken), with its NEAR AI charge per model. Apart from `rows`
    /// because the ledger's calls arrive before the quiet-period pass reads
    /// the transcript. Absent in a store written before tallies, which loads
    /// with none; a tally written before charges loads with no charge.
    #[serde(default)]
    routing: BTreeMap<KeyedDigest, RouteTally>,
    /// The highest ledger row id folded into `routing`; `None` before any.
    #[serde(default)]
    ledger_folded_through: Option<i64>,
    /// The latest `started_at` of any ledger row folded into `routing`;
    /// `None` before any. With the id, it tells a window whose newest row
    /// aged out from a ledger that started over (see
    /// `insights_route_tally::fold_plan`).
    #[serde(default)]
    ledger_folded_started: Option<DateTime<Utc>>,
    /// Where the folds have reached in each managed route's own ledger
    /// (`<config>/managed-routes/<uuid>/ledger.sqlite`), by the route's
    /// UUID: each numbers its rows on its own, so each has its own cursor,
    /// and a call is taken once by its route and id. Absent in a store
    /// written before managed routes were folded, and left out while empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    managed_folded: BTreeMap<String, StoredCursor>,
    /// The NEAR AI charge of calls no session could be named for (an
    /// opencode session through a managed route, a call with no session
    /// ID), by the hour they started in: counted in `near_ai_credit`'s sums
    /// and its unknown groups (product owner, 2026-10-28). Aged out by the
    /// hour with the tallies. Left out of the file while there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    loose_charge: Option<super::insights_route_tally::NearAiCharge>,
    /// Which fold rules `loose_charge` was kept under: below
    /// [`LOOSE_EPOCH`], the calls folds passed over before skipped calls'
    /// charges were kept are taken once more, for their charge only.
    #[serde(default, skip_serializing_if = "is_zero_epoch")]
    loose_epoch: u32,
    /// Sessions the caps evicted, by the same keyed session digest as
    /// `rows`, with the observation their row was made from. A candidate
    /// with no row whose last write is no later than the newest of these is
    /// not read; without this an evicted file would look new on every poll,
    /// be read whole as the poll's first read, and be evicted again. Aged
    /// out with the rows, and capped, oldest going first, so the newest is
    /// kept while any is. Absent in a
    /// store written before it, and left out of the file while empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    evicted: BTreeMap<KeyedDigest, Evicted>,
    /// Names this store, apart from its key: drawn at random when the
    /// store is begun, so a store begun again under the same key (an
    /// unreadable file, a clear) is another store, and an
    /// `insights_sessions` cursor from before is refused rather than read
    /// against references that now name other sessions. `0` in a store
    /// written before it.
    #[serde(default)]
    generation: u32,
    /// Claude sub-agent transcripts' rows ([`SpawnedRow`]), keyed by the
    /// same keyed digest of the harness and the transcript's address as
    /// `rows`, apart from them so no answer over the person's sessions can
    /// count one. Left out of the file while empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    spawned: BTreeMap<KeyedDigest, SpawnedRow>,
    /// The order the next sub-agent row is made in.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    spawned_next_seq: u64,
    /// The newest last write of a sub-agent transcript the allowance
    /// evicted: one with no row no newer than it is not read, or it would
    /// be read and evicted on every poll. Aged out with the rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spawned_floor: Option<DateTime<Utc>>,
}

/// The fold rules `loose_charge` is kept under (`CounterStore::loose_epoch`).
const LOOSE_EPOCH: u32 = 1;

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_epoch(epoch: &u32) -> bool {
    *epoch == 0
}

/// A [`FoldCursor`] as the store keeps it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredCursor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    through: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    started: Option<DateTime<Utc>>,
}

impl From<StoredCursor> for FoldCursor {
    fn from(cursor: StoredCursor) -> Self {
        FoldCursor {
            through: cursor.through,
            started: cursor.started,
        }
    }
}

impl From<FoldCursor> for StoredCursor {
    fn from(cursor: FoldCursor) -> Self {
        StoredCursor {
            through: cursor.through,
            started: cursor.started,
        }
    }
}

/// Each managed route's rows by its UUID, as
/// `ManagedHistory::rows_by_home` hands them out.
pub(crate) type ManagedRows<'a> = BTreeMap<String, Vec<&'a crate::routing::RoutedExchange>>;

/// The observation an evicted session's row was made from. No path and no
/// counters: only what tells an unchanged file from a changed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Evicted {
    size_bytes: u64,
    modified_at: DateTime<Utc>,
    /// Which allowance evicted it: `Some(true)` the automated one,
    /// `Some(false)` the shared caps. `None` in a store written before the
    /// automated allowance; those marks are dropped at the next write,
    /// since the session they name may have been pushed out by automated
    /// rows and should be read again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    automated: Option<bool>,
}

impl CounterStore {
    fn fold_cursor(&self) -> FoldCursor {
        FoldCursor {
            through: self.ledger_folded_through,
            started: self.ledger_folded_started,
        }
    }

    fn empty(key_fingerprint: KeyedDigest) -> Self {
        Self {
            schema: COUNTER_STORE_SCHEMA.to_string(),
            key_fingerprint,
            next_seq: 0,
            rows: BTreeMap::new(),
            routing: BTreeMap::new(),
            ledger_folded_through: None,
            ledger_folded_started: None,
            managed_folded: BTreeMap::new(),
            loose_charge: None,
            loose_epoch: LOOSE_EPOCH,
            evicted: BTreeMap::new(),
            generation: new_generation(),
            spawned: BTreeMap::new(),
            spawned_next_seq: 0,
            spawned_floor: None,
        }
    }
}

/// The person's own labels on their sessions (`insights_session_label`), in
/// their own file beside the store ([`SESSION_LABELS_FILE`]). Apart from the
/// store because they are the one thing in it that cannot be read again from
/// the sessions on disk: the store is begun again whenever it cannot be read
/// (a build that wrote a field this one does not know, a damaged file), and
/// that must never take a label with it.
///
/// So this file is read by the fields this build knows and no others: an
/// unknown field, a task or outcome word from a later build, or a later
/// schema name is no reason to refuse it. Each label is kept as written, so
/// one this build cannot read (a later build's word) is answered as unset
/// and kept for the build that can, and a change sets only the field it
/// names. A file that is not this shape at all is never written over: its
/// labels are unset in every answer and a label write is refused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LabelFile {
    schema: String,
    /// The key the session digests below were made under, as the store
    /// names its own. Under any other key they name no session, and the
    /// next write begins the file again.
    key_fingerprint: KeyedDigest,
    /// The store generation the labels were last kept against: while the
    /// store holds another, it was begun again and its rows are still
    /// coming back, so no label is dropped for a row not yet read again
    /// (`CounterPass::keep_labels`).
    #[serde(default)]
    generation: u32,
    /// Each label as written (`task`, `outcome`), by the same keyed session
    /// digest as the store's rows, so a row made again under a new
    /// reference keeps its label.
    #[serde(default)]
    labels: BTreeMap<KeyedDigest, serde_json::Value>,
    /// Anything else a later build wrote, kept as it was.
    #[serde(flatten)]
    rest: serde_json::Map<String, serde_json::Value>,
}

impl LabelFile {
    fn empty(key_fingerprint: KeyedDigest, generation: u32) -> Self {
        Self {
            schema: SESSION_LABELS_SCHEMA.to_string(),
            key_fingerprint,
            generation,
            labels: BTreeMap::new(),
            rest: serde_json::Map::new(),
        }
    }

    /// The labels this build can read, by session; one with nothing it
    /// knows set is left out.
    fn readable(&self) -> BTreeMap<KeyedDigest, SessionLabel> {
        self.labels
            .iter()
            .map(|(session, stored)| (*session, read_label(stored)))
            .filter(|(_, label)| !label.is_empty())
            .collect()
    }
}

/// A label as the labels file keeps it, read by the words this build knows:
/// any other word, or a field it does not know, is unset.
fn read_label(stored: &serde_json::Value) -> SessionLabel {
    let word = |name: &str| stored.get(name).and_then(serde_json::Value::as_str);
    SessionLabel {
        task: word("task").and_then(LabelledTask::from_wire),
        outcome: word("outcome").and_then(SessionOutcome::from_wire),
    }
}

/// The labels file as last read or written.
#[derive(Debug, Clone)]
enum LabelsRead {
    /// No file: no label set since the store's key was made, or since a
    /// clear.
    Absent,
    Read(LabelFile),
    /// A file not in the labels shape: answered as no labels, never
    /// written over.
    Unreadable,
}

/// One session the watcher found, as the pass sees it.
#[derive(Debug, Clone)]
pub(crate) struct CounterCandidate {
    /// The caller's own index, for resolving the session's folder.
    pub origin: usize,
    /// The adapter that found it.
    pub source: &'static str,
    pub path: PathBuf,
    /// The whole group's size and latest write, as the watcher judges
    /// quiescence.
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    /// Transcripts the session spawned (Claude's sub-agents, its
    /// workflows' included), not read as part of it: nonzero marks its
    /// facts as having delegated (`delegated_unread`). They are read, if at
    /// all, as rows of their own ([`SpawnedCandidate`]).
    pub group_member_count: u32,
    /// Where the watcher found it.
    pub role: CandidateRole,
}

/// Where a counter-pass candidate was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateRole {
    /// Through the watched sources, as offers are.
    Watched,
    /// In a NEAR AI managed profile (`<state>/managed-profiles/<id>/`),
    /// which only this pass reads. The one copy of its session counted: a
    /// watched copy of the same session (the post-exit export) is passed
    /// over. See `superseded_by_profiles`.
    ManagedProfile,
}

/// How a Claude sub-agent transcript was started: the session delegated to
/// it (`<session>/subagents/*.jsonl`), or a workflow the session ran did
/// (`<session>/subagents/workflows/<run>/agent-*.jsonl`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SpawnedBy {
    Agent,
    Workflow,
}

impl From<crate::source::claude_code::SpawnedKind> for SpawnedBy {
    fn from(kind: crate::source::claude_code::SpawnedKind) -> Self {
        match kind {
            crate::source::claude_code::SpawnedKind::Agent => SpawnedBy::Agent,
            crate::source::claude_code::SpawnedKind::Workflow => SpawnedBy::Workflow,
        }
    }
}

/// One Claude sub-agent transcript the watcher found beside a candidate.
#[derive(Debug, Clone)]
pub(crate) struct SpawnedCandidate {
    /// Index into the job's candidates of the session that spawned it: a
    /// sub-agent of a session this pass passes over (a managed profile's
    /// copy counts instead) is passed over with it.
    pub parent: usize,
    pub kind: SpawnedBy,
    /// The transcript as a candidate. Its `origin` is the parent's, so its
    /// folder, and whether that folder is Never, are the parent's.
    pub candidate: CounterCandidate,
}

/// A Claude sub-agent transcript's row: kept apart from the person's rows
/// (`CounterStore::spawned`), under its own allowance
/// (`COUNTER_STORE_MAX_SPAWNED_SESSIONS`), and read by Benchmarks' sub-agent
/// population only. Never in a week's totals or spend, the Sessions list,
/// the Models answers, Patterns or goals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnedRow {
    /// The row key of the session that spawned it.
    parent: KeyedDigest,
    kind: SpawnedBy,
    /// Its counters and facts, as a session's row holds them, with no
    /// harness session (a sub-agent records its parent's session ID, so it
    /// would join the parent's route tally), no goals and no `placed_at`.
    row: CounterRow,
}

/// What one pass did. Counts only.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct PassSummary {
    pub read: usize,
    /// Claude sub-agent transcripts read, after the person's own sessions
    /// and within the same per-poll budget.
    pub spawned_read: usize,
    /// Left for a later poll by the per-poll budget.
    pub deferred: usize,
    pub dropped: usize,
    /// Compressed rollouts read whose stream was corrupt or truncated:
    /// stored as unknown, never as zero.
    pub compressed_unreadable: usize,
    /// Compressed rollouts read whose output passed its bound
    /// (`source::compressed`): stored as unknown, never as zero.
    pub compressed_too_large: usize,
}

/// What the daemon's settings add to an `insights_week` answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WeekOptions {
    /// `insights_recap_card_enabled`.
    pub recap_card_enabled: bool,
    /// `insights_context_threshold`; unset, nothing is counted against one.
    pub context_threshold: Option<u32>,
    /// `insights_ledger_feed` (on by default, owner decision D3, settled
    /// 2026-10-09). Off, no row carries a route.
    pub ledger_feed: bool,
    /// Whether a proxy ledger is declared. Absent, no row carries a route.
    pub ledger_present: bool,
}

impl Default for WeekOptions {
    fn default() -> Self {
        Self {
            recap_card_enabled: crate::insights::analytics_constants::RECAP_CARD_DEFAULT_ON,
            context_threshold: None,
            ledger_feed: crate::insights::analytics_constants::LEDGER_FEED_DEFAULT_ON,
            ledger_present: false,
        }
    }
}

/// How a job's sessions are placed in folders: a candidate's project key
/// and whether its folder is Never, the contribution override included.
/// Owns what it needs, so it can run on the pass's own thread; asked, with
/// no lock of the pass held, for a session about to be read, and for one
/// whose file name matches a route tally no row matches or a session the
/// last pass found Never (`NeverSessions`).
pub(crate) type ProjectOf =
    Box<dyn FnMut(&DaemonShared, &CounterCandidate) -> (String, bool) + Send>;

/// One pass to run, made by a watcher tick: owned, so nothing it holds
/// borrows from the tick.
pub(crate) struct CounterJob {
    pub candidates: Vec<CounterCandidate>,
    /// The Claude sub-agent transcripts the candidates' sessions spawned,
    /// read as rows of their own ([`SpawnedRow`]).
    pub spawned: Vec<SpawnedCandidate>,
    pub never_project_keys: NeverFolders,
    pub now: DateTime<Utc>,
    pub quiescence_secs: u64,
    pub project_of: ProjectOf,
}

/// A job as the worker holds it, with the clear count it was made under.
struct QueuedJob {
    job: CounterJob,
    cleared: u64,
}

/// A ledger fold to run on the worker: the ledger's window as the refresh
/// left it, and the time it was read.
struct FoldJob {
    window: Vec<crate::routing::RoutedExchange>,
    /// The managed routes' ledgers, as the daemon's snapshot holds them;
    /// `Err` when they could not be read.
    managed: Result<Option<Arc<crate::routing::managed_history::ManagedHistory>>, ()>,
    now: DateTime<Utc>,
}

/// The background pass's dispatch state. A leaf lock.
#[derive(Default)]
struct WorkerSlot {
    /// A worker thread is running jobs.
    running: bool,
    /// The newest job not yet started. A newer one replaces it.
    pending: Option<QueuedJob>,
    /// The newest ledger fold not yet started. A newer one replaces it: its
    /// window holds every call the older one did that has not aged out,
    /// and the store's cursor takes each call once.
    fold: Option<FoldJob>,
}

/// Hand `job` to the background pass and return at once.
///
/// At most one pass runs at a time. While one runs, the job waits as the
/// one pending job, replacing any older one there, and the running worker
/// takes it up when it finishes: the newest sessions are always read, and
/// ticks that arrive during a long pass leave one follow-up pass, not one
/// each. With no pass running, a worker is started on the runtime's
/// blocking pool, as the settlement scan is; outside a runtime the job runs
/// here.
///
/// The clear count is taken now, so a caller holding `pass_lock` (as the
/// watcher does, and as unenroll does across its clear) makes a job that
/// a later clear voids.
pub(crate) fn submit_counter_pass(shared: &Arc<DaemonShared>, job: CounterJob) {
    let counter = &shared.insights_counter;
    let queued = QueuedJob {
        job,
        cleared: counter.cleared.load(Ordering::SeqCst),
    };
    let start = {
        let mut slot = counter.worker_slot();
        slot.pending = Some(queued);
        !std::mem::replace(&mut slot.running, true)
    };
    if start {
        start_worker(shared);
    }
}

/// Hand a ledger fold to the worker and return at once, as
/// [`submit_counter_pass`] hands it a pass. Folds and passes run one at a
/// time on the one worker, a waiting fold first.
fn submit_ledger_fold(shared: &Arc<DaemonShared>, job: FoldJob) {
    let start = {
        let mut slot = shared.insights_counter.worker_slot();
        slot.fold = Some(job);
        !std::mem::replace(&mut slot.running, true)
    };
    if start {
        start_worker(shared);
    }
}

/// Start the worker on the runtime's blocking pool, or, outside a runtime,
/// run it here.
fn start_worker(shared: &Arc<DaemonShared>) {
    let worker = Arc::clone(shared);
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            // Not awaited: the handle is dropped and the tick goes on.
            drop(handle.spawn_blocking(move || run_counter_jobs(&worker)));
        }
        Err(_) => run_counter_jobs(&worker),
    }
}

/// The worker: run pending jobs until there are none, then stop. Taking the
/// next job and stopping happen under one lock, so a job submitted as the
/// worker finishes either is taken here or starts a new worker.
fn run_counter_jobs(shared: &DaemonShared) {
    let counter = &shared.insights_counter;
    // A panicking job must not leave `running` set, or no pass would run
    // again in this process, nor a job waiting for a worker that is gone.
    struct StopOnPanic<'a>(&'a CounterPass);
    impl Drop for StopOnPanic<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                let mut slot = self.0.worker_slot();
                // The waiting jobs go too: the next tick submits fresh
                // ones, and a test waiting for idle must not hang on them.
                slot.pending = None;
                slot.fold = None;
                self.0.worker_stopped(&mut slot);
            }
        }
    }
    let _guard = StopOnPanic(counter);
    loop {
        let (fold, pass) = {
            let mut slot = counter.worker_slot();
            match slot.fold.take() {
                Some(fold) => (Some(fold), None),
                None => match slot.pending.take() {
                    Some(next) => (None, Some(next)),
                    None => {
                        counter.worker_stopped(&mut slot);
                        return;
                    }
                },
            }
        };
        if let Some(fold) = fold {
            run_fold_job(shared, fold);
        }
        if let Some(pass) = pass {
            run_counter_job(shared, pass);
        }
    }
}

/// Run one job, unless the daemon is stopping, the switch is off or the
/// store was cleared since it was made. The same three stop it between
/// files. Failures are logged inside, by fixed label only; the result is
/// recorded as `last_pass_at`, as it was when the pass ran in the tick.
fn run_counter_job(shared: &DaemonShared, queued: QueuedJob) {
    let counter = &shared.insights_counter;
    let QueuedJob { job, cleared } = queued;
    let should_stop = || {
        shared.shutdown.load(Ordering::Relaxed)
            // A poisoned lock reads as off: fail closed.
            || !shared
                .settings
                .lock()
                .map(|settings| settings.insights_counter_pass)
                .unwrap_or(false)
            || counter.cleared.load(Ordering::SeqCst) != cleared
    };
    if should_stop() {
        return;
    }
    let CounterJob {
        candidates,
        spawned,
        never_project_keys,
        now,
        quiescence_secs,
        mut project_of,
    } = job;
    let mut project_of = |candidate: &CounterCandidate| project_of(shared, candidate);
    let _ = counter.run_pass_from(
        &candidates,
        &spawned,
        &never_project_keys,
        now,
        quiescence_secs,
        &mut project_of,
        cleared,
        &should_stop,
    );
}

/// The sessions in Never folders, by keyed harness-session digest under
/// the counter key, as the last pass found them.
///
/// A Never-folder session is never read, so it has no row and nothing
/// joins its tally to its folder: the pass names it from its file name
/// instead (`name_session_digest`), and the fold drops its tally before
/// any write (owner decision D4: a Never folder leaves nothing).
#[derive(Debug, Clone, Default)]
struct NeverSessions {
    /// A global Never override is in force: no tally is kept at all.
    every: bool,
    these: BTreeSet<KeyedDigest>,
}

impl NeverSessions {
    /// Drop every tally a Never folder's session made.
    fn purge(&self, routing: &mut BTreeMap<KeyedDigest, RouteTally>) {
        if self.every {
            routing.clear();
        } else if !self.these.is_empty() {
            routing.retain(|digest, _| !self.these.contains(digest));
        }
    }

    /// Whether `purge` would drop anything from `routing`.
    fn touches(&self, routing: &BTreeMap<KeyedDigest, RouteTally>) -> bool {
        if self.every {
            !routing.is_empty()
        } else {
            routing.keys().any(|digest| self.these.contains(digest))
        }
    }
}

/// The session ID a transcript's file name carries, read without opening
/// the file, as the keyed harness-session digest a tally is made under.
///
/// - Claude Code: the session file's stem, the `sessionId` its records carry
///   (the address `claude_code` verifies a group's members against).
/// - Codex: the UUID a `rollout-<timestamp>-<uuid>` stem ends with, the
///   `session_meta` id.
///
/// `None` for any other name, so an unknown shape names no session.
fn name_session_digest(key: &DigestKey, candidate: &CounterCandidate) -> Option<KeyedDigest> {
    let id = name_session_id(candidate.source, &candidate.path)?;
    Some(harness_session_digest(key, candidate.source, id))
}

/// The raw session ID `name_session_digest` digests. A compressed Codex
/// rollout names its session as its plain twin does.
fn name_session_id<'p>(source: &str, path: &'p Path) -> Option<&'p str> {
    let address = row_address(source, path);
    let stem = Path::new(address).file_stem()?.to_str()?;
    match source {
        SOURCE_CLAUDE_CODE => Some(stem).filter(|stem| !stem.is_empty()),
        SOURCE_CODEX => {
            let rest = stem.strip_prefix("rollout-")?;
            let id = rest.get(rest.len().checked_sub(36)?..)?;
            is_uuid(id).then_some(id)
        }
        _ => None,
    }
}

/// The address a session's row is keyed by: its file's path, except that a
/// compressed Codex rollout (`<name>.jsonl.zst`) answers as the plain
/// `<name>.jsonl` Codex compressed it from. So a row first read from the
/// plain file is replaced, not doubled, when Codex compresses it in place,
/// and anything that names the session by its plain path (a queue entry)
/// still finds the row. A path that is not UTF-8 is not stripped.
fn row_address<'p>(source: &str, path: &'p Path) -> &'p str {
    let Some(text) = path.to_str() else {
        return "";
    };
    // The adapter's own rule (`source::compressed::address_of`); discovery
    // already hands the address, so this only matters for a path named
    // some other way.
    if source == SOURCE_CODEX && crate::source::compressed::is_compressed(path) {
        text.strip_suffix(crate::source::compressed::COMPRESSED_SUFFIX)
            .unwrap_or(text)
    } else {
        text
    }
}

/// The row key of the session at `path`, as the adapter `source` found it:
/// `session_key` of its `row_address`.
pub(crate) fn row_key(key: &DigestKey, source: &str, path: &Path) -> KeyedDigest {
    match path.to_str() {
        Some(_) => session_key(key, source, row_address(source, path)),
        None => session_key(key, source, &path.to_string_lossy()),
    }
}

/// Whether Analyze & save cannot find or read the candidate's file: a
/// managed profile's session (never among the watched sessions), or a
/// compressed Codex rollout (the whole-file analysis reads plain files).
fn insights_only(candidate: &CounterCandidate) -> bool {
    candidate.role == CandidateRole::ManagedProfile || compressed_file(candidate).is_some()
}

/// Whether the candidate's path names a compressed Codex rollout itself
/// (`<name>.jsonl.zst`) rather than its address.
fn is_compressed_codex(candidate: &CounterCandidate) -> bool {
    candidate.source == SOURCE_CODEX && crate::source::compressed::is_compressed(&candidate.path)
}

/// The compressed file behind a Codex candidate's address, when that is
/// what is on disk (`source::compressed::on_disk`): the adapter addresses a
/// compressed rollout by the plain path it replaced.
fn compressed_file(candidate: &CounterCandidate) -> Option<PathBuf> {
    if candidate.source != SOURCE_CODEX {
        return None;
    }
    crate::source::compressed::on_disk(&candidate.path)
        .filter(|file| crate::source::compressed::is_compressed(file))
}

/// What reading the candidate costs the per-poll budget: its file's size,
/// or for a compressed rollout what it decompresses to. Delegated
/// transcripts are counted in `size_bytes` but never read, so are not
/// charged.
fn read_cost(candidate: &CounterCandidate) -> u64 {
    if let Some(file) = compressed_file(candidate) {
        crate::source::compressed::estimated_decompressed_bytes(&file)
            .unwrap_or(candidate.size_bytes)
    } else {
        std::fs::metadata(&candidate.path)
            .map(|meta| meta.len())
            .unwrap_or(candidate.size_bytes)
    }
}

/// Which candidates this pass leaves unread because another one is the
/// same session (by index into `candidates`), and, for each one whose row
/// should go, the row key of the candidate that replaces it.
///
/// - A compressed Codex rollout and its plain twin share a row key; while
///   both are on disk (Codex between writing one and removing the other)
///   the plain file is read and the compressed one is not.
/// - A session found in a managed profile is counted from there. Any other
///   candidate whose file name names the same session (the post-exit
///   export, or a copy in the home folder) is not read, and its row is
///   dropped once the profile's row exists. The newest profile copy wins
///   among several.
fn superseded_by_profiles(
    key: &DigestKey,
    candidates: &[CounterCandidate],
) -> (BTreeSet<usize>, Vec<(KeyedDigest, KeyedDigest)>) {
    let mut skip = BTreeSet::new();
    let mut by_key: BTreeMap<KeyedDigest, usize> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let row = row_key(key, candidate.source, &candidate.path);
        match by_key.get(&row) {
            Some(&kept)
                if is_compressed_codex(&candidates[kept]) && !is_compressed_codex(candidate) =>
            {
                skip.insert(kept);
                by_key.insert(row, index);
            }
            Some(_) => {
                skip.insert(index);
            }
            None => {
                by_key.insert(row, index);
            }
        }
    }
    let mut profile: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.role != CandidateRole::ManagedProfile || skip.contains(&index) {
            continue;
        }
        let Some(id) = name_session_id(candidate.source, &candidate.path) else {
            continue;
        };
        let slot = profile.entry((candidate.source, id)).or_insert(index);
        if candidates[*slot].modified_at < candidate.modified_at {
            *slot = index;
        }
    }
    let mut replaced = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let Some(id) = name_session_id(candidate.source, &candidate.path) else {
            continue;
        };
        let Some(&keeper) = profile.get(&(candidate.source, id)) else {
            continue;
        };
        if keeper == index {
            continue;
        }
        let own = row_key(key, candidate.source, &candidate.path);
        let keeper_row = row_key(key, candidates[keeper].source, &candidates[keeper].path);
        if own != keeper_row {
            replaced.push((own, keeper_row));
        }
        skip.insert(index);
    }
    (skip, replaced)
}

/// `8-4-4-4-12` hex digits.
fn is_uuid(text: &str) -> bool {
    text.len() == 36
        && text.char_indices().all(|(at, c)| {
            if matches!(at, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// The daemon's counter pass and its store.
pub struct CounterPass {
    rows_path: PathBuf,
    keys: Box<dyn DigestKeyStore + Send + Sync>,
    /// The store as last read or staged; `None` until first needed. The
    /// answers read it; the file follows it (`flush`).
    cached: Mutex<Option<Arc<CounterStore>>>,
    /// Bumped, under the cache lock, each time a changed store is staged.
    staged: AtomicU64,
    /// The writer lock, holding the `staged` count last written to the file.
    writer: Mutex<u64>,
    /// When a pass last finished, for the answer's `updated_at`.
    last_pass_at: Mutex<Option<DateTime<Utc>>>,
    /// The last failure label logged, so a refused keychain is logged once.
    last_failure: Mutex<Option<&'static str>>,
    /// The same, for the ledger fold.
    last_fold_failure: Mutex<Option<&'static str>>,
    /// Bumped by every `clear`, so a fold that read the settings before an
    /// unenroll turned the pass off cannot write the store back after it.
    cleared: AtomicU64,
    /// The background pass: whether one is running, and the one job kept
    /// for after it. See [`submit_counter_pass`].
    worker: Mutex<WorkerSlot>,
    /// The sessions the last pass found in Never folders. In memory only,
    /// never written: a fold drops their tallies before it writes. A leaf
    /// lock.
    never_sessions: Mutex<NeverSessions>,
    /// The labels file as last read or written; `None` until first needed.
    /// A leaf lock, held across a label write, so two writes never
    /// interleave and a cached label is always one the file holds.
    labels: Mutex<Option<LabelsRead>>,
    /// Codex's goals database (`insights_codex_goals`), read when the week
    /// is answered; `None` reads no Codex goals.
    codex_goals_db: Option<PathBuf>,
    /// Signalled when the worker goes idle.
    #[cfg(test)]
    idle: std::sync::Condvar,
    /// Test seams for the background pass: a hook run before each file
    /// read, a count of passes started, and whether a tick leaves the pass
    /// running rather than waiting for it.
    #[cfg(test)]
    test_seams: TestSeams,
}

#[cfg(test)]
#[derive(Default)]
struct TestSeams {
    before_read: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    passes_started: std::sync::atomic::AtomicUsize,
    detached: std::sync::atomic::AtomicBool,
    /// Store files written, and how many of them with the cache lock held.
    writes: std::sync::atomic::AtomicUsize,
    writes_under_cache_lock: std::sync::atomic::AtomicUsize,
    /// The thread the last ledger fold ran on.
    fold_thread: Mutex<Option<std::thread::ThreadId>>,
    /// The sub-agent allowance in place of the constants: sessions, turns,
    /// tool calls.
    spawned_caps: Mutex<Option<(usize, usize, usize)>>,
}

impl CounterPass {
    /// The pass for a daemon over `store`, with the key custody owner
    /// decision D16 (open) names.
    pub fn for_store(store: &ConfigStore) -> Self {
        let mut pass = Self::with_keys(
            store.daemon_path(COUNTER_ROWS_FILE),
            default_counter_keys(store),
        );
        pass.codex_goals_db = super::insights_codex_goals::codex_goals_path();
        pass
    }

    pub(crate) fn with_keys(
        rows_path: PathBuf,
        keys: Box<dyn DigestKeyStore + Send + Sync>,
    ) -> Self {
        Self {
            rows_path,
            keys,
            cached: Mutex::new(None),
            staged: AtomicU64::new(0),
            writer: Mutex::new(0),
            last_pass_at: Mutex::new(None),
            last_failure: Mutex::new(None),
            last_fold_failure: Mutex::new(None),
            cleared: AtomicU64::new(0),
            worker: Mutex::new(WorkerSlot::default()),
            never_sessions: Mutex::new(NeverSessions::default()),
            labels: Mutex::new(None),
            codex_goals_db: None,
            #[cfg(test)]
            idle: std::sync::Condvar::new(),
            #[cfg(test)]
            test_seams: TestSeams::default(),
        }
    }

    /// The sub-agent allowance: sessions, turns, tool calls.
    fn spawned_caps(&self) -> (usize, usize, usize) {
        #[cfg(test)]
        if let Some(caps) = *self
            .test_seams
            .spawned_caps
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            return caps;
        }
        (
            COUNTER_STORE_MAX_SPAWNED_SESSIONS,
            COUNTER_STORE_MAX_SPAWNED_TURNS,
            COUNTER_STORE_MAX_SPAWNED_TOOL_CALLS,
        )
    }

    /// A sub-agent allowance in place of the constants.
    #[cfg(test)]
    pub(crate) fn set_spawned_caps_for_test(&self, sessions: usize, turns: usize, tools: usize) {
        *self
            .test_seams
            .spawned_caps
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((sessions, turns, tools));
    }

    /// Read Codex goals from `path` instead of Codex's home.
    #[cfg(test)]
    pub(crate) fn set_codex_goals_db_for_test(&mut self, path: Option<PathBuf>) {
        self.codex_goals_db = path;
    }

    /// Run `hook` before every file the pass reads.
    #[cfg(test)]
    pub(crate) fn set_before_read_for_test(&self, hook: Arc<dyn Fn() + Send + Sync>) {
        *self.test_seams.before_read.lock().unwrap() = Some(hook);
    }

    /// How many passes have started.
    #[cfg(test)]
    pub(crate) fn passes_started_for_test(&self) -> usize {
        self.test_seams.passes_started.load(Ordering::SeqCst)
    }

    /// Leave the pass running when a tick returns, as the daemon does,
    /// instead of waiting for it.
    #[cfg(test)]
    pub(crate) fn detach_for_test(&self) {
        self.test_seams.detached.store(true, Ordering::SeqCst);
    }

    /// Whether a tick should wait for the pass before it returns.
    #[cfg(test)]
    pub(crate) fn detached_for_test(&self) -> bool {
        self.test_seams.detached.load(Ordering::SeqCst)
    }

    /// Whether a worker is running and whether a job is waiting.
    #[cfg(test)]
    pub(crate) fn worker_state_for_test(&self) -> (bool, bool) {
        let slot = self.worker_slot();
        (slot.running, slot.pending.is_some())
    }

    /// Wait until no pass is running and none is pending.
    #[cfg(test)]
    pub(crate) fn await_idle_for_test(&self) {
        let slot = self.worker.lock().unwrap_or_else(|e| e.into_inner());
        let _idle = self
            .idle
            .wait_while(slot, |slot| {
                slot.running || slot.pending.is_some() || slot.fold.is_some()
            })
            .unwrap_or_else(|e| e.into_inner());
    }

    fn worker_slot(&self) -> std::sync::MutexGuard<'_, WorkerSlot> {
        // A leaf lock that is never held across a call that could panic.
        self.worker.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The worker found nothing more to run and stopped.
    fn worker_stopped(&self, slot: &mut WorkerSlot) {
        slot.running = false;
        #[cfg(test)]
        self.idle.notify_all();
    }

    /// Run one pass in the calling thread, against the store as cleared
    /// now and with nothing to stop it. The daemon's watcher runs passes
    /// through [`submit_counter_pass`] instead.
    #[cfg(test)]
    pub(crate) fn run_pass(
        &self,
        candidates: &[CounterCandidate],
        never_project_keys: &NeverFolders,
        now: DateTime<Utc>,
        quiescence_secs: u64,
        project_of: &mut dyn FnMut(&CounterCandidate) -> (String, bool),
    ) -> Result<PassSummary, &'static str> {
        let cleared = self.cleared.load(Ordering::SeqCst);
        self.run_pass_from(
            candidates,
            &[],
            never_project_keys,
            now,
            quiescence_secs,
            project_of,
            cleared,
            &|| false,
        )
        .map(Option::unwrap_or_default)
    }

    /// [`Self::run_pass`], with the Claude sub-agent transcripts the
    /// candidates spawned, as the watcher's job carries them.
    #[cfg(test)]
    pub(crate) fn run_pass_spawned(
        &self,
        candidates: &[CounterCandidate],
        spawned: &[SpawnedCandidate],
        never_project_keys: &NeverFolders,
        now: DateTime<Utc>,
        quiescence_secs: u64,
        project_of: &mut dyn FnMut(&CounterCandidate) -> (String, bool),
    ) -> Result<PassSummary, &'static str> {
        let cleared = self.cleared.load(Ordering::SeqCst);
        self.run_pass_from(
            candidates,
            spawned,
            never_project_keys,
            now,
            quiescence_secs,
            project_of,
            cleared,
            &|| false,
        )
        .map(Option::unwrap_or_default)
    }

    /// Run one pass. `project_of` resolves a candidate's project key and
    /// whether its folder is Never, the contribution override included; it
    /// is asked for a session about to be read, and for one that may hold a
    /// Never folder's tally (`NeverSessions`). `never_folders` are
    /// every folder that is Never, so rows for sessions no longer on disk
    /// drop too.
    ///
    /// `cleared` is the clear count the job was made under: once a `clear`
    /// moves it, the pass writes nothing and keeps no key. `should_stop` is
    /// asked before each file read and before the write; once it answers
    /// true the pass is abandoned and writes nothing. `Ok(None)` is an
    /// abandoned pass.
    #[allow(clippy::too_many_arguments)]
    fn run_pass_from(
        &self,
        candidates: &[CounterCandidate],
        spawned: &[SpawnedCandidate],
        never_project_keys: &NeverFolders,
        now: DateTime<Utc>,
        quiescence_secs: u64,
        project_of: &mut dyn FnMut(&CounterCandidate) -> (String, bool),
        cleared: u64,
        should_stop: &dyn Fn() -> bool,
    ) -> Result<Option<PassSummary>, &'static str> {
        #[cfg(test)]
        self.test_seams
            .passes_started
            .fetch_add(1, Ordering::SeqCst);
        let result = self.pass_inner(
            candidates,
            spawned,
            never_project_keys,
            now,
            quiescence_secs,
            project_of,
            cleared,
            should_stop,
        );
        let failure = result.as_ref().err().copied();
        if let Ok(mut last) = self.last_failure.lock() {
            if *last != failure {
                if let Some(label) = failure {
                    // A fixed label, never the error: its context can carry
                    // a path.
                    tracing::warn!(label, "insights counter pass skipped");
                }
                *last = failure;
            }
        }
        result
    }

    /// Count a store write, and whether the cache lock is held across it.
    #[cfg(test)]
    fn count_write(&self) {
        self.test_seams.writes.fetch_add(1, Ordering::SeqCst);
        if self.cached.try_lock().is_err() {
            self.test_seams
                .writes_under_cache_lock
                .fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Store files written so far, and how many with the cache lock held.
    #[cfg(test)]
    pub(crate) fn writes_for_test(&self) -> (usize, usize) {
        (
            self.test_seams.writes.load(Ordering::SeqCst),
            self.test_seams
                .writes_under_cache_lock
                .load(Ordering::SeqCst),
        )
    }

    /// The thread the last ledger fold ran on.
    #[cfg(test)]
    pub(crate) fn fold_thread_for_test(&self) -> Option<std::thread::ThreadId> {
        *self.test_seams.fold_thread.lock().unwrap()
    }

    #[cfg(test)]
    fn run_before_read_hook(&self) {
        let hook = self.test_seams.before_read.lock().unwrap().clone();
        if let Some(hook) = hook {
            hook();
        }
    }

    /// The pass. It plans and reads against the store as cached when it
    /// begins, holding no lock, and then writes under the cache lock onto
    /// the store as it stands by then: a ledger fold may have moved its
    /// tallies and cursor meanwhile, and those are kept. Rows themselves
    /// change only here, and only one pass runs at a time.
    #[allow(clippy::too_many_arguments)]
    fn pass_inner(
        &self,
        candidates: &[CounterCandidate],
        spawned: &[SpawnedCandidate],
        never_project_keys: &NeverFolders,
        now: DateTime<Utc>,
        quiescence_secs: u64,
        project_of: &mut dyn FnMut(&CounterCandidate) -> (String, bool),
        cleared: u64,
        should_stop: &dyn Fn() -> bool,
    ) -> Result<Option<PassSummary>, &'static str> {
        let key = self.keys.load_or_create().map_err(|_| UNREADABLE_KEY)?;
        // A clear since the job was made: the key just read may be one this
        // call made after the clear forgot it. Forget it again, and write
        // nothing.
        if self.cleared.load(Ordering::SeqCst) != cleared {
            if self.keys.clear().is_err() {
                tracing::warn!("insights counter key could not be forgotten");
            }
            return Ok(None);
        }
        let fingerprint = key_fingerprint(&key);
        // A store that cannot be read, or was made under another key, is
        // started again: its rows are re-derived from the sessions on disk.
        let (current, restart) = match self.load_cached() {
            Ok(Some(store)) if store.key_fingerprint == fingerprint => (store, false),
            Ok(None) => (Arc::new(CounterStore::empty(fingerprint)), false),
            Ok(Some(_)) | Err(_) => (Arc::new(CounterStore::empty(fingerprint)), true),
        };
        let cutoff = now - Duration::weeks(COUNTER_STORE_KEEP_WEEKS);
        let never = never_digests(&key, never_project_keys);
        let mut summary = PassSummary::default();
        let mut removals: BTreeSet<KeyedDigest> = current
            .rows
            .iter()
            .filter(|(_, row)| never.contains(&row.project) || row.modified_at < cutoff)
            .map(|(session, _)| *session)
            .collect();

        // The caps evict oldest first, so every eviction mark is older than
        // every row kept when it was made: the store keeps the newest
        // sessions only. Two bounds follow, for a candidate with no row:
        // - the newest mark: the caps already evicted a session at least as
        //   new, so one no newer is not read. The mark cap drops the oldest
        //   marks and never this one, so a session whose own mark was
        //   dropped stays below it and is not read again every poll.
        // - with the store at its session cap, the oldest row kept: a
        //   session older than it would be the first the caps evict.
        //
        // Both bounds are the shared caps' own: their marks and the
        // person's rows (unknown ones included). The automated allowance
        // evicts far more often, so its marks would hold the floor near the
        // present and starve an older session of the person's that arrives
        // late; an automated session is kept from being read again by its
        // own unchanged mark instead. Marks made before the allowance
        // (`automated: None`) bound nothing: they may name a session of the
        // person's that automated rows pushed out.
        //
        // While a row this poll finds still awaits its classification, the
        // person's share is not yet known, so neither bound applies and the
        // caps mark no eviction of the person's (see `enforce_caps`).
        let classifying = candidates.iter().any(|candidate| {
            analytics_source(candidate.source).is_some()
                && current
                    .rows
                    .get(&row_key(&key, candidate.source, &candidate.path))
                    .is_some_and(CounterRow::awaiting_classification)
        });
        let eviction_floor = current
            .evicted
            .values()
            .filter(|evicted| evicted.automated == Some(false))
            .map(|evicted| evicted.modified_at)
            .max();
        let kept_person = || {
            current
                .rows
                .iter()
                .filter(|(session, row)| !removals.contains(*session) && !row.is_automated())
        };
        let oldest_kept_at_cap = (kept_person().count() >= COUNTER_STORE_MAX_SESSIONS)
            .then(|| kept_person().map(|(_, row)| row.modified_at).min())
            .flatten();

        // Sessions in Never folders, named from their file names. A tally
        // no row matches may be a Never-folder session's, which is never
        // read and so never has a row; its folder is resolved here, before
        // the quiet-period and unchanged tests below skip the candidate.
        // So is every session the last pass found Never, so one whose
        // tally a fold already dropped stays known. Any other session's
        // folder is resolved only when it is about to be read.
        let every_never = matches!(never_project_keys, NeverFolders::Every);
        let mut never_sessions = NeverSessions {
            every: every_never,
            these: BTreeSet::new(),
        };
        let mut folders: BTreeMap<PathBuf, (String, bool)> = BTreeMap::new();
        if !every_never && COUNTER_PASS_EXCLUDES_NEVER_FOLDERS {
            let known_never = self
                .never_sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .these
                .clone();
            let matched: BTreeSet<KeyedDigest> = current
                .rows
                .values()
                .filter_map(|row| row.harness_session)
                .collect();
            for candidate in candidates {
                if analytics_source(candidate.source).is_none() {
                    continue;
                }
                let Some(digest) = name_session_digest(&key, candidate) else {
                    continue;
                };
                let unmatched_tally =
                    current.routing.contains_key(&digest) && !matched.contains(&digest);
                if !unmatched_tally && !known_never.contains(&digest) {
                    continue;
                }
                let folder = project_of(candidate);
                if folder.1 {
                    never_sessions.these.insert(digest);
                }
                folders.insert(candidate.path.clone(), folder);
            }
        }

        let (skip, replaced) = superseded_by_profiles(&key, candidates);
        let mut order: Vec<&CounterCandidate> = candidates
            .iter()
            .enumerate()
            .filter(|(index, _)| !skip.contains(index))
            .map(|(_, candidate)| candidate)
            .collect();
        order.sort_by_key(|candidate| std::cmp::Reverse(candidate.modified_at));
        let mut reads: Vec<(KeyedDigest, CounterRow)> = Vec::new();
        let mut bytes_budgeted = 0u64;
        // Sessions still being written: a label on one is kept while it
        // waits, though a store begun again holds no row for it yet.
        let mut writing: BTreeSet<KeyedDigest> = BTreeSet::new();
        for candidate in order {
            let Some(source) = analytics_source(candidate.source) else {
                continue;
            };
            if candidate.modified_at < cutoff {
                continue;
            }
            let session = row_key(&key, candidate.source, &candidate.path);
            if now
                .signed_duration_since(candidate.modified_at)
                .num_seconds()
                < i64::try_from(quiescence_secs).unwrap_or(i64::MAX)
            {
                writing.insert(session);
                continue;
            }
            let unchanged = current.rows.get(&session).is_some_and(|row| {
                row.size_bytes == candidate.size_bytes
                    && row.modified_at == candidate.modified_at
                    && row.harness_session_read
                    && row.event_span_read
                    && row.facts_version == SESSION_FACTS_VERSION
                    && !row.awaiting_classification()
                    && (row.streamed
                        || !matches!(
                            row.body,
                            RowBody::Unknown {
                                reason: UnknownReason::NoUsageCounters,
                                ..
                            }
                        ))
            });
            if unchanged && !removals.contains(&session) {
                continue;
            }
            // No newer than a session the caps evicted, its own unchanged
            // file included, or older than every row a full store keeps:
            // reading it would only evict it again.
            let own_mark = current.evicted.get(&session).is_some_and(|evicted| {
                evicted.automated.is_some()
                    && evicted.size_bytes == candidate.size_bytes
                    && evicted.modified_at == candidate.modified_at
            });
            if !current.rows.contains_key(&session)
                && (own_mark
                    || (!classifying
                        && (eviction_floor.is_some_and(|floor| candidate.modified_at <= floor)
                            || oldest_kept_at_cap
                                .is_some_and(|oldest| candidate.modified_at < oldest))))
            {
                continue;
            }
            let (project_key, never_folder) = match folders.remove(&candidate.path) {
                Some(folder) => folder,
                None => project_of(candidate),
            };
            if never_folder && COUNTER_PASS_EXCLUDES_NEVER_FOLDERS {
                if current.rows.contains_key(&session) {
                    removals.insert(session);
                }
                never_sessions
                    .these
                    .extend(name_session_digest(&key, candidate));
                continue;
            }
            // Charged by the one file read (`read_cost`), decompressed. The
            // first read of a poll is always allowed, so a file over the
            // whole budget is read alone rather than starved.
            let cost = read_cost(candidate);
            if reads.len() >= COUNTER_PASS_MAX_READS_PER_TICK
                || (!reads.is_empty()
                    && bytes_budgeted.saturating_add(cost) > COUNTER_PASS_MAX_BYTES_PER_TICK)
            {
                summary.deferred += 1;
                continue;
            }
            // Between files: shutdown, the switch going off or a clear
            // abandons the pass. A file already being read finishes first.
            if should_stop() {
                return Ok(None);
            }
            bytes_budgeted = bytes_budgeted.saturating_add(cost);
            removals.remove(&session);
            #[cfg(test)]
            self.run_before_read_hook();
            let read = read_body(source, candidate, &key);
            match read.compressed_failure {
                Some(crate::source::compressed::CompressedFailure::Undecodable) => {
                    summary.compressed_unreadable += 1;
                }
                Some(crate::source::compressed::CompressedFailure::TooLarge) => {
                    summary.compressed_too_large += 1;
                }
                None => {}
            }
            reads.push((
                session,
                CounterRow {
                    size_bytes: candidate.size_bytes,
                    modified_at: candidate.modified_at,
                    placed_at: candidate.started_at,
                    first_event_at: read.first_event_at,
                    project: project_digest(&key, &project_key),
                    seq: 0,
                    body: read.body,
                    harness_session: read.harness_session,
                    harness_session_read: true,
                    earliest_event_at: read.span.map(|(earliest, _)| earliest),
                    latest_event_at: read.span.map(|(_, latest)| latest),
                    event_span_read: true,
                    streamed: true,
                    facts: read.facts,
                    facts_version: SESSION_FACTS_VERSION,
                    automation: read.automation,
                    automation_read: true,
                    not_analyzable: read.not_analyzable,
                    goals: read.goals,
                },
            ));
        }
        summary.read = reads.len();

        // Claude's sub-agent transcripts, after the person's own sessions and
        // within what is left of the same budget, into their own rows.
        let parent_keys: Vec<KeyedDigest> = candidates
            .iter()
            .map(|candidate| row_key(&key, candidate.source, &candidate.path))
            .collect();
        let passed_over: BTreeSet<KeyedDigest> =
            skip.iter().map(|index| parent_keys[*index]).collect();
        let mut spawned_removals: BTreeSet<KeyedDigest> = current
            .spawned
            .iter()
            .filter(|(_, spawned)| {
                never.contains(&spawned.row.project)
                    || spawned.row.modified_at < cutoff
                    || passed_over.contains(&spawned.parent)
            })
            .map(|(session, _)| *session)
            .collect();
        let spawned_floor = current.spawned_floor.filter(|floor| *floor >= cutoff);
        let mut spawned_order: Vec<&SpawnedCandidate> = spawned
            .iter()
            .filter(|spawned| spawned.parent < candidates.len() && !skip.contains(&spawned.parent))
            .collect();
        spawned_order.sort_by_key(|spawned| std::cmp::Reverse(spawned.candidate.modified_at));
        let mut spawned_reads: Vec<(KeyedDigest, SpawnedRow)> = Vec::new();
        // Each parent's folder, resolved once.
        let mut parent_folders: BTreeMap<usize, (String, bool)> = BTreeMap::new();
        for spawned in spawned_order {
            let candidate = &spawned.candidate;
            if analytics_source(candidate.source) != Some(AnalyticsSource::ClaudeCode)
                || candidate.modified_at < cutoff
                || now
                    .signed_duration_since(candidate.modified_at)
                    .num_seconds()
                    < i64::try_from(quiescence_secs).unwrap_or(i64::MAX)
            {
                continue;
            }
            let session = row_key(&key, candidate.source, &candidate.path);
            let stored = current.spawned.get(&session);
            let unchanged = stored.is_some_and(|stored| {
                stored.row.size_bytes == candidate.size_bytes
                    && stored.row.modified_at == candidate.modified_at
                    && stored.row.facts_version == SESSION_FACTS_VERSION
            });
            if unchanged && !spawned_removals.contains(&session) {
                continue;
            }
            if stored.is_none() && spawned_floor.is_some_and(|floor| candidate.modified_at <= floor)
            {
                continue;
            }
            let (project_key, never_folder) = parent_folders
                .entry(spawned.parent)
                .or_insert_with(|| project_of(candidate))
                .clone();
            if never_folder && COUNTER_PASS_EXCLUDES_NEVER_FOLDERS {
                if stored.is_some() {
                    spawned_removals.insert(session);
                }
                continue;
            }
            let cost = read_cost(candidate);
            let read_so_far = reads.len() + spawned_reads.len();
            if read_so_far >= COUNTER_PASS_MAX_READS_PER_TICK
                || (read_so_far > 0
                    && bytes_budgeted.saturating_add(cost) > COUNTER_PASS_MAX_BYTES_PER_TICK)
            {
                summary.deferred += 1;
                continue;
            }
            if should_stop() {
                return Ok(None);
            }
            bytes_budgeted = bytes_budgeted.saturating_add(cost);
            spawned_removals.remove(&session);
            let read = read_body(AnalyticsSource::ClaudeCode, candidate, &key);
            spawned_reads.push((
                session,
                SpawnedRow {
                    parent: parent_keys[spawned.parent],
                    kind: spawned.kind,
                    row: CounterRow {
                        size_bytes: candidate.size_bytes,
                        modified_at: candidate.modified_at,
                        placed_at: None,
                        first_event_at: read.first_event_at,
                        project: project_digest(&key, &project_key),
                        seq: 0,
                        body: read.body,
                        harness_session: None,
                        harness_session_read: true,
                        earliest_event_at: read.span.map(|(earliest, _)| earliest),
                        latest_event_at: read.span.map(|(_, latest)| latest),
                        event_span_read: true,
                        streamed: true,
                        facts: read.facts,
                        facts_version: SESSION_FACTS_VERSION,
                        automation: Automation::Unknown,
                        automation_read: true,
                        not_analyzable: None,
                        goals: Vec::new(),
                    },
                },
            ));
        }
        summary.spawned_read = spawned_reads.len();
        if summary.compressed_unreadable + summary.compressed_too_large > 0 {
            // Counts only: never a path or a session.
            tracing::warn!(
                unreadable = summary.compressed_unreadable,
                too_large = summary.compressed_too_large,
                "insights counter pass could not read compressed sessions; counted as unknown"
            );
        }
        // A copy of a managed-profile session goes once the profile's own
        // row exists, so the session is never missing in between.
        for (copy, keeper) in replaced {
            let keeper_kept = (current.rows.contains_key(&keeper) && !removals.contains(&keeper))
                || reads.iter().any(|(session, _)| *session == keeper);
            if keeper_kept && current.rows.contains_key(&copy) {
                removals.insert(copy);
            }
        }

        // A v1 store is written back as v2 even when nothing else changed.
        let upgrade = current.schema != COUNTER_STORE_SCHEMA;
        let purge = never_sessions.touches(&current.routing);
        let unchanged = reads.is_empty()
            && removals.is_empty()
            && spawned_reads.is_empty()
            && spawned_removals.is_empty()
            && !restart
            && !upgrade
            && !purge;
        // Asked with no lock of this pass held: it reads the settings.
        if should_stop() {
            return Ok(None);
        }
        let mut cached = self.cached.lock().map_err(|_| STORE_WRITE_FAILED)?;
        // Under the lock `clear` holds across removal, so a clear is either
        // seen here or happens after this write and removes it.
        if self.cleared.load(Ordering::SeqCst) != cleared {
            return Ok(None);
        }
        // Under the cache lock, so a fold that writes after this sees it.
        *self
            .never_sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = never_sessions.clone();
        if unchanged {
            self.mark_pass(now);
            return Ok(Some(summary));
        }
        // The store as it stands now, not as it was when the pass began.
        let mut store = match self.load_into(&mut cached) {
            Ok(Some(store)) if store.key_fingerprint == fingerprint => (*store).clone(),
            Ok(_) | Err(_) => CounterStore::empty(fingerprint),
        };
        store.schema = COUNTER_STORE_SCHEMA.to_string();
        // A session dropped for a Never folder takes its tally with it; one
        // that only aged out keeps it until its last call does too.
        let mut gone: BTreeSet<KeyedDigest> = BTreeSet::new();
        for session in &removals {
            if let Some(row) = store.rows.remove(session) {
                summary.dropped += 1;
                if never.contains(&row.project) {
                    gone.extend(row.harness_session);
                }
            }
        }
        for (session, mut row) in reads {
            row.seq = store.next_seq;
            store.next_seq += 1;
            store.evicted.remove(&session);
            store.rows.insert(session, row);
        }
        store
            .evicted
            .retain(|_, evicted| evicted.modified_at >= cutoff);
        enforce_caps(
            &mut store,
            COUNTER_STORE_MAX_SESSIONS,
            COUNTER_STORE_MAX_TURNS,
            COUNTER_STORE_MAX_TOOL_CALLS,
            COUNTER_STORE_MAX_AUTOMATED_SESSIONS,
            !classifying,
        );
        trim_codex_series(
            &mut store,
            COUNTER_STORE_MAX_CODEX_SERIES_TURNS,
            COUNTER_STORE_MAX_CODEX_SERIES_TOOL_CALLS,
        );
        for session in &spawned_removals {
            store.spawned.remove(session);
        }
        for (session, mut spawned) in spawned_reads {
            spawned.row.seq = store.spawned_next_seq;
            store.spawned_next_seq += 1;
            store.spawned.insert(session, spawned);
        }
        let (max_spawned, max_spawned_turns, max_spawned_tools) = self.spawned_caps();
        enforce_spawned_caps(
            &mut store,
            max_spawned,
            max_spawned_turns,
            max_spawned_tools,
        );
        store.spawned_floor = store.spawned_floor.filter(|floor| *floor >= cutoff);
        never_sessions.purge(&mut store.routing);
        prune_routing(&mut store, &gone, cutoff);
        self.stage_into(&mut cached, store);
        self.mark_pass(now);
        let staged = cached.as_ref().map(Arc::clone);
        drop(cached);
        self.flush()?;
        if let Some(store) = staged {
            self.keep_labels(&store, &never, &writing, summary.deferred == 0, cleared);
        }
        Ok(Some(summary))
    }

    /// The labels file's path, beside the store.
    fn labels_path(&self) -> PathBuf {
        self.rows_path.with_file_name(SESSION_LABELS_FILE)
    }

    fn labels_slot(&self) -> std::sync::MutexGuard<'_, Option<LabelsRead>> {
        self.labels
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The labels file, read the first time it is needed, for a caller
    /// holding its lock. A failed read (a wrong owner, a transient I/O
    /// error) answers as unreadable without being kept, so the next call
    /// reads again; only a file not in the labels shape stays unreadable.
    fn read_labels<'s>(&self, slot: &'s mut Option<LabelsRead>) -> &'s LabelsRead {
        static READ_FAILED: LabelsRead = LabelsRead::Unreadable;
        if slot.is_none() {
            *slot = match std::fs::read(self.labels_path()) {
                Ok(bytes) => Some(
                    serde_json::from_slice(&bytes)
                        .map(LabelsRead::Read)
                        .unwrap_or(LabelsRead::Unreadable),
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    Some(LabelsRead::Absent)
                }
                Err(_) => return &READ_FAILED,
            };
        }
        slot.as_ref().unwrap_or(&READ_FAILED)
    }

    /// The labels this build can read, by session, made under the key
    /// `fingerprint` names; none when the file is absent, unreadable or
    /// under another key.
    fn readable_labels(&self, fingerprint: &KeyedDigest) -> BTreeMap<KeyedDigest, SessionLabel> {
        let mut slot = self.labels_slot();
        match self.read_labels(&mut slot) {
            LabelsRead::Read(file) if file.key_fingerprint == *fingerprint => file.readable(),
            _ => BTreeMap::new(),
        }
    }

    /// Write `file` as the labels file, for a caller holding its lock. The
    /// cached file changes only once the write succeeds.
    fn write_labels(&self, slot: &mut Option<LabelsRead>, file: LabelFile) -> Result<(), ()> {
        let body = serde_json::to_vec(&file).map_err(|_| ())?;
        let path = self.labels_path();
        let dir = path.parent().ok_or(())?;
        crate::config::write_atomic_0600(dir, &path, &body).map_err(|_| ())?;
        *slot = Some(LabelsRead::Read(file));
        Ok(())
    }

    /// After a pass has written `store`: keep a label only while its row is
    /// stored outside a Never folder, or its session is still being written
    /// (`writing`). So a row that aged out, was evicted or was dropped for
    /// a Never folder takes its label with it.
    ///
    /// Except while the store is catching up after being begun again: its
    /// rows come back a few at a time, within the per-poll budget, so until
    /// a pass leaves nothing for later (`caught_up`) a label waits for its
    /// row rather than going for want of one. A labels file under another
    /// key names no session any more and is removed. A failure is left for
    /// the next pass.
    fn keep_labels(
        &self,
        store: &CounterStore,
        never: &NeverDigests,
        writing: &BTreeSet<KeyedDigest>,
        caught_up: bool,
        cleared: u64,
    ) {
        let mut slot = self.labels_slot();
        // A clear since the pass began removed the file: write nothing.
        if self.cleared.load(Ordering::SeqCst) != cleared {
            return;
        }
        let LabelsRead::Read(file) = self.read_labels(&mut slot) else {
            return;
        };
        if file.key_fingerprint != store.key_fingerprint {
            if std::fs::remove_file(self.labels_path()).is_ok() {
                *slot = Some(LabelsRead::Absent);
            }
            return;
        }
        if file.generation != store.generation && !caught_up {
            return;
        }
        let mut kept = file.clone();
        kept.generation = store.generation;
        kept.labels.retain(|session, _| {
            writing.contains(session)
                || store
                    .rows
                    .get(session)
                    .is_some_and(|row| is_listed(row, never))
        });
        if kept != *file && self.write_labels(&mut slot, kept).is_err() {
            // A fixed label only.
            tracing::warn!("insights session labels could not be written");
        }
    }

    /// A pass finished at `now`, for the answer's `updated_at`. Called under
    /// the cache lock, the order `clear` takes the two in.
    fn mark_pass(&self, now: DateTime<Utc>) {
        if let Ok(mut at) = self.last_pass_at.lock() {
            *at = Some(now);
        }
    }

    #[cfg(test)]
    pub(crate) fn week_value(
        &self,
        enabled: bool,
        week: Option<NaiveDate>,
        tz: FixedOffset,
        now: DateTime<Utc>,
        never_project_keys: &NeverFolders,
        options: WeekOptions,
    ) -> serde_json::Value {
        self.week_value_with_folders(enabled, week, tz, now, never_project_keys, (options, &[]))
    }

    /// The `insights_week` answer, with the Sessions list's resolved labels.
    pub(crate) fn week_value_with_folders(
        &self,
        enabled: bool,
        week: Option<NaiveDate>,
        tz: FixedOffset,
        now: DateTime<Utc>,
        never_project_keys: &NeverFolders,
        context: (WeekOptions, &[(String, String)]),
    ) -> serde_json::Value {
        let (options, folders) = context;
        if !enabled {
            // Owner decision D4, open: nothing is read for Insights.
            return serde_json::json!({"enabled": false, "feed": FEED_COUNTER_PASS});
        }
        let unreadable = |reason: &str| {
            serde_json::json!({
                "enabled": true,
                "feed": FEED_COUNTER_PASS,
                "readable": false,
                "reason": reason,
            })
        };
        // Never creates a key: no key means no pass has stored a row yet.
        let key = match self.keys.load() {
            Ok(key) => key,
            Err(_) => return unreadable(UNREADABLE_KEY),
        };
        let store = match self.load_cached() {
            Ok(store) => store,
            Err(label) => return unreadable(label),
        };
        // Owner decision D3: with the ledger feed turned off no route is
        // shown, not even a tally folded while it was on. With no proxy
        // ledger a stored tally may lack every call since, so none is shown
        // either.
        let routing_unavailable = if !options.ledger_feed {
            Some(ROUTING_LEDGER_FEED_OFF)
        } else if !options.ledger_present {
            Some(ROUTING_NO_LEDGER)
        } else {
            None
        };
        // The answer-time join (design part B; the engine never sees it):
        // each row's engine reference to its first-event time and its
        // session's route tally, by the keyed harness-session digest under
        // the counter key (owner decision D15, extended to feed T).
        let mut joined: BTreeMap<String, RowJoin<'_>> = BTreeMap::new();
        let inputs: Vec<SessionInput> = match (&key, &store) {
            (Some(key), Some(store)) if store.key_fingerprint == key_fingerprint(key) => {
                let never = never_digests(key, never_project_keys);
                let labels = folder_labels_by_digest(key, folders);
                // In the order the pass made the rows. The store is keyed by
                // a digest of each file's path, so its own order is
                // arbitrary, and the engine emits `sessions[]` in input
                // order: a shell drawing the rows would show them shuffled.
                let mut rows: Vec<&CounterRow> = store
                    .rows
                    .values()
                    .filter(|row| !never.contains(&row.project))
                    .collect();
                rows.sort_by_key(|row| row.seq);
                rows.into_iter()
                    .map(|row| {
                        let input = row.session_input();
                        let tally = row
                            .harness_session
                            .as_ref()
                            .and_then(|digest| store.routing.get(digest));
                        joined.insert(
                            input.session_ref.clone(),
                            RowJoin {
                                session_ref: row.reference(),
                                folder: labels.get(&row.project).copied(),
                                started_at: row.first_event_at,
                                last_event_at: row.latest_event_at,
                                tally,
                            },
                        );
                        input
                    })
                    .collect()
            }
            // Rows made under another key are not this key's; the next
            // pass starts the store again.
            _ => Vec::new(),
        };
        let week_start = week.unwrap_or_else(|| local_week_start(&now, &tz));
        let goals = match (&key, &store) {
            (Some(key), Some(store)) if store.key_fingerprint == key_fingerprint(key) => {
                week_goals(
                    store,
                    key,
                    &never_digests(key, never_project_keys),
                    &folder_labels_by_digest(key, folders),
                    self.codex_goals_db.as_deref(),
                    (week_start, tz),
                )
            }
            _ => WeekGoals::default(),
        };
        let credit = match (&key, &store) {
            (Some(key), Some(store))
                if routing_unavailable.is_none()
                    && LEDGER_MONEY_IN_INSIGHTS
                    && store.key_fingerprint == key_fingerprint(key) =>
            {
                near_ai_credit_value(store, key, never_project_keys, folders, week_start, tz, now)
            }
            _ => serde_json::Value::Null,
        };
        let this = week_rollup(Feed::CounterPass, &inputs, week_start, &tz);
        let last = week_rollup(
            Feed::CounterPass,
            &inputs,
            this.week_start - Duration::days(7),
            &tz,
        );
        let changes: Vec<serde_json::Value> = this
            .sources
            .iter()
            .map(
                |line| match change_vs_last_week(&this, &last, line.source) {
                    Ok(permille) => serde_json::json!({
                        "source": line.source,
                        "permille": permille,
                        "unavailable": null,
                    }),
                    Err(reason) => serde_json::json!({
                        "source": line.source,
                        "permille": null,
                        "unavailable": reason,
                    }),
                },
            )
            .collect();
        // Every kept week up to the current one, oldest first, whatever week
        // is on screen: goals, the lever and the weekly summary card read
        // them in process (owner decision D4, open).
        let current = local_week_start(&now, &tz);
        let threshold = options.context_threshold.map(u64::from);
        let history: Vec<WeekFigures> = (0..COUNTER_STORE_KEEP_WEEKS)
            .rev()
            .map(|back| {
                week_figures(
                    Feed::CounterPass,
                    &inputs,
                    current - Duration::weeks(back),
                    tz,
                    threshold,
                )
            })
            .collect();
        let earlier: Vec<WeekRollup> = (1..=COUNTER_STORE_KEEP_WEEKS)
            .map(|back| {
                week_rollup(
                    Feed::CounterPass,
                    &inputs,
                    this.week_start - Duration::weeks(back),
                    &tz,
                )
            })
            .filter(|week| week.coverage.sessions() > 0)
            .collect();
        // Claude and Codex spend at API list prices, lines and edits
        // (product owner, 2026-10-29); `null` with no row, as
        // `near_ai_credit` is with no store.
        let work = if inputs.is_empty() {
            serde_json::Value::Null
        } else {
            let folders: BTreeMap<String, Option<String>> = joined
                .iter()
                .map(|(reference, join)| (reference.clone(), join.folder.map(str::to_owned)))
                .collect();
            super::insights_week_work::work_value(
                &inputs,
                &folders,
                crate::insights::pricing_catalog::shared_pricing_catalog(),
                this.week_start,
                tz,
                current - Duration::weeks(COUNTER_STORE_KEEP_WEEKS - 1),
                &near_ai_routes(&joined, routing_unavailable.is_none()),
            )
        };
        let picker = dated_weeks(&inputs, &tz);
        let overview = counter_overview(&this, &earlier, tz.local_minus_utc(), picker.clone());
        // The Patterns cards describe how the person works: automated
        // sessions are left out of them, as from the Models answers and the
        // Sessions feed. The token figures above count them.
        let persons: Vec<SessionInput> = inputs
            .iter()
            .filter(|input| !input.automated)
            .cloned()
            .collect();
        let mut patterns = week_patterns(
            Feed::CounterPass,
            &persons,
            this.week_start,
            tz,
            PATTERN_BAR_WEEKS,
        );
        patterns.weeks = picker;
        let comparable = comparable(&this);
        let updated_at = self
            .last_pass_at
            .lock()
            .ok()
            .and_then(|at| *at)
            .map(|at| at.to_rfc3339());
        let iso = this.week_start.iso_week();
        serde_json::json!({
            "enabled": true,
            "feed": FEED_COUNTER_PASS,
            "readable": true,
            "updated_at": updated_at,
            // The person's rows, as `insights_sessions` counts them.
            "sessions_stored": persons.len(),
            // Beside those, the sessions started by a tool, not the person.
            "automated_stored": inputs.len() - persons.len(),
            "iso_week": format!("{}-W{:02}", iso.year(), iso.week()),
            "week_start": this.week_start.to_string(),
            "comparable": comparable.is_ok(),
            "unavailable": comparable.err(),
            "change_vs_last_week": changes,
            "rollup": rollup_value(&this, &joined, routing_unavailable.is_none()),
            "routing_available": routing_unavailable.is_none(),
            "routing_unavailable": routing_unavailable,
            "near_ai_credit": credit,
            "work": work,
            "usage_by_day": usage_by_day_value(&inputs, now, tz),
            "overview": overview,
            "patterns": patterns,
            "history": history,
            "recap_card_enabled": options.recap_card_enabled,
            "goals": goals,
        })
    }

    /// Remove the store and forget the key. Idempotent. The key is forgotten
    /// whether or not a store was there: a pass makes the key before it knows
    /// whether it will write a row, so a missing store does not mean a
    /// missing key. Forgetting an absent key is not an error.
    pub(crate) fn clear(&self) -> Result<(), &'static str> {
        // The writer lock first, as `flush` takes it: a write in progress
        // finishes before the file is removed, and one after finds no store.
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut cached = self.cached.lock().map_err(|_| ERR_CLEAR_FAILED)?;
        match std::fs::remove_file(&self.rows_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(ERR_CLEAR_FAILED),
        }
        *cached = None;
        // The labels go with the store. Under their lock, so a label write
        // either finishes before the removal or sees the clear and writes
        // nothing.
        let mut labels = self.labels_slot();
        let labels_removed = match std::fs::remove_file(self.labels_path()) {
            Ok(()) => true,
            Err(error) => error.kind() == std::io::ErrorKind::NotFound,
        };
        *labels = None;
        self.cleared.fetch_add(1, Ordering::SeqCst);
        drop(labels);
        if !labels_removed {
            // The rows are gone; the next clear tries the labels again.
            return Err(ERR_CLEAR_FAILED);
        }
        if let Ok(mut at) = self.last_pass_at.lock() {
            *at = None;
        }
        if self.keys.clear().is_err() {
            // The rows are gone; the next clear tries the key again. A fixed
            // label only.
            tracing::warn!("insights counter key could not be forgotten");
        }
        Ok(())
    }

    /// The store a Sessions answer reads, with the key its rows were made
    /// under: `Ok(None)` for the store when no pass made a key or the store
    /// was made under another key. Rows are read in place, never copied.
    fn visible_store(
        &self,
    ) -> Result<(Option<DigestKey>, Option<Arc<CounterStore>>), &'static str> {
        // Never creates a key: no key means no pass has stored a row yet.
        let key = self.keys.load().map_err(|_| UNREADABLE_KEY)?;
        let store = self.load_cached()?;
        let store = match (&key, store) {
            (Some(key), Some(store)) if store.key_fingerprint == key_fingerprint(key) => {
                Some(store)
            }
            _ => None,
        };
        Ok((key, store))
    }

    /// The tag a `insights_sessions` cursor carries for the store now
    /// visible: `store_tag_of` it, or `0` with no key or store.
    #[cfg(test)]
    pub(crate) fn store_tag(&self) -> u64 {
        match self.visible_store() {
            Ok((Some(key), Some(store))) => store_tag_of(&key, &store),
            _ => 0,
        }
    }

    /// The `insights_sessions` answer's first page, at most `limit` rows.
    #[cfg(test)]
    pub(crate) fn sessions_value(
        &self,
        enabled: bool,
        never_project_keys: &NeverFolders,
        folders: &[(String, String)],
        limit: usize,
    ) -> serde_json::Value {
        self.sessions_page_value(
            enabled,
            never_project_keys,
            folders,
            super::insights_sessions::SessionsPage {
                before: None,
                store: 0,
                limit,
            },
        )
        .expect("a first page has no cursor to refuse")
    }

    /// One page of the `insights_sessions` answer: the stored sessions older
    /// by recency than the page's cursor, at most its limit, chosen and
    /// ordered by `insights_sessions::select_page` (the most recent by the
    /// time discovery knows, then dated first, newest first, undated after).
    /// Each row has an opaque reference, its harness, its first recorded
    /// event, its turns (Claude Code only), its tokens and sparkline, and its
    /// folder's label; `next` is the cursor for the following page, `null`
    /// at the end.
    ///
    /// `folders` are the project keys the daemon knows, each with the label
    /// it shows for that folder; a row's folder digest is matched against
    /// their digests under the counter key, here, so neither a key nor a
    /// digest leaves this function. No match crosses as `null`.
    pub(crate) fn sessions_page_value(
        &self,
        enabled: bool,
        never_project_keys: &NeverFolders,
        folders: &[(String, String)],
        page: super::insights_sessions::SessionsPage,
    ) -> Result<serde_json::Value, &'static str> {
        if !enabled {
            // Owner decision D4, open: nothing is read for Insights.
            return Ok(serde_json::json!({"enabled": false, "feed": FEED_COUNTER_PASS}));
        }
        let (key, store) = match self.visible_store() {
            Ok(found) => found,
            Err(reason) => return Ok(unreadable_value(reason)),
        };
        // A cursor from another store names other sessions: refuse it, and
        // the shell reads the first page.
        let tag = match (&key, &store) {
            (Some(key), Some(store)) => store_tag_of(key, store),
            _ => 0,
        };
        if page.before.is_some() && page.store != tag {
            return Err(super::insights_sessions::ERR_SESSIONS_CURSOR_INVALID);
        }
        let rows: Vec<(&KeyedDigest, &CounterRow)> = match (&key, &store) {
            (Some(key), Some(store)) => {
                let never = never_digests(key, never_project_keys);
                // The Sessions feed lists the person's sessions: automated
                // ones are left out, and not counted in `sessions_stored`.
                store
                    .rows
                    .iter()
                    .filter(|(_, row)| is_listed(row, &never))
                    .collect()
            }
            _ => Vec::new(),
        };
        let user_labels = store
            .as_ref()
            .map(|store| self.readable_labels(&store.key_fingerprint))
            .unwrap_or_default();
        let labels = folder_digests(key.as_ref(), folders);
        let stored = rows.len();
        // Recency is the first recorded event, else the time discovery
        // placed the row, else its file's last write; ties go to the row
        // made later. So a session placed just now is not cut by older
        // dated ones, and an undated row never leads a page: a large file
        // still being written would otherwise sit above every dated one.
        let (rows, next) = super::insights_sessions::select_page(
            rows,
            |(_, row)| super::insights_sessions::SessionsCursor {
                at: row.listed_at().or(row.placed_at).unwrap_or(row.modified_at),
                seq: row.seq,
            },
            |(_, row)| row.listed_at().is_some(),
            page,
        );
        let sessions: Vec<serde_json::Value> = rows
            .iter()
            .map(|(session, row)| session_row_value(row, tag, user_labels.get(*session), &labels))
            .collect();
        Ok(serde_json::json!({
            "enabled": true,
            "feed": FEED_COUNTER_PASS,
            "readable": true,
            "updated_at": self.updated_at(),
            "sessions_stored": stored,
            // Moves only when a row is made (the store's next reference), so
            // with `sessions_stored` it tells a client its rows changed;
            // `updated_at` moves on every pass.
            "revision": store.as_ref().map_or(0, |store| store.next_seq),
            "truncated": next.is_some(),
            "next": next.map(|cursor| cursor.encode(tag)),
            "sessions": sessions,
        }))
    }

    /// `insights_session_label`: apply `change` to the label of the row
    /// `reference` names and answer that row as `insights_sessions` lists
    /// it, under `session`. `Err` when no visible row has that reference
    /// (never stored, made again since, automated, or in a Never folder),
    /// or the labels file could not be written or read; the label is then
    /// as it was, in the file and in every answer.
    ///
    /// Never makes a key, and writes nothing while the pass is off. Writes
    /// only the small labels file ([`LabelFile`]), never the store: the row
    /// is found in the cached store, which is not copied, and the cache
    /// lock is released before the labels lock is taken.
    pub(crate) fn label_value(
        &self,
        enabled: bool,
        reference: &str,
        change: super::insights_sessions::LabelChange,
        never_project_keys: &NeverFolders,
        folders: &[(String, String)],
    ) -> Result<serde_json::Value, &'static str> {
        if !enabled {
            return Ok(serde_json::json!({"enabled": false, "feed": FEED_COUNTER_PASS}));
        }
        // Read before the store, so a clear after it is seen below.
        let cleared = self.cleared.load(Ordering::SeqCst);
        let (key, store) = match self.visible_store() {
            Ok((Some(key), Some(store))) => (key, store),
            Ok(_) => return Err(super::insights_sessions::ERR_SESSION_UNKNOWN),
            Err(reason) => return Ok(unreadable_value(reason)),
        };
        let never = never_digests(&key, never_project_keys);
        let (session, row) = find_listed(&key, &store, &never, reference)
            .ok_or(super::insights_sessions::ERR_SESSION_UNKNOWN)?;
        let label = {
            let mut slot = self.labels_slot();
            // A clear since the row was found removed the labels with the
            // store: the row names nothing now, and nothing is written.
            if self.cleared.load(Ordering::SeqCst) != cleared {
                return Err(super::insights_sessions::ERR_SESSION_UNKNOWN);
            }
            let current = match self.read_labels(&mut slot) {
                LabelsRead::Read(file) if file.key_fingerprint == store.key_fingerprint => {
                    file.clone()
                }
                // A file not in the labels shape is never written over.
                LabelsRead::Unreadable => {
                    return Err(super::insights_sessions::ERR_SESSION_LABEL_WRITE_FAILED);
                }
                // None yet, or one under a key that is gone, whose digests
                // name no session now.
                LabelsRead::Absent | LabelsRead::Read(_) => {
                    LabelFile::empty(store.key_fingerprint, store.generation)
                }
            };
            let mut file = current.clone();
            let mut stored = match file.labels.remove(session) {
                Some(serde_json::Value::Object(stored)) => stored,
                _ => serde_json::Map::new(),
            };
            change.apply_stored(&mut stored);
            let stored = serde_json::Value::Object(stored);
            let label = read_label(&stored);
            if stored.as_object().is_some_and(|stored| !stored.is_empty()) {
                file.labels.insert(*session, stored);
            }
            if file != current {
                self.write_labels(&mut slot, file)
                    .map_err(|_| super::insights_sessions::ERR_SESSION_LABEL_WRITE_FAILED)?;
            }
            label
        };
        let folders = folder_digests(Some(&key), folders);
        let label = (!label.is_empty()).then_some(label);
        Ok(serde_json::json!({
            "enabled": true,
            "feed": FEED_COUNTER_PASS,
            "readable": true,
            "session": session_row_value(
                row,
                store_tag_of(&key, &store),
                label.as_ref(),
                &folders,
            ),
        }))
    }

    /// The `insights_session_drill` answer for the row `reference` names, in
    /// the shape a saved snapshot's `session_drill` has, from the same
    /// engine. `Err` when no visible row has that reference.
    pub(crate) fn drill_value(
        &self,
        enabled: bool,
        reference: &str,
        never_project_keys: &NeverFolders,
        tz: FixedOffset,
    ) -> Result<serde_json::Value, &'static str> {
        if !enabled {
            return Ok(serde_json::json!({"enabled": false, "feed": FEED_COUNTER_PASS}));
        }
        let (key, store) = match self.visible_store() {
            Ok((Some(key), Some(store))) => (key, store),
            Ok(_) => return Err(super::insights_sessions::ERR_SESSION_UNKNOWN),
            Err(reason) => return Ok(unreadable_value(reason)),
        };
        let never = never_digests(&key, never_project_keys);
        let (_, row) = find_listed(&key, &store, &never, reference)
            .ok_or(super::insights_sessions::ERR_SESSION_UNKNOWN)?;
        let input = row.session_input();
        let mut drill = crate::insights::session_drill::session_drill(
            reference,
            &input.body,
            row.earliest_event_at,
            row.latest_event_at,
            tz,
        );
        drill.feed = Feed::CounterPass;
        Ok(serde_json::json!({
            "enabled": true,
            "feed": FEED_COUNTER_PASS,
            "readable": true,
            "session": drill,
        }))
    }

    /// Which of the sessions the watcher finds now (`found`: each one's
    /// adapter and path, as discovery lists it) the row `reference` names,
    /// for `insights_analyze_session`. The row is looked up as the drill
    /// looks it up (never in a Never folder, never automated), and matched
    /// to a file by its keyed session digest, the key the pass stored it
    /// under: nothing is read but the store, and no path is stored or kept.
    /// `Ok(None)` when the row is stored but no file found matches it any
    /// more (moved or deleted since it was read).
    pub(crate) fn locate(
        &self,
        reference: &str,
        never_project_keys: &NeverFolders,
        found: &[(&str, &std::path::Path)],
    ) -> Result<Option<Located>, LocateError> {
        let (key, store) = self
            .visible_store()
            .map_err(|reason| LocateError::Unreadable(unreadable_value(reason)))?;
        let (Some(key), Some(store)) = (key, store) else {
            return Err(LocateError::Unknown);
        };
        let never = never_digests(&key, never_project_keys);
        let (session, row) =
            find_listed(&key, &store, &never, reference).ok_or(LocateError::Unknown)?;
        if let Some(reason) = row.not_analyzable {
            return Err(LocateError::NotAnalyzable(reason));
        }
        let session = *session;
        Ok(found
            .iter()
            .enumerate()
            .find_map(|(index, (adapter, path))| {
                let source = analytics_source(adapter)?;
                (row_key(&key, adapter, path) == session).then_some(Located {
                    index,
                    format: source_format(source),
                })
            }))
    }

    /// The `insights_model_answers` answer: the six Models answers over the
    /// stored sessions inside the 13 weeks ending `now`, priced at list
    /// prices when the answer is built (the store holds tokens, never a list
    /// price). Each row is joined to its session's route tally by the keyed
    /// harness-session digest, as `week_value` joins it, and so to the NEAR
    /// AI charge the tally kept, which is answered as its own figure and
    /// never added to a list-priced one. With the ledger feed off or no proxy
    /// ledger declared every session is `Unobserved` with no charge, so no
    /// stored tally is read.
    pub(crate) fn model_answers_value(
        &self,
        enabled: bool,
        never_project_keys: &NeverFolders,
        ledger_feed: bool,
        ledger_present: bool,
        now: DateTime<Utc>,
    ) -> serde_json::Value {
        if !enabled {
            // Owner decision D4, open: nothing is read for Insights.
            return serde_json::json!({"enabled": false, "feed": FEED_COUNTER_PASS});
        }
        let (key, store) = match self.visible_store() {
            Ok(found) => found,
            Err(reason) => return unreadable_value(reason),
        };
        let (inputs, routes, charges, labels) = match (&key, &store) {
            (Some(key), Some(store)) => {
                let never = never_digests(key, never_project_keys);
                let mut keyed: Vec<(&KeyedDigest, &CounterRow)> = store
                    .rows
                    .iter()
                    .filter(|(_, row)| !never.contains(&row.project))
                    .collect();
                keyed.sort_by_key(|(_, row)| row.seq);
                // The person's own labels, by the engine's reference.
                let user_labels = self.readable_labels(&store.key_fingerprint);
                let labels: BTreeMap<String, SessionLabel> = keyed
                    .iter()
                    .filter_map(|(session, row)| {
                        user_labels
                            .get(*session)
                            .map(|label| (row.reference(), *label))
                    })
                    .collect();
                let rows: Vec<&CounterRow> = keyed.into_iter().map(|(_, row)| row).collect();
                let available = ledger_feed && ledger_present;
                let routes = route_map(&rows, &store.routing, available);
                let charges =
                    charge_map(&rows, &store.routing, available, LEDGER_MONEY_IN_INSIGHTS);
                let inputs: Vec<SessionInput> =
                    rows.iter().map(|row| row.session_input()).collect();
                (inputs, routes, charges, labels)
            }
            _ => (
                Vec::new(),
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
        };
        let answers = crate::insights::model_answers::model_answers_labelled(
            &inputs,
            &routes,
            &charges,
            &labels,
            crate::insights::pricing_catalog::shared_pricing_catalog(),
            now,
        );
        let tables: Vec<serde_json::Value> = answers
            .pricing_tables
            .iter()
            .map(|table| serde_json::json!({"table_id": table.table_id, "version": table.version}))
            .collect();
        serde_json::json!({
            "enabled": true,
            "feed": FEED_COUNTER_PASS,
            "readable": true,
            "updated_at": self.updated_at(),
            "window_start": answers.window_start.to_rfc3339(),
            "window_end": answers.window_end.to_rfc3339(),
            "pricing": {
                "basis": crate::insights::session_pricing::PRICING_BASIS_API_LIST,
                "tables": tables,
            },
            "rows": answers.rows,
        })
    }

    /// When a pass last finished, RFC 3339.
    fn updated_at(&self) -> Option<String> {
        self.last_pass_at
            .lock()
            .ok()
            .and_then(|at| *at)
            .map(|at| at.to_rfc3339())
    }

    /// Make the key, as a pass would, for tests outside this module.
    #[cfg(test)]
    pub(crate) fn keys_load_or_create_for_test(&self) {
        self.keys.load_or_create().unwrap();
    }

    /// Whether no key is held, for tests that check a clear forgot it.
    #[cfg(test)]
    pub(crate) fn key_is_absent_for_test(&self) -> bool {
        matches!(self.keys.load(), Ok(None))
    }

    /// The store as cached, read from disk the first time. `Ok(None)` when
    /// there is no store yet; an unreadable store is an error, never empty.
    fn load_cached(&self) -> Result<Option<Arc<CounterStore>>, &'static str> {
        let mut cached = self.cached.lock().map_err(|_| UNREADABLE_STORE)?;
        self.load_into(&mut cached)
    }

    /// `load_cached` for a caller already holding the cache lock.
    fn load_into(
        &self,
        cached: &mut Option<Arc<CounterStore>>,
    ) -> Result<Option<Arc<CounterStore>>, &'static str> {
        if let Some(store) = cached.as_ref() {
            return Ok(Some(Arc::clone(store)));
        }
        let bytes = match std::fs::read(&self.rows_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(UNREADABLE_STORE),
        };
        let store: CounterStore = serde_json::from_slice(&bytes).map_err(|_| UNREADABLE_STORE)?;
        if store.schema != COUNTER_STORE_SCHEMA && store.schema != COUNTER_STORE_SCHEMA_V1 {
            return Err(UNREADABLE_STORE);
        }
        let store = Arc::new(store);
        *cached = Some(Arc::clone(&store));
        Ok(Some(store))
    }

    #[cfg(test)]
    fn save(&self, store: CounterStore) -> Result<(), &'static str> {
        {
            let mut cached = self.cached.lock().map_err(|_| STORE_WRITE_FAILED)?;
            self.stage_into(&mut cached, store);
        }
        self.flush()
    }

    /// Make `store` the cached store, for a caller holding the cache lock;
    /// the file is written by [`Self::flush`], after that lock is released.
    /// A store equal to the cached one changes nothing and is not written.
    fn stage_into(&self, cached: &mut Option<Arc<CounterStore>>, store: CounterStore) {
        if cached.as_deref() == Some(&store) {
            return;
        }
        *cached = Some(Arc::new(store));
        self.staged.fetch_add(1, Ordering::SeqCst);
    }

    /// Write the cached store to the file, if it changed since the last
    /// write, with the cache lock released, so an answer never waits on
    /// the write.
    ///
    /// Writes are serialized by the writer lock, and each writes the store
    /// as cached when it begins, never one a caller held: the file always
    /// ends as the newest store. `clear` takes the writer lock before it
    /// removes the file and empties the cache, so a write either finishes
    /// before the removal or finds the cache empty and writes nothing. Lock
    /// order: writer, then cache; nothing holds the cache lock while it
    /// takes the writer lock.
    ///
    /// A write that fails leaves the cached store unwritten; the next flush
    /// writes it.
    fn flush(&self) -> Result<(), &'static str> {
        let mut written = self
            .writer
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (store, staged) = {
            let cached = self.cached.lock().map_err(|_| STORE_WRITE_FAILED)?;
            (
                cached.as_ref().map(Arc::clone),
                self.staged.load(Ordering::SeqCst),
            )
        };
        let Some(store) = store else {
            return Ok(());
        };
        if staged <= *written {
            return Ok(());
        }
        #[cfg(test)]
        self.count_write();
        let body = serde_json::to_vec(&*store).map_err(|_| STORE_WRITE_FAILED)?;
        let dir = self.rows_path.parent().ok_or(STORE_WRITE_FAILED)?;
        crate::config::write_atomic_0600(dir, &self.rows_path, &body)
            .map_err(|_| STORE_WRITE_FAILED)?;
        *written = staged;
        Ok(())
    }

    /// Fold the ledger's new calls into the route tallies.
    ///
    /// `enabled` is asked first: off, nothing is read, not the ledger, the
    /// store, the key or the tools (owner decision D3, on by default, and D4). The
    /// window is the ledger's own 24 hours, and the store's cursor (the
    /// highest id passed and the latest start taken) says which rows were
    /// folded already, so a call counts once across ticks, reloads, restarts
    /// of this process and restarts of the proxy's ledger (see
    /// `insights_route_tally::fold_plan`).
    /// `speakers` is asked only when a new row carries a session id.
    ///
    /// Never makes a key: before the counter pass has made one there is no
    /// store to fold into, and the next fold after the pass takes the same
    /// window. A store under another key is left for the pass to start
    /// again.
    ///
    /// Lock order: `enabled` (the settings lock) and `speakers` (the tools'
    /// config files, the private-inference state and settings locks) are
    /// asked with no lock of this pass held. The cache lock is then held
    /// across load, fold and staging the result, as `clear` holds it across
    /// removal, so the two cannot interleave; the file is written after it
    /// is released (`flush`), so an answer never waits on the write. The
    /// key is read before that lock, so a keychain's latency never holds an
    /// `insights_week` answer.
    ///
    /// The daemon runs folds on the counter pass's worker
    /// (`fold_ledger_after_refresh`), one job at a time with the passes,
    /// off the async tick that refreshed the ledger. A pass plans and reads
    /// from the store it loaded when it began, but stages under this same
    /// cache lock onto the store as it stands then, so a fold made
    /// meanwhile keeps its tallies and cursor; were the pass to write its
    /// starting store back, the next fold would take the rows still in the
    /// window again.
    #[cfg(test)]
    pub(crate) fn fold_ledger(
        &self,
        enabled: &dyn Fn() -> bool,
        ledger: &dyn crate::routing::RoutingLedger,
        speakers: &mut dyn FnMut() -> Speakers,
        now: DateTime<Utc>,
    ) -> Result<FoldSummary, &'static str> {
        self.fold_ledgers(enabled, ledger, None, speakers, now)
    }

    /// [`Self::fold_ledger`], and each managed route's ledger in `managed`
    /// by its own cursor. `managed` is `None` when the managed routes could
    /// not be read: their cursors are then kept as they are. A route no
    /// longer in a read `managed` loses its cursor.
    pub(crate) fn fold_ledgers(
        &self,
        enabled: &dyn Fn() -> bool,
        ledger: &dyn crate::routing::RoutingLedger,
        managed: Option<&ManagedRows<'_>>,
        speakers: &mut dyn FnMut() -> Speakers,
        now: DateTime<Utc>,
    ) -> Result<FoldSummary, &'static str> {
        let cleared = self.cleared.load(Ordering::SeqCst);
        if !enabled() {
            return Ok(FoldSummary::default());
        }
        let result = self.fold_inner(cleared, ledger, managed, speakers, now);
        let failure = result.as_ref().err().copied();
        if let Ok(mut last) = self.last_fold_failure.lock() {
            if *last != failure {
                if let Some(label) = failure {
                    // A fixed label only.
                    tracing::warn!(label, "insights ledger fold skipped");
                }
                *last = failure;
            }
        }
        result
    }

    fn fold_inner(
        &self,
        cleared: u64,
        ledger: &dyn crate::routing::RoutingLedger,
        managed: Option<&ManagedRows<'_>>,
        speakers: &mut dyn FnMut() -> Speakers,
        now: DateTime<Utc>,
    ) -> Result<FoldSummary, &'static str> {
        #[cfg(test)]
        {
            *self.test_seams.fold_thread.lock().unwrap() = Some(std::thread::current().id());
        }
        let window = ledger
            .exchanges_since(now - Duration::hours(crate::routing::ironwire::REFRESH_WINDOW_HOURS));
        // A first look, without the lock, to learn whether anything is new
        // and the tools need reading.
        let peek = self.load_cached()?;
        let empty = CounterStore::empty(KeyedDigest([0; 32]));
        let peeked = peek.as_deref().unwrap_or(&empty);
        let plans = FoldPlans::new(&window, managed, peeked);
        if plans.is_idle() && !loose_upgrade_due(peeked, managed) {
            return Ok(FoldSummary::default());
        }
        let speakers = if plans.needs_speakers() || loose_upgrade_due(peeked, managed) {
            speakers()
        } else {
            Vec::new()
        };

        let Some(key) = self.keys.load().map_err(|_| UNREADABLE_KEY)? else {
            return Ok(FoldSummary::default());
        };
        let mut cached = self.cached.lock().map_err(|_| UNREADABLE_STORE)?;
        // A clear since the settings were read: the key just read may be
        // the one it forgot, and the store is gone. Nothing is written.
        if self.cleared.load(Ordering::SeqCst) != cleared {
            return Ok(FoldSummary::default());
        }
        let fingerprint = key_fingerprint(&key);
        let mut store = match self.load_into(&mut cached)? {
            Some(store) if store.key_fingerprint == fingerprint => (*store).clone(),
            Some(_) => return Ok(FoldSummary::default()),
            None => CounterStore::empty(fingerprint),
        };
        // Planned again under the lock, from the cursors as they now stand.
        let plans = FoldPlans::new(&window, managed, &store);
        let upgrade = loose_upgrade_due(&store, managed);
        if plans.is_idle() && !upgrade {
            return Ok(FoldSummary::default());
        }
        if upgrade {
            // Calls folded before skipped calls' charges were kept: those
            // a fold skipped then add their charge now, once. The calls it
            // folded then add nothing again.
            let taken = |plan: &super::insights_route_tally::FoldPlan<'_>| -> BTreeSet<i64> {
                plan.rows.iter().filter_map(|row| row.id).collect()
            };
            let shared_taken = taken(&plans.shared);
            let before: Vec<&crate::routing::RoutedExchange> = window
                .iter()
                .filter(|row| row.id.is_some_and(|id| !shared_taken.contains(&id)))
                .collect();
            super::insights_route_tally::fold_loose_only(
                &mut store.loose_charge,
                &before,
                &speakers,
                LEDGER_MONEY_IN_INSIGHTS,
            );
            for (home, rows) in managed.into_iter().flatten() {
                let done = plans.managed.get(home).map(taken).unwrap_or_default();
                let before: Vec<&crate::routing::RoutedExchange> = rows
                    .iter()
                    .copied()
                    .filter(|row| row.id.is_some_and(|id| !done.contains(&id)))
                    .collect();
                super::insights_route_tally::fold_loose_only(
                    &mut store.loose_charge,
                    &before,
                    &speakers,
                    LEDGER_MONEY_IN_INSIGHTS,
                );
            }
            store.loose_epoch = LOOSE_EPOCH;
        }
        let mut summary = FoldSummary::default();
        for plan in std::iter::once(&plans.shared).chain(plans.managed.values()) {
            let folded = fold_into(
                &mut store.routing,
                &mut store.loose_charge,
                &key,
                plan,
                &speakers,
                LEDGER_MONEY_IN_INSIGHTS,
            );
            summary.folded += folded.folded;
            summary.skipped += folded.skipped;
        }
        // A Never folder leaves nothing: a session the last pass found in
        // one keeps no tally, charge or model label.
        self.never_sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .purge(&mut store.routing);
        store.ledger_folded_through = plans.shared.next.through;
        store.ledger_folded_started = plans.shared.next.started;
        if managed.is_some() {
            store.managed_folded = plans
                .managed
                .iter()
                .map(|(home, plan)| (home.clone(), StoredCursor::from(plan.next)))
                .collect();
        }
        prune_routing(
            &mut store,
            &BTreeSet::new(),
            now - Duration::weeks(COUNTER_STORE_KEEP_WEEKS),
        );
        self.stage_into(&mut cached, store);
        drop(cached);
        self.flush()?;
        Ok(summary)
    }

    /// How many sessions have a route tally, for tests outside this module.
    #[cfg(test)]
    pub(crate) fn tallied_sessions_for_test(&self) -> usize {
        self.load_cached()
            .ok()
            .flatten()
            .map_or(0, |store| store.routing.len())
    }
}

fn analytics_source(adapter: &str) -> Option<AnalyticsSource> {
    match adapter {
        SOURCE_CLAUDE_CODE => Some(AnalyticsSource::ClaudeCode),
        SOURCE_CODEX => Some(AnalyticsSource::Codex),
        _ => None,
    }
}

/// The transcript format a counted source is read as.
fn source_format(source: AnalyticsSource) -> SourceFormat {
    match source {
        AnalyticsSource::ClaudeCode => SourceFormat::ClaudeCode,
        AnalyticsSource::Codex => SourceFormat::Codex,
    }
}

/// Whether the calls folded before skipped calls' charges were kept are
/// still to be taken once more for their charge (`CounterStore::loose_epoch`).
/// Only once the managed routes could be read, so none is missed.
fn loose_upgrade_due(store: &CounterStore, managed: Option<&ManagedRows<'_>>) -> bool {
    managed.is_some() && store.loose_epoch < LOOSE_EPOCH
}

/// One fold's plans: the shared ledger's, and each managed route's by its
/// own cursor.
struct FoldPlans<'a> {
    shared: super::insights_route_tally::FoldPlan<'a>,
    managed: BTreeMap<String, super::insights_route_tally::FoldPlan<'a>>,
    /// Whether the shared cursor moves.
    shared_moved: bool,
    /// Whether any managed cursor moves or goes.
    managed_moved: bool,
}

impl<'a> FoldPlans<'a> {
    fn new(
        window: &'a [crate::routing::RoutedExchange],
        managed: Option<&ManagedRows<'a>>,
        store: &CounterStore,
    ) -> Self {
        let shared = fold_plan(window, store.fold_cursor());
        let mut moved = false;
        let managed: BTreeMap<String, super::insights_route_tally::FoldPlan<'a>> = managed
            .map(|homes| {
                moved |= store
                    .managed_folded
                    .keys()
                    .any(|home| !homes.contains_key(home));
                homes
                    .iter()
                    .map(|(home, rows)| {
                        let cursor: FoldCursor = store
                            .managed_folded
                            .get(home)
                            .copied()
                            .unwrap_or_default()
                            .into();
                        let plan = super::insights_route_tally::fold_plan_refs(rows, cursor);
                        moved |= !plan.rows.is_empty() || plan.next != cursor;
                        (home.clone(), plan)
                    })
                    .collect()
            })
            .unwrap_or_default();
        FoldPlans {
            shared_moved: shared.next != store.fold_cursor(),
            shared,
            managed,
            managed_moved: moved,
        }
    }

    /// Nothing to fold and no cursor to move.
    fn is_idle(&self) -> bool {
        self.shared.rows.is_empty() && !self.shared_moved && !self.managed_moved
    }

    fn needs_speakers(&self) -> bool {
        self.shared.needs_speakers() || self.managed.values().any(|plan| plan.needs_speakers())
    }
}

/// The watched session a stored row was made from ([`CounterPass::locate`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Located {
    /// Its place in the `found` list it was matched in.
    pub index: usize,
    /// The transcript format its harness is read as.
    pub format: SourceFormat,
}

/// Why [`CounterPass::locate`] names no row.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LocateError {
    /// The store or key could not be read: the answer to send instead.
    Unreadable(serde_json::Value),
    /// No visible row has that reference.
    Unknown,
    /// The row's file is one the analysis refuses, as the pass found it.
    NotAnalyzable(NotAnalyzable),
}

/// The folders whose rule is Never, the contribution override included
/// (owner decision D4): sessions in them are not read, and their rows are
/// dropped and never shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NeverFolders {
    /// A global Never override is in force: every folder, including ones
    /// with no rule of their own, whose keys no list holds.
    Every,
    /// These folders, by project key.
    These(Vec<String>),
}

impl NeverFolders {
    /// No folder is Never.
    pub(crate) const NONE: NeverFolders = NeverFolders::These(Vec::new());
}

/// [`NeverFolders`] under the counter key, as the rows store them.
enum NeverDigests {
    Every,
    These(BTreeSet<KeyedDigest>),
}

impl NeverDigests {
    fn contains(&self, project: &KeyedDigest) -> bool {
        match self {
            Self::Every => true,
            Self::These(digests) => digests.contains(project),
        }
    }
}

fn never_digests(key: &DigestKey, never: &NeverFolders) -> NeverDigests {
    if !COUNTER_PASS_EXCLUDES_NEVER_FOLDERS {
        return NeverDigests::These(BTreeSet::new());
    }
    match never {
        NeverFolders::Every => NeverDigests::Every,
        NeverFolders::These(keys) => NeverDigests::These(
            keys.iter()
                .map(|project_key| project_digest(key, project_key))
                .collect(),
        ),
    }
}

/// One session's counters, or unknown when they cannot be read. Never zero
/// in place of unknown. Beside them, the keyed digest of the one session ID
/// the transcript records, under the harness name the adapter reports (the
/// same string the proxy ledger's attribution uses); `None` when there is
/// none or more than one, as a saved snapshot's identity does. The file is
/// read line by line ([`scan_session`]), never held whole.
fn read_body(source: AnalyticsSource, candidate: &CounterCandidate, key: &DigestKey) -> ReadBody {
    let insights_only = insights_only(candidate);
    let mut compressed_failure = None;
    let (bytes, scanned) = if let Some(file) = compressed_file(candidate) {
        // Streamed through the bounded decoder, never to memory whole or a
        // temp file; the line cap below applies to decompressed lines.
        match crate::source::compressed::BoundedDecoder::open(&file) {
            Ok(mut decoder) => {
                let scanned = scan_session(
                    source,
                    std::io::BufReader::new(&mut decoder),
                    key,
                    COUNTER_PASS_MAX_LINE_BYTES,
                    CODEX_PER_TURN_SERIES,
                );
                compressed_failure = decoder.failure();
                // A stream that failed part way is unknown, never the part
                // read before it failed.
                let scanned = scanned.filter(|_| compressed_failure.is_none());
                (Some(decoder.produced()), scanned)
            }
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    compressed_failure =
                        Some(crate::source::compressed::CompressedFailure::of(&error));
                }
                (None, None)
            }
        }
    } else {
        let file = std::fs::File::open(&candidate.path).ok();
        // The one file's own size, as the analysis reads it: delegated
        // transcripts are counted in the candidate's, never read.
        let bytes = file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map(|meta| meta.len());
        let scanned = file.and_then(|file| {
            scan_session(
                source,
                std::io::BufReader::new(file),
                key,
                COUNTER_PASS_MAX_LINE_BYTES,
                CODEX_PER_TURN_SERIES,
            )
        });
        (bytes, scanned)
    };
    let Some(scanned) = scanned else {
        return ReadBody {
            body: RowBody::Unknown {
                source,
                reason: UnknownReason::NoUsageCounters,
            },
            harness_session: None,
            first_event_at: None,
            span: None,
            facts: None,
            automation: Automation::Unknown,
            not_analyzable: NotAnalyzable::of(source, insights_only, bytes, None),
            compressed_failure,
            goals: Vec::new(),
        };
    };
    let mut facts = scanned.facts;
    // A line passed over may have held counters: partial, never complete.
    let skipped = scanned.long_lines_skipped > 0;
    if skipped {
        if let Some(facts) = facts.as_mut() {
            facts.capped = true;
        }
    }
    let body = match scanned.body {
        ScannedBody::Claude(Some(evidence)) => {
            let mut series = evidence.series;
            series.truncated |= skipped;
            // A cap the series hit, told apart from the delegated mark
            // below, which sets `truncated` too.
            if let Some(facts) = facts.as_mut() {
                facts.capped |= series.truncated;
            }
            // Delegated transcripts are not read: partial, never complete.
            if candidate.group_member_count > 0 {
                series.truncated = true;
            }
            RowBody::Claude {
                model_labels: evidence.model_labels,
                series,
            }
        }
        ScannedBody::Codex(Some(evidence), mut series) => {
            if let Some(series) = series.as_mut() {
                series.series.truncated |= skipped;
            }
            if let (Some(facts), Some(series)) = (facts.as_mut(), series.as_ref()) {
                facts.capped |= series.series.truncated;
            }
            RowBody::Codex {
                observed: codex_observed(&evidence),
                series,
                series_trimmed: false,
            }
        }
        ScannedBody::Claude(None) | ScannedBody::Codex(None, _) => {
            // An unknown row keeps no facts: there are no counters for
            // them to sit beside.
            facts = None;
            RowBody::Unknown {
                source,
                reason: UnknownReason::NoUsageCounters,
            }
        }
    };
    if let Some(facts) = facts.as_mut() {
        facts.delegated_unread = candidate.group_member_count > 0;
    }
    ReadBody {
        body,
        facts,
        harness_session: scanned
            .harness_session_id
            .map(|id| harness_session_digest(key, candidate.source, &id)),
        first_event_at: scanned.first_event_at,
        span: scanned.span,
        automation: scanned.automation,
        not_analyzable: NotAnalyzable::of(
            source,
            insights_only,
            bytes,
            Some(scanned.session_metas),
        ),
        compressed_failure,
        goals: scanned.goals,
    }
}

/// What one read of a session file yields.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadBody {
    body: RowBody,
    harness_session: Option<KeyedDigest>,
    first_event_at: Option<DateTime<Utc>>,
    /// The earliest and latest recorded event.
    span: Option<(DateTime<Utc>, DateTime<Utc>)>,
    /// The session's facts; `None` when they could not be read or the body
    /// is unknown.
    facts: Option<SessionFacts>,
    automation: Automation,
    not_analyzable: Option<NotAnalyzable>,
    /// Why a compressed rollout could not be read to its end.
    compressed_failure: Option<crate::source::compressed::CompressedFailure>,
    /// Claude's `/goal` goals, status and counters only.
    goals: Vec<ClaudeGoal>,
}

/// The extractors' results for one transcript, from one pass over its lines.
struct ScannedSession {
    body: ScannedBody,
    /// The facts scan's result, beside the counters; `None` when it failed.
    facts: Option<SessionFacts>,
    /// The one session ID the transcript records, raw, for the caller to
    /// digest at once; `None` when it records none or more than one.
    harness_session_id: Option<String>,
    first_event_at: Option<DateTime<Utc>>,
    span: Option<(DateTime<Utc>, DateTime<Utc>)>,
    automation: Automation,
    /// Codex's `session_meta` records: the whole-file analysis takes a
    /// file with exactly one.
    session_metas: usize,
    /// Lines longer than the bound, passed over unread (an inlined image,
    /// say). Nonzero, the caller marks the session partial.
    long_lines_skipped: u32,
    /// Claude's `/goal` goals (`insights::slash_goals`); empty for Codex.
    goals: Vec<ClaudeGoal>,
}

/// Tells a Codex session its harness started from one the person started,
/// by the `source` its `session_meta` records name (owner decision,
/// 2026-10-20, option (b)). Codex writes `source` as an object holding a
/// `subagent` key when it spawns the thread itself (its automatic approvals
/// reviewer, or a thread an agent spawned), and as a plain label (`cli`,
/// `vscode`, `exec`, `mcp`) otherwise. Only the shape is read, never a
/// value inside it.
///
/// The first `session_meta` decides, and a later one never changes it. A
/// thread an agent spawned is written with its own `session_meta` and then
/// a copy of its parent's (`source: "vscode"`, say): that copy does not
/// make the thread the person's, nor unknown. Search of 2026-10-29 over
/// 907 local Codex transcripts: every one with more than one
/// `session_meta` opens with a spawned thread's and has the second within
/// its first three lines (121 then a person-started label, 10 then another
/// spawned thread), and none opens with a person-started one, so no
/// transcript that is the person's own is set aside by this rule. No
/// `session_meta`, no `source`, or `source: "unknown"` is unknown. `exec`
/// and `mcp` sessions read as the person's: a person can start either.
#[derive(Default)]
struct CodexAutomationScan {
    first: Option<Automation>,
}

impl CodexAutomationScan {
    fn observe(&mut self, record: &serde_json::Value) {
        if record.get("type").and_then(serde_json::Value::as_str) != Some("session_meta") {
            return;
        }
        let class = match record
            .get("payload")
            .and_then(|payload| payload.get("source"))
        {
            Some(serde_json::Value::Object(source)) if source.contains_key("subagent") => {
                Automation::Automated
            }
            None | Some(serde_json::Value::Null) => Automation::Unknown,
            Some(serde_json::Value::String(label)) if label == "unknown" => Automation::Unknown,
            Some(_) => Automation::Person,
        };
        self.first.get_or_insert(class);
    }

    fn finish(self) -> Automation {
        self.first.unwrap_or_default()
    }
}

/// The source's counter evidence; `None` when the transcript does not parse.
/// Codex's per-turn series beside it is `None` on its own when only the
/// per-turn reader failed.
enum ScannedBody {
    Claude(Option<crate::insights::turn_series::ClaudeTurnSeriesEvidence>),
    Codex(
        Option<Box<PersistedUsageEvidence>>,
        Option<CodexTurnSeriesEvidence>,
    ),
}

/// One pass over a transcript, line by line, feeding every extractor the
/// counter pass uses: the source's counters
/// (`insights::turn_series` for Claude Code, `insights::usage_evidence` for
/// Codex), the session facts beside them (`insights::session_facts` for
/// Claude Code, `insights::codex_series`, which also yields Codex's
/// per-turn series), the harness session ID (`insights::session_identity`), the first
/// event's time (the first record carrying a valid top-level `timestamp`,
/// the rule the Claude Code and Codex sources date a transcript by; lines
/// that are not JSON are passed over) and the recorded span
/// (`insights::time_evidence`). Each sees exactly what it would see given
/// the whole file, so a file under the extractors' own 16 MiB bound yields
/// the same results as before; a larger one is read the same way, in memory
/// bounded by one line and by what the extractors keep, at most the series
/// caps.
///
/// `None` when the file cannot be read: the caller stores unknown, never
/// zero. A line longer than `max_line` bytes is passed over, unparsed, and
/// counted in `long_lines_skipped`; the digest still covers it.
///
/// `codex_per_turn` is `CODEX_PER_TURN_SERIES` (owner decision D11): held,
/// no Codex per-turn series is kept, and the facts read beside it still are.
fn scan_session(
    source: AnalyticsSource,
    mut reader: impl BufRead,
    key: &DigestKey,
    max_line: u64,
    codex_per_turn: bool,
) -> Option<ScannedSession> {
    use sha2::Digest as _;
    let format = source_format(source);
    let mut hasher = sha2::Sha256::new();
    let mut identity = HarnessSessionScan::new(format);
    // A file that is not UTF-8 throughout has no session ID, counters or
    // span, as when it is read whole; `first_event_at` still reads its
    // other lines.
    let mut utf8 = true;
    let mut first_event: Option<DateTime<Utc>> = None;
    // The strict extractors: `None` once a record fails them.
    let mut claude =
        (source == AnalyticsSource::ClaudeCode).then(|| ClaudeTurnSeriesScan::new(key));
    let mut codex = (source == AnalyticsSource::Codex).then(CodexUsageScan::default);
    // The facts scans see the same records; one that fails leaves the facts
    // (and Codex's per-turn series) `None` and the counters as they are.
    let mut claude_facts =
        (source == AnalyticsSource::ClaudeCode).then(|| ClaudeFactsScan::new(key));
    let mut codex_series =
        (source == AnalyticsSource::Codex).then(|| CodexTurnSeriesScan::new(key));
    let mut time = Some(RecordedTimeScan::new(format));
    let mut automation = (source == AnalyticsSource::Codex).then(CodexAutomationScan::default);
    // Lenient: it never fails, and sees every record that parses.
    let mut goals = (source == AnalyticsSource::ClaudeCode).then(ClaudeGoalScan::new);
    let mut session_metas = 0usize;
    let mut line: Vec<u8> = Vec::new();
    // One-based physical line number, blank lines counted.
    let mut index: u64 = 0;
    let mut long_lines_skipped: u32 = 0;
    loop {
        line.clear();
        // Room for the line and a `\r\n` ending; more is refused below.
        let read = (&mut reader)
            .take(max_line.saturating_add(2))
            .read_until(b'\n', &mut line)
            .ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&line);
        let ended = line.last() == Some(&b'\n');
        if ended {
            // `str::lines`: a line ends at `\n` or `\r\n`.
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
        }
        index += 1;
        if line.len() as u64 > max_line {
            // Passed over, never parsed: the rest of the line is hashed and
            // dropped, so memory stays bounded and the digest still covers
            // every byte. No scan sees it, so it cannot end the strict ones.
            if !ended {
                skip_line(&mut reader, &mut hasher).ok()?;
            }
            long_lines_skipped = long_lines_skipped.saturating_add(1);
            continue;
        }
        let Ok(text) = std::str::from_utf8(&line) else {
            utf8 = false;
            claude = None;
            codex = None;
            claude_facts = None;
            codex_series = None;
            time = None;
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<serde_json::Value>(text) else {
            claude = None;
            codex = None;
            claude_facts = None;
            codex_series = None;
            time = None;
            continue;
        };
        identity.observe(&record);
        if let Some(scan) = automation.as_mut() {
            scan.observe(&record);
        }
        if let Some(scan) = goals.as_mut() {
            scan.observe(&record);
        }
        if source == AnalyticsSource::Codex && record["type"] == "session_meta" {
            session_metas += 1;
        }
        if first_event.is_none() {
            first_event = record_timestamp(&record);
        }
        if claude
            .as_mut()
            .is_some_and(|scan| scan.observe(&record).is_err())
        {
            claude = None;
        }
        if codex
            .as_mut()
            .is_some_and(|scan| scan.observe(index, &record).is_err())
        {
            codex = None;
        }
        if claude_facts
            .as_mut()
            .is_some_and(|scan| scan.observe(&record).is_err())
        {
            claude_facts = None;
        }
        if codex_series
            .as_mut()
            .is_some_and(|scan| scan.observe(&record).is_err())
        {
            codex_series = None;
        }
        if time
            .as_mut()
            .is_some_and(|scan| scan.observe(index, &record).is_err())
        {
            time = None;
        }
    }
    let digest = format!("{:x}", hasher.finalize());
    let (body, facts) = match source {
        AnalyticsSource::ClaudeCode => {
            let evidence = claude.and_then(|scan| scan.finish(digest.clone()).ok());
            // The facts index the series' declared labels: none without it.
            let facts = evidence
                .as_ref()
                .and_then(|evidence| claude_facts.map(|scan| scan.finish(&evidence.model_labels)));
            (ScannedBody::Claude(evidence), facts)
        }
        AnalyticsSource::Codex => {
            let (series, facts) = codex_series
                .and_then(|scan| scan.finish().ok())
                .map_or((None, None), |(series, facts)| {
                    (codex_per_turn.then_some(series), Some(facts))
                });
            (
                ScannedBody::Codex(
                    codex.and_then(|scan| scan.finish(digest.clone()).ok().map(Box::new)),
                    series,
                ),
                facts,
            )
        }
    };
    let span = time
        .and_then(|scan| scan.finish(digest).ok())
        .and_then(|evidence| Some((evidence.earliest?.recorded_at, evidence.latest?.recorded_at)));
    Some(ScannedSession {
        body,
        facts,
        harness_session_id: identity.finish().filter(|_| utf8),
        first_event_at: first_event,
        span,
        // Claude Code records no marker: its delegated transcripts are
        // never read as sessions of their own, so its rows are unknown.
        automation: automation.map_or(Automation::Unknown, CodexAutomationScan::finish),
        session_metas,
        long_lines_skipped,
        goals: goals.map(ClaudeGoalScan::finish).unwrap_or_default(),
    })
}

/// Read the rest of the current line, through its `\n` or to the end of
/// the file, into `hasher` only: nothing is kept.
fn skip_line(reader: &mut impl BufRead, hasher: &mut sha2::Sha256) -> std::io::Result<()> {
    use sha2::Digest as _;
    loop {
        let (used, done) = {
            let buf = reader.fill_buf()?;
            if buf.is_empty() {
                return Ok(());
            }
            match buf.iter().position(|byte| *byte == b'\n') {
                Some(end) => {
                    hasher.update(&buf[..=end]);
                    (end + 1, true)
                }
                None => {
                    hasher.update(buf);
                    (buf.len(), false)
                }
            }
        };
        reader.consume(used);
        if done {
            return Ok(());
        }
    }
}

/// A record's top-level `timestamp`, RFC 3339, as UTC.
fn record_timestamp(record: &serde_json::Value) -> Option<DateTime<Utc>> {
    let text = record.get("timestamp")?.as_str()?;
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

/// Codex's first-to-last observed change. Cached input is a subset of input
/// and `total` is input + output; nothing is added again.
fn codex_observed(evidence: &PersistedUsageEvidence) -> Option<CodexObserved> {
    let interval = evidence.interval.as_ref()?;
    let NativeTokenCounts::Codex {
        input,
        cached_input,
        output,
        total,
        ..
    } = interval.observed_delta
    else {
        return None;
    };
    let baseline_excluded = match interval.unpriced_prior {
        NativeTokenCounts::Codex { total, input, .. } => total > 0 || input > 0,
        NativeTokenCounts::ClaudeCode { .. } => return None,
    };
    Some(CodexObserved {
        first_at: interval.baseline.recorded_at,
        last_at: interval.final_snapshot.recorded_at,
        input,
        cached_input,
        output,
        total,
        baseline_excluded,
    })
}

fn default_counter_keys(store: &ConfigStore) -> Box<dyn DigestKeyStore + Send + Sync> {
    // A test build never reaches a keychain or writes a key file: a past test
    // leak left thousands of orphaned keychain items.
    if cfg!(any(test, feature = "test-credential-store")) {
        return Box::new(InMemoryDigestKeyStore::with_seed([0x7c; 32]));
    }
    match DIGEST_KEY_CUSTODY {
        DigestKeyCustody::OsKeychain => Box::new(OsCounterKeyStore),
        DigestKeyCustody::KeyFileInStore => Box::new(KeyFileInDir(
            crate::insights::turn_series::FileDigestKeyStore::new(
                store.daemon_path(COUNTER_KEY_DIR),
            ),
            store.daemon_path(COUNTER_KEY_DIR),
        )),
    }
}

/// The daemon's counter key in the OS keychain (owner decision D16, open):
/// its own entry, apart from the in-process Insights store's key.
struct OsCounterKeyStore;

impl OsCounterKeyStore {
    fn backend() -> Result<super::os_secret_store::OsSecretBackend, DigestKeyError> {
        crate::insights::turn_series::refuse_in_test_build()?;
        super::os_secret_store::OsSecretBackend::insights_digest_key(&crate::channel::KEYCHAIN)
            .map_err(|_| DigestKeyError::Unavailable("insights_counter_key_keychain"))
    }
}

impl DigestKeyStore for OsCounterKeyStore {
    fn load(&self) -> Result<Option<DigestKey>, DigestKeyError> {
        use super::credential_store::CredentialError;
        match Self::backend()?.read_insights_counter_key() {
            Ok(stored) => crate::insights::turn_series::decode_key(&stored).map(Some),
            Err(CredentialError::NoEntry) => Ok(None),
            Err(_) => Err(DigestKeyError::Unavailable("insights_counter_key_keychain")),
        }
    }

    fn load_or_create(&self) -> Result<DigestKey, DigestKeyError> {
        if let Some(key) = self.load()? {
            return Ok(key);
        }
        let bytes = crate::insights::turn_series::new_key_bytes()?;
        Self::backend()?
            .write_insights_counter_key(hex::encode(bytes).as_bytes())
            .map_err(|_| DigestKeyError::Unavailable("insights_counter_key_keychain"))?;
        // Read back: the stored key is the one every digest is made under.
        self.load()?
            .ok_or(DigestKeyError::Unavailable("insights_counter_key_keychain"))
    }

    fn clear(&self) -> Result<(), DigestKeyError> {
        use super::credential_store::CredentialError;
        match Self::backend()?.delete_insights_counter_key() {
            Ok(()) | Err(CredentialError::NoEntry) => Ok(()),
            Err(_) => Err(DigestKeyError::Unavailable("insights_counter_key_keychain")),
        }
    }
}

/// The key-file custody under D16, in a directory of its own made on first
/// use.
struct KeyFileInDir(crate::insights::turn_series::FileDigestKeyStore, PathBuf);

impl DigestKeyStore for KeyFileInDir {
    fn load(&self) -> Result<Option<DigestKey>, DigestKeyError> {
        self.0.load()
    }

    fn load_or_create(&self) -> Result<DigestKey, DigestKeyError> {
        std::fs::create_dir_all(&self.1)
            .map_err(|_| DigestKeyError::Unavailable("insights_counter_key_file"))?;
        self.0.load_or_create()
    }

    fn clear(&self) -> Result<(), DigestKeyError> {
        self.0.clear()
    }
}

/// Whether the Sessions methods show a row: the person's (not automated),
/// outside a Never folder. The one test the list, the drill, a label and
/// Analyze & save all make.
fn is_listed(row: &CounterRow, never: &NeverDigests) -> bool {
    !never.contains(&row.project) && !row.is_automated()
}

/// The row order and store tag a Sessions reference names
/// ([`CounterRow::listed_reference`]): `t`, plain digits, `.`, plain digits.
/// `None` for anything else, a reference from before the tag included.
fn parse_listed_reference(text: &str) -> Option<(u64, u64)> {
    let (seq, tag) = text.strip_prefix('t')?.split_once('.')?;
    let digits = |part: &str| {
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    };
    Some((digits(seq)?, digits(tag)?))
}

/// The listed row `reference` names in `store`, under `key`, with its
/// session digest. `None` when it names no listed row, or was answered from
/// another store (a new key, or a store begun again under the same one),
/// whose order now names other sessions.
fn find_listed<'s>(
    key: &DigestKey,
    store: &'s CounterStore,
    never: &NeverDigests,
    reference: &str,
) -> Option<(&'s KeyedDigest, &'s CounterRow)> {
    let (seq, tag) = parse_listed_reference(reference)?;
    if tag != store_tag_of(key, store) {
        return None;
    }
    store
        .rows
        .iter()
        .find(|(_, row)| row.seq == seq && is_listed(row, never))
}

/// Age out and cap the route tallies against the rows the store keeps.
fn prune_routing(store: &mut CounterStore, gone: &BTreeSet<KeyedDigest>, cutoff: DateTime<Utc>) {
    // The loose charge ages out by the hour, as a tally does by its last
    // call; gone entirely once no hour is left.
    if let Some(loose) = store.loose_charge.as_mut() {
        let first = super::insights_route_tally::unix_hour(cutoff);
        loose.by_hour.retain(|hour, _| *hour >= first);
        if loose.by_hour.is_empty() {
            store.loose_charge = None;
        }
    }
    let matched: BTreeSet<KeyedDigest> = store
        .rows
        .values()
        .filter_map(|row| row.harness_session)
        .collect();
    prune(
        &mut store.routing,
        &matched,
        gone,
        cutoff,
        COUNTER_STORE_MAX_SESSIONS + COUNTER_STORE_MAX_AUTOMATED_SESSIONS,
    );
}

/// After the routing ledger's refresh: fold its new calls into the route
/// tallies, only while both `insights_ledger_feed` (owner decision D3, on by
/// default) and `insights_counter_pass` (on by default; product owner,
/// 2026-10-29) are on. Gated on the pass too
/// because the tallies live in its store: unenroll turns the pass off and
/// clears the store, and a fold gated on the feed alone would write it back.
/// A poisoned settings lock reads as off. Failures are logged inside, by
/// fixed label only.
///
/// The fold itself runs on the counter pass's worker, as the pass runs off
/// the watcher tick: this reads the switches and, when both are on, the
/// ledger's 24-hour window (already in memory), hands both to the worker
/// and returns. The worker asks the switches again before it folds.
///
/// The managed routes' ledgers (`<config>/managed-routes/<uuid>`, the
/// Private AI sessions started from the app) are folded too, from the
/// daemon's managed-history snapshot, each by its own cursor, with or
/// without a shared `ledger`. They keep their rows, so the whole of each
/// is planned, not a 24-hour window.
pub(crate) fn fold_ledger_after_refresh(
    shared: &Arc<DaemonShared>,
    ledger: Option<&dyn crate::routing::RoutingLedger>,
) {
    if !fold_enabled(shared) {
        return;
    }
    let now = Utc::now();
    let window = ledger.map_or_else(Vec::new, |ledger| {
        ledger
            .exchanges_since(now - Duration::hours(crate::routing::ironwire::REFRESH_WINDOW_HOURS))
    });
    let managed = super::managed_history::snapshot(shared);
    if ledger.is_none() && !matches!(managed, Ok(Some(_))) {
        return;
    }
    submit_ledger_fold(
        shared,
        FoldJob {
            window,
            managed,
            now,
        },
    );
    // A test waits for the fold, unless it asked to watch it run on its
    // own; the daemon never does.
    #[cfg(test)]
    if !shared.insights_counter.detached_for_test() {
        shared.insights_counter.await_idle_for_test();
    }
}

/// Whether any proxy ledger feeds the route tallies: the shared proxy's,
/// or a managed route's (the Private AI sessions started from the app),
/// which are folded with or without a shared one.
pub(crate) fn any_ledger(shared: &DaemonShared) -> bool {
    shared.routing_ledger().is_some()
        || matches!(super::managed_history::snapshot(shared), Ok(Some(_)))
}

/// Both switches the fold needs, `insights_ledger_feed` and
/// `insights_counter_pass`. A poisoned settings lock reads as off.
fn fold_enabled(shared: &DaemonShared) -> bool {
    shared
        .settings
        .lock()
        .is_ok_and(|settings| settings.insights_ledger_feed && settings.insights_counter_pass)
}

/// Run one fold job on the worker. Failures are logged inside, by fixed
/// label only.
fn run_fold_job(shared: &DaemonShared, job: FoldJob) {
    let enabled = || !shared.shutdown.load(Ordering::Relaxed) && fold_enabled(shared);
    let mut speakers =
        || super::inference_map::speakers_from_rows(&super::harness::rows_now(shared));
    let ledger = crate::routing::FixedLedger::new(job.window);
    let homes = match &job.managed {
        Ok(Some(history)) => Some(history.rows_by_home()),
        Ok(None) => Some(ManagedRows::new()),
        Err(()) => None,
    };
    let _ = shared.insights_counter.fold_ledgers(
        &enabled,
        &ledger,
        homes.as_ref(),
        &mut speakers,
        job.now,
    );
}

/// Drop the oldest sessions, by last write and then by the order they were
/// counted, until every cap holds. Whole sessions go, never part of one, and
/// each is marked evicted. The person's rows (unknown ones included) answer
/// to the shared caps, `max_sessions`, `max_turns` and `max_tools`; automated
/// rows answer only to their own allowance, `max_automated`, so they never
/// push a session of the person's out. The pass does not read a session that
/// has no row and is no newer than the newest mark of the shared caps.
///
/// `mark_person` is false while a row still awaits its classification: an
/// eviction of the person's is then not marked, since the automated rows
/// not yet told apart may be what pushed it out. Marks from before the
/// allowance are dropped. Each allowance's marks are capped, oldest first:
/// the shared caps' at `max_sessions`, the automated allowance's at four
/// times that, since it evicts more often and its marks bound nothing but
/// their own sessions.
fn enforce_caps(
    store: &mut CounterStore,
    max_sessions: usize,
    max_turns: usize,
    max_tools: usize,
    max_automated: usize,
    mark_person: bool,
) {
    let person = || store.rows.values().filter(|row| !row.is_automated());
    let mut sessions = person().count();
    let mut turns: usize = person().map(CounterRow::turns).sum();
    let mut tools: usize = person().map(CounterRow::tool_calls).sum();
    let mut automated = store.rows.len() - sessions;
    let shared_over = |sessions: usize, turns: usize, tools: usize| {
        sessions > max_sessions || turns > max_turns || tools > max_tools
    };
    if shared_over(sessions, turns, tools) || automated > max_automated {
        let mut oldest: Vec<(DateTime<Utc>, u64, KeyedDigest)> = store
            .rows
            .iter()
            .map(|(session, row)| (row.modified_at, row.seq, *session))
            .collect();
        oldest.sort();
        for (_, _, session) in oldest {
            let shared = shared_over(sessions, turns, tools);
            if !shared && automated <= max_automated {
                break;
            }
            let Some(is_automated) = store.rows.get(&session).map(CounterRow::is_automated) else {
                continue;
            };
            if (is_automated && automated <= max_automated) || (!is_automated && !shared) {
                continue;
            }
            let Some(row) = store.rows.remove(&session) else {
                continue;
            };
            if is_automated {
                automated -= 1;
            } else {
                sessions -= 1;
                turns -= row.turns();
                tools -= row.tool_calls();
            }
            if is_automated || mark_person {
                store.evicted.insert(
                    session,
                    Evicted {
                        size_bytes: row.size_bytes,
                        modified_at: row.modified_at,
                        automated: Some(is_automated),
                    },
                );
            }
        }
    }
    store
        .evicted
        .retain(|_, evicted| evicted.automated.is_some());
    // The eviction marks are capped too, oldest going first: those are the
    // nearest to aging out. The newest mark is never the one dropped, and
    // every dropped mark is no newer than it, so the pass still skips the
    // files the shared caps' dropped marks named.
    for (class, cap) in [
        (false, max_sessions),
        (true, max_sessions.saturating_mul(4)),
    ] {
        let mut marks: Vec<(DateTime<Utc>, KeyedDigest)> = store
            .evicted
            .iter()
            .filter(|(_, evicted)| evicted.automated == Some(class))
            .map(|(session, evicted)| (evicted.modified_at, *session))
            .collect();
        if marks.len() > cap {
            marks.sort();
            let excess = marks.len() - cap;
            for (_, session) in marks.into_iter().take(excess) {
                store.evicted.remove(&session);
            }
        }
    }
}

/// The Codex series budget (owner decision, 2026-10-20): past either
/// bound, Codex rows lose their per-turn series until both hold: automated
/// rows first (nothing draws their series, so a burst of them never trims
/// the person's), then the person's; each group oldest first, by last
/// write and then by the order they were counted. Only the
/// series goes: the row, its observed change, dates and facts stay, so the
/// session still counts in every weekly figure. A trimmed row is not
/// evicted and gets no eviction mark; it is not read again while its file
/// is unchanged, because the pass's unchanged test does not look at the
/// series. A file that changes is read again whole, series and all, as the
/// newest Codex row, and the budget then trims older ones.
/// Holds the sub-agent rows to their allowance: past any of the three
/// bounds the oldest (by last write, then order made) go first, and the
/// newest last write evicted becomes the floor below which a transcript with
/// no row is not read. Never touches the person's rows.
fn enforce_spawned_caps(
    store: &mut CounterStore,
    max_sessions: usize,
    max_turns: usize,
    max_tools: usize,
) {
    let mut oldest_first: Vec<(DateTime<Utc>, u64, KeyedDigest)> = store
        .spawned
        .iter()
        .map(|(session, spawned)| (spawned.row.modified_at, spawned.row.seq, *session))
        .collect();
    oldest_first.sort();
    let mut turns: usize = store.spawned.values().map(|s| s.row.turns()).sum();
    let mut tools: usize = store.spawned.values().map(|s| s.row.tool_calls()).sum();
    for (modified_at, _, session) in oldest_first {
        if store.spawned.len() <= max_sessions && turns <= max_turns && tools <= max_tools {
            break;
        }
        if let Some(evicted) = store.spawned.remove(&session) {
            turns -= evicted.row.turns();
            tools -= evicted.row.tool_calls();
            store.spawned_floor = Some(
                store
                    .spawned_floor
                    .map_or(modified_at, |f| f.max(modified_at)),
            );
        }
    }
}

fn trim_codex_series(store: &mut CounterStore, max_turns: usize, max_tools: usize) {
    let mut turns: usize = store
        .rows
        .values()
        .map(CounterRow::codex_series_turns)
        .sum();
    let mut tools: usize = store
        .rows
        .values()
        .map(CounterRow::codex_series_tool_calls)
        .sum();
    if turns <= max_turns && tools <= max_tools {
        return;
    }
    // Automated rows first: nothing draws their series.
    let mut oldest: Vec<(bool, DateTime<Utc>, u64, KeyedDigest)> = store
        .rows
        .iter()
        .filter(|(_, row)| {
            matches!(
                row.body,
                RowBody::Codex {
                    series: Some(_),
                    ..
                }
            )
        })
        .map(|(session, row)| (!row.is_automated(), row.modified_at, row.seq, *session))
        .collect();
    oldest.sort();
    for (_, _, _, session) in oldest {
        if turns <= max_turns && tools <= max_tools {
            break;
        }
        let Some(row) = store.rows.get_mut(&session) else {
            continue;
        };
        turns -= row.codex_series_turns();
        tools -= row.codex_series_tool_calls();
        if let RowBody::Codex {
            series,
            series_trimmed,
            ..
        } = &mut row.body
        {
            *series = None;
            *series_trimmed = true;
        }
    }
}

/// `YYYY-Www`, naming a real ISO week. Its Monday.
fn parse_iso_week(text: &str) -> Option<NaiveDate> {
    let bytes = text.as_bytes();
    if bytes.len() != 8
        || bytes[4] != b'-'
        || bytes[5] != b'W'
        || !bytes[..4].iter().all(u8::is_ascii_digit)
        || !bytes[6..].iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    let year: i32 = text[..4].parse().ok()?;
    let week: u32 = text[6..].parse().ok()?;
    NaiveDate::from_isoywd_opt(year, week, Weekday::Mon)
}

/// Each folder's label by its project key's digest under `key`, for
/// matching a row's folder digest at answer time; empty with no key.
fn folder_digests<'f>(
    key: Option<&DigestKey>,
    folders: &'f [(String, String)],
) -> BTreeMap<KeyedDigest, &'f str> {
    match key {
        Some(key) => folders
            .iter()
            .map(|(project_key, label)| (project_digest(key, project_key), label.as_str()))
            .collect(),
        None => BTreeMap::new(),
    }
}

/// One `insights_sessions` row: its opaque reference, harness, first
/// recorded event, turns, tokens and sparkline, its folder's label, the
/// task the rules infer and the person's own labels. Never a digest or a
/// path.
fn session_row_value(
    row: &CounterRow,
    store_tag: u64,
    label: Option<&SessionLabel>,
    folders: &BTreeMap<KeyedDigest, &str>,
) -> serde_json::Value {
    use crate::insights::session_sparkline;
    let source = match &row.body {
        RowBody::Claude { .. } => AnalyticsSource::ClaudeCode,
        RowBody::Codex { .. } => AnalyticsSource::Codex,
        RowBody::Unknown { source, .. } => *source,
    };
    // The per-turn series, Claude Code's or Codex's (owner decision D11,
    // settled: served only while `CODEX_PER_TURN_SERIES` holds); a Codex
    // row made before the per-turn reader, or trimmed by the Codex budget,
    // has none.
    let series = row.listed_series(CODEX_PER_TURN_SERIES);
    let turns = series.map(|series| series.turns.len());
    // The row's tokens as the drill's header counts them: for Codex always
    // the observed change, series or not, so a trimmed series never moves
    // the figure. The sparkline is over the per-turn series.
    let (tokens, sparkline) = match &row.body {
        RowBody::Codex { observed, .. } => (
            observed.as_ref().map(|observed| observed.total),
            series.and_then(|series| session_sparkline::session_sparkline(&series.turns)),
        ),
        _ => match series {
            Some(series) => (
                session_sparkline::known_tokens(&series.turns),
                session_sparkline::session_sparkline(&series.turns),
            ),
            None => (None, None),
        },
    };
    serde_json::json!({
        "session_ref": row.listed_reference(store_tag),
        "source": source,
        "started_at": row.listed_at().map(|at| at.to_rfc3339()),
        "turns": turns,
        "tokens": tokens,
        "sparkline": sparkline,
        "folder": folders.get(&row.project).copied(),
        "inferred_task": row.inferred_task(),
        "user_task": label.and_then(|label| label.task),
        "user_outcome": label.and_then(|label| label.outcome),
        // Whether Analyze & save can analyze it; `false` when the core's
        // whole-file analysis refuses its file, so a shell does not offer it.
        "analyzable": row.not_analyzable.is_none(),
    })
}

/// The week's `/goal` goals (product owner, 2026-10-29): each goal whose
/// last update falls in the local week starting `week_start`, on a row the
/// Sessions list shows (not automated, not in a Never folder), with that
/// row's folder label and listed reference, so the Sessions drill opens it.
///
/// Claude's goals are the row's stored ones. Codex's are read now from its
/// goals database and joined to a Codex row by the keyed digest of the
/// thread ID, which is the session ID the row's `harness_session` names; a
/// goal no listed row matches (its rollout aged out, was archived or is in
/// a Never folder) is left out. Only status, tokens, time, checks and the
/// time of the update are answered, never a goal's text.
///
/// A goal is counted once however many rows hold it: a resumed Claude
/// transcript repeats the goal records of the one it continues, told apart
/// by when the goal was set, and a Codex thread is one goal by its digest.
/// Of the copies, the one updated last is answered.
fn week_goals(
    store: &CounterStore,
    key: &DigestKey,
    never: &NeverDigests,
    folders: &BTreeMap<KeyedDigest, &str>,
    codex_goals_db: Option<&std::path::Path>,
    (week_start, tz): (NaiveDate, FixedOffset),
) -> WeekGoals {
    let listed = || store.rows.values().filter(|row| is_listed(row, never));
    let any_codex = listed()
        .any(|row| row.harness_session.is_some() && matches!(row.body, RowBody::Codex { .. }));
    let codex = match codex_goals_db {
        Some(path) if any_codex => super::insights_codex_goals::read_codex_goals(path, key),
        _ => BTreeMap::new(),
    };
    let in_week =
        |at: Option<DateTime<Utc>>| at.filter(|at| local_week_start(at, &tz) == week_start);
    let tag = store_tag_of(key, store);
    let mut goals = Vec::new();
    // One per Claude goal set time and per Codex thread, the latest update.
    let mut claude_once: BTreeMap<DateTime<Utc>, WeekGoal> = BTreeMap::new();
    // A Codex goal is one database row: on two rows, the reference is the
    // one written last.
    let mut codex_once: BTreeMap<KeyedDigest, (DateTime<Utc>, WeekGoal)> = BTreeMap::new();
    let keep_latest = |slot: Option<&WeekGoal>, goal: &WeekGoal| {
        slot.is_none_or(|kept| goal.updated_at > kept.updated_at)
    };
    for row in listed() {
        let folder = folders.get(&row.project).map(|label| (*label).to_owned());
        for goal in &row.goals {
            let Some(updated_at) = in_week(goal.updated_at) else {
                continue;
            };
            let status = GoalStatus::from(goal.status);
            let answered = WeekGoal {
                tool: AnalyticsSource::ClaudeCode,
                status,
                met: status.met(),
                tokens: goal.tokens,
                time_secs: goal.duration_ms.map(|ms| ms / 1_000),
                iterations: goal.iterations,
                folder: folder.clone(),
                session_ref: row.listed_reference(tag),
                updated_at,
            };
            match goal.set_at {
                Some(set_at) => {
                    if keep_latest(claude_once.get(&set_at), &answered) {
                        claude_once.insert(set_at, answered);
                    }
                }
                None => goals.push(answered),
            }
        }
        if !matches!(row.body, RowBody::Codex { .. }) {
            continue;
        }
        let Some((digest, goal)) = row
            .harness_session
            .and_then(|digest| codex.get(&digest).map(|goal| (digest, goal)))
        else {
            continue;
        };
        let Some(updated_at) = in_week(goal.updated_at) else {
            continue;
        };
        let answered = WeekGoal {
            tool: AnalyticsSource::Codex,
            status: goal.status,
            met: goal.status.met(),
            tokens: goal.tokens_used,
            time_secs: goal.time_used_secs,
            iterations: None,
            folder,
            session_ref: row.listed_reference(tag),
            updated_at,
        };
        if codex_once
            .get(&digest)
            .is_none_or(|(written, _)| row.modified_at > *written)
        {
            codex_once.insert(digest, (row.modified_at, answered));
        }
    }
    goals.extend(claude_once.into_values());
    goals.extend(codex_once.into_values().map(|(_, goal)| goal));
    WeekGoals::new(goals)
}

/// What the answer joins to one engine row, by its `t{seq}` reference.
/// An answer for a store or key that could not be read: never empty, never
/// zero.
fn unreadable_value(reason: &str) -> serde_json::Value {
    serde_json::json!({
        "enabled": true,
        "feed": FEED_COUNTER_PASS,
        "readable": false,
        "reason": reason,
    })
}

fn folder_labels_by_digest<'a>(
    key: &DigestKey,
    folders: &'a [(String, String)],
) -> BTreeMap<KeyedDigest, &'a str> {
    folders
        .iter()
        .map(|(project, label)| (project_digest(key, project), label.as_str()))
        .collect()
}

struct RowJoin<'a> {
    session_ref: String,
    folder: Option<&'a str>,
    started_at: Option<DateTime<Utc>>,
    last_event_at: Option<DateTime<Utc>>,
    tally: Option<&'a RouteTally>,
}

/// The rollup as it crosses the socket: the per-session rows keep their
/// figures and coverage, gain `started_at`, `last_event_at`, `routing` and the Sessions
/// list's folder label, and keep the same opaque reference.
///
/// - `started_at` is the transcript's first recorded event
///   (`CounterRow::first_event_at`), RFC 3339, or `null`; never discovery's
///   file-write time (owner question Q1, default taken: a row may carry it,
///   though it matches a queue entry's `started_at`; the shell must still
///   not join the two).
/// - `last_event_at` is the transcript's latest recorded event
///   (`CounterRow::latest_event_at`), RFC 3339, or `null` for older or missing
///   metadata. It indicates recorded activity, never session completion,
///   and never falls back to discovery's file-write time or the current time.
/// - `routing` is the session's route tally in fixed labels and counts
///   (`insights_route_tally::routing_value`), or `null` on every row while
///   routing is unavailable.
/// - `session_ref` is `CounterRow::reference()`, never a native session ID;
///   `folder` is the Sessions list's resolved display label. Both are null
///   when no matching metadata is present.
/// - Never a native ID, a digest, a path, a backend, a model, a price or a
///   title. The NEAR AI charge crosses beside the rollup, in
///   `near_ai_credit`, never on a row.
fn rollup_value(
    rollup: &WeekRollup,
    joined: &BTreeMap<String, RowJoin<'_>>,
    routing_available: bool,
) -> serde_json::Value {
    let mut value = serde_json::to_value(rollup).unwrap_or(serde_json::Value::Null);
    if let Some(sessions) = value.get_mut("sessions").and_then(|s| s.as_array_mut()) {
        for session in sessions {
            if let Some(object) = session.as_object_mut() {
                let join = object
                    .remove("session_ref")
                    .and_then(|reference| reference.as_str().and_then(|r| joined.get(r)));
                object.insert(
                    "session_ref".to_string(),
                    serde_json::json!(join.map(|join| &join.session_ref)),
                );
                object.insert(
                    "folder".to_string(),
                    serde_json::json!(join.and_then(|join| join.folder)),
                );
                let started_at = join
                    .and_then(|join| join.started_at)
                    .map(|at| at.to_rfc3339());
                object.insert("started_at".to_string(), serde_json::json!(started_at));
                let last_event_at = join
                    .and_then(|join| join.last_event_at)
                    .map(|at| at.to_rfc3339());
                object.insert(
                    "last_event_at".to_string(),
                    serde_json::json!(last_event_at),
                );
                let routing = if routing_available {
                    routing_value(join.and_then(|join| join.tally))
                } else {
                    serde_json::Value::Null
                };
                object.insert("routing".to_string(), routing);
            }
        }
    }
    value
}

/// Every folder that is Never, the contribution override included (owner
/// decision D4): under a global Never override, every folder.
/// `None` when the folder rules cannot be read: the Never folders are then
/// unknown, and an unknown is never read as "none".
pub(crate) fn never_project_keys(shared: &DaemonShared) -> Option<NeverFolders> {
    let policy = shared.policy.lock().ok()?;
    Some(never_folders(&policy))
}

/// The Never folders under `policy`, by each folder's resolved rule.
pub(crate) fn never_folders(policy: &super::policy::ProjectPolicy) -> NeverFolders {
    if policy.holds_every_send() {
        return NeverFolders::Every;
    }
    NeverFolders::These(
        policy
            .projects
            .keys()
            .filter(|key| policy.resolve(key) == super::policy::ProjectMode::Ignore)
            .cloned()
            .collect(),
    )
}

/// The sessions sent through NEAR AI, by the engine's session reference,
/// for `work` to leave their list estimate out of the spend (D5). None
/// while routing is unavailable: no tally is read then (D3).
fn near_ai_routes(
    joined: &BTreeMap<String, RowJoin<'_>>,
    routing_available: bool,
) -> BTreeMap<String, super::insights_week_work::NearAiRoute> {
    use super::insights_route_tally::{RouteCategory, route_category};
    use super::insights_week_work::NearAiRoute;
    if !routing_available {
        return BTreeMap::new();
    }
    joined
        .iter()
        .filter_map(|(reference, join)| {
            let route = match route_category(join.tally).0 {
                RouteCategory::RoutedVerified
                | RouteCategory::RoutedUnverified
                | RouteCategory::CheckFailed => NearAiRoute::Wholly,
                RouteCategory::Mixed => NearAiRoute::Partly,
                RouteCategory::Unobserved | RouteCategory::Unrecorded | RouteCategory::Outside => {
                    return None;
                }
            };
            Some((reference.clone(), route))
        })
        .collect()
}

/// Each row's route for the Models answers, by the engine's session
/// reference: the category of its session's tally. While routing is
/// unavailable (the ledger feed off, or no proxy ledger declared) every row
/// is `Unobserved`, so a tally folded earlier is not read.
fn route_map(
    rows: &[&CounterRow],
    routing: &BTreeMap<KeyedDigest, RouteTally>,
    routing_available: bool,
) -> BTreeMap<String, ModelRoute> {
    use super::insights_route_tally::{RouteCategory, route_category};
    rows.iter()
        .map(|row| {
            let route = if routing_available {
                let tally = row
                    .harness_session
                    .as_ref()
                    .and_then(|digest| routing.get(digest));
                match route_category(tally).0 {
                    RouteCategory::Unobserved => ModelRoute::Unobserved,
                    RouteCategory::Unrecorded => ModelRoute::Unrecorded,
                    RouteCategory::Outside => ModelRoute::Outside,
                    RouteCategory::Mixed => ModelRoute::Mixed,
                    RouteCategory::CheckFailed => ModelRoute::CheckFailed,
                    RouteCategory::RoutedVerified => ModelRoute::RoutedVerified,
                    RouteCategory::RoutedUnverified => ModelRoute::RoutedUnverified,
                }
            } else {
                ModelRoute::Unobserved
            };
            (row.reference(), route)
        })
        .collect()
}

/// The tool a stored row was read from, as the rollup's `source` spells
/// it (`claude_code`, `codex`).
fn row_tool(row: &CounterRow) -> &'static str {
    let source = match &row.body {
        RowBody::Claude { .. } => AnalyticsSource::ClaudeCode,
        RowBody::Codex { .. } => AnalyticsSource::Codex,
        RowBody::Unknown { source, .. } => *source,
    };
    match source {
        AnalyticsSource::ClaudeCode => "claude_code",
        AnalyticsSource::Codex => "codex",
    }
}

/// Each row's NEAR AI charge for the Models answers, by the engine's
/// session reference: absent when its session sent no call through NEAR AI
/// (or routing is unavailable, as in `route_map`). Empty while
/// `ledger_money` (`LEDGER_MONEY_IN_INSIGHTS`, owner decision D5) is held,
/// so no stored charge is read. The week's `near_ai_credit`
/// (`near_ai_credit_value`) reads the same charges by window.
fn charge_map(
    rows: &[&CounterRow],
    routing: &BTreeMap<KeyedDigest, RouteTally>,
    routing_available: bool,
    ledger_money: bool,
) -> BTreeMap<String, crate::insights::model_answers::SessionCharge> {
    if !routing_available || !ledger_money {
        return BTreeMap::new();
    }
    rows.iter()
        .filter_map(|row| {
            let tally = routing.get(row.harness_session.as_ref()?)?;
            Some((row.reference(), session_charge(tally)?))
        })
        .collect()
}

/// `insights_week`'s `near_ai_credit`: where NEAR AI credit went
/// (product owner, 2026-10-28), from every route tally's charge
/// (`insights_credit`). `week` is the selected local week, by folder and
/// served model; `today`, `by_day` (the last 7 local days, oldest first) and
/// `last_31_days` (today and the 30 local days before it) are sums only,
/// and do not move with the selected week.
///
/// A tally is grouped by the folder of its session's stored row, matched
/// by the keyed harness-session digest; a tally with no row (a session the
/// pass has not read, or whose row aged out) is in the sums and in the
/// unknown folder. A row in a Never folder is not read; its tally is
/// already gone (`NeverSessions`).
fn near_ai_credit_value(
    store: &CounterStore,
    key: &DigestKey,
    never_project_keys: &NeverFolders,
    folders: &[(String, String)],
    week_start: NaiveDate,
    tz: FixedOffset,
    now: DateTime<Utc>,
) -> serde_json::Value {
    use super::insights_credit::{CreditSession, window_value};
    let never = never_digests(key, never_project_keys);
    let labels = folder_labels_by_digest(key, folders);
    let by_session: BTreeMap<KeyedDigest, &CounterRow> = store
        .rows
        .values()
        .filter(|row| !never.contains(&row.project))
        .filter_map(|row| Some((row.harness_session?, row)))
        .collect();
    let mut sessions: Vec<CreditSession<'_>> = store
        .routing
        .iter()
        .filter_map(|(digest, tally)| {
            let charge = tally.charge.as_ref()?;
            let row = by_session.get(digest);
            Some(CreditSession {
                charge,
                last_call_at: tally.last_call_at,
                folder: row.and_then(|row| labels.get(&row.project).copied()),
                counts_session: true,
            })
        })
        .collect();
    // Spend no session could be named for: in every sum and in each
    // unknown group, as no trace.
    if let Some(loose) = store.loose_charge.as_ref() {
        sessions.push(CreditSession {
            charge: loose,
            last_call_at: now,
            folder: None,
            counts_session: false,
        });
    }
    let midnight = |date: NaiveDate| {
        date.and_hms_opt(0, 0, 0)
            .and_then(|local| local.and_local_timezone(tz).single())
            .map_or(now, |at| at.with_timezone(&Utc))
    };
    let today = now.with_timezone(&tz).date_naive();
    let tomorrow = midnight(today + Duration::days(1));
    // The last 7 local days, oldest first, today last (owner, 2026-10-28:
    // the NEAR AI card's spend by day).
    let by_day: Vec<serde_json::Value> = (0..7)
        .rev()
        .map(|back| {
            let day = today - Duration::days(back);
            let mut value = window_value(
                &sessions,
                midnight(day),
                midnight(day + Duration::days(1)),
                false,
            );
            value["date"] = serde_json::json!(day.to_string());
            value
        })
        .collect();
    serde_json::json!({
        "basis": "near_ai_ledger",
        "week": window_value(
            &sessions,
            midnight(week_start),
            midnight(week_start + Duration::days(7)),
            true,
        ),
        "today": window_value(&sessions, midnight(today), tomorrow, false),
        "by_day": by_day,
        "last_31_days": window_value(
            &sessions,
            midnight(today - Duration::days(30)),
            tomorrow,
            false,
        ),
    })
}

/// `insights_week`'s `usage_by_day` (product owner, 2026-10-28): Claude and
/// Codex tokens on each of the last 7 local days, oldest first, today last,
/// for the usage cards' bars. It does not move with the week on screen.
/// Claude's are the four series a day stacks on "Tokens by day"; Codex's
/// are its per-turn series split by day (`WeekRollup::codex_by_day`). A day
/// is `null` (unknown, never 0) when a session on it could not be counted
/// or split; a week with no session of that tool is a true 0.
fn usage_by_day_value(
    inputs: &[SessionInput],
    now: DateTime<Utc>,
    tz: FixedOffset,
) -> serde_json::Value {
    let today = now.with_timezone(&tz).date_naive();
    let this = local_week_start(&now, &tz);
    let weeks = [this - Duration::days(7), this]
        .map(|start| week_rollup(Feed::CounterPass, inputs, start, &tz));
    let has = |week: &WeekRollup, source: AnalyticsSource| {
        week.sources.iter().any(|line| line.source == source)
    };
    let days: Vec<serde_json::Value> = (0..7)
        .rev()
        .map(|back| {
            let day = today - Duration::days(back);
            let week = weeks
                .iter()
                .find(|week| day >= week.week_start && day < week.week_start + Duration::days(7));
            let (claude, codex) = week.map_or((None, None), |week| {
                let offset = usize::try_from((day - week.week_start).num_days()).unwrap_or(0);
                let claude = match &week.by_day {
                    Some(days) => days
                        .get(offset)
                        .map(|d| d.uncached + d.cache_read + d.cache_write + d.output),
                    None => (!has(week, AnalyticsSource::ClaudeCode)).then_some(0),
                };
                let codex = match &week.codex_by_day {
                    Some(days) => days.get(offset).copied().flatten(),
                    None => (!has(week, AnalyticsSource::Codex)).then_some(0),
                };
                (claude, codex)
            });
            serde_json::json!({"date": day.to_string(), "claude_code": claude, "codex": codex})
        })
        .collect();
    serde_json::Value::Array(days)
}

/// `insights_week`.
pub fn handle_week(shared: &DaemonShared, req: &Request) -> Response {
    let week = match req.params.get("iso_week") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => match value.as_str().and_then(parse_iso_week) {
            Some(monday) => Some(monday),
            None => return Response::err(req.id, ERR_BAD_PARAMS, ERR_ISO_WEEK_INVALID),
        },
    };
    let tz = match req.params.get("tz") {
        None | Some(serde_json::Value::Null) => chrono::Local::now().offset().fix(),
        Some(_) => match super::insights_glance::parse_tz(&req.params) {
            Some(tz) => tz,
            None => {
                return Response::err(
                    req.id,
                    ERR_BAD_PARAMS,
                    super::insights_glance::ERR_TZ_INVALID,
                );
            }
        },
    };
    // A poisoned lock reads as off: fail closed, never a guess.
    let (enabled, mut options) = match shared.settings.lock() {
        Ok(settings) => (
            settings.insights_counter_pass,
            WeekOptions {
                recap_card_enabled: settings.insights_recap_card_enabled,
                context_threshold: settings.insights_context_threshold,
                ledger_feed: settings.insights_ledger_feed,
                ledger_present: false,
            },
        ),
        Err(_) => (false, WeekOptions::default()),
    };
    // Asked after the settings lock is let go. Only whether a ledger is
    // declared: the answer never reads the ledger itself.
    options.ledger_present = options.ledger_feed && any_ledger(shared);
    let never = if enabled {
        match never_project_keys(shared) {
            Some(keys) => keys,
            None => {
                return Response::ok(
                    req.id,
                    serde_json::json!({
                        "enabled": true,
                        "feed": FEED_COUNTER_PASS,
                        "readable": false,
                        "reason": UNREADABLE_POLICY,
                    }),
                );
            }
        }
    } else {
        NeverFolders::NONE
    };
    let folders = if enabled {
        super::insights_sessions::folder_labels(shared)
    } else {
        Vec::new()
    };
    Response::ok(
        req.id,
        shared.insights_counter.week_value_with_folders(
            enabled,
            week,
            tz,
            Utc::now(),
            &never,
            (options, &folders),
        ),
    )
}

mod benchmarks;
pub(crate) mod submitted;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::path::Path;
    use trace_commons_protocol::insights_usage_series::InMemoryDigestKeyStore;

    // `insights_week/tests/compressed_tests.rs`: compressed Codex rollouts
    // and managed-profile sessions, over this module's fixture.
    mod compressed_tests;
    // `insights_week/tests/spawned_tests.rs`: Claude sub-agent transcripts
    // as rows of their own.
    mod spawned_tests;

    const QUIET: u64 = 1_800;
    const PROJECT: &str = "/Users/PRIVATE_USER/code/PRIVATE_REPO";

    fn claude_bytes() -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/insights/claude-turn-series/session.jsonl"),
        )
        .unwrap()
    }

    /// One Codex session on Monday 2026-09-14: 80 input (40 cached) and 30
    /// output between the first and last counter, after a nonzero baseline.
    fn codex_bytes() -> Vec<u8> {
        codex_bytes_for("PRIVATE-CODEX-ID")
    }

    /// `codex_bytes` for the session ID `id`.
    fn codex_bytes_for(id: &str) -> Vec<u8> {
        let usage = |at: &str, input: u64, cached: u64, output: u64| {
            serde_json::json!({
                "type": "event_msg",
                "timestamp": at,
                "payload": {"type": "token_count", "info": {"total_token_usage": {
                    "input_tokens": input, "cached_input_tokens": cached,
                    "output_tokens": output, "reasoning_output_tokens": 0,
                    "total_tokens": input + output
                }}}
            })
        };
        [
            serde_json::json!({"type": "session_meta", "payload": {"id": id, "model_provider": "openai"}}),
            serde_json::json!({"type": "turn_context", "payload": {"model": "gpt-6"}}),
            usage("2026-09-14T09:00:00Z", 100, 60, 20),
            serde_json::json!({"type": "response_item", "payload": {"type": "message", "role": "assistant", "content": []}}),
            usage("2026-09-14T09:01:00Z", 180, 100, 50),
        ]
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
    }

    /// The fixture's three known turns.
    const CLAUDE_TOKENS: u64 =
        (3 + 12_000 + 40) + (5 + 12_000 + 500 + 60) + (7 + 12_500 + 100 + 20);

    /// Last written 10:00 UTC on Monday 2026-09-14.
    fn written() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 14, 10, 0, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        written() + Duration::hours(2)
    }

    fn monday() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 14).unwrap()
    }

    fn utc() -> FixedOffset {
        FixedOffset::east_opt(0).unwrap()
    }

    struct Fixture {
        dir: tempfile::TempDir,
        pass: CounterPass,
    }

    impl Fixture {
        fn new() -> Self {
            Self::keyed(3)
        }

        fn keyed(seed: u8) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let pass = CounterPass::with_keys(
                dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([seed; 32])),
            );
            Self { dir, pass }
        }

        fn rekeyed(&self, seed: u8) -> CounterPass {
            CounterPass::with_keys(
                self.dir.path().join(COUNTER_ROWS_FILE),
                Box::new(InMemoryDigestKeyStore::with_seed([seed; 32])),
            )
        }

        /// A new process over the same store whose keychain already holds
        /// the fixture's key. `rekeyed` holds no key until a pass makes one,
        /// so a fold through it returns before reading the store at all.
        fn reopened(&self) -> CounterPass {
            let pass = self.rekeyed(3);
            pass.keys.load_or_create().unwrap();
            pass
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.dir.path().join(name);
            std::fs::write(&path, bytes).unwrap();
            path
        }

        fn rows_text(&self) -> String {
            std::fs::read_to_string(self.dir.path().join(COUNTER_ROWS_FILE)).unwrap()
        }

        fn run(&self, candidates: &[CounterCandidate]) -> PassSummary {
            run_with(&self.pass, candidates, &[], false)
        }

        fn week(&self, never: &[String]) -> serde_json::Value {
            self.week_with(Some(monday()), never, WeekOptions::default())
        }

        fn week_with(
            &self,
            week: Option<NaiveDate>,
            never: &[String],
            options: WeekOptions,
        ) -> serde_json::Value {
            self.pass.week_value(
                true,
                week,
                utc(),
                now(),
                &NeverFolders::These(never.to_vec()),
                options,
            )
        }

        /// The week with the ledger feed on and a proxy ledger declared.
        fn week_routed(&self) -> serde_json::Value {
            self.week_with(Some(monday()), &[], routed())
        }
    }

    fn routed() -> WeekOptions {
        WeekOptions {
            ledger_feed: true,
            ledger_present: true,
            ..WeekOptions::default()
        }
    }

    fn sessions_of(value: &serde_json::Value) -> Vec<serde_json::Value> {
        value["rollup"]["sessions"].as_array().unwrap().clone()
    }

    fn run_with(
        pass: &CounterPass,
        candidates: &[CounterCandidate],
        never_keys: &[String],
        never: bool,
    ) -> PassSummary {
        let never_keys = NeverFolders::These(never_keys.to_vec());
        pass.run_pass(candidates, &never_keys, now(), QUIET, &mut |_| {
            (PROJECT.to_string(), never)
        })
        .unwrap()
    }

    fn candidate(source: &'static str, path: &Path) -> CounterCandidate {
        CounterCandidate {
            origin: 0,
            source,
            path: path.to_path_buf(),
            size_bytes: std::fs::metadata(path).map(|m| m.len()).unwrap_or(1),
            modified_at: written(),
            started_at: Some(written() - Duration::hours(1)),
            group_member_count: 0,
            role: CandidateRole::Watched,
        }
    }

    fn claude_line(value: &serde_json::Value) -> serde_json::Value {
        value["rollup"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["source"] == "claude_code")
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }

    #[test]
    fn a_quiesced_session_gets_one_row_holding_nothing_readable() {
        let f = Fixture::new();
        let path = f.write("PRIVATE-SESSION.jsonl", &claude_bytes());
        let summary = f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        assert_eq!(summary.read, 1);
        let text = f.rows_text();
        for forbidden in [
            "PRIVATE",
            "SESSION",
            "/Users",
            path.to_str().unwrap(),
            f.dir.path().to_str().unwrap(),
        ] {
            assert!(!text.contains(forbidden), "{forbidden} stored: {text}");
        }
        let value = f.week(&[]);
        assert_eq!(value["enabled"], true);
        assert_eq!(value["feed"], "counter_pass");
        assert_eq!(value["readable"], true);
        assert_eq!(value["iso_week"], "2026-W38");
        assert_eq!(value["week_start"], "2026-09-14");
        assert_eq!(value["sessions_stored"], 1);
        let line = claude_line(&value);
        assert_eq!(line["tokens"], CLAUDE_TOKENS);
        assert_eq!(line["sessions"], 1);
        assert_eq!(
            value["rollup"]["coverage"]["partial"], 1,
            "two turns unknown"
        );
    }

    #[test]
    fn an_unchanged_session_is_not_read_again_and_a_grown_one_replaces_its_row() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let first = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(&[first]).read, 1);
        let again = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(&[again]).read, 0, "same size and time: no read");

        let mut grown = candidate(SOURCE_CLAUDE_CODE, &path);
        grown.size_bytes += 1;
        grown.modified_at = written() + Duration::minutes(1);
        assert_eq!(f.run(&[grown]).read, 1);
        let value = f.week(&[]);
        assert_eq!(value["sessions_stored"], 1, "replaced, not added");
        assert_eq!(claude_line(&value)["tokens"], CLAUDE_TOKENS);
    }

    #[test]
    fn a_session_still_being_written_is_not_read() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let mut busy = candidate(SOURCE_CLAUDE_CODE, &path);
        busy.modified_at = now() - Duration::seconds(QUIET as i64 - 1);
        assert_eq!(f.run(&[busy]).read, 0);
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
    }

    #[test]
    fn only_claude_code_and_codex_are_counted() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        for source in [
            crate::source::SOURCE_TRAJECTORY,
            crate::source::SOURCE_GEMINI_CLI,
            crate::source::SOURCE_CLINE,
            crate::source::SOURCE_OPENCODE,
        ] {
            assert_eq!(f.run(&[candidate(source, &path)]).read, 0, "{source}");
        }
    }

    #[test]
    fn a_never_folder_is_not_read_and_its_rows_are_dropped() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        // A Never folder from the start: not read.
        let summary = run_with(&f.pass, &[candidate(SOURCE_CLAUDE_CODE, &path)], &[], true);
        assert_eq!(summary.read, 0);

        // Counted, then the folder is set to Never: hidden from the next
        // read at once, and dropped by the next pass, even if the session
        // is no longer on disk.
        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        let never = vec![PROJECT.to_string()];
        let hidden = f.week(&never);
        assert_eq!(hidden["rollup"]["coverage"]["known"], 0);
        assert_eq!(hidden["rollup"]["coverage"]["partial"], 0);
        let summary = run_with(&f.pass, &[], &never, false);
        assert_eq!(summary.dropped, 1);
        assert_eq!(f.week(&[])["sessions_stored"], 0);
    }

    /// Under a global Never override every folder is Never, including ones
    /// with no rule of their own, whose keys no list holds: hidden from the
    /// next read at once, and dropped by the next pass even when the
    /// session is no longer on disk.
    #[test]
    fn every_folder_never_hides_and_drops_every_row() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        let hidden = f.pass.week_value(
            true,
            Some(monday()),
            utc(),
            now(),
            &NeverFolders::Every,
            WeekOptions::default(),
        );
        assert_eq!(hidden["sessions_stored"], 0);
        assert_eq!(hidden["rollup"]["coverage"]["known"], 0);

        let summary = f
            .pass
            .run_pass(&[], &NeverFolders::Every, now(), QUIET, &mut |_| {
                (PROJECT.to_string(), false)
            })
            .unwrap();
        assert_eq!(summary.dropped, 1);
        assert_eq!(f.week(&[])["sessions_stored"], 0);
    }

    /// The Never folders are those whose resolved rule is Never: a global
    /// Never override makes it every folder, whatever its own rule.
    #[test]
    fn never_project_keys_follow_the_contribution_override() {
        let (_dir, s) = shared();
        let at = Utc::now();
        {
            let mut policy = s.policy.lock().unwrap();
            policy
                .set_mode("/p/ask", super::super::policy::ProjectMode::NotifyOnly, at)
                .unwrap();
            policy
                .set_mode("/p/never", super::super::policy::ProjectMode::Ignore, at)
                .unwrap();
        }
        assert_eq!(
            never_project_keys(&s),
            Some(NeverFolders::These(vec!["/p/never".to_string()]))
        );
        s.policy
            .lock()
            .unwrap()
            .set_contribution_override(super::super::policy::ProjectMode::Ignore, at, None)
            .unwrap();
        assert_eq!(never_project_keys(&s), Some(NeverFolders::Every));
        s.policy.lock().unwrap().clear_contribution_override();
        assert_eq!(
            never_project_keys(&s),
            Some(NeverFolders::These(vec!["/p/never".to_string()]))
        );
    }

    #[test]
    fn reads_per_poll_are_budgeted_and_the_rest_wait() {
        const { assert!(COUNTER_PASS_MAX_READS_PER_TICK < 64) };
        let f = Fixture::new();
        let extra = 3;
        let candidates: Vec<CounterCandidate> = (0..COUNTER_PASS_MAX_READS_PER_TICK + extra)
            .map(|i| {
                let path = f.write(&format!("c{i}.jsonl"), &codex_bytes());
                let mut c = candidate(SOURCE_CODEX, &path);
                // Newest first: the oldest wait.
                c.modified_at = written() - Duration::minutes(i as i64);
                c
            })
            .collect();
        let first = f.run(&candidates);
        assert_eq!(first.read, COUNTER_PASS_MAX_READS_PER_TICK);
        assert_eq!(first.deferred, extra);
        let second = f.run(&candidates);
        assert_eq!(second.read, extra);
        assert_eq!(second.deferred, 0);
    }

    #[test]
    fn an_unreadable_or_oversized_session_is_unknown_never_zero() {
        let f = Fixture::new();
        let missing = candidate(SOURCE_CLAUDE_CODE, &f.dir.path().join("gone.jsonl"));
        // Sparse: one line one byte over the line bound, without writing it.
        let big = f.dir.path().join("big.jsonl");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(COUNTER_PASS_MAX_LINE_BYTES + 1)
            .unwrap();
        let huge = candidate(SOURCE_CODEX, &big);
        assert_eq!(f.run(&[missing, huge]).read, 2);
        let value = f.week(&[]);
        assert_eq!(value["rollup"]["coverage"]["unknown"], 2);
        for line in value["rollup"]["sources"].as_array().unwrap() {
            assert_eq!(line["tokens"], serde_json::Value::Null, "{line}");
        }
    }

    /// A Claude Code session over 17 MB: `turns` user and assistant record
    /// pairs, each user record padded to `pad` bytes, Monday 2026-09-14
    /// from 09:00 UTC. Every assistant record has its own message ID.
    fn large_claude_bytes(turns: usize, pad: usize) -> Vec<u8> {
        let padding = "p".repeat(pad);
        let mut out = Vec::new();
        for i in 0..turns {
            let at = |s: usize| {
                (Utc.with_ymd_and_hms(2026, 9, 14, 9, 0, 0).unwrap()
                    + Duration::seconds((2 * i + s) as i64))
                .to_rfc3339()
            };
            let user = serde_json::json!({
                "type": "user", "sessionId": "PRIVATE-LARGE-SESSION",
                "timestamp": at(0),
                "message": {"role": "user", "content": padding},
            });
            let assistant = serde_json::json!({
                "type": "assistant", "sessionId": "PRIVATE-LARGE-SESSION",
                "timestamp": at(1),
                "message": {
                    "id": format!("msg_PRIVATE_{i}"), "model": "claude-fixture-model",
                    "role": "assistant", "content": [],
                    "usage": {"input_tokens": 3, "output_tokens": 10,
                        "cache_read_input_tokens": 0, "cache_creation_input_tokens": 0},
                },
            });
            for record in [user, assistant] {
                out.extend_from_slice(record.to_string().as_bytes());
                out.push(b'\n');
            }
        }
        out
    }

    #[test]
    fn a_session_over_17_mb_is_counted_with_a_date() {
        let f = Fixture::new();
        let bytes = large_claude_bytes(1_100, 16 * 1024);
        assert!(bytes.len() > 17 * 1024 * 1024, "{}", bytes.len());
        let path = f.write("large.jsonl", &bytes);
        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        let value = f.week(&[]);
        let coverage = &value["rollup"]["coverage"];
        assert_eq!(coverage["unknown"], 0, "{coverage}");
        assert_eq!(
            coverage["known"].as_u64().unwrap() + coverage["partial"].as_u64().unwrap(),
            1,
            "{coverage}"
        );
        assert_ne!(claude_line(&value)["tokens"], serde_json::Value::Null);
        let listed = f.pass.sessions_value(true, &NeverFolders::NONE, &[], 10);
        assert_eq!(
            listed["sessions"][0]["started_at"], "2026-09-14T09:00:00+00:00",
            "{listed}"
        );
        // Counted, but more than the whole-file analysis reads.
        assert_eq!(listed["sessions"][0]["analyzable"], false);
        let tag = f.pass.store_tag();
        let row = only_row(&f.pass);
        assert_eq!(
            f.pass.locate(
                &row.listed_reference(tag),
                &NeverFolders::NONE,
                &[(SOURCE_CLAUDE_CODE, path.as_path())]
            ),
            Err(LocateError::NotAnalyzable(NotAnalyzable::TooLarge))
        );
    }

    /// The whole-file analysis takes a Codex file with exactly one
    /// `session_meta`; the pass counts any, and says which it cannot
    /// analyze.
    #[test]
    fn a_codex_file_without_exactly_one_session_is_not_analyzable() {
        for (metas, analyzable) in [(0, false), (1, true), (2, false)] {
            let f = Fixture::new();
            let bytes = codex_bytes_from(&vec![None; metas]);
            let path = f.write("c.jsonl", &bytes);
            f.run(&[candidate(SOURCE_CODEX, &path)]);
            let listed = f.pass.sessions_value(true, &NeverFolders::NONE, &[], 10);
            assert_eq!(
                listed["sessions"][0]["analyzable"], analyzable,
                "{metas}: {listed}"
            );
            let row = only_row(&f.pass);
            let located = f.pass.locate(
                &row.listed_reference(f.pass.store_tag()),
                &NeverFolders::NONE,
                &[(SOURCE_CODEX, path.as_path())],
            );
            if analyzable {
                assert_eq!(located.unwrap().unwrap().index, 0);
            } else {
                assert_eq!(
                    located,
                    Err(LocateError::NotAnalyzable(NotAnalyzable::NotOneSession))
                );
            }
        }
        // A row made before this was looked for is offered, and refused,
        // if at all, when Analyze & save runs.
        let f = Fixture::new();
        let path = f.write("c.jsonl", &codex_bytes_from(&[None, None]));
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        edit_stored_rows(&f, |row| {
            assert_eq!(row["not_analyzable"], "not_one_session");
            row.remove("not_analyzable");
        });
        let pass = f.reopened();
        let f = Fixture { pass, ..f };
        let listed = f.pass.sessions_value(true, &NeverFolders::NONE, &[], 10);
        assert_eq!(listed["sessions"][0]["analyzable"], true);
    }

    /// `CODEX_PER_TURN_SERIES` (owner decision D11) held: the pass stores
    /// no Codex series, and a series stored while it held is not served, so
    /// every view reads the session as one saved before the per-turn
    /// reader. Its observed change and facts are kept.
    #[test]
    fn a_held_codex_per_turn_series_is_neither_read_nor_served() {
        let key = key_for(3);
        let bytes = codex_bytes();
        let scan = |per_turn: bool| {
            scan_session(
                AnalyticsSource::Codex,
                &bytes[..],
                &key,
                COUNTER_PASS_MAX_LINE_BYTES,
                per_turn,
            )
            .unwrap()
        };
        let ScannedBody::Codex(observed, Some(_)) = scan(true).body else {
            panic!("a series is read while D11 holds");
        };
        assert!(observed.is_some());
        let ScannedBody::Codex(held_observed, None) = scan(false).body else {
            panic!("no series is read while D11 is held");
        };
        assert_eq!(held_observed, observed);

        let f = Fixture::new();
        let path = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let row = &stored_rows(&f.pass)[0];
        let series = |input: SessionInput| match input.body {
            SessionBody::Codex { series, observed } => {
                assert!(observed.is_some());
                series.is_some()
            }
            _ => panic!("a Codex row"),
        };
        assert!(series(row.session_input_with(true)));
        assert!(!series(row.session_input_with(false)));
        assert!(row.listed_series(true).is_some());
        assert!(row.listed_series(false).is_none());
    }

    #[test]
    fn one_line_is_never_read_past_the_line_bound() {
        let key = key_for(3);
        let scan = |bytes: &[u8], max: u64| {
            scan_session(AnalyticsSource::ClaudeCode, bytes, &key, max, true)
                .expect("a long line is passed over, never refuses the file")
        };
        let line =
            br#"{"type":"user","sessionId":"PRIVATE-ID","timestamp":"2026-09-14T09:00:00Z"}"#;
        let len = line.len() as u64;
        // The bound is on the line, not its ending.
        for ending in [&b""[..], b"\n", b"\r\n"] {
            let bytes = [&line[..], ending].concat();
            let skipped = scan(&bytes, len - 1);
            assert_eq!(skipped.harness_session_id, None, "{ending:?}");
            assert_eq!(skipped.long_lines_skipped, 1, "{ending:?}");
            let read = scan(&bytes, len);
            assert_eq!(read.harness_session_id.as_deref(), Some("PRIVATE-ID"));
            assert_eq!(read.long_lines_skipped, 0);
        }
        // A long line anywhere is passed over alone, never parsed: one that
        // is not JSON would otherwise end the strict scans. The lines
        // around it are read.
        let mut three = [&line[..], b"\n"].concat();
        three.extend_from_slice(&vec![b'x'; line.len() + 1]);
        three.extend_from_slice(b"\n");
        three.extend_from_slice(line);
        three.extend_from_slice(b"\n");
        let read = scan(&three, len);
        assert_eq!(read.harness_session_id.as_deref(), Some("PRIVATE-ID"));
        assert_eq!(read.long_lines_skipped, 1);
        assert_eq!(scan(&three, len + 1).long_lines_skipped, 0);
        // The digest still covers every byte, the passed-over line too.
        let whole = |bytes: &[u8]| {
            use sha2::Digest as _;
            format!("{:x}", sha2::Sha256::digest(bytes))
        };
        let base = claude_bytes();
        let longest = base.split(|b| *b == b'\n').map(<[u8]>::len).max().unwrap() as u64;
        let first_end = base.iter().position(|b| *b == b'\n').unwrap() + 1;
        let mut long = base[..first_end].to_vec();
        long.extend_from_slice(&vec![b'x'; longest as usize + 1]);
        long.extend_from_slice(b"\n");
        long.extend_from_slice(&base[first_end..]);
        let (ScannedBody::Claude(Some(plain)), ScannedBody::Claude(Some(skipped))) =
            (scan(&base, longest).body, scan(&long, longest).body)
        else {
            panic!("the counters survive a long line");
        };
        assert_eq!(skipped.series, plain.series);
        assert_eq!(skipped.model_labels, plain.model_labels);
        assert_eq!(skipped.source_digest, whole(&long));
    }

    /// A line over `COUNTER_PASS_MAX_LINE_BYTES` (an inlined image, say) is
    /// passed over: the session keeps its counters and is marked partial,
    /// never stored as unknown and never as complete.
    #[test]
    fn a_session_with_a_long_line_is_counted_and_marked_partial() {
        let base = facts_fixture("finished");
        let first_end = base.iter().position(|b| *b == b'\n').unwrap() + 1;
        let mut bytes = base[..first_end].to_vec();
        bytes.extend_from_slice(&vec![b'x'; COUNTER_PASS_MAX_LINE_BYTES as usize + 1]);
        bytes.extend_from_slice(b"\n");
        bytes.extend_from_slice(&base[first_end..]);
        let f = Fixture::new();
        let path = f.write("image.jsonl", &bytes);
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let row = only_row(&f.pass);
        assert!(
            matches!(&row.body, RowBody::Claude { series, .. } if series.truncated),
            "counted, and partial: {:?}",
            row.body
        );
        let facts = row
            .facts
            .as_ref()
            .expect("facts are kept beside the counters");
        assert!(facts.capped && !facts.delegated_unread, "{facts:?}");
    }

    /// The whole-file read the counter pass made before it streamed, over
    /// the extractors' own `&[u8]` entry points: the oracle the line-by-line
    /// read must match for every file those accept.
    fn read_whole(
        source: AnalyticsSource,
        bytes: &[u8],
        group_member_count: u32,
        key: &DigestKey,
        harness: &str,
    ) -> ReadBody {
        use crate::insights::session_identity::harness_session_id;
        use crate::insights::usage_evidence::extract_codex_usage_evidence;
        let unknown = RowBody::Unknown {
            source,
            reason: UnknownReason::NoUsageCounters,
        };
        let body = match source {
            AnalyticsSource::ClaudeCode => {
                match crate::insights::turn_series::extract_claude_turn_series(bytes, key) {
                    Ok(evidence) => {
                        let mut series = evidence.series;
                        if group_member_count > 0 {
                            series.truncated = true;
                        }
                        RowBody::Claude {
                            model_labels: evidence.model_labels,
                            series,
                        }
                    }
                    Err(_) => unknown,
                }
            }
            AnalyticsSource::Codex => match extract_codex_usage_evidence(bytes) {
                Ok(evidence) => RowBody::Codex {
                    observed: codex_observed(&evidence),
                    series: None,
                    series_trimmed: false,
                },
                Err(_) => unknown,
            },
        };
        let first_event_at = bytes.split(|byte| *byte == b'\n').find_map(|line| {
            let record: serde_json::Value = serde_json::from_slice(line).ok()?;
            record_timestamp(&record)
        });
        let span = crate::insights::time_evidence::extract_recorded_time_evidence(
            source_format(source),
            bytes,
        )
        .ok()
        .and_then(|evidence| Some((evidence.earliest?.recorded_at, evidence.latest?.recorded_at)));
        ReadBody {
            body,
            harness_session: harness_session_id(source_format(source), bytes)
                .map(|id| harness_session_digest(key, harness, &id)),
            first_event_at,
            span,
            facts: None,
            automation: Automation::Unknown,
            goals: Vec::new(),
            not_analyzable: NotAnalyzable::of(
                source,
                false,
                Some(bytes.len() as u64),
                Some(
                    bytes
                        .split(|byte| *byte == b'\n')
                        .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
                        .filter(|record| record["type"] == "session_meta")
                        .count(),
                ),
            ),
            compressed_failure: None,
        }
    }

    /// `read` without what the whole-file read never made: the facts,
    /// Codex's per-turn series, which the facts tests cover, and the
    /// automation class, which the automated-session tests cover.
    fn without_facts(mut read: ReadBody) -> ReadBody {
        read.facts = None;
        read.goals = Vec::new();
        read.automation = Automation::Unknown;
        if let RowBody::Codex { series, .. } = &mut read.body {
            *series = None;
        }
        read
    }

    fn fixture(relative: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures/insights")
                .join(relative),
        )
        .unwrap()
    }

    #[test]
    fn the_streamed_read_matches_the_whole_file_read() {
        let f = Fixture::new();
        let key = key_for(3);
        let crlf = |bytes: Vec<u8>| {
            String::from_utf8(bytes)
                .unwrap()
                .replace('\n', "\r\n")
                .into_bytes()
        };
        let with_blank_lines = |bytes: Vec<u8>| {
            let text = String::from_utf8(bytes).unwrap();
            let mut lines: Vec<&str> = text.lines().collect();
            lines.insert(1, "");
            lines.insert(3, "   ");
            format!("\n{}\n\n", lines.join("\n")).into_bytes()
        };
        let mut not_json = claude_bytes();
        not_json.extend_from_slice(b"not json\n");
        let mut not_utf8 = claude_bytes();
        not_utf8.extend_from_slice(b"{\"type\":\"user\",\"x\":\"\xff\"}\n");
        let mut two_ids = claude_bytes();
        two_ids.extend_from_slice(b"{\"type\":\"user\",\"sessionId\":\"ANOTHER-ID\"}\n");
        let mut no_trailing_newline = claude_bytes();
        while no_trailing_newline.last() == Some(&b'\n') {
            no_trailing_newline.pop();
        }
        let mut codex_not_json = codex_bytes();
        codex_not_json.extend_from_slice(b"\n{oops\n");
        let cases: Vec<(&str, AnalyticsSource, Vec<u8>)> = vec![
            ("claude", AnalyticsSource::ClaudeCode, claude_bytes()),
            (
                "claude crlf",
                AnalyticsSource::ClaudeCode,
                crlf(claude_bytes()),
            ),
            (
                "claude blank",
                AnalyticsSource::ClaudeCode,
                with_blank_lines(claude_bytes()),
            ),
            (
                "claude no newline",
                AnalyticsSource::ClaudeCode,
                no_trailing_newline,
            ),
            ("claude not json", AnalyticsSource::ClaudeCode, not_json),
            ("claude not utf8", AnalyticsSource::ClaudeCode, not_utf8),
            ("claude two ids", AnalyticsSource::ClaudeCode, two_ids),
            ("claude empty", AnalyticsSource::ClaudeCode, Vec::new()),
            (
                "claude agent",
                AnalyticsSource::ClaudeCode,
                fixture("claude-task-attribution/agent-alpha.jsonl"),
            ),
            (
                "claude large",
                AnalyticsSource::ClaudeCode,
                large_claude_bytes(40, 64),
            ),
            ("codex", AnalyticsSource::Codex, codex_bytes()),
            ("codex crlf", AnalyticsSource::Codex, crlf(codex_bytes())),
            (
                "codex blank",
                AnalyticsSource::Codex,
                with_blank_lines(codex_bytes()),
            ),
            ("codex not json", AnalyticsSource::Codex, codex_not_json),
            ("codex as claude", AnalyticsSource::Codex, claude_bytes()),
            (
                "codex release",
                AnalyticsSource::Codex,
                fixture("codex-task-attribution/codex-release-0.154.0-tool-reasoning.jsonl"),
            ),
            (
                "codex alpha",
                AnalyticsSource::Codex,
                fixture("codex-task-attribution/codex-alpha-direct.jsonl"),
            ),
        ];
        let mut known = 0;
        for (name, source, bytes) in cases {
            let adapter = match source {
                AnalyticsSource::ClaudeCode => SOURCE_CLAUDE_CODE,
                AnalyticsSource::Codex => SOURCE_CODEX,
            };
            let path = f.write(&format!("{}.jsonl", name.replace(' ', "-")), &bytes);
            for members in [0, 2] {
                let mut c = candidate(adapter, &path);
                c.group_member_count = members;
                let streamed = read_body(source, &c, &key);
                assert_eq!(
                    without_facts(streamed.clone()),
                    read_whole(source, &bytes, members, &key, adapter),
                    "{name}"
                );
                known += usize::from(!matches!(streamed.body, RowBody::Unknown { .. }));
            }
        }
        // The cases exercise both outcomes, not only unknown.
        assert!(known >= 16, "{known}");
    }

    /// A Claude Code file just over the per-poll byte budget: blank lines,
    /// which every extractor passes over.
    fn over_the_budget(f: &Fixture, name: &str) -> PathBuf {
        let line = [vec![b' '; 1024 * 1024 - 1], vec![b'\n']].concat();
        let lines = (COUNTER_PASS_MAX_BYTES_PER_TICK / (1024 * 1024) + 1) as usize;
        let path = f.write(name, &line.repeat(lines));
        assert!(std::fs::metadata(&path).unwrap().len() > COUNTER_PASS_MAX_BYTES_PER_TICK);
        path
    }

    #[test]
    fn a_file_over_the_byte_budget_is_read_alone_and_never_starved() {
        let f = Fixture::new();
        let big = over_the_budget(&f, "big.jsonl");
        let small = f.write("small.jsonl", &claude_bytes());
        // Newest first: the big file is the poll's first read, and alone.
        let mut newest = candidate(SOURCE_CLAUDE_CODE, &big);
        newest.modified_at = written();
        let mut older = candidate(SOURCE_CLAUDE_CODE, &small);
        older.modified_at = written() - Duration::minutes(1);
        let first = f.run(&[newest.clone(), older.clone()]);
        assert_eq!((first.read, first.deferred), (1, 1));
        let second = f.run(&[newest, older]);
        assert_eq!((second.read, second.deferred), (1, 0));

        // Behind a smaller file it waits one poll, then is the first read.
        let f = Fixture::new();
        let big = over_the_budget(&f, "big.jsonl");
        let small = f.write("small.jsonl", &claude_bytes());
        let mut older = candidate(SOURCE_CLAUDE_CODE, &big);
        older.modified_at = written() - Duration::minutes(1);
        let newest = candidate(SOURCE_CLAUDE_CODE, &small);
        let first = f.run(&[newest.clone(), older.clone()]);
        assert_eq!((first.read, first.deferred), (1, 1));
        let second = f.run(&[newest, older]);
        assert_eq!((second.read, second.deferred), (1, 0));
    }

    /// Rewrite every stored row as the whole-file read left a file it
    /// skipped for its size: unknown, no usage counters, undated, and
    /// without the `streamed` mark.
    fn as_skipped_by_the_whole_file_read(f: &Fixture) {
        let mut value: serde_json::Value = serde_json::from_str(&f.rows_text()).unwrap();
        for row in value["rows"].as_object_mut().unwrap().values_mut() {
            let row = row.as_object_mut().unwrap();
            assert_eq!(row.remove("streamed"), Some(serde_json::json!(true)));
            row.insert(
                "body".into(),
                serde_json::json!({"kind": "unknown", "source": "claude_code", "reason": "no_usage_counters"}),
            );
            for field in [
                "first_event_at",
                "earliest_event_at",
                "latest_event_at",
                "harness_session",
            ] {
                row.insert(field.into(), serde_json::Value::Null);
            }
        }
        f.write(COUNTER_ROWS_FILE, value.to_string().as_bytes());
    }

    #[test]
    fn a_large_session_stored_unknown_before_streaming_is_read_once_more() {
        let f = Fixture::new();
        let path = f.write("large.jsonl", &large_claude_bytes(1_100, 16 * 1024));
        let session = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(std::slice::from_ref(&session)).read, 1);
        as_skipped_by_the_whole_file_read(&f);
        let restarted = f.reopened();
        let before = restarted.sessions_value(true, &NeverFolders::NONE, &[], 10);
        assert_eq!(before["sessions"][0]["started_at"], serde_json::Value::Null);

        // The file is unchanged, and still read again, once.
        assert_eq!(
            run_with(&restarted, std::slice::from_ref(&session), &[], false).read,
            1
        );
        let after = restarted.sessions_value(true, &NeverFolders::NONE, &[], 10);
        assert_eq!(
            after["sessions"][0]["started_at"], "2026-09-14T09:00:00+00:00",
            "{after}"
        );
        assert_ne!(after["sessions"][0]["turns"], serde_json::Value::Null);
        assert_eq!(
            run_with(&restarted, std::slice::from_ref(&session), &[], false).read,
            0,
            "not read a third time"
        );
    }

    #[test]
    fn a_session_still_unknown_when_streamed_is_not_read_on_every_poll() {
        let f = Fixture::new();
        let mut bytes = claude_bytes();
        bytes.extend_from_slice(b"not json\n");
        let path = f.write("bad.jsonl", &bytes);
        let session = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(std::slice::from_ref(&session)).read, 1);
        assert_eq!(
            f.run(std::slice::from_ref(&session)).read,
            0,
            "a streamed unknown row is current"
        );
        as_skipped_by_the_whole_file_read(&f);
        let restarted = f.reopened();
        assert_eq!(
            run_with(&restarted, std::slice::from_ref(&session), &[], false).read,
            1,
            "read once more"
        );
        assert!(stored_rows(&restarted).iter().all(|row| row.streamed
            && matches!(
                row.body,
                RowBody::Unknown {
                    reason: UnknownReason::NoUsageCounters,
                    ..
                }
            )));
        assert_eq!(
            run_with(&restarted, std::slice::from_ref(&session), &[], false).read,
            0,
            "and only once"
        );
    }

    #[test]
    fn delegated_transcripts_not_read_make_the_session_partial() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let mut group = candidate(SOURCE_CLAUDE_CODE, &path);
        group.group_member_count = 2;
        f.run(&[group]);
        let value = f.week(&[]);
        assert_eq!(value["rollup"]["coverage"]["reasons"]["truncated"], 1);
        assert_eq!(value["rollup"]["coverage"]["known"], 0);
    }

    #[test]
    fn a_small_session_with_large_delegated_transcripts_is_partial_not_unknown() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let mut group = candidate(SOURCE_CLAUDE_CODE, &path);
        group.group_member_count = 2;
        // The group is over the old whole-file bound; the file read is not.
        group.size_bytes = 16 * 1024 * 1024 + 1;
        f.run(&[group]);
        let value = f.week(&[]);
        assert_eq!(value["rollup"]["coverage"]["unknown"], 0);
        assert_eq!(value["rollup"]["coverage"]["reasons"]["truncated"], 1);
    }

    #[test]
    fn the_byte_budget_is_charged_by_the_file_read_not_the_group() {
        const SESSIONS: usize = 5;
        const { assert!(COUNTER_PASS_MAX_READS_PER_TICK >= SESSIONS) };
        let f = Fixture::new();
        let candidates: Vec<CounterCandidate> = (0..SESSIONS)
            .map(|i| {
                let path = f.write(&format!("s{i}.jsonl"), &claude_bytes());
                let mut c = candidate(SOURCE_CLAUDE_CODE, &path);
                c.group_member_count = 2;
                c.size_bytes = COUNTER_PASS_MAX_BYTES_PER_TICK;
                c.modified_at = written() - Duration::minutes(i as i64);
                c
            })
            .collect();
        let summary = f.run(&candidates);
        assert_eq!(summary.read, SESSIONS);
        assert_eq!(summary.deferred, 0);
    }

    #[test]
    fn codex_counts_its_observed_change_only() {
        let f = Fixture::new();
        let path = f.write("c.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let value = f.week(&[]);
        let line = value["rollup"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["source"] == "codex")
            .cloned()
            .unwrap();
        assert_eq!(line["tokens"], 80 + 30);
        assert_eq!(line["cache_share"]["numerator"], 40);
        assert_eq!(line["cache_share"]["denominator"], 80);
        assert_eq!(
            value["rollup"]["coverage"]["reasons"]["codex_baseline_excluded"],
            1
        );
    }

    #[test]
    fn the_oldest_sessions_go_first_past_a_cap() {
        let mut store = CounterStore::empty(KeyedDigest([0; 32]));
        for i in 0..5u8 {
            store.rows.insert(
                KeyedDigest([i; 32]),
                CounterRow {
                    size_bytes: 1,
                    modified_at: written() + Duration::minutes(i64::from(i)),
                    placed_at: None,
                    first_event_at: None,
                    project: KeyedDigest([9; 32]),
                    seq: u64::from(i),
                    body: RowBody::Codex {
                        observed: None,
                        series: None,
                        series_trimmed: false,
                    },
                    harness_session: None,
                    harness_session_read: true,
                    earliest_event_at: None,
                    latest_event_at: None,
                    event_span_read: true,
                    streamed: true,
                    facts: None,
                    facts_version: SESSION_FACTS_VERSION,
                    automation: Automation::Unknown,
                    automation_read: true,
                    not_analyzable: None,
                    goals: Vec::new(),
                },
            );
        }
        enforce_caps(&mut store, 3, usize::MAX, usize::MAX, usize::MAX, true);
        let kept: Vec<u8> = store.rows.keys().map(|k| k.0[0]).collect();
        assert_eq!(kept, vec![2, 3, 4]);
        let evicted: Vec<u8> = store.evicted.keys().map(|k| k.0[0]).collect();
        assert_eq!(evicted, vec![0, 1]);

        // The eviction marks are capped by the same count, oldest first.
        enforce_caps(&mut store, 1, usize::MAX, usize::MAX, usize::MAX, true);
        let kept: Vec<u8> = store.rows.keys().map(|k| k.0[0]).collect();
        assert_eq!(kept, vec![4]);
        let evicted: Vec<u8> = store.evicted.keys().map(|k| k.0[0]).collect();
        assert_eq!(evicted, vec![3]);
    }

    #[test]
    fn the_turn_cap_evicts_whole_sessions() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let mut store = (*f.pass.load_cached().unwrap().unwrap()).clone();
        let turns = store.rows.values().map(CounterRow::turns).sum::<usize>();
        assert!(turns > 1);
        enforce_caps(
            &mut store,
            usize::MAX,
            turns - 1,
            usize::MAX,
            usize::MAX,
            true,
        );
        assert!(store.rows.is_empty());
    }

    /// A store at its session cap would evict the oldest session it reads,
    /// so a file older than every row it keeps is not read at all: with no
    /// row it would look new, be the poll's first read, exempt from the
    /// byte budget, and be evicted again by the same pass, every poll. A
    /// file that changes is read again, and the row it displaces is marked.
    #[test]
    fn an_evicted_unchanged_session_is_not_read_again() {
        let f = Fixture::new();
        let key = f.pass.keys.load_or_create().unwrap();
        let mut store = CounterStore::empty(key_fingerprint(&key));
        for i in 0..COUNTER_STORE_MAX_SESSIONS {
            let mut digest = [0u8; 32];
            digest[..8].copy_from_slice(&(i as u64).to_be_bytes());
            store.rows.insert(
                KeyedDigest(digest),
                CounterRow {
                    size_bytes: 1,
                    modified_at: written() + Duration::seconds(1),
                    placed_at: None,
                    first_event_at: None,
                    project: KeyedDigest([9; 32]),
                    seq: i as u64,
                    body: RowBody::Codex {
                        observed: None,
                        series: None,
                        series_trimmed: false,
                    },
                    harness_session: None,
                    harness_session_read: true,
                    earliest_event_at: None,
                    latest_event_at: None,
                    event_span_read: true,
                    streamed: true,
                    facts: None,
                    facts_version: SESSION_FACTS_VERSION,
                    automation: Automation::Unknown,
                    automation_read: true,
                    not_analyzable: None,
                    goals: Vec::new(),
                },
            );
        }
        store.next_seq = COUNTER_STORE_MAX_SESSIONS as u64;
        f.pass.save(store).unwrap();

        let path = f.write("old.jsonl", &claude_bytes());
        let old = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(
            f.run(std::slice::from_ref(&old)).read,
            0,
            "older than every row a full store keeps: not read"
        );
        let stored = || {
            let reopened = f.reopened();
            let store = reopened.load_cached().unwrap().unwrap();
            assert_eq!(store.rows.len(), COUNTER_STORE_MAX_SESSIONS);
            assert!(store.rows.values().all(|row| row.modified_at > written()));
        };
        stored();

        assert_eq!(
            f.run(std::slice::from_ref(&old)).read,
            0,
            "unchanged: still not read"
        );
        let restarted = f
            .reopened()
            .run_pass(
                std::slice::from_ref(&old),
                &NeverFolders::NONE,
                now(),
                QUIET,
                &mut |_| (PROJECT.to_string(), false),
            )
            .unwrap();
        assert_eq!(restarted.read, 0, "nor after a restart");

        let mut changed = old.clone();
        changed.size_bytes += 1;
        changed.modified_at = written() + Duration::seconds(2);
        assert_eq!(f.run(&[changed]).read, 1, "a changed file is read again");
        stored();
        assert!(!f.pass.load_cached().unwrap().unwrap().evicted.is_empty());

        // The marks age out with the rows.
        let later = now() + Duration::weeks(COUNTER_STORE_KEEP_WEEKS + 1);
        f.pass
            .run_pass(&[], &NeverFolders::NONE, later, QUIET, &mut |_| {
                (PROJECT.to_string(), false)
            })
            .unwrap();
        let store = f.pass.load_cached().unwrap().unwrap();
        assert!(store.rows.is_empty());
        assert!(store.evicted.is_empty());
        assert!(!f.rows_text().contains("evicted"), "empty, left out");
    }

    /// A digest for a synthetic row or mark, distinct per `i` and `tag`.
    fn synthetic_digest(tag: u8, i: usize) -> KeyedDigest {
        let mut digest = [0u8; 32];
        digest[0] = tag;
        digest[1..9].copy_from_slice(&(i as u64).to_be_bytes());
        KeyedDigest(digest)
    }

    fn synthetic_row(modified_at: DateTime<Utc>, seq: u64, body: RowBody) -> CounterRow {
        CounterRow {
            size_bytes: 1,
            modified_at,
            placed_at: None,
            first_event_at: None,
            project: KeyedDigest([9; 32]),
            seq,
            body,
            harness_session: None,
            harness_session_read: true,
            earliest_event_at: None,
            latest_event_at: None,
            event_span_read: true,
            streamed: true,
            facts: None,
            facts_version: SESSION_FACTS_VERSION,
            automation: Automation::Unknown,
            automation_read: true,
            not_analyzable: None,
            goals: Vec::new(),
        }
    }

    /// `COUNTER_STORE_MAX_SESSIONS` eviction marks, all newer than
    /// `written()` and older than `rows_at`.
    fn full_marks(store: &mut CounterStore) {
        for i in 0..COUNTER_STORE_MAX_SESSIONS {
            store.evicted.insert(
                synthetic_digest(0xEE, i),
                Evicted {
                    size_bytes: 1,
                    modified_at: written() + Duration::seconds(1),
                    automated: Some(false),
                },
            );
        }
    }

    /// @poldsam's second review of #1499: a store holding a full set of
    /// rows and a full set of eviction marks, all newer than an old,
    /// unchanged file. Reading that file evicts it, and its new mark, the
    /// oldest, is the one the mark cap drops in the same pass. With no row
    /// and no mark it looked new on the next poll and was read again, every
    /// poll. It must be read at most once, and not again after a restart.
    #[test]
    fn an_old_session_past_full_marks_is_not_read_every_poll() {
        let f = Fixture::new();
        let key = f.pass.keys.load_or_create().unwrap();
        let mut store = CounterStore::empty(key_fingerprint(&key));
        for i in 0..COUNTER_STORE_MAX_SESSIONS {
            store.rows.insert(
                synthetic_digest(0x11, i),
                synthetic_row(
                    written() + Duration::seconds(2),
                    i as u64,
                    RowBody::Codex {
                        observed: None,
                        series: None,
                        series_trimmed: false,
                    },
                ),
            );
        }
        store.next_seq = COUNTER_STORE_MAX_SESSIONS as u64;
        full_marks(&mut store);
        f.pass.save(store).unwrap();

        let path = f.write("old.jsonl", &claude_bytes());
        let old = candidate(SOURCE_CLAUDE_CODE, &path);
        let first = f.run(std::slice::from_ref(&old)).read;
        assert!(first <= 1, "read at most once, got {first}");
        let second = f.run(std::slice::from_ref(&old)).read;
        let third = f.run(std::slice::from_ref(&old)).read;
        assert_eq!((second, third), (0, 0), "second and third polls");
        let restarted = f
            .reopened()
            .run_pass(
                std::slice::from_ref(&old),
                &NeverFolders::NONE,
                now(),
                QUIET,
                &mut |_| (PROJECT.to_string(), false),
            )
            .unwrap();
        assert_eq!(restarted.read, 0, "nor after a restart");
        let store = f.pass.load_cached().unwrap().unwrap();
        assert_eq!(store.rows.len(), COUNTER_STORE_MAX_SESSIONS);
        assert!(store.rows.values().all(|row| row.modified_at > written()));
    }

    /// The same loop with the turn cap binding instead of the session cap:
    /// a few rows, far under the session cap, holding all but a handful of
    /// the turn cap, with the marks full. The old file is read once,
    /// evicted, and never read again, also while newer sessions keep
    /// arriving and the caps keep evicting.
    #[test]
    fn an_old_session_past_full_marks_is_not_read_every_poll_under_the_turn_cap() {
        let f = Fixture::new();
        let path = f.write("old.jsonl", &claude_bytes());
        let old = candidate(SOURCE_CLAUDE_CODE, &path);
        // The old file's turns, from a throwaway pass over it.
        let (old_turns, mut body) = {
            let scratch = Fixture::new();
            let scratch_path = scratch.write("old.jsonl", &claude_bytes());
            scratch.run(&[candidate(SOURCE_CLAUDE_CODE, &scratch_path)]);
            let store = scratch.pass.load_cached().unwrap().unwrap();
            let row = store.rows.values().next().unwrap().clone();
            (row.turns(), row.body)
        };
        assert!(old_turns > 1);
        // One row holding the turn cap less one fewer than the old file's
        // turns, so the old file does not fit beside it.
        let RowBody::Claude { series, .. } = &mut body else {
            panic!("a Claude row");
        };
        series.tool_calls.clear();
        let one = series.turns[0].clone();
        series.turns = vec![one; COUNTER_STORE_MAX_TURNS - (old_turns - 1)];

        let key = f.pass.keys.load_or_create().unwrap();
        let mut store = CounterStore::empty(key_fingerprint(&key));
        store.rows.insert(
            synthetic_digest(0x11, 0),
            synthetic_row(written() + Duration::seconds(2), 0, body),
        );
        store.next_seq = 1;
        full_marks(&mut store);
        f.pass.save(store).unwrap();

        // The marks are newer than the old file, so it is below the newest
        // mark and is not read even once.
        let first = f.run(std::slice::from_ref(&old)).read;
        assert!(first <= 1, "read at most once, got {first}");
        assert_eq!(
            f.pass.load_cached().unwrap().unwrap().rows.len(),
            1,
            "not kept beside the full turn cap"
        );
        assert_eq!(f.run(std::slice::from_ref(&old)).read, 0, "second poll");
        assert_eq!(f.run(std::slice::from_ref(&old)).read, 0, "third poll");

        // Newer sessions arrive and the caps evict again; the old file is
        // still not read.
        let newer_path = f.write("newer.jsonl", &claude_bytes());
        let mut newer = candidate(SOURCE_CLAUDE_CODE, &newer_path);
        newer.modified_at = written() + Duration::seconds(3);
        assert_eq!(
            f.run(&[newer.clone(), old.clone()]).read,
            1,
            "only the newer"
        );
        assert_eq!(f.run(&[newer, old]).read, 0, "fourth poll");
    }

    #[test]
    fn sessions_older_than_the_kept_weeks_are_not_read_and_old_rows_drop() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let mut old = candidate(SOURCE_CLAUDE_CODE, &path);
        old.modified_at = now() - Duration::weeks(COUNTER_STORE_KEEP_WEEKS) - Duration::hours(1);
        assert_eq!(f.run(&[old]).read, 0);

        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        let later = now() + Duration::weeks(COUNTER_STORE_KEEP_WEEKS + 1);
        let summary = f
            .pass
            .run_pass(&[], &NeverFolders::NONE, later, QUIET, &mut |_| {
                (PROJECT.to_string(), false)
            })
            .unwrap();
        assert_eq!(summary.dropped, 1);
    }

    fn stored_rows(pass: &CounterPass) -> Vec<CounterRow> {
        pass.load_cached()
            .unwrap()
            .map(|store| store.rows.values().cloned().collect())
            .unwrap_or_default()
    }

    fn key_for(seed: u8) -> DigestKey {
        InMemoryDigestKeyStore::with_seed([seed; 32])
            .load_or_create()
            .unwrap()
    }

    #[test]
    fn each_row_stores_the_keyed_harness_session_digest() {
        let f = Fixture::new();
        let claude = f.write("c.jsonl", &claude_bytes());
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[
            candidate(SOURCE_CLAUDE_CODE, &claude),
            candidate(SOURCE_CODEX, &codex),
        ]);
        let key = key_for(3);
        let mut stored: Vec<KeyedDigest> = stored_rows(&f.pass)
            .iter()
            .map(|row| row.harness_session.expect("one session id"))
            .collect();
        stored.sort();
        let mut expected = vec![
            harness_session_digest(
                &key,
                SOURCE_CLAUDE_CODE,
                "11111111-2222-4333-8444-PRIVATESESS1",
            ),
            harness_session_digest(&key, SOURCE_CODEX, "PRIVATE-CODEX-ID"),
        ];
        expected.sort();
        assert_eq!(stored, expected);

        // The digest is the store's own; it never crosses.
        let wire = f.week(&[]).to_string();
        for digest in &expected {
            let hex = hex::encode(digest.0);
            assert!(!wire.contains(&hex), "digest on the wire: {wire}");
            assert!(!wire.contains(&hex[..16]), "digest prefix on the wire");
        }
    }

    #[test]
    fn an_ambiguous_or_unreadable_file_stores_no_session_digest() {
        let f = Fixture::new();
        let mut ambiguous = claude_bytes();
        ambiguous.extend_from_slice(b"{\"type\":\"user\",\"sessionId\":\"ANOTHER-ID\"}\n");
        let two = f.write("two.jsonl", &ambiguous);
        let none = f.write("none.jsonl", b"{\"type\":\"user\"}\n");
        let missing = f.dir.path().join("missing.jsonl");
        f.run(&[
            candidate(SOURCE_CLAUDE_CODE, &two),
            candidate(SOURCE_CLAUDE_CODE, &none),
            candidate(SOURCE_CODEX, &missing),
        ]);
        let rows = stored_rows(&f.pass);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.harness_session.is_none()));
    }

    /// Rewrite the store on disk as the v1 daemon wrote it: the v1 schema
    /// name, and no per-row session digest or its read marker.
    fn downgrade_to_v1(f: &Fixture) {
        let mut value: serde_json::Value = serde_json::from_str(&f.rows_text()).unwrap();
        value["schema"] = serde_json::json!("trace_commons.insights_counter_rows.v1");
        for row in value["rows"].as_object_mut().unwrap().values_mut() {
            let row = row.as_object_mut().unwrap();
            assert!(row.remove("harness_session").is_some());
            assert!(row.remove("harness_session_read").is_some());
        }
        f.write(COUNTER_ROWS_FILE, value.to_string().as_bytes());
    }

    fn stored_schema(f: &Fixture) -> String {
        serde_json::from_str::<serde_json::Value>(&f.rows_text()).unwrap()["schema"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn the_store_is_schema_v2() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        assert_eq!(stored_schema(&f), "trace_commons.insights_counter_rows.v2");
    }

    #[test]
    fn a_v1_store_loads_and_each_row_is_read_once_more() {
        let f = Fixture::new();
        let claude = f.write("c.jsonl", &claude_bytes());
        let codex = f.write("x.jsonl", &codex_bytes());
        let mut ambiguous = claude_bytes();
        ambiguous.extend_from_slice(b"{\"type\":\"user\",\"sessionId\":\"ANOTHER-ID\"}\n");
        let two = f.write("two.jsonl", &ambiguous);
        let all = [
            candidate(SOURCE_CLAUDE_CODE, &claude),
            candidate(SOURCE_CODEX, &codex),
            candidate(SOURCE_CLAUDE_CODE, &two),
        ];
        assert_eq!(f.run(&all).read, 3);
        downgrade_to_v1(&f);

        // A fresh pass over the same file and key, as after a restart. The
        // in-memory key store holds no key until asked to make one; a kept
        // key is what a restart finds.
        let restarted = f.rekeyed(3);
        restarted.keys.load_or_create().unwrap();
        let value = restarted.week_value(
            true,
            Some(monday()),
            utc(),
            now(),
            &NeverFolders::NONE,
            WeekOptions::default(),
        );
        assert_eq!(value["readable"], true, "a v1 store is readable");
        assert_eq!(value["sessions_stored"], 3, "its rows are kept");
        assert!(
            stored_rows(&restarted)
                .iter()
                .all(|row| row.harness_session.is_none())
        );

        assert_eq!(
            run_with(&restarted, &all, &[], false).read,
            3,
            "unchanged v1 rows are read once more"
        );
        assert_eq!(stored_schema(&f), "trace_commons.insights_counter_rows.v2");
        assert_eq!(
            stored_rows(&restarted)
                .iter()
                .filter(|row| row.harness_session.is_some())
                .count(),
            2,
            "the ambiguous file still has none"
        );
        assert_eq!(
            run_with(&restarted, &all, &[], false).read,
            0,
            "read once only, the ambiguous file included"
        );
    }

    #[test]
    fn a_v1_row_deferred_by_the_budget_is_still_read_once_more_later() {
        let f = Fixture::new();
        let all: Vec<CounterCandidate> = (0..COUNTER_PASS_MAX_READS_PER_TICK + 1)
            .map(|i| {
                let path = f.write(&format!("s{i}.jsonl"), &codex_bytes());
                let mut c = candidate(SOURCE_CODEX, &path);
                c.modified_at = written() - Duration::minutes(i as i64);
                c
            })
            .collect();
        let mut first = 0;
        while first < all.len() {
            first += f.run(&all).read;
        }
        downgrade_to_v1(&f);
        let restarted = f.rekeyed(3);
        let one = run_with(&restarted, &all, &[], false);
        assert_eq!(one.read, COUNTER_PASS_MAX_READS_PER_TICK);
        assert_eq!(one.deferred, 1);
        // Upgraded in place, not started again: the deferred row keeps its
        // v1 counters until its turn comes.
        let kept = stored_rows(&restarted);
        assert_eq!(kept.len(), all.len(), "no v1 row is dropped");
        assert_eq!(
            kept.iter()
                .filter(|row| row.harness_session.is_none())
                .count(),
            1
        );
        assert_eq!(run_with(&restarted, &all, &[], false).read, 1);
        assert_eq!(run_with(&restarted, &all, &[], false).read, 0);
        assert!(
            stored_rows(&restarted)
                .iter()
                .all(|row| row.harness_session.is_some())
        );
    }

    #[test]
    fn a_v1_store_with_nothing_to_read_is_rewritten_as_v2() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        downgrade_to_v1(&f);
        let restarted = f.rekeyed(3);
        assert_eq!(run_with(&restarted, &[], &[], false).read, 0);
        assert_eq!(stored_schema(&f), "trace_commons.insights_counter_rows.v2");
        let kept = stored_rows(&restarted);
        assert_eq!(kept.len(), 1, "the row is kept");
        assert!(
            kept[0].harness_session.is_none() && !kept[0].harness_session_read,
            "a session no longer found carries no digest and stays unread"
        );
    }

    #[test]
    fn an_unknown_store_schema_is_unreadable() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let text = f
            .rows_text()
            .replace("insights_counter_rows.v2", "insights_counter_rows.v9");
        f.write(COUNTER_ROWS_FILE, text.as_bytes());
        let restarted = f.rekeyed(3);
        let value = restarted.week_value(
            true,
            Some(monday()),
            utc(),
            now(),
            &NeverFolders::NONE,
            WeekOptions::default(),
        );
        assert_eq!(value["reason"], "store_unreadable");
    }

    #[test]
    fn a_new_key_starts_the_store_again() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let other = f.rekeyed(4);
        // Rows made under another key are not read as this key's.
        let value = other.week_value(
            true,
            Some(monday()),
            utc(),
            now(),
            &NeverFolders::NONE,
            WeekOptions::default(),
        );
        assert_eq!(value["sessions_stored"], 0);
        assert_eq!(
            run_with(&other, &[candidate(SOURCE_CLAUDE_CODE, &path)], &[], false).read,
            1
        );
        assert_eq!(
            other.week_value(
                true,
                Some(monday()),
                utc(),
                now(),
                &NeverFolders::NONE,
                WeekOptions::default()
            )["sessions_stored"],
            1
        );
    }

    #[test]
    fn a_corrupt_store_reads_as_unreadable_and_the_pass_starts_again() {
        let f = Fixture::new();
        f.write(COUNTER_ROWS_FILE, b"{not json");
        let value = f.week(&[]);
        assert_eq!(
            value,
            serde_json::json!({"enabled": true, "feed": "counter_pass", "readable": false, "reason": "store_unreadable"})
        );
        let path = f.write("s.jsonl", &claude_bytes());
        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        assert_eq!(f.week(&[])["readable"], true);
    }

    #[test]
    fn clear_removes_the_rows_and_forgets_the_key() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        assert!(f.pass.keys.load().unwrap().is_some());
        f.pass.clear().unwrap();
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
        assert!(f.pass.keys.load().unwrap().is_none());
        assert_eq!(f.week(&[])["sessions_stored"], 0);
        // Idempotent.
        f.pass.clear().unwrap();
    }

    #[test]
    fn clear_forgets_a_key_made_by_a_pass_that_wrote_no_rows() {
        let f = Fixture::new();
        f.run(&[]);
        assert!(f.pass.keys.load().unwrap().is_some(), "the pass made a key");
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
        f.pass.clear().unwrap();
        assert!(f.pass.keys.load().unwrap().is_none());
    }

    #[test]
    fn with_the_setting_off_nothing_is_read_or_created() {
        let f = Fixture::new();
        let value = f.pass.week_value(
            false,
            None,
            utc(),
            now(),
            &NeverFolders::NONE,
            WeekOptions::default(),
        );
        assert_eq!(
            value,
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        assert!(f.pass.keys.load().unwrap().is_none(), "no key made");
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
    }

    #[test]
    fn a_read_never_creates_a_key() {
        let f = Fixture::new();
        let value = f.week(&[]);
        assert_eq!(value["readable"], true);
        assert_eq!(value["sessions_stored"], 0);
        assert_eq!(value["updated_at"], serde_json::Value::Null);
        assert!(f.pass.keys.load().unwrap().is_none());
    }

    #[test]
    fn comparisons_need_comparable_weeks() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let value = f.week(&[]);
        assert_eq!(value["comparable"], false);
        assert_eq!(value["unavailable"], "below_coverage_floor");
        assert_eq!(
            value["change_vs_last_week"],
            serde_json::json!([{"source": "claude_code", "permille": null, "unavailable": "below_coverage_floor"}])
        );
    }

    #[test]
    fn the_week_carries_work_at_list_prices_once_there_are_rows() {
        let f = Fixture::new();
        assert_eq!(f.week(&[])["work"], serde_json::Value::Null);
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let value = f.week(&[]);
        let work = &value["work"];
        assert_eq!(work["basis"], "api_list");
        assert_eq!(work["week"]["sessions"], 1);
        assert_eq!(work["by_tool"][0]["key"], "claude_code");
        assert_eq!(work["by_day"].as_array().unwrap().len(), 7);
        assert_eq!(work["by_day"][0]["date"], "2026-09-14");
        assert!(work["last_week"].is_object());
        // Every row in a Never folder: no row, no figure.
        assert_eq!(
            f.week(&[PROJECT.to_string()])["work"],
            serde_json::Value::Null
        );
    }

    #[test]
    fn nothing_identifying_crosses() {
        let f = Fixture::new();
        let path = f.write("PRIVATE-SESSION.jsonl", &claude_bytes());
        // A Codex session whose ID the proxy ledger also records, with a
        // backend name and a price, folded so its tally crosses.
        let codex = f.write("PRIVATE-CODEX.jsonl", &codex_bytes_for("SESSION-SECRET"));
        f.run(&[
            candidate(SOURCE_CLAUDE_CODE, &path),
            candidate(SOURCE_CODEX, &codex),
        ]);
        let mut priced = ledger_call(1, 30, "SESSION-SECRET");
        priced.cost_usd = Some(4.25);
        assert_eq!(fold_with(&f.pass, true, vec![priced]).unwrap().folded, 1);
        let key = f.pass.keys.load().unwrap().unwrap();
        let session = hex::encode(session_key(&key, SOURCE_CLAUDE_CODE, &path.to_string_lossy()).0);
        let project = hex::encode(project_digest(&key, PROJECT).0);
        let harness = hex::encode(harness_session_digest(&key, SOURCE_CODEX, "SESSION-SECRET").0);
        let value = f.week_routed();
        // Not vacuous: the tally reached the answer.
        assert_eq!(value["routing_available"], true);
        assert!(
            sessions_of(&value)
                .iter()
                .any(|row| row["routing"]["category"] == "routed_verified"),
            "{value}"
        );
        let text = value.to_string();
        for forbidden in [
            "SESSION-SECRET",
            harness.as_str(),
            &harness[..16],
            "cost",
            "4.25",
            "SECRET-BACKEND",
            "openai",
            "/v1/responses",
            "last_call_at",
            codex.to_str().unwrap(),
            "PRIVATE",
            "/Users",
            "msg_key",
            session.as_str(),
            &session[..16],
            project.as_str(),
            &project[..16],
            path.to_str().unwrap(),
        ] {
            assert!(!text.contains(forbidden), "{forbidden} crossed: {text}");
        }
    }

    /// Product owner, 2026-10-28: the week answers where NEAR AI credit
    /// went, by task, tool and the local folder label, and today and the
    /// last 31 days as sums; a call with no charge makes every figure it
    /// is in unknown, never zero.
    #[test]
    fn the_week_answers_where_near_ai_credit_went() {
        let f = Fixture::new();
        let codex = f.write("codex.jsonl", &codex_bytes_for("SESSION-SECRET"));
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        let mut charged = ledger_call(1, 30, "SESSION-SECRET");
        charged.cost_usd = Some(0.25);
        assert_eq!(fold_with(&f.pass, true, vec![charged]).unwrap().folded, 1);
        let folders = [(PROJECT.to_string(), "app".to_string())];
        let week = |options| {
            f.pass.week_value_with_folders(
                true,
                Some(monday()),
                utc(),
                now(),
                &NeverFolders::NONE,
                (options, &folders),
            )
        };
        let value = week(routed());
        let credit = &value["near_ai_credit"];
        assert_eq!(credit["basis"], "near_ai_ledger");
        assert_eq!(credit["today"]["usd_micros"], 250_000);
        assert_eq!(credit["last_31_days"]["usd_micros"], 250_000);
        assert!(credit["today"].get("by_folder").is_none());
        let days = credit["by_day"].as_array().unwrap();
        assert_eq!(days.len(), 7);
        assert_eq!(days[6]["date"], "2026-09-14", "today last");
        assert_eq!(days[0]["date"], "2026-09-08");
        assert_eq!(days[6]["usd_micros"], 250_000);
        assert_eq!(
            days[5]["usd_micros"], 0,
            "a day with no call is a true zero"
        );
        let this = &credit["week"];
        assert_eq!(this["usd_micros"], 250_000);
        assert_eq!(this["sessions"], 1);
        assert!(this.get("by_tool").is_none(), "by tool was removed");
        assert!(this.get("by_task").is_none(), "by task was removed");
        assert_eq!(this["by_folder"][0]["key"], "app");
        assert_eq!(this["by_folder"][0]["usd_micros"], 250_000);

        // Another call with no charge: unknown, never zero.
        assert_eq!(
            fold_with(&f.pass, true, vec![ledger_call(2, 20, "SESSION-SECRET")])
                .unwrap()
                .folded,
            1
        );
        let value = week(routed());
        let this = &value["near_ai_credit"]["week"];
        assert_eq!(this["usd_micros"], serde_json::Value::Null);
        assert_eq!(this["calls_without_charge"], 1);
        assert_eq!(this["by_folder"][0]["usd_micros"], serde_json::Value::Null);
        assert_eq!(
            value["near_ai_credit"]["today"]["usd_micros"],
            serde_json::Value::Null
        );

        // The week before has none of it; a true zero.
        let before = f.pass.week_value_with_folders(
            true,
            Some(monday() - Duration::days(7)),
            utc(),
            now(),
            &NeverFolders::NONE,
            (routed(), &folders),
        );
        assert_eq!(before["near_ai_credit"]["week"]["usd_micros"], 0);
        assert_eq!(before["near_ai_credit"]["week"]["sessions"], 0);
        // Today does not move with the week on screen.
        assert_eq!(
            before["near_ai_credit"]["today"]["calls"],
            value["near_ai_credit"]["today"]["calls"]
        );

        // No money where routing is not shown.
        for options in [WeekOptions::default(), {
            let mut no_ledger = routed();
            no_ledger.ledger_present = false;
            no_ledger
        }] {
            assert_eq!(week(options)["near_ai_credit"], serde_json::Value::Null);
        }
    }

    /// A week row carries its Sessions row's short name; the Sessions
    /// methods take that name tagged with the store it was answered from.
    #[test]
    fn breakdown_rows_share_the_sessions_reference_and_allow_absent_folder() {
        let f = Fixture::new();
        let path = f.write("PRIVATE-SESSION.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let page = f.pass.sessions_value(true, &NeverFolders::NONE, &[], 20);
        let week = f.week_routed();
        let row = &sessions_of(&week)[0];
        assert_eq!(
            page["sessions"][0]["session_ref"],
            format!(
                "{}.{}",
                row["session_ref"].as_str().unwrap(),
                f.pass.store_tag()
            )
        );
        assert_eq!(row.get("folder"), Some(&serde_json::Value::Null));
    }

    #[test]
    fn breakdown_folder_labels_match_sessions_across_weeks_and_never_stays_hidden() {
        let f = Fixture::new();
        let path = f.write("PRIVATE-SESSION.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let folders = vec![(PROJECT.to_string(), "My project".to_string())];
        let page = f
            .pass
            .sessions_value(true, &NeverFolders::NONE, &folders, 20);
        for answer_now in [now(), now() + Duration::weeks(1)] {
            let week = f.pass.week_value_with_folders(
                true,
                Some(monday()),
                utc(),
                answer_now,
                &NeverFolders::NONE,
                (routed(), &folders),
            );
            let row = &sessions_of(&week)[0];
            assert_eq!(row["folder"], "My project");
            assert_eq!(row["folder"], page["sessions"][0]["folder"]);
            assert_eq!(
                page["sessions"][0]["session_ref"],
                format!(
                    "{}.{}",
                    row["session_ref"].as_str().unwrap(),
                    f.pass.store_tag()
                )
            );
            assert!(!week.to_string().contains(PROJECT));
        }
        let hidden = f.pass.week_value_with_folders(
            true,
            Some(monday()),
            utc(),
            now(),
            &NeverFolders::Every,
            (routed(), &folders),
        );
        assert!(sessions_of(&hidden).is_empty());
        assert!(!hidden.to_string().contains("My project"));
    }

    #[test]
    fn breakdown_last_event_is_recorded_activity_never_file_time() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let week = f.week_routed();
        assert_eq!(
            sessions_of(&week)[0]["last_event_at"],
            "2026-09-14T09:01:00+00:00"
        );
    }

    #[test]
    fn breakdown_rows_split_claude_by_day_and_leave_codex_null() {
        let f = Fixture::new();
        let claude = f.write("a.jsonl", &claude_bytes());
        let codex = f.write("c.jsonl", &codex_bytes());
        f.run(&[
            candidate(SOURCE_CLAUDE_CODE, &claude),
            candidate(SOURCE_CODEX, &codex),
        ]);
        let week = f.week(&[]);
        let days = week["rollup"]["by_day"].as_array().unwrap();
        for session in sessions_of(&week) {
            if session["source"] == "codex" {
                assert_eq!(session["by_day"], serde_json::Value::Null);
                continue;
            }
            let by_day = session["by_day"].as_array().unwrap();
            assert_eq!(by_day.len(), 7);
            for (amount, day) in by_day.iter().zip(days) {
                let total: u64 = ["uncached", "cache_read", "cache_write", "output"]
                    .iter()
                    .map(|series| day[series].as_u64().unwrap())
                    .sum();
                assert_eq!(amount.as_u64(), Some(total));
            }
        }
    }

    #[test]
    fn breakdown_metadata_is_null_without_a_join() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let (_, store) = f.pass.visible_store().unwrap();
        let inputs: Vec<_> = store
            .unwrap()
            .rows
            .values()
            .map(CounterRow::session_input)
            .collect();
        let rollup = week_rollup(Feed::CounterPass, &inputs, monday(), &utc());
        let value = rollup_value(&rollup, &BTreeMap::new(), false);
        assert_eq!(value["sessions"][0]["session_ref"], serde_json::Value::Null);
        assert_eq!(value["sessions"][0]["folder"], serde_json::Value::Null);
        assert_eq!(
            value["sessions"][0]["last_event_at"],
            serde_json::Value::Null
        );
    }

    /// Design section 6, part B item 5: each row carries its first-event
    /// time and its route tally, joined in the daemon by the keyed
    /// harness-session digest.
    #[test]
    fn each_row_carries_its_first_event_time_and_routing() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        let mut outside = ledger_call(2, 20, "PRIVATE-CODEX-ID");
        outside.proof = Some(crate::routing::ProofStatus::Outside);
        outside.output_tokens = None;
        fold_with(
            &f.pass,
            true,
            vec![ledger_call(1, 30, "PRIVATE-CODEX-ID"), outside],
        )
        .unwrap();
        let value = f.week_routed();
        assert_eq!(value["routing_available"], true);
        assert_eq!(value["routing_unavailable"], serde_json::Value::Null);
        let rows = sessions_of(&value);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0]["started_at"],
            (written() - Duration::hours(1)).to_rfc3339()
        );
        assert_eq!(
            rows[0]["routing"],
            serde_json::json!({
                "category": "mixed",
                "reasons": [],
                "tokens": {
                    "verified": 110,
                    "routed_unverified": 0,
                    "check_failed": 0,
                    "outside": null,
                    "unrecorded": 0,
                },
                "calls": 2,
                "calls_without_counts": 1,
            })
        );
        assert_eq!(rows[0]["session_ref"], "t0");
    }

    /// The shells' Sessions drill draws `rollup.sessions[]` in wire order and
    /// knows a row by its place, so the order is the order the counter pass
    /// made the rows in: never the store's, which is keyed by a digest of
    /// each file's path under the counter key and so arbitrary.
    #[test]
    fn session_rows_cross_in_the_order_the_pass_made_them() {
        let f = Fixture::new();
        // Eight Codex sessions, each 110 + i transcript tokens; the pass
        // reads the newest first, so session i is row i.
        let candidates: Vec<CounterCandidate> = (0..8u64)
            .map(|i| {
                let text = String::from_utf8(codex_bytes_for(&format!("ORDER-{i}"))).unwrap();
                let bytes = text
                    .replace(
                        "\"output_tokens\":50",
                        &format!("\"output_tokens\":{}", 50 + i),
                    )
                    .replace(
                        "\"total_tokens\":230",
                        &format!("\"total_tokens\":{}", 230 + i),
                    )
                    .into_bytes();
                let path = f.write(&format!("o{i}.jsonl"), &bytes);
                CounterCandidate {
                    modified_at: written() - Duration::minutes(i64::try_from(i).unwrap()),
                    ..candidate(SOURCE_CODEX, &path)
                }
            })
            .collect();
        f.run(&candidates);
        let tokens: Vec<u64> = sessions_of(&f.week_routed())
            .iter()
            .map(|row| row["tokens"].as_u64().unwrap_or_else(|| panic!("{row}")))
            .collect();
        assert_eq!(tokens, (110..118).collect::<Vec<u64>>());
    }

    /// A whole routed answer, printed between markers for shells' decode
    /// fixtures (`cargo test ... -- --nocapture`): one session with a mixed
    /// route and an uncounted call, one with no tally.
    #[test]
    fn a_routed_week_answer_for_shell_fixtures() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[
            candidate(SOURCE_CLAUDE_CODE, &path),
            candidate(SOURCE_CODEX, &codex),
        ]);
        let mut outside = ledger_call(2, 20, "PRIVATE-CODEX-ID");
        outside.proof = Some(crate::routing::ProofStatus::Outside);
        outside.output_tokens = None;
        // A charged call through NEAR AI, so `near_ai_credit` has a known
        // figure, by model too; the outside call adds no charge.
        let mut charged = ledger_call(1, 30, "PRIVATE-CODEX-ID");
        charged.cost_usd = Some(0.0125);
        charged.served_model = Some("zai-org/GLM-4.6".to_string());
        fold_with(&f.pass, true, vec![charged, outside]).unwrap();
        let value = f.week_routed();
        assert_eq!(
            value["near_ai_credit"]["week"]["by_model"][0]["key"],
            "zai-org/glm-4.6"
        );
        let mut categories: Vec<String> = sessions_of(&value)
            .iter()
            .map(|row| row["routing"]["category"].as_str().unwrap().to_string())
            .collect();
        categories.sort();
        assert_eq!(categories, ["mixed", "unobserved"]);
        // The shells decode this answer from the committed fixture, so the
        // fixture is this answer, rows in order: a change to the wire fails
        // here first.
        println!("BEGIN insights_week");
        println!("{}", serde_json::to_string_pretty(&value).unwrap());
        println!("END insights_week");
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/insights-analytics/insights_week_routed.json"
        ))
        .unwrap();
        assert_eq!(
            value, fixture,
            "re-record tests/fixtures/insights-analytics/insights_week_routed.json \
             from this test's --nocapture output"
        );
    }

    /// `started_at` is the transcript's first recorded event (owner question
    /// Q1), read from the file. Discovery's `started_at` is the file's last
    /// write, which for a live session is about now: it must never cross
    /// under that name.
    #[test]
    fn started_at_is_the_transcripts_first_event_never_the_files_write_time() {
        let f = Fixture::new();
        let claude = f.write("s.jsonl", &claude_bytes());
        let codex = f.write("x.jsonl", &codex_bytes());
        let undated = f.write(
            "u.jsonl",
            br#"{"type":"session_meta","payload":{"id":"U"}}"#,
        );
        let late = |source, path: &Path| CounterCandidate {
            started_at: Some(written()),
            ..candidate(source, path)
        };
        f.run(&[
            late(SOURCE_CLAUDE_CODE, &claude),
            late(SOURCE_CODEX, &codex),
            late(SOURCE_CODEX, &undated),
        ]);
        let mut started: Vec<serde_json::Value> = sessions_of(&f.week_routed())
            .iter()
            .map(|row| row["started_at"].clone())
            .collect();
        // A string sorts before null.
        started.sort_by_key(|at| at.to_string());
        assert_eq!(
            started,
            [
                // Claude's first line carries no timestamp; its second does.
                serde_json::json!("2026-09-14T09:00:00+00:00"),
                serde_json::json!("2026-09-14T09:00:00+00:00"),
                // No line carries one: unknown, never the write time.
                serde_json::Value::Null,
            ]
        );
    }

    #[test]
    fn a_row_with_no_tally_is_unobserved_never_outside() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[
            candidate(SOURCE_CLAUDE_CODE, &path),
            candidate(SOURCE_CODEX, &codex),
        ]);
        // A call for some other session: no row's tally.
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "ANOTHER-SESSION")]).unwrap();
        let value = f.week_routed();
        assert_eq!(value["routing_available"], true);
        let rows = sessions_of(&value);
        assert_eq!(rows.len(), 2);
        for row in rows {
            assert_eq!(
                row["routing"],
                serde_json::json!({
                    "category": "unobserved",
                    "reasons": [],
                    "tokens": null,
                    "calls": null,
                    "calls_without_counts": null,
                }),
                "{row}"
            );
        }
    }

    /// Owner decision D3: with the ledger feed turned off no row carries a
    /// route, even one folded while it was on; nor with no proxy ledger,
    /// where a stored tally may be missing every call since. The
    /// first-event time (owner question Q1) does not depend on either.
    #[test]
    fn with_the_feed_off_or_no_ledger_no_row_carries_a_route() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        for (feed, present, reason) in [
            (false, true, "ledger_feed_off"),
            (false, false, "ledger_feed_off"),
            (true, false, "no_ledger"),
        ] {
            let value = f.week_with(
                Some(monday()),
                &[],
                WeekOptions {
                    ledger_feed: feed,
                    ledger_present: present,
                    ..WeekOptions::default()
                },
            );
            assert_eq!(value["routing_available"], false, "{reason}");
            assert_eq!(value["routing_unavailable"], reason);
            let rows = sessions_of(&value);
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["routing"], serde_json::Value::Null, "{reason}");
            assert_eq!(
                rows[0]["started_at"],
                (written() - Duration::hours(1)).to_rfc3339()
            );
        }
        // The default is the feed's default, on (owner decision D3): with no
        // ledger declared the reason is the ledger, never the setting.
        assert_eq!(f.week(&[])["routing_unavailable"], "no_ledger");
    }

    /// The join happens at answer time and never in the engine, so the
    /// week's figures, coverage and overlap rule are exactly what they were
    /// without it: two rollouts recording one session ID stay two rows,
    /// each showing that session's tally.
    #[test]
    fn routing_leaves_the_weeks_figures_and_overlap_rule_unchanged() {
        let f = Fixture::new();
        let first = f.write("a.jsonl", &codex_bytes());
        let second = f.write("b.jsonl", &codex_bytes());
        f.run(&[
            candidate(SOURCE_CODEX, &first),
            candidate(SOURCE_CODEX, &second),
        ]);
        assert!(
            stored_rows(&f.pass)
                .iter()
                .all(|row| row.session_input().harness_session.is_none())
        );
        let figures = |value: &serde_json::Value| {
            let rows: Vec<serde_json::Value> = sessions_of(value)
                .into_iter()
                .map(|mut row| {
                    let object = row.as_object_mut().unwrap();
                    object.remove("routing");
                    object.remove("started_at");
                    row
                })
                .collect();
            (
                value["rollup"]["coverage"].clone(),
                value["rollup"]["sources"].clone(),
                rows,
            )
        };
        let before = figures(&f.week(&[]));
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        let after = f.week_routed();
        assert_eq!(figures(&after), before);
        assert_eq!(before.2.len(), 2);
        for row in sessions_of(&after) {
            assert_eq!(row["routing"]["category"], "routed_verified");
            assert_eq!(row["routing"]["calls"], 1);
        }
    }

    #[test]
    fn the_answer_carries_the_cores_overview_and_patterns_for_feed_t() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let value = f.week(&[]);
        let overview = &value["overview"];
        assert_eq!(overview["feed"], "counter_pass");
        assert_eq!(overview["week_start"], "2026-09-14");
        assert_eq!(overview["week_end"], "2026-09-20");
        assert_eq!(overview["sources"][0]["source"], "claude_code");
        assert_eq!(overview["sources"][0]["tokens"], CLAUDE_TOKENS);
        assert_eq!(overview["sources"][0]["change"], "below_coverage_floor");
        assert_eq!(overview["sources"][0]["best_week"], "below_coverage_floor");
        assert_eq!(overview["by_project"], "held_for_project_decision");
        assert_eq!(overview["weeks"], serde_json::json!(["2026-09-14"]));
        let patterns = &value["patterns"];
        assert_eq!(patterns["feed"], "counter_pass");
        assert_eq!(patterns["week_start"], "2026-09-14");
        assert_eq!(patterns["cards"].as_array().unwrap().len(), 4);
        assert_eq!(patterns["weeks"], serde_json::json!(["2026-09-14"]));
        assert_eq!(
            value["recap_card_enabled"],
            crate::insights::analytics_constants::RECAP_CARD_DEFAULT_ON
        );
    }

    #[test]
    fn history_runs_to_the_current_week_whatever_week_is_asked_for() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        for asked in [Some(monday()), Some(monday() - Duration::weeks(3)), None] {
            let value = f.week_with(asked, &[], WeekOptions::default());
            let history = value["history"].as_array().unwrap();
            assert_eq!(history.len(), COUNTER_STORE_KEEP_WEEKS as usize);
            assert_eq!(history.last().unwrap()["week_start"], "2026-09-14");
            assert_eq!(
                history.first().unwrap()["week_start"],
                (monday() - Duration::weeks(COUNTER_STORE_KEEP_WEEKS - 1)).to_string()
            );
            let this = history.last().unwrap();
            assert_eq!(this["tokens"]["claude_code"], CLAUDE_TOKENS);
            assert_eq!(this["sessions"], 1);
            assert_eq!(this["comparable"], false);
            assert_eq!(this["past_threshold"], serde_json::Value::Null);
            // An earlier week with no session has no figure, never zero.
            assert_eq!(history[0]["tokens"], serde_json::json!({}));
            assert_eq!(history[0]["sessions"], 0);
        }
    }

    #[test]
    fn the_threshold_count_and_the_summary_switch_follow_the_settings() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let value = f.week_with(
            Some(monday()),
            &[],
            WeekOptions {
                recap_card_enabled: false,
                context_threshold: Some(12_000),
                ..WeekOptions::default()
            },
        );
        assert_eq!(value["recap_card_enabled"], false);
        assert_eq!(
            value["history"].as_array().unwrap().last().unwrap()["past_threshold"],
            serde_json::json!({"threshold": 12_000, "sessions": 1})
        );
        let higher = f.week_with(
            Some(monday()),
            &[],
            WeekOptions {
                recap_card_enabled: true,
                context_threshold: Some(1_000_000),
                ..WeekOptions::default()
            },
        );
        assert_eq!(
            higher["history"].as_array().unwrap().last().unwrap()["past_threshold"]["sessions"],
            0
        );
    }

    #[test]
    fn a_never_folder_leaves_no_figure_in_the_overview_or_history() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let value = f.week(&[PROJECT.to_string()]);
        assert_eq!(value["overview"]["sources"], serde_json::json!([]));
        let history = value["history"].as_array().unwrap();
        assert!(history.iter().all(|week| week["sessions"] == 0));
    }

    #[test]
    fn the_week_is_a_strict_iso_week() {
        assert_eq!(parse_iso_week("2026-W38"), Some(monday()));
        assert_eq!(
            parse_iso_week("2026-W01"),
            NaiveDate::from_ymd_opt(2025, 12, 29)
        );
        for bad in [
            "2026-W54",
            "2026-W00",
            "2026-38",
            "2026-w38",
            "26-W38",
            "2026-W3",
            "2026-W038",
            " 2026-W38",
            "2026-W38 ",
        ] {
            assert_eq!(parse_iso_week(bad), None, "{bad}");
        }
    }

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn call(params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: "insights_week".to_string(),
            params,
        }
    }

    /// "Hide" on the weekly summary card sets `insights_recap_card_enabled`
    /// off; the Settings switch (`analytics_setting_recap_card`) sets it
    /// back on through the same `set_settings`, and `insights_week` and
    /// `get_settings` both follow.
    #[test]
    fn the_summary_card_switch_turns_back_on_after_hide() {
        let (_dir, s) = shared();
        s.settings.lock().unwrap().insights_counter_pass = true;
        let request = |method: &str, params: serde_json::Value| Request {
            id: 1,
            method: method.to_string(),
            params,
        };
        let state = |s: &DaemonShared| {
            let week = handle_week(s, &call(serde_json::json!({"tz": 0})))
                .result
                .unwrap();
            let settings = super::super::ipc::handle_request(
                s,
                &request("get_settings", serde_json::json!({})),
            )
            .result
            .unwrap();
            (
                week["recap_card_enabled"].clone(),
                settings["insights_recap_card_enabled"].clone(),
            )
        };
        for on in [false, true] {
            let set = super::super::ipc::handle_request(
                &s,
                &request(
                    "set_settings",
                    serde_json::json!({"insights_recap_card_enabled": on}),
                ),
            );
            assert!(set.error.is_none(), "{:?}", set.error);
            assert_eq!(state(&s), (serde_json::json!(on), serde_json::json!(on)));
        }
    }

    #[test]
    fn the_method_answers_off_when_switched_off_and_refuses_bad_params() {
        let (_dir, s) = shared();
        s.settings.lock().unwrap().insights_counter_pass = false;
        let r = handle_week(&s, &call(serde_json::json!({})));
        assert_eq!(
            r.result.unwrap(),
            serde_json::json!({"enabled": false, "feed": "counter_pass"})
        );
        for on in [false, true] {
            s.settings.lock().unwrap().insights_counter_pass = on;
            let bad_week = handle_week(&s, &call(serde_json::json!({"iso_week": "last"})));
            let err = bad_week.error.unwrap();
            assert_eq!(err.code, ERR_BAD_PARAMS);
            assert_eq!(err.message, ERR_ISO_WEEK_INVALID);
            let bad_tz = handle_week(&s, &call(serde_json::json!({"tz": "UTC"})));
            assert_eq!(
                bad_tz.error.unwrap().message,
                super::super::insights_glance::ERR_TZ_INVALID
            );
        }
        let on = handle_week(
            &s,
            &call(serde_json::json!({"iso_week": "2026-W38", "tz": 0})),
        )
        .result
        .unwrap();
        assert_eq!(on["readable"], true);
        assert_eq!(on["week_start"], "2026-09-14");
    }

    #[test]
    fn the_method_says_why_routing_is_unavailable() {
        let (_dir, s) = shared();
        s.settings.lock().unwrap().insights_counter_pass = true;
        let ask = || {
            handle_week(
                &s,
                &call(serde_json::json!({"iso_week": "2026-W38", "tz": 0})),
            )
            .result
            .unwrap()
        };
        // On by default (owner decision D3): with no ledger declared the
        // reason is the ledger, never the setting.
        assert!(s.settings.lock().unwrap().insights_ledger_feed);
        assert!(s.routing_ledger().is_none());
        let none = ask();
        assert_eq!(none["routing_available"], false);
        assert_eq!(none["routing_unavailable"], "no_ledger");
        // Only a contributor turning it off answers `ledger_feed_off`.
        s.settings.lock().unwrap().insights_ledger_feed = false;
        let off = ask();
        assert_eq!(off["routing_available"], false);
        assert_eq!(off["routing_unavailable"], "ledger_feed_off");
        s.settings.lock().unwrap().insights_ledger_feed = true;
        s.install_routing_ledger_for_test(
            crate::routing::ironwire::IronWireLedger::with_rows_for_test(Vec::new()),
        );
        let on = ask();
        assert_eq!(on["routing_available"], true);
        assert_eq!(on["routing_unavailable"], serde_json::Value::Null);
        // The feed off wins over a declared ledger.
        s.settings.lock().unwrap().insights_ledger_feed = false;
        assert_eq!(ask()["routing_unavailable"], "ledger_feed_off");
    }

    /// The Never folders cannot be read, so nothing is shown: a poisoned
    /// policy lock must not read as "no folder is Never" and let a Never
    /// folder's rows through.
    #[test]
    fn an_unreadable_folder_policy_shows_nothing() {
        let (_dir, s) = shared();
        s.settings.lock().unwrap().insights_counter_pass = true;
        std::thread::scope(|scope| {
            let _ = scope
                .spawn(|| {
                    let _held = s.policy.lock().unwrap();
                    panic!("poison the policy lock");
                })
                .join();
        });
        assert!(s.policy.is_poisoned());
        let r = handle_week(
            &s,
            &call(serde_json::json!({"iso_week": "2026-W38", "tz": 0})),
        )
        .result
        .unwrap();
        assert_eq!(
            r,
            serde_json::json!({
                "enabled": true,
                "feed": "counter_pass",
                "readable": false,
                "reason": UNREADABLE_POLICY,
            })
        );
    }

    /// A Codex call on the proxy for the fixture's session, `minutes` before
    /// `now()`.
    fn ledger_call(id: i64, minutes: i64, session: &str) -> crate::routing::RoutedExchange {
        crate::routing::RoutedExchange {
            id: Some(id),
            started_at: now() - Duration::minutes(minutes),
            client_session_id: Some(session.to_string()),
            facade: "openai".to_string(),
            path: Some("/v1/responses".to_string()),
            backend: "SECRET-BACKEND".to_string(),
            input_tokens: Some(100),
            cache_read_tokens: Some(40),
            cache_write_tokens: Some(0),
            output_tokens: Some(10),
            proof: Some(crate::routing::ProofStatus::Verified),
            ..Default::default()
        }
    }

    fn fold_with(
        pass: &CounterPass,
        enabled: bool,
        rows: Vec<crate::routing::RoutedExchange>,
    ) -> Result<super::super::insights_route_tally::FoldSummary, &'static str> {
        let ledger = crate::routing::FixedLedger::new(rows);
        pass.fold_ledger(&|| enabled, &ledger, &mut Vec::new, now())
    }

    fn tallies(
        pass: &CounterPass,
    ) -> BTreeMap<KeyedDigest, super::super::insights_route_tally::RouteTally> {
        pass.load_cached()
            .unwrap()
            .map(|store| store.routing.clone())
            .unwrap_or_default()
    }

    /// The Models answers read a route only while routing is available: a
    /// tally folded while the feed was on is not read once it is off.
    #[test]
    fn model_routes_follow_the_tally_only_while_routing_is_available() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        let mut call = ledger_call(1, 30, "PRIVATE-CODEX-ID");
        call.proof = Some(crate::routing::ProofStatus::Pending);
        fold_with(&f.pass, true, vec![call]).unwrap();
        let store = f.pass.load_cached().unwrap().unwrap();
        let rows: Vec<&CounterRow> = store.rows.values().collect();
        assert_eq!(rows.len(), 1);
        let on = route_map(&rows, &store.routing, true);
        assert_eq!(
            on.values().copied().collect::<Vec<_>>(),
            vec![ModelRoute::RoutedUnverified]
        );
        let off = route_map(&rows, &store.routing, false);
        assert_eq!(
            off.values().copied().collect::<Vec<_>>(),
            vec![ModelRoute::Unobserved]
        );
        assert_eq!(
            off.keys().collect::<Vec<_>>(),
            on.keys().collect::<Vec<_>>()
        );
    }

    /// `LEDGER_MONEY_IN_INSIGHTS` held: no stored charge reaches the
    /// Models answers, whatever the tally holds.
    #[test]
    fn held_ledger_money_reads_no_stored_charge() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        let mut call = ledger_call(1, 30, "PRIVATE-CODEX-ID");
        call.cost_usd = Some(0.25);
        fold_with(&f.pass, true, vec![call]).unwrap();
        let store = f.pass.load_cached().unwrap().unwrap();
        let rows: Vec<&CounterRow> = store.rows.values().collect();
        assert_eq!(charge_map(&rows, &store.routing, true, true).len(), 1);
        assert!(charge_map(&rows, &store.routing, true, false).is_empty());
    }

    #[test]
    fn a_fold_keys_each_tally_by_the_rows_own_session_digest() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        let summary = fold_with(
            &f.pass,
            true,
            vec![
                ledger_call(1, 30, "PRIVATE-CODEX-ID"),
                ledger_call(2, 20, "PRIVATE-CODEX-ID"),
                // Older than the ledger's window: not read.
                ledger_call(3, 25 * 60, "PRIVATE-CODEX-ID"),
            ],
        )
        .unwrap();
        assert_eq!(summary.folded, 2);
        let row_digest = stored_rows(&f.pass)[0].harness_session.unwrap();
        let routing = tallies(&f.pass);
        assert_eq!(
            routing.keys().copied().collect::<Vec<_>>(),
            vec![row_digest]
        );
        assert_eq!(routing[&row_digest].calls.verified, 2);
        assert_eq!(routing[&row_digest].tokens.verified, 220);

        // Persisted: cursor and tallies survive a reload, and folding the
        // same window again under a fresh process adds nothing.
        let reloaded = f.reopened();
        assert_eq!(tallies(&reloaded), routing);
        let again = fold_with(
            &reloaded,
            true,
            vec![
                ledger_call(1, 30, "PRIVATE-CODEX-ID"),
                ledger_call(2, 20, "PRIVATE-CODEX-ID"),
            ],
        )
        .unwrap();
        assert_eq!(again.folded, 0);
        assert_eq!(tallies(&reloaded), routing);

        // A later pass that rewrites rows keeps the tallies.
        let mut grown = codex_bytes();
        grown.extend_from_slice(b"\n");
        std::fs::write(&codex, grown).unwrap();
        assert_eq!(
            run_with(&reloaded, &[candidate(SOURCE_CODEX, &codex)], &[], false).read,
            1
        );
        assert_eq!(tallies(&reloaded), routing);

        // The store holds the digest, never the id, backend or price.
        let text = f.rows_text();
        for leak in ["PRIVATE-CODEX-ID", "SECRET-BACKEND", "cost"] {
            assert!(!text.contains(leak), "{leak} in the store");
        }
    }

    #[test]
    fn nothing_is_folded_or_read_while_the_feed_is_off() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        let before = f.rows_text();
        struct Unread;
        impl crate::routing::RoutingLedger for Unread {
            fn exchanges_since(&self, _: DateTime<Utc>) -> Vec<crate::routing::RoutedExchange> {
                panic!("the ledger is not read while the feed is off");
            }
        }
        let summary = f
            .pass
            .fold_ledger(
                &|| false,
                &Unread,
                &mut || panic!("no speakers are read while the feed is off"),
                now(),
            )
            .unwrap();
        assert_eq!(summary, Default::default());
        assert_eq!(f.rows_text(), before);
    }

    /// A ledger fold that lands while a pass is reading is kept: the pass
    /// writes its rows onto the store as it stands at the write, not over
    /// it with the store it loaded when it began. The pass runs off the
    /// watcher's tick, so the next tick's fold can arrive mid-pass.
    #[test]
    fn a_fold_during_a_pass_is_not_written_over() {
        let f = Fixture::new();
        let first = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &first)]);
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        let second = f.write("y.jsonl", &codex_bytes());

        let (entered, read_entered) = std::sync::mpsc::channel::<()>();
        let (go, wait_for_go) = std::sync::mpsc::channel::<()>();
        let wait_for_go = Mutex::new(wait_for_go);
        f.pass.set_before_read_for_test(Arc::new(move || {
            let _ = entered.send(());
            let _ = wait_for_go.lock().unwrap().recv();
        }));
        std::thread::scope(|scope| {
            let pass = scope.spawn(|| {
                f.run(&[
                    candidate(SOURCE_CODEX, &first),
                    candidate(SOURCE_CODEX, &second),
                ])
            });
            read_entered.recv().unwrap();
            fold_with(
                &f.pass,
                true,
                vec![
                    ledger_call(1, 30, "PRIVATE-CODEX-ID"),
                    ledger_call(2, 20, "PRIVATE-CODEX-ID"),
                ],
            )
            .unwrap();
            drop(go);
            assert_eq!(pass.join().unwrap().read, 1);
        });
        let store = f.pass.load_cached().unwrap().unwrap();
        assert_eq!(store.rows.len(), 2, "the pass's row is written");
        assert_eq!(
            store.ledger_folded_through,
            Some(2),
            "the fold made during the pass is kept"
        );
    }

    #[test]
    fn a_fold_before_any_pass_writes_nothing_and_makes_no_key() {
        let f = Fixture::new();
        let summary =
            fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        assert_eq!(summary, Default::default());
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
        assert!(f.pass.key_is_absent_for_test());
    }

    #[test]
    fn a_window_with_nothing_new_does_not_write_the_store() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        let path = f.dir.path().join(COUNTER_ROWS_FILE);
        std::fs::remove_file(&path).unwrap();
        // The cached store still answers; nothing new means no write.
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        assert!(!path.exists());
    }

    /// The cursor's start mark persists with its id: after a reload, a
    /// ledger that started over is still told from one whose newest row
    /// aged out, so the restarted ledger's calls are taken, once, and the
    /// rows left in an aged window are not taken again.
    #[test]
    fn the_fold_cursor_survives_a_reload_on_both_sides_of_a_restart() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        let session = "PRIVATE-CODEX-ID";
        // A long call (id 2) began before a short one (id 1) and ended last.
        let short = ledger_call(1, 50, session);
        let long = ledger_call(2, 60, session);
        assert_eq!(
            fold_with(&f.pass, true, vec![long, short.clone()])
                .unwrap()
                .folded,
            2
        );

        // A new process: the long call aged out, the short one did not.
        let reloaded = f.reopened();
        assert_eq!(fold_with(&reloaded, true, vec![short]).unwrap().folded, 0);

        // Another process: the proxy's ledger started over, ids from 1.
        let restarted = f.reopened();
        let mut first = ledger_call(1, 10, session);
        first.proof = Some(crate::routing::ProofStatus::Outside);
        assert_eq!(
            fold_with(&restarted, true, vec![first.clone()])
                .unwrap()
                .folded,
            1
        );
        assert_eq!(
            fold_with(&f.reopened(), true, vec![first]).unwrap().folded,
            0
        );
        let row_digest = stored_rows(&f.pass)[0].harness_session.unwrap();
        let tally = &tallies(&f.reopened())[&row_digest];
        assert_eq!((tally.calls.verified, tally.calls.outside), (2, 1));
    }

    /// The store file is written with the cache lock released, so an
    /// answer never waits on a write; and a fold or pass that changes
    /// nothing writes nothing.
    #[test]
    fn the_store_is_written_off_the_cache_lock_and_only_on_change() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        let (writes, locked) = f.pass.writes_for_test();
        assert_eq!(writes, 2, "one pass and one fold, each a change");
        assert_eq!(locked, 0, "no write holds the cache lock");
        // Nothing new: no write.
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        assert_eq!(f.pass.writes_for_test().0, 2);
    }

    #[test]
    fn tallies_go_with_the_store_on_clear() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 1);
        f.pass.clear().unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 0);
        // With the key forgotten, a fold that still sees the feed on writes
        // nothing back.
        fold_with(&f.pass, true, vec![ledger_call(2, 10, "PRIVATE-CODEX-ID")]).unwrap();
        assert!(!f.dir.path().join(COUNTER_ROWS_FILE).exists());
    }

    #[test]
    fn a_never_folder_drops_its_sessions_tally() {
        let f = Fixture::new();
        let codex = f.write("x.jsonl", &codex_bytes());
        f.run(&[candidate(SOURCE_CODEX, &codex)]);
        fold_with(&f.pass, true, vec![ledger_call(1, 30, "PRIVATE-CODEX-ID")]).unwrap();
        assert_eq!(tallies(&f.pass).len(), 1);
        run_with(
            &f.pass,
            &[candidate(SOURCE_CODEX, &codex)],
            &[PROJECT.to_string()],
            true,
        );
        assert!(stored_rows(&f.pass).is_empty());
        assert!(tallies(&f.pass).is_empty());
    }

    /// The session ID a Codex rollout's file name ends with, in the shape
    /// Codex writes.
    const NEVER_ID: &str = "0199a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";

    fn never_rollout(f: &Fixture) -> PathBuf {
        f.write(
            &format!("rollout-2026-09-14T09-00-00-{NEVER_ID}.jsonl"),
            &codex_bytes_for(NEVER_ID),
        )
    }

    fn charged_call(id: i64, minutes: i64, session: &str) -> crate::routing::RoutedExchange {
        let mut call = ledger_call(id, minutes, session);
        call.cost_usd = Some(0.25);
        call.served_model = Some("PRIVATE-SERVED-MODEL".to_string());
        call
    }

    /// Kristi, 2026-10-28: Private AI sessions started from the app go
    /// through managed routes, whose ledgers sit apart from the shared
    /// proxy's. Each is folded by its own cursor, so two routes' calls with
    /// the same id both count, a second fold takes neither again, and a
    /// call older than the shared ledger's 24-hour window still counts.
    #[test]
    fn managed_route_ledgers_fold_each_call_once_by_route_and_id() {
        let f = Fixture::new();
        f.pass.keys_load_or_create_for_test();
        let mut old = charged_call(1, 60 * 48, "SESSION-SECRET");
        old.cost_usd = Some(0.5);
        let other = charged_call(1, 30, "SESSION-SECRET");
        let mut homes = ManagedRows::new();
        homes.insert("route-a".to_string(), vec![&old]);
        homes.insert("route-b".to_string(), vec![&other]);
        let none = crate::routing::FixedLedger::new(Vec::new());
        let fold = |homes: &ManagedRows<'_>| {
            f.pass
                .fold_ledgers(&|| true, &none, Some(homes), &mut Vec::new, now())
                .unwrap()
        };
        assert_eq!(fold(&homes).folded, 2);
        let charge = |pass: &CounterPass| {
            let all = tallies(pass);
            assert_eq!(all.len(), 1);
            all.values().next().unwrap().charge.clone().unwrap()
        };
        assert_eq!(charge(&f.pass).known(), Some(750_000));
        assert_eq!(fold(&homes).folded, 0, "each call once");
        assert_eq!(charge(&f.pass).charged_calls, 2);

        // A new call on one route is the only one taken.
        let next = charged_call(2, 10, "SESSION-SECRET");
        homes.get_mut("route-b").unwrap().push(&next);
        assert_eq!(fold(&homes).folded, 1);
        assert_eq!(charge(&f.pass).known(), Some(1_000_000));

        // A route that is gone loses its cursor; unreadable routes keep
        // theirs.
        homes.remove("route-a");
        fold(&homes);
        let cursors = |pass: &CounterPass| {
            pass.load_cached()
                .unwrap()
                .unwrap()
                .managed_folded
                .keys()
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(cursors(&f.pass), ["route-b"]);
        f.pass
            .fold_ledgers(&|| true, &none, None, &mut Vec::new, now())
            .unwrap();
        assert_eq!(cursors(&f.pass), ["route-b"]);
        assert!(!f.rows_text().contains("SESSION-SECRET"));
    }

    /// The owner's staging data, 2026-10-28: managed routes on the
    /// `nearai` backend with no proof label, called by a tool no session
    /// is named for (opencode), two days before the week on screen, some
    /// calls refused with no charge. Before the fix every call was skipped
    /// and no NEAR AI card showed.
    fn owner_route_call(id: i64, cost: Option<f64>, status: i64) -> crate::routing::RoutedExchange {
        crate::routing::RoutedExchange {
            id: Some(id),
            started_at: now() - Duration::days(2) + Duration::minutes(id),
            client_session_id: Some("OPENCODE-SESSION".to_string()),
            facade: "openai".to_string(),
            path: Some("/v1/chat/completions".to_string()),
            backend: "nearai".to_string(),
            cost_usd: cost,
            status,
            proof: None,
            ..Default::default()
        }
    }

    fn credit_now(f: &Fixture) -> serde_json::Value {
        f.pass.week_value_with_folders(
            true,
            Some(monday()),
            utc(),
            now(),
            &NeverFolders::NONE,
            (routed(), &[]),
        )["near_ai_credit"]
            .clone()
    }

    #[test]
    fn spend_no_session_is_named_for_still_counts_in_the_sums() {
        let f = Fixture::new();
        f.pass.keys_load_or_create_for_test();
        let a = [owner_route_call(1, Some(0.058), 200)];
        let b = [
            owner_route_call(1, Some(4.72), 200),
            // Refused, not billed: a known $0.
            owner_route_call(2, None, 429),
            owner_route_call(3, None, 503),
        ];
        let mut homes = ManagedRows::new();
        homes.insert("route-a".to_string(), a.iter().collect());
        homes.insert("route-b".to_string(), b.iter().collect());
        let none = crate::routing::FixedLedger::new(Vec::new());
        let summary = f
            .pass
            .fold_ledgers(&|| true, &none, Some(&homes), &mut Vec::new, now())
            .unwrap();
        assert_eq!(summary.skipped, 4, "no session is named for these calls");
        assert_eq!(tallies(&f.pass).len(), 0);

        let credit = credit_now(&f);
        // Nothing this week: a true zero.
        assert_eq!(credit["week"]["usd_micros"], 0);
        assert_eq!(credit["week"]["calls"], 0);
        let month = &credit["last_31_days"];
        assert_eq!(month["usd_micros"], 4_778_000);
        assert_eq!(month["calls"], 4);
        assert_eq!(month["calls_without_charge"], 0);
        assert_eq!(month["sessions"], 0, "no trace is counted for them");
        assert_eq!(credit["by_day"][4]["usd_micros"], 4_778_000);
        assert_eq!(credit["by_day"][4]["date"], "2026-09-12");

        // A completed call with no charge makes the sum unknown.
        let c = [owner_route_call(1, None, 200)];
        homes.insert("route-c".to_string(), c.iter().collect());
        f.pass
            .fold_ledgers(&|| true, &none, Some(&homes), &mut Vec::new, now())
            .unwrap();
        let month = &credit_now(&f)["last_31_days"];
        assert_eq!(month["usd_micros"], serde_json::Value::Null);
        assert_eq!(month["calls_without_charge"], 1);
        // Owner, 2026-10-28: the owner's data (refused calls, one completed
        // call with no charge) reads "at least" the known sum.
        assert_eq!(month["usd_micros_known"], 4_778_000);
        assert_eq!(month["usd_partial"], true);
    }

    /// A store a build before this one folded (its cursors past the calls
    /// it skipped) takes those calls once more, for their charge only, and
    /// never counts a folded call twice.
    #[test]
    fn calls_skipped_before_the_loose_charge_are_taken_once_more() {
        let f = Fixture::new();
        f.pass.keys_load_or_create_for_test();
        let rows = [
            owner_route_call(1, Some(0.25), 200),
            owner_route_call(2, Some(0.5), 200),
        ];
        let mut homes = ManagedRows::new();
        homes.insert("route-a".to_string(), rows.iter().collect());
        let none = crate::routing::FixedLedger::new(Vec::new());
        let fold = |homes: &ManagedRows<'_>| {
            f.pass
                .fold_ledgers(&|| true, &none, Some(homes), &mut Vec::new, now())
                .unwrap()
        };
        fold(&homes);
        // As the earlier build left it: cursor moved, no loose charge.
        {
            let mut cached = f.pass.cached.lock().unwrap();
            let mut store = (*f.pass.load_into(&mut cached).unwrap().unwrap()).clone();
            assert_eq!(store.loose_epoch, LOOSE_EPOCH);
            store.loose_charge = None;
            store.loose_epoch = 0;
            f.pass.stage_into(&mut cached, store);
        }
        f.pass.flush().unwrap();
        assert_eq!(credit_now(&f)["last_31_days"]["calls"], 0);
        fold(&homes);
        assert_eq!(credit_now(&f)["last_31_days"]["usd_micros"], 750_000);
        fold(&homes);
        assert_eq!(
            credit_now(&f)["last_31_days"]["usd_micros"],
            750_000,
            "once"
        );
    }

    /// A session in a Never folder has no row (it is never read), so the
    /// row-driven drop above never reaches its tally. It leaves no tally,
    /// charge or served-model label: calls folded before the pass learns its
    /// folder go at that pass, a session still being written included, and
    /// calls folded after are not kept.
    #[test]
    fn a_never_folder_session_leaves_no_tally_charge_or_model() {
        let f = Fixture::new();
        f.pass.keys_load_or_create_for_test();
        let rollout = never_rollout(&f);
        fold_with(&f.pass, true, vec![charged_call(1, 30, NEVER_ID)]).unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 1);

        // Still being written: not read, but its folder is known Never.
        let mut live = candidate(SOURCE_CODEX, &rollout);
        live.modified_at = now();
        let summary = run_with(&f.pass, &[live], &[PROJECT.to_string()], true);
        assert_eq!(summary.read, 0);
        assert_eq!(f.pass.tallied_sessions_for_test(), 0);
        let text = f.rows_text().to_lowercase();
        for leak in ["usd_micros", "private-served-model", "charge"] {
            assert!(!text.contains(leak), "{leak} in the store");
        }

        // Calls folded after are not kept, before or after it goes quiet.
        fold_with(&f.pass, true, vec![charged_call(2, 20, NEVER_ID)]).unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 0);
        run_with(
            &f.pass,
            &[candidate(SOURCE_CODEX, &rollout)],
            &[PROJECT.to_string()],
            true,
        );
        fold_with(&f.pass, true, vec![charged_call(3, 10, NEVER_ID)]).unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 0);
        assert!(!f.rows_text().to_lowercase().contains("usd_micros"));
    }

    /// A quiet Never session the pass has already passed over leaves no
    /// tally when its calls are folded only afterwards.
    #[test]
    fn calls_folded_after_a_pass_skipped_a_never_session_leave_no_tally() {
        let f = Fixture::new();
        let rollout = never_rollout(&f);
        run_with(
            &f.pass,
            &[candidate(SOURCE_CODEX, &rollout)],
            &[PROJECT.to_string()],
            true,
        );
        fold_with(&f.pass, true, vec![charged_call(1, 30, NEVER_ID)]).unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 0);
    }

    /// A session still being written in a folder that is not Never keeps
    /// the tally its calls made before its row: the purge reads the folder,
    /// it does not drop every tally without a row.
    #[test]
    fn a_live_session_outside_never_folders_keeps_its_tally() {
        let f = Fixture::new();
        f.pass.keys_load_or_create_for_test();
        let rollout = never_rollout(&f);
        fold_with(&f.pass, true, vec![charged_call(1, 30, NEVER_ID)]).unwrap();
        let mut live = candidate(SOURCE_CODEX, &rollout);
        live.modified_at = now();
        run_with(&f.pass, &[live], &[], false);
        assert_eq!(f.pass.tallied_sessions_for_test(), 1);
    }

    /// Under a global Never override no tally is kept, matched by a row or
    /// not, and none is made.
    #[test]
    fn every_folder_never_leaves_no_tally() {
        let f = Fixture::new();
        f.pass.keys_load_or_create_for_test();
        fold_with(&f.pass, true, vec![charged_call(1, 30, "PRIVATE-UNLISTED")]).unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 1);
        f.pass
            .run_pass(&[], &NeverFolders::Every, now(), QUIET, &mut |_| {
                (PROJECT.to_string(), true)
            })
            .unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 0);
        fold_with(&f.pass, true, vec![charged_call(2, 20, "PRIVATE-UNLISTED")]).unwrap();
        assert_eq!(f.pass.tallied_sessions_for_test(), 0);
    }

    #[test]
    fn the_method_is_advertised_dispatched_and_local() {
        assert!(super::super::ipc::METHODS.contains(&"insights_week"));
        assert!(super::super::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&"insights_week"));
        let (_dir, mut s) = shared();
        s.dev_dry_run = true;
        s.settings.lock().unwrap().insights_counter_pass = false;
        let r = super::super::ipc::handle_request(&s, &call(serde_json::json!({})));
        assert_eq!(r.result.unwrap()["enabled"], false);
    }

    // Session facts and the Codex per-turn series on counter rows (Insights
    // model answers, PR 1, Task 6).

    fn facts_fixture(name: &str) -> Vec<u8> {
        fixture(&format!("claude-session-facts/{name}.jsonl"))
    }

    fn codex_series_bytes() -> Vec<u8> {
        fixture("codex-turn-series/session.jsonl")
    }

    /// The one stored row, which must exist.
    fn only_row(pass: &CounterPass) -> CounterRow {
        let rows = stored_rows(pass);
        assert_eq!(rows.len(), 1);
        rows.into_iter().next().unwrap()
    }

    /// Rewrite every stored row with `edit` over its JSON object.
    fn edit_stored_rows(
        f: &Fixture,
        edit: impl Fn(&mut serde_json::Map<String, serde_json::Value>),
    ) {
        let mut value: serde_json::Value = serde_json::from_str(&f.rows_text()).unwrap();
        for row in value["rows"].as_object_mut().unwrap().values_mut() {
            edit(row.as_object_mut().unwrap());
        }
        f.write(COUNTER_ROWS_FILE, value.to_string().as_bytes());
    }

    #[test]
    fn a_claude_row_stores_facts_and_a_codex_row_its_series() {
        let f = Fixture::new();
        let claude = f.write("claude.jsonl", &facts_fixture("finished"));
        let codex = f.write("codex.jsonl", &codex_series_bytes());
        let mut older = candidate(SOURCE_CODEX, &codex);
        older.modified_at = written() - Duration::minutes(1);
        assert_eq!(
            f.run(&[candidate(SOURCE_CLAUDE_CODE, &claude), older]).read,
            2
        );
        let rows = stored_rows(&f.pass);
        assert!(
            rows.iter()
                .all(|row| row.facts_version == SESSION_FACTS_VERSION)
        );
        let claude_row = rows
            .iter()
            .find(|row| matches!(row.body, RowBody::Claude { .. }))
            .unwrap();
        let facts = claude_row.facts.as_ref().unwrap();
        assert_eq!(facts.models.len(), 1);
        assert_eq!(facts.models[0].edits, Some(1));
        assert_eq!(facts.finished, Some(true));
        let codex_row = rows
            .iter()
            .find(|row| matches!(row.body, RowBody::Codex { .. }))
            .unwrap();
        let RowBody::Codex {
            series: Some(evidence),
            ..
        } = &codex_row.body
        else {
            panic!("no Codex series stored: {:?}", codex_row.body);
        };
        assert_eq!(evidence.model_labels, vec!["model-a".to_string()]);
        assert_eq!(evidence.series.turns.len(), 3);
        assert!(
            evidence
                .series
                .turns
                .iter()
                .all(|turn| turn.output.is_some())
        );
        let codex_facts = codex_row.facts.as_ref().unwrap();
        assert_eq!(codex_facts.models[0].edits, Some(1));
        // The engine sees both.
        let input = codex_row.session_input();
        assert!(matches!(
            input.body,
            SessionBody::Codex {
                series: Some(_),
                ..
            }
        ));
        assert_eq!(input.facts.as_ref(), Some(codex_facts));
        assert_eq!(claude_row.session_input().facts.as_ref(), Some(facts));
    }

    #[test]
    fn a_row_without_facts_is_read_once_more_then_not_again() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &facts_fixture("finished"));
        let session = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(std::slice::from_ref(&session)).read, 1);
        // A row written before facts were kept: neither field.
        edit_stored_rows(&f, |row| {
            assert!(row.remove("facts").is_some());
            assert!(row.remove("facts_version").is_some());
        });
        let restarted = f.reopened();
        let before = only_row(&restarted);
        assert_eq!((before.facts.as_ref(), before.facts_version), (None, 0));
        assert_eq!(
            run_with(&restarted, std::slice::from_ref(&session), &[], false).read,
            1,
            "read once more, though unchanged"
        );
        let after = only_row(&restarted);
        assert_eq!(after.facts_version, SESSION_FACTS_VERSION);
        assert_eq!(after.facts.unwrap().models[0].edits, Some(1));
        assert_eq!(
            run_with(&restarted, std::slice::from_ref(&session), &[], false).read,
            0,
            "and only once"
        );
    }

    #[test]
    fn a_row_with_an_old_facts_version_is_read_once_more_within_the_budget() {
        let extra = 4;
        let f = Fixture::new();
        let candidates: Vec<CounterCandidate> = (0..COUNTER_PASS_MAX_READS_PER_TICK + extra)
            .map(|i| {
                let path = f.write(&format!("c{i}.jsonl"), &codex_series_bytes());
                let mut c = candidate(SOURCE_CODEX, &path);
                c.modified_at = written() - Duration::minutes(i as i64);
                c
            })
            .collect();
        assert_eq!(f.run(&candidates).read, COUNTER_PASS_MAX_READS_PER_TICK);
        assert_eq!(f.run(&candidates).read, extra);
        assert_eq!(f.run(&candidates).read, 0);
        let mut store = (*f.pass.load_cached().unwrap().unwrap()).clone();
        for row in store.rows.values_mut() {
            row.facts_version = SESSION_FACTS_VERSION - 1;
        }
        f.pass.save(store).unwrap();
        let first = f.run(&candidates);
        assert_eq!(
            (first.read, first.deferred),
            (COUNTER_PASS_MAX_READS_PER_TICK, extra)
        );
        assert_eq!(f.run(&candidates).read, extra);
        assert_eq!(f.run(&candidates).read, 0);
        assert!(
            stored_rows(&f.pass)
                .iter()
                .all(|row| row.facts_version == SESSION_FACTS_VERSION && row.facts.is_some())
        );
    }

    #[test]
    fn a_grown_session_recomputes_every_fact() {
        let f = Fixture::new();
        let path = f.write("s.jsonl", &facts_fixture("finished"));
        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        let before = only_row(&f.pass).facts.unwrap();
        assert_eq!(before.finished, Some(true));
        assert_eq!(before.models[0].edits, Some(1));
        assert_eq!(before.models[0].interrupted, Some(0));

        // A second prompt, a second edit, then an interrupt.
        let mut grown = facts_fixture("finished");
        for line in [
            r#"{"type":"user","timestamp":"2026-01-05T10:00:20.000Z","message":{"role":"user","content":"PRIVATE_TEXT and the other one"},"sessionId":"fixture-session"}"#,
            r#"{"type":"assistant","timestamp":"2026-01-05T10:00:25.000Z","message":{"id":"m3","model":"claude-model-a","role":"assistant","content":[{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/work/PRIVATE_PATH/src/main.rs","old_string":"PRIVATE_TEXT c","new_string":"PRIVATE_TEXT d","replace_all":false}}],"usage":{"input_tokens":3,"output_tokens":5,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}},"sessionId":"fixture-session"}"#,
            r#"{"type":"user","timestamp":"2026-01-05T10:00:26.000Z","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t2","content":"ok PRIVATE_TEXT","is_error":false}]},"sessionId":"fixture-session"}"#,
            r#"{"type":"user","timestamp":"2026-01-05T10:00:27.000Z","message":{"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]},"sessionId":"fixture-session"}"#,
        ] {
            if grown.last() != Some(&b'\n') {
                grown.push(b'\n');
            }
            grown.extend_from_slice(line.as_bytes());
            grown.push(b'\n');
        }
        std::fs::write(&path, &grown).unwrap();
        let mut later = candidate(SOURCE_CLAUDE_CODE, &path);
        later.modified_at = written() + Duration::minutes(1);
        assert_eq!(f.run(&[later]).read, 1);
        let after = only_row(&f.pass).facts.unwrap();
        assert_eq!(after.finished, Some(false), "a stale finished survived");
        assert_eq!(after.models[0].edits, Some(2));
        assert_eq!(after.models[0].interrupted, Some(1));
        assert_eq!(after.user_turns, Some(2));
        assert_eq!(after.shape.edited_files.source, 2);
    }

    #[test]
    fn a_stale_classifier_version_reclassifies_without_a_read() {
        use crate::insights::analytics_constants::CLASSIFIER_VERSION;
        use crate::insights::session_facts::InferredTask;
        let f = Fixture::new();
        let path = f.write("s.jsonl", &facts_fixture("finished"));
        let session = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(std::slice::from_ref(&session)).read, 1);
        let current = only_row(&f.pass).facts.unwrap().task;
        assert_ne!(current, InferredTask::Docs);
        let mut store = (*f.pass.load_cached().unwrap().unwrap()).clone();
        for row in store.rows.values_mut() {
            let facts = row.facts.as_mut().unwrap();
            facts.classifier_version = CLASSIFIER_VERSION - 1;
            facts.task = InferredTask::Docs;
        }
        f.pass.save(store).unwrap();
        assert_eq!(f.run(std::slice::from_ref(&session)).read, 0, "no read");
        let row = only_row(&f.pass);
        assert_eq!(row.facts.as_ref().unwrap().task, InferredTask::Docs);
        let answered = row.session_input().facts.unwrap();
        assert_eq!(answered.task, current, "re-classified from the shape");
        assert_eq!(answered.classifier_version, CLASSIFIER_VERSION);
    }

    #[test]
    fn delegated_and_capped_are_told_apart() {
        use trace_commons_protocol::insights_usage_series::MAX_SERIES_TURNS;
        let f = Fixture::new();
        let path = f.write("s.jsonl", &facts_fixture("finished"));
        let mut group = candidate(SOURCE_CLAUDE_CODE, &path);
        group.group_member_count = 2;
        f.run(&[group]);
        let row = only_row(&f.pass);
        let facts = row.facts.as_ref().unwrap();
        assert!(facts.delegated_unread && !facts.capped, "{facts:?}");
        assert!(matches!(&row.body, RowBody::Claude { series, .. } if series.truncated));

        let f = Fixture::new();
        let path = f.write("large.jsonl", &large_claude_bytes(MAX_SERIES_TURNS + 8, 0));
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let facts = only_row(&f.pass).facts.unwrap();
        assert!(facts.capped && !facts.delegated_unread, "{facts:?}");
    }

    /// Owner decision, 2026-10-20: Codex per-turn series have their own
    /// allowance. A store over the shared turn and tool-call caps only
    /// because of a Codex series keeps every row.
    #[test]
    fn codex_series_do_not_count_toward_the_shared_caps() {
        let f = Fixture::new();
        let path = f.write("c.jsonl", &codex_series_bytes());
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let row = only_row(&f.pass);
        assert_eq!(row.codex_series_turns(), 3);
        assert!(row.codex_series_tool_calls() > 0);
        assert_eq!((row.turns(), row.tool_calls()), (0, 0));
        let store = (*f.pass.load_cached().unwrap().unwrap()).clone();
        let mut capped = store.clone();
        enforce_caps(&mut capped, usize::MAX, 0, 0, usize::MAX, true);
        assert_eq!(capped, store, "no row evicted, no series trimmed");
    }

    #[test]
    fn a_failed_facts_scan_keeps_the_counters() {
        let f = Fixture::new();
        // An object without a `type`: the usage scan passes over it, the
        // per-turn reader refuses it.
        let mut bytes = codex_bytes();
        bytes.extend_from_slice(b"\n{\"timestamp\":\"2026-09-14T09:02:00Z\"}\n");
        let path = f.write("c.jsonl", &bytes);
        let session = candidate(SOURCE_CODEX, &path);
        assert_eq!(f.run(std::slice::from_ref(&session)).read, 1);
        let row = only_row(&f.pass);
        assert!(
            matches!(
                row.body,
                RowBody::Codex {
                    observed: Some(_),
                    series: None,
                    series_trimmed: false,
                }
            ),
            "{:?}",
            row.body
        );
        assert_eq!(row.facts, None);
        assert_eq!(row.facts_version, SESSION_FACTS_VERSION);
        let line = f.week(&[])["rollup"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["source"] == "codex")
            .cloned()
            .unwrap();
        assert_eq!(line["tokens"], 80 + 30, "the counters are kept");
        assert_eq!(f.run(std::slice::from_ref(&session)).read, 0);
    }

    #[test]
    fn an_unknown_row_is_not_read_on_every_poll() {
        let f = Fixture::new();
        let mut bytes = facts_fixture("finished");
        bytes.extend_from_slice(b"not json\n");
        let path = f.write("bad.jsonl", &bytes);
        let session = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(std::slice::from_ref(&session)).read, 1);
        let row = only_row(&f.pass);
        assert!(matches!(row.body, RowBody::Unknown { .. }));
        assert_eq!(row.facts, None);
        assert_eq!(row.facts_version, SESSION_FACTS_VERSION);
        assert_eq!(f.run(std::slice::from_ref(&session)).read, 0);
    }

    #[test]
    fn the_stored_row_holds_no_path_text_command_or_price() {
        let f = Fixture::new();
        let claude = f.write("PRIVATE-claude.jsonl", &facts_fixture("bug-fix"));
        let finished = f.write("PRIVATE-finished.jsonl", &facts_fixture("finished"));
        let codex = f.write("PRIVATE-codex.jsonl", &codex_series_bytes());
        let candidates: Vec<CounterCandidate> = [
            (SOURCE_CLAUDE_CODE, &claude),
            (SOURCE_CLAUDE_CODE, &finished),
            (SOURCE_CODEX, &codex),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (source, path))| {
            let mut c = candidate(source, path);
            c.modified_at = written() - Duration::minutes(i as i64);
            c
        })
        .collect();
        assert_eq!(f.run(&candidates).read, 3);
        let rows = stored_rows(&f.pass);
        // Not vacuous: facts and a Codex series were stored.
        assert!(rows.iter().all(|row| row.facts.is_some()));
        assert!(rows.iter().any(|row| matches!(
            row.body,
            RowBody::Codex {
                series: Some(_),
                ..
            }
        )));
        let text = f.rows_text();
        for forbidden in [
            "PRIVATE",
            "/work",
            "lib.rs",
            "src/",
            ".rs\"",
            "Process exited",
            // The command, not the fixed tool label (`bash`, `apply_patch`).
            "exec_command",
            "cmd",
            "cost",
            "price",
            "usd",
            "USD",
            f.dir.path().to_str().unwrap(),
        ] {
            assert!(!text.contains(forbidden), "{forbidden} stored: {text}");
        }
    }

    #[test]
    fn rows_without_facts_keep_todays_week_answer() {
        let f = Fixture::new();
        let claude = f.write("claude.jsonl", &claude_bytes());
        let facts = f.write("facts.jsonl", &facts_fixture("finished"));
        let codex = f.write("codex.jsonl", &codex_bytes());
        let series = f.write("series.jsonl", &codex_series_bytes());
        let candidates: Vec<CounterCandidate> = [
            (SOURCE_CLAUDE_CODE, &claude),
            (SOURCE_CLAUDE_CODE, &facts),
            (SOURCE_CODEX, &codex),
            (SOURCE_CODEX, &series),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (source, path))| {
            let mut c = candidate(source, path);
            c.modified_at = written() - Duration::minutes(i as i64);
            c
        })
        .collect();
        assert_eq!(f.run(&candidates).read, 4);
        let answer = |pass: &CounterPass, week: Option<NaiveDate>| {
            let mut value = pass.week_value(
                true,
                week,
                utc(),
                now(),
                &NeverFolders::NONE,
                WeekOptions::default(),
            );
            value.as_object_mut().unwrap().remove("updated_at");
            // `work` prices the per-turn series and counts the facts, so it
            // moves with them by design; every other figure must not.
            value.as_object_mut().unwrap().remove("work");
            value
        };
        let weeks = [Some(monday()), NaiveDate::from_ymd_opt(2026, 1, 5)];
        let with_facts: Vec<_> = weeks
            .iter()
            .map(|week| answer(&f.reopened(), *week))
            .collect();
        // Not vacuous: facts and a series were there to strip.
        assert!(f.rows_text().contains("\"facts\":{"));
        edit_stored_rows(&f, |row| {
            row.remove("facts");
            row.remove("facts_version");
            let body = row["body"].as_object_mut().unwrap();
            if body["kind"] == "codex" {
                body.remove("series");
            }
        });
        assert!(!f.rows_text().contains("\"facts\""));
        let without: Vec<_> = weeks
            .iter()
            .map(|week| answer(&f.reopened(), *week))
            .collect();
        assert_eq!(with_facts, without);
        // A Codex series trimmed by its budget changes no weekly figure.
        edit_stored_rows(&f, |row| {
            let body = row["body"].as_object_mut().unwrap();
            if body["kind"] == "codex" {
                body.insert("series_trimmed".into(), serde_json::Value::Bool(true));
            }
        });
        assert!(f.rows_text().contains("\"series_trimmed\":true"));
        let trimmed: Vec<_> = weeks
            .iter()
            .map(|week| answer(&f.reopened(), *week))
            .collect();
        assert_eq!(with_facts, trimmed);
    }

    // The Codex series allowance (owner decision 2026-10-20, #1510).

    /// A Codex row in the week of `written()` whose observed change is
    /// `CODEX_SYNTHETIC_TOKENS`, with a series of `turns` turns and `calls`
    /// tool calls.
    fn codex_series_row(
        modified_at: DateTime<Utc>,
        seq: u64,
        turns: u32,
        calls: u32,
    ) -> CounterRow {
        use trace_commons_protocol::insights_usage_series::{
            CacheWrite, ToolCallRecord, ToolKind, TurnRecord,
        };
        let at = Utc.with_ymd_and_hms(2026, 9, 14, 9, 0, 0).unwrap();
        let series = UsageSeries {
            turns: (0..turns)
                .map(|ordinal| TurnRecord {
                    ordinal,
                    at: Some(at + Duration::seconds(i64::from(ordinal))),
                    uncached_input: Some(10),
                    cache_read: Some(20),
                    cache_write: Some(CacheWrite::TotalOnly(0)),
                    output: Some(5),
                    model_label_ix: Some(0),
                    gap_secs: None,
                    msg_key: None,
                })
                .collect(),
            tool_calls: (0..calls)
                .map(|i| ToolCallRecord {
                    turn_ordinal: i * turns.max(1) / calls.max(1),
                    tool: ToolKind::Bash,
                    args_key: KeyedDigest([7; 32]),
                    path_key: None,
                    path_ext: None,
                    result_bytes: Some(100),
                    success: Some(true),
                    paired: true,
                })
                .collect(),
            truncated: false,
        };
        let mut row = synthetic_row(
            modified_at,
            seq,
            RowBody::Codex {
                observed: Some(CodexObserved {
                    first_at: at,
                    last_at: at + Duration::minutes(1),
                    input: 80,
                    cached_input: 40,
                    output: 30,
                    total: CODEX_SYNTHETIC_TOKENS,
                    baseline_excluded: false,
                }),
                series: Some(CodexTurnSeriesEvidence {
                    model_labels: vec!["gpt-6".to_string()],
                    series,
                }),
                series_trimmed: false,
            },
        );
        row.first_event_at = Some(at);
        row.earliest_event_at = Some(at);
        row.latest_event_at = Some(at + Duration::minutes(1));
        row
    }

    const CODEX_SYNTHETIC_TOKENS: u64 = 110;

    fn is_trimmed(row: &CounterRow) -> bool {
        matches!(
            row.body,
            RowBody::Codex {
                series: None,
                series_trimmed: true,
                ..
            }
        )
    }

    /// Over the Codex budget, by turns or by tool calls, the oldest Codex
    /// rows lose their series and nothing else; the newest keep theirs, and
    /// Claude rows and every total are untouched.
    #[test]
    fn past_the_codex_budget_the_oldest_series_are_trimmed_first() {
        let build = || {
            let mut store = CounterStore::empty(KeyedDigest([0; 32]));
            for i in 0..4u8 {
                store.rows.insert(
                    KeyedDigest([i; 32]),
                    codex_series_row(
                        written() + Duration::minutes(i64::from(i)),
                        u64::from(i),
                        3,
                        2,
                    ),
                );
            }
            store
        };
        for (max_turns, max_tools) in [(7, usize::MAX), (usize::MAX, 5)] {
            let mut store = build();
            let before = store.clone();
            trim_codex_series(&mut store, max_turns, max_tools);
            let trimmed: Vec<u8> = store
                .rows
                .iter()
                .filter(|(_, row)| is_trimmed(row))
                .map(|(k, _)| k.0[0])
                .collect();
            assert_eq!(trimmed, vec![0, 1], "budget {max_turns}/{max_tools}");
            assert_eq!(store.rows.len(), 4, "no row evicted");
            assert!(store.evicted.is_empty(), "a trimmed row is not evicted");
            for (key, row) in &store.rows {
                let old = &before.rows[key];
                let (RowBody::Codex { observed, .. }, RowBody::Codex { observed: was, .. }) =
                    (&row.body, &old.body)
                else {
                    panic!("Codex rows");
                };
                assert_eq!(observed, was, "totals kept");
                assert_eq!(
                    (
                        row.modified_at,
                        row.seq,
                        row.facts.clone(),
                        row.earliest_event_at
                    ),
                    (
                        old.modified_at,
                        old.seq,
                        old.facts.clone(),
                        old.earliest_event_at
                    )
                );
                if !is_trimmed(row) {
                    assert_eq!(row, old, "the newest keep their series");
                }
            }
            let total: usize = store
                .rows
                .values()
                .map(CounterRow::codex_series_turns)
                .sum();
            assert_eq!(total, 6);
        }
    }

    /// Fields an older daemon does not know are left out of the file while
    /// they hold their defaults, so a row this build would write the way
    /// an older one did is written the same way. A row with facts or a
    /// read automation marker still names them (see the PR's downgrade note
    /// and the IPC doc's "The store").
    #[test]
    fn default_row_fields_are_left_out_of_the_file() {
        let mut row = codex_series_row(written(), 0, 1, 0);
        row.facts = None;
        row.facts_version = 0;
        row.automation = Automation::Unknown;
        row.automation_read = false;
        let value = serde_json::to_value(&row).unwrap();
        for field in ["facts", "facts_version", "automation", "automation_read"] {
            assert!(value.get(field).is_none(), "{field} written: {value}");
        }
        assert_eq!(serde_json::from_value::<CounterRow>(value).unwrap(), row);
        // Set, they are written and read back.
        row.facts_version = SESSION_FACTS_VERSION;
        row.automation = Automation::Person;
        row.automation_read = true;
        let value = serde_json::to_value(&row).unwrap();
        for field in ["facts_version", "automation", "automation_read"] {
            assert!(value.get(field).is_some(), "{field} missing: {value}");
        }
        assert_eq!(serde_json::from_value::<CounterRow>(value).unwrap(), row);
    }

    /// Automated sessions' series go before the person's, whatever their
    /// age: nothing draws an automated session's series (the Sessions feed
    /// and the Models answers leave it out), so a burst of reviewer threads
    /// never trims the person's own rows.
    #[test]
    fn automated_series_are_trimmed_before_the_persons() {
        let mut store = CounterStore::empty(KeyedDigest([0; 32]));
        for i in 0..4u8 {
            let mut row = codex_series_row(
                written() + Duration::minutes(i64::from(i)),
                u64::from(i),
                3,
                2,
            );
            // The two newest are automated.
            if i >= 2 {
                row.automation = Automation::Automated;
                row.automation_read = true;
            }
            store.rows.insert(KeyedDigest([i; 32]), row);
        }
        trim_codex_series(&mut store, 6, usize::MAX);
        let trimmed: Vec<u8> = store
            .rows
            .iter()
            .filter(|(_, row)| is_trimmed(row))
            .map(|(digest, _)| digest.0[0])
            .collect();
        assert_eq!(trimmed, vec![2, 3]);
        // Past what the automated rows hold, the person's oldest go next.
        trim_codex_series(&mut store, 3, usize::MAX);
        let trimmed: Vec<u8> = store
            .rows
            .iter()
            .filter(|(_, row)| is_trimmed(row))
            .map(|(digest, _)| digest.0[0])
            .collect();
        assert_eq!(trimmed, vec![0, 2, 3]);
    }

    /// A trimmed row is not read again on the next poll, and stays trimmed.
    #[test]
    fn a_trimmed_codex_row_is_not_read_again() {
        let f = Fixture::new();
        let older = f.write("older.jsonl", &codex_series_bytes());
        let newer = f.write("newer.jsonl", &codex_series_bytes());
        let mut old = candidate(SOURCE_CODEX, &older);
        old.modified_at = written() - Duration::minutes(1);
        let new = candidate(SOURCE_CODEX, &newer);
        let both = [old, new];
        assert_eq!(f.run(&both).read, 2);
        let mut store = (*f.pass.load_cached().unwrap().unwrap()).clone();
        trim_codex_series(&mut store, 3, usize::MAX);
        f.pass.save(store).unwrap();
        let trimmed = |pass: &CounterPass| {
            stored_rows(pass)
                .iter()
                .filter(|row| is_trimmed(row))
                .map(|row| row.modified_at)
                .collect::<Vec<_>>()
        };
        assert_eq!(trimmed(&f.pass), vec![written() - Duration::minutes(1)]);
        for poll in 0..3 {
            assert_eq!(f.run(&both).read, 0, "poll {poll}");
        }
        assert_eq!(
            trimmed(&f.reopened()),
            vec![written() - Duration::minutes(1)]
        );
    }

    /// The Sessions row and drill for a trimmed row draw no sparkline and
    /// no turns, as for a row read before the per-turn reader; its tokens
    /// are its observed change.
    #[test]
    fn a_trimmed_codex_row_has_no_sparkline_or_turns() {
        let f = Fixture::new();
        let path = f.write("c.jsonl", &codex_series_bytes());
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let listed = |f: &Fixture| {
            f.pass.sessions_value(true, &NeverFolders::NONE, &[], 10)["sessions"][0].clone()
        };
        let drill = |f: &Fixture| {
            let reference = listed(f)["session_ref"].as_str().unwrap().to_string();
            f.pass
                .drill_value(true, &reference, &NeverFolders::NONE, utc())
                .unwrap()["session"]
                .clone()
        };
        // Not vacuous: the untrimmed row draws both.
        assert_eq!(listed(&f)["turns"], 3);
        assert!(!listed(&f)["sparkline"].is_null());
        assert!(drill(&f)["series_unavailable"].is_null());
        let mut store = (*f.pass.load_cached().unwrap().unwrap()).clone();
        trim_codex_series(&mut store, 0, 0);
        f.pass.save(store).unwrap();
        let row = listed(&f);
        assert!(row["turns"].is_null(), "{row}");
        assert!(row["sparkline"].is_null(), "{row}");
        let RowBody::Codex { observed, .. } = only_row(&f.pass).body else {
            panic!("a Codex row");
        };
        assert_eq!(
            row["tokens"],
            serde_json::json!(observed.map(|observed| observed.total)),
            "its observed change, never the trimmed series' tokens"
        );
        assert_eq!(drill(&f)["series_unavailable"], "not_recorded");
    }

    /// The Models answers treat a trimmed series as unknown per-turn data,
    /// never zero: the session is counted in `unknown_facts` on every row.
    #[test]
    fn a_trimmed_codex_row_is_left_out_of_the_model_answers() {
        let f = Fixture::new();
        let path = f.write("c.jsonl", &codex_series_bytes());
        // Placed on the fixture's day, so the session stays in the window
        // once it has no dated turns.
        let at = Utc.with_ymd_and_hms(2026, 1, 6, 0, 0, 0).unwrap();
        let mut session = candidate(SOURCE_CODEX, &path);
        session.started_at = Some(at - Duration::hours(12));
        f.run(&[session]);
        let unknown = |f: &Fixture| {
            let value = f
                .pass
                .model_answers_value(true, &NeverFolders::NONE, false, false, at);
            value["rows"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["left_out"]["unknown_facts"].as_u64().unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            unknown(&f),
            vec![0; 6],
            "a candidate while its series is kept"
        );
        let mut store = (*f.pass.load_cached().unwrap().unwrap()).clone();
        trim_codex_series(&mut store, 0, 0);
        f.pass.save(store).unwrap();
        assert_eq!(unknown(&f), vec![1; 6]);
    }

    /// At the owner's measured 13-week Codex volume (about 167k stored
    /// turns and 161k tool calls, over the shared caps), a pass keeps every
    /// session row and every series, and the week counts every session.
    /// Counts rows, never time.
    #[test]
    fn a_realistic_codex_volume_keeps_every_row_and_series() {
        let f = Fixture::new();
        let key = f.pass.keys.load_or_create().unwrap();
        let mut store = CounterStore::empty(key_fingerprint(&key));
        let synthetic = COUNTER_STORE_MAX_SESSIONS - 1;
        for i in 0..synthetic {
            store.rows.insert(
                synthetic_digest(0x22, i),
                codex_series_row(
                    written() - Duration::seconds(i as i64 + 1),
                    i as u64,
                    82,
                    79,
                ),
            );
        }
        store.next_seq = synthetic as u64;
        let turns: usize = store
            .rows
            .values()
            .map(CounterRow::codex_series_turns)
            .sum();
        let calls: usize = store
            .rows
            .values()
            .map(CounterRow::codex_series_tool_calls)
            .sum();
        // Not vacuous: over the shared caps, under the Codex budget.
        assert!(turns > COUNTER_STORE_MAX_TURNS && calls > COUNTER_STORE_MAX_TOOL_CALLS);
        assert!(turns <= COUNTER_STORE_MAX_CODEX_SERIES_TURNS);
        assert!(calls <= COUNTER_STORE_MAX_CODEX_SERIES_TOOL_CALLS);
        f.pass.save(store).unwrap();

        // One real session read, so the pass writes and enforces the caps.
        let path = f.write("x.jsonl", &codex_bytes());
        assert_eq!(f.run(&[candidate(SOURCE_CODEX, &path)]).read, 1);
        let rows = stored_rows(&f.reopened());
        assert_eq!(rows.len(), COUNTER_STORE_MAX_SESSIONS);
        assert!(rows.iter().all(|row| !is_trimmed(row)));
        let week = f.week(&[]);
        assert_eq!(sessions_of(&week).len(), COUNTER_STORE_MAX_SESSIONS);
        let line = week["rollup"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["source"] == "codex")
            .cloned()
            .unwrap();
        assert_eq!(
            line["tokens"],
            synthetic as u64 * CODEX_SYNTHETIC_TOKENS + 80 + 30
        );
    }

    // Automated sessions (owner decision, 2026-10-20, option (b)). Every
    // fixture here is synthetic.

    /// `codex_bytes` whose `session_meta` names `source`; `None` leaves it
    /// out.
    fn codex_bytes_from(sources: &[Option<serde_json::Value>]) -> Vec<u8> {
        let text = String::from_utf8(codex_bytes()).unwrap();
        let mut lines: Vec<String> = text.lines().skip(1).map(str::to_owned).collect();
        for source in sources.iter().rev() {
            let mut payload =
                serde_json::json!({"id": "PRIVATE-CODEX-ID", "model_provider": "openai"});
            if let Some(source) = source {
                payload["source"] = source.clone();
            }
            lines.insert(
                0,
                serde_json::json!({"type": "session_meta", "payload": payload}).to_string(),
            );
        }
        lines.join("\n").into_bytes()
    }

    fn spawned() -> serde_json::Value {
        serde_json::json!({"subagent": {"thread_spawn": {"depth": 1}}})
    }

    /// The one row a pass makes over `bytes` as a Codex session.
    fn codex_row_of(bytes: &[u8]) -> CounterRow {
        let f = Fixture::new();
        let path = f.write("s.jsonl", bytes);
        assert_eq!(f.run(&[candidate(SOURCE_CODEX, &path)]).read, 1);
        only_row(&f.pass)
    }

    #[test]
    fn a_codex_session_its_harness_spawned_is_automated() {
        for (sources, source) in [
            (vec![Some(spawned())], "spawned"),
            (
                vec![Some(serde_json::json!({"subagent": "review"}))],
                "review",
            ),
            (
                vec![Some(
                    serde_json::json!({"subagent": {"other": "synthetic-reviewer"}}),
                )],
                "other",
            ),
            (
                vec![Some(serde_json::json!({"subagent": {"other": "guardian"}}))],
                "guardian",
            ),
            // A spawned thread followed by its parent's copied
            // `session_meta`: still the agent's, not the person's.
            (
                vec![Some(spawned()), Some(serde_json::json!("vscode"))],
                "spawned then vscode",
            ),
            (
                vec![Some(spawned()), Some(serde_json::json!("cli"))],
                "spawned then cli",
            ),
            (vec![Some(spawned()), Some(spawned())], "spawned twice"),
        ] {
            let row = codex_row_of(&codex_bytes_from(&sources));
            assert_eq!(row.automation, Automation::Automated, "{source}");
            assert!(row.automation_read);
            assert!(row.session_input().automated, "{source}");
            // The counters are read as for any other session.
            assert!(matches!(
                row.body,
                RowBody::Codex {
                    observed: Some(_),
                    ..
                }
            ));
        }
    }

    #[test]
    fn a_codex_session_the_person_started_is_not_automated() {
        for label in ["cli", "vscode", "exec", "mcp"] {
            let row = codex_row_of(&codex_bytes_from(&[Some(serde_json::json!(label))]));
            assert_eq!(row.automation, Automation::Person, "{label}");
            assert!(!row.session_input().automated, "{label}");
        }
    }

    /// Unknown is never automated: no `session_meta` source, Codex's own
    /// `unknown`, or a Claude Code session.
    #[test]
    fn an_undeterminable_session_is_unknown_and_not_automated() {
        for bytes in [
            codex_bytes(),
            codex_bytes_from(&[None]),
            codex_bytes_from(&[Some(serde_json::json!("unknown"))]),
        ] {
            let row = codex_row_of(&bytes);
            assert_eq!(row.automation, Automation::Unknown);
            assert!(row.automation_read);
            assert!(!row.session_input().automated);
        }
        let f = Fixture::new();
        let path = f.write("s.jsonl", &claude_bytes());
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let row = only_row(&f.pass);
        assert_eq!(row.automation, Automation::Unknown);
        assert!(!row.session_input().automated);
    }

    /// A spawned thread stored as unknown under the older facts version (the
    /// rule that read its parent's copied `session_meta` as mixed) is read
    /// once more, though its file is unchanged and its automation was
    /// looked for, and comes back automated.
    #[test]
    fn a_stored_spawned_thread_read_as_unknown_is_read_again_as_automated() {
        let f = Fixture::new();
        let path = f.write(
            "s.jsonl",
            &codex_bytes_from(&[Some(spawned()), Some(serde_json::json!("vscode"))]),
        );
        let candidates = [candidate(SOURCE_CODEX, &path)];
        assert_eq!(f.run(&candidates).read, 1);
        assert_eq!(f.run(&candidates).read, 0);
        let mut store = (*f.pass.load_cached().unwrap().unwrap()).clone();
        for row in store.rows.values_mut() {
            row.facts_version = SESSION_FACTS_VERSION - 1;
            row.automation = Automation::Unknown;
            assert!(row.automation_read);
        }
        f.pass.save(store).unwrap();
        assert_eq!(f.run(&candidates).read, 1);
        let row = only_row(&f.pass);
        assert_eq!(row.automation, Automation::Automated);
        assert_eq!(row.facts_version, SESSION_FACTS_VERSION);
        assert_eq!(f.run(&candidates).read, 0);
    }

    fn codex_body() -> RowBody {
        RowBody::Codex {
            observed: None,
            series: None,
            series_trimmed: false,
        }
    }

    fn classed_row(at: DateTime<Utc>, seq: u64, automation: Automation) -> CounterRow {
        let mut row = synthetic_row(at, seq, codex_body());
        row.automation = automation;
        row
    }

    /// A store full of the person's sessions keeps every one of them, older
    /// automated rows or newer, while the automated rows fit their own
    /// allowance; an unknown row counts as the person's.
    #[test]
    fn automated_rows_do_not_count_toward_the_shared_cap() {
        let mut store = CounterStore::empty(KeyedDigest([0; 32]));
        for i in 0..COUNTER_STORE_MAX_SESSIONS {
            let class = if i % 2 == 0 {
                Automation::Person
            } else {
                Automation::Unknown
            };
            store.rows.insert(
                synthetic_digest(0x11, i),
                classed_row(written() + Duration::seconds(2), i as u64, class),
            );
        }
        for i in 0..8 {
            let at = if i % 2 == 0 {
                written()
            } else {
                written() + Duration::seconds(3)
            };
            store.rows.insert(
                synthetic_digest(0xAA, i),
                classed_row(at, 10_000 + i as u64, Automation::Automated),
            );
        }
        enforce_caps(
            &mut store,
            COUNTER_STORE_MAX_SESSIONS,
            COUNTER_STORE_MAX_TURNS,
            COUNTER_STORE_MAX_TOOL_CALLS,
            COUNTER_STORE_MAX_AUTOMATED_SESSIONS,
            true,
        );
        assert_eq!(store.rows.len(), COUNTER_STORE_MAX_SESSIONS + 8);
        assert!(store.evicted.is_empty());
    }

    #[test]
    fn the_automated_allowance_evicts_the_oldest_automated_rows_first() {
        let mut store = CounterStore::empty(KeyedDigest([0; 32]));
        for i in 0..5u8 {
            store.rows.insert(
                KeyedDigest([0xA0 + i; 32]),
                classed_row(
                    written() + Duration::minutes(i64::from(i)),
                    u64::from(i),
                    Automation::Automated,
                ),
            );
        }
        // Older than every automated row, and not touched by its allowance.
        for i in 0..2u8 {
            store.rows.insert(
                KeyedDigest([0x10 + i; 32]),
                classed_row(
                    written() - Duration::hours(1),
                    10 + u64::from(i),
                    Automation::Person,
                ),
            );
        }
        enforce_caps(&mut store, 2, usize::MAX, usize::MAX, 3, true);
        let kept: Vec<u8> = store.rows.keys().map(|k| k.0[0]).collect();
        assert_eq!(kept, vec![0x10, 0x11, 0xA2, 0xA3, 0xA4]);
        let marks: Vec<(u8, Option<bool>)> = store
            .evicted
            .iter()
            .map(|(k, mark)| (k.0[0], mark.automated))
            .collect();
        assert_eq!(marks, vec![(0xA0, Some(true)), (0xA1, Some(true))]);
    }

    /// Automated marks newer than a person's session do not stop it being
    /// read: only the shared caps' own marks bound the person's sessions.
    /// An automated session's own unchanged mark still keeps it from being
    /// read again.
    #[test]
    fn automated_eviction_marks_do_not_starve_the_persons_sessions() {
        let f = Fixture::new();
        let key = f.pass.keys.load_or_create().unwrap();
        let mut store = CounterStore::empty(key_fingerprint(&key));
        for i in 0..COUNTER_STORE_MAX_SESSIONS {
            store.evicted.insert(
                synthetic_digest(0xEE, i),
                Evicted {
                    size_bytes: 1,
                    modified_at: written() + Duration::seconds(1),
                    automated: Some(true),
                },
            );
        }
        let path = f.write("old-auto.jsonl", &codex_bytes_from(&[Some(spawned())]));
        let auto = candidate(SOURCE_CODEX, &path);
        store.evicted.insert(
            session_key(&key, SOURCE_CODEX, &path.to_string_lossy()),
            Evicted {
                size_bytes: auto.size_bytes,
                modified_at: auto.modified_at,
                automated: Some(true),
            },
        );
        f.pass.save(store).unwrap();
        let path = f.write("old.jsonl", &claude_bytes());
        let person = candidate(SOURCE_CLAUDE_CODE, &path);
        assert_eq!(f.run(&[person.clone(), auto.clone()]).read, 1);
        let row = only_row(&f.pass);
        assert!(matches!(row.body, RowBody::Claude { .. }));
        assert_eq!(f.run(&[person, auto]).read, 0);
    }

    /// A mark written before the automated allowance may name a person's
    /// session that automated rows pushed out: it is dropped, so that
    /// session is read again.
    #[test]
    fn an_eviction_mark_from_before_the_allowance_does_not_bound_reads() {
        let f = Fixture::new();
        let key = f.pass.keys.load_or_create().unwrap();
        let mut store = CounterStore::empty(key_fingerprint(&key));
        store.evicted.insert(
            synthetic_digest(0xEE, 0),
            Evicted {
                size_bytes: 1,
                modified_at: written() + Duration::seconds(1),
                automated: None,
            },
        );
        f.pass.save(store).unwrap();
        let path = f.write("old.jsonl", &claude_bytes());
        assert_eq!(f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]).read, 1);
        let store = f.pass.load_cached().unwrap().unwrap();
        assert!(store.evicted.is_empty(), "the old mark is dropped");
    }

    /// A Codex row stored before the classification is read once more,
    /// within the per-poll budget, and then not again; a Claude Code row is
    /// not read again for it.
    #[test]
    fn a_row_from_before_the_classification_is_read_once_more() {
        let f = Fixture::new();
        let codex = f.write("c.jsonl", &codex_bytes_from(&[Some(spawned())]));
        let claude = f.write("s.jsonl", &claude_bytes());
        let mut claude = candidate(SOURCE_CLAUDE_CODE, &claude);
        claude.modified_at = written() - Duration::minutes(1);
        let candidates = [candidate(SOURCE_CODEX, &codex), claude];
        assert_eq!(f.run(&candidates).read, 2);
        edit_stored_rows(&f, |row| {
            row.remove("automation");
            assert!(row.remove("automation_read").is_some());
        });
        let restarted = f.reopened();
        let rows = stored_rows(&restarted);
        assert!(rows.iter().all(|row| row.automation == Automation::Unknown));
        assert_eq!(
            run_with(&restarted, &candidates, &[], false).read,
            1,
            "the Codex row only"
        );
        let rows = stored_rows(&restarted);
        assert_eq!(rows.iter().filter(|row| row.is_automated()).count(), 1);
        assert_eq!(run_with(&restarted, &candidates, &[], false).read, 0);
    }

    /// The Sessions feed and the Models answers leave an automated session
    /// out; the week's token figures still count it, and the week answer
    /// says how many automated sessions are stored.
    #[test]
    fn automated_sessions_leave_the_sessions_feed_and_the_models_answers() {
        let f = Fixture::new();
        let person = f.write(
            "p.jsonl",
            &codex_bytes_from(&[Some(serde_json::json!("cli"))]),
        );
        let auto = f.write("a.jsonl", &codex_bytes_from(&[Some(spawned())]));
        let mut auto = candidate(SOURCE_CODEX, &auto);
        auto.modified_at = written() - Duration::minutes(1);
        assert_eq!(f.run(&[candidate(SOURCE_CODEX, &person), auto]).read, 2);

        let sessions = f.pass.sessions_value(true, &NeverFolders::NONE, &[], 200);
        assert_eq!(sessions["sessions_stored"], 1);
        assert_eq!(sessions["sessions"].as_array().unwrap().len(), 1);

        // `sessions_stored` means the same in both answers: the person's
        // rows; the week adds how many automated ones are stored beside.
        let week = f.week(&[]);
        assert_eq!(week["sessions_stored"], sessions["sessions_stored"]);
        assert_eq!(week["sessions_stored"], 1);
        assert_eq!(week["automated_stored"], 1);
        let line = week["rollup"]["sources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|line| line["source"] == "codex")
            .cloned()
            .unwrap();
        assert_eq!(line["tokens"], 2 * (80 + 30), "both cost tokens");

        let answers = f
            .pass
            .model_answers_value(true, &NeverFolders::NONE, false, false, now());
        for row in answers["rows"].as_array().unwrap() {
            assert_eq!(row["left_out"]["automated"], 1, "{}", row["question"]);
        }
    }

    /// Analyze & save finds a file only for a listed row: never for an
    /// automated one, though its reference is well formed and its file is
    /// found.
    #[test]
    fn locate_finds_only_a_listed_row() {
        let f = Fixture::new();
        let person = f.write("person.jsonl", &codex_bytes());
        let automated = f.write("automated.jsonl", &codex_bytes_from(&[Some(spawned())]));
        f.run(&[
            candidate(SOURCE_CODEX, &person),
            candidate(SOURCE_CODEX, &automated),
        ]);
        let tag = f.pass.store_tag();
        let found = [
            (SOURCE_CODEX, person.as_path()),
            (SOURCE_CODEX, automated.as_path()),
        ];
        let rows = stored_rows(&f.pass);
        assert_eq!(rows.len(), 2);
        for row in rows {
            let located = f
                .pass
                .locate(&row.listed_reference(tag), &NeverFolders::NONE, &found);
            if row.is_automated() {
                assert_eq!(located, Err(LocateError::Unknown));
            } else {
                assert_eq!(located.unwrap().unwrap().index, 0);
            }
        }
    }

    /// The text a `/goal` carries in the fixtures: Claude's condition, the
    /// command's arguments and the reason, and Codex's objective. None of it
    /// may reach the store, the week's answer or the Sessions answers.
    fn assert_no_goal_text(text: &str) {
        for secret in [
            "PRIVATE-GOAL",
            super::super::insights_codex_goals::tests::OBJECTIVE,
            "make the tests pass",
            "all green",
        ] {
            assert!(!text.contains(secret), "{secret} leaked: {text}");
        }
    }

    #[test]
    fn claude_goals_are_kept_per_session_and_answered_without_their_text() {
        let f = Fixture::new();
        let path = f.write("goals.jsonl", &fixture("claude-slash-goals/session.jsonl"));
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        let row = only_row(&f.pass);
        assert_eq!(
            row.goals,
            vec![
                ClaudeGoal {
                    status: crate::insights::slash_goals::ClaudeGoalStatus::Met,
                    set_at: Some(Utc.with_ymd_and_hms(2026, 9, 14, 8, 59, 59).unwrap()),
                    updated_at: Some(Utc.with_ymd_and_hms(2026, 9, 14, 9, 8, 30).unwrap()),
                    iterations: Some(2),
                    duration_ms: Some(30_065),
                    tokens: Some(5_926),
                },
                ClaudeGoal {
                    status: crate::insights::slash_goals::ClaudeGoalStatus::Open,
                    set_at: Some(Utc.with_ymd_and_hms(2026, 9, 14, 9, 9, 0).unwrap()),
                    updated_at: Some(Utc.with_ymd_and_hms(2026, 9, 14, 9, 9, 0).unwrap()),
                    iterations: None,
                    duration_ms: None,
                    tokens: None,
                },
            ]
        );
        // The goals sit beside the counters; the series still reads.
        assert!(matches!(row.body, RowBody::Claude { .. }), "{:?}", row.body);
        assert_no_goal_text(&f.rows_text());

        let value = f.week(&[]);
        let goals = &value["goals"];
        assert_eq!(goals["met"], 1, "{goals}");
        assert_eq!(goals["total"], 2);
        assert_eq!(
            goals["by_tool"],
            serde_json::json!({"claude_code": {"met": 1, "total": 2}})
        );
        let reference = row.listed_reference(f.pass.store_tag());
        // Newest update first: the goal set last, not checked yet.
        assert_eq!(goals["goals"][0]["status"], "open");
        assert_eq!(goals["goals"][0]["met"], false);
        assert_eq!(
            goals["goals"][1],
            serde_json::json!({
                "tool": "claude_code",
                "status": "met",
                "met": true,
                "tokens": 5_926,
                "time_secs": 30,
                "iterations": 2,
                "folder": null,
                "session_ref": reference,
                "updated_at": "2026-09-14T09:08:30Z",
            })
        );
        assert_no_goal_text(&value.to_string());
        // The reference is the one the Sessions drill takes.
        let drill = f
            .pass
            .drill_value(true, &reference, &NeverFolders::NONE, utc())
            .unwrap();
        assert_eq!(drill["readable"], true, "{drill}");
        assert_no_goal_text(&drill.to_string());
        assert_no_goal_text(
            &f.pass
                .sessions_value(true, &NeverFolders::NONE, &[], 10)
                .to_string(),
        );

        // Another week has none, and a Never folder's session shows none.
        let other = f.week_with(
            Some(monday() + Duration::weeks(1)),
            &[],
            WeekOptions::default(),
        );
        assert_eq!(other["goals"]["total"], 0, "{}", other["goals"]);
        assert_eq!(other["goals"]["goals"], serde_json::json!([]));
        let never = f.week(&[PROJECT.to_string()]);
        assert_eq!(never["goals"]["total"], 0);
    }

    /// A resumed Claude transcript repeats the goal records of the one it
    /// continues: each goal is counted once, as last updated.
    #[test]
    fn a_goal_repeated_in_a_resumed_transcript_counts_once() {
        let f = Fixture::new();
        let bytes = fixture("claude-slash-goals/session.jsonl");
        let first = f.write("first.jsonl", &bytes);
        // The resumed transcript: the same records, and the open goal met.
        let mut resumed = bytes.clone();
        resumed.extend_from_slice(
            serde_json::json!({
                "type": "attachment",
                "timestamp": "2026-09-14T11:00:00.000Z",
                "sessionId": "11111111-2222-4333-8444-PRIVATESESS2",
                "attachment": {"type": "goal_status", "met": true,
                    "condition": "PRIVATE-GOAL-CONDITION-TWO ship the release",
                    "reason": "PRIVATE-GOAL-REASON done", "iterations": 1,
                    "durationMs": 1_000, "tokens": 10},
            })
            .to_string()
            .as_bytes(),
        );
        resumed.push(b'\n');
        let second = f.write("second.jsonl", &resumed);
        let mut later = candidate(SOURCE_CLAUDE_CODE, &second);
        later.size_bytes = resumed.len() as u64;
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &first), later]);
        assert_eq!(stored_rows(&f.pass).len(), 2);
        let value = f.week(&[]);
        let goals = &value["goals"];
        assert_eq!(goals["total"], 2, "{goals}");
        assert_eq!(goals["met"], 2, "the later check wins: {goals}");
        assert_no_goal_text(&value.to_string());
    }

    #[test]
    fn a_row_made_before_goals_were_kept_loads_and_is_read_again() {
        let f = Fixture::new();
        let path = f.write("goals.jsonl", &fixture("claude-slash-goals/session.jsonl"));
        f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
        // As a daemon before goals wrote it: no `goals`, an older version.
        edit_stored_rows(&f, |row| {
            assert!(row.remove("goals").is_some());
            row.insert("facts_version".into(), serde_json::json!(2));
        });
        let reopened = f.reopened();
        let before = reopened.week_value(
            true,
            Some(monday()),
            utc(),
            now(),
            &NeverFolders::NONE,
            WeekOptions::default(),
        );
        assert_eq!(before["readable"], true, "{before}");
        assert_eq!(before["goals"]["total"], 0);
        run_with(
            &reopened,
            &[candidate(SOURCE_CLAUDE_CODE, &path)],
            &[],
            false,
        );
        assert_eq!(only_row(&reopened).goals.len(), 2);
    }

    #[test]
    fn codex_goals_join_their_session_by_thread_and_carry_no_text() {
        let mut f = Fixture::new();
        let thread = "019e4769-7501-7bf3-bf74-b8b5d8e33a9b";
        let path = f.write("codex.jsonl", &codex_bytes_for(thread));
        f.run(&[candidate(SOURCE_CODEX, &path)]);
        let db = f.dir.path().join("goals_1.sqlite");
        let at = |hour| {
            Utc.with_ymd_and_hms(2026, 9, 14, hour, 30, 0)
                .unwrap()
                .timestamp_millis()
        };
        super::super::insights_codex_goals::tests::write_goals_db(
            &db,
            &[
                (thread, "complete", 42_000, 600, at(11)),
                // A thread no row matches: its rollout is gone.
                (
                    "019e0000-0000-7000-8000-000000000000",
                    "paused",
                    1,
                    1,
                    at(11),
                ),
            ],
        );
        // No database configured: no Codex goals.
        assert_eq!(f.week(&[])["goals"]["total"], 0);
        f.pass.set_codex_goals_db_for_test(Some(db.clone()));
        let value = f.week(&[]);
        let goals = &value["goals"];
        assert_eq!(goals["total"], 1, "{goals}");
        assert_eq!(goals["met"], 1);
        let row = only_row(&f.pass);
        assert_eq!(
            goals["goals"][0],
            serde_json::json!({
                "tool": "codex",
                "status": "complete",
                "met": true,
                "tokens": 42_000,
                "time_secs": 600,
                "iterations": null,
                "folder": null,
                "session_ref": row.listed_reference(f.pass.store_tag()),
                "updated_at": "2026-09-14T11:30:00Z",
            })
        );
        assert_no_goal_text(&value.to_string());
        assert_no_goal_text(&f.rows_text());
        assert!(!f.rows_text().contains(thread));
        // A goal last updated in another week is not this week's.
        let next = f.week_with(
            Some(monday() + Duration::weeks(1)),
            &[],
            WeekOptions::default(),
        );
        assert_eq!(next["goals"]["total"], 0);
        // Never written: the database is only read.
        let conn = rusqlite::Connection::open(&db).unwrap();
        let rows: i64 = conn
            .query_row("SELECT count(*) FROM thread_goals", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 2);
    }

    #[test]
    fn a_listed_reference_is_the_short_name_and_the_store_tag() {
        assert_eq!(parse_listed_reference("t0.0"), Some((0, 0)));
        assert_eq!(
            parse_listed_reference("t12.4294967295"),
            Some((12, 4_294_967_295))
        );
        for refused in [
            "t0", "t.1", "t1.", "t-1.2", "t1.+2", "x1.2", "t1.2.3", "T1.2", "t 1.2", "",
        ] {
            assert_eq!(parse_listed_reference(refused), None, "{refused}");
        }
    }
}
