# Terms of Service Read-Through Acceptance Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Load the Terms of Service from tracecommons.ai, make people scroll them before accepting (first run, and again when a new version is in effect), hold uploads until they accept, and record each acceptance on the server per org.

**Architecture:** The community site emits `/legal/terms.json` from the same source as the terms page. The contributor crate fetches it (C ABI for first run, daemon tick for re-checks), keeps `terms-state.json` in the state dir, holds uploads while an in-effect version is unaccepted, and posts acceptances to a new ingest route `POST /v1/account/terms-acceptance`, which writes an append-only RLS table. The macOS app renders the document natively in Welcome and in a sheet opened from a Home notice or a menu bar row.

**Tech Stack:** Astro 5 (site), Rust (protocol, server, ingest/axum, contributor daemon, FFI), PostgreSQL, SwiftUI (macOS 14+).

**Spec:** `docs/superpowers/specs/2026-11-09-terms-scroll-acceptance-design.md`

Two repositories, two PRs: Part A in `TraceCommons/trace-commons-community`, Part B here. Part B does not need Part A merged to build or test (its tests use local fixtures), but the feature is inert until Part A is deployed.

## Global Constraints

- The current Terms of Service and Privacy Policy are **not valid**. They are published with `in_effect: false`. A version with `in_effect: false` never starts an upload hold; first run still requires scrolling and accepting, and shows the not-in-effect line.
- The server never refuses an upload over terms.
- Production and staging both fetch `https://tracecommons.ai/legal/terms.json`.
- Fetch: HTTPS only, host `tracecommons.ai` only, no redirects (`reqwest::redirect::Policy::none()`), body cap 262144 bytes (256 KiB), connect timeout 5 s, total timeout 15 s, `no_proxy()`, no cookies.
- Version id: 1-64 chars of `[A-Za-z0-9._-]`. Hash: 64 lowercase hex chars, SHA-256 of the exact response bytes.
- `accepted_at` more than 5 minutes in the future is refused.
- Re-check interval: 6 hours, plus once at daemon start.
- New AGPL `.rs` files (server, ingest) carry the two-line copyright + SPDX header used by their neighbours. Permissive crates never depend on server/ingest.
- Logs and audit lines carry labels and the version id only. No URLs, tokens, tenant ids or account ids in log strings.
- All new user-facing strings are marked `DRAFT, NEEDS APPROVAL` in a comment beside them. No notices beyond the Home notice and the menu bar row.
- No emojis. Commit subjects: short imperative, no `feat:` prefixes. End commit messages with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Testing follows the repo's "Executing a multi-task plan" rule: write every test listed, run at most `cargo check -p <crate> --all-targets` per task, run everything once in Task B12. Builds go through `~/.claude/turn-locks/turn.sh build "<label>" -- <cmd>` with `CARGO_BUILD_JOBS=2`; whole-package test runs and PostgreSQL suites go through `turn.sh test`.

## Review Focus

1. **Terms change while the sheet is open.** The person accepts version A after the daemon has seen version B published: the daemon refuses with `terms-version-stale` and the sheet reloads; the hold stays. (Task B6 test `accept_refuses_a_version_that_is_no_longer_published`.)
2. **Restart while offline during a hold.** The hold must survive a daemon restart with no network, because the last published version is persisted, not refetched. (Task B7 test `hold_survives_restart_without_network`.)
3. **Org switch.** `cfg.tenant_id` changes after a sign-in to another org: the record is posted again for the new tenant. (Task B6 test `posts_again_when_the_tenant_changes`.)
4. **A document that fits without scrolling, or very large text sizes.** The checkbox unlocks at once when content height <= viewport height + 8pt. (Task B10 test `testShortDocumentUnlocksImmediately`.)
5. **Client clock ahead of the server.** `accepted_at` 6 minutes in the future gets 400 `terms-accepted-at-future`; 4 minutes passes. (Task B2 test `accepted_at_bounds`, Task B4 test.)

---

## Part A: community site (`TraceCommons/trace-commons-community`)

Work in a worktree under `/Users/zakimanian/code/tc-community-worktrees/terms-json` from `origin/main`.

### Task A1: Single-source terms, `terms.json`, and the build check

