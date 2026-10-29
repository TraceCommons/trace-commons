# Referral rewards -- design

Date: 2026-10-29. Owner: Zaki. Status: schema built (V142, PR 1 of 5);
nothing else is built. Working copy for discussion:
the "Trace Commons referral rewards" Claude Doc. This file is the
authority for the build.

Builds on the NEAR AI payout rail,
[`2026-10-21-nearai-credit-payout-design.md`](2026-10-21-nearai-credit-payout-design.md)
("the payout design"; decisions D1-D9), and sits beside the starter advance,
[`2026-10-21-nearai-starter-advance-design.md`](2026-10-21-nearai-starter-advance-design.md).

## Owner decisions

- **(R0) The ask.** 2026-10-29: "people can create referral codes, referral
  codes can be entered when people join or after and anyone that joins with
  your referral codes you get 10% of the near ai credits settlement in your
  account for everyone who joins for at least 3 months post launch."
- **(R1) Earning period.** 2026-10-29, chosen on a decision card: the bonus
  covers **each referee's first 3 months**. Then: "3 months promised but it
  can be extended." So the period is a setting with a floor of 92 days that
  an operator can raise and never lower.
- **(R7) Own traces vs referrals.** 2026-10-29: "It would interesting if the
  settlement system also could enable showing in the app how much users got
  for their own traces vs referrals." Section 5.
- **(R8) Abuse per code.** 2026-10-29: "Referral codes that are associated
  with a lot of abuse also needs to be detected." Section 7.

Defaults the owner has not overridden (section 9 lists them): the bonus is
on top and funded by the commons (R2); a launch-time setting opens the
program (R3); budget caps (R4); a 30-day late-entry window (R5); app copy
needs approval (R6, CLAUDE.md "no new notices without explicit approval").

## 1. Summary

- Any open account can create up to 5 referral codes.
- A new account can apply one code: at sign-up, or within 30 days of its
  account's creation. Once applied it never changes.
- The referrer earns `bps` (default 1000 = 10%) of every `near_ai` credit
  accrual the referee earns from batches created while the referral is in
  force: after it was applied, and before `referee_account_created_at +
  TRACE_COMMONS_REFERRAL_EARN_DAYS` (default and minimum 92).
- The bonus is on top. The referee's own accruals, lines and payouts are
  never read back or reduced.
- Bonus is paid on a second cumulative program on the referrer's existing
  `near_ai` link (stream `referral`), from a dedicated #1271 credential, so
  own and referral credit stay separate end to end.
- Trace Credit is untouched (D6). The starter advance does not count toward
  the base, and referral bonus never absorbs an advance.

## 2. Codes

- **Format.** 10 random Crockford base32 characters (50 bits), shown as
  `TC-XXXXX-XXXXX`. Input is normalized: uppercase, `-` and spaces removed,
  `I`/`L` read as `1`, `O` as `0`, and the `TC` prefix optional. Server-made
  only; no vanity codes.
- **Storage.** `trace_referral_codes` in the referrer's tenant.
  `code_index` = `sha256:` of HMAC-SHA256(pepper, `"tc-referral-code-v1:" ||
  normalized`), keyed by the same pepper machinery as the payout recipient
  index (`payout_recipient_seal.rs`), unique across every tenant. The code is
  sealed (`sealed_code`) so its owner can list it again.
- **Limits.** At most 5 live codes per account (checked in the store under
  an advisory lock on the account). A live code has none of `disabled_at`,
  `suspended_at`, `voided_at` set.
- **Disable.** The owner turns a code off: it stops new referrals; existing
  referrals keep earning.

## 3. Applying a code

Order of checks, one refusal label each. Every refusal returns the same
outward label, `referral_not_applied`, to the client; the precise label goes
only to `trace_referral_audit` (`referral_refused`), so codes cannot be
probed.

1. `referral_unavailable`: referral mode `disabled`, or the program has not
   started (`TRACE_COMMONS_REFERRAL_PROGRAM_START`, RFC 3339; unset means not
   started), or it has ended (`TRACE_COMMONS_REFERRAL_PROGRAM_END`, optional).
2. `referral_rate_limited`: more than 10 refused attempts by this account in
   24 hours.
3. `referral_code_unknown`: no live code with this index (resolved through
   `trace_referral_resolver`).
4. `referral_already_referred`: the account has an attribution.
5. `referral_account_too_old`: the account was created before the program
   start, or more than `TRACE_COMMONS_REFERRAL_LATE_DAYS` (default 30) ago.
6. `referral_self`: same tenant, same account, same NEAR AI subject or NEAR
   wallet anchor (`trace_near_account_anchors.anchor_hash`), or a device key
   enrolled on both accounts.

