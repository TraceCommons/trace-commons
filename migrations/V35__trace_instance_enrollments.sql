-- V31: instance-enrollment ledger (control plane above per-user tenants).
-- Isolated on a parallel INSTANCE predicate, not the tenant predicate: this
-- table is intentionally cross-tenant (an instance maps to many per-user
-- tenants), so tenant RLS would defeat its purpose. Hash-only columns.

CREATE TABLE trace_instance_enrollments (
    instance_subject_hash TEXT NOT NULL
        CHECK (instance_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    user_subject_hash     TEXT NOT NULL
        CHECK (user_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    tenant_id             TEXT NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (instance_subject_hash, user_subject_hash)
);

CREATE INDEX idx_trace_instance_enrollments_instance
    ON trace_instance_enrollments (instance_subject_hash);

ALTER TABLE trace_instance_enrollments ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_instance_enrollments FORCE ROW LEVEL SECURITY;

CREATE OR REPLACE FUNCTION trace_current_instance_subject()
RETURNS TEXT
LANGUAGE SQL
STABLE
AS $$
    SELECT NULLIF(current_setting('trace_commons.instance_subject', true), '');
$$;

DROP POLICY IF EXISTS trace_instance_isolation ON trace_instance_enrollments;
CREATE POLICY trace_instance_isolation ON trace_instance_enrollments
    USING      (instance_subject_hash = trace_current_instance_subject())
    WITH CHECK (instance_subject_hash = trace_current_instance_subject());