**Files:**
- Create: `src/legal/terms.ts` (the one source)
- Create: `src/pages/legal/terms.json.ts` (static endpoint)
- Create: `scripts/terms.mjs` (lock check, run from an `astro:build:done` integration like `scripts/aasa.mjs`)
- Create: `scripts/terms.test.mjs`
- Create: `terms.lock`
- Modify: `src/pages/about/tnc.astro` (render from `src/legal/terms.ts`)
- Modify: `src/pages/about/privacy.astro` (not-in-effect line)
- Modify: `astro.config.mjs` (register the terms integration)
- Modify: `package.json` (`test` runs both test files)
- Modify: `public/_headers` (a `/legal/*` rule: `Cache-Control: public, max-age=900, stale-while-revalidate=300`; must not overlap existing rules)

**Interfaces:**
- Produces: `GET /legal/terms.json` with body
  `{"schema_version":"trace_commons.terms.v1","version":"2026-08-22-draft","effective":"2026-08-22","in_effect":false,"title":"Terms and conditions","blocks":[...]}`.
  Block kinds: `{"kind":"heading","text"}`, `{"kind":"paragraph","text"}`, `{"kind":"list","items":[...]}`, `{"kind":"table","header":[...],"rows":[[...]]}`, `{"kind":"callout","text"}`. Inline text is Markdown limited to `*em*`, `**strong**`, `` `code` `` and `[label](url)` with `https://` or `mailto:` targets only. Part B parses exactly this.

- [ ] **Step 1: Write the failing tests** in `scripts/terms.test.mjs` (node:test):
  - `emits every word of the page`: the visible text of the built `dist/about/tnc/index.html` main content (excluding `nav.toc`) equals the concatenated block text of `dist/legal/terms.json`, whitespace-collapsed and Markdown markers stripped.
  - `relative links become absolute`: no link target in the JSON starts with `/`; `/about/privacy` becomes `https://tracecommons.ai/about/privacy`; `mailto:` is kept.
  - `lock catches an unbumped change`: `checkLock({version:"v1", sha:"aa"}, {version:"v1", sha:"bb"})` throws `terms changed without a version bump`; same version and sha passes; a new version with a new sha passes.
  - `table survives`: the JSON has one `table` block with header `["Scope","What it permits"]` and 5 rows, first cell `` `debugging_evaluation` ``.
- [ ] **Step 2:** Run `npm test`. Expected: FAIL (module missing).
- [ ] **Step 3:** Move the content of `tnc.astro` into `src/legal/terms.ts` as `export const terms = { version: "2026-08-22-draft", effective: "2026-08-22", inEffect: false, title: "Terms and conditions", blocks: [...] }`, keeping every sentence verbatim. `tnc.astro` renders the blocks with the same markup and styles as today (headings keep their ids for the contents list, which stays page-only). Add a `p.muted` line under the title on both `tnc.astro` and `privacy.astro` when `inEffect` is false: "These terms are a draft and are not yet in effect." (privacy: "This policy is a draft and is not yet in effect.") -- DRAFT, NEEDS APPROVAL. `terms.json.ts` exports `GET` returning the JSON with relative links made absolute against `https://tracecommons.ai`. `scripts/terms.mjs` exports `checkLock(lock, built)` and the integration, which reads `terms.lock` (`{"version","sha256"}` of the emitted JSON bytes) after build and fails the build on an unbumped change; `node scripts/terms.mjs update-lock dist` rewrites it.
- [ ] **Step 4:** Run `npm run check && TC_USE_DUMMY=1 npm run build && npm test`. Expected: PASS; `dist/legal/terms.json` exists.
- [ ] **Step 5:** Visually diff the built terms page against the live page (same sections, table, banner, contents list). Commit: `Publish the terms as JSON from one source`.

---

## Part B: this repository

Branch `terms-acceptance-design` in `/Users/zakimanian/code/tc-wt-ftux-walkthrough`.

### Task B1: Protocol types

**Files:**
- Create: `crates/trace-commons-protocol/src/terms.rs`
- Modify: `crates/trace-commons-protocol/src/lib.rs` (`pub mod terms;`)
- Test: unit tests in `terms.rs`; fixture `crates/trace-commons-protocol/tests/fixtures/terms-v1.json` (a trimmed copy of the Part A output: one of each block kind)