On success, one transaction writes the attribution under the referee's
tenant context and the `trace_referral_referees` mirror under the
referrer's, re-setting `trace_commons.trace_tenant_id` between statements,
plus a `referral_applied` audit row in each tenant.

`referee_ref_hash` = `sha256:` of `"tc-referral-referee-v1:" || tenant_id
|| ":" || account_id`. It is the only name a referrer's rows hold for a
referee.

**At sign-up.** The NEAR AI and wallet provisioning requests gain an
optional `referral_code`. Provisioning never fails because of it: the
account is created, and the response carries `referral: "applied"` or
`referral: "not_applied"`. The code is applied after the provisioning
transaction commits, with `applied_via = 'signup'`.

**Later.** `POST /v1/account/referral` with `{ "code": "..." }`,
`applied_via = 'later'`.

## 4. The bonus

### Base and price

For each referee `trace_payout_accruals` row on a rail whose
`settlement_batch_id` names a batch created at or after `applied_at` and
before `earn_until`, and whose referral is not voided:

- `kind = 'credit'` gives a `bonus` row: `base_units = rail_units`,
  `bonus_units = floor(rail_units * bps / 10000)`.
- `kind = 'reversal'` gives a `reversal` row for the same amount negated,
  but only when its credit produced a bonus row.
- `bps` comes from `TRACE_COMMONS_REFERRAL_BONUS_BPS` (default 1000, range
  1-10000) at the time of writing and is stored on the row, so a later
  change never re-prices one. `rate_version` and `instrument_id` are copied
  from the referee accrual.
- An unowned accrual (bare principal) counts once claimed, if its batch is
  in the window.

`earn_until = referee_account_created_at + TRACE_COMMONS_REFERRAL_EARN_DAYS
days`. It is computed when read, so raising the setting extends every live
referral without a data change. Boot refuses a value below 92
(`referral_earn_days_below_promise`).

Rows are written with `ON CONFLICT DO NOTHING` on the unique source index,
so the pass is idempotent.

### Where it runs

In each `near_ai` pass, after step 1a (accrue) for a tenant, the pass reads
that tenant's attribution (if any) and its accruals not yet mirrored, and
writes the bonus rows in a second transaction under the referrer's tenant.
A crash between the two transactions re-runs on the next pass. A missing
referrer tenant or account voids the attribution (`referral_referrer_gone`).

The pass also refreshes, under the referrer's tenant, the mirror row's
`signal_labels`, `recipient_index` and `subject_index` (section 7).

### Holds on the referral stream

Recorded in `trace_payout_holds` (rail `near_ai`) on the referrer's account,
holding only the referral stream:

- `referral_same_payout_org`: the referee's live link `recipient_index`
  equals the referrer's.
- `referral_same_payout_subject`: same `subject_index`.
- `referral_code_suspended`: the code that referred them is suspended.
- `referral_over_ceiling`: the referrer's referral owed exceeds
  `TRACE_COMMONS_REFERRAL_ACCOUNT_CEILING_NANO` (default $250).
- The account holds of the payout design (`payout_account_credit_hold`,
  `payout_account_closed`) apply to the referral stream as well.

A hold on the referee's own account (credit hold, closed) stops new bonus
rows from that referee until it clears.

## 5. Paying it: the referral stream

- Each link may carry a second program, `referral_program` =
  `"tr1-"` + first 32 hex of `sha256("nearai-referral-program-v1:" ||
  link_id)`, set when its first referral line is written.
- `trace_payout_lines.stream` is `earned` (default; everything before V142)
  or `referral`. One pending line per `(link_id, stream)`.
- `target_referral = owed_referral - sum(final_paid_referral of closed
  links)`, capped per send by the referral credential's
  `max_amount_per_transfer`, where `owed_referral = sum(bonus_units)` of the
  account's bonus rows on the rail.
- Sends use a dedicated credential, `TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL`,
  whose `max_total_amount` is the program's lifetime budget (default $2,500),
  enforced by cloud-api.
- Mode `TRACE_COMMONS_NEAR_AI_REFERRAL_MODE` = `disabled` | `dry_run` |
  `live`, default `disabled`. `live` needs the rail itself `live` and the
  credential configured, or boot refuses (`near_ai_referral_config_missing`).
  The rail's halt stops the referral stream too.
- Crash recovery, receipt lookup and the reconciliation drill run per stream,
  filtering transfer history by that stream's program.
- Closing a link reads both programs and records `final_paid` and, when the
  link has a referral program, `final_paid_referral`.
- The starter advance's `max(owed, advance)` applies to the earned stream
  only.

