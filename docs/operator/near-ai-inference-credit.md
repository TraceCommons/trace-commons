# NEAR AI inference credit settlement (stub)

Status: **not live.** The rail, its outbox (V127) and its tests exist; the
NEAR AI admin key does not, and nothing in `trace-commons-ingest` drives the
rail yet. This page covers what an operator needs once both arrive. Design:
[`../superpowers/specs/2026-10-14-near-ai-inference-credit-settlement-design.md`](../superpowers/specs/2026-10-14-near-ai-inference-credit-settlement-design.md).

## What it does

A finalized settlement batch for a tenant that has elected a NEAR AI
organization is converted at a configured rate into nano-USD and added to
that organization's NEAR AI spend ceiling through
`PATCH /v1/admin/organizations/{org_id}/limits`. That route sets an
**absolute** ceiling per credit type, with no compare-and-swap; the rail
reads the history before and after every write and freezes on anything it
did not write itself.

## Configuration

| Variable | Required | Meaning |
|---|---|---|
| `TRACE_COMMONS_NEAR_AI_CREDIT_RAIL` | no | `disabled` (default; also any unrecognised value), `dry_run`, `live`. The kill switch. |
| `TRACE_COMMONS_NEAR_AI_ADMIN_BASE_URL` | when enabled | API root including `/v1`; `https://` only |
| `TRACE_COMMONS_NEAR_AI_ADMIN_KEY` | `live` | read-write admin token (`adm_...`) |
| `TRACE_COMMONS_NEAR_AI_ADMIN_READ_KEY` | `dry_run` without the write key; reconciler | read-only admin token |
| `TRACE_COMMONS_NEAR_AI_ADMIN_USER_AGENT` | when enabled | the exact User-Agent the key was minted with |
| `TRACE_COMMONS_NEAR_AI_CREDIT_RATE` | when enabled | `numerator/denominator`, nano-USD per microcredit. No default. |
| `TRACE_COMMONS_NEAR_AI_CREDIT_MAX_DELIVERY_NANO_USD` | when enabled | per-delivery ceiling; a larger row freezes |
| `TRACE_COMMONS_NEAR_AI_CREDIT_TYPE` | no | NEAR AI credit type to write; default `grant` |
| `TRACE_COMMONS_NEAR_AI_ADMIN_TIMEOUT_MS` | no | 100 to 120000; default 15000 |

An enabled mode with a missing or invalid control refuses with
`missing_control:<name>` and never falls back to a less restrictive mode.

## Minting the keys

NEAR AI admin keys are bound to the User-Agent of the request that minted
them. A key minted from a browser session fails from the server with
`near_ai_admin_unauthorized`. Mint each key with the User-Agent you put in
`TRACE_COMMONS_NEAR_AI_ADMIN_USER_AGENT` (or with none). Keys expire; record
the expiry and rotate before it. Every write is attributed on NEAR AI's side
to the admin who minted the key.

Use two keys: `read_write` for dispatch, `read_only` for the reconciler.

## Freeze labels

| Label | Meaning | Action |
|---|---|---|
| `near_ai_grant_bucket_foreign_baseline` | the organization already had a credit row of this type we did not write | agree with NEAR AI how to treat it; there is no adopt action yet |
| `near_ai_grant_bucket_foreign_write` | someone else wrote the bucket after us; our credit may have been overwritten | read the history with NEAR AI, restore if needed, then clear the recipient freeze |
| `near_ai_grant_interleaved_write` | a foreign write landed between our read and our write; ours replaced it | the foreign amount was lost on their side; tell NEAR AI |
| `near_ai_admin_read_only_key` | dispatch is using the read-only key | fix the configuration |
| `near_ai_org_not_found` | the organization is missing or inactive | the contributor rebinds |
| `near_ai_delivery_above_ceiling` | a row exceeds the per-delivery ceiling | check the rate before raising the ceiling |
| `settlement_rail_effect_missing` | the reconciler cannot find a recorded write | escalate; do not re-send |

A frozen row is never retried automatically. A frozen row with no recorded
effect may be cancelled, which frees its batch for another payout path. A row
whose effect exists can never be cancelled.

## Database grants (V127)

V127 grants `trace_ingest_runtime` `SELECT, INSERT` and column-level
`UPDATE` on `trace_near_ai_credit_recipients`, `trace_settlement_rail_carry`
and `trace_settlement_rail_outbox`, and no `DELETE`. All three force row-level
security. The rail also reads `trace_credit_settlement_batches` and, through
its one-payout-path trigger, `trace_near_credit_outbox`, both covered by the
pilot's table-wide grants on pre-V62 tables.
