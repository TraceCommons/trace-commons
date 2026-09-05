-- Explicit default-disabled NEAR provisioning. No access grant or credit is minted.
-- Pre-account ceremonies are scoped by possession of an unpredictable hash-keyed
-- handle, not tenant: no tenant exists until wallet/device control is proven.
CREATE TABLE trace_near_provisioning_ceremonies (
    ceremony_hash TEXT PRIMARY KEY CHECK (ceremony_hash ~ '^sha256:[0-9a-f]{64}$'),
    payload JSONB NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE trace_near_provisioning_ceremonies ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_near_provisioning_ceremonies FORCE ROW LEVEL SECURITY;
CREATE POLICY trace_near_ceremony_isolation ON trace_near_provisioning_ceremonies
    USING (ceremony_hash = current_setting('trace_commons.near_ceremony_hash', true))
    WITH CHECK (ceremony_hash = current_setting('trace_commons.near_ceremony_hash', true));
CREATE INDEX trace_near_provisioning_expiry ON trace_near_provisioning_ceremonies(expires_at);

CREATE TABLE trace_near_account_anchors (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id),
    anchor_hash TEXT NOT NULL CHECK (anchor_hash ~ '^sha256:[0-9a-f]{64}$'),
    account_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, anchor_hash),
    UNIQUE (anchor_hash),
    UNIQUE (tenant_id, account_id, anchor_hash),
    CHECK (tenant_id = 'near-' || substring(anchor_hash from 8)),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
ALTER TABLE trace_near_account_anchors ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_near_account_anchors FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_near_account_anchors;
CREATE POLICY trace_corpus_tenant_isolation ON trace_near_account_anchors
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- A missing invite is represented honestly. Existing callers retain String on
-- writes, while reads use Option; the explicit origin prevents interpretation of
-- native provisioning as an invite redemption.
ALTER TABLE device_keys ALTER COLUMN invite_subject_hash DROP NOT NULL;
ALTER TABLE device_keys ADD COLUMN onboarding_origin TEXT NOT NULL DEFAULT 'invite'
    CHECK (onboarding_origin IN ('invite', 'near'));
ALTER TABLE device_keys ADD CONSTRAINT device_keys_invite_origin_binding_check
    CHECK ((onboarding_origin = 'invite' AND invite_subject_hash IS NOT NULL)
        OR (onboarding_origin = 'near' AND invite_subject_hash IS NULL));

CREATE TABLE trace_near_provisioned_devices (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id),
    principal_ref TEXT NOT NULL,
    account_id UUID NOT NULL,
    device_key_id TEXT NOT NULL REFERENCES device_keys(device_key_id),
    anchor_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, principal_ref),
    UNIQUE (tenant_id, device_key_id),
    FOREIGN KEY (tenant_id, account_id, anchor_hash)
        REFERENCES trace_near_account_anchors(tenant_id, account_id, anchor_hash)
);
ALTER TABLE trace_near_provisioned_devices ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_near_provisioned_devices FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_near_provisioned_devices;
CREATE POLICY trace_corpus_tenant_isolation ON trace_near_provisioned_devices
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
