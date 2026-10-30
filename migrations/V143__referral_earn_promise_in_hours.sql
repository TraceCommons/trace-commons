-- The referral earning promise in absolute time
-- (docs/superpowers/specs/2026-10-29-referral-rewards-design.md).
--
-- V142 checked earn_until >= referee_account_created_at + interval '92 days'.
-- Adding a days interval to a timestamptz counts calendar days in the
-- session's TimeZone, so across a daylight-saving change the bound moves by
-- an hour: in a zone that falls back it is an hour past 92 x 24 hours, and
-- the store's own earn_until (creation + 92 x 24 hours, computed in Rust) is
-- refused; in one that springs forward it is an hour short, and an
-- earn_until under the promise is accepted. Under UTC, which the service's
-- databases use, the two agree exactly.
--
-- The promise is restated as 2208 hours (92 x 24), which means the same
-- instant in every zone. Under UTC it is the same bound as before, so every
-- row V142 accepted on a UTC database still passes, and so does every row
-- the store wrote in any zone (it writes creation + 2208 hours or later).
-- The new constraint is
-- added NOT VALID and then validated, so the table is scanned without
-- blocking writes for the scan.

ALTER TABLE trace_referral_attributions
    DROP CONSTRAINT trace_referral_attributions_earn_promise;
ALTER TABLE trace_referral_attributions
    ADD CONSTRAINT trace_referral_attributions_earn_promise CHECK (
        earn_until >= referee_account_created_at + interval '2208 hours'
    ) NOT VALID;
ALTER TABLE trace_referral_attributions
    VALIDATE CONSTRAINT trace_referral_attributions_earn_promise;
