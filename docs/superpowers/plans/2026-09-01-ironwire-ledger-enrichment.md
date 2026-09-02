# IronWire Ledger Enrichment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Attach the cost, token and routing data IronWire records for each inference hop to the traces our own scrapers already build, as `RoutingDecision` events on the existing envelope.

**Architecture:** IronWire uploads nothing. The contributor daemon reads IronWire's existing loopback control API (`GET /_ironwire/log`), caches the rows, and a decorating `TraceSource` joins them onto each loaded session by the client session id both sides now record. One envelope producer stays: ours.

**Tech Stack:** Rust, `reqwest` (already a direct dependency of the contributor crate, `json` feature — **no new dependency is needed or permitted here**), `serde`, `chrono`, `tokio`.

**Spec:** `docs/superpowers/specs/2026-09-01-ironwire-ledger-enrichment-design.md` — read it before starting. This plan argues from it and does not restate its reasoning.

## Global Constraints

- **No new dependencies.** `reqwest` with `json` + `rustls-tls-native-roots` is already declared in `crates/trace-commons-contributor/Cargo.toml`. Do not add a crate, do not add a feature, do not vendor anything. If you believe you need one, stop and ask.
- **No IronWire or Ironclaw crates as dependencies.** Talk to IronWire over HTTP and parse JSON. `ironwire_ledger` itself pulls `ironclaw_common`; nothing from that tree may enter this repo. See `CLAUDE.md`: "There is **no Ironclaw path dependency**."
- **Licence boundary.** `trace-commons-contributor` and `trace-commons-protocol` are `MIT OR Apache-2.0` and ship inside proprietary harnesses. Never add `trace-commons-server`, `-gate-api` or `-gate-enclave` to their dependencies. `crates/trace-commons-server/tests/license_boundary.rs` enforces this; do not edit its expected sets.
- **Attribution only.** Everything this plan adds to an envelope is corpus metadata. It must never reach a gate, a scoring input, a credit computation, or a tenant-scoping decision. Task 6 asserts this.
- **Hash-only logging.** No raw URLs, tokens, paths or contributor identity in log strings.
- **No emojis** in code, commits, or PRs.
- **Verification command for every task** (plain `cargo check` does not apply `-D warnings`; CI does):
  ```bash
  RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
  RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run
  cargo clippy -p trace-commons-contributor --all-targets -- \
    -A clippy::type_complexity -A clippy::collapsible_if \
    -A clippy::manual_option_as_slice -A clippy::useless_vec \
    -A clippy::redundant_pattern_matching
  cargo fmt --all
  ```
- **Task 1 changes the protocol crate**, so it must additionally run `cargo test --workspace` — a protocol envelope change moves a digest pinned in the contributor crate. Testing one crate will not catch it.

## Upstream status

The join key depends on IronWire recording `client_session_id`, which is `nearai/ironwire` PR #17 (open, not merged as of writing). **Nothing in this plan blocks on it.** Every task is tested against an in-memory fake ledger. When #17 ships, the only change is that a real IronWire starts returning the field.

## File Structure

| File | Responsibility |
|---|---|
| `crates/trace-commons-protocol/src/trace_contribution.rs` (modify) | `routing_metadata` presence category; `routing_metadata_included` consent flag; presence derivation routes `RoutingDecision` payloads away from `tool_payloads` |
| `crates/trace-commons-contributor/src/routing/mod.rs` (create) | `RoutedExchange` (the row shape we consume), `RoutingLedger` trait, module docs |
| `crates/trace-commons-contributor/src/routing/ironwire.rs` (create) | `IronWireLedger`: control-API client, snapshot cache, `refresh()` |
| `crates/trace-commons-contributor/src/routing/enriched.rs` (create) | `RoutingEnrichedSource`: the decorating `TraceSource` |
| `crates/trace-commons-contributor/src/source/mod.rs` (modify) | `SessionTranscript.routing`; `all_sources` accepts an optional ledger |
| `crates/trace-commons-contributor/src/envelope.rs` (modify) | `declared_content_presence` mirrors the protocol; `raw_events_for` emits `RoutingDecision` events |
| `crates/trace-commons-contributor/src/daemon/settings.rs` (modify) | `IronWireDeclaration` tri-state, defaulting to off |
| `crates/trace-commons-contributor/src/lib.rs` (modify) | `pub mod routing;` |

### One design decision the spec left implicit

`TraceSource::load` is **synchronous** (`source/mod.rs:148-152`). An HTTP call cannot happen inside it without blocking a runtime thread.

So `RoutingLedger::exchanges_since` is sync and reads an **in-memory snapshot**. `IronWireLedger` separately exposes `async fn refresh()`, which the daemon calls on a timer and the CLI calls once before building sources. This keeps I/O entirely off the load path — which is what the spec's "off any submission-critical path" requires — and makes every test a pure function over a fixture.

---

### Task 1: A routing-metadata presence category

Without this, a `RoutingDecision` carrying `{"backend":"nearai",...}` sets `tool_payloads`, pushing envelopes to Medium residual risk and quarantining them on a default deployment for payloads they do not carry. `include_tool_payloads` has never been true anywhere in this project. Do this first; nothing else is safe to ship without it.

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs:187-217` (`ConsentMetadata`), `:5252-5258` (`EnvelopeContentPresence`), `:5309-5313` (payload branch of the derivation)
- Modify: `crates/trace-commons-contributor/src/envelope.rs:707-745` (`declared_content_presence`), `:544` (its caller)
- Modify: `crates/trace-commons-contributor/src/daemon/preview.rs:1264` (pinned digest)

**Interfaces:**
- Produces: `EnvelopeContentPresence { message_text, tool_payloads, correction, routing_metadata }`; `ConsentMetadata.routing_metadata_included: bool`; `declared_content_presence(&[RawTraceContributionEvent]) -> DeclaredPresence` where `DeclaredPresence { message_text: bool, tool_payloads: bool, routing_metadata: bool }`

- [ ] **Step 1: Write the failing test**

In `crates/trace-commons-protocol/src/trace_contribution.rs`, in the existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn routing_metadata_is_not_declared_as_a_tool_payload() {
    // A routing overlay is numbers and labels about an inference hop. It is
    // not a tool payload, and declaring it as one floors the envelope at
    // Medium residual risk and quarantines it on a default deployment for
    // content it does not carry.
    let mut envelope = sample_envelope();
    envelope.events = vec![TraceContributionEvent {
        event_id: Uuid::new_v4(),
        parent_event_id: None,
        event_type: TraceContributionEventType::RoutingDecision,
        timestamp: Utc::now(),
        redacted_content: None,
        structured_payload: serde_json::json!({"backend": "nearai", "rung": "same_model"}),
        tool_name: None,
        tool_category: None,
        tool_call_id: None,
        latency_ms: Some(1200),
        token_counts: None,
        cost_usd: None,
        success: None,
        failure_modes: Vec::new(),
        side_effect: SideEffectLevel::None,
    }];

    let presence = derive_envelope_content_presence(&envelope);
    assert!(presence.routing_metadata, "declared as routing metadata");
    assert!(!presence.tool_payloads, "NOT declared as a tool payload");
    assert!(!presence.message_text);
}

#[test]
fn a_tool_result_payload_is_still_a_tool_payload() {
    // The regression guard for the change above: routing must be carved out
    // without loosening the rule for everything else.
    let mut envelope = sample_envelope();
    envelope.events = vec![TraceContributionEvent {
        event_id: Uuid::new_v4(),
        parent_event_id: None,
        event_type: TraceContributionEventType::ToolResult,
        timestamp: Utc::now(),
        redacted_content: None,
        structured_payload: serde_json::json!({"stdout": "hello"}),
        tool_name: Some("Bash".to_string()),
        tool_category: None,
        tool_call_id: None,
        latency_ms: None,
        token_counts: None,
        cost_usd: None,
        success: Some(true),
        failure_modes: Vec::new(),
        side_effect: SideEffectLevel::None,
    }];

    let presence = derive_envelope_content_presence(&envelope);
    assert!(presence.tool_payloads);
    assert!(!presence.routing_metadata);
}
```

