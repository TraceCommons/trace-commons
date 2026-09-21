-- Copyright (C) 2026 K&Z Partners LLC
-- SPDX-License-Identifier: AGPL-3.0-or-later
--
-- Immutable executable packages for published account-bound mission rewards.
-- Runtime publication uses the existing issuer authority. Public HTTP adapters
-- may read only through trace_reward_mission_get/list and must revalidate the
-- typed package before execution. This SQL boundary protects storage and the
-- anonymous projection shape; Rust remains authoritative for canonical
-- rendering, artifact hashes, and the evolving privacy classifier.

CREATE FUNCTION public.trace_reward_mission_package_valid(p_package TEXT)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
SET search_path = pg_catalog
AS $$
DECLARE
    v_package JSONB;
    v_skill JSONB;
    v_execution JSONB;
    v_mission_text TEXT;
    v_program_text TEXT;
    v_mission UUID;
    v_program UUID;
    v_key TEXT;
    v_top_keys CONSTANT TEXT[] := ARRAY[
        'schema_version', 'mission_id', 'program_id', 'offer_version_hash',
        'task', 'skill', 'skill_sha256', 'evaluator_id',
        'evaluation_contract_hash', 'execution'
    ];
    v_skill_keys CONSTANT TEXT[] := ARRAY[
        'name', 'description', 'procedure'
    ];
    v_execution_keys CONSTANT TEXT[] := ARRAY[
        'required_model_owner', 'total_requests', 'output_token_limit',
        'request_timeout_seconds', 'max_concurrency'
    ];
BEGIN
    IF p_package IS NULL
        OR pg_catalog.octet_length(
            pg_catalog.convert_to(p_package, 'UTF8')
        ) > 65536 THEN
        RETURN FALSE;
    END IF;

    v_package := p_package::JSONB;
    IF pg_catalog.jsonb_typeof(v_package) <> 'object'
        OR NOT v_package ?& v_top_keys
        OR (
            SELECT pg_catalog.count(*)
              FROM pg_catalog.jsonb_object_keys(v_package)
        ) <> pg_catalog.array_length(v_top_keys, 1) THEN
        RETURN FALSE;
    END IF;

    IF pg_catalog.jsonb_typeof(v_package->'schema_version') <> 'number'
        OR v_package->'schema_version' <> '1'::JSONB
        OR pg_catalog.jsonb_typeof(v_package->'mission_id') <> 'string'
        OR pg_catalog.jsonb_typeof(v_package->'program_id') <> 'string'
        OR pg_catalog.jsonb_typeof(v_package->'offer_version_hash') <> 'string'
        OR pg_catalog.jsonb_typeof(v_package->'task') <> 'string'
        OR pg_catalog.jsonb_typeof(v_package->'skill') <> 'object'
        OR pg_catalog.jsonb_typeof(v_package->'skill_sha256') <> 'string'
        OR pg_catalog.jsonb_typeof(v_package->'evaluator_id') <> 'string'
        OR pg_catalog.jsonb_typeof(
            v_package->'evaluation_contract_hash'
        ) <> 'string'
        OR pg_catalog.jsonb_typeof(v_package->'execution') <> 'object' THEN
        RETURN FALSE;
    END IF;

    v_mission_text := v_package->>'mission_id';
    v_program_text := v_package->>'program_id';
    IF v_mission_text !~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
        OR v_program_text !~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$' THEN
        RETURN FALSE;
    END IF;
    v_mission := v_mission_text::UUID;
    v_program := v_program_text::UUID;
    IF v_mission = '00000000-0000-0000-0000-000000000000'::UUID
        OR v_program = '00000000-0000-0000-0000-000000000000'::UUID THEN
        RETURN FALSE;
    END IF;

    IF v_package->>'offer_version_hash' !~ '^sha256:[0-9a-f]{64}$'
        OR v_package->>'skill_sha256' !~ '^[0-9a-f]{64}$'
        OR v_package->>'evaluator_id' <> 'skill-evaluation-v1'
        OR v_package->>'evaluation_contract_hash' !~ '^[0-9a-f]{64}$'
        OR pg_catalog.btrim(v_package->>'task') = ''
        OR pg_catalog.btrim(v_package->>'task')
            IS DISTINCT FROM v_package->>'task'
        OR pg_catalog.octet_length(
            pg_catalog.convert_to(v_package->>'task', 'UTF8')
        ) > 8000
        OR pg_catalog.replace(
            pg_catalog.replace(v_package->>'task', E'\n', ''),
            E'\t',
            ''
        ) ~ '[[:cntrl:]]' THEN
        RETURN FALSE;
    END IF;

    v_skill := v_package->'skill';
    IF NOT v_skill ?& v_skill_keys
        OR (
            SELECT pg_catalog.count(*)
              FROM pg_catalog.jsonb_object_keys(v_skill)
        ) <> pg_catalog.array_length(v_skill_keys, 1) THEN
        RETURN FALSE;
    END IF;
    FOREACH v_key IN ARRAY v_skill_keys LOOP
        IF pg_catalog.jsonb_typeof(v_skill->v_key) <> 'string'
            OR pg_catalog.btrim(v_skill->>v_key) = ''
            OR pg_catalog.btrim(v_skill->>v_key)
                IS DISTINCT FROM v_skill->>v_key THEN
            RETURN FALSE;
        END IF;
    END LOOP;

    v_execution := v_package->'execution';
    IF NOT v_execution ?& v_execution_keys
        OR (
            SELECT pg_catalog.count(*)
              FROM pg_catalog.jsonb_object_keys(v_execution)
        ) <> pg_catalog.array_length(v_execution_keys, 1)
        OR pg_catalog.jsonb_typeof(
            v_execution->'required_model_owner'
        ) <> 'string'
        OR v_execution->>'required_model_owner' <> 'nearai'
        OR pg_catalog.jsonb_typeof(
            v_execution->'total_requests'
        ) <> 'number'
        OR v_execution->'total_requests' <> '24'::JSONB
        OR pg_catalog.jsonb_typeof(
            v_execution->'output_token_limit'
        ) <> 'number'
        OR v_execution->'output_token_limit' <> '900'::JSONB
        OR pg_catalog.jsonb_typeof(
            v_execution->'request_timeout_seconds'
        ) <> 'number'
        OR v_execution->'request_timeout_seconds' <> '90'::JSONB
        OR pg_catalog.jsonb_typeof(
            v_execution->'max_concurrency'
        ) <> 'number'
        OR v_execution->'max_concurrency' <> '2'::JSONB THEN
        RETURN FALSE;
    END IF;

    RETURN TRUE;
