# Native onboarding funding contract

Date: 2026-09-04. Agent C, branch `onboarding-inference-funding`.
Status: provider capability research and read-only decoder implemented;
redemption, provisioning transport, and native funding integration remain blocked.

This design separates earned contribution credit, provider organization accounting,
and a contributor's usable inference allowance. None is inferred from another.
The first implementable component is an exact, identity-checked decoder for a
provider balance response. It performs no network call and changes no funds.

## Evidence collected

On 2026-09-04, unauthenticated GET of the official
[Cloud API OpenAPI document](https://cloud-api.near.ai/api-docs/openapi.json)
returned HTTP 200, `application/json`, with API version `1.0.0`. Snapshot:
`e2e1496245986a885e956ae293f656b02ef4212bd87b0bf15fee4365af92b923` (SHA-256, 329700 bytes). The document
advertises an API-key data plane, JWT management plane, refresh tokens, and
read-only reporting tokens. Its paths contain no payment, refund, transfer, or
redemption endpoint and no idempotency parameter/header. This establishes what
is documented, not that undocumented capabilities cannot exist. No private
account endpoint or billable inference request was called.

Provider implementation references below are pinned to public upstream revision
`5f6865755386e008947924dae427d917a671a3ce`. Source inspection establishes API
semantics; it does not establish that this revision is deployed. No upstream
implementation was copied or added as a dependency.

### Confirmed management surface

| Need | Observed provider contract | Constraint |
|---|---|---|
| Organization selection/creation | `GET/POST /v1/organizations` | JWT; no funding implied by creation |
| Workspace selection/creation | `GET/POST /v1/organizations/{org_id}/workspaces` | JWT and organization permission |
| Key listing/creation | `GET/POST /v1/workspaces/{workspace_id}/api-keys` | JWT; create takes name, optional expiry and `spendLimit` |
| Key revoke | `DELETE /v1/workspaces/{workspace_id}/api-keys/{key_id}` | JWT and permissions; revocation propagation needs integration validation |
| Key ceiling | `PATCH /v1/workspaces/{workspace_id}/api-keys/{key_id}/spend-limit` | JWT; ceiling is not a new funded balance |
| Org balance | `GET /v1/organizations/{org_id}/usage/balance` | JWT plus membership; organization scope |
| Usage reconciliation | Organization/key usage history; organization `/usage/export` and `/usage/summary` | History uses JWT; export/summary use reporting token |
| Billing adjustment | `PATCH /v1/admin/organizations/{org_id}/limits` | Provider administrator/billing authority, not a customer transfer route |

These paths and their security declarations come from the live
[OpenAPI document](https://cloud-api.near.ai/api-docs/openapi.json).

[Workspace route implementation](https://github.com/nearai/cloud-api/blob/5f6865755386e008947924dae427d917a671a3ce/crates/api/src/routes/workspaces.rs)
checks duplicate key names and returns a conflict. This is not a durable
idempotency contract: a retry does not promise the original secret or operation
result. Creation documents up to ten seconds before a key becomes usable.
The key response includes optional secret material; never include the response
in diagnostic logs. Request `spendLimit` uses integer amount plus currency;
responses include amount, scale, and currency. Null limit removes a ceiling,
so missing/null must never default to an onboarding budget.

### Balance semantics that constrain the client

The [usage implementation](https://github.com/nearai/cloud-api/blob/5f6865755386e008947924dae427d917a671a3ce/crates/api/src/routes/usage.rs)
uses signed 64-bit USD nano-dollars, scale 9. `remaining` is `spend_limit -
total_spent`; both limit and remaining may be absent. Remaining can be negative.
`updated_at` comes from the usage row when one exists; idle accounts can have
old timestamps despite a fresh read. Usage, limits, and breakdown are separate
concurrent reads. Treat inconsistent arithmetic as unusable evidence and retry
a read; never repair it into available credit. A missing usage row and missing
limit yield 404, not zero. Membership is checked before returning an org balance.

Observed org accounting is shared across members/workspaces/keys. It is never
shown as the contributor's spendable amount until the provider identity mapping,
key-specific ceiling/usage, current holds, and applicable policy are resolved.
The decoder's local `observed_at` measures fetch age; provider `updated_at` is
retained separately as usage metadata. Refresh policy is supplied by callers.

### Identity and billing gaps

Provider [auth source](https://github.com/nearai/cloud-api/blob/5f6865755386e008947924dae427d917a671a3ce/crates/api/src/routes/auth.rs)
contains NEAR signature login returning access and refresh tokens, distinct from
Trace Commons wallet login. The [router](https://github.com/nearai/cloud-api/blob/5f6865755386e008947924dae427d917a671a3ce/crates/api/src/lib.rs)
nests it as `POST /v1/auth/near`. It is absent from the observed OpenAPI path
list, so deployed support, ceremony requirements, native redirect handling,
refresh lifecycle, and delegation must be verified before implementation.
Never reuse a signature across relying parties or assume a Trace Commons session
is a provider JWT. Preserve separate explicit provider authorization.

The [admin route](https://github.com/nearai/cloud-api/blob/5f6865755386e008947924dae427d917a671a3ce/crates/api/src/routes/admin.rs)
requires administrator context and updates a typed spend limit; it accepts audit
metadata but no documented operation identifier or compare-and-swap version.
Its OpenAPI security label alone does not grant ordinary JWT users this power.
An absolute limit write could overwrite concurrent billing changes. This is not
an authorized credit-redemption implementation. Provider partnership/delegated
billing authority and transactional adjustment semantics remain unestablished.

## Current Trace Commons evidence

- `AccountCreditSummary` in `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
  reports earned points, optional currency estimate, posture, period, pending
  review. Its account principal set gates ownership before summing credit events.
  It does not report redeemable value or a provider balance.
- `credit_numbers.rs` distinguishes disabled settlement, synthetic dry-run, and
  HTTP settlement; `graded=false` explicitly keeps amounts revisable.
- `near_credit.rs` mirrors settlement through non-transferable receipts and
  supports receipt reversal. `amount_micros` is not provider USD nano-dollars;
  multiplying by a scale does not create a conversion entitlement.
- Native IronWire discovery observes a local proxy; it does not provision a key
  or route an agent. Native witness preview's `witness_claim_unavailable` must
  be resolved independently before the invited contribution pilot can succeed.

## Proposed parent-owned application contract

The following are candidate version-1 response bodies, not deployed endpoints.
Parent owns protocol types, routes, authenticated mapping, and migrations.

```json
{
  "version": 1,
  "capabilities": {
    "provider_link": false,
    "provider_balance": false,
    "credential_provisioning": false,
    "contribution_redemption": false,
    "sponsored_starter": false
  },
  "contribution_credit": {"eligibility": "unknown", "reason": "policy_not_configured"},
  "inference_allowance": {"status": "unknown", "reason": "provider_link_missing"},
  "provisioning": {"status": "not_started"}
}
```

All capabilities remain false for this branch: a decoder is not an integrated
service. Existing earned-point reporting stays available through its current
contract. Eligible credit requires authoritative finalized policy and deductions
for reservations/reversals; pending estimates cannot reserve value.

For internal reconciliation only, a **synthetic** observed provider balance:

```json
{
  "scope": "provider_organization",
  "currency": "USD",
  "scale": 9,
  "total_spent": "1000000001",
  "remaining": {"status": "reported", "amount": "999999999"},
  "usage_updated_at": "2026-09-01T10:00:00Z",
  "observed_at": "2026-09-04T12:00:00Z",
  "contributor_allowance": {"status": "unknown", "reason": "key_budget_unresolved"}
}
```

Decimal integer strings in the proposed UI protocol avoid JavaScript integer
rounding; the provider wire format uses int64 JSON numbers. Unknown, stale,
unsupported, zero, negative, and failed reads remain distinct. Cached snapshots
never silently become fresh. A key's remaining ceiling is also not proof of
funding; an actual usable allowance requires all applicable constraints and a
provider-supported authorization model. Do not grant a whole shared-org balance
independently to every contributor.

Persist an auth-derived tenant/account -> provider identity -> organization ->
workspace -> key mapping. Verify provider ownership before linking. A response
ID may corroborate the expected mapping but cannot create or switch it. Keep
provider secrets out of that mapping, analytics, error messages, and logs; only
opaque secret handles belong in native IPC. Provider organization IDs must not
be included in user-visible status or hash-only audit events.

## Conditional redemption state machine

This specifies required behavior before implementation; no provider operation
currently satisfies the funding mutation contract.

| From | Evidence/event | To and durable effect |
|---|---|---|
| requested | Account authorized, amount eligible, budget available | reserved; atomically debit available credit into a hold |
| reserved | Durable operation ready before provider call | provisioning; persist request fingerprint and attempt |
| provisioning | Provider confirms matching operation/amount | confirmed; consume hold once and retain reconciliation reference |
| reserved | Cancel before provider dispatch | released; return hold exactly once |
| provisioning | Provider proves no mutation occurred | released; return hold exactly once |
| provisioning | Timeout/disconnect/ambiguous response | reconciling; retain hold, never blindly retry mutation |
| reconciling | Authoritative operation lookup confirms success/failure | confirmed or released |
| confirmed | Provider confirms full/partial refund | refunded/partially_refunded; restore only proven unused value once |

Durable idempotency key scope is tenant/account plus operation kind. Bind a
canonical fingerprint of amount/unit, destination mapping version, and policy
version; same-key retry returns the same operation, changed payload conflicts.
Concurrent devices reserve under a single transactional account budget. Reserve
before network dispatch; crash after provider success must recover by operation
lookup. Row status and credit-ledger entries commit atomically under forced RLS.

Provider mutation must supply durable idempotency plus queryable operation
outcomes, or an equivalent safe protocol. No such guarantee was found. Local
idempotency cannot resolve whether a timed-out provider request committed.
Neither duplicate key names nor an observed higher aggregate org balance prove
that a particular funding operation succeeded. Do not implement automatic
redemption over absolute admin-limit updates.

Reconciliation distinguishes consumed inference from unconsumed funded value.
Limit reduction is not a refund. Partial refunds cannot exceed confirmed unused
funding, and duplicates cannot restore value twice. Credit reversal before
provisioning cancels eligibility; afterward it records a deficit/reconciliation
case, not an unsupported provider clawback. Configure retry/backoff operationally
without choosing credit conversion rates or subsidies in this code.

## Credential and zero-history flow

For an existing funded provider account, link its identity and select its org /
workspace explicitly before offering agent configuration. Key creation is
possible in the provider API but transport is deferred until delegated auth,
permission scope, owner mapping, and secret-store handoff are specified. Store
inference keys in platform secret storage, never daemon JSON or environment
snippets. Management/refresh/admin credentials never go to IronWire or agents.
On locked/unavailable stores, fail closed. Rotation verifies a replacement before
switching; revoke abandoned keys and account for uncertain create outcomes.
A missing response secret cannot be reconstructed from a listed key prefix.

Zero history plus existing provider funds can proceed without earning first.
Zero history without funds gets a resumable local setup and a clear funding-
unavailable state. Invite admission does not imply a paid allowance. Sponsored
starter access stays disabled unless an actual sponsor, approved budget, provider
funding mechanism, and reconciliation owner exist. No amount or exchange rate is
chosen here. Body capture, witness content transfer, and provider inference are
separate consent decisions; credential setup must not enable any of them.

## Implemented boundary and remaining blockers

Implemented `src/inference_funding.rs` decodes only the balance fields needed for
accounting, binds the expected UUID, preserves signed int64 nano-dollar precision,
rejects wrong currencies/inconsistent arithmetic/oversized or malformed JSON,
and models fresh versus stale local observations independently from unspecified
remaining balance. Wire objects have no Debug/Serialize; errors contain fixed
labels. Unknown provider fields and non-authoritative display strings are ignored.

`tests/inference_funding.rs` uses synthetic data; it tests large integer precision,
negative/zero/unspecified values, organization mismatch, currency mismatch,
overflow/inconsistent snapshots, malformed/duplicate fields, input bounds, and
freshness. It does not claim tenant/RLS, credential lifecycle, or live funding
coverage. The only integration edit is `pub mod inference_funding` in server lib.

| Blocker | Needed evidence/owner |
|---|---|
| Provider delegated auth | Native/provider identity agent: deployed ceremony, refresh and consent contract |
| Account/org/workspace/key mapping | Parent and identity owner: authenticated schema, lifecycle, tenancy tests |
| Funding mutation and refund | Provider capability/partnership: authorized API with durable outcomes |
| Mutation idempotency/reconciliation | Provider contract: duplicate handling and operation-status lookup |
| Eligible credit/conversion | Product/operator policy: authoritative basis, units, pricing and limits |
| Secret lifecycle and routing | Native owners: secret-store integration, rollback, revocation verification |
| Actual contribution loop | Integration agent: native witness review and later authorized live pilot |

No missing provider capability is replaced by a speculative endpoint. Independent
local decoder work is complete once its checks pass; the funding loop is not.

## Validation

Validation commands and results are recorded after execution below. All build
artifacts use `/tmp/trace-commons-inference-funding-target`, separate from other
worktrees. No provider keys, payments, private balances, or inference calls were
created or accessed during this work.

Completed checks:

```sh
export CARGO_TARGET_DIR=/tmp/trace-commons-inference-funding-target
export RUSTFLAGS='-D warnings'
cargo test -p trace-commons-server --test inference_funding --locked --offline
cargo test -p trace-commons-server --test license_boundary --locked --offline
cargo check -p trace-commons-server --bins --locked --offline
cargo fmt --all -- --check
```

Results: six decoder tests passed; four license-boundary tests passed without
changing expected sets; server binary compilation and formatting passed.

All-target Clippy also passed with warnings denied:

```sh
cargo clippy -p trace-commons-server --all-targets --locked --offline -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

`cargo test -p trace-commons-server --no-run --locked --offline` passed with the
same target directory and warnings-denied environment. All server test targets
compiled; only the six decoder tests and four license-boundary tests were run.
`git diff --check` passed. No dependency changes required license-tree updates.