If no `sample_envelope()` helper exists in that test module, find the existing helper the neighbouring tests use to build a `TraceContributionEnvelope` and use that name instead. Do not write a new fixture builder.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-protocol routing_metadata_is_not_declared -- --nocapture
```

Expected: FAIL — `no field 'routing_metadata' on type 'EnvelopeContentPresence'`.

- [ ] **Step 3: Add the presence field**

`crates/trace-commons-protocol/src/trace_contribution.rs`, replacing the `EnvelopeContentPresence` body at `:5252-5258`:

```rust
pub struct EnvelopeContentPresence {
    pub message_text: bool,
    pub tool_payloads: bool,
    /// A contributor-authored correction. Its own class rather than part of
    /// `message_text`: see [`ConsentMetadata::correction_included`].
    pub correction: bool,
    /// Routing and cost metadata about the inference hops that produced the
    /// session -- which backend served a turn, what it cost, how long it took.
    ///
    /// Its own class because it is neither prose nor a tool payload. Folding it
    /// into `tool_payloads` would floor every enriched envelope at Medium
    /// residual risk and quarantine it on a default deployment for payloads it
    /// does not carry, and `tool_payloads_included` has never been true
    /// anywhere in this project -- so the fold would also silently change what
    /// consent an envelope declares.
    pub routing_metadata: bool,
}
```

- [ ] **Step 4: Route the payload branch by event type**

Replace the payload branch of the derivation at `:5309-5313`:

```rust
        // A marker is not a payload: `{"has_result": true}` says a result
        // existed upstream and carries none of it. See
        // `payload_carries_readable_content`.
        if payload_carries_readable_content(&event.structured_payload) {
            match event.event_type {
                // A routing overlay's payload is the backend, the rung and the
                // model pair -- labels about the hop, never content from it.
                TraceContributionEventType::RoutingDecision => {
                    presence.routing_metadata = true;
                }
                _ => presence.tool_payloads = true,
            }
        }
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-protocol routing_metadata_is_not_declared
cargo test -p trace-commons-protocol a_tool_result_payload_is_still
```

Expected: both PASS.

- [ ] **Step 6: Add the consent declaration**

In `ConsentMetadata` at `:187-217`, after `correction_included`:

```rust
    /// Whether the envelope carries routing and cost metadata about the
    /// inference hops that produced the session.
    ///
    /// A fourth content class. Unlike `correction_included` it does NOT enrol
    /// the trace in the PII backstop hold and does not floor residual risk:
    /// the class is numbers and labels -- a backend id, a rung, a token count,
    /// a price -- and carries no prose from the session.
    ///
    /// `#[serde(default)]` because every envelope submitted before this field
    /// existed omits it, and those envelopes carry no routing metadata:
    /// nothing could set one. `false` is the correct reading of their silence,
    /// not a guess.
    #[serde(default)]
    pub routing_metadata_included: bool,
```

Then fix every construction site the compiler names. Run `RUSTFLAGS="-D warnings" cargo check --workspace` and work through the list; all existing sites take `false`.

- [ ] **Step 7: Mirror the derivation on the client**

`crates/trace-commons-contributor/src/envelope.rs`. Replace the signature and return of `declared_content_presence` at `:707`:

```rust
/// What the built events actually carry, as three independent content classes.
///
/// A struct rather than a tuple of three bools: the call site at the envelope
/// builder reads them apart, and three positional bools is exactly the shape
/// that silently swaps two of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct DeclaredPresence {
    message_text: bool,
    tool_payloads: bool,
    routing_metadata: bool,
}

