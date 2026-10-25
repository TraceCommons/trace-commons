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

    IF NOT EXISTS (
        SELECT 1 FROM pg_roles WHERE rolname = 'trace_account_trust_worker'
          AND NOT rolcanlogin AND NOT rolsuper AND NOT rolbypassrls
          AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolreplication
    ) OR EXISTS (SELECT 1 FROM pg_auth_members
                 WHERE member = to_regrole('trace_account_trust_worker')) THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_worker_group_invalid';
    END IF;
    FOREACH group_name IN ARRAY ARRAY['trace_client_upload_trust_runtime', 'tc_upload_trust_login'] LOOP
        SELECT oid INTO target_oid FROM pg_roles WHERE rolname = group_name;
        IF target_oid IS NOT NULL THEN
            -- Existing leaky identities are refused, never normalized/revoked.
            IF EXISTS (SELECT 1 FROM pg_roles WHERE oid = target_oid
                        AND (rolsuper OR rolbypassrls OR rolcreatedb OR rolcreaterole OR rolreplication
                          OR (group_name = 'trace_client_upload_trust_runtime' AND rolcanlogin))) THEN
                RAISE EXCEPTION 'upload_trust_bootstrap_existing_role_elevated';
            END IF;
            IF EXISTS (SELECT 1 FROM pg_roles r WHERE r.oid <> target_oid
                       AND r.rolname <> 'trace_account_trust_worker'
                       AND (group_name <> 'tc_upload_trust_login' OR r.rolname <> 'trace_client_upload_trust_runtime')
                       AND pg_has_role(target_oid, r.oid, 'MEMBER')) THEN
                RAISE EXCEPTION 'upload_trust_bootstrap_existing_role_unexpected_membership';
            END IF;
            IF EXISTS (SELECT 1 FROM pg_auth_members WHERE member = target_oid
                        AND (admin_option OR set_option OR NOT inherit_option)) THEN
                RAISE EXCEPTION 'upload_trust_bootstrap_existing_role_membership_options_invalid';
            END IF;
            IF EXISTS (SELECT 1 FROM pg_class WHERE relowner = target_oid)
               OR EXISTS (SELECT 1 FROM pg_proc WHERE proowner = target_oid)
               OR EXISTS (SELECT 1 FROM pg_namespace WHERE nspowner = target_oid)
               OR EXISTS (SELECT 1 FROM pg_database WHERE datdba = target_oid) THEN
                RAISE EXCEPTION 'upload_trust_bootstrap_existing_role_owns_objects';
            END IF;
            IF has_schema_privilege(target_oid, 'public', 'CREATE') THEN
                RAISE EXCEPTION 'upload_trust_bootstrap_existing_role_can_create_in_public';
            END IF;
        ELSIF group_name = 'trace_client_upload_trust_runtime' THEN
            CREATE ROLE trace_client_upload_trust_runtime NOLOGIN INHERIT
                NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
        ELSE
            CREATE ROLE tc_upload_trust_login LOGIN INHERIT
                NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS;
        END IF;
    END LOOP;
END
$prepare$;

-- The worker role has only guarded recorder/evaluator seams and evaluation
-- SELECT. Do not inherit the admission runtime or its reservation authority.
GRANT trace_account_trust_worker TO trace_client_upload_trust_runtime
    WITH ADMIN FALSE, INHERIT TRUE, SET FALSE;
GRANT trace_client_upload_trust_runtime TO tc_upload_trust_login
    WITH ADMIN FALSE, INHERIT TRUE, SET FALSE;
GRANT USAGE ON SCHEMA public TO trace_client_upload_trust_runtime;
DO $connect$
BEGIN
    EXECUTE format('GRANT CONNECT ON DATABASE %I TO tc_upload_trust_login', current_database());
END
$connect$;

