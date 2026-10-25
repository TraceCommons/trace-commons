-- Prepare ONLY the dedicated upload-trust login in an already migrated database.
-- PostgreSQL 16+, run as the database administrator, never as the ingest app.
-- No password, evaluator, backfill, growth switch, or admission setting is changed.
-- Required psql variable: --set=expected_database=<explicit database name>.
\set ON_ERROR_STOP on
\if :{?expected_database}
\else
  \echo 'Refused: provide --set=expected_database=<explicit database name>'
  \quit 3
\endif
SELECT current_database() = :'expected_database' AS upload_trust_expected_database \gset
\if :upload_trust_expected_database
\else
  \echo 'Refused: connected database differs from expected_database'
  \quit 3
\endif

BEGIN;
SELECT pg_advisory_xact_lock(hashtextextended('tracecommons:tc_upload_trust_login:bootstrap:v1', 0));

DO $prepare$
DECLARE
    target_oid OID;
    group_name TEXT;
    relation_name TEXT;
    required_rls TEXT[] := ARRAY[
        'trace_accounts', 'trace_account_principals',
        'trace_near_provisioned_devices', 'device_keys',
        'trace_account_invite_grants', 'trace_account_trust',
        'trace_account_admission_budget', 'trace_submissions',
        'trace_gate_decisions', 'trace_credit_ledger', 'trace_audit_events',
        'trace_device_upload_principals', 'trace_account_trust_facts',
        'trace_account_trust_evaluations', 'trace_account_trust_frontiers',
        'trace_account_trust_dependency_locks'
    ];
BEGIN
    IF current_setting('server_version_num')::INTEGER < 160000 THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_requires_postgresql_16';
    END IF;
    IF current_setting('is_superuser') <> 'on' THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_requires_database_administrator';
    END IF;
    IF to_regclass('public._trace_commons_migrations') IS NULL THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_schema_missing';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM public._trace_commons_migrations WHERE version = 137)
       OR (SELECT max(version) FROM public._trace_commons_migrations) < 137 THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_requires_migration_137';
    END IF;

    FOREACH relation_name IN ARRAY required_rls LOOP
        IF NOT EXISTS (
            SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = 'public' AND c.relname = relation_name
               AND c.relkind IN ('r', 'p') AND c.relrowsecurity AND c.relforcerowsecurity
        ) THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_forced_rls_required: %', relation_name;
        END IF;
    END LOOP;
    IF NOT EXISTS (
        SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public' AND c.relname = 'trace_account_owned_principals'
           AND c.relkind = 'v' AND c.reloptions @> ARRAY['security_invoker=true']::TEXT[]
    ) THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_owned_principal_view_required';
    END IF;
    IF (SELECT count(*) FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace
         WHERE n.nspname = 'public'
           AND p.proname IN ('trace_account_trust_fact_candidates', 'trace_record_account_trust_fact')
           AND p.prosecdef
           AND p.proowner = to_regrole('trace_account_admission_guard')
           AND (strpos(pg_get_functiondef(p.oid), 'trace_account_owned_principals') > 0
                OR strpos(pg_get_functiondef(p.oid), 'trace_account_trust_live_owned_principal') > 0)) <> 2 THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_owned_principal_recorder_required';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_proc p
         WHERE p.oid = to_regprocedure('public.trace_account_trust_live_owned_principal(text,uuid,text)')
           AND p.prosecdef AND p.proowner = to_regrole('trace_account_admission_guard')
           AND strpos(pg_get_functiondef(p.oid), 'trace_account_owned_principals') > 0
    ) THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_owned_principal_helper_required';
    END IF;

    FOREACH group_name IN ARRAY ARRAY['trace_account_trust_worker', 'trace_account_admission_runtime'] LOOP
        IF NOT EXISTS (
            SELECT 1 FROM pg_roles WHERE rolname = group_name
             AND NOT rolcanlogin AND NOT rolsuper AND NOT rolbypassrls
             AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication
        ) THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_runtime_group_invalid: %', group_name;
        END IF;
    END LOOP;

    SELECT oid INTO target_oid FROM pg_roles WHERE rolname = 'tc_upload_trust_login';
    IF target_oid IS NOT NULL THEN
        -- Refuse an unexpected/elevated pre-existing identity. Do not repair it
        -- by revoking ownership/memberships or by changing another role.
        IF EXISTS (SELECT 1 FROM pg_roles WHERE oid = target_oid
                    AND (rolsuper OR rolbypassrls OR rolcreatedb OR rolcreaterole OR rolreplication)) THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_existing_login_elevated';
        END IF;
        IF EXISTS (SELECT 1 FROM pg_roles r WHERE r.oid <> target_oid
                    AND r.rolname NOT IN ('trace_account_trust_worker', 'trace_account_admission_runtime')
                    AND pg_has_role(target_oid, r.oid, 'MEMBER')) THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_existing_login_unexpected_membership';
        END IF;
        IF EXISTS (SELECT 1 FROM pg_auth_members WHERE member = target_oid
                    AND (admin_option OR set_option)) THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_existing_login_membership_can_administer_or_set';
        END IF;
        IF EXISTS (SELECT 1 FROM pg_class WHERE relowner = target_oid)
           OR EXISTS (SELECT 1 FROM pg_proc WHERE proowner = target_oid)
           OR EXISTS (SELECT 1 FROM pg_namespace WHERE nspowner = target_oid)
           OR EXISTS (SELECT 1 FROM pg_database WHERE datdba = target_oid) THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_existing_login_owns_objects';
        END IF;
        IF has_schema_privilege(target_oid, 'public', 'CREATE') THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_existing_login_can_create_in_public';
        END IF;
    ELSE
        CREATE ROLE tc_upload_trust_login LOGIN INHERIT
            NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
    END IF;
