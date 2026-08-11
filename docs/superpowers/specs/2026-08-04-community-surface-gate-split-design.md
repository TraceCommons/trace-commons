# Community surface gate split: publish the roster without noising consent

**Date:** 2026-08-04
**Status:** Proposed
**Scope:** `trace-commons-ingest` community publication gate, plus the payload
contract the community site reads.

## Problem

`/v1/community/leaderboard` returns:

```
503 {"error":"community snapshot is withheld by missing privacy controls:
     TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT, community_noise_mechanism,
     TRACE_COMMONS_COMMUNITY_TENANT_IDS"}
```

One gate, `community_publication_missing_controls`, decides whether a snapshot
containing three different things may be published:

| In the snapshot | What protects the people in it |
|---|---|
| `leaderboard` | Consent. Only `public_attribution` contributors appear. |
| `contributors` | Consent. Same scope, same withdrawal path. |
| `analytics` | Nothing yet. Aggregates span *every* contributor, opted in or not. |

The gate requires an approved noise mechanism before any of them publish. For
`analytics` that is exactly right. For the roster it is the wrong control
applied to the wrong risk: those contributors asked to be named, and their
handle and counts being public *is the feature they consented to*. Adding
calibrated noise to a figure someone requested you publish about them protects
no one and makes the published number wrong.

The practical result is that a surface with no outstanding privacy work is held
behind a mechanism it does not need, and `tracecommons.ai/leaderboard` ships a
"not live yet" state indefinitely.

## Non-goal

This does **not** propose approving a noise mechanism, relaxing the analytics
gate, or adding a prefix to `COMMUNITY_APPROVED_NOISE_SEED_PREFIXES`. That list
stays empty and analytics stays withheld. The existing comment on it is correct
and this design does not touch it:

> Deliberately empty. […] Listing it would make the public Laplace-noise claim
> true by fiat rather than by construction.

## Design

### Two gates, named for what they protect

```rust
/// Roster: leaderboard rows and contributor profiles. Gated on cohort
/// shape only. The disclosure control here is the public_attribution
/// consent scope, enforced at write time; noise would corrupt a figure
/// the contributor asked to have published.
fn community_roster_missing_controls(
    min_cell_count: usize,
    tenant_cohort_size: usize,
) -> Vec<&'static str>;

/// Analytics: aggregates over contributors who never opted in. Roster
/// preconditions plus an approved mechanism.
fn community_analytics_missing_controls(
    min_cell_count: usize,
    tenant_cohort_size: usize,
    noise_mechanism_approved: bool,
) -> Vec<&'static str>;
```

`min_cell_count >= COMMUNITY_MIN_CELL_COUNT_FLOOR` and
`tenant_cohort_size >= COMMUNITY_MIN_TENANT_COHORT` still apply to the roster.
Both are about cohort shape rather than inference: a min cell of one *is* the
contributor, and a single-tenant cohort republishes one tenant's corpus under a
"community" label. Neither weakens.

### Recompute writes a roster-only snapshot

The recompute path currently refuses to compute at all, for a reason worth
preserving verbatim:

> Fail closed before touching contributor data: a snapshot that cannot be
> published is not worth computing, and computing it anyway leaves un-noised
> aggregates sitting in the snapshot table.

That concern is about **un-noised aggregates at rest**, and the fix honours it
rather than overriding it: when the analytics gate fails, recompute does not
compute analytics at all.

```
roster gate fails      -> 409, as today. Nothing is computed.
roster ok, analytics fails -> compute leaderboard + contributors.
                              Do NOT compute analytics. Store
                              contents.analytics = null.
both ok                -> compute everything, as today.
```

No un-noised aggregate is ever written. The snapshot table cannot leak what was
never computed, which is a stronger position than computing and withholding at
serve time.

`CommunitySnapshotContents.analytics` becomes `Option<CommunityCorpusAnalytics>`,
and the privacy block records why:

```rust
struct CommunitySnapshotPrivacy {
    // ...existing fields...
    /// Empty when analytics were computed. Otherwise the label-only
    /// control names that blocked them, same convention as the serve
    /// path: no tenant ids, handles, or counts.
    analytics_withheld_controls: Vec<String>,
}
```

### Serve path

| Route | Gate |
|---|---|
| `/v1/community/leaderboard` | roster |
| `/v1/community/contributors/{handle}` | roster |
| `/v1/community/analytics/summary` | analytics — unchanged 503 |

`latest_publishable_community_snapshot` takes a `CommunitySurface` argument
(`Roster` or `Analytics`) and applies the matching gate.

`/v1/community/leaderboard` returns the stored contents as-is, so `analytics` is
`null` and `privacy.analytics_withheld_controls` is populated. **Omission is
never silent** — a client cannot otherwise distinguish "withheld" from "zero
activity" from "bug", and the site currently has to guess.

### Snapshots written before this change

`community_snapshot_missing_controls` reads the stored `privacy` block and
treats a missing one as a cohort of one. That stays. A pre-gate snapshot fails
the roster gate on cohort size and is refused, which is the existing behaviour
and the reason the current 503 mentions all three controls.

## What this does and does not unblock

**Necessary but not sufficient.** After this change the leaderboard publishes
only once the deployment also has:

1. `TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT >= 2`
2. `TRACE_COMMONS_COMMUNITY_TENANT_IDS` naming **at least two tenants**

Both appear in the current error, so neither holds today. (1) is configuration.
(2) is not: if the pilot has one tenant, the leaderboard stays withheld until a
second one exists, and that is correct — `COMMUNITY_MIN_TENANT_COHORT` exists
precisely so a single tenant's corpus is never published as "the community".

This design removes the *wrong* blocker. It does not manufacture a cohort.

## Testing

Extending the existing `community_publication_blocks_on_unapproved_noise_mechanism`:

- roster gate passes with an unapproved mechanism; analytics gate fails
- both gates fail below the min-cell floor
- both gates fail below the tenant cohort floor
- recompute with analytics blocked stores `analytics: null` and a populated
  `analytics_withheld_controls`, and stores **no** aggregate fields
- `/v1/community/leaderboard` 200s on a roster-only snapshot
- `/v1/community/analytics/summary` 503s on the same snapshot, with the
  unchanged message
- a pre-gate snapshot with no privacy block is still refused on both surfaces

## Consumer impact

`trace-commons-community` reads `/v1/community/leaderboard` and takes
`snapshot.analytics` from that one payload, at build time
(`scripts/fetch-snapshot.mjs`) and at runtime (`src/scripts/live-snapshot.ts`).
Both need to accept `analytics: null` and keep the existing not-live state for
`/analytics` while rendering a live `/leaderboard`. That site already models
"snapshot present, figures absent" for the placeholder case, so this is a second
input into an existing state rather than a new one.

## Alternatives rejected

**Approve the existing keyed jitter.** It hashes the true `count` into its own
noise input, so each count maps to exactly one output — the output distributions
for neighbouring datasets are disjoint point masses. It is also bounded and
never zero. It is not a DP mechanism under any calibration, and the empty
allowlist is deliberate.

**Serve analytics with a `withheld` flag and no numbers.** Same client
complexity, but requires computing aggregates in order to discard them,
reintroducing exactly the at-rest exposure the recompute comment warns about.

**Separate snapshot rows per surface.** Cleaner separation, but two rows can
disagree about the window they describe, and the roster and analytics are
computed from one pass over the same inputs. Not worth the consistency problem.