-- Only the independent worker/status call graph: no reservation locks,
-- dependency pruning, admission linkage, transition or external writer.
GRANT EXECUTE ON FUNCTION public.trace_account_trust_enable_external_growth(),
    public.trace_account_trust_external_growth_enabled(),
    public.trace_account_trust_input_generation(TEXT,UUID)
    TO trace_client_upload_trust_runtime;
GRANT SELECT (tenant_id, account_id, closed_at)
    ON public.trace_accounts TO trace_client_upload_trust_runtime;
GRANT SELECT (tenant_id, account_id, principal_ref, unlinked_at)
    ON public.trace_account_principals TO trace_client_upload_trust_runtime;
GRANT SELECT (tenant_id, account_id, principal_ref, device_key_id, anchor_hash)
    ON public.trace_near_provisioned_devices TO trace_client_upload_trust_runtime;
GRANT SELECT (tenant_id, device_key_id, revoked_at, onboarding_origin)
    ON public.device_keys TO trace_client_upload_trust_runtime;
GRANT SELECT (tenant_id, account_id, revoked_at)
    ON public.trace_account_invite_grants TO trace_client_upload_trust_runtime;
GRANT SELECT (tenant_id, account_id, authority, trust_version)
    ON public.trace_account_trust TO trace_client_upload_trust_runtime;
GRANT SELECT (tenant_id, account_id, period_id, cost_used, cost_limit, cost_bound, policy_version)
    ON public.trace_account_admission_budget TO trace_client_upload_trust_runtime;

DO $verify$
DECLARE
    target_name TEXT;
BEGIN
    FOREACH target_name IN ARRAY ARRAY['trace_client_upload_trust_runtime', 'tc_upload_trust_login'] LOOP
        IF EXISTS (
            SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = 'public' AND c.relkind IN ('r','p','v','m','f')
               AND (has_table_privilege(target_name,c.oid,'INSERT,UPDATE,DELETE,TRUNCATE,TRIGGER,REFERENCES')
                 OR has_any_column_privilege(target_name,c.oid,'INSERT,UPDATE,REFERENCES'))
        ) THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_forbidden_direct_or_column_write';
        END IF;
        IF EXISTS (
            SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
              JOIN pg_attribute a ON a.attrelid=c.oid AND a.attnum>0 AND NOT a.attisdropped
             WHERE n.nspname='public' AND c.relkind IN ('r','p','v','m','f')
               AND c.relname <> 'trace_account_trust_evaluations'
               AND has_column_privilege(target_name,c.oid,a.attnum,'SELECT')
               AND NOT EXISTS (
                   SELECT 1 FROM (VALUES
                       ('trace_accounts',ARRAY['tenant_id','account_id','closed_at']),
                       ('trace_account_principals',ARRAY['tenant_id','account_id','principal_ref','unlinked_at']),
                       ('trace_near_provisioned_devices',ARRAY['tenant_id','account_id','principal_ref','device_key_id','anchor_hash']),
                       ('device_keys',ARRAY['tenant_id','device_key_id','revoked_at','onboarding_origin']),
                       ('trace_account_invite_grants',ARRAY['tenant_id','account_id','revoked_at']),
                       ('trace_account_trust',ARRAY['tenant_id','account_id','authority','trust_version']),
                       ('trace_account_admission_budget',ARRAY['tenant_id','account_id','period_id','cost_used','cost_limit','cost_bound','policy_version'])
                   ) allowed(relation_name,column_names)
                   WHERE c.relname=allowed.relation_name AND a.attname=ANY(allowed.column_names))
        ) THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_forbidden_direct_or_column_read';
        END IF;
        IF EXISTS (SELECT 1 FROM pg_proc p JOIN pg_namespace n ON n.oid=p.pronamespace
             WHERE n.nspname='public'
               AND p.proname IN ('trace_reserve_admission','trace_transition_admission',
                   'trace_resume_legacy_admission','trace_account_admission_live_device',
                   'trace_account_admission_active_grant','trace_account_admission_linkage_ready',
                   'trace_account_trust_lock_input_generation','trace_account_trust_prune_dependency_locks',
                   'trace_record_external_account_trust_evaluation')
               AND has_function_privilege(target_name,p.oid,'EXECUTE')) THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_forbidden_admission_or_external_execute';
        END IF;
        IF has_schema_privilege(target_name,'public','CREATE')
           OR pg_has_role(target_name,'trace_account_admission_runtime','MEMBER') THEN
            RAISE EXCEPTION 'upload_trust_bootstrap_forbidden_schema_or_admission_role';
        END IF;
    END LOOP;
    IF (SELECT count(*) FROM pg_auth_members
         WHERE member='tc_upload_trust_login'::regrole) <> 1
       OR NOT EXISTS (SELECT 1 FROM pg_auth_members
         WHERE member='tc_upload_trust_login'::regrole AND roleid='trace_client_upload_trust_runtime'::regrole
           AND inherit_option AND NOT set_option AND NOT admin_option)
       OR (SELECT count(*) FROM pg_auth_members
         WHERE member='trace_client_upload_trust_runtime'::regrole) <> 1
       OR NOT EXISTS (SELECT 1 FROM pg_auth_members
         WHERE member='trace_client_upload_trust_runtime'::regrole AND roleid='trace_account_trust_worker'::regrole
           AND inherit_option AND NOT set_option AND NOT admin_option) THEN
        RAISE EXCEPTION 'upload_trust_bootstrap_runtime_membership_options_invalid';
    END IF;