**Interfaces (Produces):**
```rust
pub const TERMS_SCHEMA_VERSION: &str = "trace_commons.terms.v1";
pub const TERMS_ACCEPTANCE_PATH: &str = "/v1/account/terms-acceptance";
pub struct TermsDocument { pub schema_version: String, pub version: String, pub effective: String, pub in_effect: bool, pub title: String, pub blocks: Vec<TermsBlock> }
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TermsBlock { Heading { text: String }, Paragraph { text: String }, List { items: Vec<String> }, Table { header: Vec<String>, rows: Vec<Vec<String>> }, Callout { text: String } }
impl TermsDocument { pub fn parse(bytes: &[u8]) -> Result<TermsDocument, &'static str> }
pub fn valid_terms_version(v: &str) -> bool;
pub fn valid_content_sha256(h: &str) -> bool;
#[serde(deny_unknown_fields)] pub struct TermsAcceptanceRequest { pub terms_version: String, pub content_sha256: String, pub accepted_at: DateTime<Utc> }
impl TermsAcceptanceRequest { pub fn validate(&self, now: DateTime<Utc>) -> Result<(), &'static str> }
pub struct TermsAcceptanceResponse { pub recorded_at: DateTime<Utc> }
```
`parse` refusal labels: `terms-malformed` (JSON/shape), `terms-schema-unknown`, `terms-version-invalid`, `terms-empty` (no blocks), `terms-link-refused` (a link target not `https://` or `mailto:`). `validate` labels: `terms-version-invalid`, `terms-hash-invalid`, `terms-accepted-at-future`.

- [ ] **Step 1: Write the tests:** `parses_every_block_kind` (fixture -> 5 kinds, `in_effect == false`); `refuses_unknown_schema`; `refuses_unknown_block_kind` (-> `terms-malformed`); `refuses_relative_and_javascript_links` (`[x](/a)` and `[x](javascript:alert(1))` -> `terms-link-refused`, `[x](mailto:a@b.c)` ok); `version_shape` (`""`, 65 chars, `"a b"` false; `"2026-08-22-draft"` true); `hash_shape` (uppercase hex false); `accepted_at_bounds` (now+4min ok, now+6min -> `terms-accepted-at-future`); `request_rejects_unknown_fields`.
- [ ] **Step 2:** Implement. Link targets are found with a small scan for `](` ... `)` in every text field; no Markdown dependency.
- [ ] **Step 3:** `cargo check -p trace-commons-protocol --all-targets`. Commit: `Add terms document and acceptance protocol types`.

### Task B2: Migration V158 and store method

