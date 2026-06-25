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
which every user on that instance self-enrolls their **own device key + own
account** with **no per-user invite code**. The instance acts as a vouching
authority: it signs a short-lived enrollment attestation for each user's device
key, and the issuer verifies that signature against the instance's registered
public key.

The existing invite path is **left fully intact** — direct human contributors
keep using invite codes; instance-vouched enrollment is an additive sibling path.

## Resolved decisions (from brainstorming)

1. **Tenant model: one tenant per instance, per-user accounts.** The Ironclaw
   instance maps to a single Trace Commons tenant, provisioned **once** by the
   operator (existing `trace-commons-tenant` tooling). Each enrolling user is a
   distinct **account + principal** within that tenant, reusing the
   contributor-account Slice-1 primitives (`trace_accounts`,
   `trace_account_principals`). Cross-tenant isolation is Postgres RLS;
   within-tenant user isolation is the Slice-1 account-visibility predicate
   (set-expansion on the active principal set, `can_review`/legacy wildcards
   already removed). Tenant-per-user was rejected: it would force dynamic tenant
   + storage + KEK + RLS-grant provisioning from a self-service path (the exact
   unauthenticated-INSERT-into-RLS-forced-tables fail-open surface the
   contributor-account spec avoids) and fragment credit across thousands of
   tenants.

2. **Trust model: instance signs each enrollment (issuer model).** Operator
   registers the instance's Ed25519 **public key** once. The instance signs a
   short-lived attestation over the user's device public key; the issuer verifies
   the signature, resolves the tenant, registers the device key. No shared secret
   travels with users; each enrollment is independently fresh. Shared-bearer and
   operator-pre-authorized-batch models were rejected (leakable reusable secret;
   reintroduces per-user operator action). TEE attestation of the instance is a
   later hardening that grafts onto the same tenant resolution.

3. **Anti-Sybil: hard cap + per-instance rate limit.** The allowlist entry
   carries a `max_enrollments` ceiling; the enroll endpoint applies a per-instance
   token-bucket rate limit. Over either bound -> generic refusal + denial counter.

4. **Attestation freshness: self-asserted `exp` + `nonce`, server dedupes.**
   Single round trip. Server enforces `exp` (<=5 min), binds `aud` to this issuer
   and `tenant_id` to the registered entry (no cross-instance reuse), and records
   consumed nonces until `exp` to block replay. Replay is already idempotent on
   `device_key_id` (the registry is unique on it); the nonce cache additionally
   stops quota burn and re-add of a removed key. Server-issued challenge nonce was
   rejected as needless state + an extra round trip for the pilot.

5. **Account creation timing: eager at enroll.** Confirmed feasible because
   `onboard_device_key` already runs inside `begin_trace_tenant_transaction`
   (`db/postgres.rs:1239`), so the account + principal rows are created in the
   **same** RLS-scoped tenant transaction using Slice-1's idempotent
   create-or-reuse. The user has their own account from the first enrollment.

## Repo invariants this design must honor

- PostgreSQL-only; every Trace Commons table has forced RLS via
  `trace_current_tenant_id()`. No new RLS-bypass surface is introduced — enroll
  resolves the tenant from the registered allowlist entry and operates under
  `begin_trace_tenant_transaction(tenant_id)`, the same authenticated-tenant
  pattern `onboard_device_key` already uses.
- Hash-only / label-only audit and logs: never store or log the raw instance
  public key, nonce, signature, device public key bytes, URLs, or contributor
  identity. Subjects and actors are hashes or operator labels.
- Fail-closed: any verification, rate, cap, replay, or DB error -> one generic
  refusal with a safe label; never fall through to an unauthenticated or elevated
  state.
- Tenant scoping is auth-derived (here: derived from the verified instance
  attestation -> registered tenant), never client-asserted as ground truth — the
  `tenant_id` in the attestation is checked for **equality** against the
  registered entry, it does not select the tenant.
- Match existing module/handler/migration patterns; do not split
  `trace-commons-ingest.rs` or the issuer module.

## Components

- **`crates/trace-commons-protocol/src/onboarding.rs`** — new request/response
  message types and a canonical attestation-bytes encoder (the single source of
  truth both signer and verifier use, mirroring the
  `hash_invite_code`/`device_key_id_from_public_key_bytes` "one function, no
  drift" convention). New error-code variants on `TraceOnboardErrorCode` (or a
  sibling enum) for the enroll-specific refusals.
- **`crates/trace-commons-server/src/trace_upload_claim_allowlist.rs`** — extend
  the allowlist file schema with a tagged entry **kind** (`invite` | `instance`).
  `instance` entries carry the Ed25519 `instance_public_key`, `tenant_id`,
  `max_enrollments`, optional `rate_per_min`, and `note_label`. The snapshot gains
  an instance-entry lookup keyed by the instance subject hash. `AllowlistSource`
  trait, file caching, max-stale fail-closed, and the reserved `Near` source are
  unchanged.