EXCEPTION WHEN OTHERS THEN
    RETURN FALSE;
END;
$$;

CREATE TABLE public.trace_reward_mission_packages (
    tenant_id TEXT NOT NULL,
    program_id UUID NOT NULL,
    mission_id UUID NOT NULL UNIQUE,
    package_json TEXT NOT NULL
        CHECK (public.trace_reward_mission_package_valid(package_json)),
    offer_version_hash TEXT GENERATED ALWAYS AS (
        (package_json::JSONB)->>'offer_version_hash'
    ) STORED CHECK (offer_version_hash ~ '^sha256:[0-9a-f]{64}$'),
    task_preview TEXT GENERATED ALWAYS AS (
        pg_catalog.left((package_json::JSONB)->>'task', 240)
    ) STORED,
    package_sha256 TEXT NOT NULL CHECK (package_sha256 ~ '^[0-9a-f]{64}$'),
    publisher_hash TEXT NOT NULL
        CHECK (publisher_hash ~ '^sha256:[0-9a-f]{64}$'),
    published_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, program_id),
    FOREIGN KEY (tenant_id, program_id)
        REFERENCES public.trace_reward_programs(tenant_id, program_id)
        ON DELETE RESTRICT,
    CHECK (
        package_sha256 = pg_catalog.encode(
            pg_catalog.sha256(
                pg_catalog.convert_to(package_json, 'UTF8')
            ),
            'hex'
        )
    )
);

ALTER TABLE public.trace_reward_mission_packages ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_mission_packages FORCE ROW LEVEL SECURITY;

CREATE POLICY trace_reward_operator_tenant_access
    ON public.trace_reward_mission_packages
    TO trace_reward_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());

