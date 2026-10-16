# Settlement Estimate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a contributor, before anything is sent, roughly how many inference dollars the traces on their Mac would settle for: one menu-bar line, the news mark, a capped notification, and dollar bands on queue rows.

**Architecture:** A v2 "settlement table" (tiers priced in `trace_credit` atomic units plus a batch ratio band and an operator-set dollar rate) is parsed by one protocol-crate parser that the server (boot validation, `GET /v1/credit-estimate/settlement-table`, and `target=settlement` fitting on the admin eval route) and the daemon share. The daemon fetches it on the existing estimate-table schedule, runs a separate estimation scanner over every non-Never folder (reading `lef1` features through a path-hashed cache, never queueing), aggregates per-tier means into a dollar band, and publishes it on `status`, through the attention arbiter and the news mark. Core copy and the C ABI carry every word; the native macOS shell draws the line, the mark, the notification and the Settings switch.

**Tech Stack:** Rust (protocol, server on axum + PostgreSQL, contributor daemon on tokio), C ABI (`trace-commons-contributor-ffi`), Swift / SwiftUI (`macos/`).

**Spec:** `docs/superpowers/specs/2026-10-15-settlement-estimate-design.md` (PR #1431). Owner decisions S1-S9 are settled; do not re-litigate them. Every implementer reads the spec section their task cites.

## Global Constraints

Repo and process rules (every task):

- Work only in `/Users/zakimanian/code/trace-commons-server/.claude/worktrees/settlement-estimate-spec` on branch `settlement-estimate-spec`, with absolute paths. Never `cd` elsewhere, never `git stash`, never run anything in the background, never push, merge, open a PR, or touch the pilot / GCP. Never run `cargo +1.99.0 clippy`.
- Commit locally after each task: short imperative subject, no prefix, no emoji, last line `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Run `cargo fmt --all` before each commit.
- Verification uses the CI forms: `RUSTFLAGS="-D warnings"` on check and test; clippy as `cargo clippy -p <crate> --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Quote the real `test result:` lines; never claim green from truncated or filtered output.
- Task 1 step 1 captures the test baseline once; every later task runs its targeted tests right after each change and its crate's full suite before committing, and compares failures against the baseline.
- New `.rs` files in `trace-commons-server`, `-gate-api`, `-gate-enclave` start with `// Copyright (C) 2026 K&Z Partners LLC` and `// SPDX-License-Identifier: AGPL-3.0-or-later`. Permissive crates (`-protocol`, `-contributor`, `-contributor-ffi`) never depend on an AGPL crate. New protocol code must compile under `cargo check -p trace-commons-protocol --no-default-features`.
- Public repo: synthetic values only in code, tests, comments and commits. No real calibration numbers, dollar rates, award caps, pilot details or private repo/policy names.
- Logging is label-only and counts-only: no path, project name, per-trace figure, or table figure in a log line. Fail closed.
- S9: positive consent before any trace data goes to the server or a TEE. Nothing in this feature queues, previews, redacts remotely, or sends. The only network request added is the unauthenticated table fetch.
- New user-facing strings live in `crates/trace-commons-contributor/src/nudge_copy.rs`, each doc-commented `DRAFT, NEEDS APPROVAL`, following `docs/ui-copy-guide.md` (short action labels; titles and actions have separate keys).
- The C ABI header exists twice (`crates/trace-commons-contributor-ffi/include/trace_commons.h`, `macos/Sources/CTraceCommons/include/trace_commons.h`); edit both and `diff` them to identical.
- Before any `swift test`, run `cargo build -p trace-commons-contributor-ffi` (the Swift package links that dylib; a stale one fakes results). Run Swift as `swift test --package-path /Users/zakimanian/code/trace-commons-server/.claude/worktrees/settlement-estimate-spec/macos --filter <Name>`.

Spec values, copied verbatim:

- Route `GET /v1/credit-estimate/settlement-table`, unauthenticated, no tenant context. Env `TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH`. Boot refusal label `credit_settlement_table_invalid`. Unset env: the route answers 404 and clients draw nothing.
- Wire fields: `schema_version` (2), `features_version` (`lef1`), `version`, `pricing_version`, `instrument_id` (`trace_credit`), `atomic_units_per_credit`, `usd_micros_per_credit` (optional), `published_at`, `bytes_per_token`, `chunk_target_tokens`, `chunk_cap`, `weights`, `cut_offs`, `tiers[] {mean_atomic_units, withheld_share_micros}`, `aggregate_band {low_ratio_micros, high_ratio_micros}`, `fit_rows`.
- Refusals: schema not 2 or features not `lef1`; fewer than 2 tiers or more than `ESTIMATE_MAX_TIERS` (3); a tier mean not positive; cut-offs not strictly increasing; `aggregate_band` out of order or `low_ratio_micros == 0`; `instrument_id` not `trace_credit`; `atomic_units_per_credit == 0`; document larger than 16 KiB. No rate: parses, draws nothing. Older than 7 days by `published_at`: draws nothing. No built-in settlement table. A refused fetch leaves no table, never a stale one.
- Eval: `target` = `display` (default) | `settlement`. Labels: committed Score outcome with a `trace_credit` `pipeline_run_settlements` row: `atomic_units`; committed Score outcome with no such row: 0; held for review or no committed Score outcome: excluded. Fit: E9 time split at `ESTIMATE_FIT_TRAIN_SHARE`; cut-offs as `fit_estimate_table`; `B = 200` batches of `n = min(20, rows)`; `aggregate_band` = p10 and p90 of `actual_total / predicted_total`; seeded; candidate table has no `usd_micros_per_credit` and no `pricing_version`; report `sum(actual) / sum(predicted)` over the evaluation split as the bias check.
- Total: `sum = Σ mean_atomic_units` over known traces; low = `sum * low_ratio`, high = `sum * high_ratio`; `usd = atomic_units / atomic_units_per_credit * usd_micros_per_credit / 1e6`. Both ends under $10: low down / high up to $0.50 steps; otherwise whole dollars. Rounded low of $0 reads "up to $Y". No known trace or no drawable table: nothing at all.
- Re-estimate: watcher change, 10-minute debounce, at most hourly; daily rediscovery; table `version` or `pricing_version` change: immediately; project mode to or from Never: immediately; estimate switch turned on: full scan. Table fetch rides the existing `EstimateTableSchedule` (24 h plus jitter).
- Notification candidate: `low >= 1.25 * last_notified_low` and `low - last_notified_low >= $1`; or `pricing_version` changed and `|mid - last_notified_mid| >= 0.25 * last_notified_mid`. "Not now" only dismisses.
- Icon: lit when drawable, low end at least $1, not muted, and every existing gate open (paused, consent hold, signed out, unhealthy, suggestions switch).
- Copy: `ESTIMATE_TOTAL_LINE` = "About {low} to {high} of inference in traces on this Mac"; Settings switch "Estimate what traces on this Mac are worth", on by default.

Decisions where the spec names something that does not exist or conflicts with the code (closest faithful option):

- D1 **Separate v2 type.** `LocalEstimateTable` is `deny_unknown_fields` and band-shaped, so v2 is a new `SettlementEstimateTable` in a new module `crates/trace-commons-protocol/src/settlement_estimate.rs`. Tier assignment is shared, not copied: `local_credit_estimate.rs` gains `pub fn tier_index(score: f64, cut_offs: &[f64]) -> usize` (the count of cut-offs at or below `score`, which `estimate` already computes inline and now calls), and `SettlementEstimateTable::scoring_view(&self) -> LocalEstimateTable` builds an unvalidated v1 view carrying the same `weights`, `cut_offs` and chunk parameters so `estimate_score` and `term_value` are reused unchanged.
- D2 **Age and size limits in protocol.** The v1 limits live in the contributor crate. The protocol defines `SETTLEMENT_TABLE_MAX_BYTES = 16 * 1024`, `SETTLEMENT_TABLE_MAX_AGE_SECS = 7 * 86_400` (equal to `ESTIMATE_TABLE_MAX_AGE`), and `SETTLEMENT_TABLE_MAX_FUTURE_SECS = 86_400`; a `published_at` further than that in the future is not drawable (a far-future date would otherwise never go stale).
- D3 **Never before any open.** `discover()` itself may peek a transcript head to learn its cwd (as the watcher already does for declared roots); that is not a feature read. The scanner resolves a ref's project only from `SessionRef.cwd` or a `DaemonState.cwd_cache` hit with matching size and mtime, never through `load()`. A ref with neither is counted `unknown` and never opened. The Never test is `policy.resolve(key) == ProjectMode::Ignore`, which includes the global Never override.
- D4 **Sources.** Scanner sources = `all_sources(&settings.source_roots(&store))` (declared, bare, no routing overlay) plus `all_sources(&SourceRoots::conventional())`, refs deduplicated by path. A source the contributor declared `Off` is excluded from both sets: Off is an explicit "do not read" and fail-closed wins over S3's breadth. No trajectory auto-discovery (it depends on the daemon's cwd).
- D5 **Exclusions.** Excluded by path: a queue entry at the path with `submission_id.is_some()` or state `Uploading`/`Uploaded` (withdrawn traces were uploaded, so they fall here), `queue.kept_at_path`, `queue.dismissed_at_path` (an explicit no), and a key in `DaemonState.prior_uploads`. History records carry hashes, not paths, so they are not consulted.
- D6 **Settle rule.** "Still being written" = `now - modified_at < quiescence_secs`, the past-session picker's `DiscoveredSession::quiescent` rule (the watcher's second, size-stability observation needs state the scanner does not keep).
- D7 **One scan kind.** Every trigger runs the same scan (discover all, features through the cache); the cache makes an unchanged trace free, so "re-estimate" and "full scan" are the same code.
- D8 **Held label.** The labelled run is the one that wrote the gate decision (`trace_gate_decisions.pipeline_run_id`), not the latest run, so an in-flight re-run never relabels a settled trace. No Score outcome (a Score hold parks the run without one), a `trace_credit` settlement row with `operation_state = 'held'`, or a run that is not settled (`pipeline_runs.state <> 'complete'`) is excluded. A settled run with a `trace_credit` row labels `atomic_units`, as the spec table reads; a settled run with no row labels 0.
- D9 **Enumeration.** Settle writes a `trace_gate_decisions` row per run, so the settlement target reuses `list_recent_gate_decision_keys` for the cross-tenant enumeration and decision times, and adds one tenant-scoped `Database` method for labels. No migration.
- D10 **Seed.** `CreditEstimateEvalQuery` gains `seed: Option<u64>` (default 0) and `target`; the batch draw uses `rand::rngs::StdRng::seed_from_u64(seed)` (rand 0.8 is already a server dependency).
- D11 **Notified figures are not in the attention log.** `AttentionEntry` is `Copy`, serde-pinned and pruned after `ATTENTION_LOG_RETENTION`; a pruned baseline would re-notify. They live in a new `DaemonState.settlement_notified: Option<SettlementNotified>` (`#[serde(default)]`). The attention log still records each announcement under `Kind::SettlementEstimate`.
- D12 **First run.** The daemon has no first-run state. The first drawable estimate is never a candidate: it silently seeds `settlement_notified`. The 25% / $1 rule fires only against a prior record. Stronger than the spec; satisfies it everywhere.
- D13 **Icon state.** `nudge::mark` has two lit states with disjoint `decisions_owed` preconditions. The estimate lights `MarkState::News` with `Kind::SettlementEstimate`, only when `decisions_owed == 0`, after verdict news, lowest precedence. With decisions owed the badge is the signal and the halo rule is unchanged.
- D14 **Mute.** The kind is muted by `notify.settlement_estimate` (new, default true) and by the master switch; the estimate switch `settlement_estimate_enabled` (new, default true) hides every surface.
- D15 **S8 and the points band.** `estimate_is_drawn` returns false always, so the v1 `credit_estimate` (kept on the wire and still fetched) and the batch clause (`NUDGE_ESTIMATE_CLAUSE*`) are never drawn. Rows gain `settlement_estimate`. The suggested-order tier keeps reading v1.
- D16 **Who formats dollars.** The daemon composes `text` on `status.settlement_estimate` (as `status.nudge.text`), and the notification title and body; the C ABI composes a row's band in `tc_nudge_entry_tags_json`. Fixed strings reach shells through `tc_nudge_copy_json`. No new C ABI function.
- D17 **Copy before its consumers.** The harness listed copy seventh; its stronger rule is that each task builds only on earlier ones, and the status line and notification consume copy. Copy and its pure renderers are task 4; the C ABI change rides with the per-entry task (8).
- D18 **Banned-word test.** `no_string_uses_a_banned_word` bans "worth" and "$". S1 makes these figures money, and the spec gives the switch label with "worth". The test gains a one-entry exemption list (`SETTING_SETTLEMENT_ESTIMATE`, "worth"); no template contains "$" (the figure fill supplies it). `every_estimate_figure_is_labelled_an_estimate` accepts "estimate" or "about" and its count is updated.
- D19 **Not now.** A settlement notification's "Not now" is a plain dismiss in the shell and sends nothing. The daemon's `nudge_decline` refuses kind `settlement_estimate` (not a `NudgeKind`), so no cooldown can be written.

