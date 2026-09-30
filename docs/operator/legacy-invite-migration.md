# Legacy invite identities: coexistence, linking, and pooled-invite cutover

This runbook covers the server side of moving legacy `tenant-…` invite
identities onto NEAR accounts, as decided on 2026-09-27. It is operator
procedure; the design is in the consent spec
(`docs/superpowers/specs/2026-09-23-connect-and-forget-consent-design.md`,
"The invite-to-account migration") and the schema in
`migrations/V81__legacy_invite_link.sql`, `V91__legacy_invite_link_devices.sql`
and `V104__legacy_invite_link_device_guards.sql`.

The client half -- the daemon performing the migration, unreachable over IPC,
ordered against the void sweep, re-baselining armed folders -- is described
in `docs/contributor-daemon-ipc-v1_1.md`, "Moving a legacy invite identity".

## The rules

1. **Coexistence.** A legacy invite device and its tenant keep working exactly
   as before. Account admission (`TRACE_COMMONS_ACCOUNT_ADMISSION_ENABLED`)
   governs only the `near-`/`nearai-` namespaces; a `tenant-…` request never
   reaches the account ledger, linked or not. Nobody is forced to migrate.
2. **Links are per tenant.** One legacy invite tenant links to one NEAR
   account. A second account claiming an already-linked tenant is refused
   (`legacy_link_tenant_claimed`, HTTP 409) and recorded in
   `trace_legacy_invite_link_conflicts` for you.
3. **Pooled tenants never link.** A pooled tenant is one shared by many
   people through a shared, multi-use invite code -- the Devfolio event
   invites are the case in point. It is pooled only because you marked it, in
   `trace_legacy_invite_pooled_tenants`. Nothing infers it from `max_uses`.
4. **Revocation is respected.** A revoked invite does not carry over: the
   link is refused (`legacy_link_invite_revoked`) when the tenant's
   `onboarding_invites` row, the registry's `onboarding_invite_grants` row, or
   a prior account grant for the same invite is revoked. An invite that has
   expired or run out of uses, but was already redeemed, does carry over.
5. **A second device follows the link only under the linked invite.** A link
   carries the one invite it was made with, and only that invite is granted to
   the account. Another device of the same tenant, signed into the same
   account, gets an attestation of its own under the link (below) only if it
   joined under that same invite. A device that joined the tenant under a
   different invite -- possible wherever several invites route to one tenant
   -- is refused (`legacy_link_invite_not_linked`, HTTP 409) and keeps working
   as a legacy device. Its invite was never granted to the account, so this is
   the rule, not a fault. Each refusal leaves a hash-only
   `legacy_invite_device_attest_refused` audit row (below); it is not a
   conflict and does not block admission.

## What linking does

A contributor signed into a NEAR account (browser session or native session;
never a device bearer) asks for a challenge, has the legacy device key sign a
statement, and submits it:

```
POST /v1/account/invites/legacy-link/challenge   -> { nonce, issued_at, expires_at }
POST /v1/account/invites/legacy-link             -> countersigned record
```

The statement is raw Ed25519 over domain-separated, length-prefixed bytes
(`trace-commons.legacy-invite-link.v1`: legacy tenant, device key id, invite
subject hash, account tenant, account id, nonce, issued-at), verified the same
way the issuer verifies `x-trace-device-signature`. The account half is taken
from the session, never from the body. Encoders live in
`crates/trace-commons-protocol/src/legacy_invite_link.rs`.

On success ingest stores the link and returns it countersigned with the ingest
attestation key (`trace-commons.legacy-invite-link-record.v1`), so the record
can be verified later against the published attestation keyset. The client
needs that record: it is what the daemon checks before it re-baselines.

The database side is `trace_link_legacy_invite`, a SECURITY DEFINER function
owned by the NOLOGIN, NOBYPASSRLS role `trace_legacy_invite_link_guard`. It
requires the signing key to be the legacy tenant's registered, unrevoked,
`onboarding_origin = 'invite'` device, under the named invite, with a matching
`onboarding_invites` row. It then writes the link, a
`trace_account_invite_grants` row from that invite -- **without spending a
use** -- raises the account to `invited` exactly as a redemption would, and
appends a hash-only `legacy_invite_linked` audit row (tenant ids appear only
as `legacy_tenant_hash`). The ingest login needs nothing beyond the
`trace_account_invite_runtime` role it already holds.

