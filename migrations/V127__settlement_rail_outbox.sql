-- V127: a rail-discriminated settlement outbox, its rounding carry, and the
-- NEAR AI organization a tenant elects to receive inference credit.
-- Design: docs/superpowers/specs/2026-10-14-near-ai-inference-credit-settlement-design.md
--
-- The legacy NEAR receipt outbox (V2) is untouched. A finalized settlement
-- batch pays out on exactly one path: the triggers at the end refuse a rail
-- row beside a NEAR receipt row for the same batch, and the reverse.

CREATE TABLE trace_near_ai_credit_recipients (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    recipient_id UUID NOT NULL,
    -- The NEAR AI organization id. The one external identifier the rail must
    -- send. Never logged, never in an audit row: org_ref_hash stands in.
    org_id UUID NOT NULL,
    org_ref_hash TEXT NOT NULL CHECK (org_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    role_at_bind TEXT NOT NULL CHECK (role_at_bind IN ('owner', 'admin')),
    bound_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ,
    -- Set by the reconciler when the organization's credit bucket was
    -- written by someone else; no delivery builds on it until cleared.
    frozen_label TEXT CHECK (frozen_label IS NULL OR frozen_label ~ '^[a-z0-9_]{1,64}$'),
    frozen_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, recipient_id),
    CONSTRAINT trace_near_ai_credit_recipients_frozen_shape CHECK (
        (frozen_label IS NULL) = (frozen_at IS NULL)
    )
);

-- One active recipient per tenant.
CREATE UNIQUE INDEX trace_near_ai_credit_recipients_one_active
    ON trace_near_ai_credit_recipients (tenant_id)
    WHERE revoked_at IS NULL;

-- One active tenant per organization. NEAR AI's ceiling is absolute and
-- per organization, so two tenants writing one organization would race each
-- other's read-add-write. The index is global, across RLS, on purpose.
CREATE UNIQUE INDEX trace_near_ai_credit_recipients_one_tenant_per_org
    ON trace_near_ai_credit_recipients (org_ref_hash)
    WHERE revoked_at IS NULL;

CREATE TABLE trace_settlement_rail_carry (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    -- The rate the carry is denominated under. A different rate starts the
    -- carry over at zero (less than one atomic unit is forgone).
    rate_ref_hash TEXT NOT NULL CHECK (rate_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    carry_units NUMERIC(39, 0) NOT NULL DEFAULT 0 CHECK (
        carry_units >= 0 AND carry_units <= 340282366920938463463374607431768211455
    ),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, rail_id)
);

