-- Separately retained privacy rechecks. The original privacy_pass_* record is
-- never replaced. An assessment binds the exact latest corrected artifact.
CREATE TABLE pipeline_review_rechecks (
    tenant_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    recheck_id UUID NOT NULL,
    submission_id UUID NOT NULL,
    ordinal BIGINT NOT NULL CHECK (ordinal > 0),
    parent_recheck_id UUID,
    source_object_ref_id UUID NOT NULL,
    source_content_hash TEXT NOT NULL CHECK (source_content_hash ~ '^sha256:[0-9a-f]{64}$'),
    object_ref_id UUID NOT NULL,
    content_hash TEXT NOT NULL CHECK (content_hash ~ '^sha256:[0-9a-f]{64}$'),
    evidence JSONB NOT NULL CHECK (jsonb_typeof(evidence) = 'object'),
    residual_pii_risk TEXT NOT NULL CHECK (residual_pii_risk IN ('low','medium','high')),
    redaction_counts JSONB NOT NULL CHECK (jsonb_typeof(redaction_counts) = 'object'),
    redaction_pipeline_version TEXT NOT NULL,
    reviewer_principal_ref TEXT NOT NULL CHECK (reviewer_principal_ref ~ '^reviewer_sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, run_id, recheck_id),
    UNIQUE (tenant_id, run_id, ordinal),
    UNIQUE (tenant_id, run_id, recheck_id, content_hash),
    FOREIGN KEY (tenant_id, run_id) REFERENCES pipeline_runs (tenant_id, run_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, source_object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, submission_id, object_ref_id)
        REFERENCES trace_object_refs (tenant_id, submission_id, object_ref_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, run_id, parent_recheck_id)
        REFERENCES pipeline_review_rechecks (tenant_id, run_id, recheck_id) ON DELETE CASCADE,
    CHECK ((ordinal = 1) = (parent_recheck_id IS NULL))
);
CREATE TRIGGER pipeline_review_rechecks_reject_update BEFORE UPDATE ON pipeline_review_rechecks
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_review_assessment_mutation();
CREATE TRIGGER pipeline_review_rechecks_reject_delete BEFORE DELETE ON pipeline_review_rechecks
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_review_assessment_mutation();
ALTER TABLE pipeline_review_rechecks ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_review_rechecks FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_review_rechecks;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_review_rechecks
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
GRANT SELECT, INSERT ON pipeline_review_rechecks TO trace_ingest_runtime;

ALTER TABLE pipeline_review_assessments ADD COLUMN recheck_id UUID,
    ADD COLUMN recheck_content_hash TEXT,
    ADD CONSTRAINT pipeline_review_assessment_recheck_shape CHECK (
        (recheck_id IS NULL) = (recheck_content_hash IS NULL)
        AND (recheck_id IS NULL OR (phase = 'review' AND recheck_content_hash ~ '^sha256:[0-9a-f]{64}$'))),
    ADD CONSTRAINT pipeline_review_assessment_recheck_fk
        FOREIGN KEY (tenant_id, run_id, recheck_id, recheck_content_hash)
        REFERENCES pipeline_review_rechecks (tenant_id, run_id, recheck_id, content_hash);

ALTER TABLE pipeline_attempt_artifacts DROP CONSTRAINT pipeline_attempt_artifacts_artifact_check,
    ADD CONSTRAINT pipeline_attempt_artifacts_artifact_check CHECK (
        artifact IN ('approved','index-command','score-neighbors','privacy-pass','review-recheck'));

-- These guards keep older binaries from assessing/releasing the original
-- content after a corrected candidate has been recorded.
CREATE FUNCTION require_latest_pipeline_review_recheck_binding() RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE candidate RECORD;
BEGIN
    IF NEW.phase = 'review' THEN
        SELECT recheck_id, content_hash INTO candidate FROM pipeline_review_rechecks
         WHERE tenant_id = NEW.tenant_id AND run_id = NEW.run_id ORDER BY ordinal DESC LIMIT 1;
        IF FOUND AND (NEW.recheck_id IS DISTINCT FROM candidate.recheck_id
                     OR NEW.recheck_content_hash IS DISTINCT FROM candidate.content_hash) THEN
            RAISE EXCEPTION 'pipeline_review_recheck_required';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER pipeline_review_assessments_require_recheck_binding
    BEFORE INSERT ON pipeline_review_assessments
    FOR EACH ROW EXECUTE FUNCTION require_latest_pipeline_review_recheck_binding();

CREATE FUNCTION require_pipeline_review_recheck_approval_source() RETURNS TRIGGER LANGUAGE plpgsql SET search_path = pg_catalog, public, pg_temp AS $$
DECLARE candidate RECORD;
BEGIN
    IF NEW.approved_object_ref_id IS NOT NULL AND OLD.approved_object_ref_id IS NULL THEN
        SELECT recheck_id, content_hash, object_ref_id INTO candidate FROM pipeline_review_rechecks
         WHERE tenant_id = NEW.tenant_id AND run_id = NEW.run_id ORDER BY ordinal DESC LIMIT 1;
        IF FOUND AND NOT EXISTS (
            SELECT 1 FROM pipeline_review_assessments a
              JOIN trace_derived_records d ON d.tenant_id = a.tenant_id
               AND d.derived_id = NEW.approved_revision_id
             WHERE a.tenant_id = NEW.tenant_id AND a.run_id = NEW.run_id
               AND a.phase = 'review' AND a.recommendation = 'approve'
               AND a.recheck_id = candidate.recheck_id
               AND a.recheck_content_hash = candidate.content_hash
               AND d.submission_id = NEW.submission_id
               AND d.input_object_ref_id = candidate.object_ref_id
               AND d.input_hash = candidate.content_hash) THEN
            RAISE EXCEPTION 'pipeline_review_recheck_approval_source_mismatch';
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER pipeline_runs_require_recheck_approval_source BEFORE UPDATE ON pipeline_runs
    FOR EACH ROW EXECUTE FUNCTION require_pipeline_review_recheck_approval_source();
