-- The recipient ref of a payout link and of the request that made it
-- (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
-- section 5, "The routes").
--
-- The status route reports each link's recipient_ref so the app can check
-- the server holds the org it chose without the server echoing the org. For
-- near_ai it is sha256("nearai-payout-org-ref-v1:" || idempotency_key || ":"
-- || org_id): computed at the start of a link from the request, carried on
-- the request row, and copied onto the link when the request is confirmed.
-- The idempotency key is not kept, so the ref is computed once, here, and
-- never by opening the sealed recipient on a read.
--
-- A hash of a fresh client UUID and the org: it names nothing on its own,
-- like the other sha256: columns on these tables. Nullable: links written
-- before this column (none in production, every rail defaults to disabled)
-- and links a rail writes without a ref have none, and status reports null.
--
-- NUMBERING. V133 is the reversal hold decisions; this is the next number
-- free on main and in every open PR.
--
-- RUNTIME GRANTS. trace_ingest_runtime's table-level SELECT and INSERT on
-- both tables (V132) cover the new column; it is never updated, so no
-- column UPDATE grant is added. The enumeration role's column grant on
-- trace_payout_links (tenant_id, rail_id, state) does not include it.

ALTER TABLE trace_payout_links
    ADD COLUMN recipient_ref TEXT CHECK (recipient_ref ~ '^sha256:[0-9a-f]{64}$');

ALTER TABLE trace_payout_link_requests
    ADD COLUMN recipient_ref TEXT CHECK (recipient_ref ~ '^sha256:[0-9a-f]{64}$');

ALTER TABLE trace_payout_link_requests
    ADD CONSTRAINT trace_payout_link_requests_recipient_ref_shape CHECK (
        action = 'link' OR recipient_ref IS NULL
    );
