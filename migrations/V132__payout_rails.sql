-- Parallel payout rails: storage for paying settled Trace Credit out on
-- external rails, NEAR AI inference credit first
-- (docs/superpowers/specs/2026-10-21-nearai-credit-payout-design.md,
-- sections 4 "State and storage" and 5 "Storage").
--
-- One set of tables serves every rail; every row carries `rail_id`. Nothing
-- here reads or writes trace_credit_ledger, the settlement batch rows, or the
-- NEP-141 outbox: a rail reads finalized batches and never consumes the award.
--
-- NUMBERING. V129 is reserved by #1415 (closed) and may already be applied
-- to someone's test database, where the runner would silently skip a reuse.
-- V131 is taken by open PR #1536. This is the next number free on main and
-- in every open PR.
--
-- AMOUNTS. Every amount is a rail's atomic units, u128 in Rust, stored as
-- NUMERIC(39,0) and bounded to u128::MAX, as V94 bounds pipeline_run_
-- settlements.atomic_units: NUMERIC(39,0) holds values u128 cannot, and a row
-- above it could be stored and never loaded. Accrual amounts are signed (a
-- reversal is negative), bounded in magnitude the same way.
--
-- TENANT TABLES. links, link_requests, accruals, lines, holds and audit carry
-- tenant_id, force RLS, and use the central trace_corpus_tenant_isolation
-- policy, so they are registered in TRACE_COMMONS_RLS_TABLES.
--
-- DEPLOYMENT-WIDE TABLES. trace_payout_rate_epochs and trace_payout_control
-- are per rail, not per tenant. They force RLS with a read-all SELECT policy
-- and a write policy for trace_payout_config_writer only. Having no tenant
-- column, they cannot carry the central predicate and are not in
-- TRACE_COMMONS_RLS_TABLES (the same reason as trace_leaderboard_snapshots).
--   * rate epochs are append-only, and observed_at is always the database
--     clock: the writer holds INSERT on every other column, not that one, so
--     no caller can backdate an epoch.
--   * control is an append-only log, one row per halt or resume; the latest
--     row per rail is its state. A halt is not tenant data, so this log is
--     its audit trail rather than trace_payout_audit.
--
-- ROLES. Two NOLOGIN NOBYPASSRLS roles, assumed with SET LOCAL ROLE for one
-- transaction, never inherited:
--   * trace_payout_enumerator: SELECT (tenant_id, rail_id, state) on links and
--     a permissive SELECT policy for itself, as V61/V101 do, so a rail's pass
--     can list the tenants with a waiting, active or closing link. It reads nothing else.
--   * trace_payout_config_writer: appends rate epochs and control rows.
-- Neither is granted to trace_ingest_runtime here: that group's memberships
-- are pinned (tests/support/pilot_runtime_login.rs), and inheriting either
-- would let every runtime login write rates. The operator grants both to the
-- ingest login with INHERIT FALSE (PostgreSQL 16):
--   GRANT trace_payout_enumerator, trace_payout_config_writer
--       TO <ingest login> WITH INHERIT FALSE, SET TRUE;
--
-- RUNTIME GRANTS, on V105's terms: what the store reads and writes, nothing
-- broader. trace_payout_audit is append-only (SELECT, INSERT). Accruals are
-- never re-priced: the runtime may only set an unowned accrual's owner.
--
-- FOREIGN KEYS to trace_accounts are NO ACTION, deliberately not CASCADE,
-- and NOT deferrable: an account with payout rows refuses deletion at the
-- DELETE itself, so the unbound-account reaper (V101), which deletes each
-- candidate inside its own subtransaction, rolls that one back and counts it
-- skipped. A deferred check would fire only at the reaper's COMMIT and abort
-- the whole batch on every tick. The reaper's pinned cascade set is
-- unchanged. tenant_id cascades from trace_tenants, the V2 pattern; a
-- tenant delete removes the account and its payout rows in one statement,
-- before the end-of-statement check. The settlement batch reference is
-- deferred to commit as V94's is: batches are never deleted on their own,
-- only by a tenant cascade, which can reach parent and child in either order.
--
-- Idempotent where a role is concerned, and applies under a non-superuser
-- CREATEROLE migrator (migration_atomicity_pg).

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V132: trace_ingest_runtime is missing; V90 creates it';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_payout_enumerator') THEN
        CREATE ROLE trace_payout_enumerator NOLOGIN NOBYPASSRLS;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_payout_config_writer') THEN
        CREATE ROLE trace_payout_config_writer NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
