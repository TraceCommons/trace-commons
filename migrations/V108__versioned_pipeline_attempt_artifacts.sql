-- The objects each pipeline phase attempt writes (delivery PR 4). A row is
-- staged before its object is published and committed in the same
-- transaction as the phase commit, so the worker can remove the objects of
-- an attempt that crashed, lost its lease, or had its commit refused. A
-- committed row's object is an object ref of the submission (the same phase
-- commit records it), so deleting it is the withdrawal's job, not this
-- table's: the withdrawal queues its payload deletion and main's
-- revocation-propagation worker deletes it. The sweep only ever deletes the
-- object of a `staged` row, which no object ref names.
--
-- A compatibility Score stages its rows before it takes its tenant's Score
-- lock, so that it holds one pooled connection at a time while it holds the
-- lock (rebase 10, option D). The object key is fixed by the tenant, run,
-- lease token and artifact before the content exists, but the ciphertext
-- hash is not, so such a row is staged with `ciphertext_sha256` NULL and its
-- commit sets the hash. A `committed` row always has its hash, and so does
-- every `approved` row.

CREATE TABLE pipeline_attempt_artifacts (
    tenant_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    lease_token UUID NOT NULL,
    artifact TEXT NOT NULL CHECK (artifact IN ('approved', 'index-command', 'score-neighbors')),
    object_key TEXT NOT NULL CHECK (object_key <> ''),
    ciphertext_sha256 TEXT CHECK (ciphertext_sha256 ~ '^[0-9a-f]{64}$'),
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
        OR (state = 'committed' AND committed_at IS NOT NULL AND ciphertext_sha256 IS NOT NULL)
    ),
    -- Only a compatibility Score's two artifacts are staged without a hash.
    -- Review stages its `approved` row with one, so a row whose object the
    -- sweep deletes by key alone never names an approved object.
    CONSTRAINT pipeline_attempt_artifacts_approved_hash
        CHECK (ciphertext_sha256 IS NOT NULL OR artifact <> 'approved')
);

CREATE INDEX pipeline_attempt_artifacts_due
    ON pipeline_attempt_artifacts (tenant_id, state, cleanup_after);

-- A row moves once, from `staged` to `committed` with `committed_at` set,
-- and never changes after that. The commit may set a missing hash, to a
-- 64-character lowercase hex value, and never changes one already set. The
-- sweep deletes the object of a `staged` row, so a `committed` row moved
-- back to `staged` would hand the sweep an object the submission's refs
-- name; this trigger makes "the sweep never touches a committed object"
-- hold by construction, not only by what the code does today. Every other
-- UPDATE -- any change to a committed row, and any change to a staged row
-- but its commit -- is refused, whoever issues it. DELETE is not guarded:
-- the Score commit deletes the `staged` row of an artifact it did not
-- write, the sweep deletes due `staged` rows, and the run's cascade removes
-- the rest. The same shape as V105's
-- `reject_pipeline_review_assessment_mutation`: an ordinary invoker-rights
-- function with no settings of its own, so a non-superuser migration owner
-- can apply it.
CREATE FUNCTION guard_pipeline_attempt_artifact_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.state = 'staged'
        AND NEW.state = 'committed'
        AND NEW.committed_at IS NOT NULL
        AND NEW.tenant_id = OLD.tenant_id
        AND NEW.run_id = OLD.run_id
        AND NEW.lease_token = OLD.lease_token
        AND NEW.artifact = OLD.artifact
        AND NEW.object_key = OLD.object_key
        AND (
            (OLD.ciphertext_sha256 IS NOT NULL
                AND NEW.ciphertext_sha256 = OLD.ciphertext_sha256)
            OR (OLD.ciphertext_sha256 IS NULL
                AND NEW.ciphertext_sha256 ~ '^[0-9a-f]{64}$')
        )
        AND NEW.cleanup_after = OLD.cleanup_after
        AND NEW.staged_at = OLD.staged_at
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'pipeline attempt artifacts move only from staged to committed';
END;
$$;

CREATE TRIGGER pipeline_attempt_artifacts_guard_update
    BEFORE UPDATE ON pipeline_attempt_artifacts
    FOR EACH ROW EXECUTE FUNCTION guard_pipeline_attempt_artifact_update();

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
        RAISE EXCEPTION 'V108: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_attempt_artifacts: a write site stages its row before it
-- publishes the object the row names (INSERT), the commit transaction moves
-- a `staged` row to `committed` and sets a hash the row was staged without
-- (UPDATE), and both the Score commit (for an artifact it staged and did not
-- write) and the sweep (once it has handled a due row's object) delete a
-- `staged` row outright (DELETE). Nothing changes a `committed` row after
-- its commit, and nothing updates a row's tenant, run, lease token,
-- artifact kind, object key, set ciphertext hash, or cleanup_after (the
-- guard trigger above refuses both).
GRANT SELECT, INSERT, DELETE ON pipeline_attempt_artifacts TO trace_ingest_runtime;
GRANT UPDATE (state, committed_at, ciphertext_sha256) ON pipeline_attempt_artifacts
    TO trace_ingest_runtime;
