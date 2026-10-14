-- V129: a rail-discriminated, cumulative settlement ledger, and the NEAR AI
-- organization a tenant elects to receive inference credit.
-- Design: docs/superpowers/specs/2026-10-14-near-ai-inference-credit-settlement-design.md
--
-- A rail publishes each recipient's CUMULATIVE total, and the external side
-- keeps the maximum it has seen (NEAR AI's staking-farm pattern). So the
-- ledger only ever grows, a batch only ever adds to it, and publishing is
-- idempotent. The legacy NEAR receipt outbox (V2) is untouched; a finalized
-- settlement batch pays out on exactly one path (triggers at the end).

CREATE TABLE trace_near_ai_credit_recipients (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    recipient_id UUID NOT NULL,
    -- The NEAR AI organization id: the one external identifier the rail must
    -- send. Never logged, never in an audit row; org_ref_hash stands in.
    org_id UUID NOT NULL,
    org_ref_hash TEXT NOT NULL CHECK (org_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    role_at_bind TEXT NOT NULL CHECK (role_at_bind IN ('owner', 'admin')),
    bound_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, recipient_id)
);

-- One active recipient per tenant.
CREATE UNIQUE INDEX trace_near_ai_credit_recipients_one_active
    ON trace_near_ai_credit_recipients (tenant_id)
    WHERE revoked_at IS NULL;

-- One tenant per organization, ever, and one row: NEAR AI's total for an
-- organization never decreases, so it must always equal exactly one ledger.
-- A second tenant, or a fresh row for the same tenant, would start a new
-- "cumulative" at zero that the organization's maximum already exceeds.
-- Rebinding reactivates the row instead. Global, across RLS, on purpose,
-- and not conditional on revoked_at.
CREATE UNIQUE INDEX trace_near_ai_credit_recipients_one_tenant_per_org
    ON trace_near_ai_credit_recipients (org_ref_hash);

-- One row per (batch, rail): what the batch added. Append-only.
CREATE TABLE trace_settlement_rail_credits (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    settlement_batch_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    recipient_id UUID NOT NULL,
    source_micros BIGINT NOT NULL CHECK (source_micros >= 0),
    atomic_units NUMERIC(39, 0) NOT NULL CHECK (
        atomic_units >= 0 AND atomic_units <= 340282366920938463463374607431768211455
    ),
    rate_text TEXT NOT NULL CHECK (rate_text ~ '^[0-9]{1,30}/[0-9]{1,30}$'),
    rate_ref_hash TEXT NOT NULL CHECK (rate_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, settlement_batch_id, rail_id),
    FOREIGN KEY (tenant_id, settlement_batch_id)
        REFERENCES trace_credit_settlement_batches (tenant_id, settlement_batch_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, recipient_id)
        REFERENCES trace_near_ai_credit_recipients (tenant_id, recipient_id)
);

-- One row per (tenant, rail, recipient): the cumulative total and how far
-- the external side has confirmed it.
CREATE TABLE trace_settlement_rail_ledgers (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    recipient_id UUID NOT NULL,
    instrument_id TEXT NOT NULL CHECK (instrument_id ~ '^[a-z0-9_.-]{1,64}$'),
    instrument_decimals SMALLINT NOT NULL CHECK (instrument_decimals BETWEEN 0 AND 38),
    cumulative_micros NUMERIC(39, 0) NOT NULL DEFAULT 0 CHECK (cumulative_micros >= 0),
    cumulative_units NUMERIC(39, 0) NOT NULL DEFAULT 0 CHECK (
        cumulative_units >= 0 AND cumulative_units <= 340282366920938463463374607431768211455
    ),
    -- What the external side last confirmed holding. Never above the total.
    confirmed_units NUMERIC(39, 0) NOT NULL DEFAULT 0 CHECK (confirmed_units >= 0),
    -- Rounding remainder, denominated under rate_ref_hash. A different rate
    -- starts it over at zero (less than one atomic unit is forgone).
    rate_ref_hash TEXT NOT NULL CHECK (rate_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    carry_units NUMERIC(39, 0) NOT NULL DEFAULT 0 CHECK (carry_units >= 0),
    publish_state TEXT NOT NULL DEFAULT 'idle' CHECK (
        publish_state IN ('idle', 'due', 'leased', 'published', 'frozen')
    ),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    last_error_label TEXT CHECK (
        last_error_label IS NULL OR last_error_label ~ '^[a-z0-9_]{1,64}$'
    ),
    confirmed_at TIMESTAMPTZ,
    verified_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, rail_id, recipient_id),
    FOREIGN KEY (tenant_id, recipient_id)
        REFERENCES trace_near_ai_credit_recipients (tenant_id, recipient_id),
    CONSTRAINT trace_settlement_rail_ledgers_confirmed_bound CHECK (
        confirmed_units <= cumulative_units
    ),
    CONSTRAINT trace_settlement_rail_ledgers_lease_shape CHECK (
        (publish_state = 'leased' AND lease_token IS NOT NULL AND lease_expires_at IS NOT NULL)
        OR (publish_state <> 'leased' AND lease_token IS NULL AND lease_expires_at IS NULL)
    )
);

CREATE INDEX trace_settlement_rail_ledgers_due
    ON trace_settlement_rail_ledgers (tenant_id, rail_id, publish_state, next_attempt_at);

-- Totals only grow, and identity never changes.
CREATE FUNCTION reject_settlement_rail_ledger_regression()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.rail_id IS DISTINCT FROM OLD.rail_id
       OR NEW.recipient_id IS DISTINCT FROM OLD.recipient_id
       OR NEW.instrument_id IS DISTINCT FROM OLD.instrument_id
       OR NEW.instrument_decimals IS DISTINCT FROM OLD.instrument_decimals
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
       OR NEW.cumulative_units < OLD.cumulative_units
       OR NEW.cumulative_micros < OLD.cumulative_micros
       OR NEW.confirmed_units < OLD.confirmed_units
    THEN
        RAISE EXCEPTION 'settlement rail ledger totals only grow';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trace_settlement_rail_ledgers_reject_regression
    BEFORE UPDATE ON trace_settlement_rail_ledgers
    FOR EACH ROW EXECUTE FUNCTION reject_settlement_rail_ledger_regression();

-- One payout path per batch.
CREATE FUNCTION reject_settlement_rail_beside_near_outbox()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM trace_near_credit_outbox
         WHERE tenant_id = NEW.tenant_id
           AND settlement_batch_id = NEW.settlement_batch_id
           AND status <> 'disabled'
    ) THEN
        RAISE EXCEPTION 'settlement batch already pays out through the NEAR receipt outbox';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trace_settlement_rail_credits_one_payout_path
    BEFORE INSERT ON trace_settlement_rail_credits
    FOR EACH ROW EXECUTE FUNCTION reject_settlement_rail_beside_near_outbox();

