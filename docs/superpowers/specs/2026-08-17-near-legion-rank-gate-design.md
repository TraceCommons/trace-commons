# Rank-gated invite grants for NEAR Legion

**Date:** 2026-08-17
**Status:** Approved for implementation
**Scope:** Replace the flat `nearlegion.nfts.tg` ownership gate with a
rank-tiered gate over the NEAR Legion rank SBTs, and make the grant size a
function of rank.
**Stacked on:** `e304e285`, "Add self-serve NEAR Legion invite claims". That
commit is the base; this design edits `near_legion_claim.rs` in place.
**Requested by:** NEAR, who proposed using rank within the Legion as the gate
rather than membership in the tradeable collection.

## What changes

Today a claim is allowed if the account holds at least one token of
`nearlegion.nfts.tg` (3,333 supply, tradeable), and every successful claimant
receives the same grant: an invite code redeemable `max_uses` times, defaulting
to 3.

After this change, eligibility and grant size are both determined by which rank
SBT the account holds:

| Rank      | Contract                    | `max_uses` | Eligible |
| --------- | --------------------------- | ---------- | -------- |
| Vanguard  | `vanguard.nearlegion.near`  | 5          | yes      |
| Ascendant | `ascendant.nearlegion.near` | 3          | yes      |
| Initiate  | `initiate.nearlegion.near`  | —          | no       |
| Base NFT  | `nearlegion.nfts.tg`        | —          | no       |

Initiate and the base collection are not merely set to zero — they are removed
from the gate entirely. A tier that grants nothing has nothing to ask the chain
about, so neither contract is queried on the eligibility path. Both are queried
once on the refusal path only, purely to tell a non-qualifying holder something
truthful; see "Refusal semantics".

## On-chain facts this rests on

Verified against mainnet RPC on 2026-08-17. These are the numbers the design
reacts to; re-check them before tuning caps.

| Contract                    | `nft_metadata.name` | `nft_total_supply` |
| --------------------------- | ------------------- | ------------------ |
| `vanguard.nearlegion.near`  | Vanguard            | 66                 |
| `ascendant.nearlegion.near` | Ascendant           | 580                |
| `initiate.nearlegion.near`  | Initiate            | 23,382             |
| `nearlegion.nfts.tg`        | NEAR Legion         | 3,333              |

All four are `nft-1.0.0` NEP-171 contracts, so the existing
`nft_supply_for_owner` view call and `parse_nft_supply_response` work against
the rank contracts unchanged. No new parsing, no new RPC shape.

### The ranks are cumulative

This is the single most consequential fact in the design. A sample of 10
Vanguard token owners (`nft_tokens{from_index:0,limit:10}`) found that **all 10
also hold Ascendant and Initiate tokens**. The ranks nest:

```
Vanguard (66) ⊂ Ascendant (580) ⊂ Initiate (23,382)
```

Therefore:

- **Eligible accounts number 580, not 646.** The Ascendant supply already
  contains the Vanguards. Adding the two tiers as if disjoint double-counts.
- **The cohort splits 66 at five uses and ~514 at three.**
- **Maximum invites in circulation is 1,872** (66x5 + 514x3), assuming every
  eligible account claims and every grant is fully redeemed.
- **Probe order is a correctness requirement, not an optimisation.** Because
  every Vanguard also matches the Ascendant contract, querying Ascendant first
  would issue three-use grants to top-rank holders. See below.

The sample is 10 of 66, not a proof of nesting for all holders. The design does
not depend on the nesting being universal — highest-rank-wins is correct either
way — but the population arithmetic above does, so treat 580/1,872 as
well-supported estimates rather than guarantees.

## Rank resolution

`near_account_holds_legion_token` is replaced by a function that resolves an
account to a rank, returning `Option<Rank>`:

1. Query `nft_supply_for_owner` on `vanguard.nearlegion.near`. Non-zero ->
   `Vanguard`, stop.
2. Query `nft_supply_for_owner` on `ascendant.nearlegion.near`. Non-zero ->
   `Ascendant`, stop.
3. Otherwise `None` -> refuse.

