-- Perplexity scoring driver: attempt bookkeeping table + cross-tenant reader role.

CREATE TABLE IF NOT EXISTS trace_gate_evaluation_attempts (
    tenant_id        TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id    UUID NOT NULL,
    attempts         INT NOT NULL DEFAULT 0,
    last_attempt_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error_label TEXT,
    PRIMARY KEY (tenant_id, submission_id)
);

ALTER TABLE trace_gate_evaluation_attempts ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_gate_evaluation_attempts FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_gate_evaluation_attempts;
CREATE POLICY trace_corpus_tenant_isolation ON trace_gate_evaluation_attempts
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Cross-tenant reader role for the perplexity scoring driver's enumeration query.
-- NOBYPASSRLS: the permissive USING(true) SELECT policies below are what authorize
-- reads, so the runtime/PUBLIC role stays fully tenant-isolated.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_gate_driver') THEN
        CREATE ROLE trace_gate_driver NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_gate_driver SET statement_timeout = '5s';

GRANT SELECT ON trace_submissions TO trace_gate_driver;
GRANT SELECT ON trace_gate_decisions TO trace_gate_driver;
GRANT SELECT ON trace_object_refs TO trace_gate_driver;
GRANT SELECT ON trace_gate_evaluation_attempts TO trace_gate_driver;

DROP POLICY IF EXISTS trace_gate_driver_cross_tenant_read ON trace_submissions;
CREATE POLICY trace_gate_driver_cross_tenant_read ON trace_submissions
    FOR SELECT TO trace_gate_driver USING (true);
DROP POLICY IF EXISTS trace_gate_driver_cross_tenant_read ON trace_gate_decisions;
CREATE POLICY trace_gate_driver_cross_tenant_read ON trace_gate_decisions
    FOR SELECT TO trace_gate_driver USING (true);
DROP POLICY IF EXISTS trace_gate_driver_cross_tenant_read ON trace_object_refs;
CREATE POLICY trace_gate_driver_cross_tenant_read ON trace_object_refs
    FOR SELECT TO trace_gate_driver USING (true);
DROP POLICY IF EXISTS trace_gate_driver_cross_tenant_read ON trace_gate_evaluation_attempts;
CREATE POLICY trace_gate_driver_cross_tenant_read ON trace_gate_evaluation_attempts
    FOR SELECT TO trace_gate_driver USING (true);