-- A pre-existing role is used as found, but never one that defeats forced
-- RLS. Naming SUPERUSER or BYPASSRLS in ALTER ROLE is superuser-only, so
-- refuse rather than repair.
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles
                WHERE rolname IN ('trace_payout_enumerator', 'trace_payout_config_writer')
                  AND (rolsuper OR rolbypassrls OR rolcanlogin)) THEN
        RAISE EXCEPTION 'V132: a payout role is privileged or can log in';
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_payout_enumerator, trace_payout_config_writer;

-- Links: one generation of one account's recipient on one rail. The raw
-- recipient is needed to send, so it is sealed (as sealed_account_name is),
-- with a keyed blind index and key and pepper refs for rotation; no key is in
-- the database. recipient_unavailable_at is a mark on a link, not a state, so
-- an unavailable active link still holds the account's one active slot.
CREATE TABLE trace_payout_links (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    link_id UUID NOT NULL,
    recipient_kind TEXT NOT NULL CHECK (recipient_kind ~ '^[a-z0-9_.-]{1,64}$'),
    state TEXT NOT NULL CHECK (state IN ('waiting', 'active', 'closing', 'closed')),
    sealed_recipient JSONB NOT NULL CHECK (jsonb_typeof(sealed_recipient) = 'object'),
    recipient_index TEXT NOT NULL CHECK (recipient_index ~ '^sha256:[0-9a-f]{64}$'),
    recipient_key_ref TEXT NOT NULL CHECK (char_length(recipient_key_ref) BETWEEN 1 AND 128),
    index_pepper_ref TEXT NOT NULL CHECK (char_length(index_pepper_ref) BETWEEN 1 AND 128),
    subject_index TEXT NOT NULL CHECK (subject_index ~ '^sha256:[0-9a-f]{64}$'),
    program TEXT NOT NULL CHECK (
        octet_length(program) BETWEEN 1 AND 100 AND program ~ '^[A-Za-z0-9_.:-]+$'
    ),
    final_paid NUMERIC(39, 0) CHECK (
        final_paid >= 0 AND final_paid <= 340282366920938463463374607431768211455
    ),
    unlink_requested_at TIMESTAMPTZ,
    recipient_unavailable_at TIMESTAMPTZ,
    version BIGINT NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, link_id),
    -- The target of trace_payout_lines' foreign key, so a line names the
    -- same account and rail as its link.
    UNIQUE (tenant_id, link_id, account_id, rail_id),
    -- A closed link's program total is recorded when, and only when, it closes.
    CONSTRAINT trace_payout_links_final_paid_shape CHECK (
        (state = 'closed') = (final_paid IS NOT NULL)
    ),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE NO ACTION
);
-- At most one waiting link, and at most one link that is active OR
-- closing, per account and rail. Active and closing share one slot: a new
-- link's target is owed minus the closed links' final_paid, which leaves
-- out what a still-closing link was paid, so a link may go active only once
-- its predecessor is closed (the pass closes one and activates the other in
-- one transaction). No uniqueness on recipient_index: many accounts may
-- name one recipient (D7).
CREATE UNIQUE INDEX trace_payout_links_one_live
    ON trace_payout_links (tenant_id, account_id, rail_id)
    WHERE state IN ('active', 'closing');
CREATE UNIQUE INDEX trace_payout_links_one_waiting
    ON trace_payout_links (tenant_id, account_id, rail_id) WHERE state = 'waiting';
CREATE INDEX trace_payout_links_rail_state
    ON trace_payout_links (rail_id, state, tenant_id);
CREATE INDEX trace_payout_links_pepper_ref ON trace_payout_links (index_pepper_ref);

