-- An account may select one operator-published inference connection at a time.
-- These rows record a choice, not a provider credential or device installation.
CREATE TABLE trace_account_inference_connections (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    connection_id UUID NOT NULL,
    offer_id TEXT NOT NULL,
    provider_id TEXT NOT NULL,
    revision TEXT NOT NULL CHECK (revision ~ '^sha256:[0-9a-f]{64}$'),
    config_digest TEXT NOT NULL CHECK (config_digest ~ '^sha256:[0-9a-f]{64}$'),
    disclosure_version TEXT NOT NULL,
    state_version BIGINT NOT NULL CHECK (state_version > 0),
    selected_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, account_id, connection_id),
    UNIQUE (tenant_id, account_id, state_version),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE CASCADE
);
CREATE UNIQUE INDEX trace_account_inference_connection_live
    ON trace_account_inference_connections (tenant_id, account_id)
    WHERE revoked_at IS NULL;

CREATE TABLE trace_account_inference_connection_requests (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    idempotency_key UUID NOT NULL,
    request_digest TEXT NOT NULL CHECK (request_digest ~ '^sha256:[0-9a-f]{64}$'),
    connection_id UUID NOT NULL,
    state_version BIGINT NOT NULL CHECK (state_version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, account_id, idempotency_key),
    FOREIGN KEY (tenant_id, account_id, connection_id)
        REFERENCES trace_account_inference_connections(tenant_id, account_id, connection_id)
        ON DELETE CASCADE
);

CREATE TABLE trace_account_inference_connection_events (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    connection_id UUID NOT NULL,
    event_type TEXT NOT NULL CHECK (event_type IN
        ('inference_connection_selected', 'inference_connection_revoked')),
    state_version BIGINT NOT NULL CHECK (state_version > 0),
    revision TEXT NOT NULL CHECK (revision ~ '^sha256:[0-9a-f]{64}$'),
    config_digest TEXT NOT NULL CHECK (config_digest ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, account_id, connection_id, event_type),
    FOREIGN KEY (tenant_id, account_id, connection_id)
        REFERENCES trace_account_inference_connections(tenant_id, account_id, connection_id)
        ON DELETE CASCADE
);

ALTER TABLE trace_account_inference_connections ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_inference_connections FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_inference_connections;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_inference_connections
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
ALTER TABLE trace_account_inference_connection_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_inference_connection_requests FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_inference_connection_requests;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_inference_connection_requests
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
ALTER TABLE trace_account_inference_connection_events ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_inference_connection_events FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_inference_connection_events;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_inference_connection_events
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_inference_connection_runtime') THEN
        CREATE ROLE trace_inference_connection_runtime NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_inference_connection_runtime;
GRANT SELECT (tenant_id, account_id, closed_at), UPDATE (account_id)
    ON trace_accounts TO trace_inference_connection_runtime;
GRANT SELECT (tenant_id, account_id)
    ON trace_near_account_anchors TO trace_inference_connection_runtime;
GRANT SELECT, INSERT ON trace_account_inference_connections
    TO trace_inference_connection_runtime;
GRANT UPDATE (revoked_at, state_version) ON trace_account_inference_connections
    TO trace_inference_connection_runtime;
GRANT SELECT, INSERT ON trace_account_inference_connection_requests
    TO trace_inference_connection_runtime;
GRANT INSERT ON trace_account_inference_connection_events
    TO trace_inference_connection_runtime;
