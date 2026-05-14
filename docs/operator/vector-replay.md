# `tracedao-vector-replay`

Operator CLI that rebuilds a tenant's per-tenant vector index file
(`<TRACE_COMMONS_VECTOR_INDEX_ROOT>/<tenant_hash>.usearch`) from
PostgreSQL audit data plus the encrypted artifact store. It is the
automated replacement for the manual SSD-snapshot procedure documented
in [`backup-restore.md`](backup-restore.md).

For first-time setup, fixture shape, and rehearsal preconditions, see
[`vector-replay-setup.md`](vector-replay-setup.md). This document is
the production runbook; the setup doc is the pre-rehearsal checklist.

## When to use this

- The per-tenant `.usearch` file is corrupted (e.g. unclean shutdown
  before the next inline flush — extremely rare; usearch flushes on
  `Drop`, eviction, and every `flush_every` writes — but possible if
  the underlying disk had a hardware fault).
- The host's vector-index volume was lost and there is no SSD snapshot
  to restore from.
- The tenant's index drifted out of sync with `trace_vector_entries`
  due to an operator error (e.g. accidental file deletion).
- A fresh standby host needs to be brought up with the production
  embeddings without re-running the gate pipeline (which would mint new
  credit events).

For routine model swaps, use [`model-swap.md`](model-swap.md); for
calibration runs, use [`calibration.md`](calibration.md). This binary
is a **recovery tool only** — it does not write `trace_gate_decisions`
rows, credit events, or audit events of its own. The original audit
history is preserved as-is.

## Authentication

There is no bearer-token route. The binary runs directly on the host
with the same credentials the gate service uses:

- `DATABASE_URL` — PostgreSQL connection string.
- `TRACE_COMMONS_KEK_PROVIDER` — `local_master_key` (default) or
  `dstack`. The GCP Cloud KMS provider used by the production ingest
  binary is **not** supported by this v1 of the replay binary; if
  `gcp_cloud_kms` is configured in the gate-service environment, run
  the replay binary on a host that can use the local master key fallback
  derived from the same operator-controlled KEK material.
- `TRACE_COMMONS_ARTIFACT_KEY_HEX` — artifact-store key.
- `TRACE_COMMONS_ARTIFACT_DIR` — root path of the artifact store. v1
  supports the file-system provider only. For GCS deployments, run the
  replay on a host where the artifact bucket is mirrored to a local
  path.
- `TRACE_COMMONS_EMBEDDER_MODEL_ID`, `_CACHE_DIR`, `_MAX_TOKENS`,
  `_MATRYOSHKA_DIM` — same as the gate service.
- `TRACE_COMMONS_VECTOR_INDEX_ROOT`, `_DIM`, `_HNSW_M`,
  `_EF_CONSTRUCTION`, `_EF_SEARCH`, `_MAX_OPEN`, `_FLUSH_EVERY` — same
  as the gate service. The configured `_DIM` must match
  `Embedder::output_dim()`.

If any required env is missing, the binary exits at startup with
`VectorReplayMissingEnv: <ENV_NAME>`.

## CLI

```
Usage:
  tracedao-vector-replay --tenant-id <uuid>
                         [--fresh | --incremental]
                         [--require-embedder-match]
                         [--dry-run]
                         [--limit <N>]
                         [--page-size <N>]
```

| Flag | Default | Meaning |
| --- | --- | --- |
| `--tenant-id` | required | The tenant whose vector index to rebuild. |
| `--fresh` | default | Delete the tenant index file before scanning. |
| `--incremental` | off | Keep the existing file; skip entries already present. |
| `--require-embedder-match` | off | Refuse to replay rows whose original `gate_policy_version` does not match the configured embedder. Default is warn + skip. |
| `--dry-run` | off | List submissions that would be replayed without doing the work. |
| `--limit N` | unset | Stop after scanning N rows. Useful for partial tests. |
| `--page-size N` | 500 | Postgres page size for the audit-trail scan. |

## Mode selection

- `--fresh` is the default and the recommended mode for full recovery
  scenarios. It wipes the per-tenant index file (synchronously, with
  hash-only logging of the file name) before starting the scan. After
  `--fresh` completes, the on-disk index reflects exactly the entries
  the replay decided to insert.
- `--incremental` is the right choice when the on-disk index is mostly
  correct but you suspect a small number of entries are missing (e.g.
  the file was restored from yesterday's snapshot and today's inserts
  are absent). It skips any `vector_entry_id` already present in the
  index. The skip check goes through usearch's native `contains(key)`
  call and is cheap.

