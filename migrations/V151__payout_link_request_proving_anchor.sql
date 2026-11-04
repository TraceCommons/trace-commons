-- V151: a payout link request records the login anchor that proved it
-- (#1882).
--
-- The start of a link checks the NEAR AI login against the account's
-- recorded `near_ai_login` anchors, and the confirm re-checked only that the
-- account still held some live login anchor. An account whose proving login
-- was retired between the two (an F10 login change, a V82 merge) while it
-- kept or gained another could still land the link the old login proved.
-- The request's `subject_index` is a payout-domain keyed label and cannot be
-- compared with `anchor_hash`, so the request now stores the proving
-- anchor's `anchor_hash` itself, and the confirm requires that exact anchor
-- to be live.
--
-- WHAT IS STORED. The `anchor_hash` value of the matched
-- `trace_near_account_anchors` row: the same keyed label already stored
-- there, never a subject or an account name. It lives only as long as the
-- request (five minutes).
--
-- SHAPE. An unlink has none. A link must have one: the constraint is added
-- NOT VALID, so a link request a pre-V151 service wrote just before the
-- deploy (at most five minutes old) is left in place rather than failing
-- the migration, and every row written from now on is checked. The confirm
-- refuses such a request (no proving anchor recorded), so it can only
-- expire or be refused; the person starts again.
--
-- NUMBERING. V150 is F10's login change; this is the next number free on
-- main and in every open PR.
--
-- RUNTIME GRANTS. trace_ingest_runtime's table-level SELECT, INSERT and
-- DELETE on this table (V132) cover the new column; it is never updated.
-- Row security on the table is unchanged (forced since V132).

SET LOCAL lock_timeout = '10s';

ALTER TABLE trace_payout_link_requests
    ADD COLUMN IF NOT EXISTS proving_anchor_hash TEXT
        CHECK (proving_anchor_hash ~ '^sha256:[0-9a-f]{64}$');

ALTER TABLE trace_payout_link_requests
    DROP CONSTRAINT IF EXISTS trace_payout_link_requests_proving_anchor_shape;
ALTER TABLE trace_payout_link_requests
    ADD CONSTRAINT trace_payout_link_requests_proving_anchor_shape CHECK (
        (action = 'link' AND proving_anchor_hash IS NOT NULL)
        OR (action = 'unlink' AND proving_anchor_hash IS NULL)
    ) NOT VALID;