**Files:**
- Create: `migrations/V158__account_terms_acceptances.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (MIGRATIONS row `(158, "account_terms_acceptances", include_str!(...))`; `impl Database for PgBackend` delegation; a V158 guard test beside the V157 one)
- Modify: `crates/trace-commons-server/src/db/mod.rs` (trait method with default)
- Create: `crates/trace-commons-server/src/db/postgres_terms_acceptance.rs` (AGPL header)
- Create: `crates/trace-commons-server/tests/terms_acceptance_pg.rs` + `[[test]]` in `crates/trace-commons-server/Cargo.toml` (reason: PostgreSQL suite)

**Interfaces:**
- Produces (`Database` trait, default returns `Err(DatabaseError::Pool("terms_acceptance_unavailable".into()))`):
```rust
async fn record_terms_acceptance(&self, tenant_id: &str, account_id: Uuid, auth_method: &str, client_kind: &str, terms_version: &str, content_sha256: &str, accepted_at: DateTime<Utc>) -> Result<DateTime<Utc>, DatabaseError>
```
Returns `recorded_at`; on a repeat of the unique key returns the original row's `recorded_at`.

Table: `account_terms_acceptances(tenant_id TEXT NOT NULL REFERENCES trace_tenants ON DELETE CASCADE, account_id UUID NOT NULL, auth_method TEXT NOT NULL, client_kind TEXT NOT NULL, terms_version TEXT NOT NULL CHECK (terms_version ~ '^[A-Za-z0-9._-]{1,64}$'), content_sha256 TEXT NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'), accepted_at TIMESTAMPTZ NOT NULL, recorded_at TIMESTAMPTZ NOT NULL DEFAULT now(), UNIQUE (tenant_id, account_id, terms_version, content_sha256), FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE)`. ENABLE + FORCE RLS, policy `trace_corpus_tenant_isolation` on `trace_current_tenant_id()` (copy V97). `REVOKE ALL ... FROM PUBLIC; GRANT SELECT ON ... TO trace_ingest_runtime; GRANT INSERT (tenant_id, account_id, auth_method, client_kind, terms_version, content_sha256, accepted_at) ON ... TO trace_ingest_runtime;` No UPDATE/DELETE grant.

- [ ] **Step 1: Write the PG tests** (self-skip without `TRACE_COMMONS_PG_TEST_DATABASE_URL`, connect as `trace_ingest_runtime` like `model_training_consent_pg.rs`): `records_and_returns_recorded_at`; `repeat_is_idempotent` (second call returns the first `recorded_at`); `new_version_is_a_new_row`; `other_tenant_cannot_read` (RLS); `runtime_role_cannot_update_or_delete` (permission denied); `check_constraints_refuse_bad_hash`.
- [ ] **Step 2:** Write the migration, register it, add the V158 guard test (copy the V157 guard's assertions).
- [ ] **Step 3:** Implement `record_terms_acceptance_in_tx`: set tenant with `set_config('trace_commons.trace_tenant_id', $1, true)`, `INSERT ... ON CONFLICT (tenant_id, account_id, terms_version, content_sha256) DO NOTHING RETURNING recorded_at`; if no row returned, `SELECT recorded_at` for the key.
- [ ] **Step 4:** `cargo check -p trace-commons-server --all-targets`. Commit: `Store terms acceptances per account and org`.

### Task B3: Ingest route

**Files:**
- Create: `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/terms_acceptance.rs` (AGPL header; `#[path]` mod near `upload_allowance` in `trace-commons-ingest.rs`)
- Modify: `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs` (`account_route_groups()`: `.post_with_body_limit(TERMS_ACCEPTANCE_PATH, terms_acceptance::handler, 4096)`)
- Create: `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/tests/terms_acceptance_tests.rs` (registered in `tests.rs`)

**Interfaces:**
- Consumes: B1 `TermsAcceptanceRequest/Response`, B2 `record_terms_acceptance`.
- Produces: `pub(super) async fn handler(State(state): State<Arc<AppState>>, Extension(ctx): Extension<AccountCtx>, headers: HeaderMap, Json(body): Json<TermsAcceptanceRequest>) -> ApiResult<(HeaderMap, Json<TermsAcceptanceResponse>)>`. All three `AccountAuthMethod`s are accepted; `SessionCookie` also requires `confirm_is_same_origin(&headers)` (403 `cross-origin account mutation`). Validation failure -> 400 with the B1 label. Store via `account_db(state)?` and `internal_error`. Response carries `Cache-Control: no-store`. `auth_method` text: `session_cookie` / `device_bearer` / `native_token` (add `AccountAuthMethod::as_str` in `account_session.rs` and use it at the logout site too).

- [ ] **Step 1: Write the tests:** `terms_acceptance_requires_a_session` (no auth -> 401); `rejects_cross_site_cookie` (403); `rejects_bad_hash_and_future_time` (400 `terms-hash-invalid`, 400 `terms-accepted-at-future`); `rejects_unknown_fields` (422 or 400, whichever axum's `Json` rejection gives today; assert not 2xx); PG-backed `records_once_and_repeats_idempotently` (two POSTs, same `recorded_at`, `Cache-Control: no-store`) using `account_session_headers`; `route_is_registered_as_post` against the recorded route list.
- [ ] **Step 2:** Implement. **Step 3:** `cargo check -p trace-commons-ingest --all-targets`. Commit: `Accept terms acceptances on the account API`.

### Task B4: Channel constant and the fetch

**Files:**
- Modify: `crates/trace-commons-contributor/src/channel.rs` (`TERMS_DOCUMENT_URL` in both `values` modules, both `"https://tracecommons.ai/legal/terms.json"`; re-export; add to `HOST_VALUES`)
- Modify: `crates/trace-commons-contributor/tests/channel_hosts.rs` (a named `SHARED_WITH_PRODUCTION: &[&str] = &[channel::TERMS_DOCUMENT_URL]` exception to the staging-hosts-only assertion, with the owner decision of 2026-11-09 in a comment; a test `only_the_terms_document_is_shared` that the list has exactly that one entry)
- Create: `crates/trace-commons-contributor/src/terms.rs` (`pub mod terms;` in lib.rs)

**Interfaces (Produces):**
```rust
pub const TERMS_MAX_BYTES: usize = 262_144;
pub struct FetchedTerms { pub document: TermsDocument, pub content_sha256: String }
pub async fn fetch_published() -> Result<FetchedTerms, &'static str>;          // TERMS_DOCUMENT_URL, production policy
pub(crate) async fn fetch_from(url: &reqwest::Url, allowed_host: &str, require_https: bool) -> Result<FetchedTerms, &'static str>;
pub fn fetch_published_blocking() -> Result<FetchedTerms, &'static str>;      // own current-thread runtime, for the C ABI
```
Labels: `terms-unavailable` (network, non-2xx, timeout, redirect), `terms-host-refused`, `terms-too-large`, plus B1's parse labels. Copy the bounded read from `credit_estimate_table.rs` (`get_url_bounded` / `read_bounded`). Hash with `sha2::Sha256` over the exact bytes, lowercase hex.

- [ ] **Step 1: Write the tests** (local listener, `require_https = false`, same test server approach as `credit_estimate_table` tests): `fetches_and_hashes_exact_bytes` (hash equals `sha256` of the served bytes); `refuses_a_redirect` (302 -> `terms-unavailable`); `refuses_another_host` (`terms-host-refused`, no request made); `refuses_oversize_by_header_and_by_stream` (`terms-too-large`); `refuses_bad_schema` (`terms-schema-unknown`); `production_policy_requires_https` (`fetch_from("http://tracecommons.ai/..", "tracecommons.ai", true)` -> `terms-host-refused`).
- [ ] **Step 2:** Implement. **Step 3:** `cargo check -p trace-commons-contributor --all-targets`. Commit: `Fetch the published terms document`.

### Task B5: Terms state file and hold rule

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/terms_state.rs`

**Interfaces (Produces):**
```rust
pub const TERMS_STATE_FILE: &str = "terms-state.json";
pub struct AcceptedTerms { pub version: String, pub content_sha256: String, pub accepted_at: DateTime<Utc> }
pub struct RecordedIn { pub tenant_id: String, pub recorded_at: DateTime<Utc> }
pub struct PublishedTerms { pub version: String, pub content_sha256: String, pub in_effect: bool, pub checked_at: DateTime<Utc> }
pub struct TermsState { pub accepted: Option<AcceptedTerms>, pub recorded_in: Vec<RecordedIn>, pub published: Option<PublishedTerms> }
impl TermsState {
    pub fn load(store: &ConfigStore) -> Result<TermsState>;        // missing file -> Default
    pub fn save(&self, store: &ConfigStore) -> Result<()>;          // write_daemon_file (0600, atomic)
    pub fn held(&self) -> bool;
    pub fn accept(&mut self, terms: AcceptedTerms) -> Result<(), &'static str>;
    pub fn needs_post(&self, tenant_id: &str) -> bool;
    pub fn mark_recorded(&mut self, tenant_id: &str, recorded_at: DateTime<Utc>);
}
```
`held()` is `published.in_effect && accepted.map(|a| a.version) != Some(published.version)`; false when `published` is None. `accept` refuses `terms-version-stale` when `published` is Some and its version differs from `terms.version`; otherwise replaces `accepted` and clears `recorded_in`. `needs_post` is `accepted.is_some() && !recorded_in.contains(tenant)`.

- [ ] **Step 1: Write the tests:** `no_published_version_never_holds`; `draft_version_never_holds` (`in_effect: false`, nothing accepted -> not held); `in_effect_version_holds_until_accepted`; `user_with_no_record_is_held` (`accepted: None`, in-effect published -> held); `accept_clears_recorded_in`; `accept_refuses_stale_version`; `round_trips_with_0600` (unix mode check).
- [ ] **Step 2:** Implement. **Step 3:** `cargo check`. Commit: `Keep terms acceptance state in the state directory`.

### Task B6: Daemon IPC, posting, and status

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (sync arm `"terms_accept" => terms_state::handle_accept(shared, req)`; `DaemonShared.terms_held: AtomicBool`, loaded from `TermsState` in `DaemonShared::load`; status keys `"terms_hold": bool` and `"terms_version_owed": Option<String>` beside `consent_hold`; update the status-key and dispatcher-arm source-scan tests to include them)
- Modify: `crates/trace-commons-contributor/src/daemon/account_contribution.rs` (make `call` `pub(crate)`)
- Modify: `crates/trace-commons-contributor/src/daemon/terms_state.rs`

**Interfaces:**
- Produces IPC `terms_accept` params `{"version","content_sha256","accepted_at"}` -> `{"held": false}` or error `ERR_BAD_PARAMS` with `terms-version-stale` / B1 labels. Publishes `EVENT_STATUS_CHANGED`.
- Produces `pub(crate) async fn post_pending(shared: &DaemonShared, now: DateTime<Utc>) -> Result<()>`: if `cfg.tenant_id` needs a post and a session exists, `account_contribution::call(.., Method::POST, TERMS_ACCEPTANCE_PATH, Some(&TermsAcceptanceRequest), None)`; on 2xx `mark_recorded` + save; `account-sign-in-required` is a quiet no-op; other errors log the label only and retry next tick.

- [ ] **Step 1: Write the tests:** `accept_lifts_the_hold_and_publishes_status`; `accept_refuses_a_version_that_is_no_longer_published`; `accept_does_not_resume_a_user_pause` (paused before -> still paused after); `resume_does_not_lift_the_terms_hold`; `status_reports_terms_hold_and_owed_version`; `posts_once_per_tenant` (fake ingest: one POST, second tick none); `posts_again_when_the_tenant_changes`; `no_session_is_a_quiet_no_op` (no POST, no error label logged).
- [ ] **Step 2:** Implement. **Step 3:** `cargo check`. Commit: `Accept terms over IPC and record them per org`.

### Task B7: Re-check, hold enforcement, expiry

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs` (`TickState.terms_schedule`; pass `"terms"` in `run_tick` before `"expire"`, running `refresh_terms` when due (start, then every 21600 s) and `post_pending` every tick; `drain_approved` returns early when `shared.terms_held`; `expire_and_digest` passes `blocked || terms_held` to `Queue::expire`)
- Modify: `crates/trace-commons-contributor/src/daemon/terms_state.rs`

**Interfaces:**
- Produces `pub(crate) async fn refresh_terms(shared: &DaemonShared, now: DateTime<Utc>) -> Result<()>`: `terms::fetch_published()`; on success set `published`, save, update `terms_held`, publish status if it changed; on failure log the label and change nothing.

- [ ] **Step 1: Write the tests:** `a_new_in_effect_version_starts_the_hold`; `a_failed_fetch_never_starts_a_hold`; `held_uploads_nothing` (approved entry stays approved, no upload call); `held_entries_do_not_expire` (entry older than `queue_ttl_days` survives `expire_and_digest` while held, expires once lifted); `hold_survives_restart_without_network` (persisted `published` re-loads `terms_held = true`); `recheck_runs_at_start_then_every_six_hours`.
- [ ] **Step 2:** Implement. **Step 3:** `cargo check`. Commit: `Hold uploads while new terms are owed`.

### Task B8: C ABI and copy

**Files:**
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs` (`tc_terms_document_json`)
- Modify: `crates/trace-commons-contributor-ffi/include/trace_commons.h`, then `cp` it to `macos/Sources/CTraceCommons/include/trace_commons.h`
- Modify: `crates/trace-commons-contributor/src/first_run_copy.rs` (new strings)

**Interfaces:**
- Produces `char *tc_terms_document_json(char **err);` -> owned JSON `{"document": <TermsDocument>, "content_sha256": "<hex>"}` or NULL with `*err` an owned label; free both with `tc_string_free`. Calls `terms::fetch_published_blocking()`; needs no daemon or state dir.
- Produces copy (all DRAFT, NEEDS APPROVAL): `WelcomeLegalCopy` gains `scroll_hint: "Scroll to the end to accept"`, `loading: "Loading the Terms of Service"`, `failed: "Couldn't load the Terms of Service. Check your connection."`, `retry: "Try again"`, `not_in_effect: "These terms are a draft and are not yet in effect."`, `accept_button: "Accept"`. New top-level `terms_hold: TermsHoldCopy { notice_title: "New Terms of Service", notice_body: "Uploads are paused until you accept.", review: "Review terms", menu_row: "Uploads paused: new Terms of Service" }`.

- [ ] **Step 1: Write the tests:** FFI `terms_document_json_tolerates_a_null_err_pointer` (call with `err = NULL` under a test build where `fetch_published_blocking` is pointed at a closed local port through a `#[cfg(test)]` URL override in `terms.rs`; returns NULL, no crash) and `terms_document_json_sets_an_owned_label` (same override; `*err` is `terms-unavailable`); `abi_header_surface` covers the declaration in both headers; copy test in `first_run_copy.rs` that every new string is non-empty and has no trailing period on labels.
- [ ] **Step 2:** Implement. **Step 3:** `cargo check -p trace-commons-contributor-ffi --all-targets`. Commit: `Expose the terms document over the C ABI`.

### Task B9: Swift bridge and models

**Files:**
- Create: `macos/Sources/TCBridge/TCTerms.swift`
- Create: `macos/Sources/TCShellCore/TermsDocument.swift`
- Modify: `macos/Sources/TCShellCore/FirstRunCopy.swift` (decode new fields and `termsHold`)
- Modify: `macos/Sources/TraceCommonsApp/Models.swift` (`DaemonStatus.termsHold: Bool`, key `terms_hold`, `decodeIfPresent ?? false`, carried through the init copy)
- Modify: the menu panel's `DaemonData.Status` (same key)
- Modify: `macos/Sources/TraceCommonsApp/DaemonClient.swift` (`func termsAccept(_ terms: AcceptedTerms) throws`)
- Test: `macos/Tests/TCBridgeTests/FirstRunCopyExportTests.swift`, new `macos/Tests/TCShellCoreTests/TermsDocumentTests.swift`

**Interfaces (Produces):**
```swift
public struct TermsDocument: Decodable, Equatable { public let version: String; public let effective: String; public let inEffect: Bool; public let title: String; public let blocks: [Block]
  public enum Block: Decodable, Equatable { case heading(String), paragraph(String), list([String]), table(header: [String], rows: [[String]]), callout(String) } }
public struct FetchedTerms: Equatable { public let document: TermsDocument; public let contentSHA256: String }
public struct AcceptedTerms: Equatable { public let version: String; public let contentSHA256: String; public let acceptedAt: Date }
public enum TCTerms { public static func fetch() -> Result<FetchedTerms, TCTermsRefusal> }   // TCTermsRefusal { let label: String }
```

- [ ] **Step 1: Write the tests:** `testDecodesEveryBlockKind` (fixture JSON from B1); `testUnknownBlockKindFailsDecode`; `testWelcomeLegalDecodesNewStrings` (exact strings above); `testTermsHoldCopyDecodes`; `testStatusDecodesTermsHoldDefaultFalse`.
- [ ] **Step 2:** Implement. Commit: `Bridge the terms document and hold state to Swift`.

### Task B10: Terms reader view and Welcome

**Files:**
- Create: `macos/Sources/TCShellCore/TermsScrollGate.swift`
- Create: `macos/Sources/TraceCommonsApp/Views/Terms/TermsReader.swift` (`TermsLoader` + `TermsReader` view + block rendering)
- Modify: `macos/Sources/TraceCommonsApp/Views/FirstRun/WelcomeScreen.swift` (replace `FirstRunLegalPlaceholder` with `TermsReader`)
- Modify: `macos/Sources/TraceCommonsApp/FirstRunRunner.swift`, `AppModel.swift` (FirstRunDaemon conformance)
- Test: new `macos/Tests/TCShellCoreTests/TermsScrollGateTests.swift`; `macos/Tests/TraceCommonsAppTests/AnimatedFirstRunTests.swift`, `FirstRunRunnerTests.swift`

**Interfaces:**
- Produces `public struct TermsScrollGate { public init(); public private(set) var reachedEnd: Bool; public mutating func observe(contentHeight: CGFloat, viewportHeight: CGFloat, offset: CGFloat) }`: reached when `offset + viewportHeight >= contentHeight - 8`; once true, stays true.
- Produces `@MainActor final class TermsLoader: ObservableObject { enum State { case loading, loaded(FetchedTerms), failed(String) }; @Published private(set) var state: State; init(fetch: @escaping () async -> Result<FetchedTerms, TCTermsRefusal>); func load() async }`.
- Produces `struct TermsReader: View { init(copy: FirstRunCopy.Welcome.Legal, loader: TermsLoader, onAccepted: (AcceptedTerms?) -> Void) }`: fixed-height (proposal 260pt) scroll region; checkbox `.disabled(!gate.reachedEnd)` with the scroll hint as its help/accessibility hint; not-in-effect line when `!inEffect`; Try again re-runs `load()`; the Privacy Policy link below. Ticking yields `AcceptedTerms(version, contentSHA256, acceptedAt: Date())`, unticking yields nil. VoiceOver: the last block's `onAppear` also calls the gate with offset = contentHeight.
- Runner: replace `termsAccepted: Bool` with `@Published var acceptedTerms: AcceptedTerms?`; `canLeaveWelcome(legal:)` keeps its signature and requires `acceptedTerms != nil`. `FirstRunDaemon` gains `func recordTermsAcceptance(_ terms: AcceptedTerms) async -> Bool`; after `.startDaemon` succeeds and `acceptedTerms` is set and not yet handed over, call it once; a false return is logged by label and does not fail first run (the hold flow takes over).

- [ ] **Step 1: Write the tests:** `testShortDocumentUnlocksImmediately`; `testUnlocksAtEndWithinTolerance`; `testScrollingBackDoesNotRelock`; `testWelcomeBlockedWhileLoadingAndFailed`; `testWelcomeRequiresScrolledAcceptance` (replaces the old termsAccepted test); `testTermsHandedToDaemonOnceAfterStart` (RecordingFirstRunDaemon logs `.recordTerms` exactly once, after `.startDaemon`); `testRefusedHandOffDoesNotFailFirstRun`.
- [ ] **Step 2:** Implement. Commit: `Read the terms through before accepting in first run`.

### Task B11: Hold UI on Home and in the menu bar

**Files:**
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/MonitorNotices.swift` (`.home` branch: item id `terms-hold`, no `closeLabel`/`onClose`, primary button `copy.termsHold.review` setting `model.termsSheetOpen = true`)
- Create: `macos/Sources/TraceCommonsApp/Views/Terms/TermsHoldPresenter.swift` (ViewModifier like `AccountLoginChangePresenter`, applied in `TraceCommonsAppMain.swift`; shows `TermsReader` in a glass modal with the Accept button)
- Modify: `macos/Sources/TraceCommonsApp/AppModel.swift` (`@Published var termsSheetOpen = false`; `func acceptNewTerms(_ terms: AcceptedTerms) async -> Bool` calling `termsAccept`; on `terms-version-stale` reload the loader and keep the sheet open)
- Modify: `macos/Sources/TraceCommonsApp/Views/Monitor/MenuBarGlassPanel.swift` (`termsHoldRow` above `nudgeRow` when status `termsHold`; action sets `model.termsSheetOpen = true` then `open(.home(<Home's default page>))`)
- Test: `macos/Tests/TraceCommonsAppTests/TermsHoldTests.swift`

- [ ] **Step 1: Write the tests:** `testNoticeShownOnlyWhileHeld`; `testNoticeHasNoCloseAction`; `testReviewOpensSheet`; `testMenuRowShownOnlyWhileHeldAndOpensHomeWithSheet`; `testAcceptClosesSheetAndNoticeGoesWithHold`; `testStaleAcceptKeepsSheetAndReloads`.
- [ ] **Step 2:** Implement. Commit: `Ask for new terms from Home and the menu bar`.

### Task B12: Full verification

- [ ] Run once, in order, through the turn locks: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`; `... cargo test -p trace-commons-server --no-run`; same two for `-p trace-commons-ingest`; `cargo test -p trace-commons-protocol`; `cargo test -p trace-commons-contributor`; `cargo test -p trace-commons-contributor-ffi`; `cargo test -p trace-commons-contributor --test it -- 'channel_hosts::'`; clippy with the repo allow-list; `cargo fmt --all -- --check`; `cargo deny check licenses`.
- [ ] PostgreSQL: `cargo test -p trace-commons-server --test terms_acceptance_pg` and the ingest terms tests with `TRACE_COMMONS_PG_TEST_DATABASE_URL` and the login-resolver URL, single-threaded, on a fresh `admission_test_*` database.
- [ ] `cargo build -p trace-commons-contributor-ffi`, then `swift test` in `macos/`.
- [ ] Any failure that also fails on `main` is noted and left. Fix the rest against the spec, never by weakening a test.
- [ ] Manual: build the bundle, run a fresh profile, scroll and accept in Welcome; point a local build at a fixture with `in_effect: true` and a new version to see the Home notice and menu row.
