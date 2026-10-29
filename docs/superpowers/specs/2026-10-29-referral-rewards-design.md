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
- **(R9) Server-observed signals only.** 2026-10-29, chosen on a decision
  card ("Server-observed signals"): the client is open source, so nothing
  it sends counts as abuse evidence. A self-referral (one person, two NEAR
  AI logins) is caught only by what the server observes: a shared payout
  org or subject, near-duplicate traces, or a shared network prefix. A
  match holds the referral stream; it never voids and never pays by
  itself. Section 7, "Client is untrusted".

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
   `trace_referral_resolver`, which V142 grants to no one; the operator
   grants it to the ingest login, `docs/operator/referral-resolver-role.md`,
   and ingest refuses to start with the program or bonus on until it can
   assume it, `referral_resolver_unprovisioned`).
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

A commons from before this change parses the finish with
`deny_unknown_fields`, so it answers a finish carrying `referral_code` with
its uniform deny (400, `native sign-in could not be completed`), before it
takes the ceremony or spends the token. The daemon then sends the finish
once more without the field, on the same ceremony and token, and answers
`referral: "not_applied"`; the code is dropped, since that commons has no
`POST /v1/account/referral` either. Only that deny, and only for a finish
that carried a code, is retried, and only once: a real refusal already
took the ceremony, so its retry is refused the same way.

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
days`, stamped on the attribution when the code is applied (V142 checks it
is at least 92 days after creation). The period in force is the later of
the stamped end and the end under the current setting, so raising the
setting extends every live referral without a data change, and lowering it
affects only referrals applied afterwards: a promise already made is never
shortened. Boot refuses a value below 92
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

It also compares, for each referee, two server-observed facts about the
referee with the same facts about the referrer, each read under its own
side's tenant and compared only in the pass (section 7, "Client is
untrusted"):

- **Traces.** The distinct `trace_gate_decisions.dedup_cluster_id` values
  of each side's own submissions (through `trace_account_owned_principals`),
  the most recent 500 received since 30 days before the referee's account
  was created. Clusters come from the cross-tenant simhash scan
  (`dedup_assign`), so one cluster id means near-duplicate content in any
  tenant. V142 grants the runtime `SELECT (tenant_id, submission_id,
  dedup_cluster_id)` on `trace_gate_decisions` for this read and no other
  column.
- **Network.** Each side's `trace_referral_network_marks`, the 256 most
  recently seen within retention.

### Holds on the referral stream

Recorded in `trace_payout_holds` (rail `near_ai`) on the referrer's account,
holding only the referral stream:

- `referral_same_payout_org`: the referee's live link `recipient_index`
  equals the referrer's.
- `referral_same_payout_subject`: same `subject_index`.
- `referral_same_traces`: one of the referee's traces is in the same
  near-duplicate cluster as one of the referrer's. The referee's mirror row
  carries `referral_duplicate_traces`, which counts toward the code's score.
- `referral_same_network`: the referee and the referrer share a network
  mark. The mirror row carries `referral_same_network` as evidence for the
  operator; it never counts toward a code's or a referrer's score.
- `referral_code_suspended`: the code that referred them is suspended.
- `referral_over_ceiling`: the referrer's referral owed exceeds
  `TRACE_COMMONS_REFERRAL_ACCOUNT_CEILING_NANO` (default $250).
- The account holds of the payout design (`payout_account_credit_hold`,
  `payout_account_closed`) apply to the referral stream as well.

A hold on the referee's own account (credit hold, closed) stops new bonus
rows from that referee until it clears. So does any open hold on the
referee's own earned stream on the rail (a rail hold such as
`payout_reversal_unpriced`): while one is open no bonus is planned from the
referee's accruals; they wait, are not dropped, and are priced once it
clears. A referral-stream hold on the referee (as someone's referrer) does
not pause it.

Each referee's step writes its bonus rows last, after the holds its
evidence opens (payout org, subject, traces, network, the referrer-level
hold, a code suspension, and the ceiling when the planned bonus would cross
it). A store fault before then writes no rows, and the next pass retries
the whole step, so new bonus is never payable without its hold.

`referral_same_payout_org`, `referral_same_payout_subject`,
`referral_same_traces` and `referral_same_network` open when a referee
first shows the match (its mirror row does not yet carry the label:
`referral_duplicate_traces` for traces, the hold's own label otherwise), so
an operator who clears one is not overruled by the next pass; another
referee with the same match opens it again. The payout org, subject and
network labels on the mirror are evidence only and never scored. Every referral hold is cleared through
the payout hold route (`POST .../holds/clear`) or ended by a void through
the referral admin routes (section 6).

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
- The pass prices a line from a read made before its rail call, and an
  operator void can commit in between. So a referral line's insert takes
  the same per-account lock (`referral-bonus-v1:`) as bonus writes and
  voids, and, under it, refuses a line whose delta is more than
  `owed_referral` less every confirmed or pending referral delta. Nothing
  is written, the pass counts `referral_owed_changed`, and the next pass
  prices from the new amount.
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
= owed - sent` floored at zero. Each amount is a decimal string, so a u128
survives JSON; the daemon passes the streams through unchanged, and an app
reads an amount it cannot hold (past its integer width, or not a whole
number) as unknown and draws nothing for it, never zero. The fixture
`tests/fixtures/payout/streams.json` is read by both the daemon's test and
the macOS app's. A referral status route adds the count of
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
| `referral_refused_share` | most of the referee's submissions refused by scoring or the gates, once the referee has made at least 5 (`TRACE_COMMONS_REFERRAL_ABUSE_REFUSED_MIN_SUBMISSIONS`, 1 to 100000) | over 40% of referees |
| `referral_credit_hold_share` | open credit hold or an operator reversal | over 20% |
| `referral_advance_forfeit_share` | starter advance forfeited (once advances exist) | over 20% |
| `referral_shared_identity` | referee shares `recipient_index` or `subject_index` with another referee of the same code | 3 or more referees in one cluster |
| `referral_duplicate_traces` | one of the referee's traces is in the same near-duplicate (simhash) cluster as one of the referrer's | over 20% |
| `referral_same_network` | referee and referrer share a network mark | never: evidence only |
| `referral_same_payout_org`, `referral_same_payout_subject` | referee's live link has the referrer's `recipient_index` or `subject_index` | never: evidence only (the hold does the work) |
| `referral_burst` | applied times | more than 20 referees in 24 hours |
| `referral_never_settles` | uploads but no settled credit 30 days after joining | over 60% |

