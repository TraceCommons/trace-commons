# Pilot Allowlist Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a file-backed invite-code allowlist on `trace-commons-upload-claim-issuer` so the pilot can run "Ironclaw contributors only" without restarts. The issuer becomes the issuance boundary; downstream server controls (tenant access grants, central-issuer ABAC, RLS) keep operating against the claims it mints.

**Architecture:** Three additive layers behind a single off-by-default flag. (1) A new `invite_code` optional field on `WorkloadClaims` + a `policy_label` optional field on `UploadClaimClaims`. (2) An `AllowlistSource` trait + file source + snapshot cache that refreshes on a timer, with fail-closed past a max-stale window. (3) An optional second axum server on a localhost-only admin bind exposing `/v1/admin/allowlist-status` (counts only). No existing route changes shape; no migration; existing tests pass unchanged.

**Tech Stack:** Rust, Axum 0.8 (already in tree), `serde` + `serde_json`, `tokio` (already in tree). No new direct deps — `sha2` and `chrono` are already on `trace-commons-server`.

**Spec:** `docs/superpowers/specs/2026-05-17-pilot-allowlist-design.md`

---

## File Map

**New files**

| Path | Responsibility |
|------|----------------|
| `crates/trace-commons-server/src/trace_upload_claim_allowlist.rs` | `AllowlistSource` trait, `AllowlistSnapshot`, `FileAllowlistSource`, JSON schema deserializer, denial-counter |
| `crates/trace-commons-server/src/trace_upload_claim_issuer_admin.rs` | Admin router with `/v1/admin/allowlist-status`; dual-bind glue |
| `crates/trace-commons-server/tests/trace_upload_claim_allowlist.rs` | Contract tests for the allowlist source + integration tests for the issuance flow |
| `docs/operator/pilot-allowlist.md` | Operator runbook: how to provision invite codes, how to read `/v1/admin/allowlist-status`, denial-smoke procedure, rollback |

**Modified files**

| Path | What changes |
|------|--------------|
| `crates/trace-commons-server/src/lib.rs` | `pub mod trace_upload_claim_allowlist; pub mod trace_upload_claim_issuer_admin;` |
| `crates/trace-commons-server/src/trace_upload_claim_issuer.rs` | `WorkloadClaims` gains `invite_code: Option<String>`. `UploadClaimClaims` gains `policy_label: Option<String>`. `IssuerError` gains `PilotAllowlistNotMatched` / `PilotAllowlistStale` / `PilotAllowlistInviteCodeMissing` / `PilotAllowlistMalformed` variants with hash-only `tracing::warn`. `TraceUploadClaimIssuerConfig` gains `allowlist_source`, `allowlist_refresh_interval_seconds`, `allowlist_max_stale_seconds`, `admin_bind`. `validate_authenticated_workload_claims` (or its sibling that builds the minted claim) checks the snapshot and embeds `policy_label`. `serve_trace_upload_claim_issuer` spins the optional admin bind. |
| `crates/trace-commons-server/src/bin/trace-commons-upload-claim-issuer.rs` | No change — the binary stays a thin shell over the library. |

**Out of scope**

- The `near:<account>:<view>` allowlist source. Reserved in the CLI surface; this slice ships file-only.
- Reading `policy_label` on the ingest side. Issuer starts emitting it; ingest consumption is a follow-up slice.
- Operator bearer-token alternative for the admin endpoint. Localhost-only bind is the choice; if operations later asks for a bearer, that's an additive change.
- Changes to the Ironclaw client. Spec notes the client (and the dev workload-token signer) must populate `invite_code` before any allowlisted contributor's first refresh succeeds; that wiring lives on the client side.

---

## Pre-flight

- [ ] Establish a green baseline against the four CI gates that will judge this work.

```bash
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins
RUSTFLAGS='-D warnings' cargo test  -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -D warnings -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```

Expected: all four clean on `main`. If anything fails, stop and fix before starting.

---

## Slice 1 — invite-code field, snapshot type, file source

### Task 1: extend the workload + claim structs

- [ ] Add `invite_code: Option<String>` to `WorkloadClaims` in `trace_upload_claim_issuer.rs:333`. `#[serde(default)]` so existing workload tokens parse unchanged.
- [ ] Add `policy_label: Option<String>` to `UploadClaimClaims` in `trace_upload_claim_issuer.rs:354` with `#[serde(skip_serializing_if = "Option::is_none")]` so the minted JWT omits the field when no allowlist is configured.
- [ ] Add four error class variants to the issuer's error vocabulary (currently `IssuerError::bad_request` / `forbidden` / `internal`): `pilot_allowlist_not_matched()` (403), `pilot_allowlist_invite_code_missing()` (400), `pilot_allowlist_stale()` (503), `pilot_allowlist_malformed()` (503). Each returns an `IssuerError` with the exact label as the `message` field so the JSON error body is `{"error":"PilotAllowlistNotMatched"}`. This is the public refusal vocabulary — operators will grep for these labels.
- [ ] Existing tests must still pass with no edits — these additions are purely additive.

