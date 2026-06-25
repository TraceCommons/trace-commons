# Instance-Vouched Enrollment — Ironclaw Instance Onboarding (Design Spec)

Date: 2026-06-24
Status: Approved for planning

## Context

Today a contributor onboards by presenting an **operator-issued invite code**.
`POST /v1/onboard` (on the upload-claim issuer binary) carries
`{ invite_code, device_public_key, client_info }`; the issuer hashes the code,
looks it up in the file allowlist (`subject_hash -> tenant_id`, with a `max_uses`
cap), registers the device key under that tenant via
`Database::onboard_device_key`, and returns the tenant + ingest/issuer URLs. One
invite covers a handful of device registrations into a single tenant
(`trace_upload_claim_allowlist.rs`, `trace_upload_claim_issuer.rs::onboard`,
`db/postgres.rs::onboard_device_key`).

This model does not fit a **multitenant Ironclaw instance** that wants to bring
its **current and future users** onto the pilot. Pre-minting one invite per user
requires per-user operator action forever; a single shared high-`max_uses` invite
conflates every user under one undifferentiated principal and ships a reusable
bearer to every client.

This feature lets the operator authorize an Ironclaw **instance once**, after
which every user on that instance self-enrolls with **no per-user invite code**.
The instance acts as a vouching authority: it signs a short-lived enrollment
attestation that names the user and their device key, and the issuer verifies the
signature against the instance's registered public key, then provisions a
**dedicated Trace Commons tenant for that user** and registers the device key in
it.

The existing invite path is **left fully intact** — direct human contributors
keep using invite codes; instance-vouched enrollment is an additive sibling path.

## Dependencies and migration ordering

