-- Bind finalized ranking credit settlements directly to the calibration
-- prediction/label evidence hash that authorized the credit-bearing model.

ALTER TABLE trace_credit_settlement_batches
    ADD COLUMN IF NOT EXISTS ranking_calibration_joined_evidence_hash TEXT
        CHECK (
            ranking_calibration_joined_evidence_hash IS NULL
            OR ranking_calibration_joined_evidence_hash LIKE 'sha256:%'
        );