END
$verify$;
COMMIT;

-- Read-only preflight. Preparation never enables the invalidation switch.
BEGIN READ ONLY;
SET LOCAL ROLE tc_upload_trust_login;
SELECT json_build_object(
    'role', current_user, 'login', r.rolcanlogin, 'inherit', r.rolinherit,
    'superuser', r.rolsuper, 'bypassrls', r.rolbypassrls,
    'createdb', r.rolcreatedb, 'createrole', r.rolcreaterole
) AS upload_trust_role_preflight FROM pg_roles r WHERE r.rolname=current_user;
SELECT json_build_object(
    'upload_trust_member', pg_has_role(current_user,'trace_client_upload_trust_runtime','MEMBER'),
    'worker_member', pg_has_role(current_user,'trace_account_trust_worker','MEMBER'),
    'admission_runtime_member', pg_has_role(current_user,'trace_account_admission_runtime','MEMBER'),
    'worker_enumeration_execute', has_function_privilege(current_user,'public.trace_account_trust_worker_accounts(text,uuid,bigint)','EXECUTE'),
    'candidate_execute', has_function_privilege(current_user,'public.trace_account_trust_fact_candidates(text,uuid,bigint)','EXECUTE'),
    'fact_writer_execute', has_function_privilege(current_user,'public.trace_record_account_trust_fact(text,uuid,text,uuid)','EXECUTE'),
    'evaluation_inputs_execute', has_function_privilege(current_user,'public.trace_account_trust_evaluation_inputs(text,uuid)','EXECUTE'),
    'frontier_read_execute', has_function_privilege(current_user,'public.trace_account_trust_input_generation(text,uuid)','EXECUTE'),
    'evaluation_reader_select', has_table_privilege(current_user,'public.trace_account_trust_evaluations','SELECT'),
    'fact_column_write', has_any_column_privilege(current_user,'public.trace_account_trust_facts','INSERT,UPDATE,REFERENCES'),
    'account_column_write', has_any_column_privilege(current_user,'public.trace_accounts','INSERT,UPDATE,REFERENCES'),
    'budget_column_write', has_any_column_privilege(current_user,'public.trace_account_admission_budget','INSERT,UPDATE,REFERENCES'),
    'admission_transition_execute', has_function_privilege(current_user,'public.trace_transition_admission(text,uuid,uuid,text)','EXECUTE'),
    'external_invalidation_enabled', public.trace_account_trust_external_growth_enabled()
) AS upload_trust_contract_preflight;
COMMIT;
