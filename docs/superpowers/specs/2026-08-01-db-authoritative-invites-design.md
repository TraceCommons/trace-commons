# DB-Authoritative Invites — Design

Date: 2026-08-01
Status: approved, pending implementation plan
Branch: `db-authoritative-invites`

## Problem

Invite validity and tenant routing live in an operator-edited JSON file
(`TRACE_COMMONS_ALLOWLIST_SOURCE=file:<path>`). Provisioning an invite means
running `scripts/operator/generate-pilot-invites.py` on the pilot host, editing
the file, and restarting `trace-commons-upload-claim-issuer.service`. The
`onboarding_invites` table (V29) records only redemption counters; it is not a
source of truth.

That is workable for hand-issued pilot invites and unworkable for anything
programmatic. It also means invite entropy is decided by an operator script
rather than by the service that enforces the invite.

This design makes the database authoritative for contributor invites and adds
an authenticated admin API for their lifecycle. It is the prerequisite slice
for a later self-serve credential-proof issuance web app, which is designed
separately.

## Scope

In scope:

- New tenant-less invite table with its own RLS roles and policies.
- An `InviteRegistry` abstraction, DB-backed, with in-process cache
  invalidation on write.
- Admin create/list/revoke/status routes on the issuer, gated on the existing
  EdDSA admin JWT.
- Redemption through the registry, supporting both a fixed tenant (imported
  pilot invites) and a tenant derived at redemption.
- A one-time import of existing file invite entries, and a staged cutover that
  ends with invite entries in the allowlist file being a parse error.

Out of scope, deferred to the self-serve issuance design:

- OAuth of any kind, outbound email, any public web surface.
- Rate limiting on public issuance, CAPTCHA / Turnstile.
- On-chain (NEAR) invite sources. `AllowlistSourceSpec::Near` stays reserved
  and unimplemented.

The allowlist file is **not** retired. It keeps carrying `kind: "instance"`
TEE entries and the `policy_label`, which are operator-provisioned
infrastructure rather than contributor invites.

## Data model

### Migration V42 — `onboarding_invite_grants`

Deliberately tenant-less: an invite has no tenant until it is redeemed, and
lookup is by invite hash alone.

| Column | Notes |
|---|---|
| `invite_subject_hash` | PK, `CHECK (~ '^sha256:[0-9a-f]{64}$')` |
| `policy_label` | Invite pool / batch label; mirrors the file's `policy_label` |
| `tenant_mode` | `'fixed'` or `'derived'` |
| `fixed_tenant_id` | `NOT NULL` iff `tenant_mode = 'fixed'` |
| `tenant_template_id` | `NOT NULL` iff `tenant_mode = 'derived'`; feeds `derive_user_tenant_id()` |
| `policy_version` | Grant policy version stamped onto the redeemed device |
| `allowed_consent_scopes` | `TEXT[]`, per-invite grant defaults |
| `allowed_uses` | `TEXT[]`, per-invite grant defaults |
| `max_uses` | `INTEGER NOT NULL DEFAULT 3 CHECK (max_uses > 0)` |
| `expires_at` | `TIMESTAMPTZ NULL`; NULL means no expiry |
| `issuance_source` | Label-only: `'operator'`, `'import:file'`, `'credential:<verifier>'` |
| `issued_by_label` | Label-only. Never an identity |
| `credential_binding_hash` | `'sha256:...'`, nullable |
| `note_label` | Operator free text. Never returned to clients, never logged |
| `revoked_at`, `created_at`, `updated_at` | |

A `CHECK` constraint enforces the `tenant_mode` / tenant-column pairing in both
directions, so neither column can be set for the wrong mode.

Partial unique index:

```sql
CREATE UNIQUE INDEX idx_onboarding_invite_grants_credential
    ON onboarding_invite_grants (policy_label, credential_binding_hash)
    WHERE credential_binding_hash IS NOT NULL AND revoked_at IS NULL;
```

One verified credential yields at most one live invite per pool. Revoking frees
the binding for reissue. This constraint is what makes later automatic issuance
safe to turn on, and it costs nothing now.

`allowed_consent_scopes` / `allowed_uses` / `policy_version` on the invite
replace the process-wide onboarding grant defaults, which closes the issuer
grant-DB configuration gap that blocked `model_training` consent during the
first pilot trial upload.

V29 `onboarding_invites` is **untouched**. It continues to record per-tenant
redemption counts. The split is deliberate: V42 answers "may this code be
redeemed", V29 answers "how many times has it been redeemed under this tenant".

### RLS