### Task 2: allowlist snapshot + JSON schema

- [ ] Create `crates/trace-commons-server/src/trace_upload_claim_allowlist.rs` with:
  - `AllowlistEntry { subject_hash: String, tenant_id: String, note_label: Option<String> }` (`Deserialize` only — operator authors the file; issuer never serializes it).
  - `AllowlistFile { version: u32, generated_at: chrono::DateTime<Utc>, policy_label: String, entries: Vec<AllowlistEntry> }` (`Deserialize`). Reject `version != 1` at parse time with `PilotAllowlistMalformed`.
  - `AllowlistSnapshot { policy_label: String, generated_at: DateTime<Utc>, subject_hashes: HashSet<String>, loaded_at: Instant, source_label: String }`. Construction from `AllowlistFile`: dedupe `subject_hashes` (operator typo recovery), normalize each to lowercase, reject any entry whose hash isn't the canonical `sha256:<64 hex>` shape with `PilotAllowlistMalformed`.
  - Pure helper `pub fn hash_invite_code(code: &str) -> String` returning `sha256:hex(sha256("invite:" + code))`. This is what the operator runs locally to generate `subject_hash` values, and what the issuer runs at lookup time. Same function on both sides so they can never drift.
- [ ] Add `pub mod trace_upload_claim_allowlist;` to `lib.rs`.
- [ ] Pure-unit tests in the new module's `#[cfg(test)] mod tests`:
  - schema deserializer: happy path, unknown fields tolerated, `version: 2` rejected, malformed hash rejected, duplicate hashes deduped.
  - `hash_invite_code`: stable across runs, distinct for distinct inputs, distinct for invite codes that share a prefix.

### Task 3: file source + refresh cache

- [ ] In the same module, add:

  ```rust
  pub trait AllowlistSource: Send + Sync {
      fn snapshot(&self) -> AllowlistSnapshot;
  }
  ```

  with a single `FileAllowlistSource { path: PathBuf, refresh_interval: Duration, cached: Mutex<Option<AllowlistSnapshot>> }` impl. `snapshot()` returns the cached snapshot if `loaded_at.elapsed() < refresh_interval`; otherwise re-reads and parses the file. On parse / IO failure, log a hash-only `tracing::warn` with `error_class = PilotAllowlistMalformed` (or `PilotAllowlistSourceMissing`) and `source_label`, then return the previously-cached snapshot. The first-load failure (no cached snapshot at all) returns an error so the issuer can refuse `POST /v1/trace-upload-claim` with `PilotAllowlistStale` until a good file lands.
- [ ] Parse `--allowlist-source <SPEC>` / `TRACE_COMMONS_ALLOWLIST_SOURCE` into `enum AllowlistSourceSpec { File(PathBuf), Near { account: String, view_method: String } }`. The `Near` variant is reserved — parse it but reject at construction with `anyhow!("PilotAllowlistNearSourceNotImplemented: use file:<path>")` so the CLI surface is stable when the on-chain source ships in a later slice.
- [ ] Tests in `tests/trace_upload_claim_allowlist.rs`:
  - First `snapshot()` reads the file. Second call within the refresh interval does **not** touch the filesystem (write-then-mutate-file-then-read confirms cache).
  - After refresh interval, second call re-reads.
  - Delete file mid-run: snapshot still returns the cached value; `tracing::warn` fires with `PilotAllowlistSourceMissing`.
  - Corrupt file mid-run: snapshot still returns the cached value; warn fires with `PilotAllowlistMalformed`.

---

## Slice 2 — wire the snapshot check into issuance + `policy_label` emission

### Task 4: config + state plumbing

- [ ] Extend `TraceUploadClaimIssuerConfig` (`trace_upload_claim_issuer.rs:42`) with:

  ```rust
  pub allowlist_source: Option<AllowlistSourceSpec>,
  pub allowlist_refresh_interval_seconds: u64, // default 60
  pub allowlist_max_stale_seconds: u64,        // default 3600
  pub admin_bind: Option<std::net::SocketAddr>, // None => admin disabled
  ```