Re-linking by the same account from the same device returns the original
record unchanged. Since V91, another of the tenant's invite devices, signed
into the SAME account under the SAME invite (rule 5), gets an attestation of
its own instead: its own statement over a fresh challenge, recorded under the
existing link in `trace_legacy_invite_link_devices` by
`trace_attest_legacy_invite_device`, so it can verify the record against its
own key before its client switches. It is idempotent per device (a repeat
returns the first attestation), changes nothing about the link, the account's
trust or its invite grant, and is audited hash-only as
`legacy_invite_device_attested`. Another account is still refused
(`legacy_link_tenant_claimed`). A challenge is single-use, expires after five
minutes, and an account may hold five open at once.

An attestation is not a link, and says so. Since V104 it is countersigned
under its own domain, `trace-commons.legacy-invite-device-attestation-record.v1`,
and returned with `"kind": "device_attestation"` in the record; its
`link_id` field is the attestation's own id (`trace_legacy_invite_link_devices.attestation_id`),
not a row of `trace_legacy_invite_links`. A link record carries no `kind`,
exactly as before. `countersign_domain` on each attestation row says which
domain its countersignature is under; the only rows under the link domain are
ones V91 wrote before V104, and each is superseded in place by that device's
next attestation.

Since V104 the attestation function checks everything the link function does
on its own, rather than relying on having run second: the account is open and
NEAR-anchored; the statement's challenge was spent by the link function in
the same transaction, is still unexpired, and has not backed another
attestation (the spend is recorded against the device in
`trace_legacy_invite_link_challenges.attested_device_key_id`); the device is
the tenant's live invite device under an invite the tenant redeemed; the
tenant is not pooled; the invite is not revoked anywhere; and the live link is
this account's under this invite.

A refused attestation still spends the challenge, as every refusal after the
spend does. Undoing the spend would mean rolling back the audit row with it,
and the same challenge would only be refused again; the device asks for a new
one.

