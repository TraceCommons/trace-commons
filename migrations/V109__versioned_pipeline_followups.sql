-- Versioned pipeline follow-ups after #1143 (V105 and V106 are frozen at
-- that merge; V107 and V108 belong to #1166).
--
-- Zaki review 3, Z3-M3: a leg the V94-era code seeded with payout `pending`
-- has no batch line under the account's settlement key and hold
-- (`payout_eligible`, V105, stays false for it), so the payout never pays it.
-- Its payout is marked `disabled` -- a payout nothing will make (Ruling
-- T10-2) -- instead of reading `pending` for good. That code seeded `pending`
-- for each instrument on a payout rail, so the statement names no
-- instrument. `payout_eligible` is written once, together with `pending`,
-- when Score inserts the leg, so a `pending` leg that is not payout-eligible
-- is always such a leg. Payout-eligible legs keep their state.
--
-- Forced row security would hide every row from the migrator here (no tenant
-- is set), so it is lifted for this one statement, inside the migration's
-- transaction, and forced again at once. No grant changes. Applied by hand,
-- this file needs `psql --single-transaction -v ON_ERROR_STOP=1`: without one
-- transaction, a failure after the first statement leaves the table without
-- forced row security. Every statement of this file locks its table until
-- the commit; that costs nothing while the pipeline tables are empty, which
-- they are until a tenant is routed.
ALTER TABLE pipeline_run_settlements NO FORCE ROW LEVEL SECURITY;
UPDATE pipeline_run_settlements
   SET payout_state = 'disabled', updated_at = NOW()
 WHERE payout_state = 'pending'
   AND NOT payout_eligible;
ALTER TABLE pipeline_run_settlements FORCE ROW LEVEL SECURITY;

-- poldsam P-8: schema checks V105 and V106 left out. Each matches what the
-- routes write: a requester is `principal_sha256:` or `exporter_sha256:` and
-- 64 lowercase hex digits (the V106 check accepted `principal_sha256:a`); the
-- resolved quarantine reasons are a JSON array; and the two schema ids are
-- labels, as `selection_policy_id` is. Each check is validated against the
-- rows that exist. A database that kept the fixtures of the #1143 runtime
-- suite (a requester such as `exporter_sha256:exporttest`) fails the
-- requester check, and the migration rolls back: drop that database and
-- create it again.
ALTER TABLE pipeline_export_snapshots
    DROP CONSTRAINT pipeline_export_snapshots_requester_principal_ref_check,
    ADD CONSTRAINT pipeline_export_snapshots_requester_principal_ref_check CHECK (
        requester_principal_ref ~ '^(principal|exporter)_sha256:[0-9a-f]{64}$'
    );
ALTER TABLE pipeline_review_assessments
    ADD CONSTRAINT pipeline_review_assessments_resolved_reasons_array CHECK (
        jsonb_typeof(resolved_quarantine_reasons) = 'array'
    );
ALTER TABLE pipeline_export_snapshot_items
    ADD CONSTRAINT pipeline_export_snapshot_items_outcome_schema_id_shape CHECK (
        outcome_schema_id ~ '^[a-z0-9_.-]{1,128}$'
    ),
    ADD CONSTRAINT pipeline_export_snapshot_items_view_schema_id_shape CHECK (
        authorized_view_schema_id ~ '^[a-z0-9_.-]{1,128}$'
    );

-- poldsam P-9: two foreign keys had no index on the referencing side, so a
-- submission delete (the invalidation table's key cascades from
-- `trace_submissions`) and a run delete (the export item's deferred key to
-- `pipeline_runs`) each found their referencing rows by a scan. Plain
-- `CREATE INDEX`: a migration runs in one transaction, and both tables are
-- empty until a tenant is routed, as the tables of the data fix and the
-- checks above are.
CREATE INDEX idx_pipeline_index_invalidations_submission
    ON pipeline_index_invalidations (tenant_id, submission_id);
CREATE INDEX idx_pipeline_export_snapshot_items_run
    ON pipeline_export_snapshot_items (tenant_id, run_id);