- [ ] `from_env` reads `TRACE_COMMONS_ALLOWLIST_SOURCE`, `TRACE_COMMONS_ALLOWLIST_REFRESH_INTERVAL_SECONDS`, `TRACE_COMMONS_ALLOWLIST_MAX_STALE_SECONDS`, `TRACE_COMMONS_ISSUER_ADMIN_BIND`. All optional; missing source = allowlist disabled.
- [ ] `build_state` constructs the optional `Arc<dyn AllowlistSource>`. Stored on the existing router state struct so the handler can read it cheaply.

### Task 5: snapshot check in the issuance handler

- [ ] In `validate_authenticated_workload_claims` (or wherever the validated workload identity is in scope before signing), if the state has an allowlist:
  1. Extract `invite_code` from `WorkloadClaims`. If `None`, return `IssuerError::pilot_allowlist_invite_code_missing()`. Log `tracing::warn` with `error_class = PilotAllowlistInviteCodeMissing`.
  2. Compute `subject_hash = hash_invite_code(&invite_code)`.
  3. Call `source.snapshot()`. If `loaded_at.elapsed() > Duration::from_secs(config.allowlist_max_stale_seconds)`, return `IssuerError::pilot_allowlist_stale()` and log `tracing::warn` with `error_class = PilotAllowlistStale`, `source_label`, `snapshot_age_seconds`.
  4. If `!snapshot.subject_hashes.contains(&subject_hash)`, return `IssuerError::pilot_allowlist_not_matched()` and log `tracing::warn` with `error_class = PilotAllowlistNotMatched`, `subject_hash` (the hash, not the code), `policy_label`, `source_label`. Never log the raw invite code.
  5. Increment the denial-counter (Task 7) for `PilotAllowlistNotMatched` only — invite-code-missing and stale are operator-error categories, tracked separately so dashboards stay clean.
- [ ] In the same handler, when constructing `UploadClaimClaims`, set `policy_label: Some(snapshot.policy_label.clone())` when the allowlist passed; leave `None` when the allowlist is disabled. The minted JWT body will then carry `policy_label` only on allowlist-enforced deployments.
- [ ] Existing integration tests on the issuance happy path must still pass — exercise with `allowlist_source: None`.

### Task 6: integration test the whole flow

- [ ] In `tests/trace_upload_claim_allowlist.rs`:
  - Build a router with an allowlist file containing one entry (`hash_invite_code("INV-PILOT-001")`). Post a workload token whose `invite_code = "INV-PILOT-001"`. Expect 200 with a JWT whose decoded body contains `policy_label`.
  - Same router, post a workload token with `invite_code = "INV-NOT-LISTED"`. Expect 403 with body `{"error":"PilotAllowlistNotMatched"}`.
  - Same router, post a workload token with no `invite_code`. Expect 400 with `{"error":"PilotAllowlistInviteCodeMissing"}`.
  - Build a router whose `max_stale_seconds = 0` and whose file refresh interval is shorter than the file's mtime delta (or inject `loaded_at` via a test seam). Confirm subsequent posts get 503 `PilotAllowlistStale`.
  - Build a router with `allowlist_source: None`. Post a workload token with no `invite_code`. Expect 200 (back-compat: the field is invisible without the allowlist).

---

## Slice 3 — admin endpoint + ops runbook

### Task 7: denial-counter

- [ ] In `trace_upload_claim_allowlist.rs`, add `DenialCounter { window: Duration, samples: Mutex<VecDeque<Instant>> }` with `record()` and `count_in_window()`. Default window 3600 seconds (one hour). Trim samples older than the window on each call so the in-memory footprint stays bounded.
- [ ] Wire into the issuance handler's `PilotAllowlistNotMatched` path (Task 5 step 5). Restart resets — fine; this is process-local readiness, not an audit surface.
- [ ] Unit tests: `record` then `count_in_window` returns the right value; samples past the window evict; thread-safety under concurrent record (use a quick `tokio::task::spawn` round-trip).

### Task 8: admin router + dual-bind

- [ ] Create `crates/trace-commons-server/src/trace_upload_claim_issuer_admin.rs`:

  ```rust
  pub fn admin_router(state: AdminState) -> Router { /* /v1/admin/allowlist-status */ }
  pub struct AdminState {
      pub source: Option<Arc<dyn AllowlistSource>>,
      pub denial_counter: Arc<DenialCounter>,
      pub max_stale_seconds: u64,
  }
  ```

  Handler returns the JSON described in the spec ("Operator-facing readiness" section). Field set degrades to `{"configured": false}` when `source` is `None`.
