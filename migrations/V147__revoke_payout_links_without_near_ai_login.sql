-- V147: revoke every live payout link on an account with no recorded NEAR AI
-- login (owner decision 2026-11-03, on #1862).
--
-- Since #1862 a payout link is proved only by one of the account's recorded
-- `near_ai_login` anchors, on staging and production alike ("It should just
-- be NEAR AI login on both staging and production", owner, 2026-11-02).
-- Links made before that on an account with no such anchor were proved some
-- other way: before #1841 with no subject check at all (B9), and between
-- #1841 and #1862 through the NEAR AI wallet sign-in. The owner chose to
-- revoke them at deploy rather than keep paying them. A migration, so the
-- revocation cannot be forgotten and runs before the new code serves.
--
-- WHAT A REVOCATION IS. The two marks the payout pass already honours, set
-- on every `waiting` or `active` link of such an account:
--   * `recipient_unavailable_at`: the pass sends nothing to the link, own or
--     referral credit (`payout_pass.rs`, `pay` and the referral stream);
--   * `unlink_requested_at`: the pass moves an active link to `closing` and
--     closes it with a zero send that reads the program total, as for a
--     person's own unlink. A waiting link it may still activate, but it pays
--     nothing (it is unavailable) and is closed on the next pass.
-- No row is deleted, so the account's link version (links plus unlinks)
-- only goes up, and every state change still goes through the pass.
-- `closing` and `closed` links are already ending and are left alone.
--
-- THE WINDOW AFTER DEPLOY. A link request started under the old code waits
-- up to five minutes for its confirm, and the confirm does not re-read
-- anchors. Every pending `link` request of such an account is deleted here,
-- so its confirm answers `not_found`. This migration runs when the new
-- service starts, before it serves; after it, a start for such an account
-- is refused by the new check. This assumes the old service is stopped
-- before the new one migrates (the pilot's single-process restart); a
-- deployment that keeps an old process serving across the migration must
-- stop it first.
--
-- AUDIT. One `trace_payout_audit` row per revoked link, hash-only, in the
-- shapes the service writes: action `unlink_requested`, the link's state as
-- both prior and new state, label `near_ai_login_required`, the account and
-- link as `payout-account-v1:` / `payout-link-v1:` SHA-256 hashes (as
-- `account_id_hash` / `link_id_hash` in `postgres_payout.rs`), and a fixed
-- actor hash for this migration. Deleted requests get no row: no request
-- audit action fits, and the request's own `link_requested` row stays.
--
-- IDEMPOTENT. A link already carrying both marks is not selected again, so a
-- re-run changes nothing and writes no second audit row.
--
-- FORCED ROW SECURITY applies to the table owner too, so a plain UPDATE or
-- DELETE here, run by the non-superuser migrator, would silently touch zero
-- rows in every tenant (V144). FORCE is lifted for the tables this file reads
-- and writes and put straight back.

ALTER TABLE trace_near_account_anchors NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_links NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_link_requests NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_audit NO FORCE ROW LEVEL SECURITY;

WITH revoked AS (
    UPDATE trace_payout_links AS l
       SET recipient_unavailable_at = COALESCE(l.recipient_unavailable_at, clock_timestamp()),
           unlink_requested_at = COALESCE(l.unlink_requested_at, clock_timestamp()),
           version = l.version + 1,
           updated_at = clock_timestamp()
     WHERE l.state IN ('waiting', 'active')
       AND (l.recipient_unavailable_at IS NULL OR l.unlink_requested_at IS NULL)
       AND NOT EXISTS (
           SELECT 1 FROM trace_near_account_anchors a
            WHERE a.tenant_id = l.tenant_id
              AND a.account_id = l.account_id
              AND a.identity_source = 'near_ai_login')
    RETURNING l.tenant_id, l.rail_id, l.account_id, l.link_id, l.state
)
INSERT INTO trace_payout_audit (
    tenant_id, audit_id, rail_id, action, actor_ref_hash, account_id_hash,
    link_id_hash, prior_state, new_state, label
)
SELECT r.tenant_id,
       gen_random_uuid(),
       r.rail_id,
       'unlink_requested',
       'sha256:' || encode(sha256(convert_to('payout-migration-actor-v1:V147', 'UTF8')), 'hex'),
       'sha256:' || encode(sha256(convert_to('payout-account-v1:' || r.account_id::text, 'UTF8')), 'hex'),
       'sha256:' || encode(sha256(convert_to('payout-link-v1:' || r.link_id::text, 'UTF8')), 'hex'),
       r.state,
       r.state,
       'near_ai_login_required'
  FROM revoked r;

DELETE FROM trace_payout_link_requests AS q
 WHERE q.action = 'link'
   AND NOT EXISTS (
       SELECT 1 FROM trace_near_account_anchors a
        WHERE a.tenant_id = q.tenant_id
          AND a.account_id = q.account_id
          AND a.identity_source = 'near_ai_login');

ALTER TABLE trace_near_account_anchors FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_links FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_link_requests FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_audit FORCE ROW LEVEL SECURITY;
