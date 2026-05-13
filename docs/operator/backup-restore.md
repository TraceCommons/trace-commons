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

## Vector index: no remote backup

This is the documented weakness in v1. The vector index files are local
disk only. The rebuild path is:

1. Stop `tracedao-ingest`.
2. Empty `TRACE_COMMONS_VECTOR_INDEX_ROOT`.
3. Restart. The index will be empty.
4. Replay inserts from `trace_gate_decisions` — every row that recorded
   `inserted_vector_entry_id` has the embedding evidence hash and the
   tenant_storage_ref needed to re-derive the embedding.

The replay procedure leverages the V24 + V25 schema columns introduced
by the A4 spec: `vector_entry_id` plus the embedding-evidence hash let
the gate worker walk `trace_gate_decisions` in chronological order and
re-call the embedder to repopulate the index.

This is **not** automated in v1. The runbook step is:

```sh
# Conceptual; actual replay tooling is a separate piece of work.
# For now, the lossy fallback is: accept an empty index and let it fill
# over time. Novelty floors will trivially pass for ~1k traces until
# the index regrows.
```

A future PR will ship a `bin/tracedao-vector-replay` for automated
reconstruction. Until then, the honest operator answer is: keep the
disk on a redundant volume (e.g. zonal SSD persistent disk with
snapshots).

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
