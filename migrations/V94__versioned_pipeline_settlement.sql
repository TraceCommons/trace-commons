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
    -- The adapter's evidence of an external effect: a SHA-256 reference over
    -- the external system's receipt, recorded when the leg completes. NULL
    -- for an effect with no external record.
    external_receipt_hash TEXT,
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
    -- A leg is `leased` under its run's lease token and expiry while its
    -- adapter call is in flight.
    lease_token UUID,
    lease_expires_at TIMESTAMPTZ,
    -- Set the first time the leg is leased for an adapter call, and never
    -- cleared: a failed run reconciles a dispatched external leg against
    -- its adapter rather than forfeiting it.
    dispatched_at TIMESTAMPTZ,
    -- Diagnostic only: the run's own attempt budget governs retries.
    attempt_count INTEGER NOT NULL DEFAULT 0 CHECK (attempt_count >= 0),
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
    -- `NO ACTION`, not `RESTRICT`, on both -- PostgreSQL never defers a
    -- `RESTRICT` action no matter what the `DEFERRABLE` clause says;
    -- `NO ACTION` is the same check, deferrable. Both deferred to end of
    -- transaction because `trace_credit_ledger` cascades straight from
    -- `trace_submissions`, and `trace_credit_settlement_batches` straight
    -- from `trace_tenants` -- both siblings of this row's own cascade
    -- (through `pipeline_runs`) up the same parent chain, so a submission
    -- or tenant delete can reach either branch first. By commit time this
    -- leg's own row is already gone whenever the whole submission (or
    -- tenant) is going away together; a ledger row or batch deleted on its
    -- own, with a leg still referencing it, is still refused. Both checks
    -- run at commit, so one transaction could delete and re-insert the
    -- same key; no code does that.
    FOREIGN KEY (tenant_id, credit_event_id, instrument_id)
        REFERENCES trace_credit_ledger (
            tenant_id, credit_event_id, instrument_id
        )
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED,
    FOREIGN KEY (tenant_id, settlement_batch_id, instrument_id)
        REFERENCES trace_credit_settlement_batches (
            tenant_id, settlement_batch_id, instrument_id
        )
        ON DELETE NO ACTION
        DEFERRABLE INITIALLY DEFERRED,
    -- NUMERIC(39,0) also holds values above u128::MAX, which AtomicUnits
    -- refuses to load; a row above it could be stored and never settled.
    CONSTRAINT pipeline_run_settlements_atomic_units_bound CHECK (
        atomic_units <= 340282366920938463463374607431768211455
    ),
    -- Trace Credit settles through the microcredit ledger's signed 64-bit
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
    -- Only a dispatched leg can be in flight or complete.
    CONSTRAINT pipeline_run_settlements_dispatch_shape CHECK (
        operation_state NOT IN ('leased', 'complete')
        OR dispatched_at IS NOT NULL
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
    ),
    -- Only a complete leg carries an external receipt hash.
    CONSTRAINT pipeline_run_settlements_external_receipt_shape CHECK (
        external_receipt_hash IS NULL
        OR (
            operation_state = 'complete'
            AND external_receipt_hash ~ '^sha256:[0-9a-f]{64}$'
        )
    )
);

-- One external receipt answers one leg of a tenant.
CREATE UNIQUE INDEX pipeline_run_settlements_external_receipt_unique
    ON pipeline_run_settlements (tenant_id, external_receipt_hash)
    WHERE external_receipt_hash IS NOT NULL;

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
       OR (
           OLD.external_receipt_hash IS NOT NULL
           AND NEW.external_receipt_hash IS DISTINCT FROM OLD.external_receipt_hash
       )
       OR (
           OLD.dispatched_at IS NOT NULL
           AND NEW.dispatched_at IS DISTINCT FROM OLD.dispatched_at
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

-- The ingest runtime's grants on what this migration adds, on V92's terms:
-- what the pipeline code reads and writes, and nothing broader. The columns
-- added to trace_credit_ledger, trace_credit_settlement_batches and
-- trace_near_credit_outbox need nothing here: those tables are older than
-- V62, and the pilot's runtime holds table-wide privileges on them, which
-- cover a column added later.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V94: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

-- pipeline_runs: the Score commit records its index command and neighbor
-- reference, and Settle persists its selection and advances the index write.
GRANT UPDATE (index_command_ref, index_command_hash, index_write_state,
              score_neighbor_ref, score_neighbor_hash, settle_selection,
              settle_selection_hash)
    ON pipeline_runs TO trace_ingest_runtime;

-- pipeline_run_settlements: the Score commit inserts one leg per award, and
-- Settle, reconciliation, and failing a run advance each leg. UPDATE is per
-- column: nothing updates a leg's identity (run, instrument, amount,
-- operation reference), its payout rail, or created_at, and V94's trigger
-- refuses those too. Nothing deletes a leg; it leaves with its run, through
-- the foreign key's cascade, which runs as the table owner.
GRANT SELECT, INSERT ON pipeline_run_settlements TO trace_ingest_runtime;
GRANT UPDATE (operation_state, result_ref_hash, external_receipt_hash,
              credit_event_id, settlement_batch_id, payout_state, lease_token,
              lease_expires_at, dispatched_at, attempt_count, last_error_label,
              updated_at)
    ON pipeline_run_settlements TO trace_ingest_runtime;
