-- Phase 3 recovery state for sealed index commands and credit settlement.

ALTER TABLE pipeline_runs
    ADD COLUMN index_command_ref TEXT,
    ADD COLUMN index_command_hash TEXT
        CHECK (
            index_command_hash IS NULL
            OR index_command_hash ~ '^sha256:[0-9a-f]{64}$'
        ),
    ADD COLUMN index_write_state TEXT NOT NULL DEFAULT 'none'
        CHECK (index_write_state IN ('none', 'pending', 'complete', 'failed')),
    ADD COLUMN credit_event_id UUID,
    ADD COLUMN credit_write_state TEXT NOT NULL DEFAULT 'none'
        CHECK (credit_write_state IN ('none', 'pending', 'held', 'complete', 'failed')),
    ADD COLUMN settlement_batch_id UUID,
    ADD COLUMN payout_state TEXT NOT NULL DEFAULT 'none'
        CHECK (
            payout_state IN (
                'none',
                'disabled',
                'pending',
                'submitted',
                'confirmed',
                'failed'
            )
        );

ALTER TABLE pipeline_runs
    ADD CONSTRAINT pipeline_runs_index_command_shape CHECK (
        (
            index_membership = 'included'
            AND index_command_ref IS NOT NULL
            AND index_command_hash IS NOT NULL
        )
        OR (
            index_membership <> 'included'
            AND index_command_ref IS NULL
            AND index_command_hash IS NULL
        )
    );

ALTER TABLE trace_credit_ledger
    ADD COLUMN pipeline_run_id UUID,
    ADD COLUMN score_outcome_id UUID;

CREATE UNIQUE INDEX idx_trace_credit_ledger_pipeline_score
    ON trace_credit_ledger (tenant_id, pipeline_run_id, score_outcome_id)
    WHERE pipeline_run_id IS NOT NULL AND score_outcome_id IS NOT NULL;
