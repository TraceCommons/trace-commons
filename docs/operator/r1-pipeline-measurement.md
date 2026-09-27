# Measuring R1: certified full redaction pipelines

How many submissions on a deployment would pass **R1** of the consent spec
([`../superpowers/specs/2026-09-23-connect-and-forget-consent-design.md`](../superpowers/specs/2026-09-23-connect-and-forget-consent-design.md),
"R1. A complete redaction pipeline actually ran, per session"), measured
read-only from the database. Script:
[`../../scripts/operator/r1-pipeline-measurement.sql`](../../scripts/operator/r1-pipeline-measurement.sql).

## The definition

- **Passes R1:** the submission has a stored, verified witness certificate
  (`trace_witness_certificate_evidence`, V76) whose `redaction_policy_version`
  is **exactly** one of `FULL_REDACTION_PIPELINE_VERSIONS` in
  `crates/trace-commons-protocol/src/trace_contribution.rs`. At the time of
  writing:
  - `ironclaw-deterministic-secret-path-v3+privacy-filter-near-ai-v1`
  - `ironclaw-deterministic-secret-path-v3+privacy-filter-self-hosted-v1`
  - `ironclaw-deterministic-secret-path-v3+privacy-filter-sidecar-v2`
- **Does not pass:** no certificate; the bare deterministic identifier
  (a `deterministic-only` witness); `+privacy-filter-sidecar-v1` (could fall
  back silently); `full-pipeline` (the witness's startup mode name, never a
  certified value); anything matched only by prefix.
- **Numerator:** submissions that pass. **Denominator:** every row in
  `trace_submissions` with `received_at` in `[window_start, window_end)`,
  whatever its status (accepted, quarantined, revoked, purged all count).

The script's copy of the allowlist is checked against the protocol crate by
`crates/trace-commons-server/tests/r1_pipeline_measurement.rs`; the build
fails if they diverge.

## Where the pipeline version is recorded

| Source | Schema | What it is | Used for |
|---|---|---|---|
| `trace_witness_certificate_evidence.certificate_json` -> `redaction_policy_version` | V76+ | The certificate the server verified (signature, pinned measurement, body digest) and stored byte for byte | **The only R1 evidence** |
| `trace_submissions.redaction_pipeline_version` | V1+ | The envelope's self-reported version, with the server's own `+server-rescrub-v*` / `+near-ai-pii-backstop-v1` appended | `claimed_version_not_evidence`, suffixes stripped. Not evidence: a client can run a classifier locally or type anything |
| `trace_submissions.last_status_reason = 'witness_admitted'` | V49+ | A verified certificate kept the row out of the PII-backstop hold | Lower bound on witnessed rows only: written only when the certificate changed the outcome, overwritten by later transitions |
| `trace_submission_sessions` | V78+ | Submission -> source session | Per-session figures |
| Audit events | -- | Carry no pipeline version | Not used |
| Stored envelopes (object store) | -- | Same value as the column, after rescrub | Not used: not reachable from SQL, and self-reported |

The script detects which of these exist and says which it used in its first
result set.

**A deployment older than V76 has no certified version anywhere.** On one,
every R1 figure prints empty (NULL), deliberately not `0`. The self-reported
section still prints, and it is an answer to a different question -- "how
many clients *said* a full pipeline ran" -- not to R1.

## Running it

You need `psql` (10+), a PostgreSQL 11+ server, and a login that bypasses
row-level security: the superuser, or the migration owner if it has
`BYPASSRLS`. Every Trace Commons table is `FORCE ROW LEVEL SECURITY`, so any
other role sees zero rows with no tenant context. The script checks this first
and exits non-zero with `R1MeasurementRoleSubjectToRls` rather than report
zero.

```bash
PGOPTIONS='-c default_transaction_read_only=on' \
psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 \
  -v window_start=2026-09-01T00:00:00Z \
  -v window_end=2026-10-01T00:00:00Z \
  -f scripts/operator/r1-pipeline-measurement.sql
```

Omit both `-v window_*` flags for all time. The script needs nothing from the
build on the host: copy the one `.sql` file to wherever `psql` can reach the
database.

It is read-only three times over: `PGOPTIONS` makes the session default
read-only, the script runs inside `BEGIN TRANSACTION ISOLATION LEVEL
REPEATABLE READ READ ONLY` (so every figure comes from one snapshot), and it
ends in `ROLLBACK`. It creates nothing, not even a temporary table.

