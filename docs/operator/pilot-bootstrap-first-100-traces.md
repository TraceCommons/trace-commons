# Pilot-Bootstrap First-100-Traces Dry Run — Operator Runbook

Phase: A.6 post-PR-#67. Predecessor:
[`./pilot-bootstrap.md`](./pilot-bootstrap.md) (the harness reference)
and [`./pilot-bootstrap-dryrun-notes.md`](./pilot-bootstrap-dryrun-notes.md)
(historical defects).

PR #67 made `tracedao-pilot-bootstrap` real-data-capable against the
HF agent-traces datasets. Before scaling to 30k+ submissions, the
operator runs a controlled first-100 batch against a staging
`tracedao-ingest` to verify the gate decision distribution is sane
and that the audit chain row count matches the submission count.

## Goal

Submit 100 real swival traces through a staging `tracedao-ingest`
deployment. Verify:

- Gate-pass rate is within the A2.5 pre-pilot estimate.
- Audit chain row count is consistent with the submission count.
- No raw URLs, raw envelope bodies, or operator-secret material
  appear in logs or audit rows.

The 100-trace run is the gate condition for scaling to 1000 and
then to the full ~30k corpus.

---

## Pre-flight

1. **Staging PostgreSQL** with the current migrations applied.
   The bootstrap-tenant must already be provisioned (UUID recorded
   for the verification step below).
2. **Staging `tracedao-ingest`** running with:

   ```
   TRACE_COMMONS_TENANT_TOKENS=bootstrap-tenant:contributor:<token>
   TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=0
   ```

   The zero-credit calibration mode is required — this run is for
   shape validation, not for credit emission.
3. **Local `tracedao-pilot-bootstrap` binary**:

   ```bash
   cargo build --release --bin tracedao-pilot-bootstrap
   ```

4. **HF datasets cache reachable.** The swival dataset is ~1.5 GB
   (33,667 JSONL files). For cache layout and hygiene see
   [`./hf-dataset-cache-hygiene.md`](./hf-dataset-cache-hygiene.md).

---

## Run

```bash
./target/release/tracedao-pilot-bootstrap \
  --source jedisct1/agent-traces-swival \
  --target https://staging.example/v1/submissions \
  --tenant-token <token> \
  --limit 100 \
  --rate 1 \
  --sidecar /tmp/pilot-bootstrap-first-100.jsonl \
  --seed 1
```

Expected wall-clock: ~120 seconds (1 req/sec at `--limit 100`).
Logs are hash-only label-only; no raw envelope content should
appear.

---

## Verification checklist

Run each check after the harness exits. Each bullet is one
independent check; the run is acceptable only if all pass.

- **Sidecar line count is 100:**

  ```bash
  wc -l /tmp/pilot-bootstrap-first-100.jsonl
  ```

- **All 100 submission_ids distinct:**

  ```bash
  jq -r .submission_id /tmp/pilot-bootstrap-first-100.jsonl | sort -u | wc -l
  ```

- **Rerun is idempotent.** Run the harness a second time with the
  same `--seed 1`; the sidecar grows to 200 lines but the distinct
  `submission_id` count stays at 100:

  ```bash
  jq -r .submission_id /tmp/pilot-bootstrap-first-100.jsonl | sort -u | wc -l
  # → 100
  wc -l /tmp/pilot-bootstrap-first-100.jsonl
  # → 200
  ```

- **Per-tenant gate decision count is 100:**

  ```sql
  SELECT count(*) FROM trace_gate_decisions
   WHERE tenant_id = '<bootstrap-tenant-uuid>';
  ```

- **Per-domain gate-pass rate is sane.** Join the sidecar against
  `trace_gate_decisions` by `submission_id` and group by
  `source_domain_tag`:

  ```sql
  SELECT source_domain_tag, accepted, count(*)
    FROM trace_gate_decisions gd
    JOIN <sidecar-staging-table> s USING (submission_id)
   WHERE gd.tenant_id = '<bootstrap-tenant-uuid>'
   GROUP BY source_domain_tag, accepted
   ORDER BY source_domain_tag, accepted;
  ```

  Compare the per-domain accepted ratio against A2.5's pre-pilot
  estimate (recorded in
  `docs/superpowers/reports/2026-05-14-gate-floor-recalibration-findings.md`).

- **Audit chain row count is consistent with submission count:**

  ```sql
  SELECT count(*) FROM trace_audit_log
   WHERE tenant_id = '<bootstrap-tenant-uuid>';
  ```

  Expected > 100 (one row per submission plus ancillary rows for
  gate decision, credit-event no-op, etc.). The exact multiplier
  depends on the ingest path configuration; > 100 is the floor.

- **No raw envelope content in harness logs.** Grep the harness
  stdout/stderr for any string that resembles a URL or a raw
  message body; result must be empty.

- **No raw URLs in audit rows.** Spot-check 5 random audit rows:

  ```sql
  SELECT * FROM trace_audit_log
   WHERE tenant_id = '<bootstrap-tenant-uuid>'
   LIMIT 5;
  ```

  All payload columns should be hash digests or label-only fields.

---

## Decision after the run

- **All checks pass and gate-pass rate matches A2.5 estimate:**
  scale to `--limit 1000`, same command shape, same sidecar path
  (it appends). Repeat the verification checklist. If the 1000-run
  also passes, the harness is cleared for the full ~30k corpus.
- **Gate-pass rate is anomalous** (outside the A2.5 estimate's
  band, or zero, or 100%): STOP. File findings in
  `docs/operator/pilot-bootstrap-anomaly-<date>.md` and hand off
  to analyst review before any further scale-up.
- **Audit chain row count is wrong** (zero, or != 100 submissions
  represented, or any audit-chain integrity check fails): STOP.
  Treat as a critical incident; check the worker-route auth gates
  (see [`./hash-only-logging.md`](./hash-only-logging.md) for the
  relevant error classes) and the migration state.

---

## Teardown

- If the staging deployment is temporary, drop it after the
  1000-run also passes. If it remains in use for the full ~30k
  corpus, leave it running.
- Archive the sidecar JSONL to
  `s3://<bucket>/pilot-bootstrap/runs/<date>/` if persistence
  matters for later analysis; otherwise delete from
  `/tmp/pilot-bootstrap-first-100.jsonl`.
- Cache hygiene on the harness host:
  [`./hf-dataset-cache-hygiene.md`](./hf-dataset-cache-hygiene.md).

---

## Hash-only / no-secrets reminder

Sidecar, audit rows, and harness logs are hash-only by construction.
Verification SQL outputs can be quoted in any committed anomaly
report; raw envelope bodies, raw URLs, tenant tokens, and HF tokens
cannot.
