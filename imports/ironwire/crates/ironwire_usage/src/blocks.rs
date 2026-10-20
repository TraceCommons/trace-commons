//! Five-hour session blocks, cut from the ledger.
//!
//! The shape is borrowed from `Maciek-roboblog/Claude-Code-Usage-Monitor`
//! (`core/models.py`, `data/analyzer.py`), which worked it out against real
//! Claude Code transcripts: a window opens on the hour containing its first
//! request, runs five hours, and a long enough silence ends it early. Two
//! details in there are not obvious and are the reason this is a port rather
//! than a rewrite — the start is rounded *down to the hour* (so two people
//! comparing windows agree on where they begin), and a gap of a full session
//! length is recorded as its own block rather than closing the previous one
//! silently, so history has holes in it where the user actually stopped.
//!
//! What differs: the monitor reads Claude Code's own JSONL transcripts and
//! groups by account. IronWire has the ledger, which is every provider it
//! routed to, so blocks are cut **per backend**. A Claude five-hour window and
//! an OpenAI weekly one are not the same pool, and merging them would produce
//! a burn rate for a window that does not exist.

use chrono::{DateTime, Duration, Timelike, Utc};
use ironwire_ledger::Exchange;
use serde::{Deserialize, Serialize};

/// The window Claude Code bills against. Not a number IronWire discovered —
/// it is the provider's, and it is here because the user's own history is
/// only comparable when it is cut the same way theirs is.
pub const DEFAULT_SESSION_HOURS: i64 = 5;

/// Tokens, split the way every provider reports them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCounts {
    /// Uncached input tokens.
    pub input: i64,
    /// Output tokens.
    pub output: i64,
    /// Tokens read from the prompt cache.
    pub cache_read: i64,
    /// Tokens written to the prompt cache.
    pub cache_write: i64,
}

impl TokenCounts {
    /// Every token that counted against the window.
    ///
    /// Cache reads are in here. They are discounted, not free, and a burn rate
    /// that ignored them would understate a long agent session by an order of
    /// magnitude — cache reads are most of what a coding agent sends.
    #[must_use]
    pub const fn total(&self) -> i64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    fn add(&mut self, exchange: &Exchange) {
        self.input += exchange.input_tokens.unwrap_or(0);
        self.output += exchange.output_tokens.unwrap_or(0);
        self.cache_read += exchange.cache_read_tokens.unwrap_or(0);
        self.cache_write += exchange.cache_write_tokens.unwrap_or(0);
    }
}

/// One session window on one backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionBlock {
    /// Stable id: the window's start, and the backend it belongs to.
    pub id: String,
    /// Backend this window belongs to.
    pub backend: String,
    /// When the window opened — the top of the hour containing its first
    /// request.
    pub start: DateTime<Utc>,
    /// When it closes, `start + session length`.
    pub end: DateTime<Utc>,
    /// The last request actually seen in it. `None` for a gap.
    pub last_activity: Option<DateTime<Utc>>,
    /// A stretch of silence at least one session long. Carries no usage, and
    /// is excluded from every statistic — it is here so that history reads as
    /// "nothing happened" rather than as a window that closed early.
    pub is_gap: bool,
    /// Still open.
    pub is_active: bool,
    /// Exchanges recorded in it.
    pub exchanges: i64,
    /// Exchanges whose usage the provider never reported. Their tokens are
    /// missing from `tokens`, so a window with many of them is *understated*,
    /// and the caller has to be able to say so.
    pub without_usage: i64,
    /// Summed tokens.
    pub tokens: TokenCounts,
    /// Summed cost at metered rates — including work a subscription had
    /// already paid for, exactly as [`ironwire_ledger::Summary`] does. It is
    /// "what this would have cost on the meter", never "what you were billed".
    pub cost_usd: f64,
    /// Models seen, first use first.
    pub models: Vec<String>,
}

impl SessionBlock {
    /// Minutes of the window that actually contained work.
    ///
    /// Measured to the last request, not to the wall clock: a window opened
    /// five hours ago and used for ten minutes burned at the ten-minute rate,
    /// and dividing by three hundred would report a tenth of the truth.
    /// Floored at one minute so a single request does not divide by zero.
    #[must_use]
    pub fn active_minutes(&self) -> f64 {
        let end = self.last_activity.unwrap_or(self.end);
        let minutes = (end - self.start).num_seconds() as f64 / 60.0;
        minutes.max(1.0)
    }

    /// Minutes since the window opened, by the wall clock.
    #[must_use]
    pub fn elapsed_minutes(&self, now: DateTime<Utc>) -> f64 {
        ((now - self.start).num_seconds() as f64 / 60.0).max(0.0)
    }

    /// Minutes until it closes. Zero once it has.
    #[must_use]
    pub fn remaining_minutes(&self, now: DateTime<Utc>) -> f64 {
        ((self.end - now).num_seconds() as f64 / 60.0).max(0.0)
    }