The table is tenant-less, so `trace_current_tenant_id()` cannot govern it.
`ENABLE` + `FORCE ROW LEVEL SECURITY` as on every other Trace Commons table.
Two access paths with two different policies.

**Redemption / lookup path.** Mirrors V35's GUC-predicate style rather than a
blanket `USING (true)`:

```sql
CREATE OR REPLACE FUNCTION trace_current_invite_subject()
RETURNS TEXT LANGUAGE SQL STABLE AS $$
    SELECT NULLIF(current_setting('trace_commons.invite_subject', true), '');
$$;

CREATE POLICY invite_lookup ON onboarding_invite_grants
    FOR SELECT USING (invite_subject_hash = trace_current_invite_subject());
```

The issuer sets `trace_commons.invite_subject` to the hash of the code the
caller actually presented, inside the redemption transaction. A lookup can only
ever return the row for a code the caller already knows. The hot path cannot
enumerate live invites.

**Admin path.** Needs cross-invite visibility, so it gets a narrow role on its
own pool, following the `trace_login_resolver` (V30/V33) and
`trace_gate_driver` (V36) pattern:

```sql
CREATE ROLE trace_invite_admin NOLOGIN NOBYPASSRLS;  -- if not exists
GRANT SELECT, INSERT, UPDATE ON onboarding_invite_grants TO trace_invite_admin;

CREATE POLICY trace_invite_admin_all ON onboarding_invite_grants
    FOR ALL TO trace_invite_admin USING (true) WITH CHECK (true);
```

`NOBYPASSRLS` is load-bearing: the permissive policy is what authorizes the
role, not a bypass. The role runs on a separate pool configured by
`TRACE_COMMONS_INVITE_ADMIN_DB_URL` and is never aliased to the runtime pool.
Operator role provisioning is a documented runbook step, as with the existing
restricted roles.

## `InviteRegistry`

New module `crates/trace-commons-server/src/trace_invite_registry.rs`.
`AllowlistSnapshot` is left alone; it now carries only instance entries and the
`policy_label`. Invite lookups move out of it entirely.

```rust
pub trait InviteRegistry: Send + Sync {
    fn lookup(&self, invite_subject_hash: &str) -> Result<Option<InviteEntry>, InviteRegistryError>;
    fn note_write(&self, entry: InviteEntry);
    fn note_revoke(&self, invite_subject_hash: &str);
    fn status(&self) -> InviteRegistryStatus;
}
```

`DbInviteRegistry` holds a cached map of live invites refreshed on a timer,
with the same `stale` / `max_stale_seconds` semantics as `FileAllowlistSource`,
so the existing `PilotAllowlistStale` posture carries over unchanged.

Because invite writes happen inside the issuer process, `note_write` and
`note_revoke` are called immediately after the DB commit. A code minted through
the admin API is redeemable in the same instant, with no refresh window — which
is the requirement that self-serve issuance imposes, since a user redeems
seconds after receiving a code.

Cache freshness is a latency optimization, never a correctness boundary:
expiry, revocation, and use-count are all re-checked inside the redemption
transaction. A revoke racing a redemption is resolved by the database, not by
the cache.

Configuration:

- `TRACE_COMMONS_INVITE_REGISTRY_DB_URL` — runtime pool for lookups.
- `TRACE_COMMONS_INVITE_ADMIN_DB_URL` — `trace_invite_admin` pool.
- `TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE` — cutover flag, see below.

`AllowlistSourceSpec` gains no new variant.

## Admin API

Four routes added to `trace_upload_claim_issuer_admin.rs`. The existing
`/v1/admin/allowlist-status` route is unchanged.

Every new route is gated on the EdDSA admin JWT the issuer already signs
(`role=admin`, with `iss` / `aud` / `jti` checked), **not** on loopback binding.
Loopback binding is an acceptable posture for a counts-only status endpoint and
is not an acceptable posture for a route that mints credentials.

| Route | Behavior |
|---|---|
| `POST /v1/admin/invites` | Server generates the code, stores only the hash, returns the raw code once in the response body |
| `GET /v1/admin/invites` | Listing by hash prefix, labels, counts, state. No raw codes, no credential values |
| `POST /v1/admin/invites/{hash}/revoke` | Sets `revoked_at`, then `note_revoke` |
| `GET /v1/admin/invite-registry-status` | Counts only: live, revoked, expired, fully-consumed, cache age, stale flag |

Code generation moves server-side, using the runbook's existing unambiguous
alphabet (16 characters from `[A-Z2-9]`) drawn from a CSPRNG. The raw code
exists in exactly one response body and is never stored, logged, or
retrievable afterward. `hash_invite_code()` remains the single hashing
function; the generator and the redemption path are its only callers.

