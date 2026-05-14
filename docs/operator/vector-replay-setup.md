# `tracedao-vector-replay` — Setup Requirements & Verification

This doc captures the prerequisites and fixture shape an operator needs in
order to drive `tracedao-vector-replay` end-to-end against a controlled
scenario (recovery rehearsal, smoke gate, or DR drill). It complements
[`vector-replay.md`](vector-replay.md), which is the production runbook.

Use this doc **before** the first real recovery run, so that the rehearsal
environment matches the production preconditions the binary expects.

> Scope note: this is a "what-it-takes" document, not a reproducible
> dry-run runbook. A full end-to-end rehearsal currently requires the
> production gate-service environment (BGE-large embedder cached, KEK
> material, an artifact root populated by the gate pipeline). On a
> Mac developer machine the binary is exercised by:
>
> - unit tests on argument parsing and helpers — fully hermetic.
> - PG-backed integration tests on the two new `TraceCorpusStore`
>   methods (`stream_trace_gate_decisions_for_replay`,
>   `is_vector_entry_revoked`) — covered by
>   `crates/tracedao-server/tests/vector_replay_store_methods.rs`,
>   driven by `TRACEDAO_PG_TEST_DATABASE_URL`.
>
> The end-to-end CLI run — embedder init, KEK unwrap, artifact decrypt,
> usearch insert — has no hermetic integration harness; it must be
> rehearsed on a host that already has the gate service's runtime
> dependencies. See "Rehearsal recipe" below.

## What the binary already verifies on its own

Before any row is processed, the binary fails closed on:

- Missing `DATABASE_URL`, `TRACE_COMMONS_ARTIFACT_DIR`,
  `TRACE_COMMONS_ARTIFACT_KEY_HEX`.
- Unsupported `TRACE_COMMONS_KEK_PROVIDER`. Only `local_master_key`
  (default) and `dstack` are supported in v1. `gcp_cloud_kms` is rejected
  at startup with `KekProviderUnsupported`.
- Embedder init failure (`VectorReplayInit: FastEmbedTextEmbedderInitFailed`)
  — typically a missing or unreadable cache dir, or model id the
  fastembed runtime does not recognise.
- Embedder/vector-index dim mismatch. The binary refuses to start when
  `embedder.output_dim() != TRACE_COMMONS_VECTOR_INDEX_DIM`.

This means an operator does not have to write any extra preflight; if
the binary starts the replay loop, the wiring is correct.

## Host prerequisites

