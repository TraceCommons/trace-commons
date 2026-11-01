-- A contributor adds the model_training use to traces already sent (owner
-- decisions 2026-10-28).
--
-- The scoring award (NoveltyUtility) requires model_training in a
-- submission's stored allowed_uses, which until now were fixed at upload.
-- A contributor who sent traces without it can now add it, and only it:
-- the grant appends "model_training" to trace_submissions.consent_scopes and
-- .allowed_uses and never removes anything (removal stays withdrawal).
--
-- trace_consent_grants: one row per trace per added scope, dated when the
-- contributor gave it, never backdated. The actor is the principal storage
-- ref the request authenticated as (already a hash-derived ref, as in
-- trace_submissions.auth_principal_ref). Insert-only: a repeat grant finds
-- the row and leaves it. The table accepts only the one scope and use this
-- path may add.

CREATE TABLE IF NOT EXISTS trace_consent_grants (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    submission_id UUID NOT NULL,
    consent_scope TEXT NOT NULL CHECK (consent_scope = 'model_training'),
    allowed_use TEXT NOT NULL CHECK (allowed_use = 'model_training'),
    actor_principal_ref TEXT NOT NULL CHECK (char_length(actor_principal_ref) BETWEEN 1 AND 256),
    granted_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, submission_id, consent_scope),
    FOREIGN KEY (tenant_id, submission_id)
        REFERENCES trace_submissions(tenant_id, submission_id) ON DELETE CASCADE
);

ALTER TABLE trace_consent_grants ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_consent_grants FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_consent_grants;
CREATE POLICY trace_corpus_tenant_isolation ON trace_consent_grants
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Runtime grants, on V90's terms: the ingest runtime owns nothing and sees a
-- new table only through what is granted here. The grant inserts a row and
-- the eligibility and grant reads select. On trace_submissions the grant
-- writes the two consent columns and updated_at; the pilot's runtime already
-- holds table-wide UPDATE from its V62 grants, and a deployment built from
-- V90 onward gets exactly these columns.
REVOKE ALL ON trace_consent_grants FROM PUBLIC;
GRANT SELECT, INSERT ON trace_consent_grants TO trace_ingest_runtime;
GRANT UPDATE (consent_scopes, allowed_uses, updated_at) ON trace_submissions
    TO trace_ingest_runtime;