    /// Whether this window is worth counting in a statistic: a real, finished
    /// window with usage in it.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        !self.is_gap && !self.is_active && self.tokens.total() > 0
    }
}

/// Cut `exchanges` into session blocks, per backend, oldest first.
///
/// `exchanges` may arrive in any order — the ledger hands them back newest
/// first — so this sorts. `now` decides which windows are still open.
#[must_use]
pub fn build(exchanges: &[Exchange], session: Duration, now: DateTime<Utc>) -> Vec<SessionBlock> {
    if exchanges.is_empty() {
        return Vec::new();
    }
    // Per backend: a Claude five-hour window and an OpenAI one are different
    // pools, and one burn rate across both would describe neither.
    let mut by_backend: std::collections::BTreeMap<&str, Vec<&Exchange>> =
        std::collections::BTreeMap::new();
    for exchange in exchanges {
        by_backend
            .entry(exchange.backend.as_str())
            .or_default()
            .push(exchange);
    }

    let mut blocks = Vec::new();
    for (backend, mut group) in by_backend {
        group.sort_by_key(|e| e.started_at);
        blocks.extend(cut(backend, &group, session));
    }
    blocks.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then_with(|| a.backend.cmp(&b.backend))
    });
    for block in &mut blocks {
        block.is_active = !block.is_gap && block.end > now;
    }
    blocks
}

fn cut(backend: &str, exchanges: &[&Exchange], session: Duration) -> Vec<SessionBlock> {
    let mut blocks: Vec<SessionBlock> = Vec::new();
    let mut current: Option<SessionBlock> = None;

    for exchange in exchanges {
        let needs_new = current.as_ref().is_none_or(|block| {
            exchange.started_at >= block.end
                || block
                    .last_activity
                    .is_some_and(|last| exchange.started_at - last >= session)
        });

        if needs_new && let Some(closed) = current.take() {
            if let Some(gap) = gap_between(&closed, exchange.started_at, session) {
                blocks.push(closed);
                blocks.push(gap);
            } else {
                blocks.push(closed);
            }
        }
        let block = current.get_or_insert_with(|| open(backend, exchange.started_at, session));
        add(block, exchange);
    }
    blocks.extend(current);
    blocks
}

/// Round down to the hour, so two windows opened twenty minutes apart in the
/// same hour are the same window — which is how the provider's own five-hour
/// window behaves.
fn open(backend: &str, at: DateTime<Utc>, session: Duration) -> SessionBlock {
    let start = at
        .with_minute(0)
        .and_then(|t| t.with_second(0))
        .and_then(|t| t.with_nanosecond(0))
        .unwrap_or(at);
    SessionBlock {
        id: format!("{}|{backend}", start.to_rfc3339()),
        backend: backend.to_string(),
        start,
        end: start + session,
        last_activity: None,
        is_gap: false,
        is_active: false,
        exchanges: 0,
        without_usage: 0,
        tokens: TokenCounts::default(),
        cost_usd: 0.0,
        models: Vec::new(),
    }
}

fn add(block: &mut SessionBlock, exchange: &Exchange) {
    block.exchanges += 1;
    if exchange.has_usage() {
        block.tokens.add(exchange);
    } else {
        block.without_usage += 1;
    }
    block.cost_usd += exchange.cost_usd.unwrap_or(0.0);
    block.last_activity = Some(exchange.started_at);
    if let Some(model) = exchange
        .served_model
        .as_deref()
        .or(exchange.requested_model.as_deref())
        && !block.models.iter().any(|m| m == model)
    {
        block.models.push(model.to_string());
    }
}