CREATE POLICY trace_reward_mission_public_read
    ON public.trace_reward_mission_packages
    FOR SELECT TO trace_reward_offer_reader
    USING (TRUE);

REVOKE ALL ON public.trace_reward_mission_packages
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime,
        trace_reward_participant_guard, trace_reward_offer_reader;
GRANT SELECT, INSERT ON public.trace_reward_mission_packages
    TO trace_reward_guard;
GRANT SELECT (
    tenant_id, program_id, mission_id, package_json, offer_version_hash,
    task_preview, package_sha256, published_at
) ON public.trace_reward_mission_packages TO trace_reward_offer_reader;

CREATE FUNCTION public.trace_reward_mission_package_immutable()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $$
BEGIN
    RAISE EXCEPTION USING MESSAGE = 'reward_state_conflict';
END;
$$;

CREATE TRIGGER trace_reward_mission_package_row_immutable
BEFORE UPDATE OR DELETE ON public.trace_reward_mission_packages
FOR EACH ROW
EXECUTE FUNCTION public.trace_reward_mission_package_immutable();

CREATE TRIGGER trace_reward_mission_package_truncate_immutable
BEFORE TRUNCATE ON public.trace_reward_mission_packages
FOR EACH STATEMENT
EXECUTE FUNCTION public.trace_reward_mission_package_immutable();

CREATE FUNCTION public.trace_reward_mission_publish(
    p_tenant TEXT, p_program UUID, p_package TEXT
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_package JSONB;
    v_package_program UUID;
    v_mission UUID;
    v_identity_mode TEXT;
    v_activity_kind TEXT;
    v_offer_version_hash TEXT;
    v_closes_at TIMESTAMPTZ;
    v_suspended BOOLEAN;
    v_prior public.trace_reward_mission_packages%ROWTYPE;
    v_hash TEXT;
    v_now TIMESTAMPTZ;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, 'issuer');
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_tenant, 691)
    );
    IF p_program IS NULL
        OR p_program = '00000000-0000-0000-0000-000000000000'::UUID
        OR NOT public.trace_reward_mission_package_valid(p_package) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;

    v_package := p_package::JSONB;
    v_package_program := (v_package->>'program_id')::UUID;
    v_mission := (v_package->>'mission_id')::UUID;
    IF v_package_program IS DISTINCT FROM p_program THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
    END IF;

    SELECT package.*
      INTO v_prior
      FROM public.trace_reward_mission_packages package
     WHERE package.tenant_id = p_tenant
       AND package.program_id = p_program;
    IF FOUND THEN
        IF v_prior.mission_id IS DISTINCT FROM v_mission
            OR v_prior.package_json IS DISTINCT FROM p_package THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
        RETURN pg_catalog.jsonb_build_object(
            'schema_version', 1,
            'package_json', v_prior.package_json,
            'package_sha256', v_prior.package_sha256,
            'published_at', v_prior.published_at
        );
    END IF;

    IF EXISTS (
        SELECT 1
          FROM public.trace_reward_mission_packages package
         WHERE package.mission_id = v_mission
    ) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
    END IF;

    SELECT program.identity_mode, program.terms->>'activity_kind',
           program.closes_at, offer.offer_version_hash, control.suspended
      INTO v_identity_mode, v_activity_kind, v_closes_at,
           v_offer_version_hash, v_suspended
      FROM public.trace_reward_programs program
      JOIN public.trace_reward_offers offer
        ON offer.tenant_id = program.tenant_id
       AND offer.program_id = program.program_id
      JOIN public.trace_reward_offer_controls control
        ON control.tenant_id = program.tenant_id
       AND control.program_id = program.program_id
     WHERE program.tenant_id = p_tenant
       AND program.program_id = p_program;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    IF v_identity_mode <> 'account_bound'
        OR v_activity_kind <> 'mission_completion'
        OR v_offer_version_hash
            IS DISTINCT FROM v_package->>'offer_version_hash' THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
    END IF;
    IF v_suspended THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_offer_suspended';
    END IF;
    IF v_closes_at <= pg_catalog.clock_timestamp() THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_program_closed';
    END IF;

    v_hash := pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(p_package, 'UTF8')),
        'hex'
    );
    v_now := pg_catalog.clock_timestamp();
    INSERT INTO public.trace_reward_mission_packages(
        tenant_id, program_id, mission_id, package_json, package_sha256,
        publisher_hash, published_at
    ) VALUES (
        p_tenant, p_program, v_mission, p_package, v_hash, v_actor, v_now
    ) RETURNING * INTO v_prior;

    RETURN pg_catalog.jsonb_build_object(
        'schema_version', 1,
        'package_json', v_prior.package_json,
        'package_sha256', v_prior.package_sha256,
        'published_at', v_prior.published_at
    );