END
$prepare$;

-- Changes to an already safe login are limited to its expected LOGIN/INHERIT
-- attributes and these exact grants. A configured password is left untouched.
ALTER ROLE tc_upload_trust_login LOGIN INHERIT;
GRANT trace_account_trust_worker TO tc_upload_trust_login
    WITH ADMIN FALSE, INHERIT TRUE, SET FALSE;
GRANT trace_account_admission_runtime TO tc_upload_trust_login
    WITH ADMIN FALSE, INHERIT TRUE, SET FALSE;
GRANT USAGE ON SCHEMA public TO tc_upload_trust_login;
DO $connect$
BEGIN
    EXECUTE format('GRANT CONNECT ON DATABASE %I TO tc_upload_trust_login', current_database());
END
$connect$;

-- resolve_contribution_account -> near_provisioned_row_for_principal and
-- account_admission_status are ordinary forced-RLS reads. Grant only their
-- columns; candidate/source facts remain behind the SECURITY DEFINER seams.
GRANT SELECT (tenant_id, account_id, closed_at)
    ON public.trace_accounts TO tc_upload_trust_login;
GRANT SELECT (tenant_id, account_id, principal_ref, unlinked_at)
    ON public.trace_account_principals TO tc_upload_trust_login;
GRANT SELECT (tenant_id, account_id, principal_ref, device_key_id, anchor_hash)
    ON public.trace_near_provisioned_devices TO tc_upload_trust_login;
GRANT SELECT (tenant_id, device_key_id, revoked_at, onboarding_origin)
    ON public.device_keys TO tc_upload_trust_login;
GRANT SELECT (tenant_id, account_id, revoked_at)
    ON public.trace_account_invite_grants TO tc_upload_trust_login;
GRANT SELECT (tenant_id, account_id, authority, trust_version)
    ON public.trace_account_trust TO tc_upload_trust_login;
GRANT SELECT (tenant_id, account_id, period_id, cost_used, cost_limit, cost_bound, policy_version)
    ON public.trace_account_admission_budget TO tc_upload_trust_login;
-- SELECT on evaluations, and the guarded worker/status EXECUTE rights, come
-- from the two reviewed runtime groups. Never grant direct trust-table writes.

DO $verify$
DECLARE
    relation_name TEXT;
