//! The local trace ledger.
//!
//! Every exchange IronWire routes is recorded here, on this machine, in
//! `$IRONWIRE_HOME/ledger.sqlite`. Nothing is uploaded (`docs/TRUST.md` §4).
//!
//! The ordering matters: the ledger has to be worth having for a user who will
//! *never* share anything — `ironwire log`, cost attribution, "what did my
//! agent actually send before it did that". If the feature only paid off when
//! uploaded, the incentive would be to nudge people into uploading, and that is
//! how trust in this position gets spent.
//!
//! Bodies are **not** recorded unless `capture.bodies = true`. They contain the
//! user's source code.
#![warn(missing_docs)]

use std::path::Path;
use std::sync::{Arc, Mutex};

pub mod bodies;
pub mod proof;

pub use proof::{ProofCandidate, ProofCounts, ProofRollup, ProofStatus, ProofSummary, Route};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use ironwire_core::confidence::{ConfidenceAggregates, ConfidenceBucket};

/// Price an exchange from its observed token counts.
///
/// Delegates to `ironclaw_common::llm_costs` — the price table every NEAR AI
/// surface already shares. A second copy would drift the moment a provider
/// changes a rate, and a confidently wrong cost is worse than none.
///
/// Returns `None` when nothing was observed, so an unpriced exchange never
/// looks free.
#[must_use]
pub fn price(model: &str, usage: Option<(u32, u32, u32, u32)>) -> Option<f64> {
    let (input, output, cache_read, cache_write) = usage?;
    let cost =
        ironclaw_common::llm_costs::price_usage(model, input, output, cache_read, cache_write);
    cost.total_cost.to_string().parse().ok()
}

/// Failure reading or writing the ledger.
#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    /// SQLite said no.
    #[error("trace ledger: {0}")]
    Sqlite(#[from] rusqlite::Error),
}

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, LedgerError>;

/// One routed exchange.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exchange {
    /// Rowid, once the ledger has stored it. `None` on an exchange that has
    /// been built but not yet recorded.
    ///
    /// Exposed because a timestamp cannot page: two exchanges can share one,
    /// and a cutoff compared inclusively re-reads its own boundary row
    /// forever. This is the tie-breaker a caller advances on -- see
    /// [`Ledger::page`].
    #[serde(default)]
    pub id: Option<i64>,
    /// When the request arrived.
    pub started_at: DateTime<Utc>,
    /// Milliseconds until the first response byte reached the client.
    pub ttfb_ms: Option<i64>,
    /// Milliseconds until the response finished.
    pub total_ms: Option<i64>,
    /// Which façade received it.
    pub facade: String,
    /// Path beneath the façade.
    pub path: String,
    /// Opaque conversation key. Carries no content.
    pub conversation: String,
    /// The agent session this exchange belongs to, as the *client* named it in
    /// a request header IronWire already forwards untouched.
    ///
    /// `conversation` cannot answer this. It is a routing-affinity key --
    /// protocol family, the head of the preamble, the tool list -- deliberately
    /// stable across a whole session, which also makes it stable across two
    /// different sessions with the same tools, and across machines. Useful for
    /// stickiness, useless for saying which session a row came from.
    ///
    /// `None` when the client sent no such header.
    pub client_session_id: Option<String>,
    /// Backend that served it.
    pub backend: String,
    /// Model requested by the client.
    pub requested_model: Option<String>,
    /// Model that actually served it, as the provider reported.
    pub served_model: Option<String>,
    /// The provider's own identifier for this response, when it gave one.
    ///
    /// The handle a provider's own APIs take for this exchange. NEAR AI's
    /// `GET /v1/signature/{id}` returns a receipt signed by the enclave that
    /// served it, over the request and response hashes; without the id that
    /// receipt is unreachable the moment the response is gone. Recording it
    /// costs one string and keeps that door open -- and it stays a local row
    /// like everything else here.
    #[serde(default)]
    pub upstream_id: Option<String>,
    /// The gateway's statement that it substituted a model, verbatim:
    /// `<requested> -> <canonical>`.
    ///
    /// `served_model` cannot answer this. It is read from the response body and
    /// names the canonical model whether that model was asked for or reached
    /// through an alias, so the two cases are indistinguishable in the row that
    /// matters. This is the provider asserting the substitution happened.
    ///
    /// Load-bearing for the receipt story: cloud-api rewrites an aliased
    /// response body to add a `warning` field, which by its own account leaves
    /// the body no longer byte-matching what the backend TD signed. So a row
    /// with a value here is one whose `response_sha256` will not verify against
    /// `GET /v1/signature/{upstream_id}`, and it is better to know that from
    /// the row than to discover it from a failed verification.
    ///
    /// `None` on every provider that does not alias, which is all of them
    /// except NEAR AI today.
    #[serde(default)]
    pub model_alias_resolved: Option<String>,
    /// SHA-256 of the request body **exactly as it went upstream**, hex.
    ///
    /// Half of what a NEAR AI receipt signs (`bodies`). The bytes hashed are
    /// the ones handed to the backend -- after a model override, after the
    /// privacy filter, after translation -- because those are the bytes the
    /// enclave hashed. The client's own bytes would be a different, useless
    /// answer on exactly the routes where a receipt exists.
    ///
    /// `None` when body capture is off, or when the body could not be held
    /// whole: a digest of part of a body is a wrong answer, and a wrong
    /// digest reads as tampering.
    #[serde(default)]
    pub request_sha256: Option<String>,
    /// SHA-256 of the response body **exactly as it came back**, hex.
    ///
    /// The other half. For a streamed response this is the digest of the raw
    /// concatenated event stream, which is what NEAR AI's own verifier hashes
    /// -- not of any reassembled content.
    ///
    /// `None` unless the stream was read to its end. A response that was
    /// cancelled, restarted, or truncated has no honest digest.
    #[serde(default)]
    pub response_sha256: Option<String>,
    /// Where the captured bodies are, under `$IRONWIRE_HOME/bodies`.
    ///
    /// `None` when nothing was captured. See [`bodies::BodyStore`].
    #[serde(default)]
    pub body_ref: Option<String>,
    /// Fidelity rung.
    pub rung: String,
    /// Backends tried and rejected before this one succeeded.
    pub attempts: i64,
    /// Uncached input tokens.
    pub input_tokens: Option<i64>,
    /// Tokens read from the prompt cache.
    pub cache_read_tokens: Option<i64>,
    /// Tokens written to the prompt cache.
    pub cache_write_tokens: Option<i64>,
    /// Output tokens.
    pub output_tokens: Option<i64>,
    /// USD this exchange cost, priced from the observed token counts.
    ///
    /// `None` when the provider reported no usage — the same rule as everywhere
    /// else: a fabricated zero would understate what the user actually spent.
    /// Subscription capacity still carries a price, because "what this would
    /// have cost on the meter" is the number that makes a subscription legible.
    pub cost_usd: Option<f64>,
    /// How many distinct values the privacy filter substituted, when it was on.
    ///
    /// `None` means the filter was off — which is *not* the same as zero, and
    /// the two must not be conflated in a log a user reads to decide whether
    /// the filter is doing anything (`docs/PRIVACY.md` §7).
    pub substitutions: Option<i64>,
    /// HTTP status returned to the client.
    pub status: i64,
    /// Error, when the exchange failed.
    pub error: Option<String>,
    /// How confident the model was, reduced from per-token log-probabilities.
    ///
    /// `None` unless `capture.logprobs` was on, the route was cross-family, and
    /// the backend honoured the request — so on nearly every row. Stored as
    /// four flat columns rather than a blob: a mean nobody can query is a mean
    /// nobody will look at. The distributions themselves are never written
    /// here; they are reduced in the proxy and discarded (`docs/DESIGN.md` §8).
    #[serde(default)]
    pub confidence: Option<ConfidenceAggregates>,
    /// Whether this exchange carries a provider proof. See [`proof`].
    ///
    /// Written as `pending` or `outside` when the exchange is recorded and
    /// settled later, off the response path. `None` on a row written before
    /// the column existed -- which is not the same as `outside`, and is kept
    /// apart from it everywhere it is counted.
    #[serde(default, deserialize_with = "proof::lenient")]
    pub proof: Option<ProofStatus>,
}

impl Exchange {
    /// What a rolling body window is scoped by.
    ///
    /// The agent's own session id when it sent one -- that is the unit
    /// downstream means by "the final call of a session". A client that sends
    /// no session header falls back to the conversation key, which is always
    /// present but is a routing-affinity hash: it is equally stable across two
    /// concurrent sessions that share a tool list, so those two rotate against
    /// each other and one of them can lose its final bodies early.
    ///
    /// That trade is deliberate and it only ever errs one way. Over-merging
    /// deletes *more* than it should, which costs an attestation; the
    /// alternative -- exempting rows with no session id from rotation -- would
    /// keep every exchange's prompts and completions forever for exactly the
    /// clients we know least about.
    #[must_use]
    pub fn retention_key(&self) -> &str {
        self.client_session_id
            .as_deref()
            .unwrap_or(&self.conversation)
    }

    /// Whether the provider reported any usage at all.
    ///
    /// An exchange with no usage is recorded as such rather than as zero: a
    /// fabricated zero would silently understate a user's spend.
    #[must_use]
    pub fn has_usage(&self) -> bool {
        self.input_tokens.is_some()
            || self.output_tokens.is_some()
            || self.cache_read_tokens.is_some()
    }
}

