-- V31: Supporting index for the account-scoped trace read-back list.
--
-- `GET /v1/account/traces` filters `auth_principal_ref = ANY($principal_set)`
-- under the caller's tenant and paginates by keyset over `(received_at DESC,
-- submission_id DESC)`. A tenant-leading index on `(tenant_id, received_at
-- DESC, submission_id DESC)` lets the keyset cursor totally-order rows across
-- any number of principals (Hardening H) without an offset scan.
--
-- This is an index on the existing `trace_submissions` table — NOT a new
-- table — so it adds no RLS-policy or FORCE-ROW-LEVEL-SECURITY coverage and is
-- intentionally absent from TRACE_COMMONS_RLS_TABLES and the migration-policy
-- coverage arrays.

CREATE INDEX IF NOT EXISTS idx_trace_submissions_account_keyset
    ON trace_submissions (tenant_id, received_at DESC, submission_id DESC);
