# Graded credit scoring function — design

## Goal

A pure, deterministic per-trace **credit quality score** `q ∈ [0,1]` computed from
the gate's already-stored numeric signals (representative perplexity, peak
perplexity, representative novelty), persisted on each `trace_gate_decisions`
row. `q` is the substance filter at the base of the credit pipeline: later
sub-projects (dedup, per-contributor caps, reputation, delayed settlement)
multiply their own terms against it. This spec delivers **only** the scoring
function plus its persistence and compute paths. It does **not** settle, pay, or
gate anything — credit stays off; `q` is an observability value in shadow mode
until settlement is built.

This is sub-project #1 of the credit pipeline. Sequence:
1. **Credit scoring function** (this spec)
2. Cross-trace dedup (`dup_pen`)
3. Per-contributor accounting + caps
4. Reputation weighting
5. Delayed settlement + clawback → NEAR on-chain cutover
6. Execution/replay grounding (`ground`) — deferred; the red-team's load-bearing
   term, its own project.

Anti-fabrication basis for eventual live issuance is **economics-first**
(reputation starting near zero + delayed settlement + clawback + concave caps +
dedup make gaming uneconomic), with replay grounding as later hardening. Those
live in sub-projects 3–5, not here.

## Background: why this shape

An adversarial red-team (see the credit-farming threat model, 2026-07-12)
established that perplexity and novelty are both **content-surface metrics** that
measure "how surprising the text looks," and surprising is cheap to fabricate
(one LLM call, or `/dev/urandom | base64`). They fail in the same direction —
rare/off-distribution tokens inflate perplexity **and** embedding distance at
once — so "require both" does not save you, and a graded reward hands an attacker
a gradient to hill-climb. The scoring function's job is to be the most
gaming-resistant *arithmetic* possible on these two signals; it cannot by itself
resist industrial fabrication (that needs `ground`), which is why credit stays
off until the economic sub-projects exist.

Live data from the 2026-07-12 27B re-score of the 349-trace corpus:
perplexity_micros p25 4.87M / p50 9.51M / p90 38.5M / **max 1642M** (≈40× p90).
That max is a real rare-token/encoded-content blow-up — concrete evidence the
perplexity term must saturate.

## Non-goals

- No settlement, payout, credit-issuance, or on-chain interaction.
- No dedup, reputation, per-contributor caps, or grounding/replay (later
  sub-projects).
- No envelope decryption and no content-level features. The function reads
  **only** numeric columns already on `trace_gate_decisions`. Content-level
  hardening (per-token winsorization, gzip high-entropy exclusion) is a separate
  gate-scorer-hardening concern, not this function.
- No change to gate status, the vector/embedding index, novelty, tail-fraction,
  or the perplexity columns.
- No adaptive/percentile-of-live-corpus normalization (itself an attack surface
  and non-reproducible) — constants are pinned and versioned.

## The scoring function

Inputs (micros integers, already stored):
- `ppl_rep` = `perplexity_micros` (representative perplexity)
- `ppl_peak` = `peak_perplexity_micros`
- `nov_rep` = `novelty_score_micros` (representative novelty)

Floors are the live gate floors: `PPL_FLOOR = 6.0` (6_000_000 micros),
`NOV_FLOOR = 0.5` (500_000 micros).

```
q = clamp01( f(ppl_rep) · g(nov_rep) · a(ppl_rep, ppl_peak) )
```

**Concave, saturating transforms** (anti-Goodhart: diminishing marginal credit,
no gradient to climb; and the log-ceiling is itself the winsorizer that caps the
1642 outlier at 1.0):

```
f(ppl) = clamp01( log(1 + max(0, ppl − PPL_FLOOR)) / log(1 + PPL_CEIL − PPL_FLOOR) )
g(nov) = clamp01( log(1 + max(0, nov − NOV_FLOOR)) / log(1 + NOV_CEIL − NOV_FLOOR) )
```

- Below floor → 0. At/above ceiling → 1. Compute in floating point on the micros
  values (or on the micros/1e6 real values — implementation picks one and is
  consistent); the ratio makes the unit choice irrelevant as long as floor and
  ceiling use the same unit.

**Multiplicative, not additive:** a trace strong on one signal but weak on the
other collapses toward zero. You cannot buy credit by maxing perplexity alone
(rare-token pump) or novelty alone (distinctive-token shim); both must be
genuinely present.

**Anomaly term `a` — where `peak` earns its keep:** since `q` is driven by the
*representative*, the peak-chunk parasite (one crafted high chunk in an otherwise
junk trace) is already neutralized for credit. `peak` is therefore used **only**
as a fraud signal, never a bonus:

```
r = ppl_peak / ppl_rep                          // spikiness ratio (guard ppl_rep <= 0 → a = 1, no signal)
a = 1                             if r ≤ R_SOFT
a = 1 − (r − R_SOFT)/(R_HARD − R_SOFT)   if R_SOFT < r < R_HARD   // linear decay 1 → 0
a = 0  (+ record reason)          if r ≥ R_HARD
```

`a` **defaults to 1.0** and only bites on a suspiciously spiky profile. When `a`
drops below a threshold, persist a reason label (reusing the existing
`credit_withheld_reason` label-only pattern — no raw content).