- **`crates/trace-commons-server/src/trace_upload_claim_issuer.rs`** — new
  `enroll` method on the issuer state (sibling to `onboard`), the new
  `POST /v1/enroll` route, attestation verification, per-instance rate limiter +
  replay cache, and the cap check. Reuses `denial_counter`,
  `onboard_allowlist_snapshot`, response construction, and the env-config plumbing
  already present for `/v1/onboard`.
- **`crates/trace-commons-server/src/db/`** — a single
  `enroll_instance_device_key` (or an extension to `onboard_device_key`) that, in
  one `begin_trace_tenant_transaction`, (a) registers the device key and (b)
  create-or-reuses the `trace_accounts` row and binds the
  `trace_account_principals` row via the Slice-1
  `ON CONFLICT (tenant_id, principal_ref) DO NOTHING` logic. Enforces
  `max_enrollments` as a count of device keys registered for this tenant under
  this instance subject.

No new migration is required: V30 (contributor-account slice) already defines
`trace_accounts` / `trace_account_principals`, and `device_keys.invite_subject_hash`
(V28) already records which subject onboarded each key — the instance subject hash
reuses that existing column, so `max_enrollments` is a `COUNT(*)` over it with no
schema change. (If a later cleanup wants to document the widened meaning of
`invite_subject_hash`, that is a comment-only edit, not a structural migration, and
is out of scope here.)

## Allowlist schema extension

Today an `AllowlistEntry` is implicitly an invite (`subject_hash`, `tenant_id`,
`note_label`, `max_uses`). The file gains a tagged kind so the two subject types
parse side by side. Existing invite files remain valid (kind defaults to
`invite`):

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
      "instance_public_key": "<base64 ed25519 32-byte>",
      "tenant_id": "tenant-ironclaw-acme",
      "max_enrollments": 250,
      "rate_per_min": 20,
      "note_label": "ironclaw-acme-prod" }
  ]
}
```

Validation rules for `instance` entries:

- `instance_public_key` decodes to exactly 32 bytes; reject the whole snapshot
  otherwise (same strict posture as invite `subject_hash`).
- `tenant_id` non-empty (existing rule).
- `max_enrollments >= 1` (parallels the `max_uses > 0` rule).
- `rate_per_min` optional; falls back to an env default
  (`TRACE_COMMONS_INSTANCE_ENROLL_RATE_PER_MIN`) when omitted.
- The **instance subject hash** = `hash_instance_subject(pubkey_bytes)` =
  `sha256("instance:" || pubkey_bytes)`, namespaced exactly as
  `hash_invite_code` uses `"invite:"`, so an instance pubkey can never collide
  with an invite-code subject hash. This is the single hashing function used for
  denial accounting and the `device_keys.invite_subject_hash` attribution column.

The snapshot exposes `instance_entry(subject_hash) -> Option<InstanceEntry>`
distinct from the invite `entry(...)` lookup; the hot path verifies a signature,
so the snapshot also needs to expose the raw `instance_public_key` bytes for the
matched entry.

## Wire protocol

New protocol types in `onboarding.rs`. The invite `TraceOnboardRequest` /
`TraceOnboardResponse` are unchanged; the response type is **reused** by enroll.

```rust
pub const TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION: &str =
    "trace_commons.instance_enroll_request.v1";