fn declared_content_presence(events: &[RawTraceContributionEvent]) -> DeclaredPresence {
```

Inside, replace the two accumulator locals with a `let mut presence = DeclaredPresence::default();`, set `presence.message_text` / `presence.tool_payloads` in the existing match arms, and replace the structured-payload branch with:

```rust
        // Must agree with `derive_envelope_content_presence` in the protocol
        // crate. If the client declares honestly and the server then corrects
        // that declaration upward, the contributor is penalised for telling
        // the truth. `the_two_content_derivations_agree` pins this.
        if trace_commons_protocol::trace_contribution::payload_carries_readable_content(
            &event.structured_payload,
        ) {
            match event.event_type {
                TraceContributionEventType::RoutingDecision => presence.routing_metadata = true,
                _ => presence.tool_payloads = true,
            }
        }
```

and return `presence`.

- [ ] **Step 8: Update the caller**

`crates/trace-commons-contributor/src/envelope.rs:544`:

```rust
    let presence = declared_content_presence(&events);
```

Then, in the `ConsentMetadata` literal built below it, replace `message_text_included: message_text_included` and `tool_payloads_included: tool_payloads_included` with `presence.message_text` and `presence.tool_payloads`, and add `routing_metadata_included: presence.routing_metadata`.

- [ ] **Step 9: Run the concordance test**

```bash
cargo test -p trace-commons-contributor the_two_content_derivations_agree
```

Expected: PASS. If it fails, the two derivations have diverged — fix the client to match the protocol, never the reverse.

- [ ] **Step 10: Move the pinned envelope digest**

Adding a serialized consent field changes the envelope digest. Run:

```bash
cargo test -p trace-commons-contributor --lib daemon::preview 2>&1 | grep -A 3 "envelope_digest"
```

Take the **actual** value from the failure and replace the pinned constant at `crates/trace-commons-contributor/src/daemon/preview.rs:1264`.

**Before you do:** confirm the `residual_risk == "medium"` assertion above it still passes unchanged. That ordering exists precisely so a digest move can be told apart from a classification change. If residual risk also moved, stop — this task has done something it should not have.

Add a line to the comment block above the assertion:

```rust
        // Moved again by `routing_metadata_included`: a new consent field
        // serializes into the envelope, so the digest changes while the
        // classification does not.
```

- [ ] **Step 11: Re-export `Decimal` for the contributor crate**

`TraceContributionEvent.cost_usd` is `Option<Decimal>`, and the contributor
crate does **not** depend on `rust_decimal` — so Task 4 cannot construct one
without adding a dependency, which this plan forbids. The protocol crate
already depends on it (`Cargo.toml:17`), so expose it rather than duplicating
it.

In `crates/trace-commons-protocol/src/trace_contribution.rs`, beside the
existing `use rust_decimal::Decimal;` at `:18`:

```rust
/// Re-exported so a consumer can build a `cost_usd` without taking its own
/// dependency on `rust_decimal`. The permissive client crates ship inside
/// third-party harnesses, and every direct dependency they gain is one their
/// vendor inherits.
pub use rust_decimal::Decimal;
```

- [ ] **Step 12: Verify the workspace**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check --workspace
cargo test --workspace 2>&1 | tail -30
```

Expected: no failures. Compare against a baseline captured **before** this task started — this repo has pre-existing PostgreSQL-dependent failures when a database is configured, and those are not yours.

- [ ] **Step 13: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs \
        crates/trace-commons-contributor/src/envelope.rs \
        crates/trace-commons-contributor/src/daemon/preview.rs
git commit -m "Declare routing metadata as its own content class

A routing overlay is neither prose nor a tool payload. Folding it into
tool_payloads would floor every enriched envelope at Medium residual risk
and quarantine it on a default deployment for payloads it does not carry
-- and tool_payloads_included has never been true anywhere here, so the
fold would also silently change what consent an envelope declares.

The pinned preview digest moves because a new consent field serializes.
The residual-risk assertion above it is unchanged, which is what says
this is a declaration change and not a classification one."
```

---

### Task 2: The routing ledger client

**Files:**
- Create: `crates/trace-commons-contributor/src/routing/mod.rs`
- Create: `crates/trace-commons-contributor/src/routing/ironwire.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs`

**Interfaces:**
- Produces: `RoutedExchange` (public struct, fields below); `trait RoutingLedger: Send + Sync { fn exchanges_since(&self, from: DateTime<Utc>) -> Vec<RoutedExchange>; }`; `IronWireLedger::new(port: u16, token: String) -> Self`; `IronWireLedger::refresh(&self) -> impl Future<Output = ()>`

- [ ] **Step 1: Write the failing test**

Create `crates/trace-commons-contributor/src/routing/mod.rs` with only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(offset: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000 + offset, 0).unwrap()
    }

    fn row(session: &str, offset: i64) -> RoutedExchange {
        RoutedExchange {
            started_at: at(offset),
            client_session_id: Some(session.to_string()),
            total_ms: Some(1200),
            facade: "anthropic".to_string(),
            backend: "claude-sub".to_string(),
            requested_model: Some("claude-opus-4-6".to_string()),
            served_model: Some("claude-opus-4-6".to_string()),
            rung: "same_model".to_string(),
            attempts: 1,
            input_tokens: Some(1000),
            cache_read_tokens: Some(500),
            cache_write_tokens: None,
            output_tokens: Some(200),
            cost_usd: Some(0.02),
            status: 200,
        }
    }

    #[test]
    fn a_snapshot_returns_only_rows_at_or_after_the_cutoff() {
        let ledger = FixedLedger::new(vec![row("s-1", 0), row("s-1", 60), row("s-1", 120)]);
        let seen = ledger.exchanges_since(at(60));
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0].started_at, at(60));
    }

    #[test]
    fn an_empty_ledger_is_not_an_error() {
        // Absence and failure must be the same state everywhere downstream.
        let ledger = FixedLedger::new(Vec::new());
        assert!(ledger.exchanges_since(at(0)).is_empty());
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo test -p trace-commons-contributor routing::tests
```

Expected: FAIL — module not declared, types not found.

- [ ] **Step 3: Write the module**

`crates/trace-commons-contributor/src/routing/mod.rs`, above the test module:

```rust
//! Routing and cost data for the inference hops behind a session.
//!
//! A local inference proxy (IronWire) sits at the point every model call
//! passes through and records what each one cost, which backend served it and
//! how long it took. Our scrapers read session files and cannot see any of
//! that. This module reads the proxy's own local ledger and hands the rows to
//! the decorating source in [`enriched`], which joins them onto the sessions
//! we already build.
//!
//! Three rules hold everywhere in here:
//!
//! - **Absence and failure are the same state.** Not installed, not running,
//!   token unreadable, JSON we cannot parse: every one resolves to no rows.
//!   The correct behaviour on failure is identical to the correct behaviour on
//!   absence, so there is no permanent-versus-transient distinction to model.
//! - **Nothing here can fail a submission.** No method returns an error to the
//!   load path.
//! - **Attribution only.** These numbers are corpus metadata. They must never
//!   reach a gate, a scoring input, or a credit computation. They come from a
//!   proxy the contributor can patch.

use chrono::{DateTime, Utc};
use serde::Deserialize;

pub mod enriched;
pub mod ironwire;

/// One inference hop, as the proxy recorded it.
///
/// Deliberately a local type deserialized from the proxy's JSON rather than
/// its own row struct: this crate takes no dependency on IronWire, which pulls
/// an Ironclaw tree this repo does not have and must not gain.
///
/// Unknown fields are ignored, so a proxy release that adds a column does not
/// break us. Missing fields that we need are `Option`, so one that goes away
/// degrades a row rather than dropping it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct RoutedExchange {
    pub started_at: DateTime<Utc>,
    /// The agent session the proxy saw on the request. `None` on a proxy older
    /// than the release that records it, which simply means this row cannot be
    /// joined to anything.
    #[serde(default)]
    pub client_session_id: Option<String>,
    #[serde(default)]
    pub total_ms: Option<i64>,
    pub facade: String,
    pub backend: String,
    #[serde(default)]
    pub requested_model: Option<String>,
    #[serde(default)]
    pub served_model: Option<String>,
    pub rung: String,
    pub attempts: i64,
    #[serde(default)]
    pub input_tokens: Option<i64>,
    #[serde(default)]
    pub cache_read_tokens: Option<i64>,
    #[serde(default)]
    pub cache_write_tokens: Option<i64>,
    #[serde(default)]
    pub output_tokens: Option<i64>,
    /// What the hop cost, priced by the proxy from observed tokens.
    ///
    /// Priced, not billed: work served on a subscription is priced at what it
    /// *would* have cost on the meter. No surface may render it as money the
    /// contributor spent.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    pub status: i64,
}

/// A source of routing rows.
///
/// Synchronous by design. `TraceSource::load` is synchronous, so anything the
/// join needs must already be in memory by the time it runs -- see
/// [`ironwire::IronWireLedger::refresh`]. Returns rows rather than a `Result`:
/// there is no failure a caller could act on differently from absence.
pub trait RoutingLedger: Send + Sync {
    /// Rows at or after `from`, oldest first.
    fn exchanges_since(&self, from: DateTime<Utc>) -> Vec<RoutedExchange>;
}

/// A ledger over a fixed set of rows. Used by tests and by the `Off` state.
#[derive(Debug, Default)]
pub struct FixedLedger {
    rows: Vec<RoutedExchange>,
}

impl FixedLedger {
    #[must_use]
    pub fn new(rows: Vec<RoutedExchange>) -> Self {
        Self { rows }
    }
}

impl RoutingLedger for FixedLedger {
    fn exchanges_since(&self, from: DateTime<Utc>) -> Vec<RoutedExchange> {
        let mut rows: Vec<RoutedExchange> = self
            .rows
            .iter()
            .filter(|row| row.started_at >= from)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.started_at);
        rows
    }
}
```

Add to `crates/trace-commons-contributor/src/lib.rs`, beside the other `pub mod` lines:

```rust
pub mod routing;
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-contributor routing::tests
```

Expected: PASS. `enriched` and `ironwire` do not exist yet, so temporarily comment out their `pub mod` lines to get a green run, then restore them in the next steps.

- [ ] **Step 5: Write the failing test for the client**

Create `crates/trace-commons-contributor/src/routing/ironwire.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ledger_that_has_never_refreshed_has_no_rows() {
        // The state on every machine without the proxy, and the state during
        // the first seconds on a machine with it. Neither is an error.
        let ledger = IronWireLedger::new(8463, "t".to_string());
        assert!(ledger.exchanges_since(chrono::Utc::now()).is_empty());
    }

    #[test]
    fn a_body_we_cannot_parse_leaves_the_snapshot_untouched() {
        let ledger = IronWireLedger::new(8463, "t".to_string());
        ledger.absorb(br#"{"enabled":true,"exchanges":[{"started_at":"2026-09-01T00:00:00Z","facade":"anthropic","backend":"claude-sub","rung":"same_model","attempts":1,"status":200,"client_session_id":"s-1"}]}"#);
        assert_eq!(ledger.exchanges_since(chrono::DateTime::UNIX_EPOCH).len(), 1);

        // A proxy release renames something, or an error page is served.
        ledger.absorb(b"<html>not json</html>");
        assert_eq!(
            ledger.exchanges_since(chrono::DateTime::UNIX_EPOCH).len(),
            1,
            "the last good snapshot survives a bad response"
        );
    }
}
```

- [ ] **Step 6: Run it to verify it fails**

```bash
cargo test -p trace-commons-contributor routing::ironwire
```

Expected: FAIL — `IronWireLedger` not found.

- [ ] **Step 7: Write the client**

Above that test module in `ironwire.rs`:

```rust
//! The IronWire control-API client.
//!
//! Reads `GET /_ironwire/log` on loopback, authenticated with the token the
//! proxy writes to `$IRONWIRE_HOME/control.token` (mode 0600). The token is
//! read at call time and never copied into our settings or logged.

use std::sync::{Arc, RwLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::{RoutedExchange, RoutingLedger};

/// How long a refresh may take before we give up and keep the old snapshot.
///
/// Short because this is a loopback call to a process on the same machine. A
/// proxy slow enough to miss this is a proxy whose numbers we would rather do
/// without than wait for.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(3);

/// How far back a refresh asks for. Generous relative to the daemon's cadence
/// so a missed tick overlaps rather than gaps.
const REFRESH_WINDOW_HOURS: i64 = 24;

/// The proxy's log response. Only the fields we use.
#[derive(Debug, Deserialize)]
struct LogView {
    #[serde(default)]
    exchanges: Vec<RoutedExchange>,
}

/// A [`RoutingLedger`] backed by a local IronWire daemon.
///
/// Holds a snapshot refreshed out of band. `exchanges_since` never does I/O,
/// because it is called from `TraceSource::load`, which is synchronous and on
/// the path to a submission.
pub struct IronWireLedger {
    port: u16,
    token: String,
    snapshot: Arc<RwLock<Vec<RoutedExchange>>>,
}

impl std::fmt::Debug for IronWireLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The token is a credential for an API that can rewrite the user's
        // agent configs. It does not appear in a debug line.
        f.debug_struct("IronWireLedger")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl IronWireLedger {
    #[must_use]
    pub fn new(port: u16, token: String) -> Self {
        Self {
            port,
            token,
            snapshot: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Replace the snapshot from a response body, if it parses.
    ///
    /// A body we cannot read leaves the previous snapshot in place. Enrichment
    /// that is one refresh stale is better than enrichment that vanishes
    /// because a proxy served an error page once.
    pub(crate) fn absorb(&self, body: &[u8]) {
        let Ok(view) = serde_json::from_slice::<LogView>(body) else {
            return;
        };
        if let Ok(mut snapshot) = self.snapshot.write() {
            *snapshot = view.exchanges;
        }
    }

    /// Fetch the last [`REFRESH_WINDOW_HOURS`] from the proxy.
    ///
    /// Infallible from the caller's perspective: every failure -- not
    /// installed, not running, token rejected, body unreadable -- leaves the
    /// snapshot as it was. A 401 is a configuration fact, not a transient, so
    /// nothing here retries.
    pub async fn refresh(&self) {
        let since = Utc::now() - chrono::Duration::hours(REFRESH_WINDOW_HOURS);
        // The `Z` form deliberately: a literal `+` in a query string is a
        // space, so an offset-form timestamp arrives malformed.
        let url = format!(
            "http://127.0.0.1:{}/_ironwire/log?limit=1000&since={}",
            self.port,
            since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        );
        let Ok(client) = reqwest::Client::builder().timeout(REFRESH_TIMEOUT).build() else {
            return;
        };
        let Ok(response) = client
            .get(&url)
            .header("authorization", format!("Bearer {}", self.token))
            .send()
            .await
        else {
            tracing::debug!("routing ledger unreachable");
            return;
        };
        if !response.status().is_success() {
            tracing::debug!(status = response.status().as_u16(), "routing ledger refused");
            return;
        }
        let Ok(body) = response.bytes().await else {
            return;
        };
        self.absorb(&body);
    }
}

impl RoutingLedger for IronWireLedger {
    fn exchanges_since(&self, from: DateTime<Utc>) -> Vec<RoutedExchange> {
        let Ok(snapshot) = self.snapshot.read() else {
            return Vec::new();
        };
        let mut rows: Vec<RoutedExchange> = snapshot
            .iter()
            .filter(|row| row.started_at >= from)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.started_at);
        rows
    }
}
```

- [ ] **Step 8: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-contributor routing::
```

Expected: PASS.

- [ ] **Step 9: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
git add crates/trace-commons-contributor/src/routing/ crates/trace-commons-contributor/src/lib.rs
git commit -m "Read the local inference proxy's routing ledger

Our scrapers read session files and cannot see what a turn cost, which
backend served it, or how long it took. A local proxy records exactly
that. This reads its loopback control API over plain HTTP and serde --
no IronWire crate, which would pull an Ironclaw tree this repo does not
have and must not gain.

The read is a snapshot refreshed out of band, because TraceSource::load
is synchronous and on the path to a submission. Absence and failure are
the same state throughout: not installed, not running, token refused,
body unreadable all resolve to no rows, and a body we cannot parse
leaves the previous snapshot rather than emptying it."
```

---

### Task 3: The decorating source

**Files:**
- Create: `crates/trace-commons-contributor/src/routing/enriched.rs`
- Modify: `crates/trace-commons-contributor/src/source/mod.rs:114-142` (`SessionTranscript`)

**Interfaces:**
- Consumes: `RoutedExchange`, `RoutingLedger` (Task 2)
- Produces: `SessionTranscript.routing: Vec<RoutedExchange>`; `RoutingEnrichedSource::new(inner: Box<dyn TraceSource>, ledger: Arc<dyn RoutingLedger>) -> Self`

- [ ] **Step 1: Write the failing test**

Create `crates/trace-commons-contributor/src/routing/enriched.rs` with only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{FixedLedger, RoutedExchange};
    use chrono::TimeZone;

    fn at(offset: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.timestamp_opt(1_700_000_000 + offset, 0).unwrap()
    }

    fn row(session: Option<&str>, offset: i64) -> RoutedExchange {
        RoutedExchange {
            started_at: at(offset),
            client_session_id: session.map(str::to_string),
            total_ms: Some(1200),
            facade: "anthropic".to_string(),
            backend: "claude-sub".to_string(),
            requested_model: None,
            served_model: None,
            rung: "same_model".to_string(),
            attempts: 1,
            input_tokens: Some(1000),
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: Some(200),
            cost_usd: Some(0.02),
            status: 200,
        }
    }

    /// A source returning one transcript with a known conversation id.
    struct StubSource {
        conversation_id: Option<String>,
    }

    impl crate::source::TraceSource for StubSource {
        fn name(&self) -> &'static str {
            "stub"
        }
        fn discover(&self) -> anyhow::Result<Vec<crate::source::SessionRef>> {
            Ok(Vec::new())
        }
        fn load(&self, _r: &crate::source::SessionRef) -> anyhow::Result<crate::source::SessionTranscript> {
            let mut t = crate::source::SessionTranscript::default();
            t.conversation_id = self.conversation_id.clone();
            Ok(t)
        }
        fn session_for_path(&self, _p: &std::path::Path) -> Option<crate::source::SessionRef> {
            None
        }
    }

    fn a_ref() -> crate::source::SessionRef {
        // Build with whatever constructor the crate already provides; see the
        // existing adapters' tests for the shape.
        crate::source::SessionRef::default()
    }

    #[test]
    fn only_the_rows_for_this_session_are_attached() {
        let ledger = FixedLedger::new(vec![
            row(Some("s-1"), 0),
            row(Some("s-2"), 10),
            row(Some("s-1"), 20),
        ]);
        let source = RoutingEnrichedSource::new(
            Box::new(StubSource { conversation_id: Some("s-1".into()) }),
            std::sync::Arc::new(ledger),
        );
        let t = source.load(&a_ref()).expect("loads");
        assert_eq!(t.routing.len(), 2, "the other session's rows are not ours");
    }

    #[test]
    fn a_session_we_cannot_identify_gets_no_overlay() {
        // A transcript with no conversation id cannot be joined to anything,
        // and guessing would attribute another session's cost to this trace.
        let ledger = FixedLedger::new(vec![row(Some("s-1"), 0)]);
        let source = RoutingEnrichedSource::new(
            Box::new(StubSource { conversation_id: None }),
            std::sync::Arc::new(ledger),
        );
        assert!(source.load(&a_ref()).expect("loads").routing.is_empty());
    }

    #[test]
    fn rows_that_name_no_session_are_never_attached() {
        // A proxy older than the release that records the session id, and the
        // proxy's own auxiliary requests, both land here.
        let ledger = FixedLedger::new(vec![row(None, 0), row(None, 10)]);
        let source = RoutingEnrichedSource::new(
            Box::new(StubSource { conversation_id: Some("s-1".into()) }),
            std::sync::Arc::new(ledger),
        );
        assert!(source.load(&a_ref()).expect("loads").routing.is_empty());
    }

    #[test]
    fn an_empty_overlay_leaves_the_transcript_as_the_inner_source_built_it() {
        let source = RoutingEnrichedSource::new(
            Box::new(StubSource { conversation_id: Some("s-1".into()) }),
            std::sync::Arc::new(FixedLedger::new(Vec::new())),
        );
        let t = source.load(&a_ref()).expect("loads");
        assert!(t.routing.is_empty());
        assert_eq!(t.conversation_id.as_deref(), Some("s-1"));
    }
}
```

If `SessionRef` and `SessionTranscript` have no `Default`, derive it on `SessionTranscript` in this task and build a `SessionRef` in the test the way the existing adapter tests do. Do not add a public constructor purely for tests.

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo test -p trace-commons-contributor routing::enriched
```

