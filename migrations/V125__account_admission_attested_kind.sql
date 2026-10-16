-- V125: an account reservation records which authority admitted it.
--
-- Owner decision of 2026-10-11: an upload whose NEAR AI admission evidence
-- verifies (configured provider-TEE or gateway signer, accepted model,
-- minimum request bytes, bound to this account and artifact) is admitted
-- without debiting the bounded allowance. Such a reservation is written with
-- authority_kind 'attested' and last_charged FALSE, and creates no budget
-- row; every other reservation stays 'account'. The kind is fixed by the
-- first reservation of a submission and never rewritten, so a retry -- after
-- a release or an expired lease, with or without headers -- recovers on the
-- authority its row records.
--
-- No new grant: the admission runtime already holds table-wide INSERT and
-- UPDATE on this table (V77). Row-level security is unchanged.
--
-- Idempotent.

ALTER TABLE trace_account_admission_submissions
    DROP CONSTRAINT IF EXISTS trace_account_admission_submissions_authority_kind_check;
ALTER TABLE trace_account_admission_submissions
    ADD CONSTRAINT trace_account_admission_submissions_authority_kind_check
    CHECK (authority_kind IN ('account', 'attested'));
