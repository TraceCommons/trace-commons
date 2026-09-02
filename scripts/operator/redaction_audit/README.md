# Redaction audit benchmark

A detector-independent check on redaction, plus the corpus to measure it with.

## Why this exists

A redaction checker that shares a pattern source with the redactor it checks reports clean
forever: anything the redactor cannot see, the checker cannot see either. This auditor is
written as a *positive-evidence detector* — it looks for the shapes of sensitive data
(a capitalised name adjacent to a role word, a credential cue whose value is not a
placeholder, a path segment that is not a known-safe token, a clustered legal register)
rather than mirroring any redactor's rules. It scans object **keys** and file **paths**, not
only values.

## Running it

```
python3 redaction_audit.py --self-test     # unit fixtures, exits 0 on pass
python3 run.py                             # scores cases.jsonl, exits 0 only at 90/90
```

## The corpus

`cases.jsonl` holds 90 synthetic records: 60 positives that must be detected across five
classes (identity, legal_matter, third_party_pii, personal, secret) and 30 negatives — clean
agent-engineering content that must **not** be flagged: stack traces, dependency install
output, git diffs, security prose that discusses credentials without containing one,
config keys such as `passcode_required: true`, and identifiers that merely look
secret-adjacent.

Every value is invented. Each record carries `fixture_notice: "SYNTHETIC_FUZZ_FIXTURE"`.

## Current measurement

```
recall    60/60  = 1.00
precision 60/61  = 0.98
```

The negatives are the point. Without them a detector that flags everything scores a perfect
recall, and this one did: measured against 30 independent negatives it started at precision
0.81, which surfaced five defects — chief among them a missing word boundary that made
`participant_identity` match the "to" inside "tokens" and fire on any sentence containing an
infinitive. A checker's own hand-written negatives are an optimistic bound, not a measurement.

One false positive remains and is deliberately left in the corpus rather than tuned away:
`synthetic-fuzz-079` trips a health-term match on `cpap` in a sentence about an SDK test
suite, because that rule has no anchor requiring an actual person. A benchmark that is
adjusted until its own detector passes measures nothing.

## Known limitation

`_NAMED_PATH_RE` carries the same latent defect that was fixed in `name_near_role`:
`re.IGNORECASE` defeats its embedded proper-noun shape. No current case trips it.