pub struct TraceInstanceEnrollAttestation {
    pub device_key_id: String,   // sha256: of the device pubkey bytes
    pub aud: String,             // must equal the issuer's audience/issuer id
    pub tenant_id: String,       // must equal the registered instance entry tenant
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
(`instance_enroll_attestation_signing_bytes(&attestation)`) shared by the signer
(Ironclaw) and the verifier (issuer). It MUST be unambiguous — length-prefixed or
fixed-field-order concatenation of the five fields — so no field-boundary
confusion is possible. This function is the "one function, no drift" pin.

Response: the existing `TraceOnboardResponse` (`tenant_id`, `ingest_url`,
`issuer_url`, `audience`, `device_key_id`, optional `contributor_label`,
`community_url`, `profile_url`, `leaderboard_url`). `contributor_label` is sourced
from the instance entry's `note_label`.

## Enroll handler flow (`POST /v1/enroll`)

Fail-closed at every step; all refusals collapse to a uniform generic response
with a fixed-latency floor (the only granular signal is rate/cap which still
returns the same generic body — granularity lives only in hash-only audit).

1. **Schema + key shape.** `schema_version` matches;
   `instance_public_key` and `device_public_key` each decode to exactly 32 bytes
   (reuse the existing device-key length check). Malformed -> generic refuse.
2. **Resolve instance entry.** Compute `hash_instance_subject(instance_pubkey)`;
   look up the instance entry in the current allowlist snapshot (respecting the
   existing max-stale fail-closed check). Absent -> `denial_counter.record()` +
   generic refuse.
3. **Verify attestation.** Ed25519-verify `attestation_sig` over
   `instance_enroll_attestation_signing_bytes(attestation)` against the
   **registered** `instance_public_key` from the entry (not the request's, though
   they must be equal — verifying against the registered bytes is the
   belt-and-suspenders). Then check, all -> generic refuse on any miss:
   - `attestation.aud == issuer audience/id`,
   - `attestation.tenant_id == entry.tenant_id`,
   - `attestation.exp > now` and `attestation.exp <= now + 5 min`,
   - `attestation.device_key_id == device_key_id_from_public_key_bytes(device_pubkey)`.
4. **Replay dedupe.** Atomically consume `(instance_subject_hash, nonce)` in a
   per-process replay cache with TTL = the attestation's remaining lifetime
   (bounded by the 5-min max). Already-seen -> generic refuse. (Process-local,
   restart-resets; the `exp` bound and `device_key_id` idempotency are the
   cryptographic backstop, mirroring the allowlist `DenialCounter` posture.)
5. **Rate limit.** Per-instance-subject token bucket
   (`rate_per_min` from entry or env default). Exhausted ->
   `denial_counter.record()` + generic refuse.
6. **Cap + enroll (one tenant transaction).**
   `begin_trace_tenant_transaction(entry.tenant_id)`, then:
   - Enforce `max_enrollments`: `COUNT(*)` of `device_keys` where
     `tenant_id = entry.tenant_id AND invite_subject_hash = instance_subject_hash`;
     if the new device key is **not already** registered and the count is at the
     cap -> refuse. (Re-enrolling an already-registered device key is idempotent
     and does not consume cap.)
   - Register the device key (existing insert; unique on `device_key_id`,
     idempotent under concurrency).
   - Create-or-reuse the account: `SELECT account_id FROM
     trace_account_principals WHERE tenant_id=$t AND principal_ref=$device_key_id
     AND unlinked_at IS NULL`; if absent, `INSERT INTO trace_accounts ...
     RETURNING account_id`, then `INSERT INTO trace_account_principals ...
     ON CONFLICT (tenant_id, principal_ref) DO NOTHING`, re-select. (Exactly the
     Slice-1 mint create-or-reuse; `principal_ref` is the device-key principal.)
7. **Audit + respond.** Hash-only / label-only `instance_enroll` audit event:
   actor = instance subject hash (or `note_label`), outcome label
   `created | reused`, `tenant_id`. Never the raw pubkey, nonce, sig, device-key
   bytes, or URLs. Return `TraceOnboardResponse`.

## Error / refusal taxonomy

| Class | Outward | Audited (hash-only) |
|---|---|---|
| schema / malformed key | generic `EnrollMalformed`, 400 | yes, label only |
| unknown instance | generic refuse, 403, uniform timing | denial counter + label |
| bad signature / aud / tenant / exp / device-id mismatch | one generic refuse, 403, uniform timing | label only (no which-check leak) |
| replay (nonce seen) | generic refuse, 403 | label only |
| rate limited | generic refuse, 429 | denial counter |
| over `max_enrollments` | generic refuse, 403 | denial counter + label |
| DB / store error | generic internal, 500 | label only |

Mirrors the redeem-path discipline from the contributor-account slice: the
{unknown, bad-sig, aud, tenant, exp, device-mismatch, replay} classes collapse to
one generic forbidden with uniform status and a fixed-latency floor, so the enroll
endpoint is not an oracle. Granular distinctions live only in the hash-only audit
trail.

## Security hardening

- **A. Tenant is registered, not asserted.** `attestation.tenant_id` is checked
  for equality against the allowlist entry's tenant; it never selects the tenant.
  The transaction runs under `begin_trace_tenant_transaction(entry.tenant_id)`.
  No client input chooses a tenant.
- **B. Verify against registered key bytes.** Signature is checked against the
  allowlist entry's `instance_public_key`, not the request's copy. A forged
  request that supplies a different pubkey simply fails to resolve an entry (step
  2) or fails verification.
- **C. Audience + tenant binding stops cross-instance / cross-issuer replay.** An
  attestation minted for instance A / issuer X cannot be replayed at instance B or
  a different issuer because `aud` and `tenant_id` are signed and equality-checked.
- **D. `device_key_id` binding makes replay idempotent.** The attestation binds
  the exact device key; replay only ever re-targets that key, and the registry is
  unique on `device_key_id`. The nonce cache exists to stop cap burn and re-add of
  a removed key within the 5-min window, not to provide the primary anti-forgery
  guarantee (the signature does that).
- **E. Cap counts real registrations.** `max_enrollments` is enforced as a DB
  count of device keys attributed to the instance subject, inside the tenant
  transaction, so it cannot be raced past the ceiling. Idempotent re-enroll of an
  existing key does not consume budget.
- **F. Rate + denial visibility.** Per-instance token bucket plus the existing
  `DenialCounter` feeding `/v1/admin/allowlist-status` give operators a
  hash-only signal of abuse without exposing identities.
- **G. No new RLS-bypass.** Unlike the contributor-account redeem path, enroll has
  a resolved tenant *before* any DB write (from the verified attestation), so it
  uses the standard runtime pool under `trace_current_tenant_id()` — no restricted
  resolver role, no bypass.
- **H. Hash-only everywhere.** Instance pubkey, nonce, signature, and device-key
  bytes never land in a log line, audit row, or admin response; only their hashes
  or operator labels do.

## Testing strategy

- **Allowlist parse:** instance entries validate (32-byte pubkey,
  `max_enrollments >= 1`, optional `rate_per_min`); malformed pubkey or zero cap
  rejects the whole snapshot; existing invite-only files still parse (kind
  defaults to `invite`); mixed invite + instance file parses; instance subject
  hash is namespaced (`instance:` vs `invite:` never collide).
- **Attestation verify:** valid signature enrolls; wrong signer, tampered field,
  wrong `aud`, wrong `tenant_id`, expired, future-beyond-window, and
  device-id-mismatch each return the uniform generic refuse with identical status
  and timing floor; verification is against the registered key bytes.
- **Replay:** second enroll with the same `(instance_subject_hash, nonce)` is
  refused; a fresh nonce for the same device key is idempotent (no second account,
  no cap consumption).
- **Cap + rate:** enrollments past `max_enrollments` refuse; idempotent re-enroll
  of an existing key does not consume cap; token bucket refuses past
  `rate_per_min`; both increment the denial counter.
- **Enroll DB effects:** one tenant transaction registers the device key AND
  create-or-reuses the account + principal; a second user under the same instance
  gets a distinct account; RLS confines all rows to the instance tenant; the
  account-visibility predicate keeps user A from reading user B's traces (Slice-1
  regression already covers the read side — assert it holds for instance-enrolled
  accounts).