`scripts/operator/generate-pilot-invites.py` is retired in favor of a
`trace-commons-upload-claim-issuer --mint-invites` subcommand that calls the
same code path, so entropy and hashing cannot drift between the script and the
service.

## Redemption

`POST /v1/onboard` resolves the invite through `InviteRegistry` instead of the
allowlist snapshot. Within one transaction, having set
`trace_commons.invite_subject`:

1. Reject if absent, revoked, or `expires_at` has passed.
2. Resolve the tenant:
   - `tenant_mode = 'fixed'` — use `fixed_tenant_id`. Byte-for-byte the current
     behavior, so imported pilot invites keep working.
   - `tenant_mode = 'derived'` — `derive_user_tenant_id(tenant_template_id,
     user_subject)`.
3. Provision tenant, device key, and tenant-access grant through the existing
   `enroll_instance_user`-shaped transaction, using the invite's
   `allowed_consent_scopes` / `allowed_uses` / `policy_version`.
4. Increment the V29 consumption counter under the same transaction, keeping
   the existing `consumed_uses < max_uses` guard.

Idempotent re-registration of an already-registered device key keeps its
current behavior and does not consume a use.

## Cutover

Staged, so the live pilot is never dependent on an untested path.

1. Ship V42, the registry, and the admin API with the file still authoritative
   for invites. The registry is configured and populated but not consulted.
2. `trace-commons-upload-claim-issuer --import-file-invites <path>` writes
   existing file invite entries as `tenant_mode = 'fixed'`,
   `issuance_source = 'import:file'`, preserving `max_uses` and `note_label`.
   Idempotent on `invite_subject_hash`. Reports counts only.
3. Set `TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE=1`. Redemption now reads
   the registry. Remove `kind: "invite"` entries from `allowlist.json`.
4. The following release makes a `kind: "invite"` entry in the allowlist file a
   hard parse error, so a stale file can never silently re-authorize an invite
   that was revoked in the database.

Rollback between steps 3 and 4 is unsetting the flag, since the file entries
are still present and the import is idempotent.

## Errors

All new labels are hash-only and carry no contributor identity, raw code, or
credential value.

| Label | HTTP | Meaning |
|---|---|---|
| `InviteExpired` | 403 | Invite is valid but past `expires_at` |
| `InviteRegistryNotConfigured` | 503 | Authoritative mode is on with no registry DB |
| `InviteRegistryStale` | 503 | Cache is older than `max_stale_seconds` and has not reloaded |
| `InviteCredentialAlreadyBound` | 409 | Admin-only; credential binding already has a live invite |

`InviteNotValid` and `InviteAlreadyConsumed` keep their current meanings.
`InviteRegistryNotConfigured` is the fail-closed case: with the authoritative
flag set and no registry, redemption is refused rather than falling back to the
file.

## Testing

- Unit tests for `DbInviteRegistry` cache behavior: `note_write` makes an
  invite immediately visible, `note_revoke` immediately removes it, and staleness
  crosses `max_stale_seconds` at the same boundary `FileAllowlistSource` uses.
- `trace_corpus_pg_store` tests for the V42 policies exercised under `SET ROLE`,
  **not** on a superuser connection. A superuser connection hides exactly this
  class of policy bug, which is how the login-resolver policy gap was missed
  previously. Assert that the runtime role sees nothing without
  `trace_commons.invite_subject` set, sees only the matching row with it set,
  and that `trace_invite_admin` sees all rows.
- One redemption test per `tenant_mode`, asserting the grant fields come from
  the invite rather than from process-wide defaults.
- Import idempotency: importing the same file twice leaves one row per hash
  with unchanged `created_at`.
- Fail-closed test: with `TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE=1` and no
  registry configured, `/v1/onboard` returns `InviteRegistryNotConfigured`.
- Fail-closed test: once step 4 lands, an allowlist file containing a
  `kind: "invite"` entry fails to parse rather than being ignored.
- Revoke-races-redemption test asserting the database, not the cache, decides.

New migration numbers must avoid 30-34, which are already applied to the shared
`trace_commons_test` database. V42 is clear. `run_migrations` in
`db/postgres.rs` is hand-rolled — V42 must be wired into it explicitly.

## Follow-on

The self-serve credential-proof issuance web app is a separate design. It
consumes this slice's admin API with `issuance_source = 'credential:<verifier>'`
and `credential_binding_hash` set, and it is where domain email confirmation,
rate limiting, and the public web surface get designed. Nothing in this slice
exposes a public issuance path.