EXCEPTION WHEN unique_violation THEN
    RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
END;
$$;

-- Runs with the caller's tenant cleared, in the body rather than with a
-- `SET trace_commons.trace_tenant_id = ''` clause: PostgreSQL allows that clause, for a
-- parameter it has no definition of, only to a true superuser, so a database
-- migrated by its own non-superuser owner could not create this function.
-- `set_config` is transaction-local, not function-local, so the caller's value
-- goes back before the single RETURN. See the note in V64.
CREATE FUNCTION public.trace_reward_mission_get(p_mission UUID)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_caller_tenant TEXT := pg_catalog.current_setting('trace_commons.trace_tenant_id', true);
    v_result JSONB;
    v_suspended BOOLEAN;
    v_closes_at TIMESTAMPTZ;
BEGIN
    PERFORM pg_catalog.set_config('trace_commons.trace_tenant_id', '', true);
    IF p_mission IS NULL
        OR p_mission = '00000000-0000-0000-0000-000000000000'::UUID THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;

    SELECT pg_catalog.jsonb_build_object(
               'schema_version', 1,
               'package_json', package.package_json,
               'package_sha256', package.package_sha256,
               'published_at', package.published_at
           ),
           control.suspended,
           program.closes_at
      INTO v_result, v_suspended, v_closes_at
      FROM public.trace_reward_mission_packages package
      JOIN public.trace_reward_programs program
        ON program.tenant_id = package.tenant_id
       AND program.program_id = package.program_id
      JOIN public.trace_reward_offers offer
        ON offer.tenant_id = package.tenant_id
       AND offer.program_id = package.program_id
      JOIN public.trace_reward_offer_controls control
        ON control.tenant_id = package.tenant_id
       AND control.program_id = package.program_id
     WHERE package.mission_id = p_mission
       AND program.identity_mode = 'account_bound'
       AND program.terms->>'activity_kind' = 'mission_completion'
       AND offer.offer_version_hash = package.offer_version_hash;
    IF v_result IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    IF v_suspended THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_offer_suspended';
    END IF;
    IF v_closes_at <= pg_catalog.clock_timestamp() THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_program_closed';
    END IF;
    PERFORM pg_catalog.set_config('trace_commons.trace_tenant_id', COALESCE(v_caller_tenant, ''), true);
    RETURN v_result;
END;
$$;