BEGIN
    FOREACH relation_name IN ARRAY ARRAY[
        'trace_account_trust_facts', 'trace_account_trust_evaluations',
        'trace_account_trust_frontiers', 'trace_account_trust_dependency_locks',
        'trace_submissions', 'trace_gate_decisions', 'trace_credit_ledger',
        'trace_audit_events', 'trace_device_upload_principals'
    ] LOOP
        IF has_table_privilege('tc_upload_trust_login', 'public.' || relation_name, 'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER') THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_forbidden_direct_write: %', relation_name;
        END IF;
    END LOOP;
    IF has_table_privilege('tc_upload_trust_login', 'public.trace_account_trust_frontiers', 'SELECT')
       OR has_table_privilege('tc_upload_trust_login', 'public.trace_account_trust_dependency_locks', 'SELECT') THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_forbidden_private_table_read';
    END IF;
    IF has_schema_privilege('tc_upload_trust_login', 'public', 'CREATE') THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_forbidden_schema_create';
    END IF;
    IF EXISTS (SELECT 1 FROM pg_roles r WHERE r.rolname <> 'tc_upload_trust_login'
                AND r.rolname NOT IN ('trace_account_trust_worker', 'trace_account_admission_runtime')
                AND pg_has_role('tc_upload_trust_login', r.oid, 'MEMBER')) THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_unexpected_membership';
    END IF;
    IF has_function_privilege('tc_upload_trust_login',
        'public.trace_record_external_account_trust_evaluation(text,uuid,uuid,text,text,timestamp with time zone,integer,bigint,text,bigint)', 'EXECUTE') THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_external_evaluator_write_forbidden';
    END IF;
    IF (SELECT count(DISTINCT g.oid) FROM pg_auth_members m JOIN pg_roles g ON g.oid = m.roleid
         WHERE m.member = 'tc_upload_trust_login'::regrole
           AND g.rolname IN ('trace_account_trust_worker', 'trace_account_admission_runtime')
           AND m.inherit_option AND NOT m.set_option AND NOT m.admin_option) <> 2 THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_runtime_membership_options_invalid';
    END IF;
END
$verify$;
COMMIT;

-- Read-only preflight. The growth switch may legitimately remain false here;
-- bootstrap never calls trace_account_trust_enable_external_growth().
BEGIN READ ONLY;
SET LOCAL ROLE tc_upload_trust_login;
SELECT json_build_object(
    'role', current_user,
    'login', r.rolcanlogin,
    'inherit', r.rolinherit,
    'superuser', r.rolsuper,
    'bypassrls', r.rolbypassrls,
    'createdb', r.rolcreatedb,
    'createrole', r.rolcreaterole
) AS upload_trust_role_preflight
FROM pg_roles r WHERE r.rolname = current_user;
SELECT json_build_object(
    'worker_member', pg_has_role(current_user, 'trace_account_trust_worker', 'MEMBER'),
    'admission_runtime_member', pg_has_role(current_user, 'trace_account_admission_runtime', 'MEMBER'),
    'worker_enumeration_execute', has_function_privilege(current_user, 'public.trace_account_trust_worker_accounts(text,uuid,bigint)', 'EXECUTE'),
    'candidate_execute', has_function_privilege(current_user, 'public.trace_account_trust_fact_candidates(text,uuid,bigint)', 'EXECUTE'),
    'fact_writer_execute', has_function_privilege(current_user, 'public.trace_record_account_trust_fact(text,uuid,text,uuid)', 'EXECUTE'),
    'evaluation_inputs_execute', has_function_privilege(current_user, 'public.trace_account_trust_evaluation_inputs(text,uuid)', 'EXECUTE'),
    'frontier_read_execute', has_function_privilege(current_user, 'public.trace_account_trust_input_generation(text,uuid)', 'EXECUTE'),
    'evaluation_reader_select', has_table_privilege(current_user, 'public.trace_account_trust_evaluations', 'SELECT'),
    'fact_direct_write', has_table_privilege(current_user, 'public.trace_account_trust_facts', 'INSERT,UPDATE,DELETE'),
    'evaluation_direct_write', has_table_privilege(current_user, 'public.trace_account_trust_evaluations', 'INSERT,UPDATE,DELETE'),
    'frontier_direct_access', has_table_privilege(current_user, 'public.trace_account_trust_frontiers', 'SELECT,INSERT,UPDATE,DELETE'),
    'dependency_lock_direct_access', has_table_privilege(current_user, 'public.trace_account_trust_dependency_locks', 'SELECT,INSERT,UPDATE,DELETE'),
    'external_invalidation_enabled', public.trace_account_trust_external_growth_enabled()
) AS upload_trust_contract_preflight;
COMMIT;
