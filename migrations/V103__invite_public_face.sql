-- V96: what an invite shows a contributor before they join (#1118, Z1).
--
-- The Join screen looks an invite up without redeeming it and names the issuer
-- and its pay range. Neither belongs in the operator-only labels V42 already
-- carries: `issued_by_label` and `note_label` stay private, so the public name
-- is its own column, and the pay range is structured and operator-set rather
-- than derived from any scorer.
--
-- `credit_range_min` / `credit_range_max` are whole credit points per accepted
-- trace (the integer points the credit ledger counts in). Both or neither: a
-- half-set range would render as a nonsense bound. All three columns are NULL
-- for every existing invite, which the lookup route reports as "valid, issuer
-- unnamed, no range", never as a default.
--
-- No policy or grant changes: the columns inherit the table's RLS (V42's
-- invite_lookup and trace_invite_registry_all policies) and its table-level
-- grants, and the only reader is the registry role.
--
-- Number V96 is provisional: main is at V91 and #1115 / #1107 hold V92-V95.
-- Renumber at landing if those merge first.

ALTER TABLE onboarding_invite_grants
    ADD COLUMN IF NOT EXISTS issuer_display_name TEXT,
    ADD COLUMN IF NOT EXISTS credit_range_min BIGINT,
    ADD COLUMN IF NOT EXISTS credit_range_max BIGINT;

ALTER TABLE onboarding_invite_grants
    DROP CONSTRAINT IF EXISTS onboarding_invite_grants_issuer_display_name_shape;
ALTER TABLE onboarding_invite_grants
    ADD CONSTRAINT onboarding_invite_grants_issuer_display_name_shape
        CHECK (
            issuer_display_name IS NULL
            OR (char_length(issuer_display_name) BETWEEN 1 AND 64
                AND issuer_display_name !~ '[[:cntrl:]]'
                AND issuer_display_name = btrim(issuer_display_name))
        );

ALTER TABLE onboarding_invite_grants
    DROP CONSTRAINT IF EXISTS onboarding_invite_grants_credit_range_shape;
ALTER TABLE onboarding_invite_grants
    ADD CONSTRAINT onboarding_invite_grants_credit_range_shape
        CHECK (
            (credit_range_min IS NULL AND credit_range_max IS NULL)
            OR (credit_range_min IS NOT NULL
                AND credit_range_max IS NOT NULL
                AND credit_range_min >= 0
                AND credit_range_min <= credit_range_max)
        );