Expected: FAIL — `RoutingEnrichedSource` not found, and `SessionTranscript` has no `routing` field.

- [ ] **Step 3: Add the transcript field**

In `crates/trace-commons-contributor/src/source/mod.rs`, in `SessionTranscript` after `subagents_dropped`:

```rust
    /// Routing and cost data for the inference hops behind this session, when
    /// a local proxy recorded them and the session could be joined to them.
    ///
    /// Empty is the normal state, not a failure: most contributors run no
    /// proxy, and a session that predates one is only partly covered even
    /// where one exists. The transcript is the single carrier of everything
    /// the envelope builder needs, which is why this lives here rather than
    /// being threaded through the four builders separately.
    pub routing: Vec<crate::routing::RoutedExchange>,
```

Fix every construction site the compiler names; all take `Vec::new()`.

- [ ] **Step 4: Write the decorator**

Above the test module in `enriched.rs`:

```rust
//! A [`TraceSource`](crate::source::TraceSource) that decorates another.
//!
//! The ledger is not a session. Registering it as a fourth source would make
//! it independently discoverable, queueable, previewable and submittable,
//! because everything downstream keys on a `SessionRef` with a path and a
//! size -- and all three trait methods would have to lie. It is an overlay on
//! sessions the real adapters already build, so it decorates them.
//!
//! Wrapping rather than threading a parameter: there are three production
//! `load` call sites and four public envelope builders, and a parameter would
//! touch all of them. This has one insertion point, `all_sources`.

use std::path::Path;
use std::sync::Arc;

use crate::routing::{RoutedExchange, RoutingLedger};
use crate::source::{SessionRef, SessionTranscript, TraceSource};

/// Wraps a real adapter and attaches the routing overlay to what it loads.
pub struct RoutingEnrichedSource {
    inner: Box<dyn TraceSource>,
    ledger: Arc<dyn RoutingLedger>,
}

impl RoutingEnrichedSource {
    #[must_use]
    pub fn new(inner: Box<dyn TraceSource>, ledger: Arc<dyn RoutingLedger>) -> Self {
        Self { inner, ledger }
    }
}

impl TraceSource for RoutingEnrichedSource {
    fn name(&self) -> &'static str {
        // The adapter's own name. This decorates a source; it is not one, and
        // a session's provenance is the adapter that read it.
        self.inner.name()
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        // Untouched, deliberately. `size_bytes` and `group_modified_at` drive
        // quiescence and eligibility, and a session must be eligible on its
        // own bytes -- never on whether a proxy happened to answer.
        self.inner.discover()
    }

    fn session_for_path(&self, path: &Path) -> Option<SessionRef> {
        self.inner.session_for_path(path)
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        let mut transcript = self.inner.load(r)?;
        // Only ever additive, and only after the real load succeeded. Nothing
        // below can fail the load: no `?`, no early return with an error.
        let Some(session_id) = transcript.conversation_id.clone() else {
            return Ok(transcript);
        };
        let from = transcript
            .started_at
            .unwrap_or(chrono::DateTime::UNIX_EPOCH);
        transcript.routing = self
            .ledger
            .exchanges_since(from)
            .into_iter()
            .filter(|row| row.client_session_id.as_deref() == Some(session_id.as_str()))
            .collect::<Vec<RoutedExchange>>();
        Ok(transcript)
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-contributor routing::
```

