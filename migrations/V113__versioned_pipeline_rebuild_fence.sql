-- A committed fence for the index rebuild (delivery PR 5). While a tenant has
-- an unexpired row, no worker in any process claims that tenant's index
-- invalidations, so an invalidation cannot run before a rebuild's last write.
-- One row per rebuild: a rebuild extends and deletes only its own row, so one
-- rebuild never shortens or removes another's fence. A rebuild that ends
-- deletes its row, and the tenant's expired rows; one that is lost leaves a
-- row that expires.

CREATE TABLE pipeline_index_rebuild_fences (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    fence_id UUID NOT NULL,
    fenced_until TIMESTAMPTZ NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, fence_id)
);

ALTER TABLE pipeline_index_rebuild_fences ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_index_rebuild_fences FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_index_rebuild_fences;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_index_rebuild_fences
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V113: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- A rebuild inserts its row and extends it (only `fenced_until` changes),
-- deletes it when it ends, and the invalidation claim reads the rows.
GRANT SELECT, INSERT, DELETE ON pipeline_index_rebuild_fences TO trace_ingest_runtime;
GRANT UPDATE (fenced_until) ON pipeline_index_rebuild_fences TO trace_ingest_runtime;
