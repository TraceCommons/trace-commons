# Audit Trail Forensics

How to read the audit chain when something went wrong. The audit chain
in `trace_audit_events` is the authoritative record; the credit ledger
and gate decisions reference it.

## Tables and what they're for

| Table | What it holds |
|---|---|
| `trace_audit_events` | Hash-chained event log. Every state transition writes one row. `prev_audit_event_hash` chains to the previous row. |
| `trace_submissions` | One row per submission. State machine drives the audit rows. |
| `trace_gate_decisions` | One row per gate evaluation. References `submission_id` and stamps `gate_version_hash`. |
| `trace_credit_ledger` | Per-pass credit emission. Stamps `gate_version_hash`, references the triggering submission. |
| `trace_object_refs` / `trace_object_versions` | Active and historical pointers into the artifact store. |
| `trace_vector_entries` (V24+) | Per-entry vector index metadata; `vector_entry_id` lets you correlate `OrchestrationDecision.inserted_entry_id` back to its source submission. |
| `trace_revocation_events` | Queue of pending revocations + their propagation status. |
| `trace_near_credit_outbox` | Outbox rows for NEAR credit submission/confirmation. |

## Common forensic queries

### "Why was credit minted for submission X?"

```sql
-- 1. Find the gate decision that triggered the credit row.
SELECT cl.*, gd.gate_policy_version, gd.gate_version_hash,
       gd.perplexity_micros, gd.novelty_score_micros
  FROM trace_credit_ledger cl
  JOIN trace_gate_decisions gd
    ON gd.submission_id = cl.submission_id
   AND gd.gate_version_hash = cl.gate_version_hash
 WHERE cl.submission_id = '<submission_id>';

-- 2. Pull the audit-chain rows for that submission in order.
SELECT occurred_at, action, audit_event_hash, prev_audit_event_hash
  FROM trace_audit_events
 WHERE submission_id = '<submission_id>'
 ORDER BY occurred_at ASC;
```

### "Did the audit chain stay intact across this window?"

```sql
WITH ordered AS (
  SELECT occurred_at, audit_event_hash, prev_audit_event_hash,
         LAG(audit_event_hash) OVER (ORDER BY occurred_at) AS prior
    FROM trace_audit_events
   WHERE occurred_at BETWEEN '<start>' AND '<end>'
)
SELECT count(*) AS broken
  FROM ordered
 WHERE prior IS NOT NULL
   AND prev_audit_event_hash IS DISTINCT FROM prior;
```

A `broken > 0` is a chain drift. Equivalent to running
`POST /v1/admin/audit-chain-drill` over that window — the drill is
authoritative because it reproduces the chain hash computation.

### "Which gate version evaluated this submission?"

```sql
SELECT gate_policy_version, gate_version_hash, perplexity_micros,
       tail_fraction_micros, novelty_score_micros,
       perplexity_passed, novelty_passed,
       inserted_vector_entry_id
  FROM trace_gate_decisions
 WHERE submission_id = '<submission_id>';
```

### "What did the operator do at <time>?"

```sql
-- Filter audit events to operator/admin actions in a window.
SELECT occurred_at, action, actor_principal_ref, action_ref_hash
  FROM trace_audit_events
 WHERE actor_role IN ('admin', 'operator')
   AND occurred_at BETWEEN '<start>' AND '<end>'
 ORDER BY occurred_at ASC;
```

### "Which submissions are stuck in propagation-failed revocation?"

```sql
SELECT submission_id, vector_entry_id, attempt_count, last_error_class
  FROM trace_revocation_events
 WHERE status = 'terminal_failed'
 ORDER BY occurred_at DESC;
```

Counter: `revocation_propagation_terminal_failed_vector_entries` in
operational summary.

### "Rebuild a vector index from audit"

The vector index is the only piece of state without a remote backup.
The V24 + V25 schema makes audit-trail rebuild possible:

```sql
-- For each gate decision that inserted an entry, in order:
SELECT gd.submission_id,
       gd.tenant_storage_ref,
       gd.inserted_vector_entry_id,
       gd.embedding_evidence_hash,
       gd.gate_version_hash,
       s.canonical_summary_hash
  FROM trace_gate_decisions gd
  JOIN trace_submissions s ON s.submission_id = gd.submission_id
 WHERE gd.inserted_vector_entry_id IS NOT NULL
   AND gd.gate_version_hash = '<current hash>'
 ORDER BY gd.occurred_at ASC;
```

For each row, fetch the contribution envelope (via `trace_object_refs`),
decrypt, feed plaintext to the embedder, re-insert into the vector
index under the original `vector_entry_id`. Replay must use the same
gate_version_hash because the embedder model id is encoded in it.

A future PR (`bin/trace-commons-vector-replay`) will automate this. For now,
the procedure is manual — see [`backup-restore.md`](backup-restore.md).