## Expected runtime

| Tenant size | `--fresh` runtime (rough) |
| --- | --- |
| ~1,000 entries | ~1 minute |
| ~10,000 entries | ~10 minutes |
| ~100,000 entries | ~1–2 hours |

The bottleneck is the embedder. On the standard production H100 host
running `BAAI/bge-large-en-v1.5` with 512 max tokens, throughput is
~100–200 traces/s. Larger inputs or higher max-tokens settings reduce
that linearly. The PostgreSQL scan and the artifact-store reads are
not the limiting factors at production tenant sizes; expect to spend
>95% of wall-clock time inside `Embedder::embed`.

## Per-row event log

Every row emits exactly one structured event under target
`tracedao_vector_replay`:

```
event="vector_replay_progress"
tenant_storage_ref="tenant_sha256:..."
submission_id=<uuid>
vector_entry_id=<uuid>
status="replayed" | "skipped_revoked" | "skipped_embedder_mismatch"
       | "skipped_already_present" | "skipped_submission_not_accepted"
       | "skipped_missing_artifact" | "error"
```

Watch for non-zero counts in any `skipped_*` bucket — small counts
(e.g. revocations propagated since the original audit row was written)
are expected. Large counts indicate something is wrong; investigate
before re-running. The `error` status surfaces a stable hash-only error
class (`VectorReplayArtifactReadFailed`, `VectorReplayKekUnwrapFailed`,
`VectorReplayEmbedFailed`, `VectorReplayIndexInsertFailed`,
`VectorReplayPgQueryFailed`); raw tenant ids, submission bodies, or
secret material never appear in the log.

## Final summary

The binary prints a JSON summary on stdout when it finishes. Sample:

```json
{
  "tenant_storage_ref": "tenant_sha256:...",
  "mode": "fresh",
  "dry_run": false,
  "rows_scanned": 1234,
  "replayed": 1100,
  "skipped_revoked": 50,
  "skipped_embedder_mismatch": 80,
  "skipped_already_present": 0,
  "skipped_submission_not_accepted": 2,
  "skipped_missing_artifact": 1,
  "errors": 1,
  "elapsed_seconds": 412.3
}
```

Exit code is 0 when `errors == 0`, 1 otherwise. The summary still
prints when partial failures occur, so the operator always has the
full picture before deciding whether to re-run.

## Safety properties

- **Read-only at the audit/credit layer.** The binary does not emit
  `trace_gate_decisions` rows, credit events, or audit events. The
  original audit history is preserved as-is.
- **Tenant-scoped throughout.** Every PostgreSQL query goes through
  the tenant-scoped facade with the same forced-RLS predicates the
  ingest binary uses. The raw pool is never touched.
- **Hash-only logging.** Error class names are stable and label-only;
  tenant ids are surfaced only in canonical `tenant_sha256:...` form.

## Operational caveats

- The configured `Embedder` MUST be the one that produced the original
  embeddings — otherwise the rebuilt index is semantically different
  from the original, which corrupts novelty scores going forward. The
  binary checks `embedder.output_dim() == TRACE_COMMONS_VECTOR_INDEX_DIM`
  at startup as the load-bearing dim guard, but identity of the model
  itself is an operator responsibility. Use `--require-embedder-match`
  to fail closed on `gate_policy_version` mismatches as an extra
  guardrail; see "Caveats on `--require-embedder-match`" below.
- Stop `tracedao-ingest` (or at minimum, ensure no writes are flowing
  for the target tenant) before running with `--fresh`. The binary
  drops its in-memory tenant handle before deleting the file, but a
  concurrent gate-service write could race with the rebuild.
- The replay flushes the usearch index to disk at the configured
  `TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY` cadence AND at completion.
  A crash mid-rebuild leaves the index in whatever the last flushed
  state was; safe to re-run with `--incremental` to fill the gap or
  with `--fresh` to start clean.

### Caveats on `--require-embedder-match`

Migration V23 placed `embedder_model_id` on `trace_vector_entries`, NOT
on `trace_gate_decisions`. As a result, this v1 of the binary compares
the configured embedder's `model_id` against the gate-decision row's
`gate_policy_version` (the policy/model bundle anchor). Operators who
need a strict embedder-identity check should cross-reference the
`trace_vector_entries.embedding_model` column for the
`vector_entry_id` of interest before running with
`--require-embedder-match`. A future iteration may add a dedicated
column on `trace_gate_decisions` so the check is exact.
