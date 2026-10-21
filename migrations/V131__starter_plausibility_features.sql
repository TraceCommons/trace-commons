-- V131: per-submission features for the NEAR AI starter advance's
-- plausibility check (`starter-plausibility-v1`). See
-- docs/superpowers/specs/2026-10-21-nearai-starter-advance-design.md,
-- section 3. V129 is taken by an open PR.
--
-- One row per accepted submission, written by ingest after acceptance while
-- `TRACE_COMMONS_NEAR_AI_STARTER_MODE` is not `disabled`. `features` is the
-- hash-only `StarterEnvelopeFeatures` record: counts, hour-truncated times,
-- bucket masks and `sha256:` hashes. It holds no trace text, tool argument,
-- model name or session id. Nothing here pays, gates or scores; the
-- dry-run report reads it.
--
-- Rows go with their submission (ON DELETE CASCADE). Withdrawn, revoked and
-- quarantined submissions stay in the table and are excluded by the read,
-- which joins `trace_submissions` for the current status.

CREATE TABLE trace_starter_plausibility_features (
    tenant_id TEXT NOT NULL,
    submission_id UUID NOT NULL,
    features_version TEXT NOT NULL
        CHECK (features_version ~ '^[a-z0-9-]{1,64}$'),
    features JSONB NOT NULL CHECK (jsonb_typeof(features) = 'object'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, submission_id),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions (tenant_id, submission_id)
        ON DELETE CASCADE
);

ALTER TABLE trace_starter_plausibility_features ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_starter_plausibility_features FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_starter_plausibility_features;
CREATE POLICY trace_corpus_tenant_isolation ON trace_starter_plausibility_features
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The ingest runtime records and reads features. No UPDATE: a record is
-- written once per submission (`ON CONFLICT DO NOTHING`).
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V131: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;
GRANT SELECT, INSERT ON trace_starter_plausibility_features TO trace_ingest_runtime;
