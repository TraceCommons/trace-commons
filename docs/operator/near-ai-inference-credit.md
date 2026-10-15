# NEAR AI inference credit settlement (stub)

Status: **not live.** The rail, its ledger (V129) and its tests exist, but
three things are still missing:

- NEAR AI has not shipped the `trace_commons` credit type it depends on;
- we have no NEAR AI admin key;
- nothing in `trace-commons-ingest` drives the rail yet.

Design:
[`../superpowers/specs/2026-10-14-near-ai-inference-credit-settlement-design.md`](../superpowers/specs/2026-10-14-near-ai-inference-credit-settlement-design.md).
NEAR AI proposal:
[`../superpowers/specs/2026-10-14-near-ai-trace-commons-credit-type-proposal.md`](../superpowers/specs/2026-10-14-near-ai-trace-commons-credit-type-proposal.md).

## What it does

Each finalized settlement batch for a tenant that has elected a NEAR AI
organization is priced in nano-USD by an **award policy**, and the result is
added once to that recipient's **cumulative** total in
`trace_settlement_rail_ledgers`. The policy works per recipient per period
(for the reference policy, the UTC calendar month of the batch's
`created_at`): it states the period's cumulative award, and the batch adds
whatever that exceeds what the period was already awarded. A lower answer
adds nothing; nothing is ever taken back.

The production award policy is supplied by a private backend. The open
build's `flat_rate` policy is an uncalibrated reference. The rail publishes the total. NEAR AI
keeps the maximum it has seen, as it does for staking-farm credit, so
publishing again is always safe. Totals never decrease: the database
refuses it.

- **Push** (default): we `PUT` the total to
  `/v1/admin/organizations/{org_id}/trace-commons/credit`.
- **Pull**: NEAR AI fetches a statement of the total, Ed25519-signed with
  our statement key.

## Configuration

| Variable | Required | Meaning |
|---|---|---|
| `TRACE_COMMONS_NEAR_AI_CREDIT_RAIL` | no | `disabled` (default; also any unrecognised value), `dry_run`, `live`. The kill switch. |
| `TRACE_COMMONS_NEAR_AI_CREDIT_DELIVERY` | no | `push` (default) or `pull` |
| `TRACE_COMMONS_NEAR_AI_ADMIN_BASE_URL` | push; pull with a read key | API root including `/v1`; `https://` only |
| `TRACE_COMMONS_NEAR_AI_ADMIN_KEY` | push `live` | read-write admin token (`adm_...`) |
| `TRACE_COMMONS_NEAR_AI_ADMIN_READ_KEY` | push `dry_run` without the write key; optional otherwise | read-only admin token |
| `TRACE_COMMONS_NEAR_AI_ADMIN_USER_AGENT` | whenever an admin key is set | the exact User-Agent the key was minted with |
| `TRACE_COMMONS_NEAR_AI_CREDIT_STATEMENT_KEY` | pull | base64 PKCS#8 Ed25519 key that signs statements |
| `TRACE_COMMONS_NEAR_AI_CREDIT_AWARD_POLICY` | when enabled | `flat_rate`, or the name of a policy the binary was built with. No default; an unknown name refuses. |
| `TRACE_COMMONS_NEAR_AI_CREDIT_FLAT_RATE` | with `flat_rate` | `numerator/denominator`, nano-USD per microcredit over the month. No default. Refuses if set beside another policy. |
| `TRACE_COMMONS_NEAR_AI_CREDIT_MAX_CUMULATIVE_NANO_USD` | when enabled | per-recipient ceiling; a total above it freezes |
| `TRACE_COMMONS_NEAR_AI_ADMIN_TIMEOUT_MS` | no | 100 to 120000; default 15000 |

If the rail is enabled and a control is missing or invalid, it refuses with
`missing_control:<name>`. `trace-commons-ingest` then refuses to start. It
never falls back to a less restrictive mode or to another delivery mode.

## Minting the keys

NEAR AI binds an admin key to the User-Agent of the request that minted it.
A key minted from a browser session fails from the server with
`near_ai_admin_unauthorized`. Mint each key with the User-Agent you set in
`TRACE_COMMONS_NEAR_AI_ADMIN_USER_AGENT`, or with none. Keys expire: record
the expiry and rotate before it. On NEAR AI's side, every write is
attributed to the admin who minted the key.

## Freeze labels

A frozen ledger keeps accruing but publishes nothing until it is
unfrozen. If you unfreeze without fixing the cause, it freezes again.

| Label | Meaning | Action |
|---|---|---|
| `settlement_rail_external_ahead` | NEAR AI holds more than we ever accrued | find the extra writer, or the lost accrual, with NEAR AI |
| `settlement_rail_external_regressed` | NEAR AI's total went below what it confirmed | escalate to NEAR AI; their total is meant to be monotonic |
| `settlement_rail_external_refused_total` | NEAR AI answered a publish with less than we sent | escalate to NEAR AI |
| `near_ai_admin_read_only_key` | push is using a read-only key | fix the configuration |
| `near_ai_org_not_found` | the organization is missing or inactive | the contributor rebinds |
| `near_ai_cumulative_above_ceiling` | a total exceeds the configured ceiling | check the award policy before raising the ceiling |

## Database grants (V129)

- **`trace_near_ai_credit_recipients`:** `SELECT, INSERT`, plus
  `UPDATE (revoked_at)`.
- **`trace_settlement_rail_credits`:** `SELECT, INSERT` only. It is
  append-only. Each row records `period_key`, `policy_ref` and
  `policy_ref_hash`.
- **`trace_settlement_rail_ledgers`:** `SELECT, INSERT`, plus column-level
  `UPDATE` on `cumulative_micros`, `cumulative_units`, `confirmed_units`,
  `publish_state`, `attempt_count`, `next_attempt_at`, `lease_token`,
  `lease_expires_at`, `last_error_label`, `confirmed_at`, `verified_at` and
  `updated_at`. A trigger refuses any decrease of a total.
- **All three:** `trace_ingest_runtime` gets no `DELETE`, and row-level
  security is forced.
- **Older tables:** the rail also reads `trace_credit_settlement_batches`,
  and its one-payout-path trigger reads `trace_near_credit_outbox`. The
  pilot's table-wide grants on pre-V62 tables cover both.
