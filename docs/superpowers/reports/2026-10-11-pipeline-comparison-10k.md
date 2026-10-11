# Pipeline comparison report

- Branch revision: `vp/pipeline-comparison` at `aef9e7fc41b118164c8a751ebb996b86c1473014`.
- Pin: `docs/superpowers/specs/fixtures/versioned-pipeline-comparison-hf-pin-v1.json`.
- Run time: 3,862 seconds (1 hour 4 minutes), ended 2026-10-11T00:59Z.
- Target triple: `x86_64-unknown-linux-gnu`.
- Build profile: `release`.
- Local evidence with the reference scorer and embedder; not a production promotion.

Check: `pipeline_comparison_hf`

Bundle: `sha256:d6d82fa248403bdbcc764ff904e6e347b0edf905ec5519145c2ee598b4c62ca1`

Partial run: no. Skew: none.

## Pin

| Digest | Value |
| --- | --- |
| source_digest | `sha256:ebd5a801387ff010376c97b9aadccb2a4802a4e31de4a17fc4d3400d21d859ca` |
| order_digest | `sha256:a9d891cee558d7278252fe34fc6016fecbfeeced62f5d27d387617139b8d0476` |
| configuration_digest | `sha256:6950d05feb899f7a50388680686a67e222ee54cdd39f9fd515898034ff1218c6` |
| bootstrap_corpus_digest | `sha256:55fb790021cbc88f7b6a5e037ec803ab8333597e3734ea0bce83f59fde1b30cd` |
| holdout_corpus_digest | `sha256:dceb006220fea3b736e901a6999755dabdc65afb3ee056000d2f41f668f994fd` |

## Floors

| Floor | Micros |
| --- | --- |
| perplexity_floor_micros | 26979646 |
| tail_fraction_floor_micros | 207331 |
| novelty_floor_micros | 56404 |

## Counts

| Count | Traces |
| --- | --- |
| In the pin | 10000 |
| Compared | 10000 |
| Equal | 9852 |
| Permitted | 148 |
| Unexplained | 0 |

## Permitted differences

- `medium_risk_privacy_review` (ruling.PC-D22): 147
- `high_risk_admission_reject` (ruling.PC-D27): 1

## Distribution

| Count | Baseline | Candidate |
| --- | --- | --- |
| admit | 9999 | 9852 |
| quarantine | 1 | 147 |
| reject | 0 | 1 |
| refused | 0 | 0 |
| other | 0 | 0 |
| scored | 9999 | 9999 |
| quality_passed | 1811 | 1811 |
| quality_failed | 8188 | 8188 |
| novelty_passed | 8264 | 8264 |
| novelty_failed | 1735 | 1735 |
| member | 1321 | 1321 |
| not_member | 8678 | 8678 |
| chunks_capped | 611 | 611 |

## Unexplained fields

None.

Production blockers: local_test_only, local_reference_scorer, local_reference_embedder, synthetic_index, synthetic_settlement, static_bearer_authentication, deterministic_privacy_only, baseline_derived_scan_removed, duplicate_short_circuits_not_compared, review_start_privacy_pass_not_compared.

## Findings outside the compared fields

On each receipt, the old path reads and compares every earlier derived record of the tenant, so the cost for N traces grows with N squared (issue #1307).
The harness removes the baseline tenant's derived files after each trace (PC-D18, the blocker `baseline_derived_scan_removed`).
Thus this run does not measure that cost.
The plan review estimates the cost at 5 to 19 hours for 10,000 traces in an optimized build; this estimate is not measured.