This slice **depends on contributor-account Slice 1** (the `trace_accounts` /
`trace_account_principals` tables and the mint create-or-reuse logic, defined in
that slice's `V30__trace_accounts.sql`). The enroll flow's account/principal step
reuses those tables and that logic verbatim. As of this branch, only migrations
through **V29** are merged and Slice 1 (V30) lives on a separate branch, so:

- This work must land **after** contributor-account Slice 1 is merged.
- The new control-plane migration is numbered **V31**, immediately after Slice 1's
  V30. If Slice 1's number shifts on merge, renumber this one to stay last.
- If, for scheduling reasons, this slice must ship before Slice 1, the
  account/principal creation in enroll step 7 is the only coupling — it can be
  temporarily dropped (device key registered without an account row) and the
  account back-filled when Slice 1 lands, since the device key is the principal
  and credit accrues per principal regardless. Prefer ordering after Slice 1.

## Resolved decisions (from brainstorming)

1. **Tenant model: one tenant per user, keyed by an instance-asserted user
   subject.** Each enrolling user gets a dedicated Trace Commons tenant. The
   tenant id is **derived, never client-asserted**:
   `tenant_id = "tenant-" + sha256(instance_id || user_subject)` where
   `user_subject` is a stable per-user identifier the instance vouches for (e.g.
   a hash of the Ironclaw user id). Multi-device falls out for free: the same
   user's second device derives the same `tenant_id`, so it joins the existing
   tenant as another principal — no cross-tenant consolidation needed. Credit
   rolls up per tenant = per user automatically. `tenant_id` is itself a hash and
   is therefore non-identifying (hash-only convention preserved).

   This was chosen over one-tenant-per-instance for two stated goals weighed
   equally: (a) a **hard Postgres-RLS + KEK-AAD isolation wall** between
   co-instance users (not relying on the account-visibility predicate), and (b)
   **durable user-keyed portability/settlement** — each user is a clean
   export/credit-settlement boundary that survives device re-enrollment. The
   accepted cost is that cross-user aggregation (leaderboards, instance rollups)
   must cross the RLS boundary via a privileged aggregate path (see Cross-tenant
   aggregation).

   Feasibility was verified against the code, correcting an earlier worry that
   per-user provisioning is a heavy fail-open surface:
   - `ensure_trace_tenant` (`db/trace_corpus_pg.rs:1191`) creates the
     `trace_tenants` row **under the new tenant's own RLS context** — it sets the
     GUC to the id being created, so `WITH CHECK (tenant_id =
     trace_current_tenant_id())` passes. No `BYPASSRLS`, idempotent, and the
     onboard path already calls it.
   - Object storage is a **derived prefix** (`sha256(tenant_storage_ref)`), not a
     provisioned bucket — free per tenant.
   - The KEK uses **one configured cloud KMS key** with per-tenant isolation via
     AAD (`aad = KekContext{tenant_storage_ref, artifact_kind}.canonical_hash()`,
     `trace_artifact_kek.rs:406`) — a new tenant gets a distinct AAD
     automatically; **no per-tenant KMS provisioning**.
   - Basic contribution needs no access-grant row — the role rides in the signed
     token (`role.unwrap_or("contributor")`, `bin/trace-commons-ingest.rs:8292`).
   The only genuine per-tenant provisioning input is the **contribution policy**
   (`trace_tenant_policies`), stamped from a template on the instance entry.

2. **Trust model: instance signs each enrollment (issuer model).** Operator
   registers the instance's Ed25519 **public key** once. The instance signs the
   per-enrollment attestation; the issuer verifies the signature, derives the
   tenant, provisions it, and registers the device key. No shared secret travels
   with users; each enrollment is independently fresh. Shared-bearer and
   operator-pre-authorized-batch models were rejected. TEE attestation of the
   instance is a later hardening that grafts onto the same derivation.

3. **Anti-Sybil: hard cap + per-instance rate limit.** The instance entry carries
   a `max_enrollments` ceiling that bounds distinct **users** (tenants), plus a
   per-instance token-bucket rate limit on the enroll endpoint. A new device for
   an already-enrolled user does **not** consume cap. Over either bound -> generic
   refusal + denial counter.

4. **Attestation freshness: self-asserted `exp` + `nonce`, server dedupes.**
   Single round trip. Server enforces `exp` (<=5 min), binds `aud` to this issuer
   and `instance_id` to the registered entry (no cross-instance / cross-issuer
   reuse), and records consumed nonces until `exp` to block replay. Replay is
   already idempotent on the derived `tenant_id` + unique `device_key_id`; the
   nonce cache additionally stops cap burn and re-add of a removed key.

5. **Provisioning timing: eager at enroll.** Tenant + policy + device key +
   account/principal are all created in **one** `begin_trace_tenant_transaction`
   on the derived tenant, plus one control-plane upsert into the enrollment
   ledger (below). The user has a fully functional tenant and account from the
   first enrollment.

## Repo invariants this design must honor

- PostgreSQL-only; forced RLS on every Trace Commons table via
  `trace_current_tenant_id()`. The per-user tenant and all its rows are created
  under the derived tenant's own RLS context (the same self-bootstrapping pattern
  `ensure_trace_tenant` already uses). The one new control-plane table
  (`trace_instance_enrollments`) honors forced RLS on a parallel **instance**
  predicate, not the tenant predicate.
- Hash-only / label-only audit and logs: never store or log the raw instance
  public key, `user_subject`, nonce, signature, device public key bytes, URLs, or
  contributor identity. `user_subject` is stored only as `sha256(user_subject)`;
  `tenant_id` is already a hash. Subjects and actors are hashes or operator
  labels.
- Fail-closed: any verification, rate, cap, replay, provisioning, or DB error ->
  one generic refusal with a safe label; never fall through to an unauthenticated
  or elevated state.
- Tenant scoping is auth-derived: the tenant is **computed** from the verified
  attestation, never selected by client input. Any `tenant_id` echoed in the
  attestation is checked for equality against the server's derivation, it does not
  choose the tenant.
- Match existing module/handler/migration patterns; do not split
  `trace-commons-ingest.rs` or the issuer module.

## Components

