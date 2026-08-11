# Credit scoring: privacy risk is penalised twice

**Date:** 2026-08-07
**Status:** Proposed
**Scope:** `compute_value_scorecard` in `trace-commons-protocol`.

## The finding

Every medium-risk submission in the pilot corpus has scored exactly zero.

```
privacy_risk | status   | count | avg submission_score | max credit
-------------+----------+-------+----------------------+-----------
low          | accepted |   331 | 0.169                | 5.20
medium       | accepted |    10 | 0.000                | 1.00
high         | accepted |     3 | 0.000                | 0.50
high         | rejected |     1 | 0.000                | 0.00
```

Ten of ten. Not a distribution with a low tail — a constant.

## Why

```rust
let raw = gate
    * schema_validity
    * (0.25 * quality + 0.20 * replayability + 0.20 * novelty
       + 0.15 * coverage_bonus + 0.10 * difficulty + 0.10 * user_correction_value)
    - 0.40 * duplicate_penalty
    - 0.60 * privacy_risk;
```

`gate` and `privacy_risk` are **both pure functions of the same enum**:

| `residual_pii_risk` | `privacy_gate` | `privacy_risk_score` |
|---|---|---|
| Low | 1.0 | 0.0 |
| Medium | 0.5 | 0.5 |
| High | 0.0 | 1.0 |

So the formula multiplies the quality terms by one function of residual risk and
then subtracts another function of the same residual risk. One signal, two
penalties. For medium that is a halving *and* a flat `-0.30`:

```
medium:  raw = 0.5 * T - 0.30      (T = the weighted quality terms, max 1.0)
```

which needs `T > 0.6` to clear zero. `T` cannot realistically get there:
`replayability` is `1.0` only when `envelope.replay.replayable`, which a
recorded coding session generally is not, so 0.20 of the weight is
unavailable before anything else is measured. That leaves `quality` (capped at
0.25), `novelty`, `coverage_bonus`, `difficulty` and `user_correction_value`
needing to average near their maxima simultaneously.

Empirically they never have.

## Consequences

**The medium band is dead.** A submission can be accepted and earn nothing,
with no explanation distinguishing it from one that earned little. The
contributor sees `accepted` and `0.00`.

**It contradicts the accept flag.** `TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS`
exists to admit medium-risk work into the corpus. Scoring then guarantees that
work is unrewarded. Those two settings encode opposite intentions: one says
this contribution is worth having, the other says it is worth nothing.

**It is invisible from the leaderboard side.** The `accepted` credit-ledger row
is written only when `credit_points_pending > 0.0`, so a medium-risk
contributor cannot appear on the community leaderboard at all, no matter how
much they contribute. That is how this was found.

## Proposal

Drop the subtractive privacy term. Keep the gate.

```rust
    - 0.40 * duplicate_penalty;
```

The gate is the better of the two mechanisms: it is proportional, so risky
work is worth *less* rather than worth *less minus a constant*, and it already
expresses the full policy — including `0.0` for high risk, which the explicit
`credit_points_estimate` branch reinforces.

Because `privacy_risk_score(Low) == 0.0`, this changes **nothing** for low-risk
submissions — the 331 accepted rows above keep their exact scores. High risk is
independently zeroed by the branch below. **Only the medium band moves**, from a
guaranteed zero to `0.5 * T`, which for a typical observed `T` lands around 2
credit points out of 10.

## Deliberately not changed

**The unreachable top of the scale.** Nothing in the corpus exceeds 5.20 of a
possible 10, and with `replayability` at 0 for recorded sessions the ceiling is
8.0 before any other term falls short. That may be intended — 10 reserved for
replayable, novel, high-coverage work — but it means the scale's top half is
decorative for the current contributor population. Re-weighting is a policy
question about what credit is *for*, not a defect, and wants a deliberate
decision rather than a patch.

**The reviewer bypass.** Three high-risk submissions carry credit up to 0.50,
which the scorecard formula cannot produce. They came through
`reviewer_credit_for_record` on the approval path, which does not consult the
scorecard. Whether a reviewer should be able to award credit the formula
refuses is a real question; it is not this change.

## Testing

- a medium-risk envelope earns non-zero credit where it previously earned zero
- a low-risk envelope's score is unchanged to the cent
- a high-risk envelope still earns zero
- the ordering low > medium > high holds for otherwise identical envelopes