## Review Focus

1. A huge machine (tens of thousands of traces, a 2 GB transcript): the sum must not overflow and the scan must not block the tick. Aggregate in `u128`; the scan runs on `spawn_blocking` and stops between traces when the switch goes off. Test: `settlement_total_does_not_overflow_at_u64_means` (task 1).
2. A table whose `published_at` is in the future, or a clock that jumps: never drawn past `SETTLEMENT_TABLE_MAX_FUTURE_SECS`. Test: `a_far_future_table_is_not_drawable` (task 1).
3. A fit whose evaluation split is all zero awards (p10 ratio 0): the candidate would be refused by the parser, so the fit reports `no_table_reason = "aggregate_band_low_zero"` instead of returning an unparseable table. Test: `an_all_zero_eval_split_returns_no_table` (task 3).
4. The switch turned off while a scan is running: the scan's result must not be published and the cache must stay deleted. Test: `turning_off_mid_scan_publishes_nothing_and_leaves_no_cache` (task 6).
5. A rounding wobble (a re-estimate moving the total by one cent around a $0.50 step): `EVENT_SETTLEMENT_ESTIMATE_CHANGED` fires only when the rounded display changes. Test: `an_unrounded_change_publishes_no_event` (task 6).

---

### Task 1: Protocol settlement table v2 and aggregate math

**Files:**
- Create: `crates/trace-commons-protocol/src/settlement_estimate.rs`
- Modify: `crates/trace-commons-protocol/src/lib.rs` (add `pub mod settlement_estimate;`)
- Modify: `crates/trace-commons-protocol/src/local_credit_estimate.rs` (extract `tier_index`, used by `estimate`)

**Interfaces:**
- Consumes: `LocalEstimateFeatures`, `LocalEstimateTable`, `EstimateWeight`, `EstimateTier`, `estimate_score`, `ESTIMATE_MAX_TIERS`, `ESTIMATE_MAX_WEIGHTS`, `ESTIMATE_MAX_CHUNK_PARAMETER`, `LOCAL_ESTIMATE_FEATURES_VERSION` from `local_credit_estimate`.
- Produces (all `pub` in `trace_commons_protocol::settlement_estimate`):
  - consts `SETTLEMENT_TABLE_SCHEMA_VERSION: u32 = 2`, `SETTLEMENT_TABLE_PATH: &str = "/v1/credit-estimate/settlement-table"`, `SETTLEMENT_INSTRUMENT_ID: &str = "trace_credit"`, `SETTLEMENT_TABLE_MAX_BYTES: usize = 16 * 1024`, `SETTLEMENT_TABLE_MAX_AGE_SECS: i64 = 604_800`, `SETTLEMENT_TABLE_MAX_FUTURE_SECS: i64 = 86_400`, `USD_MICROS_PER_DOLLAR: u64 = 1_000_000`, `FINE_ROUNDING_BELOW_USD_MICROS: u64 = 10_000_000`, `FINE_STEP_USD_MICROS: u64 = 500_000`.
  - `struct SettlementTier { mean_atomic_units: u64, withheld_share_micros: u32 }`, `struct AggregateBand { low_ratio_micros: u64, high_ratio_micros: u64 }` (both `Debug, Clone, PartialEq, Serialize, Deserialize`).
  - `struct SettlementEstimateTable` with the wire fields; `pricing_version: Option<String>` and `usd_micros_per_credit: Option<u64>` skip-serialize when `None`; `published_at: DateTime<Utc>`; `fit_rows: u64`; `#[serde(deny_unknown_fields)]`.
  - `enum SettlementTableError { Malformed, TooLarge, UnsupportedSchema, FeaturesVersion, Label, Instrument, ChunkParameters, Tiers, TierMean, CutOffs, Weights, AggregateBand, AtomicUnits, OutOfRange }` (`thiserror`, label-only messages).
  - `impl SettlementEstimateTable { fn from_slice(&[u8]) -> Result<Self, SettlementTableError>; fn from_value(&Value) -> Result<Self, _>; fn validate(&self) -> Result<(), _>; fn drawable_at(&self, now: DateTime<Utc>) -> bool; fn scoring_view(&self) -> LocalEstimateTable; fn tier_index(&self, f: &LocalEstimateFeatures) -> Option<usize>; fn expected_atomic_units(&self, f: &LocalEstimateFeatures) -> Option<u64> }`.
  - `struct SettlementTotal { low_usd_micros: u64, high_usd_micros: u64, known: u64, unknown: u64 }` with `fn up_to(&self) -> bool` (rounded low is 0) and `fn mid_usd_micros(&self) -> u64` (`(low + high) / 2`).
  - `struct SettlementEntryEstimate { low_usd_micros: u64, high_usd_micros: u64, tier: Option<EstimateTier> }` (`Serialize`).
  - `fn settlement_total<'a>(features: impl IntoIterator<Item = Option<&'a LocalEstimateFeatures>>, table: &SettlementEstimateTable, now: DateTime<Utc>) -> Option<SettlementTotal>` (None when not drawable or `known == 0`).
  - `fn entry_estimate(features: &LocalEstimateFeatures, table: &SettlementEstimateTable, now: DateTime<Utc>) -> Option<SettlementEntryEstimate>`.
  - `fn round_usd_band(low_usd_micros: u128, high_usd_micros: u128) -> (u64, u64)`.
  - In `local_credit_estimate`: `pub fn tier_index(score: f64, cut_offs: &[f64]) -> usize`.