## Reading the result

Two result sets. The first names the sources:

```
 section |         bucket         |             value
---------+------------------------+-------------------------------
 source  | witness_evidence_table | present
 source  | session_mapping_table  | present
 source  | status_reason_column   | present
 source  | v76_applied_at         | 2026-09-27 00:29:05.115162-04
 source  | window_start           | -infinity
 source  | window_end             | infinity
```

The second holds the counts (`section | bucket | value`):

| section | bucket | meaning |
|---|---|---|
| `totals` | `submissions` | The denominator |
| `totals` | `distinct_trace_ids` | Same rows, one per `trace_id` |
| `totals` | `r1_pass` / `r1_fail` | The R1 split. Empty = not measurable (no V76 table) |
| `totals` | `claimed_allowlisted_not_evidence` | Rows whose self-reported version is on the allowlist, certificate or not |
| `totals` | `received_before_v76_applied` | Rows received before V76 was applied; their certificates were never stored, so they fail R1 whether or not a witness ran. Start the window after `v76_applied_at` to exclude them |
| `witness` | `evidence_present` / `evidence_present_r1_pass` / `evidence_absent` | With vs without a stored certificate |
| `witness` | `status_reason_witness_admitted` | Lower bound on witnessed rows, available before V76 |
| `certified_version` | `allowlisted:<v>` / `not_allowlisted:<v>` / `unrecognised` | Certified version of every row with a certificate |
| `namespace` | `<class>:submissions` / `:evidence_present` / `:r1_pass` | By tenant namespace: `near-`, `nearai-` (the `ANCHOR_NAMESPACES`), `other` (invite / `tenant-` / legacy). By prefix only; `is_anchored_tenant` additionally requires a hash after the prefix |
| `claimed_version_not_evidence` | `allowlisted:<v>` / `not_allowlisted:<v>` / `unrecognised` | Self-reported version, server suffixes stripped |
| `sessions` | `sessions` / `every_submission_r1_pass` / `some_submission_r1_pass` / `submissions_without_session` | V78+ only; see below |

The R1 pass rate is `r1_pass / submissions`. For a rate over only the rows
that could have carried a stored certificate, subtract
`received_before_v76_applied` from the denominator, or rerun with
`window_start` at `v76_applied_at`.

Version strings are printed only when they are on the allowlist, are
`full-pipeline`, or have the exact shape of a known pipeline identifier
(`ironclaw-deterministic-secret-path-vN[+privacy-filter-{sidecar,near-ai,self-hosted}-vN]`).
Everything else is `unrecognised`, because the self-reported column holds
whatever a contributor's envelope said. The output has no ids, tenants, paths
or content, and is safe to paste into an issue.

## Caveats

- **Submissions, not sessions.** R1 is stated per session; the primary figures
  are per submission. Where `trace_submission_sessions` exists, a session
  counts in `every_submission_r1_pass` only if every one of its submissions in
  the window passes, and in `some_submission_r1_pass` if at least one does.
  Submissions with no session mapping (anything received before V78, and any
  submission the server did not map to a source session) are counted in
  `submissions_without_session` and appear in no session figure.
- **No deduplication.** A contributor who resubmits the same session under a
  new `submission_id` is counted twice. `distinct_trace_ids` is the only
  collapse offered; the gate's content dedup clusters
  (`trace_gate_decisions.dedup_cluster_id`) are not applied. Quarantine
  remediation reuses the `submission_id`, so it is one row, and its evidence is
  whatever the last body carried.
- **Versions predating the allowlist.** A certificate can only pass if it was
  issued by a witness build that stamped an allowlisted string. Everything
  certified before `ironclaw-deterministic-secret-path-v3` or before
  `privacy-filter-sidecar-v2` existed is `not_allowlisted:<v>` by
  construction, and so is anything certified before V76 was applied (not
  stored at all). A low pass rate over a long window may be history, not the
  current fleet: look at `certified_version` and narrow the window.
- **Membership is not the whole of R1's evidence.** The protocol crate's note
  on the allowlist applies here too: the value counts only on a verified
  certificate from an approved witness signer and measurement. The evidence
  table only ever holds certificates the server verified against its pin at
  the time of receipt; the script does not re-verify them.
