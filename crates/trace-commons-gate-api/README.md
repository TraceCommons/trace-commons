# trace-commons-gate-api

Public contracts for the Trace Commons gate: the scorer traits, their result
types, the gate decision types, and a reference implementation.

Any scoring backend — including proprietary ones outside this repository —
implements these traits. Changes here are a compatibility commitment; treat the
trait surface as versioned.

The versioned pipeline's phase policies (`pipeline::AdmissionPolicy`,
`ReviewPolicy`, `ScorePolicy`, and `SettlePolicy`) are contracts in this crate
too. A substituted backend can supply its own `ScorePolicy` to the server
through `trace_commons_server::versioned_pipeline_score_registry::ScorePolicyRegistry`.
That registry is a factory keyed by the Score `implementation_id` that a bundle
manifest names. The server holds the policy only as a trait object, and the
factory is unqualified for production unless it declares otherwise. The
factory and the registry are server wiring, and they stay out of this crate.

Two parts of the Score contract are there for such a policy:

- `ScoreEvidence::award_adjustments` is a list of `AwardAdjustment` entries.
  Each one has a safe `label`, a `factor_micros` from 0 to 1,000,000, and an
  optional `basis_micros`. The policy lists them in the order it applied them,
  so an auditor can recompute the award. The list is optional and is not
  serialized when it is empty, so existing evidence keeps its bytes.
- `ScoreOutput::held_for_review` asks for the trace to be held for operator
  review instead of priced. The output awards nothing and carries a safe
  label (`ScoreOutput::hold_for_review`). An output built with
  `ScoreOutput::new` never holds.

The server runs a registered policy under the same Score lock, duplicate
checks, and gate decision row as its compatibility policy. The gate decision
row needs `perplexity_micros`, `tail_fraction_micros`,
`novelty_score_micros`, `quality_passed`, `novelty_passed`, and
`nearest_neighbor_hash` on the evidence.

## Reference implementations

`reference::ReferencePerplexityScorer` and `reference::ReferenceEmbedder` are
real but deliberately simple: byte-entropy perplexity and a feature-hashed
bag-of-tokens embedder. They are uncalibrated and materially weaker than a
production backend. They exist so the open reference server can gate traces
without one.

`Mock*` types in `trace-commons-gate-enclave` are hash-derived test doubles.
They are not scorers and must not be used to gate anything.
