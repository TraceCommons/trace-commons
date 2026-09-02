# Clearing quarantine whose cause has been fixed

The pilot holds 151 quarantined submissions. Most are held at High for a cause
that PR #506 removed. Requeueing them does not release them, and the obvious
fix — letting a clean pass supersede a stale prior — is not available.

This specifies an operator action that resets the stored prior risk, leaving the
risk ratchet exactly as it is.

## Why the ratchet cannot be relaxed

Risk resolves as `max(prior, derived)` unless `can_downgrade()`:

```rust
fn can_downgrade(&self) -> bool {
    self.complete_coverage && self.useful_classifier_result && self.residual_clean()
}
```

`useful_classifier_result` requires the classifier to have FOUND something. On a
re-run of a trace whose credential #506 already removed, it finds nothing, so
the assessment cannot downgrade and the stale High survives.

**Relaxing that conjunct was attempted and rejected.** The design considered was
to redefine it as "a classifier examined this", arguing that the driver's canary
already establishes liveness. Implementing it failed three existing tests, one
named `canary_healthy_but_no_findings_cannot_lower_high_risk`, whose comment
reads:

> Under the previous rule (healthy canary => trust the emptiness) this case
> downgraded; findings are now the only evidence.

The repository keeps a `CanaryHealthyButFindsNoRealPii` fixture precisely to
demonstrate the bypass is constructible: a classifier can pass a synthetic probe
and still silently fail on real content. That rule was tried, found unsafe, and
removed. It must not come back. See
`2026-09-01-residual-risk-supersede-design.md`, retained as the record of the
rejected approach.

So no automated signal available on that path can distinguish "this trace is
clean" from "nothing looked at it properly". The escape hatch has to be a human
assertion.

## The design

`POST /v1/admin/pii-backstop-clear-stale-prior-risk?limit=N`

Admin-only, bounded, count-only response. For each qualifying submission it
resets the stored prior residual risk to `Low` and moves it to
`awaiting_pii_backstop`, so the backstop derives a fresh verdict with nothing
stale to max against.

Nothing about `resolve_post_scrub_risk`, `can_downgrade`, or the forced-High set
changes. The privileged, audited act is an operator saying "I know #506 removed
this cause" — a claim a person is accountable for, not an inference from
silence.

### Eligibility, and why it is this narrow

A submission qualifies only when ALL hold:

1. `status = 'quarantined'`.
2. An active `rescrubbed_envelope` ref exists — what the driver will actually
   read, and what makes the row enumerable (its `submitted_envelope` ref is
   invalidated and must stay so).
3. The recorded `residual_risk_basis` contains `residual_survivor`.
4. The basis contains **no** `key_finding` and **no** `coverage_incomplete`.

Condition 3 scopes the action to the population #506 fixed. Condition 4 is the
important one: those two causes are not fixed by #506 and re-derive on every
pass anyway — keys are never rewritten, and incomplete coverage forces High
before `can_downgrade` is consulted. Including them would reset a prior that is
about to be re-derived identically, achieving nothing while widening a
privileged operation.

A submission failing any condition is left untouched and counted separately in
the response, so a caller can see the action declined rather than silently
skipped.

### What resetting to Low does NOT do

It does not accept the trace. The backstop re-derives from scratch, and the
observed population re-derives to **Medium**, not Low, because their PII was
found and removed — that floor is preserved. They release only because
`TRACE_COMMONS_ACCEPT_MEDIUM_RISK_SUBMISSIONS=true` is already configured on the
pilot (via a systemd drop-in). This design does not decide that policy; it
removes a stale input so the existing policy applies to a correct verdict.

A trace whose cause is still present re-derives to High and re-quarantines. The
reset gives it a fair re-assessment, not a pass.

## Audit

The reset is a privileged act on the record of a privacy decision, so it must be
reconstructable afterwards:

- One audit row per affected submission, hash-only per repo convention, naming
  the actor principal, the prior risk being cleared, and the basis that
  qualified it.
- The route logs a count and the limit, never submission ids or tenant content.
- The existing `residual_risk_basis` is left in place, not blanked. The record of
  what the previous pass concluded is evidence, and overwriting it would destroy
  the ability to tell later whether this action was justified.

## Testing

1. A quarantined submission with `residual_survivor` and an active rescrubbed
   ref is reset and re-enumerated.
2. One whose basis includes `key_finding` is **not** reset.
3. One whose basis includes `coverage_incomplete` is **not** reset.
4. One with no `residual_survivor` is **not** reset.
5. One with no active `rescrubbed_envelope` ref is **not** reset — it would be
   unreachable by the driver anyway.
6. The route is admin-only; a contributor token gets 403.
7. An omitted `limit` defaults to a sample, not to everything.
8. `resolve_post_scrub_risk` and `can_downgrade` are unchanged — assert the
   existing rejection tests still pass, including
   `canary_healthy_but_no_findings_cannot_lower_high_risk`.

Test 8 is not ceremonial. The entire justification for this design is that the
ratchet stays intact; a change that quietly relaxed it while adding this route
would defeat the point.

## Rollout

1. Merge, deploy.
2. Run with `limit=10`. Confirm they re-derive to Medium and accept, and that
   the quarantined count falls by the same number.
3. Only then run the remainder, in batches.

Sample first. Three hypotheses about this population have already been wrong —
that the verdicts were classifier-degradation artifacts, that the survivors sat
in `human_correction`, and that a trustworthy pass could be allowed to supersede
the prior. Each looked convincing until measured.

Throughput is not a constraint: #502's per-submission budget prevents a single
large trace monopolising the driver, which was the only reason the previous
drain needed a GPU. If it proves slow, `scripts/operator/gpu-privacy-filter-batch.sh`
spins one up in about ten minutes.

## Out of scope

- **`structured_payload.cmd` survivors.** Tracked separately; those traces
  legitimately still carry a residual finding and must not be reset by this
  action. Condition 3 does not exclude them, but their fresh pass re-derives
  High, so they re-quarantine correctly.
- **The stale `privacy_risk` column.** Quarantined rows read `medium` while the
  decision that quarantined them used High. An observability defect worth
  fixing, but it changes no behaviour and is not required here.
