-- The activation gate (delivery PR 5).

-- A bundle has one qualification row for each code revision (P5-D10), and
-- the gate needs the row of the deployed revision: a bundle that was
-- qualified on an earlier revision can be qualified again on a later one,
-- and its earlier rows stay.
ALTER TABLE pipeline_bundle_qualifications
    DROP CONSTRAINT pipeline_bundle_qualifications_pkey,
    ADD PRIMARY KEY (tenant_id, bundle_id, code_revision_hash);

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V112: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- V93 withheld UPDATE on the active bundle because no ingest route switched
-- a tenant's bundle. The qualified activation gate now does, in a statement
-- that runs only after a qualification for the deployed revision and four
-- runnable policies were found (activate_qualified_bundle_in). No other
-- statement of the ingest runtime updates this row.
GRANT UPDATE (bundle_id, selected_at) ON pipeline_active_bundles TO trace_ingest_runtime;
