# Corpus redaction re-scan — design

## Goal

Re-run the current redactor over already-stored envelopes, so that a detector
improvement reaches the traces accepted before it existed. Report what the new
detector finds, and re-quarantine what it can no longer vouch for.

The immediate driver is #187. It fixed a tokenization defect where an unspaced
`api_key=SECRET` was published verbatim while `api_key: SECRET` was redacted.
Every trace accepted before 2026-07-31 was scanned by the detector that had
that defect. The fix is forward-only, and nothing revisits the corpus.

## The constraint that shapes this

A re-scan needs plaintext. The ingest process cannot obtain it.

Verified at `main` (`d021e9e`):

- The DEK-unwrapping `KmsKeyWrapper` is constructed inside the gate-service
  builder (`trace-commons-ingest.rs:4696`, `:4760`) and moved into
  `EnclaveGateService`. It is never stored on `AppState`.
- `AppState` holds `artifact_store`, which yields ciphertext plus a wrapped
  DEK, and no way to unwrap it.
- The `TraceGateService` trait (`trace_gate_service.rs:152-194`) exposes
  `evaluate_trace`, `evaluate_trace_perplexity_only`, `invalidate_vector_entry`
  and `safe_status`. Every one returns scores or status. **None returns
  plaintext.**
- `rescore_perplexity_one` (`trace-commons-ingest.rs:46223`) is the existing
  precedent: it loads ciphertext with `load_trace_ciphertext_and_wrapped_dek`,
  hands both to the gate service, and receives only numbers back.

That is deliberate. `docs/trace-commons.md:66-68` describes the direction of
travel as scoring inside attested hardware "that even the server's operators
cannot read". Any design that routes corpus plaintext back into the ingest
process spends that property to buy a maintenance job.

## Rejected: give ingest a decryptor

The direct approach — put a `KmsKeyWrapper` on `AppState` and decrypt in the
handler — is rejected.

It would make every stored envelope readable by the ingest process for any
future reason, not just this one, and it is the exact capability the enclave
boundary exists to withhold. It also inverts the Phase B milestone: the
codebase would move *away* from operator-unreadable storage in order to fix a
redaction bug. Cost is permanent, benefit is one-off.

Note the asymmetry that makes this tempting and still wrong: ingest already
sees plaintext at submit time, because the envelope arrives in the POST body
and `rescrub_trace_envelope` runs on it there. So "ingest can see plaintext"
is already true *for a trace in flight*. It is not true *for the corpus at
rest*, and that distinction is the whole boundary.

## Design: re-scrub inside the gate service

Add one method to `TraceGateService`, mirroring the shape of
`evaluate_trace_perplexity_only`:

```rust
fn rescan_trace_redaction(
    &self,
    tenant: &GateTenantCtx,
    ciphertext: &[u8],
    wrapped_dek: &WrappedDek,
    kind: TraceArtifactKind,
) -> anyhow::Result<RedactionRescanOutcome>;
```

```rust
pub struct RedactionRescanOutcome {
    /// Detector identity the scan ran under, so a result is attributable to
    /// a revision rather than to "the current build".
    pub pipeline_version: String,
    /// Labels and counts the re-scan produced. Same shape the submit path
    /// already persists.
    pub counts: BTreeMap<String, u32>,
    pub labels_present: Vec<String>,
    pub blocked_secret_detected: bool,
    /// True when re-running the redactor changed the envelope bytes — i.e.
    /// this detector removes material the stored envelope still contains.
    pub changed: bool,
    /// Re-encrypted corrected envelope, present only when `changed`.
    pub corrected: Option<EncryptedTraceArtifact>,
}
```

Decryption, redaction and re-encryption all happen behind the trait. The
plaintext never crosses back. What ingest receives is the same class of
information it already stores on `trace_submissions.redaction_counts`.

### Where the redactor lives, and what it costs

`trace-commons-gate-enclave` does **not** currently depend on
`trace-commons-protocol`, and never references it. Its whole dependency set is
`anyhow`, `sha2`, `tracing`, `uuid`, plus the optional inference stack behind
`local-gpu-models`. That leanness looks deliberate for something intended to
run inside a TEE.

`trace-commons-protocol` brings `chrono`, `dirs`, `hex`, `regex`,
`rust_decimal`, `serde`, `serde_json`, `thiserror`, `tokio`, `url`, `uuid`,
plus optional `reqwest` for the NEAR AI privacy filter. Taking a blanket
dependency to reach one struct would roughly triple the enclave crate's
non-inference surface, including an HTTP client it has no business holding.

There is no cycle risk — `trace-commons-protocol` does not depend on
`trace-commons-gate-enclave` — so the edge is possible. The question is
whether it is wanted.

