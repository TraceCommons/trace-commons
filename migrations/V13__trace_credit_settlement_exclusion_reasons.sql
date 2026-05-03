-- Preserve aggregate ranking-credit exclusion reasons for finalized settlement
-- batches without storing raw trace, contributor, or reviewer evidence.

ALTER TABLE trace_credit_settlement_batches
    ADD COLUMN IF NOT EXISTS ranking_credit_events_excluded_reason_counts_json JSONB
        NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(ranking_credit_events_excluded_reason_counts_json) = 'object');