Expected: PASS, four new tests.

- [ ] **Step 6: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run
git add crates/trace-commons-contributor/src/routing/enriched.rs \
        crates/trace-commons-contributor/src/source/mod.rs
git commit -m "Attach the routing overlay to the session it belongs to

A decorator rather than a fourth source: the ledger is not a session,
and registering it as one would make it independently discoverable,
queueable and submittable, with all three trait methods lying about what
it is.

The join is the session id both sides record. A transcript we cannot
identify gets no overlay and rows naming no session are never attached
-- guessing would put one session's cost on another's trace. discover
is delegated untouched, so a session stays eligible on its own bytes
whether or not a proxy answered."
```

---

### Task 4: Emit the routing events

**Files:**
- Modify: `crates/trace-commons-contributor/src/envelope.rs:754-797` (`raw_events_for`) and its call site

**Interfaces:**
- Consumes: `SessionTranscript.routing` (Task 3), `routing_metadata` presence (Task 1)
- Produces: `RoutingDecision` events on the envelope

- [ ] **Step 1: Write the failing test**

In the existing `#[cfg(test)] mod tests` of `crates/trace-commons-contributor/src/envelope.rs`:

```rust
#[test]
fn a_routing_row_becomes_an_event_carrying_its_numbers_in_typed_fields() {
    // The numbers go in the typed fields the event already has. Only the
    // labels with no typed home go in `structured_payload`, and that is what
    // the routing_metadata presence class exists to declare.
    let events = raw_events_for_with_routing(
        &[],
        &[sample_routed_exchange()],
        chrono::Utc::now(),
    );
    let routing: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == TraceContributionEventType::RoutingDecision)
        .collect();
    assert_eq!(routing.len(), 1);
    let e = routing[0];
    assert_eq!(e.latency_ms, Some(1200));
    assert_eq!(e.cost_usd.is_some(), true);
    assert_eq!(e.token_counts.as_ref().map(|t| t.input_tokens), Some(1000));
    assert!(
        e.redacted_content.as_deref().unwrap_or("").is_empty(),
        "the overlay is numbers and labels, never text"
    );
    assert_eq!(e.structured_payload["backend"], "claude-sub");
    assert_eq!(e.side_effect, SideEffectLevel::None);
}

#[test]
fn routing_events_do_not_declare_a_tool_payload() {
    // The whole point of task 1, asserted where the events are actually built.
    let events = raw_events_for_with_routing(&[], &[sample_routed_exchange()], chrono::Utc::now());
    let presence = declared_content_presence(&events);
    assert!(presence.routing_metadata);
    assert!(!presence.tool_payloads);
}

#[test]
fn a_session_with_no_routing_rows_produces_exactly_what_it_did_before() {
    let before = raw_events_for(&[], chrono::Utc::now());
    let after = raw_events_for_with_routing(&[], &[], chrono::Utc::now());
    assert_eq!(before.len(), after.len());
}
```

