-- Terms of Service acceptances, one row per account, org, terms version and
-- document hash (docs/superpowers/specs/2026-11-09-terms-scroll-acceptance-design.md).
--
-- The contributor app shows the terms read-through, then its daemon posts the
-- version id and the SHA-256 of the exact bytes it rendered to
-- POST /v1/account/terms-acceptance. The server never refuses an upload over
-- terms and does not validate the hash against the published document; this
-- table only answers who accepted what, and when.
--
-- WHAT A ROW HOLDS. The authenticated tenant and account, how the request
-- authenticated (auth_method) and which kind of client sent it
-- (client_kind), the version id, the hash, the client's claimed
-- accepted_at, and the database clock's recorded_at. No raw URL, address or
-- user agent.
--
-- APPEND-ONLY. The ingest runtime may SELECT and INSERT the listed columns;
-- it has no UPDATE or DELETE. A repeat of the unique key is a no-op in the
-- writer (ON CONFLICT DO NOTHING), which then reads the original row's
-- recorded_at. Rows leave only with their tenant or account (ON DELETE
-- CASCADE).
--
-- Applied by a non-superuser CREATEROLE migrator: it creates no role
-- (trace_ingest_runtime exists since V90), and GRANT on a table needs only
-- its ownership.

CREATE TABLE IF NOT EXISTS account_terms_acceptances (
    tenant_id      TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id     UUID NOT NULL,
    auth_method    TEXT NOT NULL,
    client_kind    TEXT NOT NULL,
    terms_version  TEXT NOT NULL CHECK (terms_version ~ '^[A-Za-z0-9._-]{1,64}$'),
    content_sha256 TEXT NOT NULL CHECK (content_sha256 ~ '^[0-9a-f]{64}$'),
    accepted_at    TIMESTAMPTZ NOT NULL,
    recorded_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, account_id, terms_version, content_sha256),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE
);

ALTER TABLE account_terms_acceptances ENABLE ROW LEVEL SECURITY;
ALTER TABLE account_terms_acceptances FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON account_terms_acceptances;
CREATE POLICY trace_corpus_tenant_isolation ON account_terms_acceptances
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

REVOKE ALL ON account_terms_acceptances FROM PUBLIC;
GRANT SELECT ON account_terms_acceptances TO trace_ingest_runtime;
GRANT INSERT (tenant_id, account_id, auth_method, client_kind, terms_version,
              content_sha256, accepted_at)
    ON account_terms_acceptances TO trace_ingest_runtime;
