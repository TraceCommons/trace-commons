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

## Vector index: rebuild via `trace-commons-vector-replay`

The per-tenant vector index files are local disk only — there is no
remote backup. To recover from a corrupted or lost
`<root>/<tenant_hash>.usearch` file (or to bring up a freshly
provisioned host with the historical embeddings), use
`trace-commons-vector-replay`. The binary walks `trace_gate_decisions`
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
export TRACE_COMMONS_ARTIFACT_DIR=/var/lib/trace-commons-artifacts
export TRACE_COMMONS_EMBEDDER_MODEL_ID=BAAI/bge-large-en-v1.5
export TRACE_COMMONS_EMBEDDER_CACHE_DIR=/var/cache/trace-commons-embedder
export TRACE_COMMONS_VECTOR_INDEX_ROOT=/var/lib/trace-commons-vector-index
export TRACE_COMMONS_VECTOR_INDEX_DIM=1024

# Stop trace-commons-ingest first so the index file is not held open.
systemctl stop trace-commons-ingest

trace-commons-vector-replay \
  --tenant-id 550e8400-e29b-41d4-a716-446655440000 \
  --fresh

systemctl start trace-commons-ingest
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
primary recovery and `trace-commons-vector-replay` is the fallback when
that's lost too.

## Versioned pipeline