- **`crates/trace-commons-protocol/src/onboarding.rs`** — new request type and a
  canonical attestation-bytes encoder + a single `derive_user_tenant_id(instance_id,
  user_subject)` function (the one source of truth shared by signer and verifier,
  mirroring the `hash_invite_code` / `device_key_id_from_public_key_bytes` "one
  function, no drift" convention). New enroll-specific error-code variants.
- **`migrations/V31__trace_instance_enrollments.sql`** — the control-plane
  enrollment ledger (below), with forced RLS on an instance predicate and a
  resolver function `trace_current_instance_subject()` paralleling
  `trace_current_tenant_id()`. Hash-only columns, `sha256:`-shaped CHECKs.
- **`crates/trace-commons-server/src/trace_upload_claim_allowlist.rs`** — extend
  the allowlist file schema with a tagged entry **kind** (`invite` | `instance`).
  `instance` entries carry the Ed25519 `instance_public_key`, an `instance_id`,
  `max_enrollments`, optional `rate_per_min`, a **policy template**
  (`allowed_consent_scopes`, `allowed_uses`, `policy_version`), and `note_label`.
  The snapshot exposes an instance-entry lookup keyed by the instance subject hash
  and surfaces the raw `instance_public_key` bytes for signature verification.
  `AllowlistSource` trait, file caching, max-stale fail-closed, and the reserved
  `Near` source are unchanged.
- **`crates/trace-commons-server/src/trace_upload_claim_issuer.rs`** — new
  `enroll` method (sibling to `onboard`), the `POST /v1/enroll` route, attestation
  verification, per-instance rate limiter + replay cache, the cap check against
  the ledger, and the provisioning call. Reuses `denial_counter`,
  `onboard_allowlist_snapshot`, response construction, and env-config plumbing.
- **`crates/trace-commons-server/src/db/`** — one `enroll_instance_user` op that,
  in a single derived-tenant transaction, (a) `ensure_trace_tenant`, (b)
  create-or-reuse the `trace_tenant_policies` row from the template, (c) register
  the device key, (d) create-or-reuse the account + bind the principal (Slice-1
  logic); plus a separate instance-scoped op that upserts the ledger row and
  returns the current user count for the cap check.

## Enrollment ledger (V31 control plane)

The cap and the instance->users rollup both need visibility **across** per-user
tenants, which the tenant RLS predicate deliberately blocks. A dedicated
control-plane table sits above the per-user tenants:

```sql
CREATE TABLE trace_instance_enrollments (
  instance_subject_hash TEXT NOT NULL
    CHECK (instance_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
  user_subject_hash     TEXT NOT NULL
    CHECK (user_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
  tenant_id             TEXT NOT NULL,        -- derived per-user tenant (a hash)
  created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (instance_subject_hash, user_subject_hash)
);
CREATE INDEX idx_trace_instance_enrollments_instance
  ON trace_instance_enrollments (instance_subject_hash);

ALTER TABLE trace_instance_enrollments ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_instance_enrollments FORCE ROW LEVEL SECURITY;
CREATE OR REPLACE FUNCTION trace_current_instance_subject()
RETURNS TEXT LANGUAGE SQL STABLE AS $$
  SELECT NULLIF(current_setting('trace_commons.instance_subject', true), '');
$$;
DROP POLICY IF EXISTS trace_instance_isolation ON trace_instance_enrollments;
CREATE POLICY trace_instance_isolation ON trace_instance_enrollments
  USING      (instance_subject_hash = trace_current_instance_subject())
  WITH CHECK (instance_subject_hash = trace_current_instance_subject());
```

Triple duty:
- **Dedup:** `PRIMARY KEY (instance_subject_hash, user_subject_hash)` makes
  user_subject -> tenant a stable, idempotent mapping. (Correctness of the mapping
  also falls out of deterministic derivation + idempotent `ensure_trace_tenant`;
  the ledger is the authoritative record and the cap/rollup index.)
- **Cap:** `COUNT(*)` for an `instance_subject_hash` = its user count. A new
  device for an existing `user_subject_hash` hits the existing PK row and does not
  grow the count.
- **Rollup index:** `instance_subject_hash -> [tenant_id]` for leaderboards /
  instance-level reporting (read under the instance predicate, then aggregate the
  listed tenants via the privileged path).

RLS is on a **parallel instance dimension** so the "forced RLS on every table"
invariant holds without contorting it onto the tenant predicate. The issuer sets
`trace_commons.instance_subject` transaction-locally (mirroring
`begin_trace_tenant_transaction`) before touching the ledger; an instance can only
ever see its own enrollment rows. No raw `user_subject`, only its hash.

## Allowlist schema extension

The file gains a tagged kind so the two subject types parse side by side. Existing
invite files remain valid (kind defaults to `invite`). An `instance` entry no
longer maps to a single tenant — it authorizes the instance to mint many per-user
tenants and carries the policy template stamped onto each:

```json
{
  "version": 1,
  "generated_at": "2026-06-24T00:00:00Z",
  "policy_label": "pilot",
  "entries": [
    { "kind": "invite",
      "subject_hash": "sha256:<64 lower-hex>",
      "tenant_id": "tenant-zaki-pilot",
      "note_label": "closed-alpha-batch-1",
      "max_uses": 3 },

    { "kind": "instance",
      "instance_id": "ironclaw-acme-prod",
      "instance_public_key": "<base64 ed25519 32-byte>",
      "max_enrollments": 5000,
      "rate_per_min": 60,
      "policy_template": {
        "policy_version": "ironclaw-pilot-v1",
        "allowed_consent_scopes": ["pilot_research"],
        "allowed_uses": ["model_training", "evaluation"]
      },
      "note_label": "ironclaw-acme-prod" }
  ]
}
```

Validation rules for `instance` entries:

- `instance_public_key` decodes to exactly 32 bytes; reject the whole snapshot
  otherwise (same strict posture as invite `subject_hash`).
- `instance_id` non-empty; it is the namespace component of the tenant
  derivation, so it must be stable for the life of the instance.
- `max_enrollments >= 1`.
- `rate_per_min` optional; falls back to an env default
  (`TRACE_COMMONS_INSTANCE_ENROLL_RATE_PER_MIN`).
- `policy_template` required: `policy_version` non-empty;
  `allowed_consent_scopes` / `allowed_uses` are JSON arrays validated against the
  same vocabulary the operator policy-write path already enforces.
- The **instance subject hash** = `hash_instance_subject(pubkey_bytes)` =
  `sha256("instance:" || pubkey_bytes)`, namespaced exactly as `hash_invite_code`
  uses `"invite:"`, so an instance pubkey can never collide with an invite-code
  subject hash. This is the single hashing function used for ledger keys, denial
  accounting, and audit actor labels.

## Wire protocol

New protocol types in `onboarding.rs`. The invite `TraceOnboardRequest` /
`TraceOnboardResponse` are unchanged; the response type is **reused** by enroll.

```rust
pub const TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION: &str =
    "trace_commons.instance_enroll_request.v1";

pub struct TraceInstanceEnrollAttestation {
    pub device_key_id: String,   // sha256: of the device pubkey bytes
    pub aud: String,             // must equal the issuer's audience/issuer id
    pub instance_id: String,     // must equal the registered instance entry id
    pub user_subject: String,    // stable per-user id; server stores only its hash
    pub nonce: String,           // >=128-bit, base64url; replay key
    pub exp: i64,                // unix seconds; <= now + 5 min
}

pub struct TraceInstanceEnrollRequest {
    pub schema_version: String,
    pub instance_public_key: String,   // base64 ed25519; selects the allowlist entry
    pub device_public_key: String,     // base64 ed25519; the user's device key
    pub attestation: TraceInstanceEnrollAttestation,
    pub attestation_sig: String,       // base64 ed25519 over canonical attestation bytes
    pub client_info: TraceOnboardClientInfo,
}
```

Canonical attestation bytes: a single deterministic encoder
(`instance_enroll_attestation_signing_bytes(&attestation)`) shared by signer and
verifier — length-prefixed / fixed-field-order so no field-boundary confusion is
possible. Tenant derivation: `derive_user_tenant_id(instance_id, user_subject) =
"tenant-" + hex(sha256(instance_id || 0x1F || user_subject))` (an unambiguous
separator between the two fields). Both are "one function, no drift" pins.

Response: the existing `TraceOnboardResponse` (`tenant_id` = the derived per-user
tenant, `ingest_url`, `issuer_url`, `audience`, `device_key_id`, optional
`contributor_label` from the entry `note_label`, `community_url`, `profile_url`,
`leaderboard_url`).

## Enroll handler flow (`POST /v1/enroll`)

Fail-closed at every step; the verification-failure classes collapse to a uniform
generic refusal with a fixed-latency floor (the endpoint is not an oracle).

1. **Schema + key shape.** `schema_version` matches; `instance_public_key` and
   `device_public_key` each decode to exactly 32 bytes. Malformed -> generic
   refuse.
2. **Resolve instance entry.** `hash_instance_subject(instance_pubkey)` -> look up
   the instance entry in the current allowlist snapshot (respecting the max-stale
   fail-closed check). Absent -> `denial_counter.record()` + generic refuse.
3. **Verify attestation.** Ed25519-verify `attestation_sig` over
   `instance_enroll_attestation_signing_bytes(attestation)` against the
   **registered** `instance_public_key`. Then check (any miss -> generic refuse):
   - `attestation.aud == issuer audience/id`,
   - `attestation.instance_id == entry.instance_id`,
   - `attestation.exp > now` and `attestation.exp <= now + 5 min`,
   - `attestation.device_key_id == device_key_id_from_public_key_bytes(device_pubkey)`.
4. **Replay dedupe.** Atomically consume `(instance_subject_hash, nonce)` in a
   per-process replay cache, TTL = remaining attestation lifetime. Seen -> generic
   refuse. (Backstop: `exp` + derived-tenant/device-key idempotency.)
5. **Rate limit.** Per-instance-subject token bucket (`rate_per_min` from entry or
   env default). Exhausted -> `denial_counter.record()` + generic refuse.
6. **Derive tenant + cap (ledger transaction).** Compute
   `tenant_id = derive_user_tenant_id(entry.instance_id, attestation.user_subject)`
   and `user_subject_hash = sha256(user_subject)`. In an instance-scoped
   transaction (`SET LOCAL trace_commons.instance_subject = instance_subject_hash`):
   - If a ledger row for `(instance_subject_hash, user_subject_hash)` already
     exists, this is an **existing user** (new device or replay) -> skip the cap
     check.
   - Else enforce `max_enrollments`: if `COUNT(*)` for the instance is at the cap
     -> `denial_counter.record()` + generic refuse; otherwise `INSERT` the ledger
     row `ON CONFLICT DO NOTHING` (idempotent under concurrency).
7. **Provision + enroll (derived-tenant transaction).**
   `begin_trace_tenant_transaction(tenant_id)`, then:
   - `ensure_trace_tenant(tenant_id)` (create or no-op).
   - Create-or-reuse `trace_tenant_policies` from `entry.policy_template`
     (`INSERT ... ON CONFLICT (tenant_id) DO NOTHING`); never overwrite a policy
     an existing user tenant already has.
   - Register the device key (existing insert; unique on `device_key_id`,
     idempotent).
   - Create-or-reuse the account + bind the principal (Slice-1
     `ON CONFLICT (tenant_id, principal_ref) DO NOTHING`). `principal_ref` is the
     device-key principal; multiple devices = multiple principals in the one
     user tenant.
8. **Audit + respond.** Hash-only / label-only `instance_enroll` event: actor =
   instance subject hash (or `note_label`), `tenant_id` (a hash),
   `user_subject_hash`, outcome label `new_user | new_device | reused`. Never the
   raw pubkey, `user_subject`, nonce, sig, device bytes, or URLs. Return
   `TraceOnboardResponse`.

The ledger transaction (step 6, instance dimension) and the provisioning
transaction (step 7, tenant dimension) are separate because they live on
different RLS predicates. They cannot diverge meaningfully: both keys are
deterministic functions of the verified attestation, and every write is
idempotent, so a crash between them simply retries to the same result.

## Cross-tenant aggregation

Leaderboards, community analytics, and instance-level rollups read across many
per-user tenants. The flow: read `trace_instance_enrollments` under the instance
predicate to get the tenant list, then aggregate those tenants through a
**privileged aggregate path** — a dedicated read role / code path that iterates
the listed tenants (each under its own `begin_trace_tenant_transaction`) or runs a
narrowly-scoped cross-tenant read. This is the accepted cost of tenant-per-user
and is **out of scope for this slice** beyond defining the ledger that makes it
possible; the existing single-tenant community surfaces keep working unchanged for
invite-onboarded tenants.

## Error / refusal taxonomy

| Class | Outward | Audited (hash-only) |
|---|---|---|
| schema / malformed key | generic `EnrollMalformed`, 400 | yes, label only |
| unknown instance | generic refuse, 403, uniform timing | denial counter + label |
| bad sig / aud / instance_id / exp / device-id mismatch | one generic refuse, 403, uniform timing | label only (no which-check leak) |
| replay (nonce seen) | generic refuse, 403 | label only |
| rate limited | generic refuse, 429 | denial counter |
| over `max_enrollments` | generic refuse, 403 | denial counter + label |
| provisioning / DB error | generic internal, 500 | label only |

The {unknown, bad-sig, aud, instance, exp, device-mismatch, replay} classes
collapse to one generic forbidden with uniform status and a fixed-latency floor,
mirroring the contributor-account redeem discipline. Cap/rate return their own
statuses (403/429) but a generic body — they leak no per-user or secret
information. Granular distinctions live only in the hash-only audit trail.

## Security hardening

- **A. Tenant is derived, not asserted.** `tenant_id` is computed server-side from
  the verified `instance_id` + `user_subject`; no client input selects a tenant.
  The provisioning transaction runs under that derived tenant.
- **B. Verify against registered key bytes.** Signature is checked against the
  allowlist entry's `instance_public_key`. A request with a different pubkey fails
  to resolve an entry (step 2) or fails verification.
- **C. Audience + instance binding stops cross-instance / cross-issuer replay.**
  `aud` and `instance_id` are signed and equality-checked, so an attestation
  minted for instance A / issuer X cannot be replayed at instance B or another
  issuer.
- **D. Derived-tenant + device-key idempotency makes replay safe.** Replaying an
  attestation re-derives the same tenant and re-targets the same unique
  `device_key_id` — no new tenant, no new account, no cap growth. The nonce cache
  blocks the residual (cap burn / re-add of a removed key within the 5-min
  window).
- **E. Cap counts distinct users via the ledger, transactionally.** Enforced as a
  ledger `COUNT(*)` under the instance predicate inside the ledger transaction, so
  it cannot be raced past the ceiling. A new device for an existing user does not
  consume cap (existing PK row).
- **F. Two RLS dimensions, both forced.** Per-user data is isolated by the tenant
  predicate; the enrollment ledger is isolated by the instance predicate. Neither
  uses `BYPASSRLS`; the issuer sets the appropriate transaction-local GUC for each.
- **G. No restricted-resolver bypass needed.** Unlike the contributor-account
  redeem path, enroll knows both the instance subject (from the verified sig) and
  the derived tenant before any write, so every write is RLS-scoped on the runtime
  pool — no separate bypass role.
- **H. Hash-only everywhere.** Instance pubkey, `user_subject`, nonce, signature,
  and device-key bytes never reach a log line, audit row, or admin response; only
  their hashes (and `tenant_id`, itself a hash) or operator labels do.
- **I. Policy is never overwritten.** Stamping the policy template is
  `ON CONFLICT (tenant_id) DO NOTHING`, so re-enrollment of an existing user can
  never silently change the contribution policy their data was collected under.

## Testing strategy

- **Allowlist parse:** instance entries validate (32-byte pubkey,
  non-empty `instance_id`, `max_enrollments >= 1`, required well-formed
  `policy_template`); malformed pubkey / zero cap / missing template reject the
  whole snapshot; existing invite-only files still parse; mixed file parses;
  instance subject hash is namespaced (`instance:` vs `invite:` never collide).
- **Tenant derivation:** `derive_user_tenant_id` is stable and collision-resistant
  across the field separator (e.g. `("a","bc")` != `("ab","c")`); same
  `(instance_id, user_subject)` always yields the same `tenant_id`.
- **Attestation verify:** valid signature enrolls; wrong signer, tampered field,
  wrong `aud`, wrong `instance_id`, expired, future-beyond-window, and
  device-id-mismatch each return the uniform generic refuse with identical status
  and timing floor; verification is against the registered key bytes.
- **Replay:** second enroll with the same `(instance_subject_hash, nonce)` is
  refused; a fresh nonce for the same device is idempotent (no second account, no
  cap consumption).
- **Multi-device / per-user tenant:** user U device 1 then device 2 land in the
  **same** derived tenant as two principals; the ledger has one row for U; cap is
  not consumed by the second device; a different `user_subject` under the same
  instance gets a distinct tenant.
- **Cap + rate:** distinct users past `max_enrollments` refuse; an existing user's
  new device does not refuse; token bucket refuses past `rate_per_min`; both
  increment the denial counter.
- **Provisioning effects:** one tenant transaction creates the tenant + stamps the
  policy from the template + registers the device key + creates the account; a
  pre-existing user tenant's policy is **not** overwritten on re-enroll
  (`ON CONFLICT DO NOTHING`); RLS confines every row to the derived tenant; two
  different users cannot read each other's traces (now enforced by tenant RLS, not
  just the account predicate — assert cross-tenant denial).
