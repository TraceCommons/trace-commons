# Login-with-NEAR (Slice 3a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add login-with-NEAR (NEP-413 account-bound authenticator) and a strong-authenticator gate to the contributor-account system, mirroring the Slice 2 passkey architecture.

**Architecture:** A NEAR identity is structurally a passkey — a stored credential (`trace_near_identities`) authenticating to an existing account, bootstrapping tenant via the same narrow `trace_login_resolver` role. Enroll does the one NEAR RPC call (full-access-key binding check); login verifies the NEP-413 signature offline. A strong-authenticator gate (with a bootstrapping carve-out) protects adding/removing any authenticator.

**Tech Stack:** Rust, axum 0.8, PostgreSQL (forced RLS), in-tree `ring` (Ed25519) / `sha2` / `base64` / `reqwest` / `serde_json`. **`borsh` 1.x and `bs58` 0.5 are APPROVED** for NEP-413 encoding: use `borsh` to serialize the NEP-413 payload byte-exactly (NEAR's canonical format — guarantees our bytes match what the wallet signed) and `bs58` to decode `ed25519:<base58>` keys. The actual Ed25519 verify stays on in-tree `ring`; do NOT add a full near-sdk/near-crypto crate. Spec: `docs/superpowers/specs/2026-06-23-contributor-account-near-login-slice3a-design.md`.

**Branch/worktree:** Execute on `contributor-account-slice3a` (stacked on `contributor-account-slice2`). Relative paths only; after each commit verify the main checkout (`/Users/zakimanian/code/trace-commons-server`) shows ONLY the pre-existing `community/*` + `AGENTS.md` (no leakage). Stand up a throwaway PostgreSQL to actually RUN the DB tests (prior slices did). DB ingest suite runs `--test-threads=1`.

**Verification gates (before every Rust commit):**
```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

---

## File Structure

| File | Responsibility | Create/Modify |
|---|---|---|
| `migrations/V33__near_identities.sql` | `trace_near_identities` table + RLS + resolver grant/policy; widen `client_kind` to add `'near'` | Create |
| `crates/trace-commons-server/src/db/postgres.rs` | register table in `TRACE_COMMONS_RLS_TABLES` (`:58`); wire V33 into `run_migrations` (`:902` pattern + coverage arrays); `resolve_near_public_key_tenant` (mirror `resolve_credential_tenant` `:242`); NEAR DB ops; `issue_near_session`; surface `client_kind` from `validate_session` (`:1951`) | Modify |
| `crates/trace-commons-server/src/db/mod.rs` | NEAR `Database` trait methods; `ValidatedSession` (`:2054`) gains `client_kind`; new row structs | Modify |
| `crates/trace-commons-server/src/config.rs` | `NearConfig` (rpc_url, network, recipient) + env loaders (mirror `WebauthnConfig` `:103`) | Modify |
| `crates/trace-commons-server/src/account_near.rs` | NEP-413 verification (hand-rolled base58 + borsh payload + `ring` Ed25519), the NEAR RPC `view_access_key_list` client + full-access check | Create |
| `crates/trace-commons-server/src/account_passkey.rs` | `CeremonyState` (`:87`) gains a `NearLogin`/`NearEnroll` challenge variant; (NEAR challenge = `{nonce:[u8;32], recipient:String}`) | Modify |
| `crates/trace-commons-server/src/account_session.rs` | `AccountCtx` (`:98`) gains session-strength (`client_kind`/`is_strong`) | Modify |
| `crates/trace-commons-server/src/lib.rs` | `pub mod account_near;` | Modify |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` | NEAR enroll/login/management handlers; routes (enroll+mgmt behind the `authenticated_account_routes` middleware `:5867`, login on the main router `:5974`); the strong-auth gate helper + apply to NEAR enroll + passkey enroll/remove | Modify |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` | NEP-413 unit vectors + ceremony/login/gate/management DB tests | Modify |
| `docs/operator/login-resolver-role.md`, `docs/operator/deployment.md` | resolver now reads `trace_near_identities`; the new `TRACE_COMMONS_NEAR_*` env vars | Modify |

**Design boundary:** NEP-413 crypto + RPC client live in `account_near.rs` (lib). DB work goes through `Database`/`PgBackend`. Thin axum handlers in `trace-commons-ingest.rs` beside the Slice 2 passkey handlers.

---

## PHASE 1 — Schema, resolver, config, NEP-413 crypto

### Task 1: V33 migration + RLS registration

**Files:** Create `migrations/V33__near_identities.sql`; modify `db/postgres.rs`; test in `tests.rs`.

Mirror V32 exactly (read `migrations/V32__webauthn_credentials.sql` for the table+RLS+resolver-policy shape; `trace_tenants.tenant_id` is TEXT).

- [ ] **Step 1 — failing test** (DB-backed, mirror V32's `webauthn_migration_applies_...`): assert `trace_near_identities` has `relforcerowsecurity=true`; assert `client_kind` CHECK now contains `'near'`; assert the new table is in `TRACE_COMMONS_RLS_TABLES` coverage (the existing `trace_commons_rls_registry_matches_migration_policy_coverage` lib test will enforce it). Self-skips without DB.
- [ ] **Step 2 — run → FAIL.**
- [ ] **Step 3 — write the migration** `migrations/V33__near_identities.sql`:
  - `trace_near_identities(tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE, public_key TEXT NOT NULL, near_account_id TEXT NOT NULL, account_id UUID NOT NULL, label TEXT, created_at TIMESTAMPTZ NOT NULL DEFAULT now(), last_used_at TIMESTAMPTZ, revoked_at TIMESTAMPTZ, PRIMARY KEY (tenant_id, public_key), FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE, UNIQUE (public_key))` + `CREATE INDEX ... ON trace_near_identities (tenant_id, account_id) WHERE revoked_at IS NULL` + ENABLE + FORCE RLS + the `trace_corpus_tenant_isolation` policy (copy V32's exact block).
  - Widen client_kind: `ALTER TABLE trace_sessions DROP CONSTRAINT trace_sessions_client_kind_check, ADD CONSTRAINT trace_sessions_client_kind_check CHECK (client_kind IN ('web','device','passkey','near'));`
  - Resolver extension (mirror V32 lines 79-83):
    ```sql
    GRANT SELECT (tenant_id, public_key) ON trace_near_identities TO trace_login_resolver;
    DROP POLICY IF EXISTS trace_login_resolver_near_read ON trace_near_identities;
    CREATE POLICY trace_login_resolver_near_read ON trace_near_identities
        FOR SELECT TO trace_login_resolver
        USING (true);
    ```
  - `postgres.rs`: register `"trace_near_identities"` in `TRACE_COMMONS_RLS_TABLES` (`:58`); wire V33 into `run_migrations` (copy the V32 block `:902`, version 33, name `"near_identities"`); add V33 + the table to BOTH coverage-test arrays.
- [ ] **Step 4 — run → PASS** (the registry coverage lib test must pass).
- [ ] **Step 5 — gates + commit** `Add V33 trace_near_identities table and resolver grant`.

### Task 2: `resolve_near_public_key_tenant` + SET ROLE regression test

**Files:** `db/postgres.rs`, `tests.rs`. Mirror `resolve_credential_tenant` (`:242`) + the Slice 2 `credential_resolver_reads_tenant_across_rls_under_set_role` test.

- [ ] **Step 1 — failing test** (DB-backed, the load-bearing one): insert a `trace_near_identities` row; `SET ROLE trace_login_resolver; SELECT tenant_id FROM trace_near_identities WHERE public_key=$1;` → returns the tenant; an out-of-grant column read (e.g. `near_account_id`) rejected. Verify fail-without/pass-with (drop the policy → fail; re-add → pass). Report both.
- [ ] **Step 2 — run → FAIL.**
- [ ] **Step 3 — implement** `pub async fn resolve_near_public_key_tenant(&self, public_key: &str) -> anyhow::Result<Option<String>>` identical-shape to `resolve_credential_tenant` (narrow `login_resolver_pool`, fail-closed `missing-control` when None, `SELECT tenant_id FROM trace_near_identities WHERE public_key = $1`, same safety comment). Add it to the `Database` trait (default not-implemented Err) + impl, like Slice 2 did for `resolve_credential_tenant`.
- [ ] **Step 4 — run → PASS** (report fail-without/pass-with).
- [ ] **Step 5 — gates + commit** `Add NEAR public-key->tenant resolver under role-scoped RLS policy`.

### Task 3: NEAR config + `account_near.rs` scaffolding

**Files:** `config.rs`, `account_near.rs` (create), `lib.rs`, `trace-commons-ingest.rs` (AppState), tests inline.

- [ ] **Step 1 — failing test** (unit): `NearConfig::from_env` requires all of `TRACE_COMMONS_NEAR_RPC_URL`/`_NETWORK`/`_LOGIN_RECIPIENT` together (partial → None, fail-closed, with a `tracing::warn!` naming which are set/unset — mirror the Slice 2 `WebauthnConfig` partial-config warning); an `account_near` accessor on AppState fails closed (a safe error) when None.
- [ ] **Step 2 — run → FAIL.**
- [ ] **Step 3 — implement:** `NearConfig { rpc_url, network, recipient }` + `from_env` (mirror `WebauthnConfig` `config.rs:103-150` incl. the partial-config warning). `account_near.rs` module skeleton + `pub mod account_near;` in lib.rs. Store `Option<NearConfig>` (or the derived pieces) on AppState; an `account_near_config(state) -> ApiResult<&NearConfig>` accessor mirroring `account_webauthn` (fail-closed). Reuse the Slice 2 `CeremonyStore` for NEAR challenges — add a `CeremonyState::NearChallenge { nonce: [u8;32], recipient: String }` variant (`account_passkey.rs:87`).
- [ ] **Step 4 — run → PASS.**
- [ ] **Step 5 — gates + commit** `Add NEAR config and account_near scaffolding`.

### Task 4: NEP-413 verification (borsh + bs58 + ring) + RPC client

**Files:** `account_near.rs`, `Cargo.toml` (add `borsh = { version = "1", features = ["derive"] }` and `bs58 = "0.5"`), tests inline + in tests.rs. **This is the crypto-critical task — still pin the byte layout with a hardcoded vector to catch any version/feature surprise.** Use `borsh` for the payload serialization (NEAR's canonical format) and `bs58` for key decode; `ring` for the Ed25519 verify.

- [ ] **Step 1 — failing tests** (unit, no DB):
  - `parse_near_ed25519_pubkey`: decode a real `ed25519:<base58>` NEAR key → the 32 expected bytes (via `bs58`); a malformed key (non-base58 char, wrong decoded length) → Err.
  - `nep413_payload_bytes`: for a FIXED input `{message:"hi", nonce:[1u8;32], recipient:"app.example", callbackUrl:None}`, assert the `borsh::to_vec` bytes equal a hardcoded expected `Vec<u8>` (u32 LE tag `2_147_484_061` = `[0x1d,0x00,0x00,0x80]`; String = u32 LE len + UTF-8; `[u8;32]` raw; Option None = `[0x00]`). ALSO a `callbackUrl:Some("u")` case (Option Some = `[0x01]` + String). This pins the format even though borsh produces it.
  - `verify_nep413`: build a payload, sha256 it, sign the digest with a `ring::signature::Ed25519KeyPair` test key, and assert `verify_nep413(public_key, {message,nonce,recipient,callbackUrl}, signature)` returns Ok; a tampered message/nonce/recipient → Err; a tag-mismatch (verify the function reconstructs with the exact tag) → Err.
- [ ] **Step 2 — run → FAIL.**
- [ ] **Step 3 — implement** in `account_near.rs`:
  - `fn parse_near_ed25519_pubkey(s: &str) -> Result<[u8;32]>` — strip `ed25519:` prefix, `bs58::decode(...).into_vec()`, require exactly 32 bytes; reject malformed.
  - `fn nep413_payload_bytes(message, nonce: &[u8;32], recipient, callback_url: Option<&str>) -> Vec<u8>` — a `#[derive(BorshSerialize)]` struct `Nep413Payload { tag: u32, message: String, nonce: [u8;32], recipient: String, callback_url: Option<String> }` with `tag = 2_147_484_061`, serialized via `borsh::to_vec(&payload)`. (Borsh emits u32 LE tag, then u32 LE length-prefixed UTF-8 strings, raw [u8;32], and the 1-byte Option tag — matching NEAR wallets.) Still assert the bytes for a fixed input equal a hardcoded expected `Vec<u8>` so a borsh major/feature change can't silently shift the layout.
  - `fn verify_nep413(public_key_b58: &str, message, nonce, recipient, callback_url, signature: &[u8]) -> Result<()>` — parse key, build payload, `sha2::Sha256` digest, `ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key).verify(&digest, signature)` (mirror the lib module `crates/trace-commons-server/src/trace_upload_claim_issuer.rs:1974` — the underscore lib file, NOT the hyphenated binary). Decode the wallet signature (base64 — confirm the wallet returns base64; NEP-413/wallet-selector returns base64 signature) before verify.
  - The NEAR RPC client: `async fn near_account_has_full_access_key(cfg: &NearConfig, account_id, public_key_b58) -> Result<bool>` — POST to `cfg.rpc_url` a JSON-RPC `{method:"query", params:{request_type:"view_access_key_list", finality:"final", account_id}}` via the in-tree `reqwest`; parse `keys[]`, return true iff `public_key` is present with `access_key.permission == "FullAccess"`. Make it TESTABLE: take the RPC response JSON as an injectable input in a pure helper `fn key_list_has_full_access(json, public_key) -> bool` (unit-tested without network), and the network call thin around it. Any RPC/parse error → the caller fails closed.
- [ ] **Step 4 — run → PASS** (incl. the hardcoded-byte-layout assertions).
- [ ] **Step 5 — gates + commit** `Add NEP-413 verification and NEAR access-key RPC check`.

---

## PHASE 2 — Ceremonies

### Task 5: NEAR identity DB ops + count-active-strong

**Files:** `db/mod.rs` (trait), `db/postgres.rs` (impl), `tests.rs` (DB-backed).

- [ ] **Step 1 — failing tests** (DB, real PG): insert→load_for_login round-trips; list returns active; rename; revoke soft-deletes + remaining; cross-account isolation (A's identity not listed/renamable/removable by B — two accounts in the SAME tenant); `count_active_strong_authenticators(account)` = active passkeys + active NEAR identities (enroll one of each, assert 2; revoke one, assert 1).
- [ ] **Step 2 — run → FAIL.**
- [ ] **Step 3 — implement** as `Database` methods (tenant-tx, sibling pattern, account-scoped where applicable; mirror the Slice 2 credential ops `postgres.rs:2312-2411`):
  - `insert_near_identity(tenant, account_id, public_key, near_account_id, label)`
  - `load_near_identity_for_login(tenant, public_key) -> Option<{account_id, near_account_id}>` (RLS-scoped; `revoked_at IS NULL`; NO `ensure_trace_tenant` — login path, runs under resolved tenant)
  - `touch_near_identity_last_used(tenant, public_key)`
  - `list_account_near_identities(tenant, account_id) -> Vec<{public_key, near_account_id, label, created_at, last_used_at}>`
  - `rename_account_near_identity(tenant, account_id, public_key, label) -> bool`
  - `revoke_account_near_identity(tenant, account_id, public_key) -> {removed, remaining_strong}` (remaining_strong = count active passkeys + active near after revoke)
  - `count_active_strong_authenticators(tenant, account_id) -> i64` (`SELECT (SELECT count(*) FROM trace_webauthn_credentials WHERE ... revoked_at IS NULL) + (SELECT count(*) FROM trace_near_identities WHERE ... revoked_at IS NULL)` both account+tenant scoped).
- [ ] **Step 4 — run → PASS.**
- [ ] **Step 5 — gates + commit** `Add NEAR identity DB operations and strong-authenticator count`.

### Task 6: NEAR enroll ceremony (authenticated; RPC binding check)

**Files:** `trace-commons-ingest.rs` (2 handlers + routes), `tests.rs`. (The strong-auth GATE is added in Task 8; here, enroll is guarded by the middleware `Extension<AccountCtx>` only — note that Task 8 tightens it.)

- [ ] **Step 1 — failing tests** (DB, real PG; use a `ring` Ed25519 test keypair as the "wallet" + a mocked `view_access_key_list` response): `enroll/start` (authenticated) returns `{nonce, recipient}` + sets the ceremony cookie + stashes the `NearChallenge`; `enroll/finish` with a valid NEP-413 signature over the challenge AND a mocked RPC response containing the key as FullAccess → a `trace_near_identities` row exists with the right public_key + near_account_id + audit; `finish` with the key ABSENT or function-call-only in the RPC response → reject, no row; `finish` with a bad signature → reject; missing/expired ceremony → 400.
- [ ] **Step 2 — run → FAIL.**
- [ ] **Step 3 — implement** (behind the auth middleware → `Extension<AccountCtx>`):
  - `POST /v1/account/near/enroll/start`: `account_near_config(state)?`; generate `nonce`; stash `CeremonyState::NearChallenge{nonce, recipient}`; set the ceremony cookie (reuse the passkey ceremony cookie name/shape or a `tc_near_ceremony`); return `{nonce (b64/hex), recipient}`.
  - `POST /v1/account/near/enroll/finish`: take the ceremony state (missing/expired/wrong-variant → 400); parse `{accountId, publicKey, signature}`; `verify_nep413(...)` against the stashed nonce/recipient (fail → 400); `near_account_has_full_access_key(cfg, accountId, publicKey)` (false/Err → 400/fail-closed — the binding check); `insert_near_identity(...)`; audit `account_near_enrolled` (hash-only). Return 200 `{near_account_id, public_key}` (public identifiers).
  - To make the RPC mockable in tests: inject the RPC via a trait/closure on AppState, or gate the network call behind a function the test can stub (e.g. an `Option<MockNearRpc>` test seam). Document the approach.
  - Routes: add to `authenticated_account_routes` (`:5867`).
- [ ] **Step 4 — run → PASS.**
- [ ] **Step 5 — gates + commit** `Add NEAR enroll ceremony with access-key binding check`.

### Task 7: NEAR discoverable login + tenant bootstrap + session

**Files:** `trace-commons-ingest.rs` (2 handlers + routes), `db/*` (`issue_near_session`), `tests.rs`. UNAUTHENTICATED — full redeem-style hardening mirroring `account_passkey_login_*` (`:13547`/`:13618`).

- [ ] **Step 1 — failing tests** (DB, real PG, `ring` test wallet): enroll a NEAR identity (Task 6 path), then `login/start` → `login/finish` with a valid NEP-413 assertion → a `tc_account_session` cookie (`client_kind='near'`), 303. Security: cross-account (a NEAR identity under A's tenant only mints A's session); forged/unknown public key → uniform deny + **no `trace_tenants` write**; unknown vs missing-ceremony vs malformed-body → byte-identical deny; a stale/replayed nonce → deny.
- [ ] **Step 2 — run → FAIL.**
- [ ] **Step 3 — implement** (mirror passkey login exactly):
  - `POST /account/near/login/start`: rate-limit (per-IP + global, keys `near-login-ip:`/`near-login-global`); `nonce`+`recipient`; stash; return. Failure → uniform deny.
  - `POST /account/near/login/finish`: inner/outer timing-floor split; rate-limit (per-IP + global + per-public-key ceiling `near-login-key:{pubkey}`); a custom `NearAssertionBody` extractor → uniform deny on malformed; take ceremony state; `verify_nep413(...)` against the stashed challenge (no RPC); `resolve_near_public_key_tenant(publicKey)` (None/Err → uniform deny); under the resolved tenant load `load_near_identity_for_login` (None/revoked → uniform deny); `touch_near_identity_last_used`; `issue_near_session(tenant, account_id, NewSession{token_hash, client_kind:"near", expires_at, auth_credential_id: Some(public_key)})` (add this method, mirror `issue_passkey_session` `:13768`) — **store the authenticating NEAR `public_key` in the session's `auth_credential_id` column** (the generic "authenticating-credential id"), so Task 9's `this_session` reuses the existing mechanism; build the identical Slice 1 session cookie + 303 + no-store/no-referrer. Audit `account_near_login`. **NO `ensure_trace_tenant` before verification.** One uniform deny for every failure behind the floor.
  - Routes: add to the main router (`:5974`), NOT behind the middleware.
- [ ] **Step 4 — run → PASS** (verify the no-`trace_tenants`-write regression explicitly).
- [ ] **Step 5 — gates + commit** `Add discoverable NEAR login with tenant bootstrap`.

---

## PHASE 3 — Gate, management, sweep

### Task 8: the strong-authenticator gate (surface client_kind + apply to enroll/remove)

**Files:** `db/postgres.rs` (validate_session SELECT), `db/mod.rs` (ValidatedSession), `account_session.rs` (AccountCtx), `trace-commons-ingest.rs` (gate helper + apply), `tests.rs`.

- [ ] **Step 1 — failing tests** (DB, real PG): a **weak** (device-link, `client_kind='web'`) session is BLOCKED (403) from NEAR enroll AND passkey enroll WHEN the account already has ≥1 strong authenticator; the **carve-out** allows a weak session to add the FIRST authenticator (account with zero strong); a **strong** (passkey/near) session can always add; remove of an authenticator from a weak session is blocked when ≥1 strong exists but allowed at zero (i.e. removal returns to bootstrap); a strong session can remove. Assert the bearer (device) path is treated as weak.
- [ ] **Step 2 — run → FAIL.**
- [ ] **Step 3 — implement:**
  - Surface session strength: extend `validate_session`'s SELECT (`:1951`) to also read `client_kind`; add `client_kind: String` to `ValidatedSession` (`:2054`); `resolve_account_ctx_cookie` sets `AccountCtx.client_kind` (add the field, `account_session.rs:98`); the bearer path sets a weak marker (e.g. `client_kind="device"`). Add a helper `AccountCtx::is_strong_session(&self) -> bool` = `client_kind ∈ {"passkey","near"}`.
  - Gate helper `async fn require_authenticator_change_allowed(state, ctx) -> ApiResult<()>`: if `ctx.is_strong_session()` → Ok; else if `count_active_strong_authenticators(ctx.tenant, ctx.account) == 0` → Ok (bootstrap carve-out); else → `Err(api_error(StatusCode::FORBIDDEN, "a passkey or NEAR sign-in is required to change authenticators"))` + audit a gate-rejection event.
  - Apply it at the top of: NEAR enroll start+finish (Task 6 handlers), passkey register start+finish (`:13136`/`:13232` — the tightening), NEAR remove (Task 9), passkey remove (`:13435` — tighten). Do NOT gate the LIST/rename of identities (read/label only) — or gate rename too if you prefer; spec only requires gating add/remove. Note your choice.
- [ ] **Step 4 — run → PASS.**
- [ ] **Step 5 — gates + commit** `Add strong-authenticator gate with bootstrapping carve-out`.

### Task 9: NEAR management (list / rename / remove + this_session)

**Files:** `trace-commons-ingest.rs` (3 handlers + routes), `tests.rs`. Mirror passkey management (`:13336`/`:13384`/`:13435`).

- [ ] **Step 1 — failing tests** (DB, real PG): list returns the account's active NEAR identities with `this_session=true` for the identity that authed the current NEAR session (false for passkey/device sessions); rename; remove (gated — Task 8) soft-deletes + returns remaining_strong; cross-account isolation (B can't list/rename/remove A's — two accounts same tenant).
- [ ] **Step 2 — run → FAIL.**
- [ ] **Step 3 — implement** (guarded by the middleware `Extension<AccountCtx>`):
  - `GET /v1/account/near-identities` → `list_account_near_identities`; `this_session` by comparing each `public_key` against the current session's NEAR public key. (Surfacing the current NEAR session's public key: `auth_credential_id` is NULL for NEAR; either also surface the NEAR public key into the session row + AccountCtx like `auth_credential_id`, OR set `auth_credential_id` to the public_key for NEAR sessions and reuse it. Pick one — recommend storing the authenticating NEAR public_key in the session's `auth_credential_id` column at `issue_near_session` time and treating it as the generic "authenticating-credential id"; note the choice and keep `this_device`/`this_session` semantics consistent.)
  - `PATCH /v1/account/near-identities/{public_key}` → rename; unknown/not-owned → 404. (Gate per Task 8 choice.)
  - `DELETE /v1/account/near-identities/{public_key}` → `require_authenticator_change_allowed` then `revoke_account_near_identity`; 404 if not owned; return `{removed, remaining_strong_authenticators}`. Audit hash-only.
  - Routes: `authenticated_account_routes` (`:5867`). axum 0.8 `{public_key}` String param.
- [ ] **Step 4 — run → PASS.**
- [ ] **Step 5 — gates + commit** `Add NEAR identity management endpoints`.

### Task 10: regression sweep + operator docs + full gates

**Files:** `tests.rs`, `docs/operator/*`.

- [ ] **Step 1 — consolidate/verify the security regression suite** (real PG, all green): resolver SET-ROLE least-privilege on `trace_near_identities`; cross-account isolation (NEAR login + management); forged public key → uniform deny + no-`trace_tenants`-write; uniform-deny byte-identity; the strong-auth gate (block 2nd from weak; carve-out 1st; removal-to-bootstrap; passkey-enroll tightened); enroll RPC-fail → fail-closed (mocked); NEP-413 unit vectors (incl. `callbackUrl=None` byte layout); forced-RLS on the new table. Fill any gap.
- [ ] **Step 2 — operator docs:** `login-resolver-role.md` — the resolver role now also has `SELECT (tenant_id, public_key)` on `trace_near_identities` + the `trace_login_resolver_near_read` policy (same provisioning). `deployment.md` — the three `TRACE_COMMONS_NEAR_RPC_URL/_NETWORK/_LOGIN_RECIPIENT` vars (required together; partial → disabled + warning; pin a trusted RPC; RPC used only at enroll).
- [ ] **Step 3 — full sweep** (report each): the three gates above; `cargo test --test trace_corpus_storage_contract`; the account+passkey+near+session subset on real PG (`--test-threads=1`); `scripts/operator/pilot-bootstrap-smoke.sh`.
- [ ] **Step 4 — commit** `Finalize NEAR login slice: regression suite and operator docs`.

---

## Done criteria

All 10 tasks committed; full sweep green; the resolver `SET ROLE` test passes under the real role (fail-without/pass-with); NEP-413 byte layout pinned by hardcoded vectors incl. `callbackUrl=None`; cross-account, forged-key-no-write, uniform-deny, gate, and enroll-RPC-fail-closed regressions pass; NEAR login does no pre-verify `ensure_trace_tenant` and no RPC; only public on-chain identifiers (`public_key`/`near_account_id`) stored, never in audit/logs; the new table forced-RLS + registered + coverage-tested; `borsh` + `bs58` used for NEP-413 encoding (approved), Ed25519 verify on in-tree `ring`, no near-sdk/near-crypto crate; the NEP-413 byte layout still pinned by a hardcoded vector.

## Residual risks (carried, documented)

NEAR RPC trust at enroll; in-process challenge store / single-instance; NEP-413 recipient binding weaker than WebAuthn origin; `near_account_id` is a public on-chain identifier stored in an RLS row (kept out of audit/logs); DB suite needs `--test-threads=1`.
