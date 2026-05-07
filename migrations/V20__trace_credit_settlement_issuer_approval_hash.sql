-- Persist hash-only central issuer approval evidence for finalized credit batches.

ALTER TABLE trace_credit_settlement_batches
    ADD COLUMN IF NOT EXISTS issuer_approval_evidence_hash TEXT
        CHECK (
            issuer_approval_evidence_hash IS NULL
            OR issuer_approval_evidence_hash LIKE 'sha256:%'
        );
