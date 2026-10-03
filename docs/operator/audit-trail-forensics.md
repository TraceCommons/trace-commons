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
| `file_head_not_in_db` | The file has events past the DB's latest hashed row, and no DB row after that row is unhashed -- so no rolled-back build explains it. The file is ahead, or the two forked. | Run `/v1/admin/audit-chain-drill` and the DB reconciliation drill; treat an unexplained fork as a P0 chain drift. |
| `db_row_missing_canonical_payload` | A hashed row without its payload. The mirror always writes one with the chain fields, so this is anomalous. | Escalate; do not hand-edit either log. |
| `db_row_chain_mismatch`, `db_row_hash_mismatch`, `db_row_payload_mismatch`, `db_row_payload_not_canonical`, `db_row_payload_unreadable` | The DB row does not reproduce its own chain fields. | Treat as tampering or corruption: P0, do not edit either log. |
| `db_row_already_in_file` | The row's event id is already in the file, out of order. | Escalate; do not hand-edit the file. |

Fix the cause of the failed append (free the disk, restore the file's
permissions) before running the repair, or the restored lines and the next
append will fail the same way.

## Rolling forward after a binary rollback

> **Disabled by default since 2026-10-02.** The pilot's pre-#1043 rollback
> target, `5f239be4`, was retired on 2026-10-02, so the legacy-segment resume
> below is off unless ingest was started with
> `TRACE_COMMONS_ALLOW_LEGACY_SEGMENT_RESUME=true` (read once at startup;
> `1`, `true`, `yes` or `on`). While it is off:
>
> - the dry run (step 1) works as below and still reports
>   `file_ahead_through_legacy_rows` with every segment count and time, and
>   its response carries `legacy_segment_resume_enabled: false`;
> - every non-dry run that carries `"accept_legacy_segment": true`, or that
>   meets a legacy segment without it, is refused `409`
>   `legacy_segment_resume_disabled` before anything is written. The refusal
>   is logged by that label alone.
>
> Turn it on only for an emergency roll-forward after a rollback to a
> pre-#1043 build: set the variable, restart ingest, run step 2 for each
> affected tenant, then unset it and restart again. The dry run's
> `legacy_segment_resume_enabled: true` confirms the restart took.

Builds from before #1043 (the pilot's was `5f239be4`) did not put the file
log's chain fields on DB audit rows. Under required mirror writes they wrote
the DB row before chaining the event into the file, so the row has no
`previous_event_hash` / `event_hash`. Some kinds of event they wrote to the
file only, with no DB row of the event's id:

| File event | What the old build wrote to the DB instead |
|---|---|
| `submitted`, `quarantine_remediated`, `quarantine_operator_rescrub` | the store's `submit-audit` row, under an id derived from the submission |
| `review_decision` | the store's own status row (`actor_role` `system`, reason `review_decision`) |
| `idempotent_submit` (a re-POST) | nothing |

It also wrote store rows with no file event of their own: a status row and
an artifact-invalidation row beside a `revoked` or `maintenance` event, and a
status row for each PII-backstop pass. Every other event it mirrored under the
event's own id, unhashed.

So a rollback to such a build is safe to serve, but it leaves a mark. While
it runs, the file chain moves ahead and the DB's latest hashed row does not.
After the roll-forward, the new build chains the next event from the file's
head, the DB refuses it as stale, and every audited write for each tenant
that was active during the rollback fails with a 500. The DB-ahead repair
above has nothing to restore here.

A submission attempted in that state is refused with a 500, but its
submission row and file record were written before the audit append failed.
Its `submitted` audit event was never written. When the client retries after
the repair, the retry finds the submission and is recorded as an
`idempotent_submit`. Repair each tenant before it takes traffic, or expect
such submissions.

The same route handles this shape. It resumes the DB chain across the
segment the old build wrote:

1. Roll forward, then find the affected tenants: the audit-chain drill
   reports `audit_chain_file_head_not_db_head=1` for each one, because the
   file's head is not the DB's latest hashed row. The dry run tells you
   why. A dry run for an unaffected tenant reports `clean`.

   ```bash
   curl -sS -X POST "$INGEST/v1/admin/audit-chain-repair" \
     -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
     -d '{"purpose": "INC-1234 roll forward after rollback to <build>"}'
   ```

   Expect `divergence: "file_ahead_through_legacy_rows"`, with
   `legacy_segment_file_events` (file events past the DB's latest hashed
   row), `legacy_segment_unhashed_db_rows` (the rows the old build wrote
   there) and `legacy_segment_file_only_events` (those file events with no DB
   row of their id). Check they fit the traffic the rollback served. For
   example, two submissions, one re-POST and one status read on the old
   build give 4 file events (two `submitted`, one `idempotent_submit`, one
   `read`), 3 unhashed DB rows (two `submit-audit` rows and the read's
   mirror) and 3 file-only events (the two `submitted` and the re-POST).

   `legacy_segment_earliest_at` and `legacy_segment_latest_at` bound the
   segment's file events and DB rows. Both must fall inside the rollback
   window. A segment that reaches outside it was not written by the rollback:
   stop and treat it as chain drift.

   `chain_resumed` is `false`, and nothing is written.
2. Run it with `"dry_run": false` **and** `"accept_legacy_segment": true`.
   The second flag is the explicit acceptance of rows the repair cannot
   verify by hash, the unhashed rows the rolled-back build wrote. Without it
   a non-dry run refuses `legacy_segment_not_accepted` and writes nothing.
   This step needs `TRACE_COMMONS_ALLOW_LEGACY_SEGMENT_RESUME` set (see the
   note at the top of this section); otherwise it refuses
   `legacy_segment_resume_disabled`, with or without the flag.

   ```bash
   curl -sS -X POST "$INGEST/v1/admin/audit-chain-repair" \
     -H "Authorization: Bearer $ADMIN_TOKEN" -H 'Content-Type: application/json' \
     -d '{"dry_run": false, "accept_legacy_segment": true, "purpose": "INC-1234 roll forward after rollback to <build>"}'
   ```

   It appends one `audit_chain_repair` event, chained from the file head
   like any other event. Its `decision_inputs_hash` is the DB's latest
   hashed row's `event_hash`: the row names the exact point the DB chain
   resumes from. The response has `chain_resumed: true`, and
   `repair_audit_event_id` is that event. It holds only counts and the
   purpose's hash.
3. Run it again. It reports `clean`. Then confirm one submission succeeds,
   and run the audit-chain and db-reconciliation drills: neither reports a
   chain failure, and the audit-chain drill reports the resume
   (`db_legacy_segment_resume_count`, `db_legacy_segment_file_event_count`).
   The acceptance flag is not needed here, because a clean run takes no
   legacy path. A non-dry run on a clean chain, with or without the flag,
   still records its own `audit_chain_repair` event, as every non-dry repair
   does. That event is harmless.

Nothing in-band proves that a rolled-back build wrote the segment. A DB-side
edit could strip the hashes from the chain's tail, or delete it and plant an
unhashed row. So the repair accepts only what that build could have written,
and requires every row and event to account for each other. Holding the
tenant's append lock, it verifies all of this before writing anything:
- The DB's latest hashed row is a file event, with the same chain fields.
- Every file event after it chains from it and reproduces its own hash.
- A DB row of one of those events' ids sits after the DB head. It is exactly
  the row the old build mirrored the event as: unhashed, with no request id,
  with the event's principal, and with every column the event determines
  agreeing with it. That covers action and metadata (through the kind
  projection), status, role, reason, export and decision inputs. Only
  `object_ref_id` and `occurred_at` are not compared: the file event has no
  object ref, and `occurred_at` is the database's clock.
- An event with no DB row is of a kind the old build left file-only (the
  table above). It has the store row that build wrote beside it: a
  `submit-audit` row with the event's status, or a status row by the event's
  principal.
- Every DB row after the DB head is one of those, or another store row that
  build wrote: beside a `revoked` or `maintenance` event in the segment by
  the same principal, or a PII-backstop pass's status row.
