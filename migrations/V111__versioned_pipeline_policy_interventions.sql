-- Operator interventions on a bound policy (delivery PR 5). `runnable` (V93)
-- stays the column every reader uses; `operational_status` says why it is
-- false. V93 gave the runtime no UPDATE here and nothing wrote `runnable`,
-- so every existing row is runnable and satisfies the new check.

ALTER TABLE pipeline_bundle_policy_status
    ADD COLUMN operational_status TEXT NOT NULL DEFAULT 'runnable'
        CHECK (operational_status IN ('runnable', 'suspended', 'terminated')),
    ADD CONSTRAINT pipeline_bundle_policy_status_runnable_shape CHECK (
        runnable = (operational_status = 'runnable')
    );

CREATE TABLE pipeline_policy_interventions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    intervention_id UUID NOT NULL,
    bundle_id TEXT NOT NULL CHECK (bundle_id ~ '^sha256:[0-9a-f]{64}$'),
    phase TEXT NOT NULL CHECK (phase IN ('admission', 'review', 'score', 'settle')),
    action TEXT NOT NULL CHECK (action IN ('suspend', 'resume', 'terminate')),
    actor_principal_ref TEXT NOT NULL CHECK (
        actor_principal_ref ~ '^[A-Za-z0-9_:.-]{1,160}$'
    ),
    reason_code TEXT NOT NULL CHECK (reason_code ~ '^[a-z0-9_]{1,64}$'),
    previous_status TEXT NOT NULL CHECK (
        previous_status IN ('runnable', 'suspended', 'terminated')
    ),
    resulting_status TEXT NOT NULL CHECK (
        resulting_status IN ('runnable', 'suspended', 'terminated')
    ),
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    recorded_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, intervention_id),
    FOREIGN KEY (tenant_id, bundle_id, phase)
        REFERENCES pipeline_bundle_policy_status (tenant_id, bundle_id, phase)
        ON DELETE CASCADE
);

CREATE INDEX idx_pipeline_policy_interventions_bundle
    ON pipeline_policy_interventions (tenant_id, bundle_id, phase, recorded_at DESC);

CREATE FUNCTION reject_pipeline_policy_intervention_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'DELETE' AND pg_trigger_depth() > 1 THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'pipeline policy interventions are immutable';
END;
$$;

CREATE TRIGGER pipeline_policy_interventions_reject_update
    BEFORE UPDATE ON pipeline_policy_interventions
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_policy_intervention_mutation();
CREATE TRIGGER pipeline_policy_interventions_reject_delete
    BEFORE DELETE ON pipeline_policy_interventions
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_policy_intervention_mutation();

ALTER TABLE pipeline_policy_interventions ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_policy_interventions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_policy_interventions;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_policy_interventions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V111: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- An intervention updates the status row (and a phase commit locks it
-- FOR SHARE, which needs UPDATE on a column) and appends its record.
GRANT UPDATE (runnable, operational_status, error_label, updated_at)
    ON pipeline_bundle_policy_status TO trace_ingest_runtime;
GRANT SELECT, INSERT ON pipeline_policy_interventions TO trace_ingest_runtime;