Three options, in preference order:

1. **Extract the deterministic redactor into a leaf crate**
   (`trace-commons-redaction`) that both `trace-commons-protocol` and
   `trace-commons-gate-enclave` depend on. `DeterministicTraceRedactor`,
   `SecretLeakDetector`, the entropy pass and `RedactionReport` have no
   dependency on envelope types beyond what they redact, so the seam is real
   rather than invented. Cost is one mechanical extraction; benefit is that
   the enclave gains the detector and nothing else.
2. **Depend on `trace-commons-protocol` with `default-features = false`**, if
   the redactor path can be feature-gated away from `reqwest` and `tokio`.
   Cheaper to write, leaves the crate carrying more than it needs.
3. **Take the blanket dependency.** Simplest, and the one to justify explicitly
   rather than drift into, because it puts an HTTP client inside the enclave
   crate's dependency closure.

This is a decision to make before implementation, not during it. Option 1 is
the recommendation.

### Why `changed` rather than comparing counts

A count comparison cannot distinguish "the new detector found more" from "the
old detector recorded less". `changed` is computed by re-serializing the
envelope after re-scrubbing and comparing to what was decrypted, which answers
the operational question directly: does the stored artifact still contain
material the current detector would remove?

## Disposition

Detection first, mutation second, as separate operator actions.

**Pass 1 — audit, read-only.** Enumerate accepted submissions, re-scan each,
persist the outcome. Change nothing else. The operator gets counts by label
and a list of affected submission hashes. This is the pass that answers "is
there a live credential in the accepted corpus", and it is safe to run at any
time.

**Pass 2 — correct, explicit and separately triggered.** For submissions where
`changed`, write the corrected artifact, update `redaction_counts`, and set
status. A trace whose re-scan reports `blocked_secret_detected` goes to
`Quarantined`, matching what `status_for_risk` would have done at submit time
had the detector seen it.

Splitting the passes matters because pass 2 rewrites stored contributor
artifacts and moves accepted traces out of the corpus. That is not something
to discover as a side effect of an audit.

## Persistence

New table rather than columns on `trace_submissions`, because a re-scan is an
event that can recur per detector revision and the history is the point:

```sql
CREATE TABLE trace_redaction_rescans (
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    rescan_id UUID NOT NULL,
    pipeline_version TEXT NOT NULL,
    scanned_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    changed BOOLEAN NOT NULL,
    blocked_secret_detected BOOLEAN NOT NULL,
    counts JSONB NOT NULL DEFAULT '{}'::JSONB,
    labels_present JSONB NOT NULL DEFAULT '[]'::JSONB,
    corrected_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, rescan_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);
```

RLS forced, tenant predicate through `trace_current_tenant_id()`, matching
every other table. `run_migrations` is hand-rolled — the migration must be
wired in explicitly, not merely added to `migrations/`.

`counts` and `labels_present` are label-only and carry no trace content, which
is the same standard `trace_submissions.redaction_counts` already meets.

## Routes

Two admin routes, following `rescore-perplexity` exactly: `require_admin`,
fail-closed preconditions on `db_mirror` and `artifact_store`, spawn a
background task, return a hash-only ack, optional `?limit=N`.

- `POST /v1/admin/rescan-redaction` — pass 1, audit only.
- `POST /v1/admin/apply-redaction-rescan` — pass 2, requires a prior audit row
  and refuses submissions without one.

Pass 2 refusing to act without a stored audit row is the fail-closed property
that keeps the destructive pass from running on stale or absent evidence.

## What this does not do

- **It does not recover a leaked credential.** A key that reached the corpus
  was already in the contributor's environment and must be rotated by its
  owner. Correcting the stored artifact limits further exposure; it does not
  undo the exposure.
- **It does not re-run the NEAR AI prose filter.** That is a network call per
  trace with its own cost and failure modes. `privacy_filter:*` labels are out
  of scope; this is the deterministic detector only.
- **It does not change gate scores, novelty, dedup or credit.** Re-scrubbing
  changes envelope bytes, which would change a re-computed simhash. Deliberately
  not touched here — coupling redaction maintenance to credit is how a privacy
  fix turns into a credit incident.

## Open questions

- **Does correcting an envelope invalidate its `redaction_hash`?** It should be
  recomputed, but any downstream consumer that pinned the old value needs
  checking first.
- **Contributor notification.** A trace moving from accepted to quarantined
  after the fact is visible to its contributor and reverses a decision they
  were told about. Whether that is silent, or notified, is a policy call.
- **Ordering against #185's downgrade path.** That change lets a proven-complete
  re-scrub lower residual risk. A re-scan that both lowers and raises risk on
  the same corpus needs the interaction thought through before pass 2 runs.
