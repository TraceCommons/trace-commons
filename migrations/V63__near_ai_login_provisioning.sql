-- A NEAR AI login as a second source of the admission anchor (#836).
--
-- Invite-free admission counted its per-account limits against a wallet
-- anchor, so a contributor needed a NEAR wallet as well as the NEAR AI account
-- their receipts already come from. This adds the login as a second source.
-- The wallet path is untouched.
--
-- The two sources must never be confusable. Three things keep them apart, and
-- all three are needed:
--   * a different blind-index domain, so the anchor values live in disjoint
--     keyspaces even under the same pepper;
--   * a different tenant prefix (`nearai-` beside `near-`);
--   * `identity_source` on the anchor row and `near_ai` on the device row,
--     below, so the distinction is a fact the database holds rather than one
--     the code remembers.

-- Widen before narrowing, never the reverse: the new constraint is added while
-- the old one still applies, so the table is strictly more constrained during
-- the window and never unconstrained. The reverse order would leave a moment
-- in which any origin string was accepted.
-- `_v2`, because V58's inline CHECK is already called
-- `device_keys_onboarding_origin_check`: Postgres generates
-- `<table>_<column>_check` for an unnamed column constraint, so the obvious
-- name for the replacement collides with the constraint being replaced. Found
-- by running the migration, not by reading it.
ALTER TABLE device_keys ADD CONSTRAINT device_keys_onboarding_origin_check_v2
    CHECK (onboarding_origin IN ('invite', 'near', 'near_ai'));

-- V58 wrote the original inline on `ADD COLUMN`, so its name is generated.
-- Find it by definition rather than by that generated form -- the same
-- approach V61 used, and for the same reason: a guessed name silently matches
-- nothing and the migration reports success having dropped nothing (#723).
DO $$
DECLARE
    constraint_name text;
BEGIN
    SELECT conname INTO constraint_name
    FROM pg_constraint
    WHERE conrelid = 'device_keys'::regclass
      AND contype = 'c'
      AND conname <> 'device_keys_onboarding_origin_check_v2'
      AND pg_get_constraintdef(oid) LIKE '%onboarding_origin%'
      AND pg_get_constraintdef(oid) NOT LIKE '%invite_subject_hash%';
    IF constraint_name IS NULL THEN
        RAISE EXCEPTION 'V63 found no V58 origin CHECK to widen; refusing to leave the column governed by an unknown constraint set';
    END IF;
    EXECUTE format('ALTER TABLE device_keys DROP CONSTRAINT %I', constraint_name);
END $$;

-- The origin/invite pairing. `near_ai` joins `near` in the NULL arm: a
-- login-provisioned device redeems no invite, so a row carrying one is a
-- confusion between the two onboarding paths and the database refuses it
-- rather than trusting the writer.
ALTER TABLE device_keys ADD CONSTRAINT device_keys_invite_origin_binding_check_v2
    CHECK ((onboarding_origin = 'invite' AND invite_subject_hash IS NOT NULL)
        OR (onboarding_origin IN ('near', 'near_ai') AND invite_subject_hash IS NULL));
ALTER TABLE device_keys DROP CONSTRAINT device_keys_invite_origin_binding_check;

-- Which identity system minted this anchor.
--
-- `DEFAULT 'wallet'` is correct for every existing row: before this migration
-- the only writer was wallet provisioning. New login rows set it explicitly.
ALTER TABLE trace_near_account_anchors
    ADD COLUMN identity_source TEXT NOT NULL DEFAULT 'wallet'
        CHECK (identity_source IN ('wallet', 'near_ai_login')),
    -- How the contributor authenticated to NEAR AI. Recorded now because it
    -- cannot be backfilled: deriving it later would need every contributor to
    -- provision again. A wallet-backed NEAR AI account and a GitHub-backed one
    -- may later warrant different trust, and this is the only moment the answer
    -- is in hand.
    --
    -- Constrained by shape rather than by an enumerated list, deliberately:
    -- this repo has not observed NEAR AI's full vocabulary, and a guessed
    -- enumeration would refuse a legitimate provider the day they add one.
    -- The application maps anything outside this shape to 'unknown' before it
    -- arrives, so the column never carries free text from a third party.
    ADD COLUMN auth_provider TEXT
        CHECK (auth_provider IS NULL OR auth_provider ~ '^[a-z0-9_]{1,32}$'),
    -- The pairing, so neither column can drift from the other: a wallet anchor
    -- has no NEAR AI provider, and a login anchor always records one even if
    -- the answer is 'unknown'.
    ADD CONSTRAINT trace_near_account_anchors_identity_source_binding_check
        CHECK ((identity_source = 'wallet' AND auth_provider IS NULL)
            OR (identity_source = 'near_ai_login' AND auth_provider IS NOT NULL));
