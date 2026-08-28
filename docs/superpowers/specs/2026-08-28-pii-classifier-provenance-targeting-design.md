# Provenance-targeted PII classification

Status: design approved 2026-08-28. Server-side only; no protocol change.

## Problem

The NEAR AI privacy classifier runs on every window of every event. Throughput
is therefore `windows x round-trip`, and the round trip is ~4.5 s. A 50-window
trace takes ~3.75 min; a 5-trace batch ~19 min, which accounts for the 29-90
minute driver ticks recorded in issue #475. The backlog grows faster than it
drains.

Every other lever has been tried and failed. Chunk size, host CPU, batching,
and token budgeting changed correctness or cost but not throughput.
Concurrency is unavailable: four requests that pass when spread over 18 s fail
simultaneously, sub-second, and the trip poisons subsequent serial traffic on
the same key. The limit is enforced per API key, not per connection.

The only remaining lever is issuing fewer windows.

## Two measurements that shaped this

**Deterministic "provably inert" skipping is worthless here.** Only 0.8% of
windows lack a three-word alphabetic run and only 5.7% have fewer than six
consecutive words, measured over 50,739 windows from 60 local sessions. Agent
traces are overwhelmingly English prose. Thresholds loose enough to matter
(28% at `run < 8`) are already loose enough to hide `John Smith lives at 12 Oak
Street`. This tier was designed and then abandoned on the evidence.

**Provenance is the real lever.** Across 60 sessions and 13.2M characters:

| bucket | share of volume |
|---|---|
| tool_output | 59.5% |
| tool_args | 30.1% |
| model_prose | 5.6% |
| contributor_prose | 4.8% |

Contributor and model prose together are **10.3%** of volume. Classifying only
prose-bearing events is roughly a **10x** reduction in round trips, and it
stacks with the L1 window cache (#477), which measured 57.5% duplicate windows
on the same corpus.

Both figures come from local Claude Code transcripts. Most pilot traces
originate from that machine, so they should transfer, but the pilot hit rate is
a prediction until the deployed telemetry confirms it.

## Design

### Selection

The classifier pass iterates only events whose `event_type` is prose-bearing:

- classified: `UserMessage`, `AssistantMessage`, `Reasoning`, `Feedback`
- skipped: `ToolCall`, `ToolResult`, `HttpExchange`, `RoutingDecision`

Skipping covers both `redacted_content` and `structured_payload`; the latter
today goes through `classify_structured_payload_node` for every event.

`TraceContributionEventType` already carries this discriminator, so no envelope
field is added. This matters: an envelope change would move the golden digest
pinned in the contributor crate and force a client release.

### What does not change

The deterministic detectors still run over every event. Patterned secrets in
tool output -- keys, tokens, emails, paths -- are caught exactly as they are
today. Only the model pass narrows. The post-scrub residual scan is untouched.

### Coverage semantics

Today "complete coverage" asserts that all text was examined. Under this design
it asserts that all prose-bearing text was examined, while
`resolve_post_scrub_risk` continues to grant High-to-lower downgrades on that
basis. This is a real weakening of the claim and is recorded rather than
silent: the privacy filter summary carries the policy version and the counts of
events examined versus skipped by provenance.

Counts and a policy label only -- no event content, no identifiers -- per the
repo's hash-only audit convention. The purpose is that decisions made under the
old and new policies remain distinguishable after the fact.

### Configuration

`TRACE_COMMONS_PII_CLASSIFY_POLICY` selects `all-events` (current behaviour)
or `prose-only`. Rollback is then a config change rather than a redeploy.

It defaults to `all-events`, per the repo's fail-closed convention: an operator
who has not made the decision keeps today's behaviour. The pilot sets
`prose-only` explicitly. The same string is the policy label recorded in the
summary, so the recorded value and the configured value cannot drift.

### The accepted gap

Unpatterned PII arriving through tool output -- a file being read that contains
a name and a street address -- is no longer model-examined. Patterned secrets
there are still caught deterministically.

This gap is accepted deliberately, not overlooked, and it is falsifiable: sample
tool-output windows through the classifier offline and count prose PII the
detectors missed. If that rate is material, the follow-up is a recall-oriented
local screen over tool output that escalates flagged windows (approach B).
That decision should rest on the measurement, not on assumption -- the same
discipline that overturned the inert-window tier above.

## Testing

- Selection: each event type routes to classified or skipped as specified.
- `structured_payload` of a skipped event is not submitted.
- **PII in a `ToolResult` is not model-examined.** This encodes the accepted gap
  as a deliberate, visible decision so it cannot regress into a silent bug.
- Summary records policy version and examined/skipped counts.
- `all-events` policy reproduces current behaviour exactly.

## Out of scope

A local screening model, changes to the deterministic detectors, changes to
chunking or the token budget, and any change to the envelope format.