### Own traces vs referrals in the app (R7)

`PayoutRailLinkStatus` gains `streams: [{ stream, owed, sent, waiting,
hold_label? }]` in nano-USD, where `owed` is from accruals (earned) or bonus
rows (referral), `sent` sums `confirmed` lines of that stream, and `waiting
= owed - sent` floored at zero. A referral status route adds the count of
referees per code. The referee sees only that a code was applied.

## 6. Routes and app

Signed-in account session, as the payout link routes are
(`payout_link_routes` in ingest). Request and response types in
`trace-commons-protocol/src/referral.rs`.

| Route | Does |
| --- | --- |
| `POST /v1/account/referral-codes` | create a code; returns it once with its id |
| `GET /v1/account/referral-codes` | your codes (code text, state, referee count, bonus owed and sent) |
| `POST /v1/account/referral-codes/{code_id}/disable` | turn one off |
| `POST /v1/account/referral` | apply a code |
| `GET /v1/account/referral` | whether one is applied, and `earn_until` |
| `GET /v1/admin/referrals/suspended` | payout control token only: suspended codes by code index, signals, amounts held |
| `POST /v1/admin/referrals/codes/clear` | clear a suspension: `{tenant_id, code_index, exempt_labels?}` |
| `POST /v1/admin/referrals/codes/void` | void a code and its referees' unsent bonus |
| `POST /v1/admin/referrals/void` | void one referral: `{tenant_id, account_id_hash}` of the referee |
| `POST /v1/admin/referrals/dry-run-report` | counts by state and signal, total bonus owed, sendable and held |

App, macOS first: a Settings "Referrals" section (codes with Copy and Turn
off, people joined, credit from referrals; a field to enter a code while the
30 days last), an optional code field on the NEAR AI sign-in step, and the
own/referral split on the Insights NEAR AI credit card. Daemon IPC:
`referral.codes.list`, `referral.codes.create`, `referral.codes.disable`,
`referral.apply`, `referral.status`. All copy is drafted and marked for
owner review; no banner, notification or one-time message is added.

## 7. Detecting abusive codes (R8)

Each code is scored over its referees, from the mirror rows in the
referrer's tenant, once it has at least 5 referees. The pass writes each
referee's labels when it visits the referee's tenant; the code check runs in
the referrer's tenant.

| Signal label | Referee fact | Code trips when |
| --- | --- | --- |
| `referral_refused_share` | most of the referee's submissions refused by scoring or the gates | over 40% of referees |
| `referral_credit_hold_share` | open credit hold or an operator reversal | over 20% |
| `referral_advance_forfeit_share` | starter advance forfeited (once advances exist) | over 20% |
| `referral_shared_identity` | referee shares `recipient_index`, `subject_index` or a device key with another referee of the same code | 3 or more referees in one cluster |
| `referral_duplicate_traces` | referee's traces duplicate another referee's (content hash or simhash) | over 20% |
| `referral_burst` | applied times | more than 20 referees in 24 hours |
| `referral_never_settles` | uploads but no settled credit 30 days after joining | over 60% |

A tripped code is suspended with its labels, and every bonus from its
referees is held `referral_code_suspended`. The same rates over all of a
referrer's codes trip a referrer-level hold, `referral_referrer_suspended`.
More than 2 codes suspended in 24 hours switches referral mode to `dry_run`
by itself, with an audit row, and readiness reports `referral_tripped`.
An operator clears (optionally exempting a label) or voids. A void writes
one `void` bonus row per affected referee taking back their unsent bonus.
Thresholds are settings, tuned from the dry-run report before `live`.

## 8. Delivery plan

1. **Spec and schema** (this PR): V142, the resolver role, the RLS registry,
   `referral_rewards_pg`.
2. **Codes and apply**: code generation and sealing, the store, the routes,
   the sign-up field, self-referral checks, rate limit, protocol types.
3. **Bonus accrual and abuse checks**: the cross-tenant bonus write, holds,
   referee signals, code scoring and suspension, admin routes, dry-run
   report. Mode `dry_run`.
4. **Referral stream sends**: the second program, lines per stream, the
   dedicated credential, per-stream recovery, close and drill; status
   streams.
5. **App**: daemon IPC, macOS Settings section, sign-up field, Insights
   split.

## 9. Defaults the owner may override

- R2: bonus on top, from the commons.
- R3: the program starts at `TRACE_COMMONS_REFERRAL_PROGRAM_START`.
- R4: $2,500 lifetime on the referral credential; $250 per referrer.
- R5: a code may be applied up to 30 days after account creation, and earns
  only on batches created after it was applied.
- R6: all app copy.
