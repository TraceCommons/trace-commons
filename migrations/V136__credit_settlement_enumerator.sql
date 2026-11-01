-- Cross-tenant candidate discovery returns tenant IDs only. The settlement
-- worker switches to this role for one transaction, then performs all actual
-- settlement work through the existing tenant-scoped runtime role.
-- Deployment explicitly grants this role to the worker login, as with the
-- payout enumerator. No automatic runtime membership is added here.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='trace_credit_settlement_enumerator') THEN
        CREATE ROLE trace_credit_settlement_enumerator NOLOGIN NOBYPASSRLS;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname='trace_credit_settlement_enumerator'
               AND (rolsuper OR rolbypassrls OR rolcanlogin))
       OR EXISTS (SELECT 1 FROM pg_auth_members m JOIN pg_roles r ON r.oid=m.member
                  WHERE r.rolname='trace_credit_settlement_enumerator') THEN
        RAISE EXCEPTION 'V136: credit settlement enumerator must be unprivileged with no inherited roles';
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_credit_settlement_enumerator;
GRANT SELECT (tenant_id,credit_event_id,submission_id,event_type,points_delta,external_ref,occurred_at)
    ON trace_credit_ledger TO trace_credit_settlement_enumerator;
GRANT SELECT (tenant_id,submission_id,status)
    ON trace_submissions TO trace_credit_settlement_enumerator;
GRANT SELECT (tenant_id,status,source_credit_event_ids)
    ON trace_credit_settlement_batches TO trace_credit_settlement_enumerator;
DROP POLICY IF EXISTS trace_credit_settlement_enumerator_read ON trace_credit_ledger;
CREATE POLICY trace_credit_settlement_enumerator_read ON trace_credit_ledger
    FOR SELECT TO trace_credit_settlement_enumerator USING (current_user = 'trace_credit_settlement_enumerator');
DROP POLICY IF EXISTS trace_credit_settlement_enumerator_read ON trace_submissions;
CREATE POLICY trace_credit_settlement_enumerator_read ON trace_submissions
    FOR SELECT TO trace_credit_settlement_enumerator USING (current_user = 'trace_credit_settlement_enumerator');
DROP POLICY IF EXISTS trace_credit_settlement_enumerator_read ON trace_credit_settlement_batches;
CREATE POLICY trace_credit_settlement_enumerator_read ON trace_credit_settlement_batches
    FOR SELECT TO trace_credit_settlement_enumerator USING (current_user = 'trace_credit_settlement_enumerator');