Add a fixture beside the other test helpers:

```rust
fn sample_routed_exchange() -> crate::routing::RoutedExchange {
    crate::routing::RoutedExchange {
        started_at: chrono::Utc::now(),
        client_session_id: Some("s-1".to_string()),
        total_ms: Some(1200),
        facade: "anthropic".to_string(),
        backend: "claude-sub".to_string(),
        requested_model: Some("claude-opus-4-6".to_string()),
        served_model: Some("claude-opus-4-6".to_string()),
        rung: "same_model".to_string(),
        attempts: 1,
        input_tokens: Some(1000),
        cache_read_tokens: Some(500),
        cache_write_tokens: None,
        output_tokens: Some(200),
        cost_usd: Some(0.02),
        status: 200,
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo test -p trace-commons-contributor a_routing_row_becomes_an_event
```

Expected: FAIL — `raw_events_for_with_routing` not found.

- [ ] **Step 3: Implement**

In `crates/trace-commons-contributor/src/envelope.rs`, keep `raw_events_for` exactly as it is and add beside it:

```rust
/// [`raw_events_for`] plus one `RoutingDecision` per joined inference hop.
///
/// The routing events are appended rather than interleaved. A hop is not a
/// step in the transcript -- it is how a step was served -- and giving it a
/// position in the sequence would assert an ordering the ledger cannot
/// support: rows are timestamped by the proxy, session events by the harness,
/// and the two clocks are not the same clock.
fn raw_events_for_with_routing(
    events: &[SessionEvent],
    routing: &[crate::routing::RoutedExchange],
    now: DateTime<Utc>,
) -> Vec<RawTraceContributionEvent> {
    let mut mapped = raw_events_for(events, now);
    mapped.extend(routing.iter().map(raw_routing_event_for));
    mapped
}

/// One inference hop as an envelope event.
///
/// Numbers go in the typed fields the event already carries. Only the labels
/// with no typed home -- backend, rung, attempts, the requested/served model
/// pair, the cache token split -- go in `structured_payload`, and that is what
/// `routing_metadata` declares. `redacted_content` stays empty: nothing here
/// is text from the session, and a routing event that carried prose would
/// declare `message_text` and mean something else entirely.
fn raw_routing_event_for(row: &crate::routing::RoutedExchange) -> RawTraceContributionEvent {
    RawTraceContributionEvent {
        event_id: Uuid::new_v4(),
        parent_event_id: None,
        event_type: TraceContributionEventType::RoutingDecision,
        timestamp: row.started_at,
        redacted_content: None,
        structured_payload: serde_json::json!({
            "backend": row.backend,
            "facade": row.facade,
            "rung": row.rung,
            "attempts": row.attempts,
            "requested_model": row.requested_model,
            "served_model": row.served_model,
            "cache_read_tokens": row.cache_read_tokens,
            "cache_write_tokens": row.cache_write_tokens,
            "status": row.status,
        }),
        tool_name: None,
        tool_category: None,
        tool_call_id: None,
        latency_ms: row.total_ms.and_then(|ms| u64::try_from(ms).ok()),
        token_counts: match (row.input_tokens, row.output_tokens) {
            (Some(input), Some(output)) => Some(TokenCounts {
                input_tokens: u32::try_from(input).unwrap_or(u32::MAX),
                output_tokens: u32::try_from(output).unwrap_or(u32::MAX),
            }),
            // `None`, never a fabricated zero: an unreported count that summed
            // as zero would understate what the session actually consumed.
            _ => None,
        },
        cost_usd: row
            .cost_usd
            .and_then(|usd| {
                trace_commons_protocol::trace_contribution::Decimal::try_from(usd).ok()
            }),
        success: None,
        failure_modes: Vec::new(),
        side_effect: SideEffectLevel::None,
    }
}
```

