# Consent-Scope Broadening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Execution model: Sonnet 5 implementer subagents, controller reviews between tasks. PROCESS RULE for every implementer: run all cargo commands in the FOREGROUND and wait inline; never background builds/tests, never arm monitors, never end a turn to "wait".

**Goal:** Device-key upload claims honor the enrollment-stored instance policy ceiling (intersected with the contributor's requested scopes) instead of a hardcoded `debugging_evaluation` cap, with consent flowing end-to-end: CLI interactive consent at login → claim request → granted-set echo in the claim response → envelope stamping → per-trace scope visibility in status read-back.

**Architecture:** Two server halves plus CLI plumbing. Server: (a) the enrollment grant writer stops hardcoding pilot-default scopes and writes the instance policy template's scopes into the `trace_tenant_access_grants` row; (b) `issue_claim_for_device_key`/`issue_claim_for_device_jwt` derive the actor's scope ceiling from that grant (fallback: today's hardcoded floor) and pre-intersect the request so the existing `enforce_subset` machinery passes; the claim response additively echoes the granted sets. CLI: claim requests come from `config.consent_scopes`, envelopes stamp the response-granted set, `login` gains `--scopes` + an interactive plain-language prompt, `status` shows per-trace scopes (new additive field populated from the already-typed `TraceCommonsSubmissionRecord.consent_scopes`).

**Tech Stack:** Existing crates only. No new dependencies, no schema migrations (the grant table and columns exist and are written today).

## Global Constraints

- No new external dependencies. No schema migrations.
- Verify every task with `RUSTFLAGS="-D warnings" cargo check`/`test` for the touched crates — plain `cargo check` does not catch what CI catches. CI clippy: `cargo clippy --workspace --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. `cargo fmt -p <touched crates> -- --check` before every commit.
- Hash-only logging: scope enum labels and device_key_id may appear in logs; never key material, raw subjects, URLs with credentials, or trace content.
- Fail closed: missing/empty grant scope data falls back to today's hardcoded floor, never to something broader.
- Exact fallback floor (must stay byte-for-byte): consent `[DebuggingEvaluation, PublicAttribution]`, uses `[Debugging, Evaluation, AggregateAnalytics]` (`device_key_allowed_consent_scopes()` / `device_key_allowed_uses()`, trace_upload_claim_issuer.rs:2189-2202).
- `PublicAttribution` is always added to every device-path ceiling.
- Empty scope intersection → 403 `{"error":"consent scopes not permitted"}`. Unknown scope strings in requests already fail serde deserialization of `TraceUploadClaimRequest` with the existing 400 `"invalid upload claim request"` — this satisfies the spec's invalid-scope rejection; do NOT add a new parse path.
- No emojis. Commit style: short imperative subjects without prefixes.
- The `trace-commons-ingest.rs` test module lives in `trace_commons_ingest_internal/tests.rs` via `#[path]` — add ingest tests there, never inline.
- PG-only repo; `postgres.rs` changes get a PG-gated test following the existing `TRACE_COMMONS_PG_TEST_DATABASE_URL` convention (CI never runs PG tests; they must still compile under `-D warnings`).

## Key facts (single source of truth; file:line as of branch head `4722915`)

- Issuer state: `tenant_access_grant_db: Option<Arc<dyn Database>>` (config field trace_upload_claim_issuer.rs:112, state :465); reachable from the device paths (`&self`).
- Device paths: `issue_claim_for_device_key` :1432-1483, `issue_claim_for_device_jwt` :1485-1526. Both build `grant_principal_ref = principal_storage_ref("device:{tenant_id}:{device_key_id}")` (`principal_storage_ref` :2461 → `"principal_sha256:{hex sha256}"`) and call `issue_claim_for_authorized_actor` (:1556-1612) with `AuthorizedUploadClaimActor { actor, tenant_id, grant_principal_ref, allowed_consent_scopes, allowed_uses, policy_label }` (:622-629).
- `issue_claim_for_authorized_actor` step 1 is `enforce_subset(requested, actor.allowed_consent_scopes)` — it REJECTS requests exceeding the actor ceiling. Therefore the device paths must compute `granted = intersect(requested, ceiling)` BEFORE constructing the actor, pass `granted` as both the actor ceiling and the effective request, and let the rest of the machinery run unchanged.
- Grant fetch: `Database::list_active_trace_tenant_access_grants_for_principal(&self, tenant_id: &str, principal_ref: &str, now: DateTime<Utc>) -> Result<Vec<TraceTenantAccessGrantRecord>, DatabaseError>` (decl trace_corpus_storage.rs:1861; PG impl trace_corpus_pg.rs:1623 — filters active/unexpired/unrevoked in SQL).
- `TraceTenantAccessGrantRecord` (trace_corpus_storage.rs:921-941): `allowed_consent_scopes: Vec<String>`, `allowed_uses: Vec<String>` (raw storage strings), `role: TraceTenantAccessGrantRole` (`Contributor`…), `status: TraceTenantAccessGrantStatus` (`Active`…). Parse strings with the existing `parse_storage_grant_values::<T>` (issuer :2405).
- Existing intersection helper: `restrict_requested_allowlist` (:2420): grant list non-empty + requested empty → requested = grant values; else intersect; empty intersection → `forbidden`. Reuse its semantics; the new device-path helper mirrors them with the exact label `"consent scopes not permitted"`.
- `TraceUploadClaimResponse` (:571-577): `access_token, token_type, expires_at, expires_in` — no scope echo today. `UploadClaimClaims` (:631-654) carries `allowed_consent_scopes: Vec<ConsentScope>`, `allowed_uses: Vec<TraceAllowedUse>`.
- Enrollment grant writer: `db/postgres.rs` `enroll_instance_user` :1831 calls `upsert_onboarding_device_tenant_access_grant(&tx, &p.tenant_id, &p.device_key_id)` :3498, which HARDCODES `allowed_consent_scopes = ["debugging_evaluation","public_attribution"]`, `allowed_uses = ["debugging","evaluation","aggregate_analytics"]`. `InstanceUserProvision` (db/mod.rs:1210-1222) already carries `allowed_consent_scopes: serde_json::Value`, `allowed_uses: serde_json::Value` from the allowlist policy template — currently ignored by the writer. `principal_ref` written by `onboarding_device_principal_ref` (:3562) — same `principal_sha256:` format the issuer computes.
- Ingest status: `submission_status_handler` (bin/trace-commons-ingest.rs:11781-11831) → `submission_status_from_record` (:48545-48602). `TraceCommonsSubmissionRecord` already has `consent_scopes: Vec<ConsentScope>` (:60041). `TraceSubmissionStatusUpdate` (protocol trace_contribution.rs:3793-3808) has no scope field yet.
- CLI: `identity.rs` `build_signed_claim_request` hardcodes `"consent_scopes": ["debugging_evaluation"]` (:202) and `"allowed_uses": ["debugging","evaluation"]` (:203). `issuer_client.rs` `ClaimToken { access_token, expires_at }` (:20-23), parsed from private `ClaimTokenResponse` (:33). `envelope.rs` `build_raw_contribution` consent block hardcodes `scopes: vec![ConsentScope::DebuggingEvaluation]` (:262). `commands.rs` login writes `consent_scopes: vec!["debugging_evaluation"]` (:65); `status` table :397-429. `ContributorConfig.consent_scopes: Vec<String>`.
- E2E: `tests/e2e_enroll_and_submit.rs` `InMemoryEnrollDb` — `enroll_instance_user` (:637-651) ignores provision scopes; `list_active_trace_tenant_access_grants_for_principal` (:94-101) is `todo!("stub")`.
- Wire scope names (serde snake_case of `ConsentScope`): `debugging_evaluation`, `benchmark_only`, `ranking_training`, `model_training`, `public_attribution`. `TraceAllowedUse`: `debugging`, `evaluation`, `benchmark_generation`, `ranking_model_training`, `model_training`, `aggregate_analytics`.
- Scope→uses mapping (spec): `debugging_evaluation → [debugging, evaluation]`, `benchmark_only → [benchmark_generation]`, `ranking_training → [ranking_model_training]`, `model_training → [model_training]`, `public_attribution → []`; `aggregate_analytics` always requested.

---

### Task 1: Claim response echoes granted scopes

**Files:**
- Modify: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs` (:571-577 response struct; :1556-1612 `issue_claim_for_authorized_actor`)
- Test: same file's existing `#[cfg(test)]` module (test constants live there), or `trace_commons_ingest_internal/tests.rs` if router-level assertion is easier — prefer the issuer file's unit level.

**Interfaces:**
- Produces: `TraceUploadClaimResponse` gains
  ```rust
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub consent_scopes: Vec<ConsentScope>,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub allowed_uses: Vec<TraceAllowedUse>,
  ```
  populated in `issue_claim_for_authorized_actor` from the SAME final vectors written into `UploadClaimClaims` (after `enforce_subset` + `enforce_tenant_access_grants` mutations). Both device and workload paths get the echo for free.
- Note: the struct is `Serialize`-only today; if it lacks `Deserialize`, do not add it — tests can assert on `serde_json::Value`.

- [ ] **Step 1: Write the failing test.** In the issuer's test module, find an existing test that mints a claim through `issue_claim_for_authorized_actor` or the router (e.g. the workload-path test near the `"allowed_consent_scopes"` assertion at ~:2836) and add:

```rust
#[tokio::test]
async fn claim_response_echoes_granted_scopes() {
    // Reuse the existing minimal issuer-state/router setup from the nearest
    // passing claim-mint test in this module (copy its setup verbatim).
    // After minting, parse the response body as serde_json::Value and assert:
    // body["consent_scopes"] == the JWT's allowed_consent_scopes claim, and
    // body["allowed_uses"] == the JWT's allowed_uses claim.
    // Decode the JWT payload with base64 (split '.', STANDARD_NO_PAD decode
    // segment 1) — no signature verification needed for this assertion.
}
```

(The test body must be real code adapted from the neighboring test's setup — the comment block above states the required assertions, not placeholder logic. If no in-module claim-mint test exists, use the router `oneshot` pattern from `trace_commons_ingest_internal/tests.rs` `per_user_subjects_resolve_to_distinct_accounts`.)

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server claim_response_echoes -- --nocapture`
Expected: FAIL — `consent_scopes` missing from response JSON (or compile error on the new fields if asserted via struct).

- [ ] **Step 3: Implement.** Add the two fields to `TraceUploadClaimResponse`; in `issue_claim_for_authorized_actor`, after the claims struct is built, populate the response with `claims.allowed_consent_scopes.clone()` / `claims.allowed_uses.clone()` (clone BEFORE the claims struct is consumed by signing, or restructure to build the response vectors first).

- [ ] **Step 4: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server claim_response_echoes && RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
Expected: PASS; check clean.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/trace_upload_claim_issuer.rs
git commit -m "Echo granted consent scopes in upload-claim responses"
```

---

### Task 2: Device-path grant-backed scope ceiling

**Files:**
- Modify: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs` (:1432-1526 device paths; new helpers near :2189)
- Test: pure-helper unit tests in the issuer file's test module; router-level integration test in `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (extend the `PerUserTestDeviceKeyDb` stub pattern at :70425).

**Interfaces:**
- Produces (issuer-private):
  ```rust
  /// Ceiling for a device principal: parsed scopes from active contributor
  /// grants when a grant DB is configured and rows exist; otherwise the
  /// hardcoded pilot floor. PublicAttribution is always included.
  async fn resolve_device_scope_ceiling(
      &self,
      tenant_id: &str,
      grant_principal_ref: &str,
      now: DateTime<Utc>,
  ) -> Result<(Vec<ConsentScope>, Vec<TraceAllowedUse>), IssuerError>

  /// Spec intersection: empty requested -> full ceiling; else intersection
  /// preserving ceiling order; empty intersection -> Err (403 label
  /// "consent scopes not permitted" for scopes; "allowed uses not
  /// permitted" for uses).
  fn intersect_requested_with_ceiling<T: PartialEq + Copy>(
      requested: &[T],
      ceiling: &[T],
      empty_label: &'static str,
  ) -> Result<Vec<T>, IssuerError>
  ```
- Behavior contract consumed by Task 9 (e2e): with `tenant_access_grant_db = Some(db)` and an active `role=contributor` grant for `principal_storage_ref("device:{tenant}:{device_key_id}")` whose `allowed_consent_scopes` include `"model_training"`, a device-key claim request with `consent_scopes: ["debugging_evaluation","model_training"]` yields a claim (and response echo) containing both.

**Semantics (from the spec — implement exactly):**
1. `resolve_device_scope_ceiling`: if `self.tenant_access_grant_db` is `None` → hardcoded floor (`device_key_allowed_consent_scopes()` / `device_key_allowed_uses()`). Else call `list_active_trace_tenant_access_grants_for_principal(tenant_id, grant_principal_ref, now)`; DB error → `IssuerError::internal()`. Filter to `status == Active && role == Contributor` (the PG impl pre-filters status; filter again defensively). No matching rows → hardcoded floor. Else union the parsed scopes across matching grants using the existing `parse_storage_grant_values::<ConsentScope>` / `::<TraceAllowedUse>` (:2405); parse failure → `IssuerError::internal()` (fail closed, matches existing behavior). If a grant's `allowed_consent_scopes` is empty, treat that grant as granting the hardcoded floor (fail-closed default, mirrors `restrict_requested_allowlist`'s empty-grant semantics). Always push `ConsentScope::PublicAttribution` (dedup) before returning.
2. In BOTH `issue_claim_for_device_key` and `issue_claim_for_device_jwt`: compute the ceiling, then `granted_scopes = intersect_requested_with_ceiling(&request.consent_scopes, &ceiling_scopes, "consent scopes not permitted")?` and `granted_uses = intersect_requested_with_ceiling(&request.allowed_uses, &ceiling_uses, "allowed uses not permitted")?`, and construct the actor with `allowed_consent_scopes: granted_scopes, allowed_uses: granted_uses` (replacing the hardcoded calls at :1475-1476 and :1518-1519). Because the actor ceiling now EQUALS the effective grant, `enforce_subset` inside `issue_claim_for_authorized_actor` passes when the request is a subset — pass the request through unmodified; a request exceeding the ceiling was already clipped by the intersection, so also REPLACE the request's scope vectors with `granted_*` before calling `issue_claim_for_authorized_actor` (clone the request; it is owned at that point). Net effect: clip-to-ceiling, exactly as the spec's "broader request is clipped" test requires.
3. `enforce_tenant_access_grants` later in `issue_claim_for_authorized_actor` still runs when `require_tenant_access_grants = true`; grant∩grant is idempotent — do not modify it.

- [ ] **Step 1: Write failing unit tests** for `intersect_requested_with_ceiling` in the issuer test module:

```rust
#[test]
fn intersect_empty_request_grants_full_ceiling() {
    let ceiling = vec![ConsentScope::DebuggingEvaluation, ConsentScope::ModelTraining];
    let got = intersect_requested_with_ceiling(&[], &ceiling, "consent scopes not permitted").unwrap();
    assert_eq!(got, ceiling);
}

#[test]
fn intersect_clips_to_ceiling_and_rejects_empty() {
    let ceiling = vec![ConsentScope::DebuggingEvaluation, ConsentScope::PublicAttribution];
    let got = intersect_requested_with_ceiling(
        &[ConsentScope::ModelTraining, ConsentScope::DebuggingEvaluation],
        &ceiling,
        "consent scopes not permitted",
    )
    .unwrap();
    assert_eq!(got, vec![ConsentScope::DebuggingEvaluation]);
    let err = intersect_requested_with_ceiling(&[ConsentScope::ModelTraining], &ceiling, "consent scopes not permitted")
        .unwrap_err();
    // IssuerError renders {"error": label}; assert the label text.
    assert!(format!("{err:?}").contains("consent scopes not permitted"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server intersect_ -- --nocapture`
Expected: FAIL — helper does not exist.

- [ ] **Step 3: Implement the two helpers and wire both device paths** per Semantics above.

- [ ] **Step 4: Write the failing router-level test** in `trace_commons_ingest_internal/tests.rs`: copy the `per_user_subjects_resolve_to_distinct_accounts` stub-DB + issuer-router setup; extend the stub so `list_active_trace_tenant_access_grants_for_principal` returns one `TraceTenantAccessGrantRecord` (`role: Contributor`, `status: Active`, `allowed_consent_scopes: vec!["debugging_evaluation".into(), "model_training".into()]`, `allowed_uses: vec!["debugging".into(), "evaluation".into(), "model_training".into()]`, other fields minimal/None, timestamps `Utc::now()`), keyed so it only matches the expected `principal_ref` (assert the queried principal_ref equals `principal_sha256:` + hex sha256 of `device:{tenant}:{device_key_id}` — compute in the test with sha2). Set the issuer config's `tenant_access_grant_db` to the stub. Three assertions:
  1. Request `consent_scopes: ["debugging_evaluation","model_training"]` → 200; response `consent_scopes` contains both; JWT claim matches.
  2. Same setup, request `["benchmark_only"]` → 403 body `{"error":"consent scopes not permitted"}`.
  3. Regression pin: a second device key with NO grant row (stub returns empty vec) → 200 with exactly the hardcoded floor `["debugging_evaluation","public_attribution"]`.

- [ ] **Step 5: Run to green**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server device_grant -- --nocapture` (name the router test `device_key_claims_honor_grant_scope_ceiling`)
Expected: PASS all three assertions plus the Task-1 and pre-existing claim tests.

- [ ] **Step 6: Full server verification**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins && RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server/src/trace_upload_claim_issuer.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Derive device-key claim scopes from tenant access grants"
```

---

### Task 3: Enrollment grant writer honors the instance policy template

**Files:**
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (`upsert_onboarding_device_tenant_access_grant` :3498 and its call site in `enroll_instance_user` :1913)
- Test: pure-helper unit test in `postgres.rs`'s test module (or `db/mod.rs` if the helper lands there); PG-gated integration test following the existing `TRACE_COMMONS_PG_TEST_DATABASE_URL` convention in the PG store test file (`cargo test -p trace-commons-server --test trace_corpus_pg_store` — find the existing enrollment test there and extend it).

**Interfaces:**
- Produces:
  ```rust
  /// Normalize a policy-template scope array (serde_json::Value from
  /// InstanceUserProvision) into storage strings. Non-array, empty, or
  /// non-string-element values fall back to `defaults` (fail closed).
  fn normalize_provision_scope_values(value: &serde_json::Value, defaults: &[&str]) -> Vec<String>
  ```
  and `upsert_onboarding_device_tenant_access_grant` gains two parameters: `allowed_consent_scopes: &[String], allowed_uses: &[String]` (writer no longer hardcodes them). The call site passes `normalize_provision_scope_values(&p.allowed_consent_scopes, &["debugging_evaluation","public_attribution"])` and `normalize_provision_scope_values(&p.allowed_uses, &["debugging","evaluation","aggregate_analytics"])`.
- The `ON CONFLICT ... DO UPDATE` must also update `allowed_consent_scopes`/`allowed_uses` (so re-enrollment refreshes policy), preserving the existing `WHERE status <> 'revoked'` guard.

- [ ] **Step 1: Write failing unit tests** for the normalizer:

```rust
#[test]
fn provision_scopes_normalize_or_fall_back() {
    use serde_json::json;
    let d = ["debugging_evaluation", "public_attribution"];
    assert_eq!(
        normalize_provision_scope_values(&json!(["model_training", "debugging_evaluation"]), &d),
        vec!["model_training".to_string(), "debugging_evaluation".to_string()]
    );
    // Empty array, non-array, and mixed-type arrays all fall back.
    assert_eq!(normalize_provision_scope_values(&json!([]), &d), d.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(normalize_provision_scope_values(&json!("nope"), &d), d.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    assert_eq!(normalize_provision_scope_values(&json!([1, "x"]), &d), d.iter().map(|s| s.to_string()).collect::<Vec<_>>());
}
```

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server provision_scopes_normalize`
Expected: FAIL — function missing.

- [ ] **Step 3: Implement** the normalizer, the two new writer parameters (update the SQL VALUES and the `DO UPDATE SET` list), and the call site.

- [ ] **Step 4: Extend the PG-gated test.** In the PG store test file's existing enrollment coverage (search `enroll_instance_user`), after enrolling with an `InstanceUserProvision` whose `allowed_consent_scopes = json!(["debugging_evaluation","model_training"])`, read back via `list_active_trace_tenant_access_grants_for_principal` and assert the row's `allowed_consent_scopes` contains `"model_training"`. Keep the existing env-gated skip pattern verbatim.

- [ ] **Step 5: Verify**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server provision_scopes_normalize && RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
Expected: unit test PASS; everything compiles (PG test runs only where the env var is set — run it if `TRACE_COMMONS_PG_TEST_DATABASE_URL` is available locally, and say in the report whether it ran).

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/db/postgres.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Write instance policy scopes into onboarding device grants"
```

(Adjust the `git add` list to the files actually touched by the PG test edit.)

---

### Task 4: Submission-status scope visibility (server)

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (:3793-3808 `TraceSubmissionStatusUpdate`)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (`submission_status_from_record` :48545-48602)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (extend the existing submission-status test — search `submission_status`).

**Interfaces:**
- Produces: `TraceSubmissionStatusUpdate` gains
  ```rust
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  pub consent_scopes: Vec<ConsentScope>,
  ```
  populated in `submission_status_from_record` from `record.consent_scopes.clone()` (the field already exists on `TraceCommonsSubmissionRecord` at ingest.rs:60041). Task 8 (CLI status column) consumes this field name exactly.

- [ ] **Step 1: Write the failing test.** Extend the existing submission-status test in `trace_commons_ingest_internal/tests.rs`: after the existing status-readback assertion, add `assert_eq!(update.consent_scopes, expected_scopes_from_the_submitted_envelope)` (use whatever consent scopes that test's envelope carries — read the test's envelope construction and assert the same values).

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server submission_status -- --nocapture`
Expected: FAIL — field does not exist (compile error).

- [ ] **Step 3: Implement** the protocol field and the one-line populate.

- [ ] **Step 4: Run to green**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server submission_status && RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins && RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run`
Expected: PASS; the contributor crate still compiles against the changed protocol struct (serde-defaulted, so no breakage).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Expose consent scopes in submission status readback"
```

---

### Task 5: CLI claim requests from config + granted-set parsing

**Files:**
- Create: `crates/trace-commons-contributor/src/consent.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod consent;`)
- Modify: `crates/trace-commons-contributor/src/identity.rs` (:192-214 `build_signed_claim_request`)
- Modify: `crates/trace-commons-contributor/src/issuer_client.rs` (:20-33 `ClaimToken` / `ClaimTokenResponse`)

**Interfaces:**
- Produces in `consent.rs`:
  ```rust
  pub const VALID_SCOPES: [&str; 5] = ["debugging_evaluation", "benchmark_only", "ranking_training", "model_training", "public_attribution"];

  /// Validate a list of wire-name scopes. Unknown names error, listing the
  /// valid set. Result is deduped, preserves VALID_SCOPES order, and always
  /// includes "debugging_evaluation".
  pub fn validate_scopes(names: &[String]) -> anyhow::Result<Vec<String>>

  /// Spec mapping: scopes -> allowed uses (wire names), always including
  /// "aggregate_analytics", deduped, stable order.
  pub fn scopes_to_allowed_uses(scopes: &[String]) -> Vec<String>
  ```
- `build_signed_claim_request` replaces the hardcoded arrays: `"consent_scopes": cfg.consent_scopes` (validated via `validate_scopes` first — invalid config errors with a "re-run login" hint) and `"allowed_uses": scopes_to_allowed_uses(&cfg.consent_scopes)`.
- `ClaimToken` gains `pub consent_scopes: Vec<String>` and `pub allowed_uses: Vec<String>` (from serde-defaulted fields on `ClaimTokenResponse`); empty vec means "older issuer, no echo" — Task 6 defines the fallback.

- [ ] **Step 1: Write failing tests** in `consent.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_scopes_dedups_orders_and_always_includes_floor() {
        let got = validate_scopes(&["model_training".into(), "model_training".into()]).unwrap();
        assert_eq!(got, vec!["debugging_evaluation".to_string(), "model_training".to_string()]);
        let err = validate_scopes(&["training".into()]).unwrap_err().to_string();
        assert!(err.contains("training") && err.contains("model_training"));
    }

    #[test]
    fn scope_to_use_mapping_matches_spec() {
        let got = scopes_to_allowed_uses(&["debugging_evaluation".into(), "model_training".into()]);
        assert_eq!(got, vec!["debugging".to_string(), "evaluation".to_string(), "model_training".to_string(), "aggregate_analytics".to_string()]);
        let attribution_only = scopes_to_allowed_uses(&["public_attribution".into()]);
        assert_eq!(attribution_only, vec!["aggregate_analytics".to_string()]);
    }
}
```

And in `identity.rs` tests, extend `claim_request_signature_covers_exact_body_bytes`'s config with `consent_scopes: vec!["debugging_evaluation".into(), "model_training".into()]` and assert `parsed["consent_scopes"] == serde_json::json!(["debugging_evaluation","model_training"])` and `parsed["allowed_uses"]` includes `"model_training"` and `"aggregate_analytics"`.

And in `issuer_client.rs` tests, extend the stub in `mint_claim_sends_signed_body_verbatim_and_parses_token` to return `"consent_scopes": ["debugging_evaluation"], "allowed_uses": ["debugging"]` and assert `token.consent_scopes == vec!["debugging_evaluation"]`; add a second assertion path (same test is fine) that a response WITHOUT the fields yields empty vecs.

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor consent identity issuer_client`
Expected: FAIL — module/fields missing.

- [ ] **Step 3: Implement** `consent.rs`, the `identity.rs` change, and the `ClaimToken` fields.

- [ ] **Step 4: Run to green**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: full crate PASS (the e2e still passes: its config uses `debugging_evaluation` and the real issuer clips identically).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src
git commit -m "Send configured consent scopes in claim requests and parse granted sets"
```

---

### Task 6: CLI envelope stamping from the granted set + scope-refusal outcome

**Files:**
- Modify: `crates/trace-commons-contributor/src/envelope.rs` (:219+ `build_raw_contribution`, consent block :260-266)
- Modify: `crates/trace-commons-contributor/src/submit.rs` (per-session flow + `mint_claim` error mapping)

**Interfaces:**
- Consumes: `ClaimToken.consent_scopes` / `.allowed_uses` (Task 5), `consent::{validate_scopes, scopes_to_allowed_uses}`.
- Produces:
  ```rust
  // envelope.rs
  /// Parse wire names to typed scopes; unknown names are skipped (they were
  /// validated at login/claim time) — never panics.
  pub fn parse_scope_names(names: &[String]) -> Vec<ConsentScope>

  /// Same for allowed uses (wire names -> TraceAllowedUse, unknown skipped).
  pub fn parse_use_names(names: &[String]) -> Vec<TraceAllowedUse>

  /// Overwrite the envelope's consent metadata and trace card with the
  /// claim-granted set. Called after redaction, before size check/upload.
  pub fn apply_granted_scopes(envelope: &mut TraceContributionEnvelope, granted_scopes: &[ConsentScope], granted_uses: &[TraceAllowedUse])
  ```
- `build_raw_contribution` consent block becomes `scopes: parse_scope_names(&cfg.consent_scopes)` (falling back to `vec![ConsentScope::DebuggingEvaluation]` when empty) — the REQUESTED set; the granted set is applied later.
- `apply_granted_scopes` sets: `envelope.consent.scopes = granted_scopes.to_vec()`; `envelope.trace_card.allowed_uses = granted_uses.to_vec()`; `envelope.trace_card.consent_scope = ` the first granted scope that is not `PublicAttribution` (fallback `DebuggingEvaluation`).
- `submit.rs` flow change: keep the existing order (redact → size-check → mint → upload) but insert `apply_granted_scopes` between mint and upload — converting `token.consent_scopes`/`token.allowed_uses` (wire strings) via `parse_scope_names` / `parse_use_names` — then re-run `envelope_size_ok` (cheap; keeps the guard honest). Fallback: when `token.consent_scopes` is empty (older issuer), apply the requested set (`parse_scope_names(&cfg.consent_scopes)` and `parse_use_names(&scopes_to_allowed_uses(&cfg.consent_scopes))`).
- Scope refusal mapping: in the claim-mint error path, if the error string contains `"consent scopes not permitted"`, outcome is `Refused { reason_label: "scopes-not-permitted" }` (not `Failed`), and one hint line is printed: `hint: re-run login --scopes with a narrower selection`. Dry-run is unchanged (no mint, envelope keeps the requested set).

- [ ] **Step 1: Write failing tests.**

In `envelope.rs`:
```rust
#[test]
fn granted_scopes_overwrite_consent_and_trace_card() {
    let t = fixture_transcript();
    let cfg = test_config(); // consent_scopes: ["debugging_evaluation"]
    let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
    let redactor = trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default().unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut envelope = rt.block_on(redact_to_envelope(&redactor, raw)).unwrap();
    let scopes = vec![ConsentScope::DebuggingEvaluation, ConsentScope::ModelTraining];
    let uses = vec![TraceAllowedUse::Debugging, TraceAllowedUse::ModelTraining];
    apply_granted_scopes(&mut envelope, &scopes, &uses);
    assert_eq!(envelope.consent.scopes, scopes);
    assert_eq!(envelope.trace_card.allowed_uses, uses);
    assert_eq!(envelope.trace_card.consent_scope, ConsentScope::DebuggingEvaluation);
}
```
(Import `ConsentScope`/`TraceAllowedUse` from the protocol crate; use `#[tokio::test]` instead of a manual runtime if that matches the module's existing style.)

In `submit.rs` tests: change `stub_issuer()` to return `"consent_scopes": ["debugging_evaluation","model_training"], "allowed_uses": ["debugging","evaluation","model_training","aggregate_analytics"]`, set the test config's `consent_scopes` to `["debugging_evaluation","model_training"]`, and assert the received envelope's `consent.scopes` equals `["debugging_evaluation","model_training"]` (as JSON strings). Add a second stub issuer returning 403 `{"error":"consent scopes not permitted"}` and assert the outcome is `Refused { reason_label: "scopes-not-permitted" }` with zero ingest deliveries.

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor envelope submit`
Expected: FAIL — helpers missing.

- [ ] **Step 3: Implement** per the Interfaces block.

- [ ] **Step 4: Run to green**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: full crate PASS including e2e.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src
git commit -m "Stamp envelopes with claim-granted consent scopes"
```

---

### Task 7: Login consent — interactive prompt and --scopes flag

**Files:**
- Modify: `crates/trace-commons-contributor/src/consent.rs` (prompt helper)
- Modify: `crates/trace-commons-contributor/src/commands.rs` (login flow, :56-71 config write)
- Modify: `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs` (Login subcommand gains `--scopes`)
- Modify: `crates/trace-commons-contributor/README.md` (consent-model section)

**Interfaces:**
- Produces in `consent.rs`:
  ```rust
  #[derive(Debug, Clone, Copy, Default)]
  pub struct ConsentAnswers { pub benchmark: bool, pub ranking: bool, pub training: bool, pub attribution: bool }

  /// Pure mapping: answers -> validated wire-name scope list (always
  /// includes debugging_evaluation; all-false -> just the floor).
  pub fn scopes_from_answers(a: ConsentAnswers) -> Vec<String>

  /// Interactive prompt (called only when --scopes is absent AND
  /// std::io::stdin().is_terminal()). Prints the spec's plain-language menu,
  /// reads y/N per optional scope from the provided reader, returns answers.
  pub fn prompt_consent_answers(input: &mut impl std::io::BufRead, output: &mut impl std::io::Write) -> anyhow::Result<ConsentAnswers>
  ```
- `commands::login` gains a `scopes: Option<&str>` parameter (CSV): `Some(csv)` → split, trim, `validate_scopes`; `None` + stdin is a TTY → `prompt_consent_answers` over real stdin/stdout → `scopes_from_answers`; `None` + not a TTY → `vec!["debugging_evaluation"]`. The chosen set is written to `ContributorConfig.consent_scopes` and echoed in the login consent line (replace the current fixed sentence with one that lists the chosen scope names and states the redaction contract wording from Task 5f of the previous slice — keep the existing accurate NEAR AI phrasing).
- Prompt text (exact, from the spec):
  ```
  How may your submitted traces be used? (you can revoke submitted traces later)
    Debugging and evaluation                 [always on]
    Benchmark generation                     [y/N]
    Ranking-model training                   [y/N]
    Model training                           [y/N]
    Public attribution of your handle        [y/N]
  ```
  Each `[y/N]` line reads one input line; `y`/`Y`/`yes` → true, anything else → false.
- `--scopes` is a new optional arg on the bin's `Login` subcommand, threaded through like `--allowed-hosts` (which is already a 3rd param — login becomes `login(store, grant, allowed_hosts, scopes)`).
- README: replace the "v1 cap / debugging_evaluation only" consent-model section with: instance policy template is the ceiling; contributor picks scopes at login (interactive or `--scopes`); envelopes carry what the server granted; `status` shows per-trace scopes.

- [ ] **Step 1: Write failing tests** in `consent.rs`:

```rust
#[test]
fn answers_map_to_scopes() {
    assert_eq!(scopes_from_answers(ConsentAnswers::default()), vec!["debugging_evaluation".to_string()]);
    let all = ConsentAnswers { benchmark: true, ranking: true, training: true, attribution: true };
    assert_eq!(
        scopes_from_answers(all),
        vec![
            "debugging_evaluation".to_string(),
            "benchmark_only".to_string(),
            "ranking_training".to_string(),
            "model_training".to_string(),
            "public_attribution".to_string()
        ]
    );
}

#[test]
fn prompt_reads_yes_no_answers_without_tty() {
    let mut input = std::io::Cursor::new(b"y\nn\nyes\n\n".to_vec());
    let mut output = Vec::new();
    let a = prompt_consent_answers(&mut input, &mut output).unwrap();
    assert!(a.benchmark && !a.ranking && a.training && !a.attribution);
    let printed = String::from_utf8(output).unwrap();
    assert!(printed.contains("How may your submitted traces be used?"));
    assert!(printed.contains("Model training"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor consent`
Expected: FAIL — helpers missing.

- [ ] **Step 3: Implement** the helpers, login threading, bin flag, and README section. `is_terminal`: `use std::io::IsTerminal; std::io::stdin().is_terminal()`.

- [ ] **Step 4: Run to green + smoke**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor && cargo run -p trace-commons-contributor -- login --scopes debugging_evaluation,model_training 2>&1 | head -3`
Expected: tests PASS; smoke prints the device-key-id instructions (no grant given) without touching the scopes yet OR validates them — either ordering is fine as long as an invalid scope name in `--scopes` errors before any network call (assert that manually: `-- login --scopes bogus` exits nonzero naming valid scopes).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src crates/trace-commons-contributor/README.md
git commit -m "Add interactive consent prompt and scopes flag to login"
```

---

### Task 8: CLI status SCOPES column

**Files:**
- Modify: `crates/trace-commons-contributor/src/commands.rs` (`status` :397-429)

**Interfaces:**
- Consumes: `TraceSubmissionStatusUpdate.consent_scopes: Vec<ConsentScope>` (Task 4).
- Produces: status table columns become `["SUBMISSION", "STATUS", "SCOPES", "PENDING", "FINAL"]`; SCOPES cell renders wire names comma-joined via a pure helper `pub(crate) fn scopes_cell(scopes: &[ConsentScope]) -> String` (empty slice → `"-"`). Serialize each scope with `serde_json::to_value` and strip quotes, or a small match — either, but wire names exactly.

- [ ] **Step 1: Write the failing test** in `commands.rs` tests:

```rust
#[test]
fn scopes_cell_renders_wire_names() {
    use trace_commons_protocol::trace_contribution::ConsentScope;
    assert_eq!(scopes_cell(&[]), "-");
    assert_eq!(
        scopes_cell(&[ConsentScope::DebuggingEvaluation, ConsentScope::ModelTraining]),
        "debugging_evaluation,model_training"
    );
}
```

- [ ] **Step 2: Run to verify failure**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor scopes_cell`
Expected: FAIL.

- [ ] **Step 3: Implement** the helper and the column.

- [ ] **Step 4: Run to green**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: PASS (the submit.rs status stub returns no consent_scopes field → serde default → "-" cell; extend that stub to include `"consent_scopes": ["debugging_evaluation"]` and assert the update's field parses, if the existing status test asserts row content).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/commands.rs crates/trace-commons-contributor/src/submit.rs
git commit -m "Show per-trace consent scopes in status output"
```

---

### Task 9: E2E — training consent end-to-end

**Files:**
- Modify: `crates/trace-commons-contributor/tests/e2e_enroll_and_submit.rs`

**Interfaces:**
- Consumes: everything above; the e2e's allowlist JSON already carries a `policy_template` (added during the previous slice — read the current file).

**Changes:**
1. Allowlist `policy_template`: set `allowed_consent_scopes: ["debugging_evaluation","public_attribution","model_training"]`, `allowed_uses: ["debugging","evaluation","aggregate_analytics","model_training"]`.
2. `InMemoryEnrollDb`: store the provision's normalized scopes at `enroll_instance_user` time (a new `grants: Mutex<HashMap<(String, String), (Vec<String>, Vec<String>)>>` keyed by `(tenant_id, principal_ref)` where `principal_ref = format!("principal_sha256:{}", hex::encode(sha2::Sha256::digest(format!("device:{}:{}", tenant_id, device_key_id))))` — compute with the sha2 dev-dependency already in scope via the server crate; if sha2 isn't directly importable in the test, add nothing: reuse `trace_commons_protocol` — NO. Compute via `sha2` which IS a direct dependency of trace-commons-contributor). Implement `list_active_trace_tenant_access_grants_for_principal` to return one `TraceTenantAccessGrantRecord` built from the stored scopes (`role: Contributor`, `status: Active`, `issued_at: Utc::now() - 60s`, everything optional `None`/empty, `grant_id: Uuid::new_v4()`).
3. Issuer config: `tenant_access_grant_db: Some(db.clone())` (same in-memory DB).
4. Login: call `commands::login(&store, Some(&grant.encode()), None, Some("debugging_evaluation,model_training"))` (new 4th param).
5. Assertions added: the stub-received envelope's `consent.scopes` == `["debugging_evaluation","model_training"]`; the stub-received `trace_card.allowed_uses` includes `"model_training"`; extend the stub status route to echo `"consent_scopes": ["debugging_evaluation","model_training"]` and assert `submit::status` surfaces them.

