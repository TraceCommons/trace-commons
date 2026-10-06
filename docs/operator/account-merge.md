# Account merge (device consolidation)

Slice 3b lets a contributor fold a second device-rooted account into their
primary account so that all credit accrues to one identity. This runbook
describes the two-step flow, its irreversibility, and the operator-visible
audit surface. It is contributor-driven (no operator action is required to
perform a merge); the operator's job is to understand what the flow does and
how to read its audit trail when investigating a dispute.

> Terminology: **A** is the **surviving** account (the one the contributor keeps
> and is authenticated as). **B** is the **absorbed** account (the one being
> folded in and closed). The merge moves B into A.

## The two-step flow

Merge is a stage-then-execute handshake. Both steps run on the contributor's
own session against account A; account B is proven by possession of a single-use
login link, not by any operator grant.

1. **`POST /v1/account/merge/start`** — the caller (authenticated as A) submits
   device B's **login-link code** as proof of control over B. The handler
   resolves and **consumes** B's login link (single-use), confirms B is a
   distinct, open account in the same tenant, and stages a durable, time-bounded
   **merge proposal** (`trace_account_merge_proposals`). It returns only an
   opaque `proposal_id` — never a code, account id, or any B-side secret.

2. **`POST /v1/account/merge/confirm`** — the caller submits the durable
   `proposal_id`. This step is **strong-auth-gated**: it requires a
   strong-authenticated session on A (a weak/login-only session is rejected
   `403`). The handler re-validates the proposal (unexpired, unconsumed, owned by
   A, B still open, A still open) and then atomically executes the merge.

