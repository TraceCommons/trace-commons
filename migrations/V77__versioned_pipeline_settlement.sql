-- Recovery state for sealed index commands and independent instrument
-- settlement operations.

ALTER TABLE pipeline_runs
    ADD COLUMN index_command_ref TEXT,
    ADD COLUMN index_command_hash TEXT CHECK (
        index_command_hash IS NULL OR index_command_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    ADD COLUMN index_write_state TEXT NOT NULL DEFAULT 'none' CHECK (
        index_write_state IN ('none', 'pending', 'complete', 'failed', 'cancelled')
    ),
    ADD COLUMN score_neighbor_ref TEXT,
    ADD COLUMN score_neighbor_hash TEXT CHECK (
        score_neighbor_hash IS NULL OR score_neighbor_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    -- The Settle policy result, persisted before any effect is dispatched.
    -- It is recovery data, not a phase outcome: it holds hashes and labels.
    ADD COLUMN settle_selection JSONB,
    ADD COLUMN settle_selection_hash TEXT CHECK (
        settle_selection_hash IS NULL OR settle_selection_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    ADD CONSTRAINT pipeline_runs_index_command_shape CHECK (
        (index_command_ref IS NULL) = (index_command_hash IS NULL)
        AND (index_membership <> 'included' OR index_command_hash IS NOT NULL)
    ),
    ADD CONSTRAINT pipeline_runs_score_neighbor_shape CHECK (
        (score_neighbor_ref IS NULL) = (score_neighbor_hash IS NULL)
    ),
    ADD CONSTRAINT pipeline_runs_settle_selection_shape CHECK (
        (settle_selection IS NULL) = (settle_selection_hash IS NULL)
    );

ALTER TABLE trace_credit_ledger
    ADD COLUMN pipeline_run_id UUID,
    ADD COLUMN score_outcome_id UUID,
    ADD COLUMN instrument_id TEXT CHECK (
        instrument_id IS NULL
        OR instrument_id ~ '^[a-z0-9_.-]{1,64}$'
    ),
    ADD CONSTRAINT trace_credit_ledger_pipeline_instrument_shape CHECK (
        (
            pipeline_run_id IS NULL
            AND score_outcome_id IS NULL
            AND instrument_id IS NULL
        )
        OR (
            pipeline_run_id IS NOT NULL
            AND score_outcome_id IS NOT NULL
            AND instrument_id IS NOT NULL
        )
    );

CREATE UNIQUE INDEX idx_trace_credit_ledger_pipeline_score
    ON trace_credit_ledger (
        tenant_id, pipeline_run_id, score_outcome_id, instrument_id
    )
    WHERE pipeline_run_id IS NOT NULL;

ALTER TABLE trace_credit_ledger
    ADD CONSTRAINT trace_credit_ledger_event_instrument_unique
        UNIQUE (tenant_id, credit_event_id, instrument_id);

ALTER TABLE trace_credit_settlement_batches
    ADD COLUMN instrument_id TEXT CHECK (
        instrument_id IS NULL
        OR instrument_id ~ '^[a-z0-9_.-]{1,64}$'
    ),
    ADD CONSTRAINT trace_credit_settlement_batch_instrument_unique
        UNIQUE (tenant_id, settlement_batch_id, instrument_id);

ALTER TABLE trace_near_credit_outbox
    ADD COLUMN instrument_id TEXT CHECK (
        instrument_id IS NULL
        OR instrument_id ~ '^[a-z0-9_.-]{1,64}$'
    ),
    ADD CONSTRAINT trace_near_credit_outbox_batch_instrument_fk
        FOREIGN KEY (tenant_id, settlement_batch_id, instrument_id)
        REFERENCES trace_credit_settlement_batches (
            tenant_id, settlement_batch_id, instrument_id
        )
        ON DELETE CASCADE;

CREATE FUNCTION reject_near_outbox_instrument_mismatch()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
DECLARE
    batch_instrument_id TEXT;
BEGIN
    SELECT instrument_id
      INTO batch_instrument_id
      FROM trace_credit_settlement_batches
     WHERE tenant_id = NEW.tenant_id
       AND settlement_batch_id = NEW.settlement_batch_id;

    IF NOT FOUND OR batch_instrument_id IS DISTINCT FROM NEW.instrument_id THEN
        RAISE EXCEPTION 'settlement outbox instrument does not match its batch';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trace_near_credit_outbox_validate_instrument
    BEFORE INSERT OR UPDATE OF tenant_id, settlement_batch_id, instrument_id
    ON trace_near_credit_outbox
    FOR EACH ROW EXECUTE FUNCTION reject_near_outbox_instrument_mismatch();

CREATE TABLE pipeline_run_settlements (
    tenant_id TEXT NOT NULL,
    run_id UUID NOT NULL,
    instrument_id TEXT NOT NULL CHECK (
        instrument_id ~ '^[a-z0-9_.-]{1,64}$'
    ),
    atomic_units NUMERIC(39, 0) NOT NULL CHECK (atomic_units > 0),
    operation_ref_hash TEXT NOT NULL CHECK (
        operation_ref_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    result_ref_hash TEXT CHECK (
        result_ref_hash IS NULL
        OR result_ref_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    operation_state TEXT NOT NULL DEFAULT 'pending' CHECK (
        operation_state IN (
            'pending',
            'leased',
            'retry',
            'held',
            'complete',
            'failed',
            'forfeited'
        )
    ),
    credit_event_id UUID,
    settlement_batch_id UUID,
    payout_rail TEXT NOT NULL CHECK (
        payout_rail ~ '^[a-z0-9_.-]{1,64}$'
    ),
    payout_state TEXT NOT NULL DEFAULT 'none' CHECK (
        payout_state IN (
            'none',
            'disabled',
            'pending',
            'submitted',
            'confirmed',
            'failed'
        )
    ),
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
    max_attempts INTEGER NOT NULL DEFAULT 5 CHECK (max_attempts > 0),
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_error_label TEXT CHECK (
        last_error_label IS NULL
        OR last_error_label ~ '^[a-z0-9_]{1,64}$'
    ),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (tenant_id, run_id, instrument_id),
    UNIQUE (tenant_id, operation_ref_hash),
    UNIQUE (tenant_id, result_ref_hash),
    FOREIGN KEY (tenant_id, run_id)
        REFERENCES pipeline_runs (tenant_id, run_id)
        ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, credit_event_id, instrument_id)
        REFERENCES trace_credit_ledger (
            tenant_id, credit_event_id, instrument_id
        )
        ON DELETE RESTRICT,
    FOREIGN KEY (tenant_id, settlement_batch_id, instrument_id)
        REFERENCES trace_credit_settlement_batches (
            tenant_id, settlement_batch_id, instrument_id
        )
        ON DELETE RESTRICT,
    CONSTRAINT pipeline_run_settlements_attempt_limit CHECK (
        attempt_count <= max_attempts
    ),
    -- Trace Credit settles through the microcredit ledger's signed 64-bit
    -- column; every other instrument is unbounded within the NUMERIC(39,0)
    -- column (decision: PR #971 amendments A2).
    CONSTRAINT pipeline_run_settlements_trace_credit_bound CHECK (
        instrument_id <> 'trace_credit' OR atomic_units <= 9223372036854775807
    ),
    CONSTRAINT pipeline_run_settlements_lease_shape CHECK (
        (
            operation_state = 'leased'
            AND lease_token IS NOT NULL
            AND lease_expires_at IS NOT NULL
        )
        OR (
            operation_state <> 'leased'
            AND lease_token IS NULL
            AND lease_expires_at IS NULL
        )
    ),
    CONSTRAINT pipeline_run_settlements_credit_shape CHECK (
        credit_event_id IS NULL OR instrument_id = 'trace_credit'
    ),
    CONSTRAINT pipeline_run_settlements_batch_shape CHECK (
        settlement_batch_id IS NULL OR instrument_id = 'trace_credit'
    ),
    CONSTRAINT pipeline_run_settlements_payout_shape CHECK (
        (payout_rail = 'none' AND payout_state IN ('none', 'disabled'))
        OR (payout_rail <> 'none' AND payout_state <> 'none')
    ),
    CONSTRAINT pipeline_run_settlements_result_shape CHECK (
        (operation_state = 'complete' AND result_ref_hash IS NOT NULL)
        OR (operation_state <> 'complete' AND result_ref_hash IS NULL)
    )
);

CREATE INDEX idx_pipeline_run_settlements_work
    ON pipeline_run_settlements (
        operation_state,
        next_attempt_at,
        lease_expires_at,
        created_at,
        run_id,
        instrument_id
    )
    WHERE operation_state IN ('pending', 'leased', 'retry');

CREATE FUNCTION reject_pipeline_run_settlement_identity_mutation()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.tenant_id IS DISTINCT FROM OLD.tenant_id
       OR NEW.run_id IS DISTINCT FROM OLD.run_id
       OR NEW.instrument_id IS DISTINCT FROM OLD.instrument_id
       OR NEW.atomic_units IS DISTINCT FROM OLD.atomic_units
       OR NEW.operation_ref_hash IS DISTINCT FROM OLD.operation_ref_hash
       OR (
           OLD.result_ref_hash IS NOT NULL
           AND NEW.result_ref_hash IS DISTINCT FROM OLD.result_ref_hash
       )
       OR NEW.payout_rail IS DISTINCT FROM OLD.payout_rail
       OR NEW.created_at IS DISTINCT FROM OLD.created_at
    THEN
        RAISE EXCEPTION 'pipeline settlement identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER pipeline_run_settlements_reject_identity_update
    BEFORE UPDATE ON pipeline_run_settlements
    FOR EACH ROW EXECUTE FUNCTION reject_pipeline_run_settlement_identity_mutation();

ALTER TABLE pipeline_run_settlements ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_run_settlements FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_run_settlements;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_run_settlements
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