Match the exact field set of `RawTraceContributionEvent` as `raw_event_for` at `:799` constructs it — if a field there is absent here, the compiler will say so.

`Decimal` comes from the protocol re-export added in Task 1, Step 11. **Do not add `rust_decimal` to this crate's `Cargo.toml`** — these client crates ship inside third-party harnesses, and every direct dependency they gain is one their vendor inherits.

- [ ] **Step 4: Switch the builder to the routing-aware version**

At `crates/trace-commons-contributor/src/envelope.rs`, find where `raw_events_for` is called in `build_raw_contribution_with_id` and replace it:

```rust
    let events = raw_events_for_with_routing(&t.events, &t.routing, now);
```

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-contributor envelope::
```

Expected: PASS. The pinned digest in `daemon/preview.rs` should NOT move — that fixture has no routing rows, so its envelope is unchanged. If it does move, something is being emitted for an empty overlay; fix that rather than repinning.

- [ ] **Step 6: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
cargo test --workspace 2>&1 | tail -20
git add crates/trace-commons-contributor/src/envelope.rs
git commit -m "Emit one routing event per joined inference hop

RoutingDecision rather than HttpExchange: HttpExchange means the agent
made an outbound call and carries SideEffectLevel::ReadOnly, and an
inference hop is not a side effect the agent performed.

Numbers go in the typed fields the event already has -- latency,
token counts, cost. Only the labels with no typed home go in the
structured payload, which is what routing_metadata declares. The events
are appended rather than interleaved: a hop is how a step was served,
not a step, and the proxy's clock is not the harness's clock."
```

---

### Task 5: Declare the proxy, and default to off

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs` (after `SourceDeclaration` at `:178-198`)
- Modify: `crates/trace-commons-contributor/src/source/mod.rs:350-383` (`all_sources`)

**Interfaces:**
- Consumes: `IronWireLedger` (Task 2), `RoutingEnrichedSource` (Task 3)
- Produces: `DaemonSettings.ironwire: Option<IronWireDeclaration>`; `all_sources(claude, codex, trajectory_path, routing: Option<Arc<dyn RoutingLedger>>)`

- [ ] **Step 1: Write the failing test**

In `crates/trace-commons-contributor/src/daemon/settings.rs` tests:

```rust
#[test]
fn a_contributor_who_never_mentioned_the_proxy_is_not_probed() {
    // The divergence from SourceDeclaration, and the reason for it. For a
    // session root, `None` falls back to the conventional location. There is
    // no conventional location for a local service: connecting to 127.0.0.1
    // unasked is a probe of something the contributor never mentioned, which
    // is the same mistake the source tri-state exists to have fixed.
    let settings: DaemonSettings = serde_json::from_str("{}").expect("empty settings load");
    assert!(settings.ironwire.is_none());
    assert!(
        ironwire_ledger_for(settings.ironwire.as_ref()).is_none(),
        "no declaration means no reader is built at all"
    );
}