A tripped code is suspended with its labels, and every bonus from its
referees is held `referral_code_suspended`. The same rates over all of a
referrer's codes trip a referrer-level hold, `referral_referrer_suspended`.
Like the server-observed holds, it opens on new evidence, so an operator's
clear stands over the same referees: when a referee's labels are written
(or first written), the referrer is scored with them and without, and the
hold opens only if the referrer trips with them and a tripped label is new
or is carried by more referees than before. A label an operator exempted
when clearing a code is dropped from that code's referees in this rollup;
`referral_shared_identity` and `referral_burst`, which no one referee
carries, are exempt in it only when every code with a live referee exempts
them.
More than 2 codes suspended in 24 hours switches referral mode to `dry_run`
by itself, with an audit row, and readiness reports `referral_tripped`.
The switch is the halt of the pseudo-rail `near_ai.referral`, and the
operator lifts it with `POST /v1/admin/payout-rails/near_ai.referral/resume`
(or sets it by hand with `.../halt`): the payout control token, and a
control row with the operator's hash, as for a rail. The route is unknown
while the referral stream is not attached.

A referee's refused share counts only past a minimum of their own
submissions because a code is public: a stranger who signs up with it and
uploads one bad trace is not evidence against its owner.

An operator clears (optionally exempting a label) or voids. A void writes
one `void` bonus row per affected referee taking back their unsent bonus.
The referral stream sends one amount per account, so what was sent is
allotted to the account's referees in the order each first earned, each
up to their net bonus; a referee's unsent bonus is their net less that
share. A void never takes back what was sent and never another
referee's unsent bonus.
Thresholds are settings, tuned from the dry-run report before `live`.

### Client is untrusted (R9)

The contributor client is open source, so anything it sends can be forged:
no device hash, install id or client-reported field is abuse evidence, and
none is collected for it. Self-referral, one person with two NEAR AI logins
(two `nearai-` tenants), is caught only by signals the server observes
itself, and each one holds the referrer's referral stream for an operator:

- **Same payout org or subject** (`referral_same_payout_org`,
  `referral_same_payout_subject`), from the payout link blind indexes.
- **Near-duplicate traces** (`referral_same_traces`; mirror label
  `referral_duplicate_traces`, scored): the strong signal, since a second
  login uploading the first's traces is the cheap way to farm the bonus.
  Bounded per side to the most recent 500 submissions since 30 days before
  the referee's account was created.
- **Same network prefix** (`referral_same_network`; mirror label
  `referral_same_network`, not scored). Recorded in
  `trace_referral_network_marks(tenant_id, account_id, mark, first_seen_at,
  last_seen_at)`, in each account's own tenant under forced RLS. The mark
  is `sha256:` + hex of HMAC-SHA256 under the operator secret
  `TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY` (base64, at least 32 bytes)
  over a domain tag and the canonical prefix: IPv4 /24, IPv6 /48, an
  IPv4-mapped address as its IPv4. It is keyed because there are only 2^24
  IPv4 /24s, so an unkeyed hash is reversed by enumeration. The address is
  the rightmost `X-Forwarded-For` hop, which the Caddy edge overwrites (the
  source the rate limiter uses); a client-set hop to its left, an
  unattributed or malformed value, or an address that is not public
  unicast (loopback, private, link-local, unique-local, multicast) records
  nothing. Marks are recorded only for accounts in the program (they own a
  code or have an attribution): on the code create, list, apply and status
  routes and on a NEAR AI or wallet sign-up that carries a code, refreshed
  at most hourly. Trace submission does not record one: the upload path
  authenticates a device principal, not an account, and resolving it on
  every upload for every contributor costs more than the signal adds.

**The coworker case.** Colleagues on one office, campus or carrier-NAT
network refer each other legitimately, and that is the main way a network
match happens. So `referral_same_network` holds the stream for an operator
to look at and is never counted toward a code's or a referrer's
suspension, however many referees carry it.

**Retention.** A mark is kept for the earning period plus 30 days
(`TRACE_COMMONS_REFERRAL_EARN_DAYS` + 30, 122 by default) after it was last
seen. Recording deletes the account's older marks, the pass deletes the
tenant's when it visits it, and reads ignore older marks, so a missed
delete never matches.

**Key.** With the program configured (`TRACE_COMMONS_REFERRAL_PROGRAM_START`
set) or the bonus mode not `disabled`, ingest refuses to start without the
key (`referral_network_mark_key_missing`); a present but malformed key
refuses whatever the configuration (`referral_network_mark_key_invalid`).
Rotating the key makes earlier marks incomparable with new ones until they
expire. Nothing logs an address, a prefix, a mark or a cluster id.

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