- **Ledger RLS:** an instance can read/insert only its own ledger rows under the
  instance predicate; a different instance subject sees none.
- **Audit:** `instance_enroll` event is hash-only / label-only; no raw pubkey,
  `user_subject`, nonce, sig, device bytes, or URL appears in any row or log;
  `new_user | new_device | reused` outcome label is correct.
- **Invite path untouched:** existing `/v1/onboard` tests still pass; the invite
  flow shares no mutable state with enroll.
- Local verification per CLAUDE.md: `RUSTFLAGS="-D warnings" cargo check -p
  trace-commons-server --bins`, `RUSTFLAGS="-D warnings" cargo test -p
  trace-commons-server --no-run`, clippy with the repo allow-list, the
  PostgreSQL-backed contract tests, and a V31 migration/RLS test (forced RLS +
  instance-isolation policy + cross-instance denial).

## Out of scope / later

- **TEE attestation of the instance** (binding the Ironclaw enclave measurement to
  `instance_id`) replaces the registered-pubkey check with a quote verification
  while reusing the same derivation and downstream provisioning.
- **User-owned-key portability across hosts.** Today the tenant is keyed to
  `(instance_id, user_subject)`, so a user is durable within one host. Keying the
  tenant to a user-controlled key (NEAR / passkey) so it survives *across* hosts is
  a later graft: a strong user authenticator re-points the same `user_subject` (or
  links the existing tenant) — the contributor-account Slice-2/3 identity work is
  where that lands. The ledger's `user_subject_hash` is the join key it will reuse.