## Repairing a required-mirror lockout

With `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES` on, every audit append writes
the DB row first and the file line (`tenants/<key>/audit/events.jsonl`)
second, with the same precomputed `previous_event_hash` / `event_hash`. If
the DB commit succeeds and the file append then fails (disk full, a
permissions change, a crash between the two), the DB is one event ahead of
the file. Every later append for that tenant chains from the file's head,
which the DB refuses as stale, so the tenant's audited writes fail with a
500 until it is repaired. This fails closed; nothing forks.

Symptoms: one tenant's submissions, reviews or maintenance start failing
while other tenants are healthy, with `Trace Commons DB dual-write audit
mirror failed` warnings (hash-only) in the log.

The repair re-appends the missing file lines from the DB rows. Each such row
holds its file event verbatim as `canonical_event_json`, so the line is
restored byte-for-byte, not reconstructed. It runs under the admin token for
the affected tenant:

```bash
# 1. Dry run (the default): reports the gap, writes nothing.
curl -sS -X POST "$INGEST/v1/admin/audit-chain-repair" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d '{"purpose": "INC-1234 file append failed after DB commit"}'

# 2. Repair.
curl -sS -X POST "$INGEST/v1/admin/audit-chain-repair" \
  -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
  -d '{"dry_run": false, "purpose": "INC-1234 file append failed after DB commit"}'
```

The response is hash-only: `divergence` (`clean` or `db_ahead_of_file`),
`file_events_restorable`, `file_events_restored`, the restored audit event
ids, `purpose_hash`, and `repair_audit_event_id`. A non-dry run records its
own `audit_chain_repair` event (counts and the purpose's hash only) after
the chain is whole again. Running it twice is safe: the second run reports
`clean` and restores nothing.

It only restores a DB-ahead tail it can verify. It holds the tenant's append
lock, finds the file's head among the DB's hashed rows, and requires every
row after it to carry its payload, chain from the row before, and reproduce
its own hash. Anything else returns `409` with a label and writes nothing:

| Label | Meaning | Next step |
|---|---|---|
| `file_head_not_in_db` | The file has an event the DB lacks: the file is ahead, or the two forked. Not the lockout this repairs. | Run `/v1/admin/audit-chain-drill` and the DB reconciliation drill; treat an unexplained fork as a P0 chain drift. |
| `db_row_missing_canonical_payload` | A hashed row without its payload. The mirror always writes one with the chain fields, so this is anomalous. | Escalate; do not hand-edit either log. |
| `db_row_chain_mismatch`, `db_row_hash_mismatch`, `db_row_payload_mismatch`, `db_row_payload_not_canonical`, `db_row_payload_unreadable` | The DB row does not reproduce its own chain fields. | Treat as tampering or corruption: P0, do not edit either log. |
| `db_row_already_in_file` | The row's event id is already in the file, out of order. | Escalate; do not hand-edit the file. |

Fix the cause of the failed append (free the disk, restore the file's
permissions) before running the repair, or the restored lines and the next
append will fail the same way.

### One writer per tenant

The append lock that orders a tenant's audit chain is in-process. Run one
`trace-commons-ingest` process per file root. A second process on the same
root (an overlapping restart, or a second replica on shared storage) does not
fork the chain in required-mirror mode -- the DB refuses the loser's stale
`previous_event_hash` before its file line is written -- but its append
fails rather than waiting. Without required-mirror mode the file is written
first, and two processes can both pass the file's check-then-append, so a
second writer there can fork the file chain. Stop the old process before the
new one serves traffic. Replicas with separate file roots each hold a
different file chain against one DB chain, which the mirror cannot
reconcile; horizontal scaling is out of scope for the pilot
([`architecture.md`](architecture.md)).

## Reading hash-only fields

Every "ref hash" or "action ref hash" in audit rows is sha256-prefixed.
You can't reverse it to the original value. You can:
- Compute `sha256(known_value)` and compare to find a known principal.
- Group by hash to count distinct values.
- Verify a hypothesis ("I think this is principal X") by hashing X's
  ref.

This is intentional — the audit table can be exported to operators for
debugging without leaking principal identity.

## When to call this an incident

- **Any** chain drift not explained by a known restore. P0.
- Any `revocation_propagation_terminal_failed_*` counter `> 0` not
  cleared within an hour (covers `vector_entries`, `object_refs`,
  `export_manifests`, `export_manifest_items`, `derived_records`,
  `benchmark_artifacts`, `ranker_artifacts`, `credit_settlements`,
  `worker_queues`, `physical_delete_receipts`). P1.
- A credit row stamped with a `gate_version_hash` not in
  `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS`. P1.
- A submission with an `accepted` state but no corresponding
  gate-decision row. P2 — investigate why the worker did not run.
