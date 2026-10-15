-- V130: a human assessment per phase, so an operator can resolve a Score
-- hold as well as a Review hold. V128 and V129 are taken by open PRs.
--
-- A registered Score policy may ask for a trace to be held for operator
-- review instead of priced: the run parks in `awaiting_review` at Score
-- under the policy's label. `pipeline_review_assessments` held one
-- assessment per run (V105's UNIQUE (tenant_id, run_id)), which a run that
-- was assessed at Review has already spent. Each assessment now names the
-- phase it resolves, and a run holds at most one per phase:
--
-- - `phase` is `review` for every existing row and every Review assessment,
--   and `score` for the assessment of a Score hold;
-- - `hold_label` is the label the Score policy held the run under, recorded
--   with a Score assessment (the run's parking label is cleared when the
--   assessment releases it), and NULL for a Review assessment.
--
-- The table stays append-only: V105's triggers still refuse every UPDATE
-- and direct DELETE. Adding a column with a constant default rewrites no
-- row and fires no row trigger.

ALTER TABLE pipeline_review_assessments
    ADD COLUMN phase TEXT NOT NULL DEFAULT 'review'
        CHECK (phase IN ('review', 'score')),
    ADD COLUMN hold_label TEXT
        CHECK (hold_label IS NULL OR hold_label ~ '^[a-z0-9_]{1,64}$');

ALTER TABLE pipeline_review_assessments
    ADD CONSTRAINT pipeline_review_assessments_hold_label_matches_phase
        CHECK ((phase = 'score') = (hold_label IS NOT NULL));

ALTER TABLE pipeline_review_assessments
    DROP CONSTRAINT pipeline_review_assessments_tenant_id_run_id_key;
ALTER TABLE pipeline_review_assessments
    ADD CONSTRAINT pipeline_review_assessments_tenant_id_run_id_phase_key
        UNIQUE (tenant_id, run_id, phase);

-- Tenant isolation, as V105 set it: forced row-level security, the tenant
-- predicate through trace_current_tenant_id(). Re-stated so this migration
-- alone shows the table's isolation; nothing changes.
ALTER TABLE pipeline_review_assessments ENABLE ROW LEVEL SECURITY;
ALTER TABLE pipeline_review_assessments FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON pipeline_review_assessments;
CREATE POLICY trace_corpus_tenant_isolation ON pipeline_review_assessments
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The ingest runtime's grants, as V105 set them: SELECT and INSERT on the
-- whole table, which covers the two new columns, and no UPDATE or DELETE.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V130: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;
GRANT SELECT, INSERT ON pipeline_review_assessments TO trace_ingest_runtime;
