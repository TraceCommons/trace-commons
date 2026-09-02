# Credentials surviving in structured tool payloads

PR #506 made the PII backstop run the deterministic credential sweep over
`event.redacted_content`, because the classifier is trained on prose PII and
produces no span for an AWS key or a bearer token. It deliberately did **not**
do the same for `event.structured_payload`.

This records why, what the remaining exposure is, and what a fix has to resolve
before it can be written.

## The evidence

The path recording added in #505 logged, from a live requeued submission:

```
residual_secret_locations=["envelope.events[].structured_payload.cmd"]
```

So a High/Critical secret is sitting in a tool payload after the backstop has
run. The residual scan walks every string leaf of the serialized envelope, so it
sees it; the backstop rewrites `structured_payload` from the classifier's spans
alone, so nothing removes it. Same shape as the `redacted_content` defect #506
fixed.

That trace quarantines, correctly. It is not a leak — the control is doing its
job — but it is a trace held for a secret the pipeline can detect and does not
remove.

## Why #506 did not simply extend the sweep

The synchronous path does not redact payloads blindly. It calls:

```rust
redactor.redact_json_value(event.tool_name.as_deref(), &event.structured_payload, &mut state)
```

Redaction there is **tool-aware**. Passing the tool name is not decoration: it
means some payloads, or some fields within them, are treated differently
depending on which tool produced them. A blind deterministic sweep over every
payload leaf would ignore that entirely and could destroy content a tool
contract requires — which is a different kind of damage from leaving a secret
in, and not obviously a smaller one.

`cmd` is exactly the field where this bites. A shell command is the most
likely place for a credential to appear inline, and also the field whose exact
text is most load-bearing for a trace's usefulness as training data.

That caution was reasonable when #506 shipped, and the investigation below
resolves it: the tool-aware function already performs the sweep, so applying it
is not a new policy and cannot bypass per-tool handling. What remains is a
narrower judgement about whether a redacted command is worth keeping -- and the
sync path has already answered it one way.

## The fix

**Question 1 is answered: `redact_json_value` already sweeps secrets.** It
chains three passes and the last one is the deterministic sweep over every
string leaf:

```rust
fn redact_json_value(&self, tool_name, value, state) -> (Value, RedactionReport) {
    let tool_redacted   = redact_tool_specific_payload(tool_name, value, &mut report);
    let keyed_redaction = redact_sensitive_json(&tool_redacted);
    count_sensitive_field_redactions(&tool_redacted, &keyed_redaction, &mut report);
    let redacted        = self.redact_json_strings(keyed_redaction, state, &mut report);
    (redacted, report)
}
```

`redact_json_strings` calls `redact_text_with_state` per leaf -- the same
function whose credential detection #506 added to the prose path.

So the async backstop is not missing a *policy*, it is missing a *call*. It
rewrites `structured_payload` from `classify_structured_payload_node` alone and
never runs the deterministic pass afterwards.

The fix mirrors #506 exactly: after the classifier rewrites the payload, run

```rust
redactor.redact_json_value(event.tool_name.as_deref(), &payload, &mut state)
```

and merge the report.

This **is** the tool-aware path, so the concern that motivated deferring it
dissolves: `redact_tool_specific_payload` still runs first and still honours
whatever per-tool handling exists. There is no blind sweep and no new exemption
list to design, because the existing function already encodes both.

### What remains open

Only one thing, and it is a judgement rather than a design:

- **Is a partially-redacted `cmd` worth keeping?** A trace whose command becomes
  `<REDACTED_SECRET>` may be useless as training data while still being safe.
  But note this is already the sync path's behaviour for freshly submitted
  traces -- the backstop is the inconsistent one. Making the two paths agree is
  a defensible goal on its own, and diverging from the sync path would need its
  own justification.

The population is countable from the `residual_secret_at:` labels #505 records,
without reading contributor content. Worth measuring before shipping, to know
whether this affects one trace or many.

## What is already true

- The exposure is **bounded and visible**: these traces quarantine rather than
  release, and #505 names the field.
- The prior-risk reset design
  (`2026-09-01-quarantine-prior-risk-reset-design.md`) does not clear these:
  their fresh pass re-derives High, so they re-quarantine correctly even if
  reset.
- The fix is small, mirrors a change already reviewed and shipped, and reuses
  the tool-aware function rather than inventing a parallel policy.
