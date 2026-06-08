-- Agent-driven onboarding device-key registry.
--
-- A device key is the post-invite credential Ironclaw uses to authenticate
-- upload-claim requests. The key id is derived from the raw public-key bytes
-- (`sha256:<hex>`), so the registry can reject cross-tenant key reuse without
-- storing any contributor identity.

CREATE TABLE IF NOT EXISTS device_keys (
    device_key_id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    public_key TEXT NOT NULL,
    invite_subject_hash TEXT NOT NULL,
    client_info JSONB NOT NULL DEFAULT '{}'::JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    CHECK (device_key_id ~ '^sha256:[0-9a-f]{64}$'),
    CHECK (invite_subject_hash ~ '^sha256:[0-9a-f]{64}$')
);

CREATE INDEX IF NOT EXISTS idx_device_keys_tenant_created
    ON device_keys (tenant_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_device_keys_tenant_active
    ON device_keys (tenant_id, created_at DESC)
    WHERE revoked_at IS NULL;

ALTER TABLE device_keys ENABLE ROW LEVEL SECURITY;
ALTER TABLE device_keys FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON device_keys;
CREATE POLICY trace_corpus_tenant_isolation ON device_keys
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