- [ ] **Step 1: Make the changes; run to verify the NEW assertions fail first** (temporarily assert before wiring config/scopes if needed — acceptable to combine: run once with old config to see the envelope carry only `debugging_evaluation`, then wire and see both).

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --test e2e_enroll_and_submit -- --nocapture`

- [ ] **Step 2: Run the full crate + server suites**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor && RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
Expected: all PASS / compile clean.

- [ ] **Step 3: Commit**

```bash
git add crates/trace-commons-contributor/tests
git commit -m "Prove training consent end-to-end against the real issuer"
```

---

### Task 10: Docs and verification sweep

**Files:**
- Modify: `README.md` (root — the contributor CLI paragraph: consent is scope-based, not capped)
- Modify: `docs/trace-commons-roadmap.md` (mark/append the consent-broadening line referencing the spec)
- Modify: `docs/superpowers/specs/2026-07-08-consent-scope-broadening-design.md` (Status: Implemented)

**Steps:**

- [ ] **Step 1: Make the doc edits** (one short paragraph each; no emojis; reference `docs/superpowers/specs/2026-07-08-consent-scope-broadening-design.md`).

- [ ] **Step 2: Full verification sweep** (all must pass; paste outputs in the report):

```bash
cargo fmt -p trace-commons-contributor -p trace-commons-protocol -- --check
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server claim_response_echoes device_key_claims submission_status provision_scopes_normalize
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy --workspace --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
```

- [ ] **Step 3: Commit**

```bash
git add README.md docs/
git commit -m "Document scope-based consent and complete verification sweep"
```
