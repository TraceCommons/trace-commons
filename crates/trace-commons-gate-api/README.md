# trace-commons-gate-api

Public contracts for the Trace Commons gate: the scorer traits, their result
types, the gate decision types, and a reference implementation.

Any scoring backend — including proprietary ones outside this repository —
implements these traits. Changes here are a compatibility commitment; treat the
trait surface as versioned.

## Reference implementations

`reference::ReferencePerplexityScorer` and `reference::ReferenceEmbedder` are
real but deliberately simple: byte-entropy perplexity and a feature-hashed
bag-of-tokens embedder. They are uncalibrated and materially weaker than a
production backend. They exist so the open reference server can gate traces
without one.

`Mock*` types in `trace-commons-gate-enclave` are hash-derived test doubles.
They are not scorers and must not be used to gate anything.
