# Insights pricing data (PR 0) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship reviewed API list-price tables with provenance, split the cache-write rate into 5-minute and 1-hour categories, and add one pricing function that prices a session's stored turns, for both the Models answers (PR 1) and the Sessions-tab cost PR.

**Architecture:** The protocol crate's `insights_pricing` keeps the checked integer arithmetic; its Claude accounting gains two cache-write categories in place of one. The tables are JSON files embedded in the insights crate and loaded by `production_pricing_catalog()`, each validated against a `table_id` pinned in the file. A new `insights::session_pricing` module normalizes model labels, prices each turn through `select_newest_applicable`, sums exact numerators and rounds once. Nothing here is stored or shown; no surface reads it until PR 1.

**Tech Stack:** Rust; crates `trace-commons-protocol` (MIT OR Apache-2.0) and `trace-commons-contributor-insights` (MIT OR Apache-2.0); `serde_json`, `chrono`, `sha2` (already dependencies).

**Spec:** `docs/superpowers/specs/2026-10-20-insights-model-answers-design.md` (section 3 and Delivery item 0). Review context: PR #1501, poldsam's review point 1 and the reply.

## Global Constraints

- Verification, run before every commit on the crates the task touches:
  - `cargo fmt --all -- --check`
  - `RUSTFLAGS="-D warnings" cargo clippy -p <crate> --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
  - `RUSTFLAGS="-D warnings" cargo test -p <crate>`
  - `RUSTFLAGS="-D warnings" cargo check -p trace-commons-protocol --no-default-features` and `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor-insights --no-default-features` (the "permissive crates, standalone" CI job).
  - Known environmental failures to ignore, and only these: `an_unentitled_process_reports_one`, `daemon::ipc::tests::managed_sign_in_redeem_hosts_the_proxy_and_ends_it`, `daemon::managed::tests::opencode_starts_on_the_sign_in_without_a_model`. Capture a baseline failure list before Task 1 and compare against it.
- **Never a price from memory.** Every rate in a shipped table is read from the vendor page fetched in Task 3, on the day of the fetch. No rate appears in this plan, in a test written before Task 3, or in a commit message. Synthetic tests use synthetic providers and models (`synthetic-provider`, `synthetic-model`) as `pricing_catalog.rs` already does.
- Each shipped table records `provenance.source_url` (the page the rates were read from) and `provenance.retrieved_at` (the fetch time, UTC).
- Unknown is never zero: a turn that cannot be priced is unpriced, and a session with any unpriced counted turn has no total. Never $0 in place of unknown.
- `CacheWrite::TotalOnly` is never list-priced under Claude accounting (spec section 3).
- Rows store tokens only; nothing in this PR stores a price or puts one on a wire. `LEDGER_MONEY_IN_INSIGHTS` stays `false` here; PR 1 flips it.
- Labels only in logs and errors: fixed `insights_pricing_*` labels, never a model label, URL or count.
- No new dependencies. Both crates stay MIT OR Apache-2.0; nothing here may depend on an AGPL crate (`crates/trace-commons-server/tests/license_boundary.rs`).
- No user-facing copy in this PR. (The "at API prices" words are PR 2's, through `analytics_copy.rs` and `docs/ui-copy-guide.md`.)
- Tests count operations, never wall-clock time.
- Commits: short imperative subject, no `feat:`/`fix:` prefix, no emojis, body ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- A turn dated before every entry's `effective_from` (or undated) must be unpriced (`WindowUnresolved` / `Undated`), never priced at the newest table's rate.
- A Claude turn whose cache write is `TotalOnly` (the proxy-ledger shape) must be unpriced (`CacheLifetimeUnknown`), never priced at the 5-minute or 1-hour rate.
- A Codex turn with a nonzero cache write (6,521 of the owner's last 13 weeks of `token_count` events) must be priced with the write inside `UncachedInput`, never added on top: in every one of those events `total_tokens == input_tokens + output_tokens` and `cache_write_input_tokens <= input_tokens - cached_input_tokens`. If the fetched OpenAI page states a separate cache-write charge, stop and ask the owner.
- A Claude session holding `<synthetic>` turns (stored with `model_label_ix: None`, because `safe_model` rejects `<` and `>`; all 26 in the owner's corpus have zero usage) must still have a total.
- One unpriced turn among many priced turns makes the session total `None` while the other models' figures stay; the total is never the sum of the priced part.
- Token counts near `u32::MAX` on every turn of an 8,192-turn session must sum without wrapping; an overflow is `Unpriced(Overflow)`, never a panic or a small number.

Each line has its test in Task 2. (Six lines: the `<synthetic>` case was found while writing PR 1's plan and is cheap to pin.)

---

## File Structure

- Modify `crates/trace-commons-protocol/src/insights_pricing.rs`: the category split, `MAX_RATES_PER_ENTRY`, `PRICING_TABLE_SCHEMA_VERSION`.
- Modify `crates/trace-commons-contributor-insights/src/insights/usage.rs` (`accounting_components`, lines 89-131; tests at 411 and 488).
- Create `crates/trace-commons-contributor-insights/src/insights/session_pricing.rs`: label normalization, turn and session pricing.
- Modify `crates/trace-commons-contributor-insights/src/insights.rs:29` area: `pub mod session_pricing;`.
- Create `crates/trace-commons-contributor-insights/src/insights/pricing_tables/anthropic.json` and `.../pricing_tables/openai.json`: one table per source page.
- Modify `crates/trace-commons-contributor-insights/src/insights/pricing_catalog.rs`: `production_pricing_catalog()` loads the two tables; the module doc (line 3) and the test at lines 265-274 change.

---

### Task 1: Split the cache-write category

**Files:**
- Modify: `crates/trace-commons-protocol/src/insights_pricing.rs:13,18,39-45,150-159` and its tests (`rates`, `table`, `input` helpers at 402-465)
- Modify: `crates/trace-commons-contributor-insights/src/insights/usage.rs:89-131,411,488`
- Modify: `crates/trace-commons-contributor-insights/src/insights/pricing_catalog.rs` test helpers if they name the old variant (they use Codex accounting today; check)

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `BillableTokenCategory::{UncachedInput, CachedInput, CacheReadInput, CacheWriteFiveMinuteInput, CacheWriteOneHourInput, Output}`; `CacheCreationInput` is removed. Serde spellings `cache_write_five_minute_input`, `cache_write_one_hour_input`.
  - `PricingAccounting::ClaudeCode.required_categories() == &[UncachedInput, CacheReadInput, CacheWriteFiveMinuteInput, CacheWriteOneHourInput, Output]`; Codex unchanged: `&[UncachedInput, CachedInput, Output]`.
  - `PRICING_TABLE_SCHEMA_VERSION: u32 = 2` (decision: bump, so a v1 table with the single category can never validate; no saved estimate or table exists anywhere on `origin/main`, so nothing migrates).
  - `MAX_RATES_PER_ENTRY = 5`.
  - `NativeTokenCounts::accounting_components()` for `ClaudeCode` returns `[(UncachedInput, input), (CacheReadInput, cache_read_input), (CacheWriteFiveMinuteInput, cache_creation_5m), (CacheWriteOneHourInput, cache_creation_1h), (Output, output)]`.

- [ ] **Step 1: Write the failing tests**

In `insights_pricing.rs` tests:

```rust
#[test]
fn claude_accounting_prices_each_cache_write_lifetime_at_its_own_rate() {
    // Synthetic rates: 5-minute write 7, 1-hour write 11 nanos per million
    // tokens. 1_000_000_000 tokens of each: 7 + 11 = 18 micros.
    let estimate = calculate_estimated_cost(&claude_table(), &claude_input([0, 0, 1_000_000_000, 1_000_000_000, 0])).unwrap();
    assert_eq!(estimate.estimated_cost_usd_micros, 18);
    assert_eq!(estimate.breakdown.iter().map(|b| b.category).collect::<Vec<_>>(),
               PricingAccounting::ClaudeCode.required_categories());
}