/// Aggregate view over a window, for `ironwire log --summary`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Exchanges in the window.
    pub exchanges: i64,
    /// Exchanges whose usage the provider never reported.
    pub without_usage: i64,
    /// Summed uncached input tokens.
    pub input_tokens: i64,
    /// Summed cache reads.
    pub cache_read_tokens: i64,
    /// Summed cache writes.
    ///
    /// Anthropic bills these above the base input rate, so a breakdown that
    /// omits them cannot explain why a bill moved — which is exactly the
    /// question someone reads this to answer.
    pub cache_write_tokens: i64,
    /// Summed output tokens.
    pub output_tokens: i64,
    /// Summed USD across the window, priced from observed usage.
    pub cost_usd: f64,
    /// Per-backend exchange counts, descending.
    pub by_backend: Vec<(String, i64)>,
    /// Cache reads as a fraction of every prompt token — reads, writes and
    /// uncached input alike.
    ///
    /// Writes belong in the denominator: a turn that wrote 100k tokens and read
    /// none had a 0% hit rate, and leaving writes out would report it as
    /// perfect or as undefined.
    ///
    /// `None`, never `0.0`, when nothing in the window reported usage. A
    /// fabricated zero reads as "your cache is broken" when the truth is "the
    /// provider told us nothing" — the same rule as everywhere else here.
    pub cache_hit_rate: Option<f64>,
    /// Exchanges that wrote to the cache and read nothing: a fresh prefix with
    /// nothing reused. The number that actually diagnoses a thrashing route.
    pub cold_starts: i64,
    /// Cache reads and writes per backend, in the same order as
    /// [`Self::by_backend`].
    ///
    /// A cross-family exchange never reports a write — the translator has no
    /// field to map — so a hit rate over a mixed window compares unlike things.
    /// The per-backend split is what makes the cost of a descent visible.
    pub cache_by_backend: Vec<(String, i64, i64)>,
    /// Per-backend priced cost, in the same order.
    ///
    /// Kept per backend rather than only as a total because the total answers
    /// the wrong question: work done on a subscription is priced here too — as
    /// what it *would* have cost on the metered API — and reporting that as
    /// spend tells a user they were billed for capacity they had already paid
    /// for. Only the caller knows which backend is which.
    pub cost_by_backend: Vec<(String, f64)>,
}

/// The ledger.
///
/// Cloneable and shareable: the connection is behind a mutex because SQLite
/// writes here are tiny, infrequent relative to inference, and always off the
/// response path.
#[derive(Clone)]
pub struct Ledger {
    conn: Arc<Mutex<rusqlite::Connection>>,
    /// Exchanges appended since this process started.
    ///
    /// Exists so a reader can cheaply ask "is anything I derived from this
    /// ledger still current?" without a query. Anything cached off the rows —
    /// the burn rate on `ironwire status` — is stale the moment this moves,
    /// and a time-based expiry alone cannot see that: a request arriving one
    /// millisecond after a report was built would be invisible until the
    /// timer ran out.
    writes: Arc<std::sync::atomic::AtomicU64>,
}

impl std::fmt::Debug for Ledger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ledger").finish_non_exhaustive()
    }
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS exchanges (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at         TEXT    NOT NULL,
    ttfb_ms            INTEGER,
    total_ms           INTEGER,
    facade             TEXT    NOT NULL,
    path               TEXT    NOT NULL,
    conversation       TEXT    NOT NULL,
    client_session_id  TEXT,
    backend            TEXT    NOT NULL,
    requested_model    TEXT,
    served_model       TEXT,
    model_alias_resolved TEXT,
    rung               TEXT    NOT NULL,
    attempts           INTEGER NOT NULL,
    input_tokens       INTEGER,
    cache_read_tokens  INTEGER,
    cache_write_tokens INTEGER,
    output_tokens      INTEGER,
    cost_usd           REAL,
    substitutions      INTEGER,
    status             INTEGER NOT NULL,
    error              TEXT,
    mean_confidence         REAL,
    confidence_variability  REAL,
    confidence_bucket       TEXT,
    confidence_tokens       INTEGER
);
CREATE INDEX IF NOT EXISTS exchanges_started_at ON exchanges (started_at);
CREATE INDEX IF NOT EXISTS exchanges_conversation ON exchanges (conversation);
";

