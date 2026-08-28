# PII classify policy: `TRACE_COMMONS_PII_CLASSIFY_POLICY`

The NEAR AI privacy filter is a remote prose-PII classifier: it catches
names, addresses, and other unpatterned PII in free text that the
deterministic regex detectors cannot match. It runs in exactly one
production path: the [server-side PII backstop](pii-backstop.md). Submit-time
redaction (`rescrub_trace_envelope`) is deterministic-regex-only and never
contacts the remote classifier — this policy has no effect on it. This
runbook covers `TRACE_COMMONS_PII_CLASSIFY_POLICY`, the knob that controls
which events the backstop's classifier examines. Because the backstop is the
only place the remote classifier runs, this policy governs all of the remote
classifier's traffic, and the ~10x round-trip reduction below applies to all
of it.

## Why this knob exists

The classifier is slow and cannot be parallelized. Round trips are
sequential, at roughly 4.5 seconds each per window — that single number is
why a backstop driver tick over 5 traces takes 29-90 minutes. Concurrency
makes it worse, not faster: 4 requests spread over 18 seconds succeed, but
the same 4 requests issued simultaneously all fail in about a second, and
the failure briefly poisons subsequent traffic on the same API key. There is
no lever to make individual requests faster; the only lever is to send it
fewer requests.

Measured over 60 local sessions and 13.2M characters of real agent traces,
event volume breaks down as:

| Event kind | Share of characters |
|---|---|
| `ToolResult` (tool output) | 59.5% |
| `ToolCall` (tool args) | 30.1% |
| `AssistantMessage` (model prose) | 5.6% |
| `UserMessage` / contributor prose | 4.8% |

Contributor and model prose together are 10.3% of trace volume. Restricting
classification to prose-bearing events is therefore roughly a 10x reduction
in round trips against the same trace population. This breakdown was
measured on local agent-trace transcripts; the reduction realised on any
given deployment depends on that deployment's own mix of tool traffic to
prose, so confirm it against the recorded `events_examined` /
`events_skipped_by_policy` counts rather than assuming this figure holds.

## The two policies

| Value | Behavior |
|---|---|
| `all-events` *(default)* | Every event in the trace is sent to the classifier. Today's behavior. |
| `prose-only` | Only `UserMessage`, `AssistantMessage`, `Reasoning`, and `Feedback` events are sent. `ToolCall`, `ToolResult`, `RoutingDecision`, and `HttpExchange` events are skipped. |

An unset or unrecognized value yields `all-events`. This is deliberate: a
typo in the env var must never silently narrow what gets examined for PII —
the failure direction is toward examining more, not less.

## The accepted gap under `prose-only`

State this plainly to anyone deciding whether to enable `prose-only`: it is
a deliberate, documented trade-off, not an oversight.

Under `prose-only`, unpatterned PII arriving through tool output is no
longer examined by the remote classifier. For example, a file read that
returns a name and a street address in its contents will not be sent to the
privacy filter — the event kind is `ToolResult`, which the policy skips.

This does not remove PII protection from tool events entirely. The
deterministic detectors (regex-based) still run over **every** event
regardless of policy, so patterned secrets — API keys, tokens, emails, and
similar — are still caught in tool output under `prose-only`. What is lost
is coverage for the unpatterned, prose-shaped PII that only the remote
classifier can recognize, when it appears inside tool output rather than in
a message.

A test asserts this gap explicitly so it cannot silently regress into
covering more, or silently narrow to cover less, without the test failing.

## What gets recorded

The privacy filter summary carries three fields that make decisions made
under different policies distinguishable after the fact:

- `classify_policy` — the policy label in effect for that classification.
- `events_examined` — count of events actually sent to the classifier.
- `events_skipped_by_policy` — count of events withheld by the policy.

These are a label and two integer counts — never trace content, per this
repo's hash-only/label-only logging convention.

## Confirming the active policy

The ingest binary logs the resolved policy label at startup, at the top of
`main`, before other config validation runs. This means the log line
appears even if boot later aborts on an unrelated missing-control check —
it is the first place to look to confirm what a running (or crash-looping)
deployment is actually configured to do, without needing the process to
finish starting.

## Changing the policy

This is a config change, not a redeploy. Set the env var and restart the
service:

```sh
export TRACE_COMMONS_PII_CLASSIFY_POLICY=prose-only
```

Rollback is the same operation in reverse — set the var back to
`all-events` (or unset it) and restart. There is no migration, no data
rewrite, and no persisted state tied to the policy; it only governs which
events an in-flight classification call examines.

## See also

- [`pii-backstop.md`](pii-backstop.md) — the async server-side re-redaction
  pass that also drives calls through the same NEAR AI privacy filter.
- [`hash-only-logging.md`](hash-only-logging.md) — the logging conventions
  this doc follows for `classify_policy` / `events_examined` /
  `events_skipped_by_policy`.