- [ ] In `serve_trace_upload_claim_issuer`, if `config.admin_bind.is_some()` AND `config.allowlist_source.is_some()`, bind a second `axum::serve` on the admin address with `admin_router`. On bind failure, `anyhow::bail!("PilotAllowlistAdminBindFailed: …")` so the process exits non-zero at startup. Reuse the same graceful-shutdown signal so SIGTERM tears both down in lockstep.
- [ ] Refuse to start when `admin_bind.is_some()` but the bind address is non-loopback (`!addr.ip().is_loopback()`): bail with `PilotAllowlistAdminBindNotLoopback`. Defense against an operator accidentally exposing the admin endpoint over the internet. Override with `TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC=1` for the (rare) case where someone wants a real internal bearer-gated mount.
- [ ] Test: `admin_router` returns `{"configured": false}` when source is `None`. Returns full fields when source is set. Refuses `0.0.0.0:8081` at startup with the named error label.

### Task 9: operator runbook

- [ ] Create `docs/operator/pilot-allowlist.md`. Sections:
  1. **What this gate does** — one paragraph, links to the spec.
  2. **Provisioning an invite code** — operator generates a random string (suggest 16 chars `[A-Z0-9]`, no leading zero), hashes via a copy-pasteable Python one-liner or `cargo run --bin trace-commons-upload-claim-issuer -- --hash-invite-code <CODE>` (if we ship that subcommand — open question), appends to the JSON file.
  3. **Reading `/v1/admin/allowlist-status`** — example `curl` against a localhost bind, what each field means, what to do when `stale: true`.
  4. **Denial smoke** — known-not-listed test code, expect 403 `PilotAllowlistNotMatched`.
  5. **Rollback** — unset the env, restart, behavior returns to MVP issuance.
  6. **Adding a contributor mid-pilot** — edit file, wait up to `refresh_interval_seconds` (default 60), no restart needed.
- [ ] Cross-link from `docs/operator/README.md` index.
- [ ] Add a brief mention in the top-level `README.md` "Status: Pilot Deployment" table row for "Contributor gate" pointing at the new runbook.

### Task 10: invite-code hashing helper subcommand (optional — operator-friendly)

- [ ] Add `--hash-invite-code <CODE>` subcommand to the issuer binary. Prints `sha256:<64 hex>` for the input and exits 0. Single delegated call to `hash_invite_code`. No tests — it's a trivial CLI shim — but include it in the help text and reference it from the runbook.

---

## Verification

After every slice, before moving to the next:

```bash
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins
RUSTFLAGS='-D warnings' cargo test  -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -D warnings -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
cargo test  -p trace-commons-server trace_upload_claim_allowlist
```

End of plan: full `cargo test -p trace-commons-server` is green, all four CI jobs would pass.

---

## Risk register

| Risk | Mitigation |
|---|---|
| Operator commits an allowlist JSON containing raw invite codes by mistake | The schema *only* accepts `subject_hash`; raw `invite_code` fields parse-rejected with `PilotAllowlistMalformed`. Schema documented at the top of the JSON file via `version` + `policy_label` so a glance tells you it's the right shape. |
| Allowlist file deleted in production | Snapshot cache holds last good for `max_stale_seconds` (default 1h); past that, issuance fails closed with `PilotAllowlistStale`. Operator gets `tracing::warn` per request. |
| Admin endpoint accidentally exposed to the public internet | Task 8 startup guard refuses non-loopback bind unless `TRACE_COMMONS_ISSUER_ADMIN_BIND_ALLOW_PUBLIC=1`. Default off. |
| Workload-token signers (pilot operator, dev tooling) don't know about `invite_code` | Existing tokens parse fine (`#[serde(default)]`); they just fail issuance with `PilotAllowlistInviteCodeMissing` — clear refusal class, easy to grep. The runbook (Task 9) calls this out as a coordination item. |
| Drift between operator-side and issuer-side hashing | Both call the same `hash_invite_code(&str)` — the operator via the `--hash-invite-code` subcommand (Task 10), the issuer in the handler (Task 5). Single source of truth. |

---

## Notes for the executor

- **Don't refactor the issuer module.** The existing 1,643-line `trace_upload_claim_issuer.rs` is large but stable; this slice adds beside it (`trace_upload_claim_allowlist.rs`, `trace_upload_claim_issuer_admin.rs`) and adds a small number of named fields / variants in-place. No module split.
- **Don't read or log raw invite codes.** Every diagnostic surface — log line, audit row, admin response, error body — uses `subject_hash` only. The raw code goes to the issuer in the workload token, gets hashed once, and is dropped.
- **Don't make the admin endpoint required for issuance to work.** It's optional; allowlist gating works without it. Operator can add it later without re-deploying anything else.
- **Don't introduce new direct deps.** Spec relies entirely on what's already in `trace-commons-server`'s Cargo.toml. If the executor finds themselves reaching for `tokio-cron`, `dashmap`, etc., they're over-engineering — stop and re-read the spec.
