# Transient-failure policy for the perplexity scoring driver

Sibling of `2026-08-26-transient-failure-policy-design.md`, which fixed the
same defect in the PII backstop driver. This document records the
investigation of the scoring driver, the finding that the defect is real
there too, and the shape of the fix.

## The defect, confirmed from the code

`score_one_submission` calls
`bump_gate_evaluation_attempt_and_log_exhaustion` from seven sites. Six of
them are `db_mirror` errors from the two cost-control branches
(skip-duplicate lookup, cross-submission cache lookup, decision insert) plus
"trace submission not found". The seventh, branch 3, is the delegation to
`evaluate_and_record_gate`, which is the only site that reaches an upstream:

    evaluate_and_record_gate
      -> TraceGateService::evaluate_trace
      -> EnclaveGateOrchestrator::chunk_and_score_perplexity
      -> PerplexityScorer::score_chunk
      -> NearAiPerplexityScorer::fetch_logprobs  (HTTP to NEAR AI)

Every error on that path -- a connect failure, a read timeout, a 502, a 402
Payment Required -- arrives at branch 3 as an opaque `anyhow::Error` and is
charged to the submission's `trace_gate_evaluation_attempts` row. Once
`attempts >= max_attempts`, migration V36's `MAX_ATTEMPTS` filter in
`list_submissions_needing_gate_decision` stops returning the row, and nothing
ever re-enumerates it. The submission is permanently unscored. That is the
same permanent consequence the backstop driver had, reached the same way, for
failures that have nothing to do with the trace.

Two things make it worse here than in the backstop driver:

- `NearAiPerplexityScorer` has **no retry policy at all**. There is no
  equivalent of the privacy adapter's `MAX_CLASSIFY_ATTEMPTS = 4` with
  backoff: one send failure is one spent attempt. The adapter absorbs three
  502s before it charges anything; the scorer charges on the first.
- The orchestrator scores a trace chunk by chunk and fails the whole
  evaluation on the first chunk's error, so a large trace has more
  independent chances to catch a bad second.

Production corroborates it: 63 submissions at `attempts >= 5` across three
tenants, 58 of them accumulated during the 402 outage.

## What is transient

The rule is "may this failure be charged to the trace?", and the answer is
yes only when the trace or the request built from it is what the backend
objected to. The classification is made once, at the HTTP boundary in
`fetch_logprobs`, and carried out as a type.

Permanent (charged to the trace):

- `400`, `413`, `422` -- the request we built from this trace was rejected:
  prompt too long for the context window, malformed, unprocessable. Scoring
  it again produces the same answer.
- Non-UTF-8 plaintext, unparseable response body, missing/empty logprobs, a
  null logprob past position 0. Body-shape errors are permanent by the same
  reasoning the backstop adapter uses.

Transient (not charged):

- Transport errors and timeouts (`NearAiScorerHttpSendFailed`) and body-read
  failures (`NearAiScorerHttpBodyReadFailed`).
- Every `5xx`.
- `402 Payment Required`, `408 Request Timeout`, `429 Too Many Requests` --
  account and rate conditions. 402 is the status that caused the incident;
  classifying it permanent would leave the actual outage unfixed.
- Every other `4xx`, including `401`, `403`, `404`. None of them is the
  trace's fault: they describe a broken key, a revoked account, or a
  misconfigured endpoint. A deployment in that state should stall its queue
  loudly, not quietly shred it. The circuit breaker below bounds the cost of
  that choice.

This is a deliberate divergence from the backstop adapter, which treats all
`4xx` as permanent. The scoring path is stated as an allowlist of
trace-attributable statuses rather than a `is_server_error()` test, because
that is the question actually being asked.

## Mechanism

`trace-commons-gate-api` gains `perplexity::ScorerFailure`, a two-variant
error with an `is_transient()` accessor, exactly mirroring
`TraceContributionError`'s shape. It lives in gate-api because that is where
gate contracts live and because `trace-commons-gate-enclave` -- where the
scorer is -- depends on gate-api and not on `trace-commons-protocol`, so the
protocol crate's `TraceContributionError` is not reachable from the scorer
and would in any case be the wrong name for a scoring failure. It is the same
mechanism, not a parallel one: a typed variant read with a downcast, never a
string match.

`ScorerFailure`'s `Display` is its `reason` verbatim, so the labels written
to `trace_gate_evaluation_attempts.last_error` stay byte-identical to
today's and remain comparable with rows already in production.

`scorer_status_is_transient(u16)` also lives in gate-api, unconditionally
compiled, so the status table above is unit-tested by the default-features CI
job rather than only by the `near-ai-scorer` build that CI never runs tests
for.

The driver reads the classification with
`is_transient_gate_scoring_failure(&anyhow::Error)`, a `downcast_ref` that
sees through the orchestrator's `.context("PerplexityScorerInferenceFailed")`
and `evaluate_and_record_gate`'s `?`. A downcast miss is permanent, which
preserves today's behaviour for every error that is not the scorer's HTTP
boundary.

`GateOutcome` gains a `TransientFailed { label }` variant so the tick can
tell the two apart. `evaluate_and_record_gate` never produces it; only
`score_one_submission` does.

## Circuit breaker

The same reasoning applies as in the backstop driver: once transient failures
cost the trace nothing, a dead NEAR AI endpoint makes every tick re-enumerate
and re-hit the same batch forever, at real money per request.
`MAX_CONSECUTIVE_SCORE_DRIVER_FAILURES = 3` consecutive per-item failures end
the tick and leave the rest of the batch untouched for the next one, matching
`MAX_CONSECUTIVE_PII_BACKSTOP_FAILURES`. `PerplexityDriverTickSummary` gains
`transient` and `breaker_tripped` so a short tick is distinguishable from a
short batch.

## Deliberately not changed

The six DB-error call sites are **not** given this treatment. A pool error
there is not the trace's fault either, so the same argument formally applies,
but the exposure is far smaller and the fix would be speculative: the bump
travels through the same `db_mirror` as the failed read, so a DB outage fails
the bump too (`if let Ok(attempts)`) and charges nothing. Only an
intermittent, read-specific failure could charge a trace, and there is no
production evidence of it. Recorded as a follow-up rather than fixed here.

The scorer's missing retry/backoff policy is also left alone. It is a real
gap -- the adapter's `MAX_CLASSIFY_ATTEMPTS` has no counterpart -- but it is a
separate change with its own cost profile (retries inside a
`spawn_blocking` hold a blocking-pool thread, and each retry is a paid
inference call on a possibly large chunk). This fix removes the harm the gap
causes; closing the gap itself is its own issue.