#[test]
fn a_schema_v1_table_never_validates() {
    let mut old = table(vec![entry(0, None, 1)]);
    old.schema_version = 1;
    old.table_id = old.canonical_table_id().unwrap();
    assert_eq!(old.validate(), Err(PricingError::InvalidTable));
}

#[test]
fn split_category_serde_spellings_are_pinned() {
    assert_eq!(serde_json::to_value(BillableTokenCategory::CacheWriteFiveMinuteInput).unwrap(), "cache_write_five_minute_input");
    assert_eq!(serde_json::to_value(BillableTokenCategory::CacheWriteOneHourInput).unwrap(), "cache_write_one_hour_input");
}
```

Add `claude_table()` / `claude_input([u64; 5])` helpers beside the existing Codex ones. In `usage.rs` update the tests at 411 (category list) and 488 (the `(CacheCreationInput, 50)` pair) to the split, with a 5m/1h pair that is not symmetric (for example 30 and 20) so a swap fails.

- [ ] **Step 2: Run to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol insights_pricing`
Expected: compile errors naming `CacheWriteFiveMinuteInput`.

- [ ] **Step 3: Implement the split** in `insights_pricing.rs` and `usage.rs` per the Interfaces block. `accounting_components` reads `cache_creation_5m` and `cache_creation_1h` (already validated to sum to `cache_creation_input` by `validate_accounting`).