The split exists so that the proof-of-control (consuming B's link) and the
irreversible mutation (executing the merge) are separate, and the irreversible
half sits behind the strong-auth gate.

## What execute does (atomic, irreversible)

`execute_merge` runs as **one tenant-scoped transaction** — a partial merge is
never observable. It:

- moves B's **principals** (device identities) to A;
- moves B's **WebAuthn authenticators** to A;
- **clears B's payout designation** (a single account may designate at most one
  active payout NEAR identity; B's stale designation must not survive);
- moves B's **NEAR account anchors and NEAR-provisioned devices** to A, so a
  device provisioned to B is a live device of A and B's NEAR account signs in
  to A. Revoked devices move and stay revoked. This runs through V82's
  `trace_near_account_merge`; the rule is stated in
  [`account-invite-trust.md`](./account-invite-trust.md#account-merge);
- **revokes B's sessions** (B can no longer be used);
- **closes B** (B is a closed account from here on; see
  [`pilot-contributor-onboarding.md`](./pilot-contributor-onboarding.md) for what
  closed-account gating means — closed accounts resolve no session).

B's **credit** is not copied or rewritten row-by-row. Credit attributes through
**dynamic principal→account resolution**: once B's principals belong to A, B's
historical credit resolves to A on read and at settlement time. There is no
backfill step and no second source of truth to keep in sync.

**This is irreversible.** There is no `unmerge`. B is closed, B's principals and
authenticators now belong to A, and B's credit re-attributes to A. Treat a merge
confirm as a one-way door.

## The abandoned-start gotcha: a start BURNS device B's link

`POST /v1/account/merge/start` **consumes** device B's login link as part of
staging the proposal. The link is **single-use**. If the contributor calls
`start` and then never calls `confirm` (abandons the flow, or the proposal
expires), **B's login link is already burned** — it cannot be reused to retry.

To retry after an abandoned or expired start, the contributor must **re-mint a
fresh login link for device B** and call `start` again with the new code. This
is by design: the link is the proof-of-control token, and a consumed token must
not be replayable.

## Strong-auth gate

`merge/confirm` requires a strong-authenticated session. A login-only / weak
session is rejected `403` before any mutation. This matches the gate on other
irreversible account-mutation surfaces (e.g. payout designation). Rejections are
**uniform**: invalid code, expired/consumed link, self-merge (B == A), a
foreign/unknown/double-spent proposal, and a closed surviving or absorbed
account all reject without leaking which condition tripped (no existence
oracle).

## Completing a pending withdrawal after merge

A successful merge response can include `withdrawal_completion_pending: true`.
The merge has committed and its proposal is consumed; do not retry merge
confirm. The remaining source-session withdrawal work must run for the
**affected tenant**. The in-process revocation-propagation scheduler authenticates
with one token and covers only that token's tenant. Other contributor tenants
need their own worker invocation or external per-tenant schedule.

Scrape `GET /v1/admin/operational-metrics` with an admin credential for each
tenant and alert when
`trace_commons_operational_withdrawal_completion_pending == 1`. Its only label
is the hashed `tenant_storage_ref`. This gauge is re-derived from persisted
incomplete source-session withdrawals, so a process restart or a worker run
for another tenant does not clear it. A database read error fails the scrape;
alert on failed/missing scrapes too, rather than interpreting them as zero.

1. Use a `revocation_worker` (or admin) bearer credential authenticated to the
   affected tenant. A contributor account-session credential is insufficient;
   a worker token for the deployment's default tenant cannot recover another
   tenant. Provision the appropriate tenant-scoped worker credential through
   the deployment's normal credential management if needed.
2. Call `POST /v1/workers/revocation-propagation` with that credential and
   `{"purpose":"complete pending merge withdrawals","dry_run":false,"limit":100}`.
   Inspect `withdrawal_completions_checked`, `withdrawal_completions_completed`,
   and `withdrawal_completions_failed`. Repair the failing gate, storage, or DB
   dependency when failures remain, and rerun this idempotent worker.
3. Continue bounded runs until `withdrawal_completions_checked` is zero and
   confirm the affected tenant's operational gauge is zero. A full batch can
   leave more work; a zero count for another tenant proves nothing about this
   merge. Keep the incident open while the gauge is one or the scrape fails.

The signal covers source-session-mapped withdrawals, including merge fan-out;
it does not claim reconciliation for unmapped route withdrawals.

## Audit surface (hash-only / label-only)

Two audit events bracket the flow. Both are **hash-only / label-only** — they
carry no principal refs, public keys, account UUIDs, codes, or contributor
identity in their metadata:

- **`account_merge_started`** — written by `merge/start` when a proposal is
  staged.
- **`account_merged`** — written by `merge/confirm` when `execute_merge`
  commits.

When investigating a consolidation dispute, correlate these two labels within
the tenant's audit chain by timestamp. You will not find the account ids in the
rows — that is intentional. The proposal row in
`trace_account_merge_proposals` carries an attribution-only
`absorbed_principal_count` for operator review, but no PII.

## Migration note: V34 was edited during development

The `V34__account_consolidation.sql` migration (which creates
`trace_account_merge_proposals`, adds the payout-designation column + partial
unique index to `trace_near_identities`, and adds `payout_near_account_id` to
the settlement outbox) was **edited after an earlier draft was applied during
development** — the `absorbed_principal_count` column was widened to `BIGINT`.

`refinery` checksums applied migrations and **will not re-run a migration whose
file changed after it was applied**; it will instead error on a checksum
mismatch. Any **dev or staging** database that applied an *earlier* V34 must be
**recreated** (drop + re-migrate) to pick up the current V34.

This is **moot for production** — no database has applied V34 yet — but note it
for any dev/staging DB that may have an earlier copy.

## See also

- [`pilot-contributor-onboarding.md`](./pilot-contributor-onboarding.md) — the
  login-link mint flow and contributor session model.
- [`login-resolver-role.md`](./login-resolver-role.md) — the least-privilege
  role behind login-link resolution.
- [`settlement-mode.md`](./settlement-mode.md) — how merged/consolidated credit
  reaches (or is held off) the chain, and payout designation.
- [`audit-trail-forensics.md`](./audit-trail-forensics.md) — reading the audit
  chain when investigating a dispute.