CREATE FUNCTION reject_near_outbox_beside_settlement_rail()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.status <> 'disabled' AND EXISTS (
        SELECT 1 FROM trace_settlement_rail_credits
         WHERE tenant_id = NEW.tenant_id
           AND settlement_batch_id = NEW.settlement_batch_id
    ) THEN
        RAISE EXCEPTION 'settlement batch already pays out through a settlement rail';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trace_near_credit_outbox_one_payout_path
    BEFORE INSERT ON trace_near_credit_outbox
    FOR EACH ROW EXECUTE FUNCTION reject_near_outbox_beside_settlement_rail();

ALTER TABLE trace_near_ai_credit_recipients ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_near_ai_credit_recipients FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_near_ai_credit_recipients;
CREATE POLICY trace_corpus_tenant_isolation ON trace_near_ai_credit_recipients
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_settlement_rail_credits ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_settlement_rail_credits FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_settlement_rail_credits;
CREATE POLICY trace_corpus_tenant_isolation ON trace_settlement_rail_credits
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_settlement_rail_ledgers ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_settlement_rail_ledgers FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_settlement_rail_ledgers;
CREATE POLICY trace_corpus_tenant_isolation ON trace_settlement_rail_ledgers
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V129: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- Least privilege, on V92's terms. Nothing deletes; credits are never
-- updated; a ledger's identity and totals are guarded by the trigger above.
GRANT SELECT, INSERT ON trace_near_ai_credit_recipients TO trace_ingest_runtime;
GRANT UPDATE (revoked_at) ON trace_near_ai_credit_recipients TO trace_ingest_runtime;

GRANT SELECT, INSERT ON trace_settlement_rail_credits TO trace_ingest_runtime;

GRANT SELECT, INSERT ON trace_settlement_rail_ledgers TO trace_ingest_runtime;
GRANT UPDATE (cumulative_micros, cumulative_units, confirmed_units, rate_ref_hash,
              carry_units, publish_state, attempt_count, next_attempt_at, lease_token,
              lease_expires_at, last_error_label, confirmed_at, verified_at, updated_at)
    ON trace_settlement_rail_ledgers TO trace_ingest_runtime;
