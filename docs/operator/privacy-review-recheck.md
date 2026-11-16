# Recheck a held trace

## Save and review a corrected version

`pipeline-recheck` sends covered trace content to the configured classifier
backend. It is not a host-only scan. Use it only for traces authorized for that
processing. Credentials stay on the server; no caller-supplied replacement
content is accepted. Saving a version, reviewing it, indexing it and recording
credit are separate operations.

The pipeline review API saves corrected content separately from the original
privacy pass. A successful recheck does not approve a trace or grant credit.
Apply migration V159 before running the new server binary.
The candidate table and review-assessment bindings are tenant-scoped and
append-only; do not clear `privacy_pass_*` columns to retry a check.

1. Run `trace-commons-review pipeline-quarantine --json`. A Review hold includes
   `review_source_hash` and, after a recheck, `review_candidate`. Score holds
   use their existing assessment flow and do not accept privacy candidates.
2. Claim the run with `pipeline-claim --run-id <RUN_UUID>`. The lease is 30 minutes.
3. Run `pipeline-recheck --run-id <RUN_UUID> --lease-token <LEASE_UUID>
   --recheck-id <REQUEST_UUID> --expected-source-hash <SOURCE_SHA256> --json`.
   Reuse the request UUID and original expected hash when retrying that request.
   A committed retry returns the same candidate. A new request starts from the
   latest candidate, preserving previous redactions. In-flight conflicts,
   changed sources and expired claims refuse; a busy classifier slot returns 503.
4. Inspect the candidate's hash and evidence. `coverage_complete`,
   `residual_scan_complete` and an `all-events` policy are required for approval;
   unresolved corrections, detected sensitive keys, survivors and coverage gaps
   cannot be overridden by listing a resolved hold reason. A prior High rating
   is not reset; complete safe evidence allows an explicit review of that
   candidate. Human correction text remains exactly as submitted under S5.
5. To inspect exact content, use
   `GET /v1/review/pipeline/runs/{run_id}/rechecks/{candidate_id}/content` with
   the review bearer and `X-Review-Lease-Token` header. This requires a live own
   claim, current consent and a successful content-read audit. Store returned
   plaintext only in a private file; metadata responses never include it.
   The response uses `Cache-Control: no-store`. Candidate history metadata is
   available at `GET /v1/review/pipeline/runs/{run_id}/rechecks?limit=25`.
6. Record `pipeline-assessment --run-id <RUN_UUID> --lease-token <LEASE_UUID>
   --decision approve --reason privacy_rechecked
   --resolved-quarantine-reason <HOLD_REASON> --candidate-id <CANDIDATE_UUID>
   --candidate-content-hash <CONTENT_SHA256> --json`. Repeat the resolved-reason
   flag for every applicable hold. Approval and rejection both require the latest
   exact candidate once any exists. Stale or unbound decisions return 409.

The worker reads only the candidate bound to the assessment, creates its normal
approved revision, then runs Score/index/Settle with the original run and
submission IDs. Verify the indexed approved content and the credit ledger
separately. Existing consent, scoring, account holds and settlement rules still
determine whether and how much credit is recorded; this path does not edit those
rules or append a manual award. Retries reuse the existing credit source key.

Rechecks run through the configured real classifier with AllEvents coverage,
plus the previously unchecked side prose. A local deterministic pass runs first.
The manual-review profile has these fixed limits, shared across covered event
text, structured fields and keys, side prose, and correction inspection:

| Limit | Value |
|---|---|
| Aggregate classifier-input text | 32 MiB |
| One field | 32 MiB |
| Nodes | 4,194,304 |
| Depth | 24 |

Budgets do not reset per event. Ordinary upload budgets remain unchanged.
Adapter transport/window limits still apply, so fitting this profile does not
guarantee a classifier can process the trace. One classifier recheck per process may run;
its deadline is bounded by the configured classifier deadline and remaining
review claim, with time reserved for the response. Incomplete checks stay held.

Credentials, reviewer role and tenant access grants are checked again after
external I/O, along with current consent/ABAC, before candidate commit or content
readback. Locally detected credentials in keys or immutable corrections stop
those values from reaching the classifier. Other correction text may be
classified on a copy; the stored correction is never overwritten.

Withdrawal invalidates candidate object references through the existing deletion
path. DB guards prevent older binaries from approving the old pass after a
candidate exists.

## Field-name findings

Manual rechecks recognize exact public property names from the compiled
`trace_commons.review_key_catalog.v1` catalog. It covers reviewed parameter
paths and types for spawn_agent, update_plan, exec_command and write_stdin.
This recognizes public syntax; it does not authenticate a tool or grant
permission. Only the property name is recognized. Its value and every nested
field still receive the normal scans. Trace-supplied schemas, manifest hashes,
lookalike tool names and tool results cannot extend this catalog.

Other keys are classified as synthetic JSON `{key:null}`. The JSON context
avoids treating isolated programming names as prose and includes no associated
value. A key finding remains held; names are never rewritten. Missing context
inspection, missing span provenance, invalid offsets, findings outside the key,
credentials and budget failures cannot produce complete coverage. Native
span-backed classifiers provide the required provenance; text-only or declining
backends leave contextual key coverage incomplete.