- [ ] **Step 1: Capture the baseline (once for the whole plan)**

```bash
W=/Users/zakimanian/code/trace-commons-server/.claude/worktrees/settlement-estimate-spec
cargo test --manifest-path $W/Cargo.toml -p trace-commons-protocol 2>&1 | grep 'test result:'
cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor 2>&1 | grep 'test result:'
cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor-ffi 2>&1 | grep 'test result:'
RUSTFLAGS="-D warnings" cargo test --manifest-path $W/Cargo.toml -p trace-commons-server --no-run 2>&1 | tail -3
cargo test --manifest-path $W/Cargo.toml -p trace-commons-server --lib credit_estimate_fit 2>&1 | grep 'test result:'
cargo test --manifest-path $W/Cargo.toml -p trace-commons-server --bin trace-commons-ingest credit_estimate_tests 2>&1 | grep 'test result:'
cargo build --manifest-path $W/Cargo.toml -p trace-commons-contributor-ffi && swift test --package-path $W/macos 2>&1 | tail -5
```

Record every `test result:` line (passed / failed / ignored counts) in the task report; later tasks compare against these. Do not pipe through `head`.

- [ ] **Step 2: Write the failing tests** in `settlement_estimate.rs`'s `#[cfg(test)] mod tests`

Fixture `table_a()`: the spec's wire shape with `weights: [{"term": "ln_content_bytes", "weight": 1.0}]`, `cut_offs: [5.0]`, `atomic_units_per_credit: 1_000_000`, `usd_micros_per_credit: 1_000_000`, tiers `[{1_000_000, 300_000}, {4_000_000, 100_000}]`, band `{600_000, 1_500_000}`, `published_at` = `T0` (`2026-10-20T00:00:00Z`), `version: "s3"`, `pricing_version: "p2"`. Features helper `f(content_bytes)` with version `lef1`; `f(100)` scores `ln(101) ≈ 4.6` (tier 0), `f(1000)` scores `ln(1001) ≈ 6.9` (tier 1).

```rust
#[test] fn each_refusal_rule_refuses() {
    // one mutation of table_a() per rule; assert the named error:
    // schema_version 1 -> UnsupportedSchema; features_version "lef2" -> FeaturesVersion;
    // one tier -> Tiers; four tiers -> Tiers; a mean of 0 -> TierMean;
    // cut_offs [5.0, 5.0] with three tiers -> CutOffs;
    // band {1_500_000, 600_000} -> AggregateBand; low_ratio_micros 0 -> AggregateBand;
    // instrument_id "other" -> Instrument; atomic_units_per_credit 0 -> AtomicUnits;
    // a document padded past 16 KiB through from_slice -> TooLarge.
}
#[test] fn a_table_without_a_rate_parses_but_is_not_drawable() // usd_micros_per_credit absent: Ok, drawable_at(T0) == false; Some(0): also not drawable
#[test] fn a_stale_table_is_not_drawable() // drawable_at(T0 + 7d - 1s) true; drawable_at(T0 + 7d) false
#[test] fn a_far_future_table_is_not_drawable() // drawable_at(T0 - 1d - 1s) false; drawable_at(T0 - 1d) true
#[test] fn tier_assignment_matches_v1_for_the_same_weights_and_cut_offs() // a v1 LocalEstimateTable with the same weights, cut_offs and two bands: estimate(f, &v1).tier index == table_a().tier_index(f) for f(100), f(1000), f(147) (ln 148 ≈ 4.997, tier 0) and f(148) (ln 149 ≈ 5.004, tier 1)
#[test] fn the_sum_band_and_dollars_are_hand_computed() {
    // [f(100), f(100), f(1000)] -> sum 6 credits = $6: low $3.60 -> 3_500_000, high $9.00 -> 9_000_000
    // [f(100) x3, f(1000)]      -> $7: high $10.50 crosses $10, whole dollars: (4_000_000, 11_000_000)
}
#[test] fn a_zero_low_end_reads_up_to() // usd 100_000/credit, one f(100): $0.10 -> (0, 500_000), up_to() true
#[test] fn unknown_traces_are_counted_and_contribute_nothing() // [Some(f(100)), None, Some(f(0)), Some(lef2 features)] -> known 1, unknown 3, same dollars as [f(100)] alone
#[test] fn nothing_is_drawn_without_a_known_trace() // all None -> settlement_total == None
#[test] fn an_entry_band_uses_the_batch_ratios() // entry_estimate(f(1000)) -> (2_000_000, 6_000_000), tier Some(Higher)
#[test] fn settlement_total_does_not_overflow_at_u64_means() // tier mean u64::MAX, 10_000 traces -> Some, high_usd_micros == u64::MAX (saturated), no panic
```

- [ ] **Step 3: Run to verify they fail**

Run: `cargo test --manifest-path $W/Cargo.toml -p trace-commons-protocol settlement_estimate`
Expected: compile failure (module and types do not exist).

- [ ] **Step 4: Implement**

- Extract `tier_index` in `local_credit_estimate.rs` and call it from `estimate`; v1 tests stay green.
- `from_slice`: refuse `len > SETTLEMENT_TABLE_MAX_BYTES` with `TooLarge` before parsing; then `from_value`. `from_value` reads `schema_version` first (as v1 does), bounds `weights` and `tiers` array lengths before deserializing, then `validate`.
- `validate` applies the spec refusals plus v1's label rule (`valid_label`, also for `pricing_version` when present), chunk-parameter range, weight rules (no repeated term, `|w| <= ESTIMATE_MAX_ABS_WEIGHT`), `withheld_share_micros <= 1_000_000`, `cut_offs.len() == tiers.len() - 1`, finite cut-offs.
- `drawable_at`: rate is `Some(r)` with `r > 0`, and `-SETTLEMENT_TABLE_MAX_FUTURE_SECS <= (now - published_at).num_seconds() < SETTLEMENT_TABLE_MAX_AGE_SECS`.
- `tier_index`: `None` when `features.version != features_version` or `content_bytes == 0` or `estimate_score(features, &self.scoring_view())` is `None`; else `local_credit_estimate::tier_index(score, &self.cut_offs)`.
- Dollar math in `u128` throughout: `atomic * usd_micros_per_credit / atomic_units_per_credit`, then `* ratio_micros / 1_000_000`. `round_usd_band`: when `high < FINE_ROUNDING_BELOW_USD_MICROS` floor low and ceil high to `FINE_STEP_USD_MICROS`, else to `USD_MICROS_PER_DOLLAR`; saturate to `u64::MAX`.

- [ ] **Step 5: Run the tests and the permissive build**

Run: `cargo test --manifest-path $W/Cargo.toml -p trace-commons-protocol` then `RUSTFLAGS="-D warnings" cargo check --manifest-path $W/Cargo.toml -p trace-commons-protocol --no-default-features` and clippy on `-p trace-commons-protocol`.
Expected: the 11 new tests pass; total = baseline + 11; check and clippy clean.

- [ ] **Step 6: Commit**

```bash
git -C $W add crates/trace-commons-protocol
git -C $W commit -m "Add the settlement estimate table and its machine-wide total" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

### Task 2: Server settlement-table route and env

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/credit_estimate.rs` (loader + handler)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (`AppState` field near line 1706, boot near 5036, route beside `/v1/credit-estimate/table` near 8639)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (the two `test_state` literals near 6204 and 30233; tests in `mod credit_estimate_tests`)
- Modify: `docs/operator/README.md` or the runbook that lists ingest env vars, if one lists `TRACE_COMMONS_CREDIT_ESTIMATE_TABLE_PATH` (grep first; add the new var beside it only if the old one is documented)

**Interfaces:**
- Consumes: `SettlementEstimateTable::from_slice`, `SETTLEMENT_TABLE_PATH` (task 1).
- Produces: `AppState.credit_settlement_table: Option<Arc<SettlementEstimateTable>>`; `pub(super) const CREDIT_SETTLEMENT_TABLE_PATH_ENV: &str = "TRACE_COMMONS_CREDIT_SETTLEMENT_TABLE_PATH"`; `pub(super) fn settlement_table_from_path(path: Option<&Path>) -> anyhow::Result<Option<SettlementEstimateTable>>`; `pub(super) fn settlement_table_from_env() -> anyhow::Result<Option<Arc<SettlementEstimateTable>>>`; `pub(super) async fn settlement_table_handler(State(state)) -> Response`.

