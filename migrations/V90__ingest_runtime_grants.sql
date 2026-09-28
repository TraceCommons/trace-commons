-- The ingest runtime's grants on tables that migrations since V62 created
-- without granting it anything.
--
-- A least-privilege deployment connects ingest as a login that owns nothing.
-- The pilot's login holds its table privileges through a NOLOGIN group,
-- trace_ingest_runtime, granted table-wide once when the schema was at V62;
-- pg_default_acl is empty, so every later table is invisible to it until
-- something grants on it. Three of those tables sit on routes every released
-- client calls:
--
--   * V78: every submission and status write takes FOR UPDATE on the
--     source-session row, and withdrawal reads the submission mapping and
--     stamps withdrawn_at. Without these grants every new upload and every
--     withdrawal fails with permission denied.
--   * V76: the idempotent re-POST of an existing submission reads the stored
--     witness evidence, and a verified certificate writes it.
--   * V65-V68: the invoker-rights trigger trace_revoke_token_bundles() on
--     trace_submissions updates trace_token_bundles whenever a submission is
--     revoked, withdrawn, purged or re-scrubbed, and withdrawal's token
--     cleanup reads both token tables and marks attachments deleted.
--
-- This migration names that group as the schema's ingest runtime role. Where
-- it already exists (the pilot) its grants are repaired in place; elsewhere it
-- is created, and an operator whose ingest login is not the migrator grants it
-- once: GRANT trace_ingest_runtime TO <ingest runtime login>.
--
-- Least privilege, per path:
--
--   * trace_submission_sessions: SELECT only. The runtime never writes a
--     mapping outside account admission (below) or an account merge (the V78
--     definer function).
--   * trace_source_sessions: SELECT and UPDATE (withdrawn_at). A row lock needs
--     UPDATE on some column, and withdrawn_at is the one withdrawal writes.
--   * Claiming a source session inserts into both tables, and only the
--     account-admission path claims one, so INSERT goes to
--     trace_account_admission_runtime, not to the general runtime.
--   * trace_witness_certificate_evidence: membership in V76's
--     trace_witness_evidence_runtime, which already holds exactly that
--     table's column grants.
--   * Token bundles: column grants, not a SECURITY DEFINER trigger. The
--     withdraw handler's pending_token_bundle_deletions() issues the same
--     UPDATE (state, processing_state, processing_summary) directly on the
--     runtime pool, so a definer trigger would leave the runtime needing those
--     three columns anyway and add a guard role for nothing. The runtime gets
--     SELECT on both tables (the cleanup reads whole rows), those three
--     bundle columns, and UPDATE (deleted, prepared) on attachments, which is
--     what marking a withdrawn bundle's objects deleted writes. Creating,
--     staging, committing and processing bundles (the opt-in
--     TRACE_COMMONS_BUNDLE_SERVER_ID routes) are not granted here.
--   * trace_accounts: the pilot's table-wide UPDATE lets the runtime rewrite
--     account_id, which account admission's readiness check refuses. Ingest
--     writes one column of trace_accounts, closed_at, when a merge closes the
--     absorbed account; created_at is the column V75/V77 grant for row locks.
--     Every other trace_accounts write in ingest is an INSERT, which this does
--     not touch.
--
-- Idempotent, and applied by a non-superuser CREATEROLE migrator: CREATE ROLE
-- is guarded, table GRANT/REVOKE only needs table ownership, and granting
-- trace_witness_evidence_runtime needs the ADMIN option the migrator holds by
-- having created it in V76 (PostgreSQL 16) or CREATEROLE (15 and earlier).

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        CREATE ROLE trace_ingest_runtime NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
-- A pre-existing group is used as found, but never one that defeats forced
-- RLS. Naming SUPERUSER or BYPASSRLS in ALTER ROLE is superuser-only, so
-- refuse rather than repair.
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime'
               AND (rolsuper OR rolbypassrls)) THEN
        RAISE EXCEPTION 'V90: trace_ingest_runtime is privileged';
    END IF;
END $$;

GRANT USAGE ON SCHEMA public TO trace_ingest_runtime;

-- V78 source sessions.
GRANT SELECT ON trace_submission_sessions TO trace_ingest_runtime;
GRANT SELECT, UPDATE (withdrawn_at) ON trace_source_sessions TO trace_ingest_runtime;
GRANT INSERT ON trace_source_sessions, trace_submission_sessions
    TO trace_account_admission_runtime;

-- V76 witness evidence. Membership rows are per server, not per database, so
-- skip the GRANT when it is already there rather than rewrite the catalog row
-- another database's run just wrote.
DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_auth_members m
         WHERE m.roleid = 'trace_witness_evidence_runtime'::regrole
           AND m.member = 'trace_ingest_runtime'::regrole
    ) THEN
        GRANT trace_witness_evidence_runtime TO trace_ingest_runtime;
    END IF;
END $$;

-- V65-V68 token bundles.
GRANT SELECT ON trace_token_bundles, trace_token_attachments TO trace_ingest_runtime;
GRANT UPDATE (state, processing_state, processing_summary)
    ON trace_token_bundles TO trace_ingest_runtime;
GRANT UPDATE (deleted, prepared) ON trace_token_attachments TO trace_ingest_runtime;

-- trace_accounts: only the columns ingest writes or locks with. REVOKE on the
-- table also removes any column-level UPDATE, account_id's included.
REVOKE UPDATE ON trace_accounts FROM trace_ingest_runtime;
GRANT UPDATE (created_at, closed_at) ON trace_accounts TO trace_ingest_runtime;

-- A REVOKE only removes grants this role made, and PostgreSQL warns rather
-- than fails otherwise. Check the result, not the statement: the group itself
-- must hold no UPDATE on account_id, directly or table-wide.
DO $$ BEGIN
    IF EXISTS (
        SELECT 1 FROM pg_class c, aclexplode(c.relacl) a
         WHERE c.oid = 'public.trace_accounts'::regclass
           AND a.grantee = 'trace_ingest_runtime'::regrole
           AND a.privilege_type = 'UPDATE'
    ) OR EXISTS (
        SELECT 1 FROM pg_attribute att, aclexplode(att.attacl) a
         WHERE att.attrelid = 'public.trace_accounts'::regclass
           AND att.attname = 'account_id'
           AND a.grantee = 'trace_ingest_runtime'::regrole
           AND a.privilege_type = 'UPDATE'
    ) THEN
        RAISE EXCEPTION 'V90: trace_ingest_runtime can still update trace_accounts.account_id';
    END IF;
END $$;