Cost is one RPC round-trip for a Vanguard, two for an Ascendant or a
non-holder. The common case is two, one more than today. This is accepted: the
alternative orderings are all wrong or slower.

**Security invariant: rank resolution is server-side and deterministic.** The
claim request carries no rank field, and the client cannot influence which
contract is queried or in what order. Given a fixed account and a fixed chain
state, resolution always returns the same rank. This invariant is what makes
per-rank policy labels safe (below); if a client could steer resolution, a
Vanguard could claim once as Vanguard and again as Ascendant.

An RPC error at any step is a hard failure returning `NearRpcUnavailable`, not
a fall-through to the next rank. Falling through would let a transient Vanguard
RPC failure silently downgrade a top-rank holder to a three-use grant.

## Per-rank caps

The current cap is a single global bound on live grants for the whole policy
(`if live >= state.legion.cap`, default 100). With eligibility falling to 580
accounts, that global cap — not the rank tiers — becomes the binding
constraint: the first 100 claimants take everything, and a Vanguard arriving
101st is refused while nothing distinguishes them from an early Ascendant. The
tiering would stop meaning anything at exactly the moment it matters.

Each rank therefore gets its own cap, implemented as **its own policy label**:

| Rank      | Policy label             | Default cap             |
| --------- | ------------------------ | ----------------------- |
| Vanguard  | `near-legion-vanguard`   | 66 (the full cohort)    |
| Ascendant | `near-legion-ascendant`  | 580 (the full cohort)   |

Separate labels rather than a new per-rank counting mechanism, because
`count_live(policy_label)` already does exactly the required count, and the
existing V42 partial unique index already scopes one-claim-per-account by
policy label. Both work unchanged. This also gives operators what the original
`POLICY_LABEL` comment wanted — the ability to list or revoke a cohort as a
unit — at rank granularity.

Defaults are set to the full eligible population so the cap acts as a circuit
breaker rather than a rationing device: the gate itself (soulbound, 580
accounts, rank-assigned) is the scarcity mechanism. An operator who wants a
smaller pilot lowers the cap via environment variable without a code change.

**Consequence of separate labels:** the one-claim-per-account uniqueness is now
scoped per rank, so the safety of the whole scheme rests on the resolution
invariant above. A test must assert that an account holding both Vanguard and
Ascendant — the common case, per the nesting finding — can claim exactly once,
under `near-legion-vanguard`, and is refused with
`InviteCredentialAlreadyBound` on a second attempt.

## Configuration

`TRACE_COMMONS_NEAR_LEGION_CONTRACT` (single contract) and
`TRACE_COMMONS_NEAR_LEGION_MAX_USES` (single grant size) no longer have
coherent meanings and are removed. Replaced by:

```
TRACE_COMMONS_NEAR_LEGION_VANGUARD_CONTRACT   default vanguard.nearlegion.near
TRACE_COMMONS_NEAR_LEGION_VANGUARD_MAX_USES   default 5
TRACE_COMMONS_NEAR_LEGION_VANGUARD_CAP        default 66

TRACE_COMMONS_NEAR_LEGION_ASCENDANT_CONTRACT  default ascendant.nearlegion.near
TRACE_COMMONS_NEAR_LEGION_ASCENDANT_MAX_USES  default 3
TRACE_COMMONS_NEAR_LEGION_ASCENDANT_CAP       default 580
```

`TRACE_COMMONS_NEAR_LEGION_ENABLED`, `_TENANT_TEMPLATE`, `_DENYLIST`, and
`_GRANT_TTL_DAYS` are unchanged and remain shared across both ranks. The
existing fail-closed rules carry over verbatim: the feature returns `None`
unless explicitly enabled and given a tenant template, and an unparseable
numeric value falls back to its default rather than widening the surface.

## The denylist stays

`nearlegion.near` holds **1 Ascendant token** (verified on-chain), so without
the denylist the project account could claim. It stays.

`intents.near` holds **0 of both ranks**, consistent with the rank SBTs being
non-transferable — the swap-contract accumulation that motivated the original
denylist cannot occur for soulbound tokens. It is retained anyway: it costs one
string comparison, and retaining it is cheaper than proving non-transferability
from the contract source.

