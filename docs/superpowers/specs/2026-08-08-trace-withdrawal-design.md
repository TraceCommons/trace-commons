# Trace withdrawal — design

Date: 2026-08-08
Status: approved for planning
Scope: sub-project 1.6. Server + client. The first server-side work in this effort.

## Why

The consent prompt already tells contributors they can revoke submitted
traces. There is no server endpoint, no daemon method, and no UI. The promise
is currently false.

It is also the only honest exit from quarantine. Forty-eight pilot traces are
held for operator privacy review and none has ever been reviewed. A
contributor in that position today has no recourse and no explanation, and
concludes either that the operator is ignoring them or that their work was
judged unfit. The first is true. Withdrawal is both the ethical answer and the
pressure valve that stops forty-eight becoming four hundred.

## What withdrawal means

**Withdrawal is a request to stop using a trace and to delete its content.**
It is not a claim that every derived artifact can be unmade. The UI must not
imply otherwise.

Three tiers, and the contributor sees which applies:

| Trace state | What withdrawal does |
|---|---|
| `submitted`, `quarantined` — not yet in the commons | Content deleted. Nothing was distributed. Complete. |
| `accepted` — in the commons, not yet used downstream | Content deleted, trace excluded from future exports and training sets. |
| `accepted` and already included in a published export or benchmark | Content deleted and excluded going forward. Copies already distributed cannot be recalled, and the UI says exactly that. |

Credit already awarded is **not** clawed back. Withdrawal is not a punishment
and treating it as one would deter honest use.

## Server

`POST /v1/account/traces/{submission_id}/withdraw`

Authenticated by the account session, the same auth that already guards
`/v1/account/traces/{submission_id}/content`. Not the device key: withdrawal
is an account-level act and should survive losing a device.

- Tenant-scoped through the existing RLS predicate. A contributor can withdraw
  only their own traces; there is no cross-tenant path.
- Idempotent. Withdrawing twice is a success, not an error.
- Deletes the stored content and the encrypted artifact, retaining a hash-only
  tombstone row: `submission_id`, `withdrawn_at`, prior status, and a
  `distribution_reach` label recording which tier applied. Audit stays
  hash-only, per repo convention.
- Response reports the tier that applied, so the client can tell the
  contributor the truth rather than a generic success.

A new migration adds the tombstone table and a `withdrawn_at` column. It must
be wired into the hand-rolled `run_migrations` — this repo does not discover
migrations automatically.

**Retention interaction:** withdrawal must also remove the trace from any
vector index and dedup cluster it participates in, or the content survives in
derived form. That is the part most likely to be missed.

## Daemon

| Method | Behaviour |
|---|---|
| `withdraw {submission_id}` | Calls the endpoint, updates the history cache, returns the tier that applied. |
| `withdraw_bulk {status}` | Withdraws everything in a status, for the realistic "take back all my quarantined traces" case. |

Withdrawal requires an account session, which the daemon does not currently
hold — it holds a device key. The daemon returns a distinct
`account-session-required` error, and the shells route the contributor
through account sign-in rather than showing a bare failure.

## UI

In History, per row and as a bulk action on the quarantine group.

Confirmation copy, tier-aware. For a quarantined trace:

> **Withdraw this trace?**
>
> It is waiting for privacy review and has not entered the commons. Its
> content will be deleted. No one but a reviewer has seen it.
>
> Credit already recorded stays.
>
> [ Keep it ]  [ Withdraw ]

For an accepted trace already included in a published export:

> **Withdraw this trace?**
>
> Its content will be deleted and it will be excluded from future exports and
> training sets.
>
> **It was included in an export published on 12 June.** Copies already
> distributed cannot be recalled. We cannot undo that and will not pretend
> otherwise.
>
> Credit already recorded stays.
>
> [ Keep it ]  [ Withdraw anyway ]

## Testing

- Endpoint: own trace succeeds; another tenant's trace 404s rather than 403s,
  so existence is not disclosed; twice is idempotent; content is genuinely
  gone afterwards; the tombstone carries no content, path, or identity.
- Each of the three tiers returns its correct label.
- Vector index and dedup cluster no longer return the withdrawn trace.
- Daemon: history cache reflects withdrawal without a refresh; a daemon with
  no account session reports `account-session-required` and not a generic
  failure.

## Out of scope

- Withdrawing on someone else's behalf, including operator-initiated removal.
- Any change to how credit is scored.