impl Ledger {
    /// Open (and migrate) the ledger at `path`.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] when the file cannot be opened or migrated.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        Self::init(conn)
    }

    /// Open an in-memory ledger. Used by tests.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] when the schema cannot be created.
    pub fn in_memory() -> Result<Self> {
        Self::init(rusqlite::Connection::open_in_memory()?)
    }

    fn init(conn: rusqlite::Connection) -> Result<Self> {
        // WAL so a `ironwire log` read never blocks the daemon's writes.
        let _: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        conn.execute_batch(SCHEMA)?;
        add_missing_columns(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            writes: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        })
    }

    /// Append an exchange.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a write failure. Callers on the response path
    /// must log and continue: a ledger problem must never fail a user's
    /// inference request.
    ///
    /// Returns the new row's id, which is what [`Ledger::supersede_bodies`]
    /// rotates against: rows are ordered by *completion*, and a timestamp
    /// cannot separate two exchanges that finished in the same instant.
    pub fn record(&self, exchange: &Exchange) -> Result<i64> {
        let conn = self.lock();
        conn.execute(
            "INSERT INTO exchanges (
                started_at, ttfb_ms, total_ms, facade, path, conversation, client_session_id,
                backend, requested_model, served_model, upstream_id,
                request_sha256, response_sha256, body_ref, rung, attempts,
                input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
                cost_usd, substitutions, status, error,
                mean_confidence, confidence_variability, confidence_bucket, confidence_tokens,
                model_alias_resolved, proof, proof_attempts
             ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,
                       ?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,0)",
            rusqlite::params![
                exchange.started_at.to_rfc3339(),
                exchange.ttfb_ms,
                exchange.total_ms,
                exchange.facade,
                exchange.path,
                exchange.conversation,
                exchange.client_session_id,
                exchange.backend,
                exchange.requested_model,
                exchange.served_model,
                exchange.upstream_id,
                exchange.request_sha256,
                exchange.response_sha256,
                exchange.body_ref,
                exchange.rung,
                exchange.attempts,
                exchange.input_tokens,
                exchange.cache_read_tokens,
                exchange.cache_write_tokens,
                exchange.output_tokens,
                exchange.cost_usd,
                exchange.substitutions,
                exchange.status,
                exchange.error,
                exchange.confidence.map(|c| f64::from(c.mean_confidence)),
                exchange.confidence.map(|c| f64::from(c.variability)),
                exchange.confidence.map(|c| c.bucket.as_str()),
                exchange
                    .confidence
                    .and_then(|c| i64::try_from(c.token_count).ok()),
                exchange.model_alias_resolved,
                exchange.proof.map(ProofStatus::as_str),
            ],
        )?;
        let id = conn.last_insert_rowid();
        self.writes
            .fetch_add(1, std::sync::atomic::Ordering::Release);
        Ok(id)
    }

    /// Give up the bodies of every earlier exchange in the same session.
    ///
    /// The rolling window: only the most recent exchange of a session keeps
    /// its bodies, so at the end of a session exactly one exchange's bodies
    /// remain -- the final call, which is the only one anything downstream
    /// attests. Retaining every turn's full prompts and completions would hold
    /// the maximum possible amount of the user's content to buy something
    /// nothing uses.
    ///
    /// Returns the references whose files the caller must now unlink. The row
    /// gives up `body_ref` and **keeps both digests**: a hash is not the
    /// content, it is the thing a receipt is checked against, and a row that
    /// still pointed at a file we had deleted would claim a body it cannot
    /// produce. Afterwards such a row reads honestly as "I know what was
    /// hashed; I no longer hold the bytes".
    ///
    /// Scoped by `session`, which is the caller's [`Exchange::client_session_id`]
    /// when the client sent one and its `conversation` key otherwise -- see
    /// [`Exchange::retention_key`].
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a read or write failure.
    pub fn supersede_bodies(&self, session: &str, before: i64) -> Result<Vec<String>> {
        const PREDICATE: &str = "body_ref IS NOT NULL AND id < ?2
             AND COALESCE(client_session_id, conversation) = ?1";
        let mut conn = self.lock();
        // One transaction, under the connection lock the whole ledger shares,
        // so a concurrent `record` cannot land between the read and the update
        // and have its reference dropped without its file being unlinked.
        let tx = conn.transaction()?;
        let superseded = {
            let mut statement =
                tx.prepare(&format!("SELECT body_ref FROM exchanges WHERE {PREDICATE}"))?;
            statement
                .query_map(rusqlite::params![session, before], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.execute(
            &format!("UPDATE exchanges SET body_ref = NULL WHERE {PREDICATE}"),
            rusqlite::params![session, before],
        )?;
        tx.commit()?;
        Ok(superseded)
    }

    /// Every body reference a row still claims.
    ///
    /// The set a sweep keeps; anything else in the store is an orphan. Two
    /// windows produce those, and neither is avoidable by ordering alone: a
    /// crash after the files are written and before the row is inserted, and a
    /// crash after a reference is dropped and before the file is unlinked.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a read failure.
    pub fn live_body_refs(&self) -> Result<std::collections::BTreeSet<String>> {
        let conn = self.lock();
        let mut statement =
            conn.prepare("SELECT body_ref FROM exchanges WHERE body_ref IS NOT NULL")?;
        let refs = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(refs)
    }

    /// How many exchanges have been appended since this process started.
    ///
    /// Not a row count and not persisted — it is a change token. Equal on two
    /// reads means nothing was written between them, which is the only thing a
    /// cache built from these rows needs to know.
    #[must_use]
    pub fn writes(&self) -> u64 {
        self.writes.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Most recent exchanges, newest first.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a read failure.
    pub fn recent(&self, limit: usize) -> Result<Vec<Exchange>> {
        let conn = self.lock();
        let mut statement = conn.prepare(&format!(
            "{COLUMNS} FROM exchanges ORDER BY id DESC LIMIT ?1"
        ))?;
        let rows = statement.query_map([limit], read_exchange)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(LedgerError::from)
    }

    /// Every exchange since `from`, oldest first.
    ///
    /// Unlike [`Ledger::recent`] this is unbounded by count, because the
    /// window is the bound: callers pass a cutoff, not a page size. What reads
    /// it is [`ironwire_usage`](https://docs.rs/ironwire_usage), which cuts
    /// these rows into session windows and cannot do that from a truncated
    /// tail — a missing first request moves the window's start.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a read failure.
    pub fn since(&self, from: DateTime<Utc>) -> Result<Vec<Exchange>> {
        let conn = self.lock();
        let mut statement = conn.prepare(&format!(
            "{COLUMNS} FROM exchanges WHERE started_at >= ?1 ORDER BY started_at ASC"
        ))?;
        let rows = statement.query_map([from.to_rfc3339()], read_exchange)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(LedgerError::from)
    }

    /// One page of the window starting at `from`, oldest first.
    ///
    /// The paging read. `since` exists for a caller that wants the whole
    /// window at once and can hold it; this is for one that reads forward and
    /// must be able to stop.
    ///
    /// Two things a timestamp cutoff alone cannot do, and this can:
    ///
    /// - **Advance.** `after_id` is compared exclusively, so a caller passes
    ///   the last id it saw and never receives that row again. A timestamp
    ///   cutoff is inclusive against a column that is not unique -- two
    ///   exchanges in the same millisecond share one -- so a caller paging on
    ///   time re-reads its own boundary every request, and once `limit`
    ///   exchanges share a timestamp it stops moving entirely.
    /// - **Stay bounded.** `LIMIT` is applied by SQLite rather than by
    ///   truncating a `Vec` afterwards. Truncating means an old cutoff reads
    ///   every matching row into memory while holding the ledger mutex, which
    ///   blocks the daemon's writes for the length of a full-ledger scan on a
    ///   busy install -- and then throws almost all of it away.
    ///
    /// Ordered by `id`, not `started_at`: rowids are assigned in insert order
    /// and are unique, so they are a total order over exactly the rows a
    /// caller has already seen. `started_at` is neither.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a read failure.
    pub fn page(
        &self,
        from: DateTime<Utc>,
        after_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<Exchange>> {
        let conn = self.lock();
        let mut statement = conn.prepare(&format!(
            "{COLUMNS} FROM exchanges
             WHERE started_at >= ?1 AND id > ?2
             ORDER BY id ASC LIMIT ?3"
        ))?;
        let rows = statement.query_map(
            rusqlite::params![
                from.to_rfc3339(),
                after_id.unwrap_or(0),
                i64::try_from(limit).unwrap_or(i64::MAX),
            ],
            read_exchange,
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(LedgerError::from)
    }

    /// Aggregate the last `window`.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a read failure.
    pub fn summary(&self, since: DateTime<Utc>) -> Result<Summary> {
        let conn = self.lock();
        let cutoff = since.to_rfc3339();

        let mut summary: Summary = conn.query_row(
            "SELECT COUNT(*),
                    SUM(CASE WHEN input_tokens IS NULL AND output_tokens IS NULL
                             AND cache_read_tokens IS NULL THEN 1 ELSE 0 END),
                    COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(cache_read_tokens), 0),
                    COALESCE(SUM(cache_write_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(CASE WHEN COALESCE(cache_write_tokens, 0) > 0
                                       AND COALESCE(cache_read_tokens, 0) = 0
                                      THEN 1 ELSE 0 END), 0)
             FROM exchanges WHERE started_at >= ?1",
            [&cutoff],
            |row| {
                Ok(Summary {
                    exchanges: row.get(0)?,
                    without_usage: row.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    input_tokens: row.get(2)?,
                    cache_read_tokens: row.get(3)?,
                    cache_write_tokens: row.get(4)?,
                    output_tokens: row.get(5)?,
                    cost_usd: row.get(6)?,
                    cold_starts: row.get(7)?,
                    // Filled in below, once we know whether anything reported.
                    cache_hit_rate: None,
                    by_backend: Vec::new(),
                    cost_by_backend: Vec::new(),
                    cache_by_backend: Vec::new(),
                })
            },
        )?;

        // One `GROUP BY`, not three: `status` runs this on every render and the
        // menu bar app will poll it.
        let mut statement = conn.prepare(
            "SELECT backend, COUNT(*), COALESCE(SUM(cost_usd), 0.0),
                    COALESCE(SUM(cache_read_tokens), 0),
                    COALESCE(SUM(cache_write_tokens), 0)
             FROM exchanges WHERE started_at >= ?1
             GROUP BY backend ORDER BY COUNT(*) DESC",
        )?;
        let rows = statement
            .query_map([&cutoff], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        summary.by_backend = rows
            .iter()
            .map(|(backend, count, ..)| (backend.clone(), *count))
            .collect();
        summary.cost_by_backend = rows
            .iter()
            .map(|(backend, _, cost, ..)| (backend.clone(), *cost))
            .collect();
        summary.cache_by_backend = rows
            .into_iter()
            .map(|(backend, _, _, reads, writes)| (backend, reads, writes))
            .collect();

        // A rate is shown only when something was actually reported. The sums
        // above coalesce NULL to zero, which is right for a total and wrong for
        // deciding whether there is anything to describe — so the decision uses
        // the counter that distinguishes the two.
        let prompt_tokens =
            summary.input_tokens + summary.cache_read_tokens + summary.cache_write_tokens;
        summary.cache_hit_rate = (summary.exchanges > summary.without_usage && prompt_tokens > 0)
            .then(|| {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "token counts are far below f64's exact-integer range"
                )]
                {
                    summary.cache_read_tokens as f64 / prompt_tokens as f64
                }
            });
        Ok(summary)
    }

    /// Up to `limit` exchanges still waiting for a proof check, oldest first.
    ///
    /// Only rows deferred fewer than `max_attempts` times: the retry budget is
    /// enforced here, in the query, so a row that keeps failing stops being
    /// read at all rather than being read and skipped forever.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a read failure.
    pub fn proof_candidates(&self, limit: usize, max_attempts: u32) -> Result<Vec<ProofCandidate>> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT id, backend, served_model, requested_model, upstream_id,
                    request_sha256, response_sha256, model_alias_resolved,
                    COALESCE(proof_attempts, 0)
             FROM exchanges
             WHERE proof = 'pending' AND COALESCE(proof_attempts, 0) < ?1
             ORDER BY id ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(
            rusqlite::params![
                i64::from(max_attempts),
                i64::try_from(limit).unwrap_or(i64::MAX)
            ],
            |row| {
                Ok(ProofCandidate {
                    id: row.get(0)?,
                    backend: row.get(1)?,
                    served_model: row.get(2)?,
                    requested_model: row.get(3)?,
                    upstream_id: row.get(4)?,
                    request_sha256: row.get(5)?,
                    response_sha256: row.get(6)?,
                    model_alias_resolved: row.get(7)?,
                    attempts: u32::try_from(row.get::<_, i64>(8)?).unwrap_or(u32::MAX),
                })
            },
        )?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(LedgerError::from)
    }

    /// Settle a `pending` exchange's proof status.
    ///
    /// Only ever moves a row out of `pending`, and only into a settled label:
    /// a verdict already written is never overwritten, so two checks racing on
    /// one row cannot leave the weaker answer standing over the stronger one,
    /// and nothing can move a row back to `pending`. Returns whether the row
    /// changed.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a write failure.
    pub fn settle_proof(&self, id: i64, status: ProofStatus) -> Result<bool> {
        if !status.is_settled() {
            return Ok(false);
        }
        let conn = self.lock();
        let changed = conn.execute(
            "UPDATE exchanges SET proof = ?1 WHERE id = ?2 AND proof = 'pending'",
            rusqlite::params![status.as_str(), id],
        )?;
        Ok(changed > 0)
    }

    /// Record that a check could not finish this time, and should be retried.
    ///
    /// Returns the attempts now spent, so the caller can settle the row once
    /// the budget is gone.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a write failure.
    pub fn defer_proof(&self, id: i64) -> Result<u32> {
        let conn = self.lock();
        conn.execute(
            "UPDATE exchanges SET proof_attempts = COALESCE(proof_attempts, 0) + 1
             WHERE id = ?1 AND proof = 'pending'",
            [id],
        )?;
        let attempts: Option<i64> = conn
            .query_row(
                "SELECT proof_attempts FROM exchanges WHERE id = ?1",
                [id],
                |row| row.get(0),
            )
            .ok()
            .flatten();
        Ok(attempts.and_then(|n| u32::try_from(n).ok()).unwrap_or(0))
    }

    /// Calls, cost and proof status per model and backend since `since`.
    ///
    /// One `GROUP BY` in SQL, folded in Rust. The model is the served one,
    /// falling back to the requested one when the provider named none.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a read failure.
    pub fn proof_summary(&self, since: DateTime<Utc>) -> Result<ProofSummary> {
        let conn = self.lock();
        let mut statement = conn.prepare(
            "SELECT COALESCE(served_model, requested_model), backend, proof,
                    COUNT(*),
                    SUM(CASE WHEN cost_usd IS NULL THEN 0 ELSE 1 END),
                    COALESCE(SUM(cost_usd), 0.0)
             FROM exchanges WHERE started_at >= ?1
             GROUP BY 1, 2, 3",
        )?;
        let rows = statement
            .query_map([since.to_rfc3339()], |row| {
                Ok(proof::GroupRow {
                    model: row.get(0)?,
                    backend: row.get(1)?,
                    status: row
                        .get::<_, Option<String>>(2)?
                        .as_deref()
                        .and_then(ProofStatus::parse),
                    calls: row.get(3)?,
                    priced_calls: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
                    cost_usd: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(proof::fold(rows))
    }

    /// Drop records older than `retain`, keeping the file bounded.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a write failure.
    pub fn prune(&self, now: DateTime<Utc>, retain: Duration) -> Result<usize> {
        let conn = self.lock();
        let cutoff = (now - retain).to_rfc3339();
        Ok(conn.execute("DELETE FROM exchanges WHERE started_at < ?1", [cutoff])?)
    }

    /// The body references on rows [`Ledger::prune`] is about to delete.
    ///
    /// Read before the delete, because afterwards there is nothing left to say
    /// which files on disk are orphans. Without this, retention would bound
    /// the ledger and leave the bodies -- the user's source code -- on the
    /// machine forever, which is the opposite of what the setting promises.
    ///
    /// # Errors
    ///
    /// [`LedgerError::Sqlite`] on a read failure.
    pub fn body_refs_before(&self, now: DateTime<Utc>, retain: Duration) -> Result<Vec<String>> {
        let conn = self.lock();
        let cutoff = (now - retain).to_rfc3339();
        let mut statement = conn.prepare(
            "SELECT body_ref FROM exchanges WHERE started_at < ?1 AND body_ref IS NOT NULL",
        )?;
        let refs = statement
            .query_map([cutoff], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(refs)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        match self.conn.lock() {
            Ok(conn) => conn,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

/// Columns added to `exchanges` after the first release, as
/// `(name, definition)`.
///
/// `SCHEMA` is `CREATE TABLE IF NOT EXISTS`, which is a no-op against a ledger
/// that already exists -- so a column added there alone would reach new
/// installs and no upgrade. Every read goes through `COLUMNS`, so a missing one
/// is not a degraded row, it is every query failing. Hence this list, applied
/// on open.
const ADDED_COLUMNS: &[(&str, &str)] = &[
    ("client_session_id", "TEXT"),
    ("upstream_id", "TEXT"),
    ("request_sha256", "TEXT"),
    ("response_sha256", "TEXT"),
    ("body_ref", "TEXT"),
    ("mean_confidence", "REAL"),
    ("confidence_variability", "REAL"),
    ("confidence_bucket", "TEXT"),
    ("confidence_tokens", "INTEGER"),
    ("model_alias_resolved", "TEXT"),
    ("proof", "TEXT"),
    ("proof_attempts", "INTEGER"),
];

/// Add any column in [`ADDED_COLUMNS`] the open ledger does not have.
///
/// Additive and nullable only: this deliberately cannot drop, rename or retype
/// a column, so a downgrade to an older IronWire keeps working against the same
/// file -- it simply does not select the new column.
fn add_missing_columns(conn: &rusqlite::Connection) -> Result<()> {
    let mut statement = conn.prepare("PRAGMA table_info(exchanges)")?;
    let existing: std::collections::BTreeSet<String> = statement
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?;
    drop(statement);
    for (name, definition) in ADDED_COLUMNS {
        if !existing.contains(*name) {
            conn.execute_batch(&format!(
                "ALTER TABLE exchanges ADD COLUMN {name} {definition}"
            ))?;
        }
    }
    Ok(())
}

/// The column list every read shares, in the order [`read_exchange`] expects.
/// Kept in one place because the two are only correct together, and a column
/// added to one and not the other shifts every index after it.
const COLUMNS: &str = "SELECT id, started_at, ttfb_ms, total_ms, facade, path, conversation,
            client_session_id, backend,
            requested_model, served_model, upstream_id,
            request_sha256, response_sha256, body_ref, rung, attempts,
            input_tokens, cache_read_tokens, cache_write_tokens, output_tokens,
            cost_usd, substitutions, status, error,
            mean_confidence, confidence_variability, confidence_bucket, confidence_tokens,
            model_alias_resolved, proof";

/// Rebuild the confidence aggregate from its four columns.
///
/// All four or nothing. A row written before these columns existed has them all
/// null; a partial set means something wrote half an aggregate, and half an
/// aggregate is not a weaker claim than a whole one, it is a different one. An
/// unrecognised bucket name — which is what a *newer* IronWire writing into the
/// same file would leave — is treated the same way, so a downgrade reads an
/// absence rather than a guess.
fn read_confidence(row: &rusqlite::Row<'_>) -> rusqlite::Result<Option<ConfidenceAggregates>> {
    // Indices track [`COLUMNS`], which gained three body columns at 12-14 and
    // pushed every later field along by three.
    let mean: Option<f64> = row.get(25)?;
    let variability: Option<f64> = row.get(26)?;
    let bucket: Option<String> = row.get(27)?;
    let tokens: Option<i64> = row.get(28)?;
    let (Some(mean), Some(variability), Some(bucket), Some(tokens)) =
        (mean, variability, bucket, tokens)
    else {
        return Ok(None);
    };
    let (Some(bucket), Ok(token_count)) =
        (ConfidenceBucket::parse(&bucket), usize::try_from(tokens))
    else {
        return Ok(None);
    };
    Ok(Some(ConfidenceAggregates {
        mean_confidence: mean as f32,
        variability: variability as f32,
        bucket,
        token_count,
    }))
}

fn read_exchange(row: &rusqlite::Row<'_>) -> rusqlite::Result<Exchange> {
    Ok(Exchange {
        id: row.get(0)?,
        started_at: parse_time(&row.get::<_, String>(1)?),
        ttfb_ms: row.get(2)?,
        total_ms: row.get(3)?,
        facade: row.get(4)?,
        path: row.get(5)?,
        conversation: row.get(6)?,
        client_session_id: row.get(7)?,
        backend: row.get(8)?,
        requested_model: row.get(9)?,
        served_model: row.get(10)?,
        upstream_id: row.get(11)?,
        request_sha256: row.get(12)?,
        response_sha256: row.get(13)?,
        body_ref: row.get(14)?,
        rung: row.get(15)?,
        attempts: row.get(16)?,
        input_tokens: row.get(17)?,
        cache_read_tokens: row.get(18)?,
        cache_write_tokens: row.get(19)?,
        output_tokens: row.get(20)?,
        cost_usd: row.get(21)?,
        substitutions: row.get(22)?,
        status: row.get(23)?,
        error: row.get(24)?,
        confidence: read_confidence(row)?,
        // Index 29, after the four confidence columns `read_confidence` takes.
        model_alias_resolved: row.get(29)?,
        proof: row
            .get::<_, Option<String>>(30)?
            .as_deref()
            .and_then(ProofStatus::parse),
    })
}

/// Timestamps are written by us in RFC 3339, so a parse failure means the row
/// was hand-edited. Falling back to the epoch keeps `ironwire log` working
/// instead of failing the whole query over one bad row.
fn parse_time(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| DateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(offset: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + offset, 0).expect("valid timestamp")
    }

    fn exchange(backend: &str, at_secs: i64) -> Exchange {
        Exchange {
            id: None,
            model_alias_resolved: None,
            started_at: at(at_secs),
            ttfb_ms: Some(420),
            total_ms: Some(9_100),
            facade: "anthropic".into(),
            path: "/v1/messages".into(),
            conversation: "c-1".into(),
            client_session_id: None,
            backend: backend.into(),
            requested_model: Some("claude-opus-4-6".into()),
            served_model: Some("claude-opus-4-6".into()),
            upstream_id: None,
            request_sha256: None,
            response_sha256: None,
            body_ref: None,
            rung: "preferred".into(),
            attempts: 1,
            input_tokens: Some(12),
            cache_read_tokens: Some(98_000),
            cache_write_tokens: Some(2_048),
            output_tokens: Some(137),
            cost_usd: Some(0.42),
            substitutions: None,
            status: 200,
            error: None,
            confidence: None,
            proof: None,
        }
    }

    /// The cache is the largest cost lever in a session and the thing the whole
    /// router protects; the aggregate silently dropped the most expensive token
    /// class entirely.
    #[test]
    fn the_summary_accounts_for_cache_writes() {
        let ledger = Ledger::in_memory().expect("opens");
        ledger.record(&exchange("claude-sub", 0)).expect("records");
        let summary = ledger.summary(at(-60)).expect("summarises");
        assert_eq!(summary.cache_write_tokens, 2_048);
        // 98000 / (12 + 98000 + 2048)
        let rate = summary.cache_hit_rate.expect("usage was reported");
        assert!((rate - 0.9795).abs() < 0.001, "got {rate}");
    }

    /// A fabricated 0% reads as "your cache is broken" when the truth is "the
    /// provider told us nothing".
    #[test]
    fn a_window_with_no_reported_usage_has_no_hit_rate() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut silent = exchange("claude-sub", 0);
        silent.input_tokens = None;
        silent.cache_read_tokens = None;
        silent.cache_write_tokens = None;
        silent.output_tokens = None;
        ledger.record(&silent).expect("records");

        let summary = ledger.summary(at(-60)).expect("summarises");
        assert_eq!(summary.cache_hit_rate, None);
        assert_eq!(summary.without_usage, 1);
    }

    /// Distinct from the above, and the distinction is the point: this cache
    /// really is doing nothing, and the user should see that.
    #[test]
    fn an_all_cold_window_reports_zero_rather_than_nothing() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut cold = exchange("claude-sub", 0);
        cold.cache_read_tokens = Some(0);
        cold.cache_write_tokens = Some(50_000);
        ledger.record(&cold).expect("records");

        let summary = ledger.summary(at(-60)).expect("summarises");
        assert_eq!(summary.cache_hit_rate, Some(0.0));
        assert_eq!(summary.cold_starts, 1, "a fresh prefix reusing nothing");
    }

    #[test]
    fn cache_is_reported_per_backend() {
        let ledger = Ledger::in_memory().expect("opens");
        ledger.record(&exchange("claude-sub", 0)).expect("records");
        let mut translated = exchange("nearai", 1);
        // The translator has no field to map, so a cross-family exchange
        // reports reads and never writes.
        translated.cache_write_tokens = Some(0);
        translated.cache_read_tokens = Some(0);
        ledger.record(&translated).expect("records");

        let summary = ledger.summary(at(-60)).expect("summarises");
        let near = summary
            .cache_by_backend
            .iter()
            .find(|(backend, ..)| backend == "nearai")
            .expect("present");
        assert_eq!((near.1, near.2), (0, 0));
    }

    /// An exchange with no prompt tokens at all must contribute to neither side
    /// of the ratio rather than dividing by zero.
    #[test]
    fn an_exchange_with_no_prompt_tokens_cannot_divide_by_zero() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut empty = exchange("claude-sub", 0);
        empty.input_tokens = Some(0);
        empty.cache_read_tokens = Some(0);
        empty.cache_write_tokens = Some(0);
        ledger.record(&empty).expect("records");
        assert_eq!(
            ledger.summary(at(-60)).expect("summarises").cache_hit_rate,
            None
        );
    }

    /// A ledger written by an older IronWire has no `upstream_id` column.
    /// `ADDED_COLUMNS` is what makes opening it work rather than failing every
    /// query, so this opens a pre-existing table and reads through it.
    #[test]
    fn an_older_ledger_gains_the_column_on_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ledger.sqlite");
        {
            let conn = rusqlite::Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE exchanges (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    started_at TEXT NOT NULL,
                    ttfb_ms INTEGER, total_ms INTEGER,
                    facade TEXT NOT NULL, path TEXT NOT NULL,
                    conversation TEXT NOT NULL,
                    backend TEXT NOT NULL,
                    requested_model TEXT, served_model TEXT,
                    rung TEXT NOT NULL, attempts INTEGER NOT NULL,
                    input_tokens INTEGER, cache_read_tokens INTEGER,
                    cache_write_tokens INTEGER, output_tokens INTEGER,
                    cost_usd REAL, substitutions INTEGER, status INTEGER NOT NULL,
                    error TEXT
                )",
            )
            .expect("old schema");
        }
        let ledger = Ledger::open(&path).expect("opens an older ledger");
        assert!(
            ledger.page(at(0), None, 10).expect("reads").is_empty(),
            "an upgraded ledger reads rather than failing every query"
        );
    }

    /// The whole point of reducing log-probabilities in the proxy is that the
    /// four numbers reach a place a person can query. Accumulating them and
    /// never storing them is the failure mode this test exists to prevent.
    #[test]
    fn a_confidence_aggregate_survives_the_round_trip() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut written = exchange("claude-sub", 0);
        written.confidence = Some(ConfidenceAggregates {
            mean_confidence: 0.5,
            variability: 0.25,
            bucket: ConfidenceBucket::Ambiguous,
            token_count: 137,
        });
        ledger.record(&written).expect("records");

        let read = ledger.recent(1).expect("reads");
        let back = read[0].confidence.expect("the aggregate came back");
        assert!((back.mean_confidence - 0.5).abs() < f32::EPSILON);
        assert!((back.variability - 0.25).abs() < f32::EPSILON);
        assert_eq!(back.bucket, ConfidenceBucket::Ambiguous);
        assert_eq!(back.token_count, 137);
    }

    /// Capture is off by default, so this is the overwhelmingly common row. An
    /// absent aggregate must read back absent rather than as a measured zero —
    /// the same rule the token counts and the substitution count already
    /// follow.
    #[test]
    fn an_exchange_with_no_logprobs_stores_no_confidence() {
        let ledger = Ledger::in_memory().expect("opens");
        ledger.record(&exchange("claude-sub", 0)).expect("records");
        assert!(ledger.recent(1).expect("reads")[0].confidence.is_none());
    }

    /// Every column two separate pieces of work added has to survive their
    /// merge, and the positional reader has to be right *against a migrated
    /// file* -- where the physical column order is the order the `ALTER`s ran
    /// in, not the order a fresh `CREATE TABLE` produces. `COLUMNS` is an
    /// explicit select list so the two agree, and this is what says so.
    #[test]
    fn an_upgraded_ledger_gains_every_added_column_and_reads_both_sets_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ledger.sqlite");
        {
            let conn = rusqlite::Connection::open(&path).expect("open");
            conn.execute_batch(
                "CREATE TABLE exchanges (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    started_at TEXT NOT NULL, ttfb_ms INTEGER, total_ms INTEGER,
                    facade TEXT NOT NULL, path TEXT NOT NULL, conversation TEXT NOT NULL,
                    backend TEXT NOT NULL, requested_model TEXT, served_model TEXT,
                    rung TEXT NOT NULL, attempts INTEGER NOT NULL,
                    input_tokens INTEGER, cache_read_tokens INTEGER,
                    cache_write_tokens INTEGER, output_tokens INTEGER,
                    cost_usd REAL, substitutions INTEGER, status INTEGER NOT NULL,
                    error TEXT
                )",
            )
            .expect("pre-column schema");
        }

        let ledger = Ledger::open(&path).expect("opens an older ledger");
        let present: std::collections::BTreeSet<String> = {
            let conn = ledger.lock();
            let mut statement = conn
                .prepare("PRAGMA table_info(exchanges)")
                .expect("pragma");
            statement
                .query_map([], |row| row.get::<_, String>(1))
                .expect("reads")
                .collect::<rusqlite::Result<_>>()
                .expect("collects")
        };
        for (name, _) in ADDED_COLUMNS {
            assert!(
                present.contains(*name),
                "the upgrade did not add `{name}`; every read goes through \
                 COLUMNS, so a missing one fails every query"
            );
        }

        // Both features' values, through the migrated file, read back on the
        // right fields. A positional slip puts one where the other belongs and
        // this is where it shows.
        let mut row = exchange("nearai", 0);
        row.request_sha256 = Some("a".repeat(64));
        row.response_sha256 = Some("b".repeat(64));
        row.body_ref = Some("1700000000000000000-000000".into());
        row.confidence = Some(ConfidenceAggregates {
            mean_confidence: 0.5,
            variability: 0.25,
            bucket: ConfidenceBucket::Ambiguous,
            token_count: 137,
        });
        ledger.record(&row).expect("records");

        let back = ledger.recent(1).expect("reads").remove(0);
        assert_eq!(
            back.request_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(
            back.response_sha256.as_deref(),
            Some("b".repeat(64).as_str())
        );
        assert_eq!(back.body_ref.as_deref(), Some("1700000000000000000-000000"));
        assert_eq!(back.status, 200, "a later column did not shift into status");
        assert_eq!(back.rung, "preferred", "nor into rung");
        let confidence = back.confidence.expect("the aggregate came back");
        assert_eq!(confidence.bucket, ConfidenceBucket::Ambiguous);
        assert_eq!(confidence.token_count, 137);
        assert!((confidence.mean_confidence - 0.5).abs() < f32::EPSILON);
    }

    /// The rolling window, stated as the invariant it exists for: however many
    /// turns a session takes, exactly one of them is still holding bodies.
    #[test]
    fn only_the_most_recent_exchange_of_a_session_keeps_its_bodies() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut ids = Vec::new();
        for turn in 0..5 {
            let mut row = exchange("nearai", turn);
            row.client_session_id = Some("session-a".into());
            row.body_ref = Some(format!("170000000000000000{turn}-00000{turn}"));
            row.request_sha256 = Some(format!("{turn:064}"));
            let id = ledger.record(&row).expect("records");
            let superseded = ledger
                .supersede_bodies(row.retention_key(), id)
                .expect("rotates");
            ids.push((id, superseded));
        }

        // Every turn but the first hands back exactly its predecessor.
        assert!(ids[0].1.is_empty(), "the first turn supersedes nothing");
        for (turn, (_, superseded)) in ids.iter().enumerate().skip(1) {
            assert_eq!(
                superseded,
                &vec![format!("170000000000000000{}-00000{}", turn - 1, turn - 1)],
                "turn {turn} releases only the turn before it"
            );
        }
        let held: Vec<_> = ledger
            .recent(10)
            .expect("reads")
            .into_iter()
            .filter(|row| row.body_ref.is_some())
            .collect();
        assert_eq!(held.len(), 1, "exactly one exchange still holds bodies");
        assert_eq!(
            held[0].body_ref.as_deref(),
            Some("1700000000000000004-000004")
        );
    }

    /// A row that gave up its bodies must not go on claiming them, and must
    /// still say what was hashed. Those two together are what make the state
    /// legible rather than a dangling pointer.
    #[test]
    fn a_superseded_row_keeps_its_digests_and_drops_its_reference() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut first = exchange("nearai", 0);
        first.client_session_id = Some("session-a".into());
        first.body_ref = Some("1700000000000000000-000000".into());
        first.request_sha256 = Some("a".repeat(64));
        first.response_sha256 = Some("b".repeat(64));
        ledger.record(&first).expect("records");

        let mut second = exchange("nearai", 1);
        second.client_session_id = Some("session-a".into());
        second.body_ref = Some("1700000000000000001-000001".into());
        let id = ledger.record(&second).expect("records");
        ledger
            .supersede_bodies(second.retention_key(), id)
            .expect("rotates");

        let rows = ledger.recent(10).expect("reads");
        let older = rows
            .iter()
            .find(|r| r.started_at == at(0))
            .expect("first row");
        assert_eq!(older.body_ref, None, "it no longer claims a body");
        assert_eq!(
            older.request_sha256.as_deref(),
            Some("a".repeat(64).as_str())
        );
        assert_eq!(
            older.response_sha256.as_deref(),
            Some("b".repeat(64).as_str())
        );
    }

    /// Rotation is per session. One agent's turn must never collect another
    /// agent's bodies.
    #[test]
    fn one_session_rotating_leaves_another_sessions_bodies_alone() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut other = exchange("nearai", 0);
        other.client_session_id = Some("session-b".into());
        other.body_ref = Some("1700000000000000000-000000".into());
        ledger.record(&other).expect("records");

        for turn in 1..4 {
            let mut row = exchange("nearai", turn);
            row.client_session_id = Some("session-a".into());
            row.body_ref = Some(format!("170000000000000000{turn}-00000{turn}"));
            let id = ledger.record(&row).expect("records");
            let superseded = ledger
                .supersede_bodies(row.retention_key(), id)
                .expect("rotates");
            assert!(
                !superseded.contains(&"1700000000000000000-000000".to_string()),
                "session-b's bodies were released by session-a's turn {turn}"
            );
        }
        let rows = ledger.recent(10).expect("reads");
        let kept = rows.iter().find(|r| r.started_at == at(0)).expect("row");
        assert_eq!(
            kept.body_ref.as_deref(),
            Some("1700000000000000000-000000"),
            "the other session still holds its own"
        );
    }

    /// A client that sends no session header still rotates, against the
    /// conversation key. The alternative -- exempting it -- would retain
    /// everything forever for exactly the clients we know least about.
    #[test]
    fn a_client_with_no_session_header_still_rotates() {
        let ledger = Ledger::in_memory().expect("opens");
        for turn in 0..3 {
            let mut row = exchange("nearai", turn);
            row.client_session_id = None;
            row.body_ref = Some(format!("170000000000000000{turn}-00000{turn}"));
            assert_eq!(row.retention_key(), "c-1", "falls back to the conversation");
            let id = ledger.record(&row).expect("records");
            ledger
                .supersede_bodies(row.retention_key(), id)
                .expect("rotates");
        }
        let held = ledger
            .recent(10)
            .expect("reads")
            .into_iter()
            .filter(|row| row.body_ref.is_some())
            .count();
        assert_eq!(held, 1);
    }

    /// A crash between writing the files and inserting the row, or between
    /// dropping a reference and unlinking, leaves files nothing points at.
    /// The startup sweep is what collects them.
    #[test]
    fn the_sweep_removes_files_no_row_claims_and_keeps_the_ones_it_does() {
        let home = tempfile::tempdir().expect("tempdir");
        let store = bodies::BodyStore::open(home.path()).expect("store opens");
        let live = store.store(b"kept", b"kept").expect("stored");
        let orphan = store.store(b"orphan", b"orphan").expect("stored");

        let ledger = Ledger::in_memory().expect("opens");
        let mut row = exchange("nearai", 0);
        row.body_ref = Some(live.clone());
        ledger.record(&row).expect("records");

        let removed = store
            .retain_only(&ledger.live_body_refs().expect("live refs"))
            .expect("sweeps");
        assert_eq!(removed, 2, "both files of the orphaned pair");
        assert!(store.read(&live).is_ok(), "the referenced pair survived");
        assert!(store.read(&orphan).is_err(), "the orphan is gone");
    }

    /// Retention has to reach the bodies too. A ledger that pruned its rows
    /// and left the user's source code on disk forever would be a privacy
    /// promise the setting does not keep.
    #[test]
    fn the_bodies_due_for_removal_are_the_ones_on_the_rows_being_pruned() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut old_row = exchange("claude-sub", 0);
        old_row.body_ref = Some("1700000000000000000-000000".into());
        let mut recent = exchange("claude-sub", 100_000);
        recent.body_ref = Some("1700000100000000000-000001".into());
        // A row with capture off must not contribute a reference at all.
        let plain = exchange("claude-sub", 1);
        for row in [&old_row, &recent, &plain] {
            ledger.record(row).expect("records");
        }

        let refs = ledger
            .body_refs_before(at(50_000), Duration::seconds(0))
            .expect("lists");
        assert_eq!(refs, vec!["1700000000000000000-000000".to_string()]);
    }

    /// The digests are the halves of what a receipt signs, so they have to
    /// survive the round trip through SQLite exactly as written.
    #[test]
    fn the_body_digests_survive_the_round_trip() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut row = exchange("nearai", 0);
        row.request_sha256 =
            Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855".into());
        row.response_sha256 =
            Some("2c26b46b68ffc68ff99b453c1d30413413422d706483bfa0f98a5e886266e7ae".into());
        row.body_ref = Some("1700000000000000000-000000".into());
        ledger.record(&row).expect("records");
        let rows = ledger.recent(1).expect("reads");
        assert_eq!(rows[0].request_sha256, row.request_sha256);
        assert_eq!(rows[0].response_sha256, row.response_sha256);
        assert_eq!(rows[0].body_ref, row.body_ref);
    }

    /// The id is what makes a provider's own receipt reachable later, so it
    /// has to survive the round trip rather than being dropped on write.
    #[test]
    fn an_upstream_id_survives_the_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let ledger = Ledger::open(&dir.path().join("ledger.sqlite")).expect("open");
        let mut exchange = exchange("claude-sub", 0);
        exchange.upstream_id = Some("c54961ab1d594cf591e5566caa21196b".into());
        ledger.record(&exchange).expect("records");

        let rows = ledger.page(at(0), None, 10).expect("reads");
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].upstream_id.as_deref(),
            Some("c54961ab1d594cf591e5566caa21196b"),
            "the provider's response id must come back as it went in"
        );
    }

    #[test]
    fn records_round_trip() {
        let ledger = Ledger::in_memory().expect("opens");
        let written = exchange("claude-sub", 0);
        ledger.record(&written).expect("records");
        let read = ledger.recent(10).expect("reads");
        assert_eq!(read.len(), 1);
        // The row comes back with the id SQLite assigned it, which an
        // exchange on its way in does not have. Everything else round-trips
        // unchanged.
        assert!(read[0].id.is_some(), "a stored exchange carries its rowid");
        assert_eq!(
            Exchange {
                id: None,
                ..read[0].clone()
            },
            written
        );
    }

    #[test]
    fn recent_is_newest_first_and_respects_the_limit() {
        let ledger = Ledger::in_memory().expect("opens");
        for i in 0..5 {
            ledger.record(&exchange("claude-sub", i)).expect("records");
        }
        let read = ledger.recent(3).expect("reads");
        assert_eq!(read.len(), 3);
        assert_eq!(read[0].started_at, at(4));
        assert_eq!(read[2].started_at, at(2));
    }

    #[test]
    fn an_exchange_with_no_reported_usage_is_counted_not_zeroed() {
        // Recording a fabricated zero would understate the user's real spend.
        let ledger = Ledger::in_memory().expect("opens");
        let mut unknown = exchange("claude-sub", 0);
        unknown.input_tokens = None;
        unknown.cache_read_tokens = None;
        unknown.cache_write_tokens = None;
        unknown.output_tokens = None;
        unknown.cost_usd = None;
        assert!(!unknown.has_usage());
        ledger.record(&unknown).expect("records");
        ledger.record(&exchange("claude-sub", 1)).expect("records");

        let summary = ledger.summary(at(-1)).expect("summarises");
        assert_eq!(summary.exchanges, 2);
        assert_eq!(summary.without_usage, 1);
        assert_eq!(summary.output_tokens, 137, "only the reported one counts");
    }

    #[test]
    fn the_summary_groups_by_backend() {
        let ledger = Ledger::in_memory().expect("opens");
        for i in 0..3 {
            ledger.record(&exchange("claude-sub", i)).expect("records");
        }
        ledger
            .record(&exchange("anthropic-key", 3))
            .expect("records");

        let summary = ledger.summary(at(-1)).expect("summarises");
        assert!((summary.cost_usd - 4.0 * 0.42).abs() < 1e-9);
        assert_eq!(
            summary.by_backend,
            vec![
                ("claude-sub".to_string(), 3),
                ("anthropic-key".to_string(), 1)
            ]
        );
        assert_eq!(summary.cache_read_tokens, 98_000 * 4);
    }

    #[test]
    fn the_summary_window_excludes_older_records() {
        let ledger = Ledger::in_memory().expect("opens");
        ledger.record(&exchange("claude-sub", 0)).expect("records");
        ledger
            .record(&exchange("claude-sub", 10_000))
            .expect("records");
        let summary = ledger.summary(at(5_000)).expect("summarises");
        assert_eq!(summary.exchanges, 1);
    }

    #[test]
    fn the_write_token_moves_on_a_write_and_not_on_a_read() {
        // What tells a cache built from these rows that it has gone stale. A
        // clock cannot: a request landing a millisecond after a report was
        // built would stay invisible until the timer ran out.
        let ledger = Ledger::in_memory().expect("opens");
        let before = ledger.writes();
        ledger.record(&exchange("claude-sub", 0)).expect("records");
        let after = ledger.writes();
        assert_ne!(before, after);
        let _ = ledger.recent(10).expect("reads");
        let _ = ledger.summary(at(-1)).expect("summarises");
        assert_eq!(ledger.writes(), after, "a read is not a change");
        // Clones share it: the daemon hands copies around.
        assert_eq!(ledger.clone().writes(), after);
    }

    #[test]
    fn a_client_session_id_survives_the_round_trip() {
        let ledger = Ledger::in_memory().expect("in-memory ledger");
        let mut e = exchange("claude-sub", 0);
        e.client_session_id = Some("5db811ed-ce4a-45a7-ab00-56890e111668".into());
        ledger.record(&e).expect("record");
        let back = ledger.recent(1).expect("recent");
        assert_eq!(
            back[0].client_session_id.as_deref(),
            Some("5db811ed-ce4a-45a7-ab00-56890e111668")
        );
    }

    #[test]
    fn a_ledger_written_before_the_column_existed_still_opens() {
        // The upgrade path that `CREATE TABLE IF NOT EXISTS` cannot provide.
        // Every read goes through `COLUMNS`, so without the ALTER this is not
        // a degraded row -- it is every query failing on an existing install.
        let conn = rusqlite::Connection::open_in_memory().expect("sqlite");
        conn.execute_batch(
            "CREATE TABLE exchanges (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                started_at TEXT NOT NULL, ttfb_ms INTEGER, total_ms INTEGER,
                facade TEXT NOT NULL, path TEXT NOT NULL, conversation TEXT NOT NULL,
                backend TEXT NOT NULL, requested_model TEXT, served_model TEXT,
                rung TEXT NOT NULL, attempts INTEGER NOT NULL,
                input_tokens INTEGER, cache_read_tokens INTEGER,
                cache_write_tokens INTEGER, output_tokens INTEGER,
                cost_usd REAL, substitutions INTEGER, status INTEGER NOT NULL, error TEXT
            );",
        )
        .expect("pre-column schema");

        let ledger = Ledger::init(conn).expect("open an older ledger");
        ledger.record(&exchange("claude-sub", 0)).expect("record");
        let back = ledger.recent(1).expect("read back");
        assert_eq!(back.len(), 1, "an upgraded ledger still reads");
        assert!(back[0].client_session_id.is_none());
    }

    /// A *new* reader opening an *old* file, which is the direction a consumer
    /// hits: a pinned `ironwire_ledger` reads a ledger written by whatever
    /// IronWire the user has installed, which is usually older.
    ///
    /// The distinguishing detail is that the row is inserted *before* the
    /// migration runs, so it genuinely predates the column rather than being
    /// written through the new schema. `COLUMNS` names `model_alias_resolved`
    /// in its `SELECT`, so without the `ALTER` this is not a degraded row, it
    /// is every query failing against an existing install.
    #[test]
    fn a_row_written_before_the_alias_column_reads_back_through_the_new_reader() {
        let dir =
            std::env::temp_dir().join(format!("ironwire-alias-compat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("ledger.sqlite3");
        let _ = std::fs::remove_file(&path);

        {
            let conn = rusqlite::Connection::open(&path).expect("sqlite");
            conn.execute_batch(
                "CREATE TABLE exchanges (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    started_at TEXT NOT NULL, ttfb_ms INTEGER, total_ms INTEGER,
                    facade TEXT NOT NULL, path TEXT NOT NULL, conversation TEXT NOT NULL,
                    backend TEXT NOT NULL, requested_model TEXT, served_model TEXT,
                    rung TEXT NOT NULL, attempts INTEGER NOT NULL,
                    input_tokens INTEGER, cache_read_tokens INTEGER,
                    cache_write_tokens INTEGER, output_tokens INTEGER,
                    cost_usd REAL, substitutions INTEGER, status INTEGER NOT NULL, error TEXT
                );
                INSERT INTO exchanges
                    (started_at, facade, path, conversation, backend, requested_model,
                     served_model, rung, attempts, status)
                 VALUES
                    ('2026-09-09T12:00:00+00:00', 'openai', '/v1/chat/completions',
                     'conv-1', 'nearai', 'qwen3-coder', 'qwen3-coder', 'primary', 1, 200);",
            )
            .expect("a ledger from before the column");
        }

        let ledger = Ledger::open(&path).expect("the new reader opens an old file");
        let back = ledger.recent(1).expect("the new SELECT runs against it");
        assert_eq!(back.len(), 1, "the pre-existing row is still readable");
        assert_eq!(back[0].backend, "nearai", "the old row read back intact");
        assert_eq!(
            back[0].model_alias_resolved, None,
            "a row that predates the column has no substitution to report"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_page_never_returns_a_row_the_caller_already_saw() {
        // The boundary bug a timestamp cursor cannot avoid: `since` is
        // inclusive, so paging on the last row's time re-reads that row every
        // request. `after_id` is exclusive.
        let ledger = Ledger::in_memory().expect("in-memory ledger");
        for offset in 0..5 {
            ledger
                .record(&exchange("claude-sub", offset))
                .expect("record");
        }
        let first = ledger.page(at(0), None, 2).expect("first page");
        assert_eq!(first.len(), 2);
        let cursor = first.last().and_then(|e| e.id).expect("rows carry an id");

        let second = ledger.page(at(0), Some(cursor), 2).expect("second page");
        assert_eq!(second.len(), 2);
        assert!(
            second.iter().all(|e| e.id != Some(cursor)),
            "the cursor row must not come back"
        );
    }

    #[test]
    fn paging_advances_even_when_every_exchange_shares_a_timestamp() {
        // The stall a timestamp cursor cannot escape: once `limit` exchanges
        // share one instant, a caller paging on time asks the same question
        // forever. Ids are unique, so this walks the whole window.
        let ledger = Ledger::in_memory().expect("in-memory ledger");
        for _ in 0..5 {
            ledger.record(&exchange("claude-sub", 0)).expect("record");
        }
        let mut seen = Vec::new();
        let mut cursor = None;
        loop {
            let page = ledger.page(at(0), cursor, 2).expect("page");
            if page.is_empty() {
                break;
            }
            cursor = page.last().and_then(|e| e.id);
            seen.extend(page.iter().filter_map(|e| e.id));
            assert!(seen.len() <= 5, "paging must terminate, not loop");
        }
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), 5, "every row seen exactly once");
    }

    #[test]
    fn a_page_is_bounded_by_the_query_not_by_truncating_afterwards() {
        // The whole point of the LIMIT living in SQL: an old cutoff must not
        // read the entire ledger into memory under the mutex first.
        let ledger = Ledger::in_memory().expect("in-memory ledger");
        for offset in 0..50 {
            ledger
                .record(&exchange("claude-sub", offset))
                .expect("record");
        }
        assert_eq!(ledger.page(at(0), None, 3).expect("page").len(), 3);
    }

    #[test]
    fn since_returns_the_whole_window_oldest_first() {
        // Oldest first, and uncapped: session windows are cut from the first
        // request in them, and a truncated tail would move where they start.
        let ledger = Ledger::in_memory().expect("opens");
        for i in [5_000, 0, 10_000, 1_000] {
            ledger.record(&exchange("claude-sub", i)).expect("records");
        }
        let rows = ledger.since(at(1_000)).expect("reads");
        assert_eq!(rows.len(), 3, "the one before the cutoff is excluded");
        assert_eq!(rows[0].started_at, at(1_000));
        assert_eq!(rows[2].started_at, at(10_000));
    }

    #[test]
    fn pruning_bounds_the_file() {
        let ledger = Ledger::in_memory().expect("opens");
        ledger.record(&exchange("claude-sub", 0)).expect("records");
        ledger
            .record(&exchange("claude-sub", 100_000))
            .expect("records");
        let removed = ledger
            .prune(at(100_000), Duration::seconds(50_000))
            .expect("prunes");
        assert_eq!(removed, 1);
        assert_eq!(ledger.recent(10).expect("reads").len(), 1);
    }

    #[test]
    fn a_failed_exchange_records_its_error() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut failed = exchange("claude-sub", 0);
        failed.status = 429;
        failed.error = Some("rate limited".into());
        failed.attempts = 2;
        ledger.record(&failed).expect("records");
        let read = ledger.recent(1).expect("reads");
        assert_eq!(read[0].error.as_deref(), Some("rate limited"));
        assert_eq!(read[0].attempts, 2);
    }

    #[test]
    fn cost_comes_from_the_shared_price_table_and_is_none_without_usage() {
        // A subscription turn still gets a price: "what this would have cost on
        // the meter" is what makes the subscription's value legible.
        let priced =
            price("claude-opus-4-6", Some((12, 137, 98_000, 2_048))).expect("a known model prices");
        assert!(priced > 0.0, "a paid model must not price at zero");
        // Cache reads are discounted, so the same tokens billed fresh cost more.
        let fresh = price("claude-opus-4-6", Some((98_012, 137, 0, 2_048))).expect("prices");
        assert!(
            fresh > priced,
            "cache reads should be cheaper: {fresh} vs {priced}"
        );
        assert_eq!(price("claude-opus-4-6", None), None);
    }

    #[test]
    fn it_survives_a_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ledger.sqlite");
        {
            let ledger = Ledger::open(&path).expect("opens");
            ledger.record(&exchange("claude-sub", 0)).expect("records");
        }
        let ledger = Ledger::open(&path).expect("reopens");
        assert_eq!(ledger.recent(10).expect("reads").len(), 1);
    }

    fn nearai(at_secs: i64) -> Exchange {
        let mut exchange = exchange("nearai", at_secs);
        exchange.served_model = Some("Qwen/Qwen3.6-35B-A3B-FP8".into());
        exchange.upstream_id = Some("c54961ab1d594cf591e5566caa21196b".into());
        exchange.request_sha256 = Some("aa".repeat(32));
        exchange.response_sha256 = Some("bb".repeat(32));
        exchange.proof = Some(ProofStatus::Pending);
        exchange
    }

    #[test]
    fn a_proof_status_survives_the_round_trip() {
        let ledger = Ledger::in_memory().expect("opens");
        ledger.record(&nearai(0)).expect("records");
        let mut outside = exchange("claude-sub", 1);
        outside.proof = Some(ProofStatus::Outside);
        ledger.record(&outside).expect("records");

        let rows = ledger.page(at(0), None, 10).expect("reads");
        assert_eq!(rows[0].proof, Some(ProofStatus::Pending));
        assert_eq!(rows[1].proof, Some(ProofStatus::Outside));
    }

    /// A row from before the column existed must read as "no status", never
    /// as `outside` -- that would paint every old NEAR AI call red.
    #[test]
    fn a_row_without_a_status_reads_as_absent() {
        let ledger = Ledger::in_memory().expect("opens");
        ledger.record(&exchange("nearai", 0)).expect("records");
        assert_eq!(ledger.recent(1).expect("reads")[0].proof, None);
    }

    /// What a newer IronWire might write must not fail an older reader.
    #[test]
    fn an_unrecognised_label_reads_as_absent() {
        let ledger = Ledger::in_memory().expect("opens");
        let id = ledger.record(&nearai(0)).expect("records");
        ledger
            .lock()
            .execute("UPDATE exchanges SET proof = 'quantum' WHERE id = ?1", [id])
            .expect("writes");
        assert_eq!(ledger.recent(1).expect("reads")[0].proof, None);
        let json = r#"{"proof":"quantum"}"#;
        #[derive(serde::Deserialize)]
        struct Row {
            #[serde(default, deserialize_with = "proof::lenient")]
            proof: Option<ProofStatus>,
        }
        let row: Row = serde_json::from_str(json).expect("parses leniently");
        assert_eq!(row.proof, None);
    }

    #[test]
    fn only_pending_rows_are_candidates() {
        let ledger = Ledger::in_memory().expect("opens");
        let pending = ledger.record(&nearai(0)).expect("records");
        let mut outside = exchange("claude-sub", 1);
        outside.proof = Some(ProofStatus::Outside);
        ledger.record(&outside).expect("records");
        ledger.record(&exchange("nearai", 2)).expect("legacy row");

        let candidates = ledger.proof_candidates(10, 5).expect("reads");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].id, pending);
        assert_eq!(candidates[0].attempts, 0);
        assert_eq!(
            candidates[0].upstream_id.as_deref(),
            Some("c54961ab1d594cf591e5566caa21196b")
        );
    }

    /// A verdict is written once. A second, weaker one racing in behind it
    /// must not replace it, and nothing may reopen a settled row.
    #[test]
    fn a_settled_status_is_never_overwritten() {
        let ledger = Ledger::in_memory().expect("opens");
        let id = ledger.record(&nearai(0)).expect("records");
        assert!(
            ledger
                .settle_proof(id, ProofStatus::Verified)
                .expect("settles")
        );
        assert!(!ledger.settle_proof(id, ProofStatus::Failed).expect("no-op"));
        assert!(
            !ledger
                .settle_proof(id, ProofStatus::Pending)
                .expect("no-op")
        );
        assert_eq!(
            ledger.recent(1).expect("reads")[0].proof,
            Some(ProofStatus::Verified)
        );
        assert!(ledger.proof_candidates(10, 5).expect("reads").is_empty());
    }

    /// An outside row is settled from the moment it is written.
    #[test]
    fn an_outside_row_cannot_be_promoted() {
        let ledger = Ledger::in_memory().expect("opens");
        let mut outside = exchange("claude-sub", 0);
        outside.proof = Some(ProofStatus::Outside);
        let id = ledger.record(&outside).expect("records");
        assert!(
            !ledger
                .settle_proof(id, ProofStatus::Verified)
                .expect("no-op")
        );
    }

    #[test]
    fn the_retry_budget_is_enforced_by_the_query() {
        let ledger = Ledger::in_memory().expect("opens");
        let id = ledger.record(&nearai(0)).expect("records");
        assert_eq!(ledger.defer_proof(id).expect("defers"), 1);
        assert_eq!(ledger.defer_proof(id).expect("defers"), 2);
        assert_eq!(ledger.proof_candidates(10, 3).expect("reads").len(), 1);
        assert_eq!(ledger.proof_candidates(10, 2).expect("reads").len(), 0);
        assert_eq!(
            ledger.proof_candidates(10, 3).expect("reads")[0].attempts,
            2
        );
    }

    #[test]
    fn the_proof_summary_groups_by_model_and_backend() {
        let ledger = Ledger::in_memory().expect("opens");
        let a = ledger.record(&nearai(0)).expect("records");
        ledger.record(&nearai(1)).expect("records");
        ledger
            .settle_proof(a, ProofStatus::Verified)
            .expect("settles");
        let mut outside = exchange("claude-sub", 2);
        outside.proof = Some(ProofStatus::Outside);
        outside.cost_usd = None;
        ledger.record(&outside).expect("records");
        // Before the window: not counted.
        ledger.record(&nearai(-100)).expect("records");

        let summary = ledger.proof_summary(at(-10)).expect("summarises");
        assert_eq!(summary.routed.calls, 2);
        assert_eq!(summary.routed.proof.verified, 1);
        assert_eq!(summary.routed.proof.pending, 1);
        assert!((summary.routed.cost_usd - 0.84).abs() < 1e-9);
        assert_eq!(summary.outside.calls, 1);
        assert_eq!(
            summary.outside.priced_calls, 0,
            "an unpriced call is not free"
        );
        assert_eq!(summary.outside.cost_usd, 0.0);
        let near = &summary.groups[0];
        assert_eq!(near.backend, "nearai");
        assert_eq!(near.model.as_deref(), Some("Qwen/Qwen3.6-35B-A3B-FP8"));
        assert_eq!(near.route, Route::Routed);
        assert_eq!(near.work_kind, None);
    }

    /// Opening an older file adds the two columns; its rows read as absent.
    #[test]
    fn an_older_ledger_gains_the_proof_columns_on_open() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("ledger.sqlite");
        {
            let ledger = Ledger::open(&path).expect("open");
            ledger.record(&exchange("nearai", 0)).expect("records");
            let conn = ledger.lock();
            conn.execute_batch(
                "ALTER TABLE exchanges DROP COLUMN proof;
                 ALTER TABLE exchanges DROP COLUMN proof_attempts;",
            )
            .expect("simulate an older file");
        }
        let ledger = Ledger::open(&path).expect("reopens");
        assert_eq!(ledger.recent(1).expect("reads")[0].proof, None);
        assert!(ledger.proof_candidates(10, 5).expect("reads").is_empty());
    }
}

/// Leased token capture evidence.
pub mod token_spool;