- **Audit:** `instance_enroll` event is hash-only / label-only; no raw pubkey,
  nonce, sig, device bytes, or URL appears in any row or log; created-vs-reused
  outcome label is correct.
- **Invite path untouched:** existing `/v1/onboard` tests still pass; the invite
  flow shares no mutable state with enroll.
- Local verification per CLAUDE.md: `RUSTFLAGS="-D warnings" cargo check -p
  trace-commons-server --bins`, `RUSTFLAGS="-D warnings" cargo test -p
  trace-commons-server --no-run`, clippy with the repo allow-list, and the
  PostgreSQL-backed contract tests.

## Out of scope / later

- **TEE attestation of the instance** (binding the Ironclaw enclave measurement to
  the tenant) replaces the registered-pubkey check with a quote verification while
  reusing the same tenant resolution and downstream enroll. Grafts on without
  touching the account/RLS/audit shape.
- **NEAR on-chain instance registry** rides the already-reserved `Near`
  `AllowlistSource` variant — instance entries resolve from a view call instead of
  the file, no handler change.
- **Tenant-per-user**, **shared-bearer enrollment token**, and **operator
  pre-authorized user batches** were considered and rejected (see Resolved
  decisions).
- **Per-account login-link / web read-back** for instance-enrolled users is
  already provided by the contributor-account slice — enrollment creates exactly
  the account shape that slice consumes; no additional work here.

## Residual risks / accepted tradeoffs

1. **Replay cache is process-local.** A restart clears consumed nonces, so a
   captured attestation could be replayed across a restart within its <=5-min
   `exp`. Accepted: replay is idempotent on `device_key_id` (no new account, no
   different key), and the only residual is one unit of cap consumption for a key
   the instance already vouched for. A durable nonce store is deferred (matches the
   `DenialCounter` process-local posture).
2. **Instance key compromise = enroll up to `max_enrollments` into the tenant.**
   Bounded by the cap and rate limit; mitigated by operator rotation of the
   registered pubkey (drop/replace the allowlist entry). This is the intended
   trust boundary — the instance is a vouching authority by design.
3. **`max_enrollments` is per-instance-subject, not per-real-human.** A misbehaving
   instance can spend its budget on synthetic users. Accepted for the pilot; the
   instance is a trusted TEE-hosted source and the cap bounds blast radius.
4. **Allowlist schema change touches the invite parser.** Mitigated by defaulting
   `kind` to `invite` so every existing file remains valid, with explicit
   round-trip tests on invite-only, instance-only, and mixed files.