To see second-device refusals (tenant context = the account's tenant):

```sql
BEGIN;
SELECT set_config('trace_commons.trace_tenant_id', '<account-tenant>', true);
SELECT created_at, safe_metadata->>'legacy_tenant_hash',
       safe_metadata->>'invite_subject_hash',
       safe_metadata->>'linked_invite_subject_hash'
  FROM trace_account_audit
 WHERE action = 'legacy_invite_device_attest_refused'
   AND outcome = 'invite_not_linked';
COMMIT;
```

Both invites appear only as subject hashes, and the tenant only as
`legacy_tenant_hash`. If the device's invite should in fact follow the
account, that is a new decision for the invite's owner, not a repair: nothing
here grants a second invite to a linked account.

Instance-enrolled devices are `invite`-origin in `device_keys` but redeemed no
invite; they have no `onboarding_invites` row and cannot link.

## How a device learns its invite

The statement names the invite the tenant was onboarded under, by its subject
hash. Clients never stored that hash, and most invitees received their code
out of band and no longer have it. So the issuer tells a device its own:

```
POST /v1/device/invite-subject   (issuer)  -> { invite_subject_hash }
```

The body (`tenant_id`, `device_key_id`, `issued_at`) is signed with the
device key over its exact bytes and sent with `x-trace-device-key-id` and
`x-trace-device-signature`, exactly as a device-key upload claim is. The
issuer answers only for that device, from `device_keys`, and refuses by name:
`device_key_not_registered`, `device_key_revoked`,
`device_not_invite_onboarded` (a NEAR or NEAR AI device), and
`device_invite_subject_request_stale` (`issued_at` more than five minutes
from the issuer clock). Without the device registry configured it answers 503.
Each read logs one hash-only line (`device invite subject read`, with the
device as a storage ref and the outcome label). Types are in
`crates/trace-commons-protocol/src/device_invite_subject.rs`.

An issuer older than this route answers 404, and the client falls back to
asking the contributor to paste their invite.

## Enabling linking

Linking is off until you switch it on, and deploying V81 does not switch it
on. Do these in order:

1. **Mark every pooled tenant** (below). A pooled tenant left unmarked can be
   claimed by the first of its members to link.
2. Confirm the ingest attestation key is configured
   (`TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KEY_PEM`, `…_PUBLIC_KEY_PEM`,
   `…_SIGNING_KID`). Linking countersigns with it.
3. Set `TRACE_COMMONS_LEGACY_INVITE_LINK_ENABLED=true` and restart ingest.
   With the switch on and no attestation key, ingest refuses to start
   (`legacy_invite_link_requires_attestation_key`). With it off, both routes
   answer 503 `legacy_invite_link_not_enabled`.

## Marking a pooled tenant

The marker is keyed by tenant and forced under RLS like every other table, so
set the tenant context in the same transaction. Run as the migration owner:

```sql
BEGIN;
SELECT set_config('trace_commons.trace_tenant_id', '<tenant-id>', true);
INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
VALUES ('<tenant-id>', 'shared event invite');
COMMIT;
```

The tenant must already exist (it is created on first redemption). The
`reason_label` is operator text, 1 to 128 characters; keep it free of names.

Candidates to review before enabling linking: every tenant whose invite
`max_uses` is far above the individual shape (3), and every catch-all tenant
that more than one invite routes to. `legacy_invite_tenants|review_before_linking`
in the [counts query](./legacy-invite-counts.md) counts the unmarked ones.
Decide each one; do not mark by rule.

Marking changes nothing for the tenant's devices: they onboard and contribute
exactly as before (`legacy_invite_link_pg` asserts it for a Devfolio-shaped
invite).

## Resolving a conflict

A row in `trace_legacy_invite_link_conflicts` with `resolved_at IS NULL`, for
a tenant that is not pooled, is an **ambiguous identity**: it blocks account
admission at startup (see readiness below). Decide who the tenant belongs to
out of band -- never from a name -- then either:

- the existing link is right: set `resolved_at` on the conflict row;
- the existing link is wrong: revoke it and its grant, then resolve the
  conflict, and let the right account link;
- the tenant is in fact shared: mark it pooled. Pooled tenants never count as
  ambiguous, so the conflict stops blocking.

Revoking a link (tenant context = the linked account's tenant):

```sql
BEGIN;
SELECT set_config('trace_commons.trace_tenant_id', '<account-tenant>', true);
UPDATE trace_legacy_invite_links SET revoked_at = now()
 WHERE legacy_tenant_id = '<legacy-tenant>' AND revoked_at IS NULL
RETURNING link_id;
-- Second-device attestations under that link (V91). They have no revocation
-- of their own and need none: each names the link by link_id, and one under
-- a revoked link attests to nothing. Listed so the record of what was revoked
-- is complete.
SELECT attestation_id, device_key_id
  FROM trace_legacy_invite_link_devices
 WHERE link_id = '<link-id from above>';
UPDATE trace_account_invite_grants SET revoked_at = now()
 WHERE tenant_id = '<account-tenant>' AND account_id = '<account-id>'
   AND invite_subject_hash = '<invite-hash>' AND revoked_at IS NULL;
COMMIT;
```

Nothing carries an attestation over to a later link: attestations are keyed
by `link_id`, so if the tenant is linked again, each further device attests
afresh under the new link.

A revoked grant demotes the account's admission authority as any revoked grant
does, and the same account cannot re-link that invite afterwards.

## Revoking a file invite so it does not carry over

The link function reads the database, not the allowlist file. Removing an
entry from `/etc/tracecommons/allowlist.json` stops new device onboarding but
is invisible here. To also stop a file invite carrying over to an account,
set `revoked_at` on its `onboarding_invites` row (tenant context = that
tenant). Revoking an invite does not revoke devices already onboarded with it.

## Readiness under coexistence

`readiness_blockers` in the [counts query](./legacy-invite-counts.md)
counts each rule below separately. Ingest refuses to start with account
admission on
(`account_admission_permissions_or_linkage_not_ready`) while
`trace_account_admission_linkage_ready()` is false. Since V81 it is false only
when one of these exists:

1. an active device in `near-`/`nearai-` that is not a NEAR or NEAR AI device
   with live account linkage (unchanged from V77);
2. an active device outside those namespaces whose origin is not `invite`;
3. an open account outside those namespaces whose tenant is neither pooled nor
   has any invite-onboarded device -- an identity with no invite behind it;
4. an unresolved conflict on a tenant that is not pooled.

It no longer blocks on, and counts as coexisting: every invite-onboarded
legacy device and its tenant's accounts, linked or not, and every pooled
tenant with its devices and accounts.

Readiness still reads only the durable inventory. Static contributor tokens on
the replica are checked separately at startup and must still resolve to a live
NEAR account; a legacy-namespace static token still blocks.

## Cutover for pooled event invites

When Devfolio (or any event) moves off bulk invites, nothing breaks and
nothing is lost. The steps, in order:

1. **Mint a new shared code in the invite registry**, `tenant_mode =
   'derived'`, `max_uses` set to the number of accounts you want to admit.
   New hackers sign in with a NEAR account and redeem it at
   `POST /v1/account/invites/redeem` (#1016). Each distinct account spends one
   use; an account that already redeemed it spends nothing on a repeat; at
   `max_uses` further accounts are refused (`account_trust_pg`,
   `one_shared_code_admits_many_accounts_up_to_max_uses`). The account path
   reads the registry directly, whichever onboarding path the issuer is on.
   Two things to know: redeeming records invite trust only -- it does not
   onboard a device or apply the invite's consent scopes or allowed uses --
   and if you later set `TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE`, the same
   `derived` code would also onboard devices, each into its own derived
   tenant. Give it an `expires_at` if it should stop by a date.
2. **Stop the old shared codes at a time you choose** by removing their
   entries from the allowlist file (hot-reloads within 60 seconds). Device
   onboarding with them then fails with the usual collapsed `InviteNotValid`.
   The old codes could never be redeemed on the account path: they are not in
   the registry, and an imported copy would be `fixed` to the pooled tenant,
   which no NEAR account matches.
3. **Existing pooled devices keep working.** Upload claims check the device
   key, not the invite, so removing the file entry leaves every onboarded
   device contributing to the pooled tenant as before, until you retire it.
4. **Retire the pooled tenant when you decide to**, by revoking its devices.
   Keep the pooled marker.
5. **History stays where it is.** Nothing is moved, merged or re-attributed:
   submissions, credit and dedup clusters remain under the pooled tenant. A
   pooled tenant cannot be linked, so no single account can claim that
   history.
6. **Each hacker can claim their own account going forward** through step 1.
   What they contributed under the pooled tenant stays pooled.

## Verification

- `legacy_invite_link_pg` (CI, `database suites against a real PostgreSQL`):
  the link function as a login holding only `trace_account_invite_runtime`;
  pooled, claimed, revoked, expired, instance-enrolled and wrong-key cases;
  signature and challenge binding; the runtime's lack of cross-tenant reads;
  every readiness rule above; and the Devfolio-shaped onboarding regression.
  `the_attestation_function_holds_every_refusal_on_its_own` calls
  `trace_attest_legacy_invite_device` directly as that login for each of its
  refusals, including rule 5, a challenge spent in another transaction, one
  spend shared by two devices, and a challenge that expired between the two
  calls; removing any one of the V104 checks listed above turns it red.
- `account_trust_pg`: the shared multi-use account invite.
- `admission_pg_tests` (ingest bin): a legacy invite identity still
  contributes with account admission on.
- `tests/legacy_invite_link_tests.rs` (ingest bin): route session rules, the
  off switch, and the handler pair end to end.
- `device_invite_subject_pg` (CI, `database suites against a real
  PostgreSQL`): a device onboarded through the real `/v1/onboard` reads back
  its stored invite hash, and the named refusals hold against real rows.
