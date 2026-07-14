# Per-contributor concave caps — design

## Goal

Bound how much credit a **single contributor identity** can extract per epoch,
independent of duplication. The credit pipeline is `raw = q · dup_pen · …`;
cross-trace dedup (#2) caps duplication *across* identities, but a farm that
submits many *distinct, non-duplicate* traces under one identity still
accumulates linearly. This slice adds the orthogonal defense: a per-decision
`contributor_factor` that makes the k-th unit of credit from the same identity
worth diminishingly less, so one identity's total credit **per epoch** asymptotes
to a hard ceiling `K` no matter how many traces it submits.

Shadow-only, like #1/#2: `contributor_factor` is persisted/derivable but
multiplies nothing that pays until the settlement sub-project (#5).

This is sub-project #3 of the credit pipeline. Prior: #1 shadow credit-quality
`q` (PR #168, V2-recalibrated PR #170), #2 cross-trace dedup `dup_pen` (PR #169).
Remaining after this: #4 reputation weighting, #5 delayed settlement + clawback →
NEAR on-chain, #6 execution/replay grounding.

## Background: rate limit, not lifetime ban

The 2026-07-12 credit-farming red-team's anti-fabrication basis is
economics-first: fresh identities earn ~0, and concave caps bound accumulation.
A *lifetime* asymptote (one identity earns at most `K` ever) is the strongest
anti-sybil bound but is a lifetime ban — a genuine prolific contributor who has
earned ~`K` earns ~0 forever after, conflating one farming burst with sustained
real contribution.

A **per-epoch** cap turns it into a *rate limit*: each epoch a contributor's
running total resets, so honest sustained work keeps earning ~`K` per epoch
indefinitely while a farm is throttled to ~`K` per epoch. This is the property a
credit-*emission* system wants, and it aligns the cap with the on-chain
settlement epochs (#5): the cap is computed per settlement round. Epoch length is
7 days (weekly).

Combined with dedup (#2, cross-identity duplication) and reputation (#4,
fresh-identity → ~0), the per-epoch concave cap is the per-identity throttle of
the economics-first stack.

**Known gap this slice deliberately does not close (→ #4).** The concave cap
bounds a contributor's *earned* credit `R` to ~`K` per epoch, but it does not
penalize the *composition* of that credit: an honest contributor reaching `K`
with a few high-`q` traces and a spammer reaching the same `K` with a flood of
low-`q` traces both land at ~`K`. Penalizing a burst of low-score traces so a
junk-flood earns *sub-`K`* is a per-contributor quality-velocity / track-record
signal — the core mechanic of #4 (reputation weighting), which multiplies into
the pipeline as a separate orthogonal factor (`raw = q · dup_pen ·
contributor_factor · reputation`). Kept out of #3 so each slice is one concept;
#3 sets the ceiling, #4 lowers where a flooder lands under it.

## The concave cap

A contributor's cumulative "raw" credit **within the current epoch** is
`R = Σ(q · dup_pen)` over their scored decisions in that epoch. Consistent with
the #1 credit-quality and #2 dedup enumerations, this is **not** filtered by
submission acceptance status — every scored decision contributes. A
gate-rejected trace has low `q` (it fails the perplexity/novelty floors), so it
adds little to `R`, and its own `contributor_factor` is moot since a rejected
trace earns no base credit to multiply. Effective credit is a saturating concave
function of `R`:

```
effective(R) = K · (1 − exp(−R / K))
```

- `effective` starts at 0, rises with marginal slope 1.0 at `R = 0`, and
  asymptotes to `K` as `R → ∞` — a hard per-epoch ceiling on any one identity.

Each decision's **`contributor_factor`** is the *marginal* effective-per-raw at
its point in the running total. For a decision with increment `r = q · dup_pen`
and prior in-epoch cumulative `R_before`:

```
contributor_factor = [effective(R_before + r) − effective(R_before)] / r     (r > 0)
                   = exp(−R_before / K)                                        (r → 0 limit)
```

The factor decays smoothly from 1.0 (first trace of the epoch) toward 0 as the
contributor accumulates. The pipeline term is `raw_capped = q · dup_pen ·
contributor_factor`.

**Epoch bucketing** is deterministic and globally consistent (no genesis anchor):

```
epoch_index = floor(decided_at_unix_secs / (epoch_days · 86400))
```

`R` resets at each epoch boundary — a hard reset, so the first trace of each
epoch is back at factor 1.0. Boundary-timing is not an exploit: an identity still
earns at most ~`K` per epoch either way.

## Inputs

All inputs are already on the decision row, plus one join for identity:

- `r_micros = credit_quality_micros / dedup_cluster_size` — `dup_pen =
  1 / dedup_cluster_size`. `dedup_cluster_size` NULL → treat as 1 (dup_pen = 1);
  `credit_quality_micros` NULL → `r = 0` (contributes nothing to `R`).
- Contributor identity is `actor_principal_ref` (the hashed principal storage
  ref; already the leaderboard key), read via `trace_gate_decisions ⋈
  <submissions>` on `submission_id`, on the gate-driver reader pool
  (submissions are already gate-driver-readable, per
  `list_submissions_needing_gate_decision`).

No denormalization and no contributor registry table — mirrors dedup: membership
is the join key, the running total is a cross-tenant ordered sum, and only the
per-decision snapshot is persisted.

## Persistence

**New columns on `trace_gate_decisions`** (migration V41; RLS-scoped writes via
the tenant pool, cross-tenant reads via the gate-driver reader):

- `contributor_factor_micros INTEGER` — the marginal factor · 1e6 for this
  decision.
- `contributor_cumulative_raw_micros BIGINT` — the in-epoch `R_after` snapshot
  (running total including this decision), for audit and idempotent recompute.
- `contributor_cap_epoch BIGINT` — the `epoch_index` this decision fell in (the
  batch pass partitions on it).
- `contributor_cap_version INTEGER` — versioned constants stamp.

Migration wired into the hand-rolled `run_migrations` (a migration file alone is
inert; verify by dropping the columns + the `_trace_commons_migrations` row and
confirming `run_migrations` recreates them).

`CONTRIBUTOR_CAP_CONSTANTS_V1 { k_micros, epoch_days, version }` — pinned,
versioned. `k_micros = 25_000_000` (K = 25.0 per epoch), `epoch_days = 7`,
`version = 1`. `K` is a calibration starting point seeded so the decay is
*visible* on the pilot's single-contributor corpus (R ≈ 50 within a week →
factors span 1.0 → ~exp(−2) ≈ 0.14); tune on the backfill.

## Compute — batch-only in v1

`POST /v1/admin/recompute-contributor-caps` (mirrors `recluster-dedup`:
`require_admin`, fail-closed if the DB mirror is absent, hash-only ack,
background task, `?limit=N`).

A **single forward pass** per `(actor_principal_ref, epoch_index)`: enumerate
decisions cross-tenant ordered by `(actor_principal_ref, epoch_index,
decided_at ASC)`, accumulate `R` per contributor-epoch (resetting at each epoch
boundary), compute `contributor_factor` and `R_after`, and UPDATE each decision's
`contributor_*` columns. Each decision's factor depends only on *prior* decisions
in the same epoch, so — unlike dedup's two-pass final-size fixup — one forward
pass is correct and stable.

**Inline at gate time is deferred.** The cap is inherently a replay/cumulative
quantity; the batch pass over `decided_at` order is the canonical computation. An
inline single-pass factor would be based on partial state the next batch pass
overwrites anyway. (Follow-up #3b can add an inline snapshot if a live-ish factor
is wanted.)

## Isolation and audit

- The UPDATE touches only the `contributor_*` columns, exact PK `(tenant_id,
  decision_id)`, tenant pool + `begin_trace_tenant_transaction`. Leaves `q`,
  perplexity, novelty, dedup, status, credit byte-identical.
- Cross-tenant enumeration + the identity join run on the gate-driver reader
  pool, no tenant GUC.
- Hash-only audit: counts, decision ids as existing gate code logs them, error
  hashes. Never raw trace text, contributor identity in the clear (the
  `actor_principal_ref` is already a hashed ref), or the join inputs.

## Testing

**Unit — cap math** (pure, no DB): `effective` monotone and saturating; marginal
`contributor_factor` starts at 1.0 for `R_before = 0` and decays toward 0;
`r → 0` limit equals `exp(−R_before/K)`; deterministic; bounded `[0, 1e6]`.

**Anti-farm guarantee as an assertion**: a contributor's total effective credit
in one epoch — `Σ(r · contributor_factor)` over the epoch — is bounded above by
`K` regardless of how many traces (N) they submit; adding more traces yields
diminishing total.

**Epoch reset**: two decisions in different epochs both see `R_before = 0` (both
start at factor 1.0); within an epoch the factor is non-increasing along
`decided_at`.

**Cross-tenant aggregation**: the same `actor_principal_ref` across two tenants
shares one per-epoch running total (proves the cap counts across tenants).

**Forward-pass ordering**: a decision's factor depends only on prior in-epoch
decisions; a later arrival does not change an earlier decision's snapshot.

**Column isolation** (real-Postgres): the recompute UPDATE touches only
`contributor_*` columns and leaves `credit_quality`/dedup/perplexity/novelty/
status byte-identical before/after (mirrors #1/#2).

**Distribution validation on the pilot**: run the batch recompute, inspect the
`contributor_factor` distribution and per-contributor `R`; confirm non-degeneracy
and the visible decay, and answer how concentrated the pilot corpus is on one
identity. Plus batch idempotency: re-running yields stable factors.

## Rollout

Ships with the batch route behind the admin credential (no inline, no migration
beyond V41). After deploy: run the recompute over the pilot decisions, inspect
the `contributor_factor` / `R` distributions, calibrate `K` (and reconsider
`epoch_days`) → V2, re-run. No payout, no gating.

## Non-goals

- No settlement, payout, or gating. `contributor_factor` multiplies nothing that
  pays.
- No inline gate-time computation in v1 (deferred to #3b).
- No contributor registry table (would fight forced RLS); membership is the join
  key, size/running-total is a cross-tenant ordered sum.
- No change to `q`/credit_quality, dedup, perplexity, novelty, tail-fraction,
  gate status, credit, reputation, or the vector indexes.
- No cross-epoch carry-over or sliding window in v1 (hard reset each epoch).
