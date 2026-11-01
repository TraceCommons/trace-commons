-- Remove passkeys (owner decision 2026-10-31: "completely remove passkeys
-- from the system right now. We are just going to use normal near ai cloud
-- logins").
--
-- Forward-only. Everything below runs in the one transaction the migration
-- runner opens for this file, so a database either has every step or none.
--
-- WHAT HAPPENS TO PASSKEY-ORIGIN ACCOUNTS
--   * `bound` (V97): untouched. The account keeps its near.ai anchor, its
--     principals, devices and sessions, and signs in with NEAR AI as before.
--   * `unbound` and `closed`: deleted. Their sessions, login links, near
--     identities and other account-keyed rows go with them through the
--     existing ON DELETE CASCADE from trace_accounts (the set V101's header
--     lists). Their tenant row is left in place, as the V101 reaper left it.
--   * An `unbound` or `closed` account that a non-cascading row keeps from
--     being deleted (ON DELETE RESTRICT / NO ACTION: account trust and
--     admission, legacy invite links, payout and referral tables, near
--     anchors) is not deleted and does not stop the migration. It is closed
--     (trace_accounts.closed_at) with every session revoked, which is what
--     makes it unusable without its binding row: session validation requires
--     `closed_at IS NULL`, and so do the NEAR AI anchor lookups.
--
-- SESSIONS A PASSKEY AUTHENTICATED, in every account, are revoked: a browser
-- passkey session (client_kind 'passkey') and a native session minted by a
-- native passkey sign-in both name their credential in auth_credential_id.
-- That includes `bound` and legacy accounts; they sign in again with NEAR AI
-- or a login link. Sessions any other way minted are not touched.
--
-- ORDER MATTERS. Every target is closed and its sessions revoked BEFORE any
-- delete, and all of it BEFORE trace_account_bindings is dropped: V97 reads a
-- missing binding row as a legacy account, which is never gated, so dropping
-- the table first would leave a surviving unbound account ungated.
--
-- FORCED ROW SECURITY applies to the table owner too, so a plain UPDATE or
-- DELETE here, run by the non-superuser migrator, would silently touch zero
-- rows in every tenant (V50 and V109 hit the same thing). FORCE is lifted for
-- the tables this file reads and writes directly and put straight back
-- (the two passkey tables are then dropped). Cascaded
-- deletes are referential actions, which run with row security off.
--
-- AUDIT. trace_account_audit and trace_audit_events are keyed to the tenant,
-- not the account, and are not touched: hash-only rows that record past
-- passkey events stay readable.
--
-- WHAT IS DROPPED
--   * trace_unbound_passkey_account_count() (V98, V102) and
--     trace_reap_unbound_accounts(BIGINT, BIGINT, INTEGER) (V101). Each is
--     owned by its NOLOGIN guard, so the migrator holds the guard just long
--     enough to drop it, as V98 and V101 did to create them.
--   * The reaper guard's grants and permissive policies on trace_accounts
--     (V101).
--   * trace_account_bindings (V97) and trace_webauthn_credentials (V32), with
--     their policies and column grants (including the login resolver's V32
--     grant).
--
-- WHAT IS KEPT
--   * The roles trace_unbound_account_count_guard,
--     trace_unbound_account_reaper_guard and trace_unbound_account_reaper.
--     Roles are per server, not per database: another database on the same
--     server that ran V98/V101 still holds grants to them, and DROP ROLE would
--     fail there and abort this migration. After this file they hold no
--     privilege in this database. An operator may drop them by hand once every
--     database on the server is past V144.
--   * trace_sessions' V32 columns (auth_credential_id and the rotation pair)
--     and the client_kind CHECK that still admits 'passkey'. They are labels on
--     existing rows; the service no longer treats a 'passkey' session as a
--     strong one.
--
-- Applied by a non-superuser CREATEROLE migrator, like V98 and V101.

ALTER TABLE trace_account_bindings NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_accounts NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_sessions NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_webauthn_credentials NO FORCE ROW LEVEL SECURITY;

CREATE TEMPORARY TABLE v144_passkey_targets ON COMMIT DROP AS
    SELECT tenant_id, account_id
      FROM trace_account_bindings
     WHERE state IN ('unbound', 'closed');

-- 1. Close every target. A survivor of step 4 stays closed.
UPDATE trace_accounts a
   SET closed_at = COALESCE(a.closed_at, now())
  FROM v144_passkey_targets t
 WHERE a.tenant_id = t.tenant_id AND a.account_id = t.account_id;

-- 2. Revoke every target's sessions, live or not.
UPDATE trace_sessions s
   SET revoked_at = COALESCE(s.revoked_at, now())
  FROM v144_passkey_targets t
 WHERE s.tenant_id = t.tenant_id AND s.account_id = t.account_id;

-- 3. Revoke every live session a passkey authenticated, in any account.
UPDATE trace_sessions s
   SET revoked_at = now()
  FROM trace_webauthn_credentials c
 WHERE s.tenant_id = c.tenant_id
   AND s.auth_credential_id = c.credential_id
   AND s.revoked_at IS NULL;

-- 4. Delete each target, one sub-transaction each. A non-cascading reference
--    refuses the delete with 23503; that account stays closed (step 1) and
--    sessionless (step 2), and the rest go on.
DO $$
DECLARE
    t RECORD;
BEGIN
    FOR t IN SELECT tenant_id, account_id FROM v144_passkey_targets LOOP
        BEGIN
            DELETE FROM trace_accounts a
             WHERE a.tenant_id = t.tenant_id AND a.account_id = t.account_id;
        EXCEPTION WHEN foreign_key_violation THEN
            NULL;
        END;
    END LOOP;
END $$;

ALTER TABLE trace_account_bindings FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_accounts FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_sessions FORCE ROW LEVEL SECURITY;

-- 5. The definer functions, each dropped while holding its owning guard.
GRANT trace_unbound_account_count_guard TO CURRENT_USER;
DROP FUNCTION IF EXISTS public.trace_unbound_passkey_account_count();
REVOKE trace_unbound_account_count_guard FROM CURRENT_USER;

GRANT trace_unbound_account_reaper_guard TO CURRENT_USER;
DROP FUNCTION IF EXISTS trace_reap_unbound_accounts(BIGINT, BIGINT, INTEGER);
REVOKE trace_unbound_account_reaper_guard FROM CURRENT_USER;

-- 6. The reaper guard's hold on trace_accounts, which stays.
DROP POLICY IF EXISTS trace_unbound_reaper_read ON trace_accounts;
DROP POLICY IF EXISTS trace_unbound_reaper_lock ON trace_accounts;
DROP POLICY IF EXISTS trace_unbound_reaper_delete ON trace_accounts;
REVOKE ALL ON trace_accounts FROM trace_unbound_account_reaper_guard;

-- 7. The passkey tables, with their policies and grants.
DROP TABLE trace_account_bindings;
DROP TABLE trace_webauthn_credentials;

-- 8. Nothing left in this database for the three roles to reach.
REVOKE USAGE ON SCHEMA public FROM trace_unbound_account_count_guard;
REVOKE USAGE ON SCHEMA public FROM trace_unbound_account_reaper_guard;
REVOKE USAGE ON SCHEMA public FROM trace_unbound_account_reaper;
