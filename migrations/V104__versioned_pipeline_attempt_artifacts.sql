-- The objects each pipeline phase attempt writes (delivery PR 4). A row is
-- staged before its object is published and committed in the same
-- transaction as the phase commit, so the worker can remove the objects of
-- an attempt that crashed, lost its lease, or had its commit refused. A
-- committed row's object is an object ref of the submission (the same phase
-- commit records it), so deleting it is the withdrawal's job, not this
-- table's: the withdrawal queues its payload deletion and main's
-- revocation-propagation worker deletes it. The sweep only ever deletes the
-- object of a `staged` row, which no object ref names.

CREATE TABLE pipeline_attempt_artifacts (
    tenant_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    lease_token UUID NOT NULL,
    artifact TEXT NOT NULL CHECK (artifact IN ('approved', 'index-command', 'score-neighbors')),
    object_key TEXT NOT NULL CHECK (object_key <> ''),
    ciphertext_sha256 TEXT NOT NULL CHECK (ciphertext_sha256 ~ '^[0-9a-f]{64}$'),
    state TEXT NOT NULL DEFAULT 'staged' CHECK (state IN ('staged', 'committed')),
    cleanup_after TIMESTAMPTZ NOT NULL,
    staged_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    committed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, run_id, lease_token, artifact),
    UNIQUE (tenant_id, object_key),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE,
    CHECK (
        (state = 'staged' AND committed_at IS NULL)
        OR (state = 'committed' AND committed_at IS NOT NULL)
    )
);

CREATE INDEX pipeline_attempt_artifacts_due
    ON pipeline_attempt_artifacts (tenant_id, state, cleanup_after);

ALTER TABLE pipeline_attempt_artifacts ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_attempt_artifacts FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_attempt_artifacts;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_attempt_artifacts
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The ingest runtime's grants on what this migration adds, on V92's terms:
-- what the pipeline code reads and writes, and nothing broader. A table or
-- column the code does not use gets no grant.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V104: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_attempt_artifacts: a write site stages its row before it
-- publishes the object the row names (INSERT), the commit transaction moves
-- a `staged` row to `committed` (UPDATE), and the sweep deletes a due
-- `staged` row outright once it has handled that row's object (DELETE).
-- Nothing changes a `committed` row after its commit, and nothing updates a
-- row's tenant, run, lease token, artifact kind, object key, ciphertext
-- hash, or cleanup_after.
GRANT SELECT, INSERT, DELETE ON pipeline_attempt_artifacts TO trace_ingest_runtime;
GRANT UPDATE (state, committed_at) ON pipeline_attempt_artifacts TO trace_ingest_runtime;
