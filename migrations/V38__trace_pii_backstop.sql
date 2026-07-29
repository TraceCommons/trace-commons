-- Server-side NEAR AI PII backstop: attempt bookkeeping table + cross-tenant reader role.

CREATE TABLE IF NOT EXISTS trace_pii_backstop (
    tenant_id        TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id    UUID NOT NULL,
    attempts         INT NOT NULL DEFAULT 0,
    last_attempt_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error_label TEXT,
    PRIMARY KEY (tenant_id, submission_id)
);

ALTER TABLE trace_pii_backstop ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_pii_backstop FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_pii_backstop;
CREATE POLICY trace_corpus_tenant_isolation ON trace_pii_backstop
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_pii_backstop_driver') THEN
        CREATE ROLE trace_pii_backstop_driver NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_pii_backstop_driver SET statement_timeout = '5s';

-- Column-level grants only: exactly the columns
-- `list_submissions_awaiting_pii_backstop` (crates/trace-commons-server/src/
-- db/trace_corpus_pg.rs) selects, joins on, or filters by. A compromised
-- driver credential must not be able to enumerate object keys, tenant
-- identity beyond scoping, or any other cross-tenant column.
REVOKE ALL ON trace_submissions FROM trace_pii_backstop_driver;
REVOKE ALL ON trace_object_refs FROM trace_pii_backstop_driver;
REVOKE ALL ON trace_pii_backstop FROM trace_pii_backstop_driver;

GRANT SELECT (tenant_id, submission_id, received_at, status)
    ON trace_submissions TO trace_pii_backstop_driver;
GRANT SELECT (tenant_id, submission_id, artifact_kind, invalidated_at, deleted_at)
    ON trace_object_refs TO trace_pii_backstop_driver;
GRANT SELECT (tenant_id, submission_id, attempts, last_attempt_at)
    ON trace_pii_backstop TO trace_pii_backstop_driver;

DROP POLICY IF EXISTS trace_pii_backstop_driver_cross_tenant_read ON trace_submissions;
CREATE POLICY trace_pii_backstop_driver_cross_tenant_read ON trace_submissions
    FOR SELECT TO trace_pii_backstop_driver USING (true);
DROP POLICY IF EXISTS trace_pii_backstop_driver_cross_tenant_read ON trace_object_refs;
CREATE POLICY trace_pii_backstop_driver_cross_tenant_read ON trace_object_refs
    FOR SELECT TO trace_pii_backstop_driver USING (true);
DROP POLICY IF EXISTS trace_pii_backstop_driver_cross_tenant_read ON trace_pii_backstop;
CREATE POLICY trace_pii_backstop_driver_cross_tenant_read ON trace_pii_backstop
    FOR SELECT TO trace_pii_backstop_driver USING (true);
