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
-- SCOPE. The predicate is "no `near_ai_login` anchor", not "proved by a
-- wallet": per the owner's instruction it revokes every live link on such
-- an account, including a principal-linked legacy or pilot account's own
-- links and links made before #1841 whatever proved them. It does NOT touch
-- a link V146 carried to a legacy account from a NEAR AI account (a live
-- row in `trace_legacy_payout_carries`): that link was proved by the NEAR
-- AI account's own recorded login, and it is how F11 pays legacy credit.
-- A legacy account whose own link is revoked here is paid again only
-- through a carry at a carry event (owner, 2026-11-03, on #1879: confirm,
-- tenant link, or a new account in a linked legacy tenant; never on a
-- pass); the carry lands as the revoked link's waiting successor, and the
-- pass closes the revoked link before activating it, so never both. It
-- cannot see one case: a link proved through #1841's wallet path on an
-- account that also holds a `near_ai_login` anchor (for example after a V82
-- merge) is kept, because `subject_index` is a payout-domain label and
-- cannot be compared with `anchor_hash`. Only the `near_ai` rail is
-- touched; the rule is that rail's proof.
--
-- WHAT A REVOCATION IS. The same two moves V146's carry release makes
-- (`trace_legacy_payout_release_carry`):
--   * a `waiting` link, which has never sent (only an active link is paid),
--     closes at `final_paid` 0, so the pass never activates it. V146's
--     guard allows that only for a carried link, so this file sets the
--     trigger aside for that one UPDATE and applies the guard's own
--     condition itself: the link has no payout line at all. A waiting link
--     with a line should not exist (the store writes lines only to active
--     links); if one does, this migration refuses to run rather than leave
--     it to be activated and paid, and an operator resolves it first.
--   * an `active` link gets `unlink_requested_at`, exactly as a person's
--     own unlink: on its next run the pass moves it to `closing`, before
--     anything is paid, and closes it with the zero send that reads the
--     program total (one zero send out of that run's send window).
-- `closing` and `closed` links are already ending and are left alone. No
-- recipient is marked unavailable, so status reports an unlink, not an
-- unavailable organization.
--
-- REQUESTS. Pending link and unlink requests are left to expire (five
-- minutes); nothing is deleted. The new service's confirm re-reads the
-- account's NEAR AI login and refuses a link without one, whatever the
-- deploy topology, logging `no_near_ai_login`. A pending unlink request of
-- a revoked active link fails its version check, since the unlink mark
-- moves the account's link version, and that link is being closed anyway.
-- A pass run by an old process that loaded links before this commits can
-- still act on them once: stop the old service before the new one starts.
--
-- AUDIT. One `trace_payout_audit` row per revoked link, hash-only, in the
-- shapes V146's release writes: `link_closed` (waiting to closed) or
-- `unlink_requested` (active to active), label `near_ai_login_required`,
-- the account and link hashed by V146's `trace_payout_account_hash` /
-- `trace_payout_link_hash` (the Rust `account_id_hash` / `link_id_hash`),
-- and a fixed actor hash for this migration.
--
-- IDEMPOTENT. A revoked waiting link is closed and a revoked active link
-- carries the unlink mark, so neither is selected again; a re-run changes
-- nothing and writes no second audit row.
--
-- LOCKS. Lifting FORCE and setting the trigger aside take an ACCESS
-- EXCLUSIVE lock on each table touched (anchors, links, lines, audit,
-- carries) for the whole migration, which stalls NEAR sign-ins (they read
-- the anchors). A `lock_timeout` makes a deploy that cannot get the locks
-- fail instead of queueing every login. The runner does not retry: the
-- service fails to start, and it is retried only if the supervisor
-- restarts it (the pilot's systemd unit does).
--
-- FORCED ROW SECURITY applies to the table owner too, so a plain UPDATE
-- here, run by the non-superuser migrator, would silently touch zero rows
-- in every tenant (V144). FORCE is lifted for the tables this file reads
-- and writes and put straight back.

SET LOCAL lock_timeout = '10s';

ALTER TABLE trace_near_account_anchors NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_links NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_lines NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_audit NO FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_payout_carries NO FORCE ROW LEVEL SECURITY;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM trace_payout_links l
         WHERE l.rail_id = 'near_ai' AND l.state = 'waiting'
           AND NOT EXISTS (
               SELECT 1 FROM trace_near_account_anchors a
                WHERE a.tenant_id = l.tenant_id
                  AND a.account_id = l.account_id
                  AND a.identity_source = 'near_ai_login')
           AND NOT EXISTS (
               SELECT 1 FROM trace_legacy_payout_carries c
                WHERE c.tenant_id = l.tenant_id
                  AND c.link_id = l.link_id
                  AND c.released_at IS NULL)
           AND EXISTS (
               SELECT 1 FROM trace_payout_lines p
                WHERE p.tenant_id = l.tenant_id AND p.link_id = l.link_id)
    ) THEN
        RAISE EXCEPTION 'V147: a waiting near_ai payout link on an account with no NEAR AI login has a payout line; resolve it before this migration runs'
            USING ERRCODE = 'check_violation';
    END IF;
END $$;

CREATE TEMPORARY TABLE v147_revoked ON COMMIT DROP AS
    SELECT l.tenant_id, l.rail_id, l.account_id, l.link_id, l.state
      FROM trace_payout_links l
     WHERE l.rail_id = 'near_ai'
       AND (l.state = 'waiting'
            OR (l.state = 'active' AND l.unlink_requested_at IS NULL))
       AND NOT EXISTS (
           SELECT 1 FROM trace_near_account_anchors a
            WHERE a.tenant_id = l.tenant_id
              AND a.account_id = l.account_id
              AND a.identity_source = 'near_ai_login')
       AND NOT EXISTS (
           SELECT 1 FROM trace_legacy_payout_carries c
            WHERE c.tenant_id = l.tenant_id
              AND c.link_id = l.link_id
              AND c.released_at IS NULL);

ALTER TABLE trace_payout_links DISABLE TRIGGER trace_payout_links_guard_update;
UPDATE trace_payout_links AS l
   SET state = 'closed', final_paid = 0, version = l.version + 1,
       updated_at = clock_timestamp()
  FROM v147_revoked r
 WHERE r.state = 'waiting'
   AND l.tenant_id = r.tenant_id AND l.link_id = r.link_id
   AND l.state = 'waiting';
ALTER TABLE trace_payout_links ENABLE TRIGGER trace_payout_links_guard_update;

UPDATE trace_payout_links AS l
   SET unlink_requested_at = clock_timestamp(), version = l.version + 1,
       updated_at = clock_timestamp()
  FROM v147_revoked r
 WHERE r.state = 'active'
   AND l.tenant_id = r.tenant_id AND l.link_id = r.link_id
   AND l.state = 'active' AND l.unlink_requested_at IS NULL;

INSERT INTO trace_payout_audit (
    tenant_id, audit_id, rail_id, action, actor_ref_hash, account_id_hash,
    link_id_hash, prior_state, new_state, label
)
SELECT r.tenant_id,
       gen_random_uuid(),
       r.rail_id,
       CASE r.state WHEN 'waiting' THEN 'link_closed' ELSE 'unlink_requested' END,
       'sha256:' || encode(sha256(convert_to('payout-migration-actor-v1:V147', 'UTF8')), 'hex'),
       public.trace_payout_account_hash(r.account_id),
       public.trace_payout_link_hash(r.link_id),
       r.state,
       CASE r.state WHEN 'waiting' THEN 'closed' ELSE 'active' END,
       'near_ai_login_required'
  FROM v147_revoked r;

ALTER TABLE trace_near_account_anchors FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_links FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_lines FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_audit FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_payout_carries FORCE ROW LEVEL SECURITY;