`python3 scripts/operator/pipeline.py restore-drill` (also run as the last
step of `pipeline.py qualify`) is the versioned pipeline's own restore
check. See
[pipeline-qualification.md](pipeline-qualification.md#pipelinepy-restore-drill---postgres-admin-url-url)
for what it runs, step by step.

**What it proves.** A `pg_dump` / `pg_restore` round trip of the pipeline's
PostgreSQL rows, plus a byte-for-byte copy of the encrypted artifact
directory, resumes a pending run to the same settlement legs and Trace
Credit ledger event the original run reached, with the index's entry set
and the pending-run set unchanged and no duplicate effect. Before the
resume, the restored database keeps every trace table's RLS (enabled,
forced, and the tenant policy's predicate), every RLS policy the dumped
database had and no other, the runtime login's full privilege set, every
tenant's rows, and every tenant's audit chain. The policy comparison is
against the dumped database: a policy that was already too wide before the
dump is not detected here.

**What it does not prove.** The artifact "restore" is a local filesystem
copy (`shutil.copytree`), never a restore from a remote object store; the
drill's report always carries the `filesystem_restore_local_only` blocker.
A remote-provider restore drill (GCS or another configured object store) is
promotion work, not part of this release. Host client-tool versions also
matter here: with `--postgres-admin-url`, the drill's dump and restore run
the *host's* installed `pg_dump` / `pg_restore` against that server, not a
version pinned in a container. A host `pg_restore` 17 emits `SET
transaction_timeout`, which a PostgreSQL 16 server refuses -- keep the
host's client major version equal to the target server's. The default
container mode is unaffected: it starts and restores against its own
digest-pinned `postgres:16` image regardless of the host's installed
client tools.

**After a real restore,** rebuild the pipeline's index before the pipeline
worker processes any tenant. A worker that runs first scores pending runs
against an empty or partial index, so their novelty, and their credit,
comes out too high. The rebuild route is served only by an ingest build
that injects a pipeline runtime; the repository binary injects none and
answers `404` there. Do these steps in this order:

1. Restore PostgreSQL (the pipeline's rows restore with everything else --
   there is no separate pipeline backup or restore path for the database). The
   routing rows, the activation events, the receipt ownership rows, the policy
   interventions, the qualifications, and the rebuild fences are pipeline rows:
   they come back with the database, as they were at the time of the backup. A
   routing change made after that time is lost with the rest of it. Read each
   tenant's row and events (`GET /v1/admin/pipeline/routing`) after step 5, and
   repeat a change that the restore undid.
2. Restore the encrypted object store.
3. Start every `trace-commons-ingest` process with
   `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` and
   `TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` both unset. The pipeline
   runtime still starts, and the rebuild route serves every tenant (it
   refuses a tenant on either list, `409`
   `pipeline_index_rebuild_tenant_active`).
   The worker reads both lists once, at start, so it drains no tenant: it
   scores no pending run, and it pays and invalidates nothing. Keep these
   processes out of client traffic until step 5. While a tenant is on neither
   list, a new upload of a tenant whose routing row says `pipeline` is refused
   with `503` `pipeline_tenant_not_served`, and does not take the legacy path.
   An upload of a tenant whose row says `contained` is refused with `503`
   `pipeline_receipt_intake_contained`. A tenant with no row, or whose row says
   `legacy`, takes the legacy path.
4. For each tenant, call `POST /v1/workers/pipeline/index-rebuild` with that
   tenant's vector worker bearer token or an admin token -- the same gate
   as `main`'s vector index worker route (see
   [`operator-binaries.md`](operator-binaries.md) for the credential). It
   replays every complete, included run's sealed index command through the
   service's own index writer, returns a hash-only report
   (`command_count`, `entry_count`, `unchanged_entry_count`,
   `skipped_run_count`, `command_set_hash`), and appends one `vector_index`
   audit row. It creates no outcomes and no credit. It is safe to run again
   if it is interrupted: a repeat reports the entries it already wrote as
   unchanged. Each process runs at most one rebuild per tenant at a time: a
   second request for a tenant whose rebuild is running is refused (`409`
   `pipeline_index_rebuild_in_progress`). A run's writes hold its rows for
   at most the smaller of the Settle lease and 30 seconds; past that, the
   rebuild stops with `503` `index_unavailable`. A run's deadline starts before
   its fence write (below), so it includes the wait for the run's row lock: a
   slow wait, for a withdrawal that holds the row for example, fails the run
   with `503` `index_unavailable` and writes nothing. A rerun starts again from
   the first run and reports the entries already written as unchanged; a
   run whose writes take longer than that deadline fails on every rerun. A
   fence that cannot be written (a database fault) stops the rebuild before
   that run's first write with `503` `index_rebuild_fence_unavailable`. Rerun
   once the database is healthy.

   A withdrawal during the rebuild is safe because of the rebuild's fence, not
   because no withdrawal happens: withdrawals come from clients, from `main`'s
   retention maintenance, and from the revocation-propagation reconciler, and
   keeping client traffic away stops only the first. A withdrawal of a complete
   run queues the run's index removal at once. While a tenant has an unexpired
   row in `pipeline_index_rebuild_fences` (V113), no worker in any process
   claims that tenant's index invalidations, so a removal cannot run before the
   rebuild's last write:

   - A rebuild keeps one row for itself. Before each run's writes it sets the
     row to the run's deadline plus the fence margin (60 seconds,
     `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS`), and it never shortens it.
     When the rebuild ends it deletes its own row and the tenant's expired rows.
     It never shortens or deletes another rebuild's row, so two rebuilds of one
     tenant (a client retry that reaches another replica, for example) each hold
     their own fence.
   - A run withdrawn before its entries are written is skipped: the rebuild
     looks at the run again after it has set the fence.
   - A withdrawal of a run whose entries are being written usually waits for
     them, but not always (a lost database session, an abort past the shutdown
     grace period, a process exit). The fence then holds the removal back until
     the write's time has passed.
   - A rebuild that is lost (a lost session, an abort, a process exit, or an
     index call still running at the deadline plus the margin) leaves its row,
     and the row expires on its own. Its fence holds the tenant's index removals
     back for at most 90 seconds at the defaults: a run deadline of 30 seconds
     (the smaller of the Settle lease and the 30 second dispatch budget) plus the
     60 second margin, counted from the lost rebuild's last fence write. The
     worker then claims the removals on its next invalidation pass. No operator
     action is needed. The tenant's next rebuild deletes the expired row when it
     ends.
   - The margin is a contract with the index writer. The fence does not enforce
     it. The guarantee holds as long as each index call returns within the 60
     second margin; an index call that takes longer is outside what the fence
     covers.

   Step 3 stays. The route still refuses a tenant that its own process routes or
   drains (`409` `pipeline_index_rebuild_tenant_active`), because a Score of that
   tenant must not read a partly rebuilt index, and that refusal reaches only its
   own process. Run the rebuild only in step 3's configuration, on every process.
5. Restart every `trace-commons-ingest` process with both lists set back to
   their values before the restore. The worker then resumes the pending
   runs, and processes the queued invalidations and payouts, against the
   rebuilt index.

`pipeline.py restore-drill` does not exercise this route: it rebuilds the
index in process, before its app starts.

## Model weights

Re-downloadable via `stage-models.sh`. Keep
`scripts/operator/.model-checksums` in version control so a fresh
download is verified against the same SHA256.

## Disaster recovery exercise

Quarterly, run this end-to-end:

1. Snapshot PG, stash a recent GCS object listing.
2. Spin up a parallel `trace-commons-ingest` pointed at a restored PG +
   restored GCS bucket clone.
3. Run the [smoke test](smoke-test.md).
4. Verify the audit chain drill passes.
5. Tear down.

If step 4 fails, treat as a P1 incident: backups are not actually
recovering correctly.
