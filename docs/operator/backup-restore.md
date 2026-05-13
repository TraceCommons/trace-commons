# Backup and Restore

What's backed up, where, and how to restore — written with honest RPO/RTO
targets rather than aspirational ones.

## What lives where

| Data | Backing store | Backup mechanism | RPO | RTO |
|---|---|---|---|---|
| Submissions, audit chain, credit ledger, gate decisions | PostgreSQL | Cloud SQL automated snapshots (hourly) + PITR (1-7 days configurable) | ~1 hour for snapshot, near-zero for PITR | 15-30 min to restore |
| Encrypted artifact bytes | GCS bucket | Object versioning + soft-delete (configurable retention) | ~0 (versions retain prior bytes) | minutes (re-point reads at prior generation) |
| Vector index files (HNSW per-tenant) | Local disk under `TRACE_COMMONS_VECTOR_INDEX_ROOT` | **None remote.** Local-disk only. | "from last manual snapshot" — could be hours/days | depends on rebuild path |
| Embedder model cache | Local disk under `TRACE_COMMONS_EMBEDDER_CACHE_DIR` | None. Re-downloaded via `stage-models.sh`. | n/a | minutes |
| Perplexity model weights | Local disk under `TRACE_COMMONS_PERPLEXITY_MODEL_PATH` | None. Re-downloaded via `stage-models.sh`. | n/a | 10-60 min (re-download) |
| KEK (Cloud KMS) | GCP-managed | GCP's responsibility | n/a | n/a |
| Local gate-service master key (`TRACE_COMMONS_GATE_SERVICE_MASTER_KEY`) | Operator-held secret | Operator's responsibility (vault, sealed env, etc.) | n/a | minutes |

## PostgreSQL: backup and restore

### Backup

Cloud SQL automated backups + PITR is the recommended posture. Enable
both. Verify monthly that a restore actually works by spinning up a
parallel instance from a recent backup.

For self-hosted Postgres, `pg_basebackup` + WAL archiving to GCS.

### Restore

```sh
gcloud sql backups restore <backup-id> --restore-instance=<new-instance>
```

After restore, validate the audit chain:

```sh
curl -s -X POST -H "Authorization: Bearer $ADMIN" \
  "$BASE/v1/admin/audit-chain-drill" | jq
```

A `AuditChainDriftRejected` after restore means the backup ended
mid-write. Use PITR to a slightly earlier point and try again.

## GCS: encrypted artifact bytes

Versioning + soft-delete is the only realistic protection. The bytes
are encrypted with DEKs that are themselves wrapped with the Cloud KMS
KEK; loss of the KEK is loss of the artifacts, full stop.

### To restore an accidentally-deleted artifact

```sh
gcloud storage objects restore gs://<bucket>/<object>#<generation>
```

The DEK wrapping that artifact lives in `trace_object_refs` /
`trace_object_versions` in PG; the GCS object generation is what gets
referenced. As long as you have both, restoration is straightforward.

## Vector index: rebuild via `tracedao-vector-replay`

The per-tenant vector index files are local disk only — there is no
remote backup. To recover from a corrupted or lost
`<root>/<tenant_hash>.usearch` file (or to bring up a freshly
provisioned host with the historical embeddings), use
`tracedao-vector-replay`. The binary walks `trace_gate_decisions`
chronologically for the requested tenant, re-fetches each accepted
submission's encrypted envelope from the artifact store, decrypts via
KMS, re-embeds with the configured embedder, and reinserts at the
canonical `vector_entry_id`. It does **not** emit gate-decision rows,
audit events, or credit events — the original audit trail is preserved
as-is.

Concrete invocation, single tenant, fresh rebuild:

```sh
export DATABASE_URL=postgres://...
export TRACE_COMMONS_KEK_PROVIDER=local_master_key          # or "dstack"
export TRACE_COMMONS_ARTIFACT_KEY_HEX=...
export TRACE_COMMONS_ARTIFACT_DIR=/var/lib/tracedao-artifacts
export TRACE_COMMONS_EMBEDDER_MODEL_ID=BAAI/bge-large-en-v1.5
export TRACE_COMMONS_EMBEDDER_CACHE_DIR=/var/cache/tracedao-embedder
export TRACE_COMMONS_VECTOR_INDEX_ROOT=/var/lib/tracedao-vector-index
export TRACE_COMMONS_VECTOR_INDEX_DIM=1024

# Stop tracedao-ingest first so the index file is not held open.
systemctl stop tracedao-ingest

tracedao-vector-replay \
  --tenant-id 550e8400-e29b-41d4-a716-446655440000 \
  --fresh

systemctl start tracedao-ingest
```

The binary prints a JSON summary on stdout when it finishes and exits
non-zero if any per-row error occurred. See
[`vector-replay.md`](vector-replay.md) for the full reference: flag
semantics, `--incremental` vs `--fresh` selection, `--dry-run`,
`--require-embedder-match`, expected runtimes, and the per-row event-log
fields the operator should watch.

Operators who want to avoid the full rebuild path should still keep
`TRACE_COMMONS_VECTOR_INDEX_ROOT` on a redundant volume (e.g. zonal SSD
persistent disk with snapshots) so the file-level restore is the
primary recovery and `tracedao-vector-replay` is the fallback when
that's lost too.

## Model weights

Re-downloadable via `stage-models.sh`. Keep
`scripts/operator/.model-checksums` in version control so a fresh
download is verified against the same SHA256.

## Disaster recovery exercise

Quarterly, run this end-to-end:

1. Snapshot PG, stash a recent GCS object listing.
2. Spin up a parallel `tracedao-ingest` pointed at a restored PG +
   restored GCS bucket clone.
3. Run the [smoke test](smoke-test.md).
4. Verify the audit chain drill passes.
5. Tear down.

If step 4 fails, treat as a P1 incident: backups are not actually
recovering correctly.