| Resource | Notes |
| --- | --- |
| PostgreSQL reachable via `DATABASE_URL` | Same instance the gate service writes to (or a clone with the schema + audit history). The binary runs `run_migrations()` on startup; on a clone, this is a no-op if migrations are already at head. |
| Embedder model on disk under `TRACE_COMMONS_EMBEDDER_CACHE_DIR` | Default model is `BAAI/bge-large-en-v1.5` (~1.3GB on disk). Fastembed will download on first use if the cache is empty and the host has internet; in air-gapped recoveries, pre-stage the cache. |
| Artifact store mounted at `TRACE_COMMONS_ARTIFACT_DIR` | v1 of the binary supports the file-system provider only. For GCS deployments, see [`vector-replay.md`](vector-replay.md#authentication) for the GCS-mirror workaround. |
| Vector-index root writable at `TRACE_COMMONS_VECTOR_INDEX_ROOT` | The binary writes `<root>/<tenant_sha256_first16hex>.usearch` (and a sidecar). Must be a real filesystem; tmpfs is fine for rehearsal. |
| KEK material matching the original artifacts | For `local_master_key` provider: `TRACE_COMMONS_ARTIFACT_KEY_HEX` must be the same key the gate service used to wrap DEKs for the target tenant. Replay cannot recover from KEK rotation alone — that is by design. |
| `local-gpu-models` feature build | `cargo build -p tracedao-server --bin tracedao-vector-replay --features local-gpu-models`. The default-feature build does not include the binary at all. |

## Fixture shape (what the binary scans)

A row qualifies for replay iff **all** of the following hold:

1. `trace_gate_decisions.vector_entry_id IS NOT NULL` — filtered at the
   SQL layer by `stream_trace_gate_decisions_for_replay`.
2. The owning `trace_submissions.status = 'Accepted'`. Anything else
   (tombstoned, rejected, pending) is skipped with
   `skipped_submission_not_accepted`.
3. No `trace_revocation_propagation_items` row with
   `target = VectorEntry{vector_entry_id}`,
   `action = InvalidateVector`, `status = Done`. A pending row does
   **not** suppress replay — the worker has not actually invalidated
   the entry yet. Verified by
   `is_vector_entry_revoked` (see
   `crates/tracedao-server/tests/vector_replay_store_methods.rs`).
4. An active envelope artifact exists for the submission, returned by
   `get_latest_active_trace_object_ref(SubmittedEnvelope)`. If the
   envelope was rotated out (e.g. retention purge) the row is skipped
   with `skipped_missing_artifact`.
5. The original `gate_policy_version` matches the currently configured
   embedder model id. Under default behaviour the binary skips with a
   warning; with `--require-embedder-match` it counts as a hard skip.
   See "Caveats on `--require-embedder-match`" in
   [`vector-replay.md`](vector-replay.md) for the policy-version vs
   embedder-id distinction.

## Rehearsal recipe (production-equivalent host)

Use this on a non-production host that has the gate service's runtime
already provisioned (same KEK material, same embedder cache, same
artifact root). The goal is a low-stakes pass that exercises every
code path before the real recovery.

1. Pick a low-volume tenant with at least ~50 replay-eligible rows.
   Confirm row count:

   ```sql
   SELECT count(*)
     FROM trace_gate_decisions d
     JOIN trace_submissions s ON s.submission_id = d.submission_id
                              AND s.tenant_id = d.tenant_id
    WHERE d.tenant_id = '<tenant_uuid>'
      AND d.vector_entry_id IS NOT NULL
      AND s.status = 'Accepted';
   ```

2. Snapshot the current per-tenant usearch file out of band so you can
   diff against the rebuilt file.

3. Run with `--dry-run --limit 50` first. The binary still initialises
   the embedder, KEK wrapper, and artifact store, so this exercises
   every dependency except the embed + index-write steps. Expected
   summary: `replayed == 50`, `errors == 0`, all `skipped_*` counters
   are zero unless you specifically expect revocations/embedder
   mismatches for the chosen tenant.

4. Run with `--fresh --limit 50` against a scratch
   `TRACE_COMMONS_VECTOR_INDEX_ROOT` to keep production untouched. Diff
   the resulting `.usearch` file size and entry count against the
   production snapshot for the same tenant slice. The entry count
   (`usearch --inspect` or the gate service's own metrics endpoint
   after pointing it at the scratch root) should match `replayed`.

5. Re-run with `--incremental` against the same scratch root. Expected
   summary: `replayed == 0`, `skipped_already_present == 50`. This
   confirms the contains-check short-circuit.

6. Run with `--require-embedder-match` against a tenant whose
   historical rows were written under a different
   `gate_policy_version`. Expected:
   `skipped_embedder_mismatch == <count_of_old_rows>`, `errors == 0`.

If all four runs exit 0 with the expected summary counters, the
binary's surface is operator-ready against the rehearsed dataset.

## Expected output shape

`stdout` is a single pretty-printed JSON object:

```json
{
  "tenant_storage_ref": "tenant_sha256:<32 hex>",
  "mode": "fresh" | "incremental",
  "dry_run": true | false,
  "rows_scanned": 0,
  "replayed": 0,
  "skipped_revoked": 0,
  "skipped_embedder_mismatch": 0,
  "skipped_already_present": 0,
  "skipped_submission_not_accepted": 0,
  "skipped_missing_artifact": 0,
  "errors": 0,
  "elapsed_seconds": 0.0
}
```

`stderr` carries one `vector_replay_progress` tracing event per row,
plus init-time errors. The progress event is hash-only — see
[`vector-replay.md`](vector-replay.md#per-row-event-log).

Exit code: `0` iff `errors == 0`.

## Known limitations (v1)

- **GCP Cloud KMS not wired.** The async unwrap path used by
  `tracedao-ingest` is not mirrored here. Operators on GCS-only
  deployments must rehearse on a host that can use the
  `local_master_key` fallback derived from the same operator-controlled
  KEK material. Tracked in the operational caveats section of
  [`vector-replay.md`](vector-replay.md).
- **No hermetic CLI integration harness.** Unit tests cover argument
  parsing and helpers; PG-backed tests cover the two new store methods.
  The full CLI pipeline (embedder + KEK + artifact + usearch) has no
  end-to-end test target — rehearsal on a real host is the qualifying
  signal.
- **`--require-embedder-match` compares against
  `gate_policy_version`, not the V23 `trace_vector_entries.embedding_model`
  column.** Operators who need a strict embedder-identity check should
  cross-reference `trace_vector_entries.embedding_model` directly before
  enabling the flag.

## Pre-rehearsal checklist

Before scheduling a rehearsal slot on a real host:

- [ ] Target tenant uuid is known and has > 0 replay-eligible rows.
- [ ] `DATABASE_URL` points at the right Postgres (read-only at the
      audit/credit layer; the binary will not write
      `trace_gate_decisions`, but it does write usearch files).
- [ ] `TRACE_COMMONS_ARTIFACT_KEY_HEX` matches the historical wrapping
      key for the tenant's artifacts.
- [ ] `TRACE_COMMONS_EMBEDDER_MODEL_ID` and `_CACHE_DIR` match the
      gate-service configuration that produced the historical entries.
- [ ] `TRACE_COMMONS_VECTOR_INDEX_DIM` equals `embedder.output_dim()`
      for the configured model. Mismatch fails closed at startup.
- [ ] Scratch `TRACE_COMMONS_VECTOR_INDEX_ROOT` is configured for the
      rehearsal so production indices are not touched.
- [ ] `tracedao-ingest` is stopped (or the target tenant is quiesced)
      for the duration of the `--fresh` run.

After every box is ticked, follow the "Rehearsal recipe" above. The
production runbook is [`vector-replay.md`](vector-replay.md).
