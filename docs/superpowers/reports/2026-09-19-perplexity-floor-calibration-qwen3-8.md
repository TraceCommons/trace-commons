# Perplexity floor calibration for Qwen/Qwen3.8-27B

Date: 2026-09-19
Status: **recommended, not yet applied.** Update this line when the pilot's
`TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` changes.

Recommended floor: **1,500,000 micros (1.5).** Previous: 6,000,000 (6.0),
calibrated for `Qwen3.6-27B-FP8`.

## Why this was needed

The perplexity floor is a property of the scorer model. On 2026-09-09 the
pilot's scorer moved from `Qwen/Qwen3.6-27B-FP8` to `Qwen/Qwen3.8-27B`,
because the 3.6 endpoint had stopped answering. The floor stayed at 6.0.

Daily aggregates from `trace_gate_decisions`, scored rows only (a row with
`perplexity_micros = 0` that is marked passed never had perplexity computed
and is excluded):

| period | scored | failed the floor | median of scored rows |
|---|---|---|---|
| 08-31 to 09-02 (3.6) | 45 | 2 (4%) | about 19 to 25 |
| 09-03 to 09-08 | 0 | none | nothing scored; the 3.6 endpoint was already dead |
| 09-09 to 09-19 (3.8) | 307 | 184 (60%) | about 3 to 6.5 |

Under 3.8 the same floor refused 60% of scored traces. `perplexity_passed`
feeds admission and credit.

## Method

This does not reuse `docs/operator/a27-perplexity-floor-calibration.md`.
That procedure calibrated from the bake-off corpus, whose classes were
separable by source format alone (#204, #205), and the bake-off scores a
text-only rendering while the gate scores rendered envelope events, tool
results included, in a token-weighted aggregate.

Instead:

1. **Real traces, from production's own scoring.** The gate stores
   `perplexity_micros` on every row it scores, pass or fail, so the rows
   scored since the switch already are the 3.8 distribution over real
   traces, computed by the production packer and aggregate. Read through the
   column-scoped `trace_gate_driver` role, aggregates only.
2. **Junk anchors.** Nine synthetic junk envelopes, rendered in the
   canonical `event_type (tool): content` form and scored on the same model
   with the production arithmetic (`logprobs[1..]`, generated token
   included). Each is a single chunk, so the packer does not enter.
3. **Placement.** Whole-trace perplexity falls as traces get longer and is
   set largely by tool output, so its defensible job is to refuse junk, not
   to grade work: put the floor just above the junk it can separate.

## Real traces under 3.8

307 scored rows, `decided_at >= 2026-09-09 05:49:19Z`, `perplexity_micros > 0`:

| n | min | p05 | p10 | p25 | p50 | p75 | p90 | p95 | max |
|---|---|---|---|---|---|---|---|---|---|
| 307 | 1.65 | 2.17 | 2.44 | 3.19 | 5.14 | 7.85 | 13.73 | 23.99 | 182.42 |

Share of those traces each candidate floor would refuse:

| floor | 1.0 | 1.5 | 2.0 | 2.5 | 3.0 | 3.5 | 4.0 | 5.0 | 6.0 |
|---|---|---|---|---|---|---|---|---|---|
| refused | 0 | 0 | 7 | 35 | 70 | 93 | 114 | 149 | 184 |
| share | 0.0% | 0.0% | 2.3% | 11.4% | 22.8% | 30.3% | 37.1% | 48.5% | 59.9% |

By trace length:

| chunks scored | n | p10 | p50 | p90 | median peak chunk | under 6.0 |
|---|---|---|---|---|---|---|
| 1 | 19 | 4.37 | 6.94 | 27.34 | 6.94 | 42.1% |
| 2-3 | 8 | 4.63 | 13.57 | 29.96 | 28.38 | 25.0% |
| 4-8 | 18 | 3.74 | 7.14 | 41.25 | 43.07 | 44.4% |
| 9-15 | 24 | 2.68 | 5.45 | 10.46 | 45.14 | 54.2% |
| 16 (the cap) | 238 | 2.29 | 4.62 | 11.23 | 37.96 | 64.3% |

78% of scored traces hit the 16-chunk cap, and those have the lowest
whole-trace perplexity. A floor on this signal refuses long traces first.
Their peak-chunk perplexity is high: the content is there, diluted.

## Junk anchors under 3.8

| synthetic envelope | scored tokens | perplexity |
|---|---|---|
| scaffold only: 60 tool events with no content | 330 | 1.14 |
| empty after redaction | 220 | 1.39 |
| one assistant line repeated 40 times | 440 | 1.94 |
| a pasted MIT licence | 127 | 2.58 |
| greeting only | 18 | 3.57 |
| greeting, three turns | 43 | 4.74 |
| a refusal | 28 | 5.95 |
| a bare login prompt | 34 | 8.19 |
| random characters | 49 | 104.71 |

## Reading

- **Structural junk separates.** Empty and scaffold-only envelopes score
  1.14 to 1.39; every real trace scores 1.65 or more. A floor of 1.5 sits in
  that gap: it refuses none of the 307 real traces and catches both.
- **Conversational junk does not.** Greetings, a refusal and a login prompt
  score 3.6 to 8.2, at or above the median of real long traces (4.62). A
  floor high enough to catch a bare greeting would refuse roughly half of
  real work. These envelopes are 18 to 43 scored tokens long; a minimum
  scored-token rule would catch them and perplexity cannot.
- **Random characters score 105.** No floor catches junk that is surprising.

So 1.5 is the largest floor that refuses no real pilot trace and still does
the one job this signal can do under this model. A floor of 2.0 would also
catch repetition junk (1.94) at the cost of 7 real traces (2.3%).

## Limits

- The junk anchors are synthetic, nine in number, and short-text perplexity
  is noisy. No labeled junk from production was available.
- 307 traces over ten days from one pilot population.
- The separating gap is 1.39 to 1.65. It is narrow. 1.5 is a holding value,
  not a settled one.
- 78% of the sample is capped at 16 chunks, so the distribution describes a
  strided subset of most traces rather than their whole content.

## Follow-ups

- The 184 traces refused between 2026-09-09 and 2026-09-19 were scored under
  3.8, so re-deriving `perplexity_passed` for them under a corrected floor is
  legitimate. The re-score route has no date filter, and a full pass would
  also rewrite rows scored under 3.6. What a failed perplexity gate did to
  credit for those traces has not been established. Decide both before
  acting.
- Conversational junk needs a structural check (minimum scored tokens, or a
  substance judgement), not a perplexity floor.
- The per-author signal (#965, #966) and the dry-run re-score (#967) exist to
  find out whether perplexity over the agent's own prose can grade work
  where the whole-trace value cannot. Until that is calibrated, the floor's
  job stays what it is here: refusing structurally empty envelopes.
