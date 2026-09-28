-- An account's binding state, for native passkey identity (Z2, slice S1;
-- docs/superpowers/specs/2026-09-28-native-passkey-identity-design.md).
--
-- A passkey-origin account is created `unbound` and flips to `bound` in the
-- one transaction that attaches its near.ai identity (S3). Until then ingest
-- restricts its sessions to a short allowlist of account routes. This
-- migration is inert on its own: nothing writes a row until S2.
--
-- ABSENCE OF A ROW MEANS LEGACY, NOT UNBOUND. Every account that exists today,
-- and every account created by a path other than passkey creation, has no row
-- and is never gated. So a passkey-origin account must never exist without
-- its row: ingest writes both in one transaction, through
-- PgBackend::insert_passkey_origin_account, and nothing else writes a
-- passkey-origin account.
--
-- What the S5 reaper keys on is already here, so it needs no schema change:
-- `state = 'unbound'`, `created_at` (partial index below), and last activity,
-- which is trace_sessions.last_seen_at -- slid forward by every authenticated
-- request and never deleted, so no activity column is duplicated here and no
-- per-request write is added.
--
-- Grants: the ingest runtime gets SELECT only, which is what S1 does (the
-- session validation query joins this table). Inserting (S2) and flipping to
-- bound (S3) are granted by the migrations that add those writers, so a
-- runtime compromised today cannot fabricate or change a binding.
--
-- Idempotent where it can be, and applied by a non-superuser CREATEROLE
-- migrator: it creates no role (trace_ingest_runtime exists since V90), and
-- GRANT on a table needs only its ownership.

CREATE TABLE IF NOT EXISTS trace_account_bindings (
    tenant_id  TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id UUID NOT NULL,
    -- One value now; a later account-less origin is a widened CHECK, not a
    -- new table.
    origin     TEXT NOT NULL CHECK (origin = 'passkey'),
    state      TEXT NOT NULL CHECK (state IN ('unbound', 'bound', 'closed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    bound_at   TIMESTAMPTZ,
    CHECK ((state = 'bound') = (bound_at IS NOT NULL)),
    PRIMARY KEY (tenant_id, account_id),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE
);

-- The S5 reaper's scan: unbound accounts older than the TTL.
CREATE INDEX IF NOT EXISTS trace_account_bindings_unbound_created_idx
    ON trace_account_bindings (created_at) WHERE state = 'unbound';

ALTER TABLE trace_account_bindings ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_account_bindings FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_account_bindings;
CREATE POLICY trace_corpus_tenant_isolation ON trace_account_bindings
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

GRANT SELECT ON trace_account_bindings TO trace_ingest_runtime;
