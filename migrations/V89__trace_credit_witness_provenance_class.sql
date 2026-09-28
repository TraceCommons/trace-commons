-- Credit events record the witness provenance class of the credited trace
-- (#1059), as a label for later analysis.
--
-- A label, never a weight: in v1 provenance earns no account trust and is no
-- volume multiplier (#1061, earned-trust decision 5), so points_delta is
-- computed exactly as before and nothing reads this column to price a credit.
--
-- The value is derived from the V76 current-object read, so an attested class
-- is recorded only where a verified v2 certificate covers the trace's current
-- accepted artifact. NULL means "not recorded": every event written before
-- this migration, and any event whose lookup was unavailable. Strictly
-- additive; no existing row is rewritten.
--
-- The CHECK keeps the column to the four labels, so it can never carry free
-- text such as a receipt signer, a model name or a URL.
ALTER TABLE trace_credit_ledger
    ADD COLUMN IF NOT EXISTS witness_provenance_class TEXT
        CONSTRAINT trace_credit_ledger_witness_provenance_class_label
        CHECK (
            witness_provenance_class IS NULL
            OR witness_provenance_class IN (
                'provider_tee_final_call',
                'gateway_final_call',
                'unattested',
                'legacy_v1'
            )
        );
