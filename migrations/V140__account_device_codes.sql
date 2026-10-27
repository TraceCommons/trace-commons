-- Linking a headless device with a one-time code (owner decisions
-- 2026-10-27: the signed-in laptop mints, the server redeems; a device that
-- joined by code renews its own session with its device key).
--
-- trace_account_device_codes: one row per minted code. Only the hash of the
-- code's secret is stored. `mint_nonce_hash` is the hash of the nonce in the
-- minting device's proof, unique so a captured proof cannot mint twice.
-- `redeemed_device_key_id` names the device that used the code; a device
-- with such a row is the only kind that may renew its session by key.
--
-- trace_device_session_renewals: the nonce of every accepted renewal proof,
-- unique so a captured proof cannot be replayed.

CREATE TABLE IF NOT EXISTS trace_account_device_codes (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    code_id UUID NOT NULL,
    account_id UUID NOT NULL,
    code_hash TEXT NOT NULL UNIQUE CHECK (code_hash ~ '^sha256:[0-9a-f]{64}$'),
    minted_by_device_key_id TEXT NOT NULL REFERENCES device_keys(device_key_id),
    mint_nonce_hash TEXT NOT NULL UNIQUE CHECK (mint_nonce_hash ~ '^sha256:[0-9a-f]{64}$'),
    label TEXT CHECK (label IS NULL OR char_length(label) BETWEEN 1 AND 64),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    redeemed_at TIMESTAMPTZ,
    redeemed_device_key_id TEXT REFERENCES device_keys(device_key_id),
    PRIMARY KEY (tenant_id, code_id),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE,
    CHECK ((redeemed_at IS NULL) = (redeemed_device_key_id IS NULL))
);

CREATE INDEX IF NOT EXISTS idx_trace_account_device_codes_outstanding
    ON trace_account_device_codes (tenant_id, account_id, expires_at)
    WHERE redeemed_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_trace_account_device_codes_redeemed_device
    ON trace_account_device_codes (tenant_id, redeemed_device_key_id)
    WHERE redeemed_device_key_id IS NOT NULL;

ALTER TABLE trace_account_device_codes ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_device_codes FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_device_codes;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_device_codes
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

CREATE TABLE IF NOT EXISTS trace_device_session_renewals (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    nonce_hash TEXT NOT NULL PRIMARY KEY CHECK (nonce_hash ~ '^sha256:[0-9a-f]{64}$'),
    device_key_id TEXT NOT NULL REFERENCES device_keys(device_key_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_trace_device_session_renewals_device
    ON trace_device_session_renewals (tenant_id, device_key_id, created_at DESC);

ALTER TABLE trace_device_session_renewals ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_device_session_renewals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_device_session_renewals;
CREATE POLICY trace_corpus_tenant_isolation ON trace_device_session_renewals
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Runtime grants, on V90's terms: the ingest runtime owns nothing and sees a
-- new table only through what is granted here. Minting inserts a code;
-- redeeming locks the row and stamps the two redemption columns; the status
-- read selects. Renewal inserts a nonce and reads the last one.
REVOKE ALL ON trace_account_device_codes, trace_device_session_renewals FROM PUBLIC;
GRANT SELECT, INSERT ON trace_account_device_codes TO trace_ingest_runtime;
GRANT UPDATE (redeemed_at, redeemed_device_key_id) ON trace_account_device_codes
    TO trace_ingest_runtime;
GRANT SELECT, INSERT ON trace_device_session_renewals TO trace_ingest_runtime;