**Per-trace cap is structural:** `clamp01` ⇒ `q ∈ [0,1]`; no single trace exceeds
one unit of quality.

### Constants

`PPL_FLOOR = 6_000_000`, `NOV_FLOOR = 500_000` are fixed by the gate.
`PPL_CEIL`, `NOV_CEIL`, `R_SOFT`, `R_HARD` are **calibration outputs** derived
from the 27B-consistent 349-row distribution during implementation (starting
proposals to validate: `PPL_CEIL ≈ 38_500_000` = p90; `NOV_CEIL` from the
observed novelty p90; `R_SOFT`/`R_HARD` from the observed `peak/rep`
distribution). They are **pinned constants stamped with a calibration version**,
never a live percentile. Recalibration = bump the version + re-run the batch
pass.

## Persistence

New columns on `trace_gate_decisions` (new migration; confirm the next migration
number against `migrations/` during planning — the shared test DB already has
30–34 applied):
- `credit_quality_micros` INTEGER — `q` × 1e6, range `[0, 1_000_000]`.
- `credit_quality_anomaly_ratio_micros` INTEGER — `r = peak/rep` × 1e6, persisted
  for observability even when `a` did not bite.
- `credit_quality_calibration_version` — identifies the pinned constant-set that
  produced this `q`.
- Anomaly withholds reuse the existing `credit_withheld_reason` column pattern
  (label/enum only). No new reason column.

RLS is forced on the table like every Trace Commons table; the new columns follow
the existing tenant predicate. No new DB role.

## Compute paths (one pure function underneath)

1. **Inline at gate time.** After `evaluate_and_record_gate` writes a decision
   row, compute `q` from the numeric columns just written and store it. Pure
   arithmetic on values already in hand — no decryption, no index, no extra I/O.
   New traces get a `q` immediately.
2. **Batch admin route** `POST /v1/admin/score-credit-quality`. Mirrors the
   shipped perplexity re-score route exactly: admin credential reuse
   (`require_admin`, no new gate), hash-only ack, background pass, idempotent,
   resumable, `?limit=N`. Cross-tenant enumeration via the gate-driver reader
   pool (no tenant GUC); tenant-scoped UPDATE via the tenant pool touching only
   the `credit_quality_*` columns. Backfills the existing 349 rows and re-runs
   after any recalibration. One failure skips that row and continues; never
   aborts the pass.

**Recalibration is a versioned event:** changing any `*_CEIL`/`R_*` constant
bumps `credit_quality_calibration_version`; the batch pass recomputes every `q`
and re-stamps. Deterministic per `(inputs, version)`, so the pass is idempotent —
no "done" marker needed.

## Config / gating

- The scoring function is always on (pure arithmetic); it writes shadow values
  only. Nothing consumes `q` for payment in this spec.
- Batch route behind the admin credential; does nothing until called.
- Hash-only audit: counts, submission-ids as existing gate code does, error
  hashes. Never raw trace text, keys, URLs, or contributor identity.

## Testing

**Unit — the pure function** (`f`, `g`, `a`):
- monotonic non-decreasing in their input; concave (diminishing returns:
  successive equal input increments yield non-increasing output increments);
  `clamp01`; below-floor → 0; at/above-ceiling → 1.
- Determinism: same `(inputs, version)` → same `q`.
- Property-based layer: monotonicity, concavity, and multiplicative-collapse hold
  for randomly-sampled inputs across the domain, not only hand-picked points.

**Anti-gaming guarantees as assertions** — synthetic decision rows, one per
red-team attack, asserting `q(genuine) > q(any gamed)`:
- rare-token pump: high `ppl` (incl. the 1642M outlier), low `nov` → `q` LOW
  (multiplicative collapse; `f` saturates the outlier to 1.0 so it earns no more
  than a p90 trace).
- distinctive-token shim: high `nov`, low `ppl` → `q` LOW.
- peak parasite: high `peak`, low `rep` → `a` bites → `q` reduced/0 + reason.
- genuine: both mid-high, low spikiness → `q` HIGH.

**Column isolation** — the batch route writes only the `credit_quality_*`
columns; mirror the shipped re-score isolation test: in-memory double **and** a
real-Postgres test asserting perplexity/novelty/status/credit/vector columns are
byte-identical before/after.

**Distribution validation on real data** — compute `q` over the 27B-consistent
349 rows; assert **non-degeneracy** (spreads across `[0,1]`; not the 81%-zero
degeneracy the tail-fraction metric showed). This run also produces the
calibrated `PPL_CEIL`/`NOV_CEIL`/`R_SOFT`/`R_HARD` values that get pinned.

## Rollout

Ships with inline scoring active (shadow only) and the batch route behind the
admin credential. After merge + deploy: run the batch pass over the 349 rows,
inspect the `q` distribution and the anomaly-flagged set, pin the calibration
constants, re-run. No payout, no gating — pure observability ahead of the
settlement sub-projects.

## Residual risks (not covered here)

- Genuinely-novel-but-useless real work still scores; only `ground` or human
  judgment closes that.
- Fabricated-but-plausible traces (LLM-generated sessions) score like real work;
  neutralized only by the economic sub-projects + eventual replay grounding, not
  by this arithmetic.
- Semantic duplication / paraphrase farms are handled by dedup (sub-project 2),
  not here.
