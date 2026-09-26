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

## Activation blockers

Two separate identity changes remain required before activating trust-based
admission or folder arming:

- Legacy device invitees need a reviewed migration from
  `device_keys.invite_subject_hash` to the authenticated NEAR account. It must
  preserve a legitimate spent single-use invitation without spending it again,
  establish an authenticated identity link, and coordinate client grant-identity
  re-baselining. This PR does not invent that mapping or transfer authority.
- Account merge needs reviewed transfer and conflict rules for
  `trace_account_trust`, `trace_account_invite_grants`, trust events, and
  `trace_near_account_anchors`. The current merge does not transfer these rows;
  an absorbed invited account does not confer trust on its surviving account.
  Authorization, tenant boundaries, versions, audit, and idempotency must be
  covered by real PostgreSQL tests before activation.

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