-- Runs with the caller's tenant cleared, in the body rather than with a
-- `SET trace_commons.trace_tenant_id = ''` clause: PostgreSQL allows that clause, for a
-- parameter it has no definition of, only to a true superuser, so a database
-- migrated by its own non-superuser owner could not create this function.
-- `set_config` is transaction-local, not function-local, so the caller's value
-- goes back before the single RETURN. See the note in V64.
CREATE FUNCTION public.trace_reward_mission_list(
    p_before UUID, p_limit INTEGER
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_caller_tenant TEXT := pg_catalog.current_setting('trace_commons.trace_tenant_id', true);
    v_entries JSONB;
    v_next_cursor UUID;
BEGIN
    PERFORM pg_catalog.set_config('trace_commons.trace_tenant_id', '', true);
    IF p_limit IS NULL OR p_limit < 1 OR p_limit > 50
        OR (
            p_before IS NOT NULL
            AND p_before = '00000000-0000-0000-0000-000000000000'::UUID
        ) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;

    WITH candidates AS MATERIALIZED (
        SELECT package.mission_id, package.program_id,
               package.package_sha256, offer.offer_version_hash,
               package.task_preview,
               package.published_at
          FROM public.trace_reward_mission_packages package
          JOIN public.trace_reward_programs program
            ON program.tenant_id = package.tenant_id
           AND program.program_id = package.program_id
          JOIN public.trace_reward_offers offer
            ON offer.tenant_id = package.tenant_id
           AND offer.program_id = package.program_id
          JOIN public.trace_reward_offer_controls control
            ON control.tenant_id = package.tenant_id
           AND control.program_id = package.program_id
         WHERE program.identity_mode = 'account_bound'
           AND program.terms->>'activity_kind' = 'mission_completion'
           AND program.closes_at > pg_catalog.clock_timestamp()
           AND NOT control.suspended
           AND offer.offer_version_hash = package.offer_version_hash
           AND (p_before IS NULL OR package.mission_id < p_before)
         ORDER BY package.mission_id DESC
         LIMIT p_limit + 1
    ), chosen AS MATERIALIZED (
        SELECT candidate.*
          FROM candidates candidate
         ORDER BY candidate.mission_id DESC
         LIMIT p_limit
    )
    SELECT COALESCE(
               pg_catalog.jsonb_agg(
                   pg_catalog.jsonb_build_object(
                       'mission_id', chosen.mission_id,
                       'program_id', chosen.program_id,
                       'package_sha256', chosen.package_sha256,
                       'offer_version_hash', chosen.offer_version_hash,
                       'task_preview', chosen.task_preview,
                       'published_at', chosen.published_at
                   )
                   ORDER BY chosen.mission_id DESC
               ),
               '[]'::JSONB
           ),
           CASE
               WHEN (SELECT pg_catalog.count(*) > p_limit FROM candidates)
               THEN (
                   SELECT last_visible.mission_id
                     FROM chosen last_visible
                    ORDER BY last_visible.mission_id ASC
                    LIMIT 1
               )
               ELSE NULL
           END
      INTO v_entries, v_next_cursor
      FROM chosen;

    PERFORM pg_catalog.set_config('trace_commons.trace_tenant_id', COALESCE(v_caller_tenant, ''), true);
    RETURN pg_catalog.jsonb_build_object(
        'schema_version', 1,
        'entries', v_entries,
        'next_cursor', v_next_cursor
    );
END;
$$;

GRANT trace_reward_guard TO CURRENT_USER;
GRANT trace_reward_offer_reader TO CURRENT_USER;
GRANT CREATE ON SCHEMA public
    TO trace_reward_guard, trace_reward_offer_reader;

ALTER FUNCTION public.trace_reward_mission_package_valid(TEXT)
    OWNER TO trace_reward_guard;
ALTER FUNCTION public.trace_reward_mission_package_immutable()
    OWNER TO trace_reward_guard;
ALTER FUNCTION public.trace_reward_mission_publish(TEXT, UUID, TEXT)
    OWNER TO trace_reward_guard;
ALTER FUNCTION public.trace_reward_mission_get(UUID)
    OWNER TO trace_reward_offer_reader;
ALTER FUNCTION public.trace_reward_mission_list(UUID, INTEGER)
    OWNER TO trace_reward_offer_reader;

REVOKE CREATE ON SCHEMA public
    FROM trace_reward_guard, trace_reward_offer_reader;

REVOKE ALL ON FUNCTION public.trace_reward_mission_package_valid(TEXT)
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_mission_package_immutable()
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_mission_publish(TEXT, UUID, TEXT)
    FROM PUBLIC, trace_reward_participant_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_mission_get(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.trace_reward_mission_list(UUID, INTEGER)
    FROM PUBLIC;

GRANT EXECUTE ON FUNCTION public.trace_reward_mission_publish(TEXT, UUID, TEXT)
    TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION public.trace_reward_mission_get(UUID)
    TO CURRENT_USER, trace_reward_guard, trace_reward_participant_runtime;
GRANT EXECUTE ON FUNCTION public.trace_reward_mission_list(UUID, INTEGER)
    TO CURRENT_USER, trace_reward_guard, trace_reward_participant_runtime;

REVOKE trace_reward_guard FROM CURRENT_USER;
REVOKE trace_reward_offer_reader FROM CURRENT_USER;