## Refusal semantics

`AccountHoldsNoLegionToken` currently means "holds no Legion token". After this
change the same label would fire for an Initiate holder or a base-collection
holder — 23,382 + 3,333 accounts who hold something real, who would previously
have qualified, and who would be told they hold nothing. That copy is wrong in
a way users will notice and report.

A distinct refusal is added:

- `AccountHoldsNoLegionToken` — holds none of the queried rank contracts.
- `AccountRankNotEligible` (new, 400) — holds a Legion credential, but not at a
  qualifying rank.

Distinguishing the two requires querying the contracts the eligibility path
deliberately skips: `initiate.nearlegion.near` and `nearlegion.nfts.tg`. A
non-zero result from either yields `AccountRankNotEligible`.

These diagnostic queries run **only on the refusal path**, after both rank
probes return zero — never on the success path, where they would add two
round-trips to every claim. Only a genuine non-holder pays the cost, and they
are already being refused. A failure of either diagnostic query degrades to
`AccountHoldsNoLegionToken` rather than failing the request: it exists to
improve copy, not to gate anything.

## Status endpoint

`GET /v1/onboard/near-legion/status` currently returns a single
`{claimed, cap, remaining, maxUses}`. It becomes per-rank so `/legion` can show
a Vanguard visitor their own tier rather than an aggregate:

```json
{
  "vanguard":  { "claimed": 0, "cap": 66,  "remaining": 66,  "maxUses": 5 },
  "ascendant": { "claimed": 0, "cap": 580, "remaining": 580, "maxUses": 3 }
}
```

This is a breaking response-shape change. It is safe to make unilaterally: the
only consumer is `src/scripts/legion-claim.ts` in the community site, which has
never been deployed (the page exists solely on the unpushed local branch
`devfolio-integration-page`). Both sides land together.

## No migration

The feature is gated behind `TRACE_COMMONS_NEAR_LEGION_ENABLED`, the base
branch has never been pushed, and no PR exists. No grant has ever been issued
under the flat gate, so there is no live cohort to reconcile, no grandfathering
rule, and no backfill. The old `near-legion` policy label can simply fall out of
use.

## Testing

Mirroring the existing inline test module and its fake-RPC harness:

- Vanguard holder receives a 5-use grant under `near-legion-vanguard`.
- Ascendant holder receives a 3-use grant under `near-legion-ascendant`.
- **Holder of both ranks receives 5 uses under the Vanguard label** — the
  nesting case, and the one that matters most.
- That same account is refused `InviteCredentialAlreadyBound` on a second
  claim, proving per-rank labels did not open a double-claim path.
- Initiate-only and base-collection-only holders are refused
  `AccountRankNotEligible`, not `AccountHoldsNoLegionToken`.
- An account holding nothing is refused `AccountHoldsNoLegionToken`.
- `nearlegion.near` is refused `AccountNotEligible` despite holding Ascendant.
- A Vanguard-contract RPC error refuses `NearRpcUnavailable` and does **not**
  fall through to an Ascendant grant.
- Each rank's cap is enforced independently: a full Vanguard cap does not
  refuse an Ascendant claimant, and vice versa.

## Community site

`src/pages/legion.astro` and `src/scripts/legion-claim.ts` need matching work,
tracked on branch `legion-rank-copy` (worktree `tc-legion-rank-copy`, based on
`704ca91`):

- Eligibility copy: rank SBTs, not the tradeable collection.
- Show the resolved rank and its grant size ("Vanguard — 5 invites").
- Render `AccountRankNotEligible` distinctly from `AccountHoldsNoLegionToken`.
- Consume the per-rank status shape.

## Out of scope

- Any change to the Celestine Sloth Society slice, which is a separate module
  on a separate branch and shares no code touched here.
- Re-ranking, rank refresh, or reacting to a rank changing after a claim. A
  grant reflects rank at claim time and is not revisited.
- Making Initiate eligible at a lower grant size. Explicitly decided against:
  23,382 accounts is closer to "has a NEAR wallet" than to a vetted cohort.