- [ ] **Step 1: Write the failing tests** in `mod credit_estimate_tests`

```rust
#[tokio::test] async fn settlement_route_is_404_without_an_installed_table() // default test_state: GET SETTLEMENT_TABLE_PATH -> 404, no auth header sent
#[tokio::test] async fn settlement_route_serves_the_installed_table_unauthenticated() // set the field; 200; SettlementEstimateTable::from_value(&body) == installed; Cache-Control "public, max-age=3600"; X-Content-Type-Options nosniff
#[test] fn settlement_table_from_path_loads_validates_or_refuses() // None -> Ok(None); valid file -> Some; schema_version 1, missing file, and a file over 16 KiB -> err.to_string() == "credit_settlement_table_invalid"
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --manifest-path $W/Cargo.toml -p trace-commons-server --bin trace-commons-ingest credit_estimate_tests`
Expected: compile failure.

- [ ] **Step 3: Implement** the loader (read bytes, `from_slice`, any failure maps to `anyhow!("credit_settlement_table_invalid")`), the handler (404 via `StatusCode::NOT_FOUND.into_response()` when `None`; else JSON with the same two headers as `table_handler`), the `AppState` field set at boot with `settlement_table_from_env()?` (boot fails on an invalid file), the route registered next to the v1 table route outside every auth and tenant layer, and `None` in both test-state literals. Update the module doc to name the new route.

- [ ] **Step 4: Run tests**

Run: the Step 2 command, then `RUSTFLAGS="-D warnings" cargo check --manifest-path $W/Cargo.toml -p trace-commons-server --bins` and server clippy.
Expected: 3 new tests pass, no regressions against the baseline line for `credit_estimate_tests`.

- [ ] **Step 5: Commit** — subject `Serve the settlement estimate table when the operator installs one`.

### Task 3: Eval `target=settlement` and `fit_settlement_table`

**Files:**
- Modify: `crates/trace-commons-server/src/credit_estimate_fit.rs` (settlement fit and its pure tests)
- Modify: `crates/trace-commons-server/src/db/mod.rs` (trait method, default empty), `crates/trace-commons-server/src/db/postgres.rs` (impl)
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (row type beside `GateDecisionKeyRow`)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/credit_estimate.rs` (query fields, settlement branch)
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (`PerplexityDriverTestDb` impl near line 74383 with a seedable map; handler tests)

**Interfaces:**
- Consumes: task 1's `SettlementEstimateTable`, `SettlementTier`, `AggregateBand`; existing `EstimateFitInput` time-split approach, `least_squares`, `merge_under_floor`, `quantile`, `ESTIMATE_FIT_TRAIN_SHARE`, `ESTIMATE_FIT_TERMS`.
- Produces:
  - `pub struct SettlementLabelRow { pub submission_id: Uuid, pub label: SettlementLabel }`, `pub enum SettlementLabel { Settled(u64), WithheldAtZero, Excluded }` in `trace_corpus_storage.rs`.
  - `Database::list_settlement_labels(&self, tenant_id: &str, submission_ids: &[Uuid]) -> Result<Vec<SettlementLabelRow>, DatabaseError>` (default `Ok(Vec::new())`). PostgreSQL impl on the tenant-scoped pool (`tenant_transaction`): per submission, the `pipeline_runs` row named by its latest `trace_gate_decisions.pipeline_run_id` (D8); LEFT JOIN `phase_outcomes` with `phase = 'score'`; LEFT JOIN `pipeline_run_settlements` with `instrument_id = 'trace_credit'`, reading `atomic_units::TEXT`. No such run, no score outcome, run `state <> 'complete'`, or `operation_state = 'held'`: `Excluded`; row: `Settled(parse)`; no row: `WithheldAtZero`. A value that does not parse as `u64`: `Excluded`.
  - `pub enum EstimateEvalTarget { Display, Settlement }` (`serde(rename_all = "snake_case")`, default `Display`); `CreditEstimateEvalQuery` gains `#[serde(default)] target` and `#[serde(default)] seed: Option<u64>`.
  - `pub struct SettlementEvalRow { pub features: LocalEstimateFeatures, pub settled_atomic_units: u64, pub tenant_tag: String }`, `pub struct SettlementFitInput { pub row: SettlementEvalRow, pub decided_at: DateTime<Utc> }`.
  - `pub const SETTLEMENT_FIT_BATCHES: usize = 200`, `pub const SETTLEMENT_FIT_BATCH_SIZE: usize = 20`.
  - `pub struct SettlementFitReport { pub fit_rows: usize, pub held_out_rows: usize, pub tiers: usize, pub batches: usize, pub batch_size: usize, pub seed: u64, pub bias_ratio: Option<f64>, pub table: Option<SettlementEstimateTable>, pub no_table_reason: Option<&'static str> }` (`Serialize`).
  - `pub fn fit_settlement_table(inputs: &[SettlementFitInput], seed: u64, published_at: DateTime<Utc>) -> SettlementFitReport`.
  - Response gains `#[serde(skip_serializing_if = "Vec::is_empty")] settlement_rows: Vec<SettlementEvalRow>` and `#[serde(skip_serializing_if = "Option::is_none")] settlement_fit: Option<SettlementFitReport>`; counts reuse `labelled` / `unlabelled` (excluded counts as unlabelled).

- [ ] **Step 1: Write the failing pure tests** in `credit_estimate_fit.rs`

```rust
#[test] fn settlement_fit_is_reproducible_from_a_seed() // same inputs, seed 7 twice -> equal reports; seed 8 -> aggregate_band may differ but tiers equal
#[test] fn withheld_rows_count_zero_in_tier_means() // synthetic rows where half of tier 0 is 0: mean_atomic_units == mean over all its rows including zeros; withheld_share_micros == 500_000
#[test] fn the_bias_check_is_reported() // eval split actual sum 3x predicted sum -> bias_ratio Some(3.0) within 1e-9
#[test] fn the_candidate_has_no_rate_and_no_pricing_version_and_parses() // table.usd_micros_per_credit None, pricing_version None; serde round trip through SettlementEstimateTable::from_value is Ok
#[test] fn an_all_zero_eval_split_returns_no_table() // eval split all 0 -> table None, no_table_reason Some("aggregate_band_low_zero")
```

Use synthetic features in two well-separated groups (`content_bytes` in the hundreds and in the tens of thousands, so the fitted weights cannot move a row across tiers) and synthetic awards; at least `2 * ESTIMATE_MIN_TIER_ROWS` rows per tier so tiers survive `merge_under_floor`.

- [ ] **Step 2: Run to verify they fail** — `cargo test --manifest-path $W/Cargo.toml -p trace-commons-server --lib credit_estimate_fit` → compile failure.