- **Cross-tenant aggregation path** (leaderboards / instance rollups) — only the
  ledger that enables it is in scope here; the privileged aggregate read is a
  separate slice.
- **NEAR on-chain instance registry** rides the already-reserved `Near`
  `AllowlistSource` variant — instance entries resolve from a view call instead of
  the file, no handler change.
- **One-tenant-per-instance**, shared-bearer token, operator pre-authorized user
  batches, and per-device-keyed tenants were considered and rejected (see Resolved
  decisions and the brainstorming record).

## Residual risks / accepted tradeoffs

1. **Cross-user aggregation now crosses RLS.** Leaderboards / instance rollups
   require the privileged aggregate path over the ledger's tenant list. Accepted
   explicitly as the price of the hard per-user isolation wall; bounded by keeping
   that path narrow and read-only.
2. **`user_subject` stability is the instance's responsibility.** If the instance
   emits a different `user_subject` for the same human, that human gets a second
   tenant (split credit). We cannot detect this server-side (we only see hashes);
   it is a documented instance-integration contract. Mitigated later by
   user-owned-key linking.
3. **Replay cache is process-local.** A restart clears consumed nonces, so a
   captured attestation could be replayed within its <=5-min `exp`. Replay is
   idempotent on the derived tenant + device key, so the only residual is one unit
   of cap accounting for a user the instance already vouched; a durable nonce store
   is deferred (matches the `DenialCounter` posture).
4. **Instance key compromise = enroll up to `max_enrollments` users into fresh
   tenants.** Bounded by the cap + rate limit; mitigated by operator rotation of
   the registered pubkey. This is the intended trust boundary — the instance is a
   vouching authority by design.
5. **Tenant count grows with users, not customers.** Many lightweight tenants
   (rows + derived prefixes + AAD contexts, no per-tenant KMS/bucket) instead of a
   few heavy ones. Accepted; the provisioning cost per tenant was verified to be a
   handful of idempotent inserts.
6. **Allowlist schema change touches the invite parser.** Mitigated by defaulting
   `kind` to `invite` so every existing file stays valid, with explicit round-trip
   tests on invite-only, instance-only, and mixed files.
