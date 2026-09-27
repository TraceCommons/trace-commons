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

Still not transferred: `trace_near_account_anchors` and
`trace_near_provisioned_devices` rows stay on the absorbed account, so a NEAR
device provisioned to it is not a live device of the survivor. That is a
separate identity rule and is not decided here.

## Activation blockers

Two separate identity changes remain required before activating trust-based
admission or folder arming (the merge half of the second is covered above):

- Legacy device invitees need a reviewed migration from
  `device_keys.invite_subject_hash` to the authenticated NEAR account. It must
  preserve a legitimate spent single-use invitation without spending it again,
  establish an authenticated identity link, and coordinate client grant-identity
  re-baselining. This PR does not invent that mapping or transfer authority.
- Account merge: trust, grants, and trust events are handled by V80 as
  described under "Account merge" above, with real PostgreSQL coverage of
  authorization, versions, revocation, and audit. The NEAR anchor and
  provisioned-device rows are still not transferred and still need a reviewed
  rule before activation.

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