fn gap_between(
    previous: &SessionBlock,
    next: DateTime<Utc>,
    session: Duration,
) -> Option<SessionBlock> {
    let last = previous.last_activity?;
    (next - last >= session).then(|| SessionBlock {
        id: format!("gap-{}|{}", last.to_rfc3339(), previous.backend),
        backend: previous.backend.clone(),
        start: last,
        end: next,
        last_activity: None,
        is_gap: true,
        is_active: false,
        exchanges: 0,
        without_usage: 0,
        tokens: TokenCounts::default(),
        cost_usd: 0.0,
        models: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{at, exchange};

    fn session() -> Duration {
        Duration::hours(DEFAULT_SESSION_HOURS)
    }

    #[test]
    fn a_window_opens_on_the_hour_containing_its_first_request() {
        // Rounding down is what makes two users' windows comparable, and what
        // matches the provider's own five-hour window.
        let e = exchange("claude-sub", at("2026-08-09T13:37:00Z"));
        let blocks = build(&[e], session(), at("2026-08-09T14:00:00Z"));
        assert_eq!(blocks[0].start, at("2026-08-09T13:00:00Z"));
        assert_eq!(blocks[0].end, at("2026-08-09T18:00:00Z"));
    }

    #[test]
    fn requests_inside_one_window_land_in_one_block() {
        let blocks = build(
            &[
                exchange("claude-sub", at("2026-08-09T13:10:00Z")),
                exchange("claude-sub", at("2026-08-09T15:00:00Z")),
                exchange("claude-sub", at("2026-08-09T17:59:00Z")),
            ],
            session(),
            at("2026-08-09T18:30:00Z"),
        );
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].exchanges, 3);
        assert_eq!(blocks[0].last_activity, Some(at("2026-08-09T17:59:00Z")));
    }

    #[test]
    fn a_request_past_the_window_opens_the_next_one() {
        let blocks = build(
            &[
                exchange("claude-sub", at("2026-08-09T13:10:00Z")),
                exchange("claude-sub", at("2026-08-09T18:05:00Z")),
            ],
            session(),
            at("2026-08-09T19:00:00Z"),
        );
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[1].start, at("2026-08-09T18:00:00Z"));
    }

    #[test]
    fn a_full_session_of_silence_is_recorded_as_a_gap_not_as_usage() {
        // Ported deliberately: without this, history shows a window that
        // closed early and nothing to say the user simply stopped working.
        let blocks = build(
            &[
                exchange("claude-sub", at("2026-08-09T01:00:00Z")),
                exchange("claude-sub", at("2026-08-09T20:00:00Z")),
            ],
            session(),
            at("2026-08-09T21:00:00Z"),
        );
        assert_eq!(blocks.len(), 3);
        assert!(blocks[1].is_gap);
        assert_eq!(blocks[1].tokens.total(), 0);
        assert!(!blocks[1].is_active, "a gap is never the open window");
    }

    #[test]
    fn two_backends_never_share_a_window() {
        // A Claude five-hour window and an OpenAI weekly one are different
        // pools; one burn rate across both would describe neither.
        let blocks = build(
            &[
                exchange("claude-sub", at("2026-08-09T13:10:00Z")),
                exchange("openai-key", at("2026-08-09T13:20:00Z")),
            ],
            session(),
            at("2026-08-09T14:00:00Z"),
        );
        assert_eq!(blocks.len(), 2);
        assert_ne!(blocks[0].backend, blocks[1].backend);
        assert!(blocks.iter().all(|b| b.exchanges == 1));
    }

    #[test]
    fn the_ledgers_newest_first_order_is_not_assumed() {
        let blocks = build(
            &[
                exchange("claude-sub", at("2026-08-09T17:00:00Z")),
                exchange("claude-sub", at("2026-08-09T13:10:00Z")),
            ],
            session(),
            at("2026-08-09T18:00:00Z"),
        );
        assert_eq!(blocks.len(), 1, "got: {blocks:?}");
        assert_eq!(blocks[0].start, at("2026-08-09T13:00:00Z"));
    }

    #[test]
    fn an_exchange_with_no_reported_usage_is_counted_not_zeroed() {
        // Same rule as the ledger: a window whose provider went quiet is
        // understated, and the count is how the caller gets to say so.
        let mut quiet = exchange("claude-sub", at("2026-08-09T13:00:00Z"));
        quiet.input_tokens = None;
        quiet.output_tokens = None;
        quiet.cache_read_tokens = None;
        quiet.cache_write_tokens = None;
        let blocks = build(&[quiet], session(), at("2026-08-09T14:00:00Z"));
        assert_eq!(blocks[0].without_usage, 1);
        assert_eq!(blocks[0].tokens.total(), 0);
    }

    #[test]
    fn an_open_window_is_active_and_a_closed_one_is_not() {
        let blocks = build(
            &[
                exchange("claude-sub", at("2026-08-09T01:00:00Z")),
                exchange("claude-sub", at("2026-08-09T13:00:00Z")),
            ],
            session(),
            at("2026-08-09T14:00:00Z"),
        );
        assert!(!blocks[0].is_active, "01:00–06:00 has closed");
        assert!(blocks.last().expect("a block").is_active);
    }

    #[test]
    fn a_window_used_briefly_is_measured_to_its_last_request() {
        // Dividing a ten-minute burst by the five-hour window would report a
        // thirtieth of the real rate.
        let blocks = build(
            &[
                exchange("claude-sub", at("2026-08-09T13:00:00Z")),
                exchange("claude-sub", at("2026-08-09T13:10:00Z")),
            ],
            session(),
            at("2026-08-09T17:00:00Z"),
        );
        assert!((blocks[0].active_minutes() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn a_window_with_one_request_does_not_divide_by_zero() {
        let blocks = build(
            &[exchange("claude-sub", at("2026-08-09T13:00:00Z"))],
            session(),
            at("2026-08-09T13:01:00Z"),
        );
        assert!((blocks[0].active_minutes() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn cache_reads_count_towards_the_window() {
        // They are discounted, not free — and they are most of what a coding
        // agent sends, so leaving them out would understate a long session by
        // an order of magnitude.
        let counts = TokenCounts {
            input: 12,
            output: 137,
            cache_read: 98_000,
            cache_write: 2_048,
        };
        assert_eq!(counts.total(), 100_197);
    }
}