- [ ] **Step 4: Run the protocol and insights suites**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol && RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights`
Expected: PASS, including `accounting_components_are_the_priceable_categories` and `serialized_contract_contains_no_u128_or_float_values`. If an envelope or wire digest pin in the protocol crate moves, stop: this change must not touch an envelope type.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-protocol/src/insights_pricing.rs crates/trace-commons-contributor-insights/src/insights/usage.rs
git commit -m "Price 5-minute and 1-hour cache writes at their own rates" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Session pricing module

**Files:**
- Create: `crates/trace-commons-contributor-insights/src/insights/session_pricing.rs` (tests in the same file)
- Modify: `crates/trace-commons-contributor-insights/src/insights.rs` (add `pub mod session_pricing;` beside `pricing_catalog`)

**Interfaces:**
- Consumes: Task 1's categories; `PricingCatalog::select_newest_applicable`, `calculate_estimated_cost` (existing); `TurnRecord`, `CacheWrite` from `trace_commons_protocol::insights_usage_series`.
- Produces (PR 1 and the Sessions-tab PR use exactly these):
  - `pub const PRICING_BASIS_API_LIST: &str = "api_list";`
  - `pub fn normalize_model_label(label: &str) -> Option<String>`: `None` for `<synthetic>` (defence only: the scans never store it, see rule 1); otherwise ASCII-lowercased, with one trailing `-YYYYMMDD` (8 digits) or `-latest` removed, and `None` if the result is empty or fails the pricing id charset (`[A-Za-z0-9-_.:]`, at most 96 bytes).
  - `pub fn pricing_provider(accounting: PricingAccounting) -> &'static str`: `ClaudeCode -> "anthropic"`, `Codex -> "openai"`. The provider is the accounting family's publisher, never inferred from the label.
  - `pub struct PricingTableRef { pub table_id: String, pub version: u32 }` (`Ord`, `Serialize`).
  - `pub enum UnpricedReason { UsageUnknown, Undated, NoLabel, CacheLifetimeUnknown, MissingPrice, WindowUnresolved, Overflow, InvalidCatalog }` (`Copy`, `Ord`, serde snake_case).
  - `pub struct ModelPrice { pub priced_turns: u32, pub unpriced_turns: u32, pub usd_micros: Option<u64>, pub unpriced_reasons: BTreeMap<UnpricedReason, u32> }`: `usd_micros` is `Some` only when `unpriced_turns == 0` and `priced_turns > 0`.
  - `pub struct SessionPrice { pub by_model: BTreeMap<String, ModelPrice>, pub skipped_turns: u32, pub unlabelled_turns: u32, pub total_usd_micros: Option<u64>, pub tables: Vec<PricingTableRef> }`: keyed by normalized label; `total_usd_micros` is `Some` only when every counted turn is priced and at least one is; `tables` sorted, distinct.
  - `pub fn price_session(catalog: &PricingCatalog, accounting: PricingAccounting, model_labels: &[String], turns: &[TurnRecord]) -> SessionPrice`.
  - Codex turn mapping, which PR 1's Codex reader must store to match: `uncached_input = input - cached_input` (which already includes any cache write), `cache_read = cached_input`, `output = output` (reasoning is a subset of output), `cache_write = Some(CacheWrite::TotalOnly(cache_write_input_tokens))` (recorded for token figures; never priced on top).
  - Skip rule PR 1's attribution reuses: a turn with `model_label_ix: None` whose `known_usage()` is `Some` with `total() == 0` is skipped (no model, not unpriced); see rule 1.