Fresh `evidence.key_diagnostics` records catalog version, recognized/contextual
key counts and at most 128 findings. Each finding carries a domain-separated
hashed location, fixed category/reason, optional confidence in millionths and
an applicable compiled schema version. Raw keys, values, dynamic paths and
private classifier spans are not included. `truncated` means more findings
exist; it never makes a held candidate eligible. Missing confidence stays
unavailable. Older candidates may have no diagnostics; do not invent locations
for their aggregate key-finding flag.

A newly deployed policy does not change stored candidates or assessments.
Create a fresh recheck candidate and inspect its exact ID/hash before approval.
Complete AllEvents coverage, immutable identity/consent/correction rules,
residual scanning and existing approval eligibility requirements still apply.

## Standalone diagnostic

`privacy-recheck` is an operator diagnostic. It does not claim a review lease,
change a stored artifact, update a pipeline run, or record an assessment.
Every result has `approval_eligible: false`.

Build the current protocol implementation without adding dependencies:

```sh
RUSTFLAGS='-D warnings' CARGO_INCREMENTAL=0 \
  cargo build -p trace-commons-protocol --example privacy-recheck \
  --features near-ai-privacy-filter
```

Supply one JSON object per line on standard input:

```json
{"run_ref":"a private operator reference","expected_sha256":"sha256:<hash of exact envelope bytes>","envelope_json":"<the exact UTF-8 JSON envelope>"}
```

Inputs must be trusted server-generated pass JSON with unique object keys. The
raw JSON-value parser does not detect duplicate keys in arbitrary hand-written
JSON.

Use a trusted on-host reader to retrieve an operable run's privacy-pass artifact,
verify its tenant, object receipt and ciphertext hash, decrypt it, and verify the
recorded plaintext hash. Read and decrypt on the host; do not copy credentials
into logs or diagnostic output. Classifier modes send covered text to the
configured backend. The diagnostic also checks the exact input hash and the protocol envelope size
limit. Classifier mode refuses fields the typed protocol model would discard.
Input is processed one line at a time; no plaintext artifact is written.

Start with the detection-only check:

```sh
./privacy-recheck --residual-only
```

This scans the exact original JSON, including unknown or legacy fields, without
modifying it or calling a classifier. It uses the same bounded per-leaf residual
scanner as the typed protocol API. It does not deserialize into a typed envelope
first, so serialization omissions cannot hide content from this check.
A successful empty scan means no *patterned secret* survived this detector; it
is not a complete privacy assessment or permission to release the trace.

The default mode performs the current deterministic scrub and configured real
classifier on a clone. Its default event policy is `prose-only`. Supply the same classifier environment as ingest without displaying or
copying credentials. Missing, invalid or no-op backends fail closed. The
classifier feature is required for this mode; residual-only also builds without
it. `--all-events` explicitly asks the classifier to examine tool events too.
That policy can take much longer and should be tested on a small trace first.
One adapter is reused for the input stream so duplicate windows share its cache.

These classifier modes use ordinary scrub coverage and limits. Even
`--all-events` does not create the backend's complete manual-review evidence or
save a corrected version. Use the pipeline API above for candidate preparation.

Standard output contains JSON results only. References are returned as
`run_ref_hash` (SHA-256 of the supplied reference), so arbitrary reference text
is never echoed. Progress on standard error contains only record numbers.
Errors use fixed labels; parse errors and upstream response bodies are omitted.
Residual location labels are returned only as `location_sha256:<hash>` with
counts: object keys can contain contributor text even when they look like schema
fields. New redaction-count labels are likewise returned as
`label_sha256:<hash>`. Neither raw paths, object keys nor matches are printed.
The fixed risk/basis labels remain readable. Unsupported-field refusals include
`coverage.mismatch_count` and up to eight mismatch samples. Samples contain only
fixed mismatch/type labels and SHA-256 hashes of field names and RFC 6901 JSON
Pointers; contributor names, values and paths are never printed. A coverage
mismatch still refuses the full classifier mode. Residual-only scans the original
JSON directly and does not omit those fields.

Full-mode results distinguish the original residual scan from the scrubbed
output scan, record both byte hashes, and report new redaction counts and the
current risk. `content_changed` compares every envelope field except privacy
annotations and `trace_card.redaction_pipeline_version`. It therefore includes
side channels outside the event list. Input bytes and original artifacts remain
unchanged.

Do not reset a prior High risk. An empty classifier result preserves High under
current policy. A scan failure, incomplete coverage or surviving secret remains
unresolved. Newly redacted output does not make the *original* stored pass safe
to approve. The pipeline API above saves corrected content and binds a separate
review assessment to the exact candidate.

Validate the diagnostic with:

```sh
RUSTFLAGS='-D warnings' CARGO_INCREMENTAL=0 \
  cargo test -p trace-commons-protocol --example privacy-recheck \
  --features near-ai-privacy-filter
```
