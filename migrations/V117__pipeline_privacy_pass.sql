-- V117: the Review-start privacy pass's record (spec 2026-10-09, pipeline
-- async privacy rescrub).
--
-- The receipt runs the bounded, local deterministic redactor only; the
-- prose-PII classifier runs in a server-owned pass at the start of the Review
-- dispatch. This is the pass record, not a classifier verdict (CMP-002): the
-- object it stored, the hashes of its input and output, the merged
-- residual-risk labels, and whether it held the run for a human. All six are
-- NULL until the pass commits, and a run received before V117 keeps them
-- NULL. The two approval columns link a human approval of an escalated run
-- to the pass.
ALTER TABLE pipeline_runs
    ADD COLUMN privacy_pass_object_ref_id UUID,
    ADD COLUMN privacy_pass_content_hash TEXT CHECK (
        privacy_pass_content_hash IS NULL OR privacy_pass_content_hash ~ '^sha256:[0-9a-f]{64}$'),
    ADD COLUMN privacy_pass_source_hash TEXT CHECK (
        privacy_pass_source_hash IS NULL OR privacy_pass_source_hash ~ '^sha256:[0-9a-f]{64}$'),
    ADD COLUMN privacy_pass_residual_risk_basis JSONB CHECK (
        privacy_pass_residual_risk_basis IS NULL
        OR jsonb_typeof(privacy_pass_residual_risk_basis) = 'array'),
    ADD COLUMN privacy_pass_outcome TEXT CHECK (
        privacy_pass_outcome IS NULL OR privacy_pass_outcome IN ('cleared', 'escalated')),
    ADD COLUMN privacy_pass_recorded_at TIMESTAMPTZ,
    ADD COLUMN privacy_pass_approval_assessment_hash TEXT CHECK (
        privacy_pass_approval_assessment_hash IS NULL
        OR privacy_pass_approval_assessment_hash ~ '^sha256:[0-9a-f]{64}$'),
    ADD COLUMN privacy_pass_approval_resolved_reasons JSONB CHECK (
        privacy_pass_approval_resolved_reasons IS NULL
        OR jsonb_typeof(privacy_pass_approval_resolved_reasons) = 'array'),
    ADD CONSTRAINT pipeline_runs_privacy_pass_shape CHECK (
        (privacy_pass_object_ref_id IS NULL) = (privacy_pass_content_hash IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_source_hash IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_residual_risk_basis IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_outcome IS NULL)
        AND (privacy_pass_object_ref_id IS NULL) = (privacy_pass_recorded_at IS NULL)),
    -- IS NOT DISTINCT FROM, not `=`: with no pass the outcome is NULL, and
    -- `NULL = 'escalated'` is NULL, which a CHECK lets through.
    ADD CONSTRAINT pipeline_runs_privacy_pass_approval_shape CHECK (
        (privacy_pass_approval_assessment_hash IS NULL)
            = (privacy_pass_approval_resolved_reasons IS NULL)
        AND (privacy_pass_approval_assessment_hash IS NULL
             OR privacy_pass_outcome IS NOT DISTINCT FROM 'escalated')),
    -- NO ACTION + DEFERRABLE for the reason V95 gives for
    -- pipeline_runs_approved_object_ref_fk: trace_object_refs also cascades
    -- straight from trace_submissions, a sibling of this row's own cascade,
    -- so a submission delete can reach either branch first.
    ADD CONSTRAINT pipeline_runs_privacy_pass_object_ref_fk
        FOREIGN KEY (tenant_id, submission_id, privacy_pass_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id)
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED;

-- The pass stages its object as a `privacy-pass` attempt artifact, committed
-- with the pass record. Like `approved`, it is always staged with its hash.
-- V108's inline artifact CHECK is named by PostgreSQL
-- (pipeline_attempt_artifacts_artifact_check); the name is kept.
ALTER TABLE pipeline_attempt_artifacts
    DROP CONSTRAINT pipeline_attempt_artifacts_artifact_check,
    ADD CONSTRAINT pipeline_attempt_artifacts_artifact_check CHECK (
        artifact IN ('approved', 'index-command', 'score-neighbors', 'privacy-pass')),
    DROP CONSTRAINT pipeline_attempt_artifacts_approved_hash,
    ADD CONSTRAINT pipeline_attempt_artifacts_approved_hash CHECK (
        ciphertext_sha256 IS NOT NULL OR artifact NOT IN ('approved', 'privacy-pass'));

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V117: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_runs: the privacy pass records its result; commit_review links an
-- escalated run's approval to it. No RLS change: a column of an already
-- forced table inherits its tenant policy.
GRANT UPDATE (privacy_pass_object_ref_id, privacy_pass_content_hash,
              privacy_pass_source_hash, privacy_pass_residual_risk_basis,
              privacy_pass_outcome, privacy_pass_recorded_at,
              privacy_pass_approval_assessment_hash,
              privacy_pass_approval_resolved_reasons)
    ON pipeline_runs TO trace_ingest_runtime;

-- A run received from here on needs a privacy pass before Review may
-- approve it. Existing rows are exempt (FALSE); the default then flips, so
-- a receipt written by either binary gets TRUE, and an approval by a binary
-- that has no pass fails this CHECK instead of approving unclassified bytes.
-- The runtime is granted no UPDATE on the column: only the receipt's INSERT
-- (through the default) sets it.
ALTER TABLE pipeline_runs
    ADD COLUMN privacy_pass_required BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE pipeline_runs
    ALTER COLUMN privacy_pass_required SET DEFAULT TRUE,
    ADD CONSTRAINT pipeline_runs_privacy_pass_before_approval CHECK (
        NOT privacy_pass_required
        OR approved_object_ref_id IS NULL
        OR privacy_pass_object_ref_id IS NOT NULL);