Pricing rules `price_session` implements, per turn:
1. `model_label_ix` is `None`: if its usage is known and totals zero (the `<synthetic>` shape), the turn is skipped (`skipped_turns`), neither counted nor unpriced; otherwise `NoLabel` (counted in `unlabelled_turns`, and it makes the total `None`).
2. The label normalizes to `None` -> skipped if it was `<synthetic>`, else `NoLabel`.
3. `known_usage()` is `None` -> `UsageUnknown`. `at` is `None` -> `Undated`.
4. Claude accounting: `Split { m5, h1 }` maps to the two write categories; `TotalOnly` -> `CacheLifetimeUnknown`. Codex accounting: `uncached_input` -> `UncachedInput`, `cache_read` -> `CachedInput`, `output` -> `Output`; the cache-write field is not a category (it is inside `uncached_input`).
5. `select_newest_applicable` with `observed_from == observed_until == at`; `MissingPrice` / `UnresolvedWindow` / `InvalidQuery` / `InvalidCatalog` map to `MissingPrice` / `WindowUnresolved` / `InvalidCatalog` / `InvalidCatalog`; `calculate_estimated_cost` overflow -> `Overflow`.
6. A priced turn's exact numerator is `sum(breakdown.tokens * breakdown.usd_nanos_per_million_tokens)` in `u128` (checked). A model's figure and the session total are each the checked sum of exact numerators, rounded half-up to micros once (`(n + 500_000_000) / 1_000_000_000`), so the total is never the sum of rounded model figures.

- [ ] **Step 1: Write the failing tests** (synthetic catalog built as in `pricing_catalog.rs` tests, `synthetic-provider` replaced by `anthropic`/`openai` only where `pricing_provider` requires it, with synthetic model ids such as `test-model-a`)

```rust
#[test] fn labels_normalize_by_case_date_stamp_and_latest() {
    assert_eq!(normalize_model_label("Test-Model-A-20261001").as_deref(), Some("test-model-a"));
    assert_eq!(normalize_model_label("test-model-a-latest").as_deref(), Some("test-model-a"));
    assert_eq!(normalize_model_label("test-model-a-2026100").as_deref(), Some("test-model-a-2026100")); // 7 digits kept
    assert_eq!(normalize_model_label("<synthetic>"), None);
    assert_eq!(normalize_model_label("vendor/model"), None);
    assert_eq!(normalize_model_label("-latest"), None);
}
#[test] fn a_claude_split_write_prices_each_part_at_its_rate() { /* m5 and h1 nonzero; expected micros = hand-computed from synthetic rates */ }
#[test] fn a_total_only_write_is_never_list_priced() {
    assert_eq!(price.by_model["test-model-a"].unpriced_reasons[&UnpricedReason::CacheLifetimeUnknown], 1);
    assert_eq!(price.total_usd_micros, None);
}
#[test] fn a_codex_cache_write_is_priced_inside_uncached_input_not_on_top() { /* TotalOnly(5) and TotalOnly(0) with the same other counters price identically */ }
#[test] fn one_unpriced_turn_leaves_the_total_unknown_and_other_models_priced() {
    assert!(price.by_model["test-model-a"].usd_micros.is_some());
    assert_eq!(price.by_model["test-model-b"].usd_micros, None);
    assert_eq!(price.total_usd_micros, None);
}
#[test] fn an_undated_turn_or_one_before_every_entry_is_unpriced() { /* Undated and WindowUnresolved, never the newest rate */ }
#[test] fn an_unlabelled_zero_usage_turn_is_skipped_not_unpriced() { assert_eq!(price.skipped_turns, 1); assert!(price.total_usd_micros.is_some()); }
#[test] fn an_unlabelled_turn_with_usage_counts_and_blocks_the_total() { assert_eq!(price.unlabelled_turns, 1); assert_eq!(price.total_usd_micros, None); }
#[test] fn an_unlabelled_turn_with_unknown_usage_blocks_the_total() { assert_eq!(price.unlabelled_turns, 1); assert_eq!(price.total_usd_micros, None); }
#[test] fn the_total_rounds_once_over_exact_numerators() {
    // Two models, each 0.4 micro exactly: each model rounds to 0, total 0.8 rounds to 1.
    assert_eq!(price.by_model["test-model-a"].usd_micros, Some(0));
    assert_eq!(price.total_usd_micros, Some(1));
}
#[test] fn maximal_counts_over_the_turn_cap_never_wrap() {
    // MAX_SERIES_TURNS turns, every counter u32::MAX, the largest synthetic rate: Some(exact) or Overflow, never a wrapped value.
}
#[test] fn an_empty_catalog_prices_nothing_and_says_why() { assert_eq!(..unpriced_reasons[&UnpricedReason::MissingPrice], n); }
#[test] fn tables_used_are_listed_once_in_order() { assert_eq!(price.tables.len(), 2); }
```

