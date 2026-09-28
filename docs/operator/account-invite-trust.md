# Account invite trust foundation

The account invite redemption route records trust for an authenticated,
NEAR-anchored account. This foundation does not activate contribution admission,
folder arming, or a production bounded policy. Existing policy parsing is inert
unless a later activation explicitly supplies reviewed controls.

Redemption trims the code exactly as device onboarding does and requires 16
uppercase ASCII letters or digits. A fixed-tenant invite is usable only within
its configured tenant. A derived invite may elevate the authenticated account
in its existing tenant; redemption does not create or migrate an identity.
Invite policy labels, allowed uses, and consent scopes remain device-onboarding
controls: account trust is a trust signal, not an upload or consent grant.

An already-invited account must still present a known, unrevoked, unexpired,
tenant-compatible invite with an available use (or the same invite it already
redeemed). An unrelated exhausted invite is invalid. It records an idempotency event and returns its
current trust version without spending another use or raising its trust version.
An exact idempotency replay returns the original recorded result; a new key
reads the current version. Only an actual elevation writes the hash-only
`account_invite_redeemed` account audit entry, in the same transaction.

The runtime role's UPDATE permission on `trace_accounts.created_at` exists only
because PostgreSQL requires UPDATE privilege on a column for `SELECT FOR UPDATE`.
The transaction never updates that column, account identity, or account closure.

## Account merge

Since V80, an executed account merge carries invite trust to the surviving
account:

- Every invite grant row of the absorbed account is copied onto the survivor
  with its original invite hash, grant `trust_version`, `granted_at`, and
  `revoked_at`. The absorbed account's rows stay on the closed account.
- A revoked grant never confers trust, and a merge never clears a revocation.
  When both accounts hold a grant from the same invite, a revocation on either
  side wins.
- The survivor's authority is re-derived from its combined grants: `invited`
  with any unrevoked grant, otherwise `bounded` when either account had a trust
  row. This is the invariant admission already enforces, so the survivor keeps
  the higher of the two trusts unless the only grant behind it is revoked.
- An authority change sets the survivor's `trust_version` past both accounts'
  versions, so a reservation priced under the old authority is refused at
  processing. An unchanged authority keeps the survivor's version.
- Redemption idempotency events (`trace_account_trust_events`) stay with the
  absorbed account. The merge audit row records `invite_grants_carried` as a
  count only.

The merge login needs no privilege on the trust tables. The carry-over runs
through `trace_account_trust_merge`, a SECURITY DEFINER function owned by the
NOLOGIN, NOBYPASSRLS `trace_account_trust_merge_guard`. It acts only within
the caller's tenant context and only for a proposal consumed by the same
transaction, the same proof the reward and source-session merge hooks use.
`account_merge_pg` runs it under a merge login that has no trust-table rights.

NEAR anchors and NEAR-provisioned devices follow the identity too (V82):

- Every `trace_near_account_anchors` row of the absorbed account, and every
  `trace_near_provisioned_devices` row provisioned under those anchors, is
  re-keyed onto the survivor. Only `account_id` changes; no row is created or
  deleted. A device provisioned to the absorbed account is then a live device
  of the survivor, and the absorbed account's NEAR account signs in to the
  survivor. Left on the closed account, those devices would stop being live
  and `trace_account_admission_linkage_ready()` would turn false.
- A survivor that already holds an anchor ends up holding both. The same NEAR
  account cannot be anchored on both sides, and the same device cannot be
  provisioned to both: `anchor_hash` and `(tenant_id, device_key_id)` are
  UNIQUE, so the re-key cannot collide.
- Revocation lives on `device_keys`, which the merge does not touch. A revoked
  device moves with its anchor and stays revoked.
- The merge audit row records `near_anchors_carried` and
  `near_devices_carried` as counts only.

This runs through `trace_near_account_merge`, owned by the NOLOGIN,
NOBYPASSRLS `trace_near_account_merge_guard`, under the same tenant check and
consumed-proposal proof. The guard can read only the tenant, account, and
anchor columns and can update only `account_id`; it never reads the sealed
account name. `account_merge_pg` runs it under the same restricted merge login.

## Activation blockers

Two separate identity changes remain required before activating trust-based
admission or folder arming (the merge half of the second is covered above):

- Legacy device invitees: the server half is V81
  ([legacy invite migration](./legacy-invite-migration.md)). A contributor
  links their legacy invite tenant to their NEAR account with a statement
  signed by the legacy device key; the link carries the tenant's invite onto
  the account without spending a use, is countersigned by ingest, and is
  refused for pooled tenants and revoked invites. Client grant-identity
  re-baselining is still a separate contributor PR.
- Account merge: trust, grants, and trust events are handled by V80, and NEAR
  anchors and provisioned devices by V82, as described under "Account merge"
  above, with real PostgreSQL coverage of authorization, versions,
  revocation, device liveness, readiness, and audit. No merge blocker
  remains.

V75 permits only `invited` trust. The future account-admission V77 migration
broadens that constraint to `bounded`; bounded-to-invited promotion must be
verified against the actual combined migrations in that integration slice.
The V75-only test deliberately does not ALTER the schema to simulate V77.
Neither this migration dependency nor the two identity blockers is an approval
to activate a policy.

## Verification

CI runs `account_trust_pg` against a dedicated database under a non-superuser,
non-BYPASSRLS runtime role. The account invite HTTP fixture uses another database
and proves real session success, code trimming, strict body fields, cross-origin
403, and session rotation. The account resolver rejects device bearers with 401
before handler dispatch; an injected device context independently tests the
handler's defensive 403 and its exact safe refusal label.