-- Pending link or unlink requests (spec section 5), awaiting a passkey
-- confirmation. Taken (deleted) when confirmed or discarded.
CREATE TABLE trace_payout_link_requests (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    idempotency_key UUID NOT NULL CHECK (
        idempotency_key <> '00000000-0000-0000-0000-000000000000'::uuid
    ),
    handle_hash TEXT NOT NULL CHECK (handle_hash ~ '^sha256:[0-9a-f]{64}$'),
    request_digest TEXT NOT NULL CHECK (request_digest ~ '^sha256:[0-9a-f]{64}$'),
    action TEXT NOT NULL CHECK (action IN ('link', 'unlink')),
    expected_current_version BIGINT CHECK (expected_current_version > 0),
    recipient_kind TEXT CHECK (recipient_kind ~ '^[a-z0-9_.-]{1,64}$'),
    sealed_recipient JSONB CHECK (jsonb_typeof(sealed_recipient) = 'object'),
    recipient_index TEXT CHECK (recipient_index ~ '^sha256:[0-9a-f]{64}$'),
    recipient_key_ref TEXT CHECK (char_length(recipient_key_ref) BETWEEN 1 AND 128),
    index_pepper_ref TEXT CHECK (char_length(index_pepper_ref) BETWEEN 1 AND 128),
    subject_index TEXT CHECK (subject_index ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    expires_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (tenant_id, account_id, rail_id, idempotency_key),
    UNIQUE (tenant_id, handle_hash),
    -- A link request carries a whole sealed recipient; an unlink none.
    CONSTRAINT trace_payout_link_requests_recipient_shape CHECK (
        (
            action = 'link'
            AND recipient_kind IS NOT NULL AND sealed_recipient IS NOT NULL
            AND recipient_index IS NOT NULL AND recipient_key_ref IS NOT NULL
            AND index_pepper_ref IS NOT NULL AND subject_index IS NOT NULL
        )
        OR (
            action = 'unlink'
            AND recipient_kind IS NULL AND sealed_recipient IS NULL
            AND recipient_index IS NULL AND recipient_key_ref IS NULL
            AND index_pepper_ref IS NULL AND subject_index IS NULL
        )
    ),
    CONSTRAINT trace_payout_link_requests_expiry_shape CHECK (expires_at > created_at),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE NO ACTION
);

-- Accruals: one row per rail per settled batch line (credit), or per
-- reversal event (reversal), priced once and never re-priced. account_id is
-- NULL for a line keyed to a bare principal until the principal's account
-- claims it; it is set once.
CREATE TABLE trace_payout_accruals (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    accrual_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    kind TEXT NOT NULL CHECK (kind IN ('credit', 'reversal')),
    settlement_batch_id UUID NOT NULL,
    credit_account_hash TEXT NOT NULL CHECK (credit_account_hash ~ '^sha256:[0-9a-f]{64}$'),
    reversal_event_id UUID,
    account_id UUID,
    award_units NUMERIC(39, 0) NOT NULL,
    rail_units NUMERIC(39, 0) NOT NULL,
    rate_version TEXT NOT NULL CHECK (rate_version ~ '^[A-Za-z0-9_.-]{1,64}$'),
    instrument_id TEXT NOT NULL CHECK (instrument_id ~ '^[a-z0-9_.-]{1,64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    claimed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, accrual_id),
    -- A credit is keyed by its batch line, a reversal by its event; a NULL
    -- in a key column would let a duplicate past the partial unique index.
    CONSTRAINT trace_payout_accruals_key_shape CHECK (
        (kind = 'credit' AND reversal_event_id IS NULL)
        OR (kind = 'reversal' AND reversal_event_id IS NOT NULL)
    ),
    -- A credit adds and a reversal takes back.
    CONSTRAINT trace_payout_accruals_sign_shape CHECK (
        (kind = 'credit' AND award_units > 0 AND rail_units >= 0)
        OR (kind = 'reversal' AND award_units < 0 AND rail_units <= 0)
    ),
    CONSTRAINT trace_payout_accruals_bound CHECK (
        abs(award_units) <= 340282366920938463463374607431768211455
        AND abs(rail_units) <= 340282366920938463463374607431768211455
    ),
    CONSTRAINT trace_payout_accruals_claim_shape CHECK (
        claimed_at IS NULL OR account_id IS NOT NULL
    ),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE NO ACTION,
    FOREIGN KEY (tenant_id, settlement_batch_id)
        REFERENCES trace_credit_settlement_batches(tenant_id, settlement_batch_id)
        ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED
);
CREATE UNIQUE INDEX trace_payout_accruals_one_credit
    ON trace_payout_accruals (tenant_id, rail_id, settlement_batch_id, credit_account_hash)
    WHERE kind = 'credit';
CREATE UNIQUE INDEX trace_payout_accruals_one_reversal
    ON trace_payout_accruals (tenant_id, rail_id, reversal_event_id)
    WHERE kind = 'reversal';
CREATE INDEX trace_payout_accruals_owed
    ON trace_payout_accruals (tenant_id, rail_id, account_id);
CREATE INDEX trace_payout_accruals_unclaimed
    ON trace_payout_accruals (tenant_id, credit_account_hash)
    WHERE account_id IS NULL;

-- Lines: one send to one link's program. Written pending, and committed,
-- before any network call. A link has at most one pending line, and one
-- external record answers at most one line per rail.
CREATE TABLE trace_payout_lines (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    line_id UUID NOT NULL,
    account_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    link_id UUID NOT NULL,
    target_total NUMERIC(39, 0) NOT NULL CHECK (
        target_total >= 0 AND target_total <= 340282366920938463463374607431768211455
    ),
    delta NUMERIC(39, 0) NOT NULL CHECK (
        delta >= 0 AND delta <= 340282366920938463463374607431768211455
    ),
    owed_through_accrual_count BIGINT NOT NULL CHECK (owed_through_accrual_count >= 0),
    state TEXT NOT NULL CHECK (state IN ('pending', 'confirmed', 'no_op', 'held', 'failed')),
    receipt_hash TEXT CHECK (receipt_hash ~ '^sha256:[0-9a-f]{64}$'),
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    last_error_label TEXT CHECK (last_error_label ~ '^[a-z0-9_.-]{1,64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    resolved_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, line_id),
    CONSTRAINT trace_payout_lines_receipt_shape CHECK (
        state <> 'confirmed' OR receipt_hash IS NOT NULL
    ),
    CONSTRAINT trace_payout_lines_resolved_shape CHECK (
        (state = 'pending') = (resolved_at IS NULL)
    ),
    FOREIGN KEY (tenant_id, link_id, account_id, rail_id)
        REFERENCES trace_payout_links(tenant_id, link_id, account_id, rail_id)
        ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED
);
CREATE UNIQUE INDEX trace_payout_lines_one_pending
    ON trace_payout_lines (tenant_id, link_id) WHERE state = 'pending';
CREATE UNIQUE INDEX trace_payout_lines_one_receipt
    ON trace_payout_lines (rail_id, receipt_hash) WHERE receipt_hash IS NOT NULL;
CREATE INDEX trace_payout_lines_account
    ON trace_payout_lines (tenant_id, account_id, rail_id);

-- Holds: per account and rail, with a label. A cleared row stays.
CREATE TABLE trace_payout_holds (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    hold_id UUID NOT NULL,
    account_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    label TEXT NOT NULL CHECK (label ~ '^[a-z0-9_.-]{1,64}$'),
    opened_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    cleared_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, hold_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE NO ACTION
);
CREATE UNIQUE INDEX trace_payout_holds_one_open
    ON trace_payout_holds (tenant_id, account_id, rail_id, label) WHERE cleared_at IS NULL;

-- Audit: append-only and hash-only. No account id, recipient, program,
-- amount or rate: an actor ref hash, an account id hash and a link id hash
-- (each a domain-separated SHA-256 of a random UUID), states and a label.
CREATE TABLE trace_payout_audit (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    audit_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    action TEXT NOT NULL CHECK (action IN (
        'link_requested', 'unlink_requested', 'link_created', 'link_confirmed',
        'link_activated', 'link_closing', 'link_closed', 'link_released',
        'link_recipient_unavailable', 'hold_set', 'hold_cleared', 'accrual_claimed'
    )),
    actor_ref_hash TEXT NOT NULL CHECK (actor_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    account_id_hash TEXT CHECK (account_id_hash ~ '^sha256:[0-9a-f]{64}$'),
    link_id_hash TEXT CHECK (link_id_hash ~ '^sha256:[0-9a-f]{64}$'),
    prior_state TEXT CHECK (prior_state IN ('waiting', 'active', 'closing', 'closed')),
    new_state TEXT CHECK (new_state IN ('waiting', 'active', 'closing', 'closed')),
    label TEXT CHECK (label ~ '^[a-z0-9_.-]{1,64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, audit_id)
);
CREATE INDEX trace_payout_audit_rail ON trace_payout_audit (tenant_id, rail_id, created_at);

-- Rate epochs: one immutable rational per (rail, version), observed by the
-- database clock.
CREATE TABLE trace_payout_rate_epochs (
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    version TEXT NOT NULL CHECK (version ~ '^[A-Za-z0-9_.-]{1,64}$'),
    numerator NUMERIC(39, 0) NOT NULL CHECK (
        numerator > 0 AND numerator <= 340282366920938463463374607431768211455
    ),
    denominator NUMERIC(39, 0) NOT NULL CHECK (
        denominator > 0 AND denominator <= 340282366920938463463374607431768211455
    ),
    pricing_label TEXT CHECK (pricing_label ~ '^[A-Za-z0-9_.-]{1,64}$'),
    observed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (rail_id, version)
);
-- Every accrual names an epoch its rail recorded, so each price traces to
-- one immutable rate. Epochs are never deleted, so NO ACTION never fires on
-- a delete; the check runs at the insert. Foreign-key checks bypass RLS.
ALTER TABLE trace_payout_accruals
    ADD CONSTRAINT trace_payout_accruals_rate_epoch
    FOREIGN KEY (rail_id, rate_version)
    REFERENCES trace_payout_rate_epochs (rail_id, version) ON DELETE NO ACTION;

-- Control: the kill switch, as an append-only log per rail. seq orders the
-- log: the latest row by seq is the rail's state, so a halt and a resume
-- written at the same microsecond still resolve in the order written.
CREATE TABLE trace_payout_control (
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    change_id UUID NOT NULL,
    seq BIGINT GENERATED ALWAYS AS IDENTITY,
    halted BOOLEAN NOT NULL,
    label TEXT CHECK (label ~ '^[a-z0-9_.-]{1,64}$'),
    actor_ref_hash TEXT NOT NULL CHECK (actor_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    changed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (rail_id, change_id)
);
CREATE INDEX trace_payout_control_latest ON trace_payout_control (rail_id, seq DESC);

-- Settled values are final. The column grants below let the runtime role
-- write these columns at all; these triggers keep it from writing a settled
-- value again, whatever a caller's WHERE clause says. Each raises
-- check_violation. They are SECURITY INVOKER, so the pending-line lookup
-- runs under the caller's tenant context and forced RLS.
CREATE FUNCTION trace_payout_links_guard_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.state = 'closed' THEN
        RAISE EXCEPTION 'trace_payout_links: a closed link is final'
            USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.state IS DISTINCT FROM OLD.state AND NOT (
           (OLD.state = 'waiting' AND NEW.state = 'active')
        OR (OLD.state = 'active' AND NEW.state = 'closing')
        OR (OLD.state = 'closing' AND NEW.state = 'closed')
    ) THEN
        RAISE EXCEPTION 'trace_payout_links: a link moves only waiting, active, closing, closed'
            USING ERRCODE = 'check_violation';
    END IF;
    -- A plpgsql statement takes a fresh snapshot, so a pending line that
    -- committed while this UPDATE waited on the link row is seen here (the
    -- store's line insert holds the link row FOR SHARE until it commits).
    -- The rail pass's lock serialises these already; this is the backstop.
    IF NEW.state = 'closed' AND EXISTS (
        SELECT 1 FROM trace_payout_lines
         WHERE tenant_id = OLD.tenant_id AND link_id = OLD.link_id AND state = 'pending'
    ) THEN
        RAISE EXCEPTION 'trace_payout_links: a link with a pending line does not close'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER trace_payout_links_guard_update
    BEFORE UPDATE ON trace_payout_links
    FOR EACH ROW EXECUTE FUNCTION trace_payout_links_guard_update();

CREATE FUNCTION trace_payout_lines_guard_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.state <> 'pending' THEN
        RAISE EXCEPTION 'trace_payout_lines: a resolved line is final'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER trace_payout_lines_guard_update
    BEFORE UPDATE ON trace_payout_lines
    FOR EACH ROW EXECUTE FUNCTION trace_payout_lines_guard_update();

CREATE FUNCTION trace_payout_accruals_guard_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.account_id IS NOT NULL THEN
        RAISE EXCEPTION 'trace_payout_accruals: an owned accrual keeps its owner'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER trace_payout_accruals_guard_update
    BEFORE UPDATE ON trace_payout_accruals
    FOR EACH ROW EXECUTE FUNCTION trace_payout_accruals_guard_update();

-- Row level security.
ALTER TABLE trace_payout_links ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_links FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_payout_links;
CREATE POLICY trace_corpus_tenant_isolation ON trace_payout_links
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_payout_link_requests ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_link_requests FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_payout_link_requests;
CREATE POLICY trace_corpus_tenant_isolation ON trace_payout_link_requests
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_payout_accruals ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_accruals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_payout_accruals;
CREATE POLICY trace_corpus_tenant_isolation ON trace_payout_accruals
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_payout_lines ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_lines FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_payout_lines;
CREATE POLICY trace_corpus_tenant_isolation ON trace_payout_lines
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_payout_holds ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_holds FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_payout_holds;
CREATE POLICY trace_corpus_tenant_isolation ON trace_payout_holds
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_payout_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_audit FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_payout_audit;
CREATE POLICY trace_corpus_tenant_isolation ON trace_payout_audit
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The enumerator: a permissive SELECT policy for itself alone. Permissive
-- policies OR together, so every other role keeps tenant isolation.
DROP POLICY IF EXISTS trace_payout_enumerator_read ON trace_payout_links;
CREATE POLICY trace_payout_enumerator_read ON trace_payout_links
    FOR SELECT TO trace_payout_enumerator USING (TRUE);

ALTER TABLE trace_payout_rate_epochs ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_rate_epochs FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_payout_rate_epochs_read ON trace_payout_rate_epochs;
CREATE POLICY trace_payout_rate_epochs_read ON trace_payout_rate_epochs
    FOR SELECT USING (TRUE);
DROP POLICY IF EXISTS trace_payout_rate_epochs_append ON trace_payout_rate_epochs;
CREATE POLICY trace_payout_rate_epochs_append ON trace_payout_rate_epochs
    FOR INSERT TO trace_payout_config_writer WITH CHECK (TRUE);

ALTER TABLE trace_payout_control ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_payout_control FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_payout_control_read ON trace_payout_control;
CREATE POLICY trace_payout_control_read ON trace_payout_control
    FOR SELECT USING (TRUE);
DROP POLICY IF EXISTS trace_payout_control_append ON trace_payout_control;
CREATE POLICY trace_payout_control_append ON trace_payout_control
    FOR INSERT TO trace_payout_config_writer WITH CHECK (TRUE);

-- Grants.
GRANT SELECT (tenant_id, rail_id, state) ON trace_payout_links TO trace_payout_enumerator;

GRANT SELECT ON trace_payout_rate_epochs, trace_payout_control TO trace_payout_config_writer;
GRANT INSERT (rail_id, version, numerator, denominator, pricing_label)
    ON trace_payout_rate_epochs TO trace_payout_config_writer;
GRANT INSERT (rail_id, change_id, halted, label, actor_ref_hash)
    ON trace_payout_control TO trace_payout_config_writer;

-- The runtime: links are inserted, and the pass moves their state, version
-- and close total, marks an unavailable recipient, and records an unlink
-- request. A link's account, rail, recipient and program never change.
GRANT SELECT, INSERT ON trace_payout_links TO trace_ingest_runtime;
GRANT UPDATE (state, final_paid, unlink_requested_at, recipient_unavailable_at,
              version, updated_at)
    ON trace_payout_links TO trace_ingest_runtime;
-- Requests are recorded, read, and taken (deleted) once.
GRANT SELECT, INSERT, DELETE ON trace_payout_link_requests TO trace_ingest_runtime;
-- Accruals are inserted once and never re-priced; only an unowned accrual's
-- owner is set, once.
GRANT SELECT, INSERT ON trace_payout_accruals TO trace_ingest_runtime;
GRANT UPDATE (account_id, claimed_at) ON trace_payout_accruals TO trace_ingest_runtime;
-- Lines are inserted pending and resolved; a line's target, link and account
-- never change.
GRANT SELECT, INSERT ON trace_payout_lines TO trace_ingest_runtime;
GRANT UPDATE (state, receipt_hash, delta, attempts, last_error_label, updated_at,
              resolved_at)
    ON trace_payout_lines TO trace_ingest_runtime;
GRANT SELECT, INSERT ON trace_payout_holds TO trace_ingest_runtime;
GRANT UPDATE (cleared_at) ON trace_payout_holds TO trace_ingest_runtime;
-- Audit: append-only.
GRANT SELECT, INSERT ON trace_payout_audit TO trace_ingest_runtime;
-- Rate epochs and control: read only; writes go through the writer role.
GRANT SELECT ON trace_payout_rate_epochs, trace_payout_control TO trace_ingest_runtime;