CREATE TABLE trace_settlement_rail_outbox (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    outbox_id UUID NOT NULL,
    settlement_batch_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    instrument_id TEXT NOT NULL CHECK (instrument_id ~ '^[a-z0-9_.-]{1,64}$'),
    instrument_decimals SMALLINT NOT NULL CHECK (instrument_decimals BETWEEN 0 AND 38),
    source_micros BIGINT NOT NULL CHECK (source_micros >= 0),
    atomic_units NUMERIC(39, 0) NOT NULL CHECK (
        atomic_units >= 0 AND atomic_units <= 340282366920938463463374607431768211455
    ),
    rate_text TEXT NOT NULL CHECK (rate_text ~ '^[0-9]{1,30}/[0-9]{1,30}$'),
    rate_ref_hash TEXT NOT NULL CHECK (rate_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    -- Written into the external system's audit field; built from the rail,
    -- instrument and outbox id, never from the tenant id.
    delivery_hash TEXT NOT NULL CHECK (delivery_hash ~ '^sha256:[0-9a-f]{64}$'),
    recipient_id UUID NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (
        state IN (
            'pending',
            'leased',
            'retry',
            'delivered',
            'reconciled',
            'frozen',
            'cancelled',
            'carried'
        )
    ),
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    prior_units NUMERIC(39, 0) CHECK (prior_units IS NULL OR prior_units >= 0),
    written_units NUMERIC(39, 0) CHECK (written_units IS NULL OR written_units >= 0),
    external_receipt_hash TEXT CHECK (
        external_receipt_hash IS NULL OR external_receipt_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    last_error_label TEXT CHECK (
        last_error_label IS NULL OR last_error_label ~ '^[a-z0-9_]{1,64}$'
    ),
    delivered_at TIMESTAMPTZ,
    reconciled_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, outbox_id),
    UNIQUE (tenant_id, settlement_batch_id, rail_id),
    UNIQUE (tenant_id, delivery_hash),
    FOREIGN KEY (tenant_id, settlement_batch_id)
        REFERENCES trace_credit_settlement_batches (tenant_id, settlement_batch_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, recipient_id)
        REFERENCES trace_near_ai_credit_recipients (tenant_id, recipient_id),
    -- A zero conversion is consumed into the carry and never sent.
    CONSTRAINT trace_settlement_rail_outbox_carried_shape CHECK (
        (state = 'carried') = (atomic_units = 0)
    ),
    CONSTRAINT trace_settlement_rail_outbox_lease_shape CHECK (
        (state = 'leased' AND lease_token IS NOT NULL AND lease_expires_at IS NOT NULL)
        OR (state <> 'leased' AND lease_token IS NULL AND lease_expires_at IS NULL)
    ),
    -- An effect that exists carries its evidence.
    CONSTRAINT trace_settlement_rail_outbox_delivered_shape CHECK (
        state NOT IN ('delivered', 'reconciled')
        OR (
            external_receipt_hash IS NOT NULL
            AND prior_units IS NOT NULL
            AND written_units IS NOT NULL
            AND delivered_at IS NOT NULL
        )
    ),
    CONSTRAINT trace_settlement_rail_outbox_receipt_shape CHECK (
        (external_receipt_hash IS NULL) = (written_units IS NULL)
        AND (written_units IS NULL) = (prior_units IS NULL)
    )
);

CREATE INDEX trace_settlement_rail_outbox_due
    ON trace_settlement_rail_outbox (tenant_id, rail_id, state, next_attempt_at);

-- Identity is immutable, as V94 does for pipeline legs. Evidence, once
-- recorded, is too: a later attempt may not overwrite a receipt.
CREATE FUNCTION reject_settlement_rail_outbox_identity_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.outbox_id IS DISTINCT FROM OLD.outbox_id
       OR NEW.settlement_batch_id IS DISTINCT FROM OLD.settlement_batch_id
       OR NEW.rail_id IS DISTINCT FROM OLD.rail_id
       OR NEW.instrument_id IS DISTINCT FROM OLD.instrument_id
       OR NEW.instrument_decimals IS DISTINCT FROM OLD.instrument_decimals
       OR NEW.source_micros IS DISTINCT FROM OLD.source_micros
       OR NEW.atomic_units IS DISTINCT FROM OLD.atomic_units
       OR NEW.rate_text IS DISTINCT FROM OLD.rate_text
       OR NEW.rate_ref_hash IS DISTINCT FROM OLD.rate_ref_hash
       OR NEW.delivery_hash IS DISTINCT FROM OLD.delivery_hash
       OR NEW.recipient_id IS DISTINCT FROM OLD.recipient_id
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
       OR (
           OLD.external_receipt_hash IS NOT NULL
           AND (
               NEW.external_receipt_hash IS DISTINCT FROM OLD.external_receipt_hash
               OR NEW.prior_units IS DISTINCT FROM OLD.prior_units
               OR NEW.written_units IS DISTINCT FROM OLD.written_units
           )
       )
       OR (OLD.state IN ('reconciled', 'carried') AND NEW.state IS DISTINCT FROM OLD.state)
       OR (OLD.state = 'cancelled' AND NEW.state IS DISTINCT FROM OLD.state)
       -- A row whose effect exists can never be cancelled.
       OR (NEW.state = 'cancelled' AND OLD.external_receipt_hash IS NOT NULL)
    THEN
        RAISE EXCEPTION 'settlement rail outbox identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trace_settlement_rail_outbox_reject_identity_update
    BEFORE UPDATE ON trace_settlement_rail_outbox
    FOR EACH ROW EXECUTE FUNCTION reject_settlement_rail_outbox_identity_mutation();

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

CREATE TRIGGER trace_settlement_rail_outbox_one_payout_path
    BEFORE INSERT ON trace_settlement_rail_outbox
    FOR EACH ROW EXECUTE FUNCTION reject_settlement_rail_beside_near_outbox();

CREATE FUNCTION reject_near_outbox_beside_settlement_rail()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.status <> 'disabled' AND EXISTS (
        SELECT 1 FROM trace_settlement_rail_outbox
         WHERE tenant_id = NEW.tenant_id
           AND settlement_batch_id = NEW.settlement_batch_id
           AND state <> 'cancelled'
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

ALTER TABLE trace_settlement_rail_carry ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_settlement_rail_carry FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_settlement_rail_carry;
CREATE POLICY trace_corpus_tenant_isolation ON trace_settlement_rail_carry
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_settlement_rail_outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_settlement_rail_outbox FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_settlement_rail_outbox;
CREATE POLICY trace_corpus_tenant_isolation ON trace_settlement_rail_outbox
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V127: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- Least privilege, on V92's terms. Nothing deletes: a recipient is revoked,
-- an outbox row is cancelled or kept, a carry is overwritten.
GRANT SELECT, INSERT ON trace_near_ai_credit_recipients TO trace_ingest_runtime;
GRANT UPDATE (revoked_at, frozen_label, frozen_at)
    ON trace_near_ai_credit_recipients TO trace_ingest_runtime;

GRANT SELECT, INSERT ON trace_settlement_rail_carry TO trace_ingest_runtime;
GRANT UPDATE (rate_ref_hash, carry_units, updated_at)
    ON trace_settlement_rail_carry TO trace_ingest_runtime;

GRANT SELECT, INSERT ON trace_settlement_rail_outbox TO trace_ingest_runtime;
GRANT UPDATE (state, attempt_count, next_attempt_at, lease_token, lease_expires_at,
              prior_units, written_units, external_receipt_hash, last_error_label,
              delivered_at, reconciled_at, updated_at)
    ON trace_settlement_rail_outbox TO trace_ingest_runtime;
