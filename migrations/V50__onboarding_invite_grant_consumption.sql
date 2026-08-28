-- V50: make an invite's use limit bind, whatever tenant a redeemer lands in.
--
-- V42 stores max_uses on the tenant-less grant row and says plainly that V29
-- "keeps counting redemptions per tenant". Under InviteTenantMode::Fixed that
-- is one counter for one tenant and the limit holds. Under Derived the tenant
-- is computed from the redeemer's own device key, so every redeemer gets a
-- fresh onboarding_invites row starting at zero and the limit never binds:
-- a max_uses=1 invite admits as many devices as care to present it.
--
-- The limit and the counter lived in different tables and, in derived mode,
-- never met. This puts the counter beside the limit.
--
-- V29 is left alone. Per-tenant accounting is still the right answer to "how
-- many redemptions under this tenant"; it is simply not the answer to "has
-- this invite been used up".

ALTER TABLE onboarding_invite_grants
    ADD COLUMN IF NOT EXISTS consumed_uses INTEGER NOT NULL DEFAULT 0
        CHECK (consumed_uses >= 0);

-- Backfill from the per-tenant counters, so an invite already redeemed under
-- several derived tenants does not wake up with a full fresh allowance. This
-- is the whole point of the migration: without it, every invite in the wild
-- gets max_uses more redemptions on the day this ships.
--
-- LEAST() because the sum can already exceed max_uses -- that over-redemption
-- is exactly the defect being closed, and the constraint below would reject
-- the true figure. Clamping records the invite as exhausted, which it is.
--
-- Both tables FORCE row level security, which applies to the table owner too,
-- so a plain UPDATE here would silently touch zero rows and the backfill would
-- look like it worked. Lift FORCE for the statement and put it straight back.
ALTER TABLE onboarding_invite_grants NO FORCE ROW LEVEL SECURITY;
ALTER TABLE onboarding_invites NO FORCE ROW LEVEL SECURITY;

UPDATE onboarding_invite_grants g
   SET consumed_uses = LEAST(
           g.max_uses,
           COALESCE(
               (SELECT SUM(i.consumed_uses)::INTEGER
                  FROM onboarding_invites i
                 WHERE i.invite_subject_hash = g.invite_subject_hash),
               0)
       );

ALTER TABLE onboarding_invites FORCE ROW LEVEL SECURITY;
ALTER TABLE onboarding_invite_grants FORCE ROW LEVEL SECURITY;

ALTER TABLE onboarding_invite_grants
    DROP CONSTRAINT IF EXISTS onboarding_invite_grants_consumed_within_max;
ALTER TABLE onboarding_invite_grants
    ADD CONSTRAINT onboarding_invite_grants_consumed_within_max
        CHECK (consumed_uses <= max_uses);

-- The redemption path runs in a tenant-scoped transaction under the runtime
-- role, not the registry role, so it needs its own way to consume. Scoped to
-- the same GUC the lookup policy uses: a caller can only consume the row for
-- a code it already presented, and still cannot enumerate live invites.
DROP POLICY IF EXISTS invite_consume ON onboarding_invite_grants;
CREATE POLICY invite_consume ON onboarding_invite_grants
    FOR UPDATE
    USING (invite_subject_hash = trace_current_invite_subject())
    WITH CHECK (invite_subject_hash = trace_current_invite_subject());
