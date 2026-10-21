# NEAR AI starter advance for new contributors -- design

Date: 2026-10-21. Owner: Zaki. Status: draft. Slice 1 (section 7) is built:
features are recorded and a dry-run report reads them; nothing is sent.

Builds on the NEAR AI payout rail in
[`2026-10-21-nearai-credit-payout-design.md`](2026-10-21-nearai-credit-payout-design.md)
(draft PR #1532, "the payout design" below). Section and decision numbers
(D1-D8) refer to that document. Until #1532 merges, that link resolves only
on its branch.

## Owner decisions

- **(A1) Instant starter credit.** 2026-10-21: a new contributor whose
  first uploads plausibly look like a few days of real Claude Code or Codex
  traces gets at least $5 of NEAR AI inference credit almost immediately,
  without waiting for full scoring, judged by weaker heuristics.
- **(A2) Advance, not bonus.** 2026-10-21: the $5 is an advance that later
  earned NEAR AI credit absorbs, not a bonus on top of it.

Everything else here is a proposal; section 8 lists what is still open.

## 1. Summary

A new contributor gets a one-time **$5 NEAR AI starter advance** on the
`near_ai` rail, sent within about ten minutes of both of these being true:

1. their first accepted uploads pass `starter-plausibility-v1`, a cheap,
   versioned rule set (section 3), and
2. they have a confirmed `near_ai` link (payout design section 5, passkey
   step-up per D5).

- **Advance, not a new award (A2).** Later earned NEAR AI credit fills the
  $5 first, then sends resume. A contributor whose traces score well costs
  the commons nothing extra; one whose traces fail costs at most $5.
- **Trace Credit is untouched (D6).** The advance never creates, settles
  or reverses Trace Credit, and never appears in Trace Credit numbers.
- **The heuristics are a speed bump, not the defence.** Everything they
  read is declared by the client and an LLM can fake it. Loss is bounded by
  one advance per account, per NEAR AI subject and per recipient org, a
  daily cap, and a lifetime budget that cloud-api enforces through a
  dedicated #1271 credential (section 4).
- **Reconciliation is automatic.** Each advance ends `absorbed`,
  `forfeited`, `expired` or `refused_duplicate` (section 5).

This meets the condition
[`2026-09-04-onboarding-funding-contract-design.md`](2026-09-04-onboarding-funding-contract-design.md)
set for any sponsored starter: "an actual sponsor, approved budget, provider
funding mechanism, and reconciliation owner". #1271 is the mechanism; this
design supplies the budget and the reconciliation.

## 2. Placement in the payout design

The advance rides the `near_ai` rail as the payout design builds it: same
link, same per-link program, same cumulative send, same pass. It changes
one formula and adds one table.

### The target formula

The payout design sets a live link's target to
`owed - sum(final_paid of the account's closed links)`. With an advance:

```
target = max(owed, advance) - sum(final_paid of the account's closed links)
```

`owed` is unchanged: the sum of accruals from finalized batches. `advance`
is the account's advance in rail units (5,000,000,000 nano-USD for $5)
while it is `sent`, `absorbed` or `forfeited`, and zero otherwise. Because
#1271's cumulative mode sends `max(target - program_total, 0)`, nothing else
is needed: the first send pays the $5, later sends pay nothing until `owed`
passes $5, then pay the difference.

A relink is covered too: an account paid the advance on link A then
relinked to B has `final_paid(A) = advance`, so B's target is
`max(owed, advance) - advance`, which pays only earned credit above $5.

Crash recovery, the one-pending-line rule, per-send capping and the
per-link program all work unchanged, because an advance is just a larger
target on an ordinary line.

### What it reuses

| Payout design piece | Use here |
|---|---|
| D3 eligibility, D5 passkey link | An advance is granted before linking but sent only to a confirmed link |
| Per-link program (`tc1-...`) | The advance and later earned sends share it, so absorption is a cumulative total |
| `subject_index`, `recipient_index` on `trace_payout_links` | The uniqueness keys for one advance per NEAR AI subject and per org |
| Per-rail mode and kill switch | A separate `TRACE_COMMONS_NEAR_AI_STARTER_MODE` (`disabled`, `dry_run`, `live`, default `disabled`) inside the rail; the rail's halt stops advances too |
| Account ceiling | Must be at least the advance amount, or ingest refuses to start with starter mode `live` |
| Reconciliation drill | Adds advance counts by state |

New: the `trace_payout_advances` table (section 6), the plausibility
evaluator (section 3), a dedicated #1271 credential for advance sends
(section 4), and app copy, which needs owner approval of its wording before
it ships (CLAUDE.md, no new notices without approval).

## 3. Plausibility check

`starter-plausibility-v1` reads the features ingest records for each
envelope after admission, redaction and the residual scan, counting only
submissions that stayed `accepted`. It grants when every rule passes. There is no weighted
score, so a refusal names the rule that failed and the thresholds stay easy
to reason about. Thresholds are proposals to tune in dry run (section 5).

Each envelope is reduced to a hash-only record: a session hash, a content
hash, hour-truncated first and last event times, counts (tool calls,
results, failures, known tool names, turns, think-time gaps, token and cost
events), power-of-two bucket masks for turn gaps and tool latencies, and a
model family. It never stores trace text, a tool argument, a model name or a
session id. The evaluator folds an account's records at report time.

| Rule | Proposed threshold | Why it suggests real use | Cost to fake |
|---|---|---|---|
| Accepted by ingest (precondition, not one of the eight rules) | Only `accepted`, unwithdrawn submissions are recorded and read; the account read enforces it | The bar every upload already meets | None extra |
| Source agent | `source_session.adapter` is `claude-code` or `codex`, with an identity the protocol accepts (UUID native ids for both) | Scope of A1 | Trivial |
| Volume | At least 8 distinct sessions and 150 tool calls | A few days of real work produces dozens of sessions | Low with an LLM |
| Span | First or last events fall on at least 3 distinct UTC days, first to last at least 48 h, none in the future (envelopes older than 180 days are not counted) | "A few days" | Trivial |
| Rhythm | Gaps between turns are irregular, with human think-time pauses; tool latencies are not constant | Scripted generators tend to be regular | Moderate |
| Agent signature | At least 90% of envelopes that name a model use the adapter's family (Claude models for Claude Code, OpenAI models for Codex); at least 60% of tool names fit the agent's tool set (MCP tools count for both) | Mixed or unknown shapes point at a synthetic source | Low once known |
| Tool pairing | Over 95% of tool calls have a matching result; at least one tool result failed (retries are not detected in v1) | Real agent runs pair calls and include failures | Moderate |
| Usage | Token counts on at least half the assistant messages, input tokens growing step to step at least half the time (context grows); some list-price cost present | Real CLIs report usage this way | Moderate |
| Not copied | No two of the account's envelopes share a content hash. Cross-account duplicates (exact hash, simhash) are checked at send time, with the identity rules (section 4) | Catches replayed or shared corpora | High for replay, low for fresh fakes |

### Self-declared input

`TraceContributionEnvelope` marks `source_session`, `conversation_id` and
similar fields as attribution only, never a gate or scoring input. This
check deliberately uses them for one capped decision that pays no Trace
Credit. That is acceptable only because the amount is bounded and the
identity rules in section 4 are the real gate. The fields still never reach
a gate, a score or tenant scoping elsewhere. If the owner prefers to keep
the rule absolute, drop the source-agent, span and signature rules and
grant on volume plus identity alone (section 8).

### Why rules, not a model

A learned classifier would be harder to fake, but it needs labels that do
not exist yet. The dry run produces them: every first upload's rule outcome
next to its eventual score. A later version can be a model trained on that.

## 4. Abuse resistance

The threat is farming: many fake accounts, each with LLM-generated traces,
each collecting $5. The heuristics slow that down; the controls below cap
it.

### Identity: one advance per

- **Account, per rail:** unique `(tenant_id, account_id, rail_id)`.
- **NEAR AI subject:** unique `subject_index` among advances that were
  sent. One person with ten accounts and one NEAR AI login gets one advance.
- **Recipient org:** unique `recipient_index` among advances that were
  sent. Ten NEAR AI logins that all pay one org get one advance. This is
  stricter than D7, which allows many accounts per org for earned credit;
  the stricter rule applies only to advances.
- **Enrolled device**, a soft signal: an operator report lists advances
  that share a device enrollment, without blocking.

Subject and org are known only after linking, so these checks run when the
pass is about to send the advance, under the rail's single-flight lock. An
advance whose subject or org already holds one closes as
`refused_duplicate`, and the account waits for earned credit like anyone
else.

### Money: three caps

| Cap | Where enforced | Proposed pilot value |
|---|---|---|
| Per advance | Fixed amount in config; the advance credential's `max_amount_per_transfer` | $5 |
| Per day | The pass, counted in the database | 50 advances a day; overflow waits for the next day |
| Lifetime | The advance credential's `max_total_amount`, enforced by cloud-api under its own lock | $2,500 (500 advances), raised by updating the credential |

The lifetime cap is the strongest control: even a bug in the pass cannot
send more than the credential allows. Routing, decided at send time under
the rail's single-flight lock:

- If `owed` already covers the advance (`owed >= advance`), for example
  credit that settled before the account linked, the advance is closed as
  `absorbed` without being sent, and the line goes out on the rail's normal
  credential as usual.
- Otherwise the line is split. The normal credential sends the earned part
  (`owed` less closed links' `final_paid`), and the advance credential sends
  only the top-up, `advance - owed`, which is never more than the $5
  per-transfer cap.

Both send on the link's program, and #1271 sums a cumulative total across
credentials, so the program total still reaches `max(owed, advance)` and
absorption works. The advance credential only ever carries top-ups, so
advances never draw down the earned-credit budget and earned credit never
draws down the advance budget.

### Value to an attacker is low

#1271 transfer credit can only be spent on NEAR AI inference and cannot be
re-sent, so farmed advances cannot be pooled or sold through NEAR AI.

### Detection after the fact

- Full scoring runs on every advanced trace as usual. An account whose
  counted traces are mostly refused forfeits its advance (section 5).
- A forfeiture rate above a set level (proposed 20% over the last 100 sent
  advances) switches starter mode to `dry_run` by itself, with an audit row,
  and readiness reports it. Earned credit is unaffected.
- The rail's halt route (payout design section 4) stops advances at once.

### Accepted residual risk

A careful attacker with many distinct NEAR AI logins and orgs can still
collect $5 each until the daily cap or the forfeiture trip stops them. The
worst case is the lifetime cap. That is the price of paying before scoring.

## 5. Reconciliation with full scoring

Full scoring runs on advanced traces exactly as on any other, and the target
formula handles the money. Reconciliation labels each advance.

| State | Entered when | Money effect |
|---|---|---|
| `granted` | Rules pass; no confirmed link yet | None |
| `expired` | Granted, no confirmed link within 30 days | None; earned credit still pays later |
| `refused_duplicate` | At send time, the subject or org already holds a sent advance | None |
| `sent` | The pass confirms a line whose target includes the advance | $5 out |
| `absorbed` | `owed` reaches the advance amount | None; the advance is now ordinary earned credit |
| `forfeited` | Scoring rejects most counted traces, or they are withdrawn before settling | $5 recorded as a loss |

Only `sent` moves money. `absorbed` and `forfeited` are labels on an
advance already sent.

**Forfeit rule (proposed).** Once every envelope the rules counted has a
final outcome, the advance is forfeited if fewer than half of them settled
with positive credit. A counted envelope withdrawn before it settles counts
as refused. A forfeited account keeps earning normally, and its later
earned credit still fills the $5 before any new send, so a contributor who
improves pays the advance back with no one acting. A forfeited advance that
is later covered stays labelled `forfeited` for calibration and records the
date it was covered.

**Reversals.** An operator reversal lowers `owed` (payout design section
4). With an advance in place the target is `max(owed, advance)`, so a
reversal below $5 sends nothing and claws nothing back: the same netting
rule the payout design already uses.

**Trace Credit stays as scored.** The app's Trace Credit numbers never
include the advance. Any app text about the advance is a new notice and
needs owner approval of its wording before it ships. Draft for review:
"$5 of NEAR AI credit sent to {org name}. Your next $5 of earned NEAR AI
credit covers it."

**Dry run before live.** In `dry_run` the evaluator records would-grant
decisions and the pass sends nothing for advances. Two data sets calibrate
it:

1. A backfill over existing contributors' first uploads, which already have
   scores. Available on day one.
2. New accounts in dry run for two to four weeks.

Proposed bar for `live`: at least 80% of would-grant accounts reach $5 of
earned NEAR AI credit within 30 days, and at most 5% would have been
forfeited.

The reconciliation drill reports counts by state and outstanding exposure
(sent and not yet absorbed), as totals only, with no amount per account.

## 6. Storage

`trace_payout_advances`, RLS forced, tenant predicates through
`trace_current_tenant_id()`:

- `tenant_id`, `account_id`, `rail_id`, `advance_id` (UUID),
  `rule_version`, `rail_units NUMERIC(39,0)`, `state`, `subject_index`,
  `recipient_index` (both NULL until send time), `line_id` (the payout line
  that carried it), `counted_envelope_count`, timestamps per state.
- Unique `(tenant_id, account_id, rail_id)`.
- The subject and org uniqueness rules span tenants, so they cannot be a
  per-tenant unique index under forced RLS. They are checked by a narrow
  role in the style of `trace_payout_enumerator`, with `SELECT` on
  `(rail_id, subject_index, recipient_index, state)` only, assumed with
  `SET ROLE` for that one query, under the rail's single-flight lock.
- Plausibility features live in their own hash-only table, one record per
  submission (V131, `trace_starter_plausibility_features`), deleted with
  their submission. Pruning records once an advance exists, or once an
  account is past its first 30 days, comes with the advance schema.
- Every state change writes a `trace_payout_audit` row (rail id, action,
  advance id hash, prior and new state, label, time).
- Logs and audit stay hash-only: no recipient, subject, tenant or account
  id, and no amounts per account in logs.

## 7. Delivery plan

Four small PRs. The first has no dependency on the payout design and can
start now; the rest stack after payout plan Task 5 (the pass).

1. **Evaluator and dry-run report (built).**
   - `trace_commons_server::starter_plausibility`: `extract_features` reduces
     an envelope to a hash-only `StarterEnvelopeFeatures` record; `evaluate`
     applies `starter-plausibility-v1` to an account's records. Both pure.
   - V131 `trace_starter_plausibility_features`: one record per submission,
     written once by ingest after the submission lands, while
     `TRACE_COMMONS_NEAR_AI_STARTER_MODE=dry_run` (default `disabled`; `live`
     refuses startup until PR 3). The write runs on its own task and never
     blocks or fails an upload. The account read counts only `accepted`,
     unwithdrawn submissions of live principal links.
   - `POST /v1/admin/starter-advance/dry-run-report?limit=N`, admin only:
     label-only counts (accounts scanned, with features, would grant, first
     rule not met). It enumerates accounts through the earned-trust worker
     seam, so the ingest login needs `trace_account_trust_worker`
     (`docs/operator/account-trust.md`).
   - Not yet: the backfill over existing first uploads. Features exist only
     for submissions received in `dry_run`, because building them needs the
     envelope, which is stored encrypted. A backfill would read the stored
     artifacts through the same path the rescrub uses; it is its own PR.
2. **Advance schema.** `trace_payout_advances`, the cross-tenant uniqueness
   role, audit actions.
3. **Pass change and advance credential.** The new target formula, the
   advance credential on the `near_ai` rail, starter mode, the daily cap,
   and the boot check that the account ceiling covers the advance.
4. **Reconciliation, drill and app.** The forfeit rule, state counts in the
   drill and readiness, the auto-trip to `dry_run`, and the macOS status
   line once its wording is approved.

## 8. Still open

- **Self-declared fields in a money decision.** Proposed: allowed for this
  capped advance only (section 3). Alternative: keep the attribution-only
  rule absolute and grant on volume plus identity.
- **Budget values:** $5 per advance, 50 a day, $2,500 lifetime for the
  pilot.
- **Agents:** Claude Code and Codex only at first, or also OpenCode, Cline
  and Gemini CLI.
- **App wording** for the advance.
- **One advance per recipient org.** Proposed (section 4): one sent advance
  per `recipient_index`, stricter than D7, so in a shared org only the first
  contributor gets an advance. Alternatives: a per-org count cap, or rely on
  the subject and account rules plus the money caps.
- **Fact to check with NEAR AI:** that one sender org can hold two `oc_`
  credentials with separate `max_total_amount` caps sending on the same
  program. #1271's cumulative sum across credentials implies it, but it has
  not been tested in staging.