- There is at least one DB row after the latest hashed row. That is the old
  build's mark; without it the file running ahead is unexplained.

It refuses anything else with a `409` and writes nothing:

| Label | Meaning | Next step |
|---|---|---|
| `db_head_not_in_file` | The DB's latest hashed row is not a file event, or its chain fields differ. A fork or a tampered row, not a rollback. | P0 chain drift: do not edit either log. |
| `file_chain_break_after_db_head` | The file does not chain on from the DB's latest hashed row, or an event after it does not reproduce its hash. | P0: the file was edited or forked. |
| `legacy_row_mismatch` | An unhashed DB row disagrees with the file event of its id, or the event names no principal. The old build copied those fields from the event and always named one, so this is not its doing. | P0: treat as tampering. |
| `legacy_row_before_db_head` | A row of a segment event's id sits at or before the DB head. The old build wrote it after. | P0: treat as tampering. |
| `legacy_file_only_event_unexpected` | A segment event has no DB row, and the old build mirrored every event of its kind. Its hashed row was deleted, or it was never written. | P0: treat as a deletion. |
| `legacy_event_unattributed` | A file-only segment event names no principal or role. | P0: treat as tampering. |
| `legacy_submit_row_missing` | A file-only `submitted`, `quarantine_remediated` or `quarantine_operator_rescrub` has no `submit-audit` row with its status after the DB head. | P0: treat as a deletion. |
| `legacy_status_row_missing` | A file-only `review_decision` has no store status row after the DB head. | P0: treat as a deletion. |
| `legacy_row_unexplained` | A DB row after the DB head is neither a segment event's row nor a store row the old build wrote. | P0: treat as a planted row. |
| `legacy_segment_not_accepted` | The state is a legacy segment the repair can resume across, but the non-dry run did not carry `"accept_legacy_segment": true`. Nothing was written. | Review the dry run's counts and times, then rerun with the flag. |
| `legacy_segment_resume_disabled` | The legacy-segment resume is off: ingest was started without `TRACE_COMMONS_ALLOW_LEGACY_SEGMENT_RESUME`. Returned for any non-dry run that carries `"accept_legacy_segment": true`, before the repair reads either log, and for any non-dry run that meets a legacy segment. Takes precedence over `legacy_segment_not_accepted`. Nothing was written. | The dry run still diagnoses the state. Resume only in an emergency: set the variable, restart ingest, rerun, then unset it and restart. |
| `file_head_not_in_db` | The file is ahead with no DB rows after the DB head. | See the table above. |

How the drills read it afterwards. The DB chain is the hashed rows in order,
and an unhashed row neither breaks nor restarts it. A hashed row that does
not chain from the one before is a failure, except for a repair row like
this. That row passes only if all of these hold:
- it is an `audit_chain_repair` row (`Retain`, not a dry run) with its
  canonical payload;
- its `decision_inputs_hash` is exactly the hashed row before it;
- it carries the chain fields of the file event with its id;
- that hashed row is an ancestor of it in the file chain.

A hashed row lost before it therefore still breaks the chain, and so does a
resume across a fork. The rows the old build wrote remain unhashed legacy
rows (`db_legacy_event_count`), since the table is insert-only. The file
chain was never broken, and the audit-chain drill verifies it end to end.
Inside a segment such a row resumes across, the db-reconciliation and
rollback drills count the old build's file-only events and store rows apart
(`db_audit_legacy_segment_file_only_event_count`), not as missing events or
reader-parity drift.

The resume row and its file line are written together. The store takes the
tenant's advisory lock, re-checks that the DB head is still the one planned
against, and inserts the row. Only then, before it commits, is the file line
appended. So a repair whose plan went stale fails before it touches the file,
and two overlapping repairs (an old and a new process on one file root)
cannot both write a line. If the commit fails after the file append, the
line is file-only. The next dry run reports
`legacy_segment_resume_interrupted: true`, and the next repair writes that
line's DB row rather than a second resume event.

Rolling back again later is handled the same way. Each roll-forward needs one
repair per affected tenant, and each repair is its own resume point.

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
