-- Upload claim principals are provenance, not account authenticators. Bind
-- them to the device whose signature was verified at claim issuance. Their
-- ownership follows that device's active account membership; no independent
-- membership can outlive a parent unlink or be moved to another account.
ALTER TABLE device_keys ADD CONSTRAINT device_keys_tenant_device_unique UNIQUE(tenant_id, device_key_id);
CREATE TABLE trace_device_upload_principals (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    upload_principal_ref TEXT NOT NULL CHECK (upload_principal_ref ~ '^principal_sha256:[0-9a-f]{64}$'),
    device_principal_ref TEXT NOT NULL CHECK (device_principal_ref ~ '^principal_sha256:[0-9a-f]{64}$'),
    device_key_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, upload_principal_ref),
    CHECK (upload_principal_ref <> device_principal_ref),
    CHECK (device_principal_ref = 'principal_sha256:' ||
        encode(sha256(convert_to('device:' || tenant_id || ':' || device_key_id, 'UTF8')), 'hex')),
    FOREIGN KEY(tenant_id, device_key_id) REFERENCES device_keys(tenant_id,device_key_id) ON DELETE CASCADE
);
CREATE INDEX trace_device_upload_principals_parent
    ON trace_device_upload_principals(tenant_id, device_principal_ref);
ALTER TABLE trace_device_upload_principals ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_device_upload_principals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_device_upload_principals;
CREATE POLICY trace_corpus_tenant_isolation ON trace_device_upload_principals
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Serialize both write directions. An upload-only alias must never later
-- become an independent login principal, even when login and issuance race.
CREATE FUNCTION trace_upload_principal_namespace_guard() RETURNS trigger
LANGUAGE plpgsql SET search_path = public, pg_temp AS $$
DECLARE candidate TEXT;
BEGIN
    IF TG_TABLE_NAME = 'trace_account_principals' THEN
        candidate := NEW.principal_ref;
    ELSE
        candidate := NEW.upload_principal_ref;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended(NEW.tenant_id || ':' || candidate, 135));
    IF TG_TABLE_NAME = 'trace_account_principals' THEN
        IF EXISTS (SELECT 1 FROM trace_device_upload_principals
                   WHERE tenant_id=NEW.tenant_id AND upload_principal_ref=candidate) THEN
            RAISE EXCEPTION 'device_upload_principal_namespace_conflict' USING ERRCODE='23514';
        END IF;
    ELSE
        IF EXISTS (SELECT 1 FROM trace_account_principals
                   WHERE tenant_id=NEW.tenant_id AND principal_ref=candidate) THEN
            RAISE EXCEPTION 'device_upload_principal_namespace_conflict' USING ERRCODE='23514';
        END IF;
    END IF;
    RETURN NEW;
END $$;
CREATE TRIGGER trace_upload_principal_namespace_guard
    BEFORE INSERT OR UPDATE ON trace_device_upload_principals
    FOR EACH ROW EXECUTE FUNCTION trace_upload_principal_namespace_guard();
CREATE TRIGGER trace_account_principal_namespace_guard
    BEFORE INSERT OR UPDATE OF tenant_id, principal_ref ON trace_account_principals
    FOR EACH ROW EXECUTE FUNCTION trace_upload_principal_namespace_guard();

-- security_invoker is required: a view must not inherit its owner's ability
-- to bypass RLS. PostgreSQL 15 is the supported minimum.
CREATE VIEW trace_account_owned_principals WITH (security_invoker = true) AS
    SELECT tenant_id, account_id, principal_ref
      FROM trace_account_principals WHERE unlinked_at IS NULL
    UNION
    SELECT p.tenant_id, p.account_id, u.upload_principal_ref AS principal_ref
      FROM trace_account_principals p
      JOIN trace_device_upload_principals u
        ON u.tenant_id = p.tenant_id AND u.device_principal_ref = p.principal_ref
     WHERE p.unlinked_at IS NULL;

REVOKE ALL ON trace_device_upload_principals, trace_account_owned_principals FROM PUBLIC;
GRANT SELECT, INSERT ON trace_device_upload_principals TO trace_ingest_runtime;
GRANT SELECT ON trace_account_owned_principals TO trace_ingest_runtime;