- [ ] **Step 3: Implement `fit_settlement_table`**: sort by `decided_at`, split at `ESTIMATE_FIT_TRAIN_SHARE`; fit weights by least squares of `settled_atomic_units` on `ESTIMATE_FIT_TERMS` over the fit split and place cut-offs exactly as `fit_estimate_table` does (reuse its helpers; refactor them to take a label slice if needed, keeping v1 tests green); tier means and withheld shares from the fit split (round means to `u64`; a mean of 0 gives `no_table_reason = "tier_mean_not_positive"`); on the eval split, `StdRng::seed_from_u64(seed)`, `SETTLEMENT_FIT_BATCHES` batches of `min(SETTLEMENT_FIT_BATCH_SIZE, rows)` drawn without replacement within a batch; ratios `actual / predicted`; band = `quantile` p10 / p90 times 1e6, rounded; low 0 gives `"aggregate_band_low_zero"`. Candidate `version` is `"fit"`, `instrument_id` `trace_credit`, `atomic_units_per_credit` 1_000_000 as a named const `SETTLEMENT_FIT_ATOMIC_UNITS_PER_CREDIT` (the pipeline's microcredit unit), `published_at` the argument, `fit_rows` the fit split's size. Validate the candidate with `validate()` before returning it; a refusal becomes `no_table_reason = "candidate_refused"`.

- [ ] **Step 4: Run** the Step 2 command. Expected: 5 new pass, v1 fit tests unchanged.

- [ ] **Step 5: Write the failing handler tests** in `mod credit_estimate_tests`, seeding `PerplexityDriverTestDb` settlement labels for the fixture's four submissions as `Settled(150_000)`, `WithheldAtZero`, `Excluded`, `Settled(600_000)` (the double stores labels already decided; the D8 SQL rules are pinned by the PostgreSQL test in Step 7):

```rust
#[tokio::test] async fn settlement_eval_excludes_held_and_counts_withheld_as_zero() // target Settlement: settlement_rows.len() == 3; sorted settled_atomic_units == [0, 150_000, 600_000]; rows empty; counts.unlabelled includes the excluded one
#[tokio::test] async fn settlement_eval_rows_carry_no_submission_id_or_tenant() // serialized response contains no fixture submission id string and no "tenant-a"/"tenant-b"
#[tokio::test] async fn display_remains_the_default_target() // query without target -> rows populated, settlement_rows empty (today's behaviour)
#[test] fn the_eval_query_refuses_an_unknown_target() // serde_urlencoded "target=points" -> Err
```

- [ ] **Step 6: Run to verify they fail**, then implement: in `run_credit_estimate_eval`, branch on `query.target`; the settlement branch reuses the enumeration, tenant tags, derive step and shuffle, reads `list_settlement_labels` per tenant instead of gate credit labels, skips `Excluded` (counted unlabelled), and with `fit=true` calls `fit_settlement_table(&inputs, query.seed.unwrap_or(0), Utc::now())`. The completion log line adds `target` as a label only. Implement the PostgreSQL method and the test-double method (a `RwLock<BTreeMap<(String, Uuid), SettlementLabel>>` with a `seed_settlement_label` helper).

- [ ] **Step 7: Add a PostgreSQL test for the label SQL** beside `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/pipeline_http_pg_tests.rs` (same fixture helpers, `#[ignore]` as the repo's CI-selector convention for pg suites, self-skipping without `TRACE_COMMONS_PG_TEST_DATABASE_URL`): `settlement_labels_follow_the_settled_run` with four runs: complete with a `trace_credit` row (Settled), complete with none (WithheldAtZero), Score outcome present but run not complete and no row (Excluded), and a Score hold with no outcome (Excluded). Run it locally only if a local `admission_test_*` database is available (see CLAUDE.md); otherwise state in the task report that CI's PostgreSQL job is the first run of this SQL. Never report the SQL green from the test double alone.

- [ ] **Step 8: Run** `cargo test --manifest-path $W/Cargo.toml -p trace-commons-server --bin trace-commons-ingest credit_estimate_tests`, `RUSTFLAGS="-D warnings" cargo test --manifest-path $W/Cargo.toml -p trace-commons-server --no-run`, and server clippy. Expected: 4 new pass; `--no-run` builds.

- [ ] **Step 9: Commit** — subject `Fit a settlement table from settled trace credit on the eval route`.

### Task 4: Core copy and pure renderers for the settlement estimate

**Files:**
- Modify: `crates/trace-commons-contributor/src/nudge_copy.rs`
- Modify: `crates/trace-commons-contributor/src/nudge_render.rs`

**Interfaces:**
- Consumes: `SettlementTotal`, `SettlementEntryEstimate` (task 1).
- Produces (all in `NUDGE_COPY`, each `DRAFT, NEEDS APPROVAL`):
  - `ESTIMATE_TOTAL_LINE` = "About {low} to {high} of inference in traces on this Mac"
  - `ESTIMATE_TOTAL_LINE_UP_TO` = "Up to {high} of inference in traces on this Mac, as an estimate"
  - `ESTIMATE_TOTAL_LABEL` = "Estimate"
  - `ESTIMATE_TOTAL_ACTION` = "Review traces"
  - `NOTIFY_SETTLEMENT_TITLE` = "Estimate for traces on this Mac"
  - `NOTIFY_SETTLEMENT_BODY` = "About {low} to {high} of inference, estimated on this Mac. Nothing was sent."
  - `NOTIFY_SETTLEMENT_BODY_UP_TO` = "Up to {high} of inference, estimated on this Mac. Nothing was sent."
  - `NOTIFY_SETTLEMENT_BODY_PRICING` = "Pricing changed. Traces on this Mac are now estimated at about {low} to {high} of inference."
  - `NOTIFY_SETTLEMENT_ACTION_REVIEW` = "Review traces", `NOTIFY_SETTLEMENT_ACTION_NOT_NOW` = "Not now"
  - `MARK_A11Y_SETTLEMENT` = "An estimate for traces on this Mac is ready.", `MARK_TOOLTIP_SETTLEMENT` = "Estimate ready"
  - `ENTRY_ESTIMATE_BAND` changes to "Estimate: about {low} to {high} of inference"; new `ENTRY_ESTIMATE_BAND_UP_TO` = "Estimate: up to {high} of inference"
  - `SETTLEMENT_ESTIMATE_EXPLAINER` = "Made on this Mac from the traces' size and makeup. Nothing was sent to make it. The rate comes from Trace Commons' published pricing. What a trace actually gets is decided when it is scored. A single trace's figure is less certain than the total."
  - `SETTING_SETTLEMENT_ESTIMATE` = "Estimate what traces on this Mac are worth", `SETTING_SETTLEMENT_ESTIMATE_HELP` = "Worked out on this Mac from your traces' size and makeup. Nothing is sent to make the estimate."
  - `nudge_render::usd_text(usd_micros: u64) -> String` ("$4", "$4.50", "$0.50", "$12"); `nudge_render::settlement_total_line(total: &SettlementTotal) -> String`; `nudge_render::settlement_notification(total: &SettlementTotal, pricing_changed: bool) -> NotificationText` (actions `review` and `not_now`, through the existing `action` helper and `ACTION_*` ids); `nudge_render::settlement_mark_text() -> MarkText` (or extend the existing mark-text composer with a `Kind::SettlementEstimate` arm in task 7 if mark text is composed per kind; read `nudge_render.rs` and pick the one that matches); `nudge_render::entry_usd_band(e: &SettlementEntryEstimate) -> String`.

These are first drafts written to the copy guide; they go to owner and design review after the UI is built. The plan fixes their keys and placeholders, not their final wording.

- [ ] **Step 1: Write the failing tests**

```rust
// nudge_render tests
#[test] fn usd_text_drops_cents_on_whole_dollars() // usd_text(4_000_000) == "$4"; 4_500_000 -> "$4.50"; 500_000 -> "$0.50"; 12_000_000 -> "$12"
#[test] fn the_total_line_is_about_or_up_to() // (3_500_000, 9_000_000) -> "About $3.50 to $9 of inference in traces on this Mac"; (0, 500_000) -> starts "Up to $0.50"
#[test] fn the_settlement_notification_has_core_words_and_two_actions() // titles/bodies filled; pricing_changed picks NOTIFY_SETTLEMENT_BODY_PRICING; actions ids == ["review", "not_now"]
#[test] fn an_entry_band_is_dollars_or_up_to() // (2_000_000, 6_000_000) -> "Estimate: about $2 to $6 of inference"; (0, 500_000) -> "Estimate: up to $0.50 of inference"
// nudge_copy tests (edit the existing ones)
// every_estimate_figure_is_labelled_an_estimate: accept "estimate" or "about" (case-insensitive) and update the count to the new number of {low}/{high} strings
// no_string_uses_a_banned_word: add `const EXEMPT: &[(&str, &str)] = &[("SETTING_SETTLEMENT_ESTIMATE", "worth")];` and skip exactly those pairs
// every_notification_body_fits_the_windows_clamp_at_maximal_counts: add the three settlement bodies filled with "$99,999" at both ends
// every_counted_string_has_a_singular_or_is_count_neutral: list the new strings as count-neutral
```

- [ ] **Step 2: Run to verify they fail** — `cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor nudge_` → compile failures.

- [ ] **Step 3: Implement** the constants (added to `NUDGE_COPY` and to the module doc's placeholder list: `{low}` / `{high}` in a settlement string are dollar figures from `usd_text`) and the renderers. Amounts above $999 use no thousands separator unless a test above requires it; keep `usd_text` integer-only (no floats).

- [ ] **Step 4: Run** `cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor nudge_` then the full contributor suite. Expected: new tests pass; the pre-existing entry-band test in `nudge_render` (`a_row_draws_its_estimate_only_when_the_core_says_to`) is the only expected change and is updated in task 8, so if it fails here because `ENTRY_ESTIMATE_BAND` changed, update its expected string now to the dollar wording with the same numbers and note it.

- [ ] **Step 5: Commit** — subject `Word the settlement estimate in core copy`.

### Task 5: Feature cache and the estimation scanner

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/settlement_scan.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs` (`pub mod settlement_scan;`)
- Modify: `crates/trace-commons-contributor/src/config.rs` (`pub const DAEMON_SETTLEMENT_FEATURES_FILE: &str = "daemon-settlement-features.json";`)

**Interfaces:**
- Consumes: `TraceSource`, `SessionRef`, `ProjectPolicy::resolve`, `ProjectMode`, `policy::project_for`, `CwdCacheEntry`, `queue::Queue` (`kept_at_path`, `dismissed_at_path`, entries by path), `queue::estimate_features_of`, `ConfigStore::{read,write,remove}_daemon_file`.
- Produces:
  - `pub fn path_hash(source: &str, path: &Path) -> String` (`"sha256:" + hex(sha256(source || 0x00 || path bytes))`).
  - `pub struct CachedFeatures { pub source: String, pub size_bytes: u64, pub modified_at: DateTime<Utc>, pub features: Option<LocalEstimateFeatures> }` (`Serialize, Deserialize`).
  - `pub struct FeatureCache { entries: BTreeMap<String, CachedFeatures> }` with `load(store: &ConfigStore) -> Self` (unreadable or malformed file: empty), `save(&self, store) -> anyhow::Result<()>`, `delete(store) -> anyhow::Result<()>`, `len(&self) -> usize`.
  - `pub struct ScanInputs<'a> { pub policy: &'a ProjectPolicy, pub queue: &'a Queue, pub prior_uploads: &'a BTreeMap<String, PriorUpload>, pub cwd_cache: &'a BTreeMap<String, CwdCacheEntry>, pub quiescence_secs: u64, pub now: DateTime<Utc> }`.
  - `pub struct ScanOutcome { pub features: Vec<Option<LocalEstimateFeatures>>, pub skipped_sources: Vec<&'static str>, pub ignored: usize, pub excluded: usize, pub not_settled: usize, pub no_project: usize, pub cache_hits: usize, pub cache_misses: usize, pub stopped: bool }`.
  - `pub fn scan(sources: &[Box<dyn TraceSource>], inputs: &ScanInputs<'_>, cache: &mut FeatureCache, should_stop: &dyn Fn() -> bool) -> ScanOutcome`.
  - `pub fn estimation_sources(declared: &SourceRoots) -> Vec<Box<dyn TraceSource>>` (D4).

`scan` takes `&Queue`, not `&mut Queue` and not `DaemonShared`: that is the structural proof that it cannot enqueue, preview or upload (S4).

Per ref, in this order: dedupe by path; excluded (D5) → `excluded += 1`, skip; resolve the project key (D3) — none → `no_project += 1`, push `None`; `policy.resolve(key) == Ignore` → `ignored += 1`, skip, nothing opened; not settled (D6) → `not_settled += 1`, skip; cache hit on `(path_hash, source, size, mtime)` → push cached; else `source.load(ref)` → `estimate_features_of(&transcript)` (a load error caches `features: None`), `cache_misses += 1`. After the loop, drop cache entries whose key was not seen this scan. Between refs, `if should_stop() { stopped = true; break }` and `std::thread::yield_now()`. A discover error pushes the source name to `skipped_sources`. No logging inside `scan`.

- [ ] **Step 1: Write the failing tests** with a `DoubleSource` (name, refs it returns, a set of paths whose `load` panics with "opened an Ignore folder", a counter of loads)

```rust
#[test] fn an_ignore_folder_is_never_opened() // policy sets the ref's cwd project to Ignore; load panics if called; outcome.ignored == 1, features empty
#[test] fn the_global_never_override_is_never_opened() // folder mode NotifyOnly but contribution override Ignore; same assertions
#[test] fn a_ref_without_a_cheap_cwd_is_unknown_and_unopened() // cwd None, no cwd_cache hit: load panics if called; features == [None], no_project == 1
#[test] fn uploaded_kept_dismissed_and_prior_uploads_are_excluded() // four refs, one per D5 rule, plus one waiting; features.len() == 1, excluded == 4
#[test] fn a_trace_still_being_written_is_not_counted_yet() // modified now - quiescence + 1s -> not_settled == 1
#[test] fn the_cache_hits_on_an_unchanged_key_and_misses_on_an_mtime_change() // scan twice: second has cache_hits 1, loads 1 total; bump mtime: cache_misses 1, loads 2
#[test] fn the_cache_holds_no_path_or_content() // saved file bytes contain no path string and no transcript text; keys start "sha256:"
#[test] fn the_scanner_cannot_enqueue() // queue with N entries before; after scanning 3 fresh settled refs, queue.all().len() == N and identical
#[test] fn a_failing_source_is_skipped_by_name() // discover Err -> skipped_sources == ["double"], other source still scanned
#[test] fn should_stop_ends_the_scan_between_traces() // stop after first -> stopped true, features.len() == 1
```

- [ ] **Step 2: Run to verify they fail** — `cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor settlement_scan` → compile failure.

- [ ] **Step 3: Implement** per the Interfaces block.

- [ ] **Step 4: Run** the Step 2 command, then the full contributor suite and clippy. Expected: 10 new pass; no regressions.

- [ ] **Step 5: Commit** — subject `Scan this Mac's traces for an estimate without queueing any`.

### Task 6: Table fetch, re-estimate triggers, status, event and the off switch

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/settlement_estimate.rs`
- Modify: `crates/trace-commons-contributor/src/credit_estimate_table.rs` (settlement fetch)
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs` (`pub mod settlement_estimate;`; fetch inside `refresh_estimate_table`; `settlement_estimate::tick` in the poll arm after `watcher::tick`)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (`DaemonShared.settlement: Mutex<SettlementEstimateState>`, `EVENT_SETTLEMENT_ESTIMATE_CHANGED`, `status_value_at`, `set_settings` path)
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs` (`settlement_estimate_enabled`, default true, `apply_settings_object` arm beside `"suggestions_enabled"`)
- Modify: `crates/trace-commons-contributor/src/daemon/watcher.rs` (note a change after a pass that queued or superseded at least one entry)
- Modify: `crates/trace-commons-contributor/src/daemon/policy.rs` or the IPC handler that sets a project mode / the contribution override (request an immediate re-estimate when the resolved mode moves to or from `Ignore`)

**Interfaces:**
- Consumes: tasks 1, 4, 5.
- Produces:
  - In `credit_estimate_table.rs`: `impl EstimateTableClient { pub async fn fetch_settlement(&self) -> Result<SettlementEstimateTable, EstimateTableClientError> }` (same request rules as `fetch`: no credential, no redirect, size cap `SETTLEMENT_TABLE_MAX_BYTES`); `pub enum SettlementFetchEffect { Replace, Clear, Keep }` with `of(&Result<..>)`: `Ok` → Replace; `Refused | ResponseTooLarge | NotFound` → Clear; anything else → Keep.
  - In `settlement_estimate.rs`: consts `RESCAN_DEBOUNCE = Duration::minutes(10)`, `RESCAN_MIN_INTERVAL = Duration::hours(1)`, `RESCAN_DAILY = Duration::hours(24)`; `pub struct RescanSchedule` with `note_sessions_changed(&mut self, now)`, `request_immediate(&mut self)`, `due(&self, now) -> bool`, `started(&mut self, now)`; `pub struct SettlementShown { pub total: SettlementTotal, pub table_version: String, pub pricing_version: Option<String>, pub estimated_at: DateTime<Utc> }`; `pub struct SettlementEstimateState { pub table: Option<SettlementEstimateTable>, pub schedule: RescanSchedule, pub shown: Option<SettlementShown>, pub scanning: bool, pub generation: u64 }`; `pub(crate) fn apply_table_fetch(shared, outcome)`; `pub(crate) fn tick(shared: &Arc<DaemonShared>, now)`; `pub(crate) fn set_enabled(shared, on: bool)`; `pub(crate) fn status_value(shared, now) -> Option<serde_json::Value>`.
  - `pub const EVENT_SETTLEMENT_ESTIMATE_CHANGED: &str = "settlement_estimate_changed";` (payload `{}`), also published with `EVENT_STATUS_CHANGED`.
  - `status.settlement_estimate` (present only when drawable, the switch is on, and `known > 0`): `{"low_usd_micros", "high_usd_micros", "up_to", "known", "unknown", "table_version", "pricing_version", "estimated_at", "text"}` with `text = nudge_render::settlement_total_line(&total)`.

Rules:
- `due`: `immediate`, or never scanned, or (`changed_at` set and `now - changed_at >= RESCAN_DEBOUNCE` and `now - last_scan_at >= RESCAN_MIN_INTERVAL`), or `now - last_scan_at >= RESCAN_DAILY`. `started` clears `immediate` and `changed_at`.
- `apply_table_fetch`: a `Replace` whose `(version, pricing_version)` differs from the table in force, or a `Clear` that removed one, calls `request_immediate()`; a `Clear` also drops `shown` and publishes.
- `tick`: returns at once when the switch is off, a scan is running, or not `due`. Otherwise sets `scanning`, snapshots policy, queue, `prior_uploads`, `cwd_cache`, quiescence and the source roots under short locks (none nested), bumps `generation`, and runs `settlement_scan::scan` inside `tokio::task::spawn_blocking` with a `should_stop` that reads the switch and shutdown flag. On completion it discards the result if the switch is off or `generation` moved; else saves the cache, computes `settlement_total` with the table in force at completion, and when the rounded `(low, high, up_to)` or table versions differ from `shown`, replaces `shown` and publishes the event. Log one label-only line: counts only (`known`, `unknown`, `ignored`, `excluded`, `skipped_sources` count).
- `set_enabled(false)`: bump `generation`, clear `shown`, `FeatureCache::delete`, publish when something was shown. `set_enabled(true)`: `request_immediate()`.
- Settlement fetch rides the existing call: inside `refresh_estimate_table`, after the v1 fetch, the same client calls `fetch_settlement` (skip it when the switch is off). No new schedule.

- [ ] **Step 1: Write the failing tests** (unit tests in `settlement_estimate.rs` with a `test_support` `DaemonShared`; client tests in `credit_estimate_table.rs` with the axum `serve` helper already there)

```rust
#[tokio::test] async fn the_settlement_request_is_exact_and_carries_no_credentials() // path SETTLEMENT_TABLE_PATH, no authorization/cookie/x-api-key/x-tenant-id
#[tokio::test] async fn a_404_or_refused_settlement_table_clears_and_an_outage_keeps() // NotFound and Refused -> Clear; 503 -> Keep
#[test] fn the_debounce_waits_ten_minutes_and_at_most_hourly() // scan at t; change at t+5m: due false at t+14m, false at t+59m, true at t+60m; change at t+2h: due at t+2h10m, not before
#[test] fn the_daily_scan_is_due_without_a_change() // last scan t: due at t+24h
#[tokio::test] async fn a_version_or_pricing_change_rescans_immediately() // apply Replace with pricing_version p3 over p2 -> due(now) true; identical table -> false
#[tokio::test] async fn a_mode_change_to_or_from_never_rescans_immediately() // set a project's mode to Ignore through the IPC handler -> schedule.due(now)
#[tokio::test] async fn status_carries_the_estimate_only_when_drawable() // no table -> no key; table without rate -> no key; drawable + 3 known -> key with text "About ..."
#[tokio::test] async fn an_unrounded_change_publishes_no_event() // two totals rounding to the same band -> one EVENT_SETTLEMENT_ESTIMATE_CHANGED, not two
#[tokio::test] async fn the_off_switch_deletes_the_cache_and_hides_the_estimate() // set_settings {"settlement_estimate_enabled": false}: cache file gone, status has no key, event published
#[tokio::test] async fn turning_off_mid_scan_publishes_nothing_and_leaves_no_cache() // generation bumped while scan runs -> result discarded, file absent
#[tokio::test] async fn turning_on_requests_a_full_scan() // false then true -> due(now)
#[tokio::test] async fn a_scan_and_estimate_make_no_request_but_the_table_fetch() // config ingest_url, privacy-filter and inference endpoints all point at one axum router that records every path and answers 500 to anything but SETTLEMENT_TABLE_PATH and the v1 table path; run refresh_estimate_table (due) and settlement_estimate::tick to completion over a fixture source with settled traces; recorded paths ⊆ {ESTIMATE_TABLE_PATH, SETTLEMENT_TABLE_PATH}, and the queue length is unchanged
#[test] fn settings_round_trip_the_estimate_switch() // default true; a file without the key loads true; apply false persists
```

- [ ] **Step 2: Run to verify they fail** — `cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor settlement` → compile failures.

- [ ] **Step 3: Implement** per the Interfaces and Rules blocks. The watcher's change note is one call (`shared.settlement.lock()...schedule.note_sessions_changed(now)`) after a pass that queued or superseded an entry; the mode hook compares `policy.resolve(key)` before and after the change.

- [ ] **Step 4: Run** the Step 2 command, the full contributor suite, `RUSTFLAGS="-D warnings" cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor --no-run`, and clippy. Expected: 13 new pass; no regressions against baseline.

- [ ] **Step 5: Commit** — subject `Keep a machine-wide settlement estimate on status`.

### Task 7: Attention kind, notification thresholds and the news mark

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/attention.rs` (`Kind::SettlementEstimate`, `Fact::EstimateAnnounced`, caps, `NotifySettings.settlement_estimate`)
- Modify: `crates/trace-commons-contributor/src/daemon/settings.rs` (`NotifyKinds.settlement_estimate`, default true via a `default_notify_settlement_estimate` fn; `notify_snapshot`; the settings-object key `notify.settlement_estimate`)
- Modify: `crates/trace-commons-contributor/src/daemon/state.rs` (`settlement_notified`; the state.rs label pin test gains the new kind label)
- Modify: `crates/trace-commons-contributor/src/daemon/settlement_estimate.rs` (pure candidacy)
- Modify: `crates/trace-commons-contributor/src/daemon/reengage.rs` (candidate, notification, record)
- Modify: `crates/trace-commons-contributor/src/daemon/nudge.rs` (`MarkInputs.settlement_lit`, `mark`)
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (fill `settlement_lit`; mark text for the kind)

**Interfaces:**
- Consumes: tasks 4 and 6 (`SettlementShown`, `settlement_notification`, mark copy).
- Produces:
  - `Kind::SettlementEstimate` (label `"settlement_estimate"`, precedence 4, `Kind::ALL: [Kind; 5]`), `default_caps` → `KindCaps { min_interval: N5_MIN_INTERVAL, per_week: Some(N5_STANDALONE_PER_WEEK), counts_folds: false }` with `pub const N5_MIN_INTERVAL: Duration = Duration::hours(72)` and `pub const N5_STANDALONE_PER_WEEK: u32 = 1` (DRAFT, owner decision pending, like the other caps).
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)] pub struct SettlementNotified { pub low_usd_micros: u64, pub mid_usd_micros: u64, pub pricing_version: Option<String>, pub at: DateTime<Utc> }`; `DaemonState.settlement_notified: Option<SettlementNotified>` (`#[serde(default, skip_serializing_if = "Option::is_none")]`).
  - `pub const SETTLEMENT_NOTIFY_GROWTH_MICROS: u64 = 250_000` (25%), `pub const SETTLEMENT_NOTIFY_MIN_RISE_USD_MICROS: u64 = 1_000_000`, `pub const SETTLEMENT_MARK_MIN_LOW_USD_MICROS: u64 = 1_000_000`.
  - `pub enum SettlementCandidacy { Candidate { pricing_changed: bool }, SeedBaseline, Quiet }`; `pub fn settlement_candidacy(shown: &SettlementShown, last: Option<&SettlementNotified>) -> SettlementCandidacy` (pure; integer arithmetic: `low * 1_000_000 >= last_low * 1_250_000`, `low >= last_low + 1_000_000`; pricing: `shown.pricing_version != last.pricing_version` and `|mid - last_mid| * 1_000_000 >= last_mid * 250_000`; `last == None` → `SeedBaseline`).
  - `MarkInputs.settlement_lit: bool` = shown and `low_usd_micros >= SETTLEMENT_MARK_MIN_LOW_USD_MICROS` and `settlement_estimate_enabled` and `notify.settlement_estimate` and master on.

Rules: `reengage::prepare` builds the candidate only when its existing `closed` is false, the estimate switch and `suggestions_enabled` are on, and health is good (the same health source as `LeadInputs.healthy`); `SeedBaseline` writes `settlement_notified` with no notification; the candidate is `foldable: false`, `resolves: Fact::EstimateAnnounced`, `since: shown.estimated_at`, `ttl: None`. `apply` posts it through `EVENT_REENGAGE_DUE` with kind `settlement_estimate`, records attention, and writes `settlement_notified` from `shown`. `fold_text` returns `None` for the kind. `mark`: with `decisions_owed == 0` and a fresh history poll, kinds are `[VerdictsLanded?, SettlementEstimate?]`, state `News` when either; with `decisions_owed > 0` unchanged. `nudge_decline` with kind `settlement_estimate` is refused as today's unknown-kind path (D19).

- [ ] **Step 1: Write the failing tests**

```rust
// settlement_estimate.rs (pure)
#[test] fn the_first_estimate_seeds_the_baseline_silently() // last None -> SeedBaseline
#[test] fn growth_needs_both_25_percent_and_one_dollar() // last low 4_000_000: low 5_000_000 -> Candidate; 4_999_999 -> Quiet; last low 2_000_000: low 2_500_000 (25%, $0.50) -> Quiet; 3_000_000 -> Candidate
#[test] fn a_pricing_change_moves_the_mid_25_percent_either_way() // last mid 8_000_000 p2; p3 mid 10_000_000 -> Candidate{pricing_changed: true}; 6_000_000 -> Candidate; 9_999_999 -> Quiet; same pricing mid 6_000_000 -> Quiet
// reengage.rs
#[test] fn a_settlement_estimate_posts_once_with_core_words() // seeded baseline, then a grown estimate -> one reengage_due {kind: "settlement_estimate"}, title NOTIFY_SETTLEMENT_TITLE, settlement_notified updated
#[test] fn each_gate_holds_the_settlement_notification() // one sub-case each: paused, consent hold, signed out, unhealthy, suggestions off, estimate switch off, notify.settlement_estimate off, master off, quiet hours, daily cap, min gap -> no settlement frame
#[test] fn not_now_only_dismisses() // nudge_decline {kind: "settlement_estimate"} is refused and the nudge ledger is unchanged; the next grown estimate still posts after its kind interval
// nudge.rs
#[test] fn the_estimate_lights_the_news_mark_when_nothing_is_owed() // owed 0, fresh poll, settlement_lit -> News, kinds == [SettlementEstimate]; with verdict news -> [VerdictsLanded, SettlementEstimate]
#[test] fn the_estimate_mark_follows_every_gate() // paused / consent hold / not enrolled / suggestions off / mark switch off -> None; unhealthy -> Unknown; settlement_lit false -> None; owed > 0 -> halo rules unchanged
// attention.rs: extend the existing every-kind label/precedence tests to five kinds
```

- [ ] **Step 2: Run to verify they fail** — `cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor -- settlement attention nudge::tests reengage` → compile failures.

- [ ] **Step 3: Implement** per the Interfaces and Rules blocks. Every exhaustive `match` on `Kind` gains the arm; IPC `notify` key lists and the settings `notify` object accept `settlement_estimate`.

- [ ] **Step 4: Run** the Step 2 command, the full contributor suite, and clippy. Expected: new tests pass; existing attention, reengage and nudge tests green.

- [ ] **Step 5: Commit** — subject `Notify when the settlement estimate grows, and light the news mark`.

### Task 8: Per-entry dollar estimates and the C ABI

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (`estimate_is_drawn` returns false; `settlement_estimate_value(e, table, now)`; `list_pending` rows)
- Modify: `crates/trace-commons-contributor/src/nudge_render.rs` (`EntryUsdEstimate`, `entry_tags`)
- Modify: `crates/trace-commons-contributor-ffi/src/nudge.rs` (`tc_nudge_entry_tags_json` reads `settlement_estimate`)
- Modify: `crates/trace-commons-contributor-ffi/include/trace_commons.h` and `macos/Sources/CTraceCommons/include/trace_commons.h` (doc comment of `tc_nudge_entry_tags_json`)

**Interfaces:**
- Consumes: task 1 `entry_estimate`, task 4 `entry_usd_band`, `SETTLEMENT_ESTIMATE_EXPLAINER`, task 6 `SettlementEstimateState.table`.
- Produces:
  - Row field `settlement_estimate: {"low_usd_micros", "high_usd_micros", "tier"?, "drawn": true}`, present only when the table is drawable, the estimate switch is on, the entry has features, and the entry is not on its way or sent (the same `sent` rule as `local_credit_estimate_for`).
  - `pub struct EntryUsdEstimate { pub low_usd_micros: u64, pub high_usd_micros: u64, pub tier: Option<String>, pub drawn: bool }`; `pub fn entry_tags(mission_fit: Option<u64>, estimate: Option<&EntryUsdEstimate>) -> EntryTags` (band via `entry_usd_band`, tier tags as today, explainer `SETTLEMENT_ESTIMATE_EXPLAINER`; nothing when `drawn` is false or `high < low`).
  - `tc_nudge_entry_tags_json` input key `settlement_estimate`; `credit_estimate` is ignored.

- [ ] **Step 1: Write the failing tests**

```rust
// ipc.rs
#[tokio::test] async fn list_pending_rows_carry_a_dollar_estimate_under_a_drawable_table() // fixture entry with features f(1000) under table_a: row["settlement_estimate"]["low_usd_micros"] == 2_000_000, high 6_000_000
#[tokio::test] async fn no_row_estimate_without_a_drawable_table_or_once_sent() // no table / no rate / entry Uploaded -> key absent
#[test] fn the_points_band_is_never_drawn() // estimate_is_drawn(published three-tier slot) == false; row credit_estimate["drawn"] == false; batch clause absent from status.nudge.text
// nudge_render.rs (replace a_row_draws_its_estimate_only_when_the_core_says_to)
#[test] fn a_row_draws_its_dollar_estimate_only_when_the_core_says_to() // drawn (2_000_000, 6_000_000, "higher") -> "Estimate: about $2 to $6 of inference", "Higher estimate", explainer; drawn false -> no band
// ffi nudge.rs
#[test] fn entry_tags_read_the_settlement_estimate() // {"settlement_estimate": {...drawn: true}} -> estimate_band present; {"credit_estimate": {...drawn: true}} only -> {}
```

Expected flips, to update rather than revert `estimate_is_drawn`: in `daemon/ipc.rs`, `the_estimate_is_drawn_only_under_a_published_tiered_table`, `status_credit_estimate_sums_only_the_known_candidates`, `status_nudge_credit_estimate_sums_the_backlog_when_it_leads` and any other test asserting `drawn: true` or a batch estimate clause now assert `drawn: false` and no clause (S8; D15). `list_pending_renders_the_estimate_only_for_entries_with_features` keeps its `credit_estimate` presence assertions.

- [ ] **Step 2: Run to verify they fail** — `cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor -- list_pending points_band row_draws` and `cargo test --manifest-path $W/Cargo.toml -p trace-commons-contributor-ffi nudge`.

- [ ] **Step 3: Implement**, then update the header comment for `tc_nudge_entry_tags_json` in both copies (it carries `mission_fit` and `settlement_estimate`; the band is dollars) and run `diff crates/trace-commons-contributor-ffi/include/trace_commons.h macos/Sources/CTraceCommons/include/trace_commons.h` — expected: no output.

- [ ] **Step 4: Run** both Step 2 commands, then the full contributor and ffi suites and clippy on both crates. Expected: new tests pass; no regressions.

- [ ] **Step 5: Commit** — subject `Show queue rows' estimates in inference dollars`.

### Task 9: macOS menu line, icon state, notification and Settings switch

**Files:**
- Modify: `macos/Sources/TCShellCore/DataContract/DaemonDataModels.swift` (`Status.settlementEstimate: SettlementEstimate?`, `QueueEntry.settlementEstimate`)
- Modify: `macos/Sources/TCShellCore/NudgeCopy.swift` (`NudgeEntryTags.input(for:)` sends `settlement_estimate`; Settings `Switch.settlementEstimate` row with `SETTING_SETTLEMENT_ESTIMATE` and help, writing `settlement_estimate_enabled`; `NudgeCopy.Key` cases for every new key)
- Modify: `macos/Sources/TCShellCore/NudgeSurface.swift` (`Kind.settlementEstimate = "settlement_estimate"`; intent `review` opens Traces, `not_now` dismisses only)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift` (`settlementRow` after `nudgeRow`; pure `MenuPanelStatus.settlementLine(_ status: DaemonData.Status?, available: Bool) -> SettlementLine?`)
- Modify: `macos/Sources/TraceCommonsApp/Notifier.swift` (plan accepts the kind; "Not now" posts no IPC)
- Test: `macos/Tests/TraceCommonsAppTests/MenuBarGlassPanelTests.swift`, `NudgeNotificationTests.swift`, `NudgeMenuBarTests.swift`, `NudgeSettingsSectionTests.swift`; `macos/Tests/TCShellCoreTests/NudgeSurfaceTests.swift`, `NudgeWordingTests.swift`; `macos/Tests/TCBridgeTests/NudgeCopyExportTests.swift`

**Interfaces:**
- Consumes: `status.settlement_estimate` (task 6), `mark` / `mark_kinds` / `mark_text` with `settlement_estimate` (task 7), row `settlement_estimate` and `tc_nudge_entry_tags_json` (task 8), copy keys (task 4) via `tc_nudge_copy_json`.
- Produces: `DaemonData.SettlementEstimate { lowUsdMicros: UInt64, highUsdMicros: UInt64, upTo: Bool, known: Int, unknown: Int, tableVersion: String, pricingVersion: String?, estimatedAt: String, text: String }`; `struct SettlementLine { text: String; label: String; action: String }`.

The row uses `MenuBarGlassPanel`'s existing glass components (match `nudgeRow`'s button shape): the core's `text`, the `ESTIMATE_TOTAL_LABEL` caption, and an `ESTIMATE_TOTAL_ACTION` button that opens the Traces view through the existing `open(_:)` path. No figure is added to the strip itself; the strip's mark is the daemon's `mark`, unchanged code. Accessibility label and button label are separate keys.

- [ ] **Step 1: Write the failing tests**

```swift
func testSettlementLineRendersFromAStatusFixture() // status JSON with settlement_estimate.text "About $3.50 to $9 of inference in traces on this Mac" -> SettlementLine.text equal, label "Estimate", action "Review traces"
func testSettlementLineIsAbsentWhenNotDrawable() // no key -> nil; available false -> nil
func testTheNewsMarkLightsForTheEstimate() // nudge mark "news", mark_kinds ["settlement_estimate"] -> .news; paused / unavailable -> .none
func testSettlementNotificationPlansAndNotNowOnlyDismisses() // ReengageDue kind settlement_estimate -> plan non-nil with two actions; intent for not_now sends nothing (the test client records zero calls)
func testSettingsShowTheEstimateSwitch() // rows include the switch with the core's label, bound to settlement_estimate_enabled
func testCopyExportCarriesTheSettlementKeys() // every new key decodes from tc_nudge_copy_json
```

- [ ] **Step 2: Build the dylib and run to verify they fail**

```bash
cargo build --manifest-path $W/Cargo.toml -p trace-commons-contributor-ffi
swift test --package-path $W/macos --filter 'Settlement|NudgeNotification|NudgeMenuBar|NudgeSettings|NudgeCopyExport'
```

Expected: compile failures for the new symbols.

- [ ] **Step 3: Implement** per the Files and Interfaces blocks.

- [ ] **Step 4: Run** the Step 2 filter, then the whole `swift test --package-path $W/macos`. Expected: new tests pass; failures equal the baseline from task 1 step 1.

- [ ] **Step 5: Commit** — subject `Show the settlement estimate in the macOS menu bar, notifications and Settings`. The UI goes to design review after this lands; say so in the task report.
