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
   routing change made after that time is lost with the rest of it. Step 3 says
   how to check the rows before traffic returns.
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

   Bring each tenant's audit rows level with its audit file first, before
   any other request of this step. The audit file
   (`tenants/<key>/audit/events.jsonl` under the ingest root) is not in the
   database. It still holds the tenant's events from after the backup, and the
   restored database does not: the file's chain is ahead of the database's.
   In a deployment that requires the database mirror
   (`TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES`, or account admission is on), the
   database then refuses each new audit row of that tenant, because the row
   does not chain from the database's own latest row. Until the two are level:

   - `GET /v1/admin/pipeline/routing` answers `500`, so you cannot read the
     state, the record id, or `active_bundle_qualified_on_revision`.
   - `contain`, `deactivate`, and a policy intervention commit their change
     and answer `500` `pipeline_change_committed_audit_failed`, with no
     routing row in the answer. `qualifications` does the same.
   - `POST /v1/admin/audit-chain-repair` does not repair this case. It
     restores a file that is behind the database. Here the file is ahead, and
     it answers `409` with the label `file_head_not_in_db`.

   The database mirror backfill makes the two level. Send it for each tenant,
   with that tenant's admin credential:

   ```sh
   curl -sS -X POST "$BASE/v1/admin/maintenance" \
     -H "Authorization: Bearer $ADMIN" -H 'Content-Type: application/json' \
     -d '{"backfill_db_mirror": true, "prune_export_cache": false, "purpose": "restore: audit rows from the file"}'
   ```

   It writes a database row for each event of the file that the database does
   not have, in the file's order, and then appends its own audit row. In the
   answer, `db_mirror_backfill_failed` must be 0. Then
   `POST /v1/admin/audit-chain-drill` verifies the chain, and the routes of
   this step answer. The call is `main`'s retention call, and the backfill is
   one of its effects: it also writes the database rows of the tenant's other
   file records that the database lost, and it marks expired and revoked
   records. [pipeline-activation.md](pipeline-activation.md), "Legacy drain
   report", lists the effects, the dry run, and the body that a deployment
   with `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN` needs. A tenant with
   no audit event after the backup needs no backfill.

   A pipeline receipt admitted after the backup has no run in the restored
   database, and its `submitted` event in the file has no status. The
   backfill writes that event with the status `received`, because its
   submission has no file record (a pipeline submission never has one), and
   then the tenant's later events. It does not need a pipeline runtime in
   the process that runs it. The run itself is not restored: the receipt is
   lost with the rest of the database's changes after the backup.

   If a tenant must be stopped before its backfill, send `contain`. It
   commits and stops the tenant's uploads, although it answers the `500`
   label. The record id of that containment is in the log's error line
   (`pipeline admin action committed and its audit row was not appended`, the
   field `activation_record_id`), and `GET /v1/admin/pipeline/routing` shows
   it after the backfill. A change that committed before the backfill gets no
   audit row: its record is its routing event and its two log lines.

   Check each tenant's routing before step 5 puts the processes back into client
   traffic. A containment made after the backup comes back as `pipeline`, and step
   5 would then take that tenant's uploads. Read each tenant's row and events
   (`GET /v1/admin/pipeline/routing`, with the tenant's admin credential; it needs
   only the routing store). Repeat a lost containment first
   (`POST /v1/admin/pipeline/contain`, which needs only the routing store and
   works in this configuration; it needs no expectation in its body).

   Read `active_bundle_qualified_on_revision` in the same answer, for each
   tenant, whatever its row says. A process refuses the new uploads of a tenant
   whose row says `pipeline` while the tenant's active bundle has no
   qualification on the process's revision (`503`
   `pipeline_bundle_not_qualified`), and the restored database holds only the
   qualifications from the time of the backup. Both scope lists are unset in
   this step, so no start warning and no count names these tenants: this field
   is the only place that shows them. For each tenant where it is `false`,
   record the qualification again (`POST /v1/admin/pipeline/qualifications`,
   which works in this step), or contain the tenant, before step 5. Where it
   is null, the tenant has no active bundle, or the build has no revision; a
   build with no revision refuses every `pipeline` tenant's new uploads (`503`
   `bundle_runtime_revision_unknown`).

   Check each tenant's policy suspensions too, also before step 5. Step 5
   resumes Settle, credit, and the NEAR payout dispatch for every listed
   tenant. A policy that was suspended after the backup is runnable again in
   the restored database, and step 5 then scores, settles, and pays under it.
   For each tenant, read `suspended_policy_count` (`GET
   /v1/admin/pipeline/operational-summary`) and the interventions of each of
   its bundles (`GET /v1/admin/pipeline/policy-interventions?bundle_id=...`).
   `GET /v1/admin/pipeline/routing` names the active bundle, and its events
   name the earlier ones. Repeat each lost suspension
   (`POST /v1/admin/pipeline/policy-interventions` with the action `suspend`,
   which works in this configuration). Repeat a lost suspension of a Settle
   policy first. A `resume` made after the backup is lost too: the policy is
   suspended again, and its runs wait until you resume it again.

   The restored database cannot show a change that it lost. A containment or a
   suspension made after the backup is gone from the row, from the events, and
   from the interventions, so the restored state reads like a tenant that was
   never contained or suspended. Find the lost changes in a record outside the
   database:

   - The ingest log. Each successful admin action logs one line, `pipeline
     admin action recorded`, with the tenant's storage reference
     (`tenant_sha256:...`), an action label (`qualify`, `activate`, `rollback`,
     `contain`, `deactivate`, `policy_suspend`, `policy_resume`), and an
     evidence hash. Read the lines from the time of the backup onward. A line
     shows that an action happened, for which tenant, and when. It does not
     show the bundle of an activation or a rollback, or the bundle and the
     phase of a policy action.
   - `main`'s audit file. Each successful admin action appends one event of
     the kind `pipeline_activation` to the tenant's audit file
     (`tenants/<key>/audit/events.jsonl` under the ingest root), which a
     database restore does not reach. The event holds the actor's
     `principal_ref`, the time, and an action label (`pipeline_qualify`,
     `pipeline_activate`, `pipeline_rollback`, `pipeline_contain`,
     `pipeline_deactivate`, `pipeline_policy_suspend`,
     `pipeline_policy_resume`). Like the log line, it does not show the bundle
     or the phase. An action that answered `500`
     `pipeline_change_committed_audit_failed` can have no event there: the
     log has an error line for it. The audit rows in the database are lost
     with the other rows; the file is the copy to read.
   - Your own record. The code keeps no other record of an admin action
     outside the database. Keep a record of each admin action (the route, the
     body, and the time) in a place that a database restore does not reach.

   If you cannot tell whether a tenant was contained, or one of its policies
   was suspended, after the backup, contain the tenant until you know.

   Only four changes work in this step: `contain`, `deactivate`, and the
   `suspend` and `resume` of a policy. `activate` and `rollback` do not. Both
   scope lists are unset here, and these two routes refuse a tenant that is not
   on the receipts list of the process (`409` `pipeline_tenant_not_in_scope`).

   So an activation or a rollback that was made after the backup cannot be
   repeated before step 5. The restored database selects the bundle from the
   time of the backup. After a lost rollback, that is the bundle that you
   rolled back from, and its row says `pipeline`: step 5 returns the tenant's
   uploads to it. After a lost activation of a newer bundle, it is the bundle
   that you replaced. After a lost first activation, the tenant has no row, or
   its row says `legacy`, and step 5 sends its uploads to the legacy path. For
   each such tenant, do this:

   1. In this step, contain the tenant (`POST /v1/admin/pipeline/contain`).
      Step 5 then takes none of its new uploads, on either path.
   2. After step 5, repeat the change. Each `rollback` and each `activate`
      needs `expected_record_id` in its body: the `activation_record_id` of
      the tenant's routing row, which `GET /v1/admin/pipeline/routing` shows
      and which the answer of your last change also carries (the containment
      of step 1 here). Without it the answer is `422`
      `pipeline_request_invalid`; with an id that is no longer in force it is
      `409` `pipeline_routing_state_changed`. For a lost rollback, send
      `rollback` for the earlier bundle, with the record id of the
      containment. The tenant stays contained. Then send `activate` for that
      bundle, which is now the active one, with the record id that the
      rollback answered, to open the tenant again. For a lost activation,
      send `activate` for the bundle again, with the record id of the
      containment. That activation opens the tenant.

   A qualification that was recorded after the backup is lost too, and the
   gate needs it: record it again (`POST /v1/admin/pipeline/qualifications`,
   which works in this step) before the `rollback` or the `activate`. A lost
   `deactivate` can be repeated in this step. Send it with the
   `activation_record_id` that `GET /v1/admin/pipeline/routing` shows as
   `expected_record_id`. For a tenant that you contained in this step, the
   `deactivate` is refused without that id (`409`
   `pipeline_routing_expectation_required`); `expected_state` alone is not
   enough.
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
   once the database is healthy. When the store call for a stored command
   fails for a reason other than integrity, the rebuild stops with `503`
   `index_command_unreadable`; a second run helps once the store is back. A
   command the store reports missing or corrupt stops it with `409`
   `index_command_invalid`, which a second run does not change.

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
   - A rebuild never shortens or deletes another rebuild's row. Two rebuilds of
     one tenant (a client retry that reaches another replica, for example) each
     hold their own fence.
   - A run withdrawn before its entries are written is skipped: the rebuild
     looks at the run again after it has set the fence.
   - A withdrawal of a run whose entries are being written usually waits for
     them, but not always (a lost database session, an abort past the shutdown
     grace period, a process exit). The fence then holds the removal back until
     the write's time has passed.
   - A rebuild that is lost leaves its row, and the row expires on its own. A
     rebuild is lost by a lost session, an abort, a process exit, or an index
     call still running at the deadline plus the margin.
   - The fence of a lost rebuild holds the tenant's index removals back for at
     most 90 seconds at the defaults. That is a run deadline of 30 seconds (the
     smaller of the Settle lease and the 30 second dispatch budget) plus the
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