- [ ] **Step 2: Run to verify they fail**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights session_pricing`
Expected: FAIL to compile (module missing).

- [ ] **Step 3: Implement** the Interfaces block in `session_pricing.rs`. Module doc: local only, no I/O, no persistence; "at API prices" is a list-price estimate, not what a subscription costs; a turn is priced or unpriced, never zero.

- [ ] **Step 4: Run to verify they pass**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights session_pricing`
Expected: PASS (13 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor-insights/src/insights/session_pricing.rs crates/trace-commons-contributor-insights/src/insights.rs
git commit -m "Price a session's turns at list prices, unknown never zero" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Fetch the rates and ship the tables

**Files:**
- Create: `crates/trace-commons-contributor-insights/src/insights/pricing_tables/anthropic.json`, `.../pricing_tables/openai.json` (each one `PricingTable`, serde form)
- Modify: `crates/trace-commons-contributor-insights/src/insights/pricing_catalog.rs:1-4` (module doc), `:158-160` (`production_pricing_catalog`), `:265-274` (production test)
- Test: `pricing_catalog.rs` tests

**Interfaces:**
- Consumes: Task 1 (schema 2, split categories), Task 2 (`normalize_model_label`, `pricing_provider`, `price_session`).
- Produces:
  - `production_pricing_catalog() -> PricingCatalog` holding exactly two tables: version 1 (Anthropic) and version 2 (OpenAI). It parses the embedded JSON (`include_str!`) and validates through `PricingCatalog::from_tables`; a table that fails to parse or validate is a test failure, and at runtime `production_pricing_catalog()` returns an empty catalog with a fixed `tracing::warn!("insights pricing tables invalid")` rather than panicking (everything is then `MissingPrice`, never $0).
  - `pub const SHIPPED_PRICING_TABLES: &[(u32, &str)]`: each version with its pinned `table_id`, in `pricing_catalog.rs`.
  - Nothing else. A label absent from the catalog is `MissingPrice` by construction; no list of unpriced labels is kept in source.

**Disclosure rule for this task.** The repository and its PRs are public. The labels found in Step 1 may include pre-release or early-access model names. Only a model name printed on the fetched vendor page may appear in source, a test, a commit message or the PR body. The Step 1 and Step 5 outputs go to the owner privately (the implementer's hand-off report), never into the repository or onto GitHub.

- [ ] **Step 1: Record the labels the owner's machine has actually used**

Run these and keep the output (privately) for Step 5:

```bash
jq -r '[.rows[] | select(.body.kind=="claude") | .body.model_labels[]] | group_by(.) | map("\(.[0]) \(length)")[]' \
  "$HOME/Library/Application Support/trace-commons/insights-counter-rows.json"
find ~/.claude/projects -name '*.jsonl' -newermt "$(date -v-13w +%F)" -print0 \
  | xargs -0 grep -h '"type":"assistant"' | jq -r '.message.model // empty' | sort | uniq -c | sort -rn
find ~/.codex/sessions -name '*.jsonl' -newermt "$(date -v-13w +%F)" -print0 \
  | xargs -0 grep -h '"type":"turn_context"' | jq -r '.payload.model // empty' | sort | uniq -c | sort -rn
```

Expected classes: Claude `claude-*` labels and `<synthetic>`; Codex `gpt-*` labels, some with a `-latest` suffix; `codex-auto-review` (Codex's reviewer, left out of every answer by PR 1, so its price does not matter); and labels containing `/`, which `safe_model` rejects, so they are never stored as labels. Codex rows in the counter store carry no labels until PR 1, so Codex labels come from the transcripts.

- [ ] **Step 2: Fetch each vendor page, keep its bytes, and pick the page whose bytes carry the rates**

For each candidate URL — Anthropic `https://www.anthropic.com/pricing` and `https://docs.anthropic.com/en/docs/about-claude/pricing`; OpenAI `https://openai.com/api/pricing/` and `https://platform.openai.com/docs/pricing`:

```bash
d=$(mktemp -d); curl -sSL -A 'Mozilla/5.0' -o "$d/page.html" -w '%{http_code} %{url_effective}\n' "<url>"
shasum -a 256 "$d/page.html"; date -u +%Y-%m-%dT%H:%M:%SZ
grep -c '<one label from Step 1>' "$d/page.html"
```

Also read each page with WebFetch to transcribe the table. The table's `source_url` is the one URL whose fetched bytes contain the rates, after redirects (curl's `url_effective`; if that is on another vendor host, add that origin to the test's list in Step 4); `content_sha256` is `sha256:` plus the hex digest of those bytes; `retrieved_at` is the `date -u` time of that fetch. If no URL's raw bytes contain the rates (a client-rendered page), use the URL WebFetch read, hash the text WebFetch returned (saved to a file in `$d`), and say so in the `pricing_catalog.rs` module doc and the PR body: provenance has no notes field (`deny_unknown_fields`). Never edit a rate to match memory.

- [ ] **Step 3: Transcribe the rates**

One `ModelPriceEntry` per model the page prices and Step 1 observed (normalized label as `model`, `pricing_provider` as `provider`):
- Unit: `usd_nanos_per_million_tokens` = dollars per million tokens x 1,000,000,000. A page figure of `$A.BC / MTok` becomes `A_BC0_000_000` nanos; check every entry by converting back.
- Anthropic (`ClaudeCode` accounting): base input -> `UncachedInput`; cache hits/reads -> `CacheReadInput`; 5-minute cache writes -> `CacheWriteFiveMinuteInput`; 1-hour cache writes -> `CacheWriteOneHourInput`; output -> `Output`. If the page gives no 1-hour (or no 5-minute) rate for a model, that model gets no entry (it is `MissingPrice`); never derive one rate from another.
- OpenAI (`Codex` accounting): input -> `UncachedInput`; cached input -> `CachedInput`; output -> `Output`. Standard tier only (not batch, flex or priority). Codex cache writes are inside `input_tokens` (Review Focus line 3), so a page with no separate cache-write charge needs nothing more; record that in the `session_pricing.rs` module doc. If the page states a separate cache-write charge, stop and ask the owner.
- If the page prices a model by request size (a long-context tier), stop and ask the owner: the table has no threshold field, and turns above one must not be priced at the lower rate.
- `effective_from`: the date the page states for that price. If the page states none, **stop and ask the owner** whether to set it to the start of the 13-week window before `retrieved_at` or to `retrieved_at` itself (older turns then read "Not priced"). Do not choose. `effective_until: None`.
- `table_id`: compute with `canonical_table_id()` (a one-off `#[test] #[ignore] fn print_table_ids()` that prints them is fine; delete it before committing) and pin it in the JSON and in `SHIPPED_PRICING_TABLES`.
- Provenance: `publisher` `"Anthropic"` / `"OpenAI"`; `reviewed_by: "Pending owner review"`, `reviewed_at` = `retrieved_at`, `published_at` = `retrieved_at`. Task 4 replaces all three after the owner's review.

- [ ] **Step 4: Write the failing tests** in `pricing_catalog.rs` (replacing `production_catalog_is_validated_empty_and_contains_no_rates`) and `session_pricing.rs`

```rust
#[test] fn the_production_catalog_carries_the_two_shipped_tables() {
    let catalog = production_pricing_catalog();
    assert_eq!(catalog.tables().iter().map(|t| (t.version, t.table_id.as_str())).collect::<Vec<_>>(),
               SHIPPED_PRICING_TABLES.to_vec());
}
#[test] fn every_shipped_table_names_its_source_page_and_fetch_date() {
    for t in production_pricing_catalog().tables() {
        // The insights crate has no `url` dependency; match the four fetched origins as prefixes.
        assert!(["https://www.anthropic.com/", "https://docs.anthropic.com/", "https://openai.com/", "https://platform.openai.com/"]
            .iter().any(|origin| t.provenance.source_url.starts_with(origin)));
        assert!(t.provenance.retrieved_at <= t.provenance.reviewed_at);
    }
}
#[test] fn every_shipped_table_is_owner_reviewed() {
    // Fails until Task 4: the merge gate for the rates.
    for t in production_pricing_catalog().tables() { assert_ne!(t.provenance.reviewed_by, "Pending owner review"); }
}
#[test] fn every_entry_is_a_normalized_label_under_its_accountings_provider() {
    for e in production_pricing_catalog().tables().iter().flat_map(|t| &t.entries) {
        assert_eq!(normalize_model_label(&e.model).as_deref(), Some(e.model.as_str()));
        assert_eq!(e.provider, pricing_provider(e.accounting));
    }
}
#[test] fn a_shipped_rate_prices_a_known_turn_end_to_end() {
    // One Claude and one Codex turn, each dated at its entry's effective_from
    // (whatever the owner chose in Step 3), through price_session:
    // total Some(_), tables == both refs.
}
```

- [ ] **Step 5: Run the gate test, and check label coverage privately**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights pricing`
Expected: only `every_shipped_table_is_owner_reviewed` fails.

Then, for every distinct label from Step 1, run `normalize_model_label` (a scratch `#[test] #[ignore]` reading a local file outside the repository, deleted before committing, is fine) and classify it as exactly one of: priced (an entry exists), on the page but not priced (say why: no cache-write rate, request-size tier), not on the page, reviewer (`codex-auto-review`), or not a label (`<synthetic>`, rejected by `safe_model`). Every label must land in exactly one class. Send the table, with each label's turn count, to the owner in the hand-off report. If most of the owner's Codex or Claude turns fall under "not on the page", say so plainly: question 4 will then read "Not priced" for those models on his machine.

- [ ] **Step 6: Update the module doc** of `pricing_catalog.rs`: the catalog ships two reviewed tables, one per source page, with no runtime fetch; how the hash was taken (raw bytes, or the rendered text, per Step 2).

- [ ] **Step 7: Commit** (the gate test stays red; PR 0 is opened as a draft)

```bash
git add crates/trace-commons-contributor-insights/src/insights/pricing_tables crates/trace-commons-contributor-insights/src/insights/pricing_catalog.rs crates/trace-commons-contributor-insights/src/insights/session_pricing.rs
git diff --cached | grep -c '<each Step 1 label not printed on a vendor page>'   # must print 0 for every such label
git commit -m "Ship Anthropic and OpenAI list-price tables pending owner review" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

The PR body lists, per table: source URL, fetch time, hash method, and every priced model with its rates in $/MTok as read from the page. It names no label that is not on a vendor page.

---

### Task 4: Owner review of the rates

**Files:**
- Modify: both `pricing_tables/*.json` (provenance), `pricing_catalog.rs` (`SHIPPED_PRICING_TABLES`)

**Interfaces:**
- Consumes: Task 3's tables.
- Produces: final `SHIPPED_PRICING_TABLES` ids (PR 1 does not pin them; it reads the catalog).

- [ ] **Step 1: Ask the owner to review the rates in the PR body against the pages.** Wait for an explicit approval. A change he asks for goes back through Task 3 Steps 2-3 (refetch, new hash, new `retrieved_at`).
- [ ] **Step 2: Set provenance**: `reviewed_by` = the owner's name as he gives it, `reviewed_at` = the approval time (UTC), `published_at` = `reviewed_at`. Recompute and pin both `table_id`s.
- [ ] **Step 3: Run the full suites**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol && RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights && RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`, then fmt, clippy and the two `--no-default-features` checks.
Expected: all green against the Task 1 baseline, `every_shipped_table_is_owner_reviewed` included.
- [ ] **Step 4: Commit**

```bash
git add crates/trace-commons-contributor-insights/src/insights/pricing_tables crates/trace-commons-contributor-insights/src/insights/pricing_catalog.rs
git commit -m "Record the owner's review of the list-price tables" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

## Self-review notes

- Spec section 3 coverage: data with provenance (Task 3), no runtime fetch (Task 3, embedded), cache-write split and `TotalOnly` never priced (Tasks 1-2), price at answer time per turn with `select_newest_applicable` (Task 2), unpriced turn makes its session unpriced (Task 2), production-catalog test pins the shipped tables (Task 3). "At API prices" copy, the ledger charge, "Not priced" row state and the `LEDGER_MONEY_IN_INSIGHTS` flip are PR 1/PR 2/Sessions-PR work by the spec's Delivery list.
- Spec says a `[1m]` context suffix is kept on labels. `safe_model` (`usage_evidence.rs:148`) and the pricing `safe_id` both reject `[` and `]`, and no Claude `message.model` in the owner's last 13 weeks carries one, so no code handles it here; a bracketed label is not a label.