#[test]
fn a_proxy_declared_off_builds_no_reader() {
    let settings: DaemonSettings =
        serde_json::from_str(r#"{"ironwire":{"mode":"off"}}"#).expect("loads");
    assert!(ironwire_ledger_for(settings.ironwire.as_ref()).is_none());
}

#[test]
fn a_watched_proxy_round_trips_its_port() {
    let settings: DaemonSettings =
        serde_json::from_str(r#"{"ironwire":{"mode":"watch","port":8463}}"#).expect("loads");
    assert_eq!(
        settings.ironwire,
        Some(IronWireDeclaration::Watch { port: 8463 })
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
cargo test -p trace-commons-contributor a_contributor_who_never_mentioned
```

Expected: FAIL — no `ironwire` field.

- [ ] **Step 3: Add the declaration**

In `settings.rs`, after `SourceDeclaration`'s `impl` block:

```rust
/// What the contributor said about a local inference proxy.
///
/// Deliberately NOT the same tri-state semantics as [`SourceDeclaration`].
/// There, `None` means "never asked" and falls back to the conventional
/// per-user location. Here `None` means **off**, with no fallback.
///
/// A session root has a conventional location to fall back to. A local service
/// does not: connecting to `127.0.0.1:8463` because nobody said otherwise is a
/// probe of a service the contributor never mentioned, which is exactly the
/// error the source tri-state was introduced to stop making about their files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum IronWireDeclaration {
    /// Read the proxy's ledger on this loopback port.
    Watch { port: u16 },
    /// The contributor said they do not use it. Nothing is read.
    Off,
}

impl IronWireDeclaration {
    /// The port to read, or `None` when the proxy is off.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        match self {
            IronWireDeclaration::Watch { port } => Some(*port),
            IronWireDeclaration::Off => None,
        }
    }
}
```

Add to `DaemonSettings`, beside the source declarations:

```rust
    /// A local inference proxy, when the contributor declared one. Absent
    /// means off: see [`IronWireDeclaration`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ironwire: Option<IronWireDeclaration>,
```

And beside it, the constructor the tests call:

```rust
/// Build a routing ledger for a declaration, or nothing.
///
/// The token is read from `$IRONWIRE_HOME/control.token` at build time and
/// never copied into our settings file. An unreadable token yields no reader:
/// absence and failure are the same state.
#[must_use]
pub fn ironwire_ledger_for(
    declaration: Option<&IronWireDeclaration>,
) -> Option<std::sync::Arc<crate::routing::ironwire::IronWireLedger>> {
    let port = declaration?.port()?;
    let home = std::env::var_os("IRONWIRE_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".ironwire")))?;
    let token = std::fs::read_to_string(home.join("control.token")).ok()?;
    Some(std::sync::Arc::new(
        crate::routing::ironwire::IronWireLedger::new(port, token.trim().to_string()),
    ))
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-contributor settings::
```

Expected: PASS.

- [ ] **Step 5: Wire the decorator into the registry**

In `crates/trace-commons-contributor/src/source/mod.rs`, change `all_sources` to take a fourth argument and wrap each adapter:

```rust
pub fn all_sources(
    claude: Option<SourceDeclaration>,
    codex: Option<SourceDeclaration>,
    trajectory_path: Option<PathBuf>,
    routing: Option<std::sync::Arc<dyn crate::routing::RoutingLedger>>,
) -> Vec<Box<dyn TraceSource>> {
    let mut sources: Vec<Box<dyn TraceSource>> = Vec::new();

    // ... every existing push, unchanged ...

    // One insertion point for the whole overlay. Without a declared proxy the
    // adapters are returned bare, which is the majority case and costs one
    // branch.
    let Some(routing) = routing else {
        return sources;
    };
    sources
        .into_iter()
        .map(|source| {
            Box::new(crate::routing::enriched::RoutingEnrichedSource::new(
                source,
                std::sync::Arc::clone(&routing),
            )) as Box<dyn TraceSource>
        })
        .collect()
}
```

Then update the two call sites the compiler names (`commands.rs:644` and `:870`) and any others, passing `None` for now at the CLI sites and `ironwire_ledger_for(settings.ironwire.as_ref()).map(|l| l as Arc<dyn RoutingLedger>)` at the daemon site.

- [ ] **Step 6: Add the registry test**

In `source/mod.rs` tests:

```rust
#[test]
fn without_a_declared_proxy_the_adapters_are_returned_bare() {
    let sources = all_sources(Some(SourceDeclaration::Off), Some(SourceDeclaration::Off), None, None);
    assert!(sources.is_empty());
}

#[test]
fn a_declared_proxy_decorates_every_adapter_without_adding_one() {
    let ledger: std::sync::Arc<dyn crate::routing::RoutingLedger> =
        std::sync::Arc::new(crate::routing::FixedLedger::new(Vec::new()));
    let bare = all_sources(None, None, None, None).len();
    let wrapped = all_sources(None, None, None, Some(ledger)).len();
    assert_eq!(bare, wrapped, "decorating must not add or drop a source");
}
```

- [ ] **Step 7: Run and verify**

```bash
cargo fmt --all
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
cargo test -p trace-commons-contributor
```

Expected: PASS.

- [ ] **Step 8: Let the daemon say it is declared but reading nothing**

Version skew fails silently: we parse another project's JSON with no shared
type, so a renamed field degrades us to an empty overlay. That is the right
failure and an invisible one, so the daemon has to be able to say so.

Add to `IronWireLedger`:

```rust
    /// Whether the last refresh produced any rows.
    ///
    /// Distinguishes "declared, reading nothing" from "not declared" in the
    /// daemon's health output. Not an error state -- a machine whose proxy was
    /// installed today legitimately reports this -- but a user who declared a
    /// proxy and sees no enrichment needs somewhere to look.
    #[must_use]
    pub fn has_rows(&self) -> bool {
        self.snapshot.read().is_ok_and(|rows| !rows.is_empty())
    }
```

Surface it once in the daemon's existing health reporting (find the module that
already reports source roots and follow its shape). Report it **once at
refresh-state change, never per poll**, and never as an error.

Test:

```rust
#[test]
fn a_declared_proxy_that_returns_nothing_is_distinguishable_from_no_proxy() {
    let ledger = IronWireLedger::new(8463, "t".to_string());
    assert!(!ledger.has_rows(), "declared but empty");
}
```

- [ ] **Step 9: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/settings.rs \
        crates/trace-commons-contributor/src/source/mod.rs \
        crates/trace-commons-contributor/src/commands.rs
git commit -m "Read a local proxy only when the contributor declared one

Absent means off, with no fallback -- deliberately unlike the source
roots, where absent falls back to the conventional location. A session
root has a conventional location. A local service does not: connecting
to 127.0.0.1 because nobody said otherwise is a probe of something the
contributor never mentioned, which is the error the source tri-state
exists to have stopped making about their files.

The decorator has one insertion point, so the registry is the only place
that learns about any of this."
```

---

### Task 6: Pin attribution-only

The routing numbers come from a proxy the contributor can patch and rebuild. If they ever reach a scorer or a credit computation, a modified proxy reporting inflated costs becomes a credit-farming vector. Today the barrier is a three-field allowlist that exists for signal-quality reasons and has a raw-text fallback — a mechanism, but not one anybody wrote down as this rule. This test writes it down.

**Files:**
- Create: `crates/trace-commons-contributor/tests/routing_is_attribution_only.rs`

**Interfaces:**
- Consumes: everything from Tasks 1-5

- [ ] **Step 1: Write the test**

```rust
//! Routing metadata is corpus metadata and nothing else.
//!
//! `cost_usd` and the token counts arrive from a local proxy the contributor
//! controls: it is Apache-2.0 and they can build their own. If either ever
//! reaches a scoring or credit input, a patched proxy reporting inflated costs
//! is a direct credit-farming vector.
//!
//! There is a real barrier today -- the gate renders envelopes through a
//! three-field allowlist (`event_type`, `tool_name`, `redacted_content`) and
//! numeric fields are structurally unreachable through it. But that allowlist
//! exists for signal quality, not for this rule, and it falls back to raw text
//! when an envelope has no renderable events. So the property is a side effect
//! of two decisions made for other reasons, and this is the test that makes it
//! a rule instead.

use trace_commons_contributor::routing::RoutedExchange;

/// A cost no real ledger would produce, so a match is unambiguous.
const SENTINEL_COST: f64 = 133_713.37;

#[test]
fn a_routing_cost_never_appears_in_the_text_the_gate_scores() {
    let envelope = envelope_with_routing_cost(SENTINEL_COST);
    let rendered = trace_commons_protocol::trace_contribution::canonical_scored_text(&envelope);
    assert!(
        !rendered.contains("133713"),
        "the scored text must not carry a routing cost:\n{rendered}"
    );
}

#[test]
fn a_routing_event_does_not_declare_a_tool_payload() {
    let envelope = envelope_with_routing_cost(SENTINEL_COST);
    let presence =
        trace_commons_protocol::trace_contribution::derive_envelope_content_presence(&envelope);
    assert!(presence.routing_metadata);
    assert!(
        !presence.tool_payloads,
        "declaring routing as a tool payload quarantines the envelope"
    );
}
```

Build `envelope_with_routing_cost` from the crate's existing envelope-building path with a `SessionTranscript` whose `routing` holds one `RoutedExchange` carrying `SENTINEL_COST`. Reuse whatever fixture helper the envelope tests already use; do not write a second builder.

**If `canonical_scored_text` does not exist** under that name, find the function the gate actually renders with — it is the one described in the spec as the three-field allowlist, in `trace-commons-gate-enclave`. **That crate is AGPL and this one is permissive**, so it cannot be imported here. In that case, assert the weaker but still meaningful property in this crate — that the sentinel appears in no `redacted_content` and no `tool_name` on any event — and open an issue for the gate-side half. Write the reason in the test file; do not silently downgrade it.

- [ ] **Step 2: Run it**

```bash
cargo test -p trace-commons-contributor --test routing_is_attribution_only
```

Expected: PASS. If the first test fails, the routing cost is reaching the scorer and Task 4 put a number somewhere it must not be.

- [ ] **Step 3: Commit**

```bash
git add crates/trace-commons-contributor/tests/routing_is_attribution_only.rs
git commit -m "Pin routing metadata as attribution only

The numbers come from a proxy the contributor can patch and rebuild, so
if they ever reach a scoring or credit input a modified proxy reporting
inflated costs is a credit-farming vector.

A barrier exists today, but it is a side effect: the gate renders through
a three-field allowlist built for signal quality, and it falls back to
raw text when an envelope has no renderable events. This makes the
property a rule with a test behind it rather than a consequence nobody
wrote down."
```

---

## Not in this plan

- **The raw-text fallback in the gate's chunker.** Real, and out of scope for a contributor-side change. It needs its own issue: either refuse an envelope with no renderable events, or strip metadata on that path.
- **A read-scoped IronWire control token.** Reading the ledger currently means holding a token that can also rewrite the user's agent configs. Raised upstream separately.
- **Any UI or consent copy.** The declaration is settings-file only in this plan. Naming `cost_usd` on the consent card is a follow-up, and the spec says it needs saying explicitly rather than folding under "routing metadata".
