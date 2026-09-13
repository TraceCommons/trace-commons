-- Account-bound reward offers, participant reservations, and self-only history.
-- INTEGRATION: register V71 in the server migration list; public HTTP reads call
-- trace_reward_offer_get(uuid), authenticated account routes call only the three
-- trace_reward_participant_* functions, and execute_merge calls
-- trace_reward_accounts_merge after consuming its proposal while holding tenant
-- advisory lock namespace 691. Provision each participant DB login explicitly in
-- trace_reward_participant_logins; never grant it trace_reward_runtime or either
-- function-owner role.

DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_roles
         WHERE rolname = 'trace_reward_participant_guard'
    ) THEN
        CREATE ROLE trace_reward_participant_guard NOLOGIN NOBYPASSRLS;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_roles
         WHERE rolname = 'trace_reward_participant_runtime'
    ) THEN
        CREATE ROLE trace_reward_participant_runtime NOLOGIN NOBYPASSRLS;
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_catalog.pg_roles
         WHERE rolname = 'trace_reward_offer_reader'
    ) THEN
        CREATE ROLE trace_reward_offer_reader NOLOGIN NOBYPASSRLS;
    END IF;
END $$;

ALTER ROLE trace_reward_participant_guard NOSUPERUSER NOLOGIN NOBYPASSRLS;
ALTER ROLE trace_reward_participant_runtime NOSUPERUSER NOLOGIN NOBYPASSRLS;
ALTER ROLE trace_reward_offer_reader NOSUPERUSER NOLOGIN NOBYPASSRLS;

ALTER TABLE public.trace_reward_programs
    DROP CONSTRAINT trace_reward_programs_identity_mode_check;
ALTER TABLE public.trace_reward_programs
    ADD CONSTRAINT trace_reward_programs_identity_mode_check
    CHECK (identity_mode IN ('operator_asserted', 'account_bound'));

CREATE TABLE public.trace_reward_offers (
    tenant_id TEXT NOT NULL REFERENCES public.trace_tenants(tenant_id) ON DELETE CASCADE,
    program_id UUID NOT NULL,
    manifest JSONB NOT NULL CHECK (
        pg_catalog.jsonb_typeof(manifest) = 'object'
        AND pg_catalog.pg_column_size(manifest) <= 32768
    ),
    offer_version_hash TEXT NOT NULL
        CHECK (offer_version_hash ~ '^sha256:[0-9a-f]{64}$'),
    work_namespace_hash TEXT NOT NULL
        CHECK (work_namespace_hash ~ '^sha256:[0-9a-f]{64}$'),
    publisher_hash TEXT NOT NULL CHECK (publisher_hash ~ '^sha256:[0-9a-f]{64}$'),
    published_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, program_id),
    UNIQUE (program_id),
    FOREIGN KEY (tenant_id, program_id)
        REFERENCES public.trace_reward_programs(tenant_id, program_id) ON DELETE RESTRICT
);

CREATE TABLE public.trace_reward_offer_controls (
    tenant_id TEXT NOT NULL,
    program_id UUID NOT NULL,
    suspended BOOLEAN NOT NULL DEFAULT FALSE,
    updater_hash TEXT NOT NULL CHECK (updater_hash ~ '^sha256:[0-9a-f]{64}$'),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, program_id),
    FOREIGN KEY (tenant_id, program_id)
        REFERENCES public.trace_reward_offers(tenant_id, program_id) ON DELETE RESTRICT
);

CREATE TABLE public.trace_reward_participant_logins (
    tenant_id TEXT NOT NULL REFERENCES public.trace_tenants(tenant_id) ON DELETE CASCADE,
    login_role NAME NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, login_role)
);

CREATE TABLE public.trace_reward_principals (
    tenant_id TEXT NOT NULL REFERENCES public.trace_tenants(tenant_id) ON DELETE CASCADE,
    reward_principal_id UUID NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, reward_principal_id)
);

-- A binding's participant hash never changes. Account merges join binding groups
-- by reward_principal_id while prior ledger rows continue to reference their
-- original participant hashes.
CREATE TABLE public.trace_reward_principal_accounts (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    reward_principal_id UUID NOT NULL,
    participant_hash TEXT NOT NULL CHECK (participant_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, account_id),
    UNIQUE (tenant_id, participant_hash),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES public.trace_accounts(tenant_id, account_id) ON DELETE RESTRICT,
    FOREIGN KEY (tenant_id, reward_principal_id)
        REFERENCES public.trace_reward_principals(tenant_id, reward_principal_id)
        ON DELETE RESTRICT
);

CREATE INDEX trace_reward_principal_accounts_group_idx
    ON public.trace_reward_principal_accounts(
        tenant_id, reward_principal_id, participant_hash
    );

CREATE TABLE public.trace_reward_participant_reservations (
    tenant_id TEXT NOT NULL,
    reservation_id UUID NOT NULL,
    participant_hash TEXT NOT NULL CHECK (participant_hash ~ '^sha256:[0-9a-f]{64}$'),
    offer_version_hash TEXT NOT NULL
        CHECK (offer_version_hash ~ '^sha256:[0-9a-f]{64}$'),
    work_namespace_hash TEXT NOT NULL
        CHECK (work_namespace_hash ~ '^sha256:[0-9a-f]{64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, reservation_id),
    FOREIGN KEY (tenant_id, reservation_id)
        REFERENCES public.trace_reward_reservations(tenant_id, reservation_id)
        ON DELETE RESTRICT,
    FOREIGN KEY (tenant_id, participant_hash)
        REFERENCES public.trace_reward_principal_accounts(tenant_id, participant_hash)
        ON DELETE RESTRICT
);

CREATE INDEX trace_reward_participant_reservations_history_idx
    ON public.trace_reward_participant_reservations(
        tenant_id, participant_hash, created_at DESC, reservation_id DESC
    );
CREATE INDEX trace_reward_participant_reservations_work_idx
    ON public.trace_reward_participant_reservations(
        tenant_id, participant_hash, work_namespace_hash
    );

ALTER TABLE public.trace_reward_offers ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_offers FORCE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_offer_controls ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_offer_controls FORCE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_participant_logins ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_participant_logins FORCE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_principals ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_principals FORCE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_principal_accounts ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_principal_accounts FORCE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_participant_reservations ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.trace_reward_participant_reservations FORCE ROW LEVEL SECURITY;

CREATE POLICY trace_reward_operator_tenant_access ON public.trace_reward_offers
    TO trace_reward_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_reward_operator_tenant_access ON public.trace_reward_offer_controls
    TO trace_reward_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());

CREATE POLICY trace_corpus_tenant_isolation ON public.trace_reward_offers
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_corpus_tenant_isolation ON public.trace_reward_offer_controls
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_corpus_tenant_isolation ON public.trace_reward_participant_logins
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_corpus_tenant_isolation ON public.trace_reward_principals
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_corpus_tenant_isolation ON public.trace_reward_principal_accounts
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_corpus_tenant_isolation
    ON public.trace_reward_participant_reservations
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());

-- Existing reward tables only granted their operator guard access in V69.
CREATE POLICY trace_reward_participant_tenant_access ON public.trace_reward_programs
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_reward_participant_tenant_access ON public.trace_reward_reservations
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_reward_participant_tenant_access ON public.trace_reward_decisions
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_reward_participant_tenant_access ON public.trace_reward_awards
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
CREATE POLICY trace_reward_participant_tenant_access ON public.trace_reward_invalidations
    TO trace_reward_participant_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());

-- The public function owner sees only published account-bound programs and the
-- ledger columns needed to calculate committed capacity. It is unassumable by
-- the application login.
CREATE POLICY trace_reward_offer_public_read ON public.trace_reward_offers
    FOR SELECT TO trace_reward_offer_reader USING (TRUE);
CREATE POLICY trace_reward_offer_public_read ON public.trace_reward_offer_controls
    FOR SELECT TO trace_reward_offer_reader USING (TRUE);
CREATE POLICY trace_reward_offer_public_read ON public.trace_reward_programs
    FOR SELECT TO trace_reward_offer_reader
    USING (
        identity_mode = 'account_bound'
        AND EXISTS (
            SELECT 1 FROM public.trace_reward_offers offer
             WHERE offer.tenant_id = trace_reward_programs.tenant_id
               AND offer.program_id = trace_reward_programs.program_id
        )
    );
CREATE POLICY trace_reward_offer_public_read ON public.trace_reward_reservations
    FOR SELECT TO trace_reward_offer_reader
    USING (
        EXISTS (
            SELECT 1 FROM public.trace_reward_offers offer
             WHERE offer.tenant_id = trace_reward_reservations.tenant_id
               AND offer.program_id = trace_reward_reservations.program_id
        )
    );
CREATE POLICY trace_reward_offer_public_read ON public.trace_reward_awards
    FOR SELECT TO trace_reward_offer_reader
    USING (
        EXISTS (
            SELECT 1 FROM public.trace_reward_offers offer
             WHERE offer.tenant_id = trace_reward_awards.tenant_id
               AND offer.program_id = trace_reward_awards.program_id
        )
    );

REVOKE ALL ON public.trace_reward_offers, public.trace_reward_offer_controls,
    public.trace_reward_participant_logins, public.trace_reward_principals,
    public.trace_reward_principal_accounts,
    public.trace_reward_participant_reservations
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;

GRANT USAGE ON SCHEMA public TO trace_reward_participant_guard,
    trace_reward_participant_runtime, trace_reward_offer_reader;

GRANT SELECT, INSERT ON public.trace_reward_offers TO trace_reward_guard;
GRANT SELECT, INSERT ON public.trace_reward_offer_controls TO trace_reward_guard;
GRANT UPDATE (suspended, updater_hash, updated_at)
    ON public.trace_reward_offer_controls TO trace_reward_guard;

GRANT SELECT ON public.trace_reward_participant_logins,
    public.trace_reward_principals, public.trace_reward_principal_accounts,
    public.trace_reward_participant_reservations, public.trace_accounts,
    public.trace_account_merge_proposals, public.trace_reward_programs,
    public.trace_reward_offers, public.trace_reward_offer_controls,
    public.trace_reward_reservations, public.trace_reward_decisions,
    public.trace_reward_awards, public.trace_reward_invalidations
    TO trace_reward_participant_guard;
GRANT INSERT ON public.trace_reward_principals,
    public.trace_reward_principal_accounts,
    public.trace_reward_participant_reservations,
    public.trace_reward_reservations TO trace_reward_participant_guard;
-- PostgreSQL requires UPDATE privilege on at least one column for SELECT FOR
-- SHARE. The guard is NOLOGIN and unassumable by the runtime; account_id is not
-- updated by any reward function.
GRANT UPDATE (account_id) ON public.trace_accounts
    TO trace_reward_participant_guard;
GRANT UPDATE (account_id, reward_principal_id)
    ON public.trace_reward_principal_accounts TO trace_reward_participant_guard;
GRANT EXECUTE ON FUNCTION public.trace_current_tenant_id()
    TO trace_reward_participant_guard;
GRANT EXECUTE ON FUNCTION public.trace_reward_capacity_used(TEXT, UUID, TEXT)
    TO trace_reward_participant_guard;

GRANT SELECT (tenant_id, program_id, manifest, offer_version_hash,
    work_namespace_hash, published_at)
    ON public.trace_reward_offers TO trace_reward_offer_reader;
GRANT SELECT (tenant_id, program_id, suspended)
    ON public.trace_reward_offer_controls TO trace_reward_offer_reader;
GRANT SELECT (tenant_id, program_id, terms, terms_hash, award_units, capacity_units,
    participant_cap_units, closes_at, reservation_ttl_seconds, identity_mode)
    ON public.trace_reward_programs TO trace_reward_offer_reader;
GRANT SELECT (tenant_id, reservation_id, program_id, participant_hash, award_units, state, expires_at)
    ON public.trace_reward_reservations TO trace_reward_offer_reader;
GRANT SELECT (tenant_id, reservation_id)
    ON public.trace_reward_awards TO trace_reward_offer_reader;

CREATE FUNCTION public.trace_reward_manifest_valid(p_manifest JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
SET search_path = pg_catalog
AS $$
DECLARE
    v_expected CONSTANT TEXT[] := ARRAY[
        'schema_version', 'definition', 'rubric', 'required_evidence', 'rights',
        'challenge_policy', 'evaluator_policy', 'reservation_terms'
    ];
    v_text_fields CONSTANT TEXT[] := ARRAY[
        'definition', 'rubric', 'required_evidence', 'rights',
        'challenge_policy', 'evaluator_policy', 'reservation_terms'
    ];
    v_key TEXT;
BEGIN
    IF p_manifest IS NULL
        OR pg_catalog.jsonb_typeof(p_manifest) <> 'object'
        OR pg_catalog.pg_column_size(p_manifest) > 32768
        OR NOT p_manifest ?& v_expected
        OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(p_manifest))
            <> pg_catalog.array_length(v_expected, 1)
        OR pg_catalog.jsonb_typeof(p_manifest->'schema_version') <> 'number'
        OR (p_manifest->>'schema_version')::NUMERIC <> 1 THEN
        RETURN FALSE;
    END IF;

    FOREACH v_key IN ARRAY v_text_fields LOOP
        IF pg_catalog.jsonb_typeof(p_manifest->v_key) <> 'string'
            OR pg_catalog.btrim(p_manifest->>v_key) = ''
            OR pg_catalog.octet_length(
                pg_catalog.convert_to(p_manifest->>v_key, 'UTF8')
            ) > 4096 THEN
            RETURN FALSE;
        END IF;
    END LOOP;
    RETURN TRUE;
EXCEPTION WHEN OTHERS THEN
    RETURN FALSE;
END;
$$;

CREATE FUNCTION public.trace_reward_participant_login_authorize(p_tenant TEXT)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_super BOOLEAN;
    v_bypass BOOLEAN;
BEGIN
    IF p_tenant IS NULL OR p_tenant = ''
        OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;

    SELECT role.rolsuper, role.rolbypassrls
      INTO v_super, v_bypass
      FROM pg_catalog.pg_roles role
     WHERE role.rolname = session_user;
    IF NOT FOUND OR v_super OR v_bypass
        OR NOT pg_catalog.pg_has_role(
            session_user, 'trace_reward_participant_runtime', 'member'
        )
        OR NOT EXISTS (
            SELECT 1 FROM public.trace_reward_participant_logins login
             WHERE login.tenant_id = p_tenant
               AND login.login_role = session_user::NAME
        ) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;
END;
$$;

CREATE FUNCTION public.trace_reward_participant_authorize(
    p_tenant TEXT, p_account UUID
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
BEGIN
    PERFORM public.trace_reward_participant_login_authorize(p_tenant);
    IF p_account IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;
    PERFORM 1 FROM public.trace_accounts account
     WHERE account.tenant_id = p_tenant
       AND account.account_id = p_account
       AND account.closed_at IS NULL
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;
END;
$$;

CREATE FUNCTION public.trace_reward_participant_identity(
    p_tenant TEXT, p_account UUID, p_create BOOLEAN
)
RETURNS TABLE (reward_principal_id UUID, participant_hash TEXT)
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $$
DECLARE
    v_principal UUID;
    v_participant TEXT;
BEGIN
    SELECT binding.reward_principal_id, binding.participant_hash
      INTO v_principal, v_participant
      FROM public.trace_reward_principal_accounts binding
     WHERE binding.tenant_id = p_tenant AND binding.account_id = p_account;
    IF FOUND THEN
        RETURN QUERY SELECT v_principal, v_participant;
        RETURN;
    END IF;
    IF NOT p_create THEN
        RETURN;
    END IF;

    v_principal := pg_catalog.gen_random_uuid();
    v_participant := 'sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            pg_catalog.jsonb_build_array(
                'trace-reward-principal-v1', p_tenant, v_principal
            )::TEXT,
            'UTF8'
        )),
        'hex'
    );
    INSERT INTO public.trace_reward_principals(tenant_id, reward_principal_id)
    VALUES (p_tenant, v_principal);
    INSERT INTO public.trace_reward_principal_accounts(
        tenant_id, account_id, reward_principal_id, participant_hash
    ) VALUES (p_tenant, p_account, v_principal, v_participant);
    RETURN QUERY SELECT v_principal, v_participant;
END;
$$;

-- Canonical capacity invariant for public availability, global admission, and
-- account caps. NULL selects every participant; an empty array selects none.
CREATE FUNCTION public.trace_reward_capacity_used_for_aliases(
    p_tenant TEXT, p_program UUID, p_aliases TEXT[]
)
RETURNS NUMERIC
LANGUAGE SQL
SET search_path = pg_catalog
AS $$
    SELECT COALESCE(pg_catalog.sum(reservation.award_units::NUMERIC), 0)
      FROM public.trace_reward_reservations reservation
     WHERE reservation.tenant_id = p_tenant
       AND reservation.program_id = p_program
       AND (p_aliases IS NULL OR reservation.participant_hash = ANY(p_aliases))
       AND (
           EXISTS (
               SELECT 1 FROM public.trace_reward_awards award
                WHERE award.tenant_id = reservation.tenant_id
                  AND award.reservation_id = reservation.reservation_id
           )
           OR reservation.state = 'submitted'
           OR (
               reservation.state = 'reserved'
               AND reservation.expires_at > pg_catalog.clock_timestamp()
           )
       );
$$;

CREATE FUNCTION public.trace_reward_account_bound_insert_only()
RETURNS TRIGGER
LANGUAGE plpgsql
SET search_path = pg_catalog
AS $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM public.trace_reward_programs program
         WHERE program.tenant_id = NEW.tenant_id
           AND program.program_id = NEW.program_id
           AND program.identity_mode = 'account_bound'
    ) AND current_user <> 'trace_reward_participant_guard' THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER trace_reward_account_bound_insert_only
BEFORE INSERT ON public.trace_reward_reservations
FOR EACH ROW EXECUTE FUNCTION public.trace_reward_account_bound_insert_only();

CREATE FUNCTION public.trace_reward_offer_publish(
    p_tenant TEXT, p_program UUID, p_terms JSONB, p_manifest JSONB
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_program public.trace_reward_programs%ROWTYPE;
    v_offer public.trace_reward_offers%ROWTYPE;
    v_terms_hash TEXT;
    v_version_hash TEXT;
    v_definition_hash TEXT;
    v_key TEXT;
    v_manifest_key TEXT;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, 'issuer');
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_tenant, 691)
    );
    IF p_program IS NULL OR NOT public.trace_reward_terms_valid(p_terms)
        OR NOT public.trace_reward_manifest_valid(p_manifest) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;

    FOREACH v_key IN ARRAY ARRAY[
        'definition_hash', 'rubric_hash', 'required_evidence_hash', 'rights_hash',
        'challenge_policy_hash', 'evaluator_policy_hash'
    ] LOOP
        v_manifest_key := CASE v_key
            WHEN 'definition_hash' THEN 'definition'
            WHEN 'rubric_hash' THEN 'rubric'
            WHEN 'required_evidence_hash' THEN 'required_evidence'
            WHEN 'rights_hash' THEN 'rights'
            WHEN 'challenge_policy_hash' THEN 'challenge_policy'
            WHEN 'evaluator_policy_hash' THEN 'evaluator_policy'
        END;
        IF p_terms->>v_key IS DISTINCT FROM 'sha256:' || pg_catalog.encode(
            pg_catalog.sha256(pg_catalog.convert_to(
                p_manifest->>v_manifest_key, 'UTF8'
            )),
            'hex'
        ) THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
    END LOOP;

    SELECT * INTO v_program FROM public.trace_reward_programs program
     WHERE program.tenant_id = p_tenant AND program.program_id = p_program;
    SELECT * INTO v_offer FROM public.trace_reward_offers offer
     WHERE offer.tenant_id = p_tenant AND offer.program_id = p_program;
    IF v_offer.program_id IS NOT NULL THEN
        IF v_program.terms IS DISTINCT FROM p_terms
            OR v_offer.manifest IS DISTINCT FROM p_manifest THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
        RETURN public.trace_reward_offer_get(p_program);
    END IF;
    IF v_program.program_id IS NOT NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
    END IF;
    IF (p_terms->>'closes_at')::TIMESTAMPTZ <= pg_catalog.clock_timestamp() THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_program_closed';
    END IF;

    v_terms_hash := 'sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(p_terms::TEXT, 'UTF8')), 'hex'
    );
    v_definition_hash := 'sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            p_manifest->>'definition', 'UTF8'
        )),
        'hex'
    );
    v_version_hash := 'sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            pg_catalog.jsonb_build_array(
                'trace-reward-offer-v1', v_terms_hash, p_manifest
            )::TEXT,
            'UTF8'
        )),
        'hex'
    );

    INSERT INTO public.trace_reward_programs(
        tenant_id, program_id, terms, terms_hash, creator_hash, sponsor_hash,
        award_units, capacity_units, participant_cap_units, closes_at,
        reservation_ttl_seconds, identity_mode
    ) VALUES (
        p_tenant, p_program, p_terms, v_terms_hash, v_actor,
        p_terms->>'sponsor_hash',
        ((p_terms->>'award_units')::NUMERIC)::BIGINT,
        ((p_terms->>'capacity_units')::NUMERIC)::BIGINT,
        ((p_terms->>'participant_cap_units')::NUMERIC)::BIGINT,
        (p_terms->>'closes_at')::TIMESTAMPTZ,
        ((p_terms->>'reservation_ttl_seconds')::NUMERIC)::INTEGER,
        'account_bound'
    ) RETURNING * INTO v_program;
    INSERT INTO public.trace_reward_offers(
        tenant_id, program_id, manifest, offer_version_hash,
        work_namespace_hash, publisher_hash
    ) VALUES (
        p_tenant, p_program, p_manifest, v_version_hash,
        v_definition_hash, v_actor
    ) RETURNING * INTO v_offer;
    INSERT INTO public.trace_reward_offer_controls(
        tenant_id, program_id, suspended, updater_hash
    ) VALUES (p_tenant, p_program, FALSE, v_actor);

    RETURN public.trace_reward_offer_get(p_program);
EXCEPTION WHEN unique_violation THEN
    RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
END;
$$;

CREATE FUNCTION public.trace_reward_offer_suspend(
    p_tenant TEXT, p_program UUID, p_suspended BOOLEAN
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_control public.trace_reward_offer_controls%ROWTYPE;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, 'issuer');
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_tenant, 691)
    );
    IF p_program IS NULL OR p_suspended IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT * INTO v_control FROM public.trace_reward_offer_controls control
     WHERE control.tenant_id = p_tenant AND control.program_id = p_program;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    IF v_control.suspended IS DISTINCT FROM p_suspended THEN
        UPDATE public.trace_reward_offer_controls
           SET suspended = p_suspended,
               updater_hash = v_actor,
               updated_at = pg_catalog.clock_timestamp()
         WHERE tenant_id = p_tenant AND program_id = p_program
         RETURNING * INTO v_control;
    END IF;
    RETURN public.trace_reward_offer_get(p_program);
END;
$$;

CREATE FUNCTION public.trace_reward_offer_get(p_program UUID)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
SET trace_commons.trace_tenant_id = ''
AS $$
DECLARE
    v_result JSONB;
BEGIN
    IF p_program IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT pg_catalog.jsonb_build_object(
        'schema_version', 1,
        'program_id', program.program_id,
        'offer_version_hash', offer.offer_version_hash,
        'terms_hash', program.terms_hash,
        'activity_kind', program.terms->>'activity_kind',
        'denomination', 'cloud_credits',
        'award_units', program.award_units,
        'capacity_units', program.capacity_units,
        'capacity_used_units', used.units,
        'capacity_remaining_units', program.capacity_units - used.units,
        'participant_cap_units', program.participant_cap_units,
        'closes_at', program.closes_at,
        'reservation_ttl_seconds', program.reservation_ttl_seconds,
        'published_at', offer.published_at,
        'suspended', control.suspended,
        'manifest', offer.manifest
    ) INTO v_result
      FROM public.trace_reward_offers offer
      JOIN public.trace_reward_programs program
        ON program.tenant_id = offer.tenant_id
       AND program.program_id = offer.program_id
      JOIN public.trace_reward_offer_controls control
        ON control.tenant_id = offer.tenant_id
       AND control.program_id = offer.program_id
      CROSS JOIN LATERAL (
          SELECT public.trace_reward_capacity_used_for_aliases(
              offer.tenant_id, offer.program_id, NULL
          )::BIGINT AS units
      ) used
     WHERE offer.program_id = p_program;
    IF v_result IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    RETURN v_result;
END;
$$;

CREATE FUNCTION public.trace_reward_participant_reservation_projection(
    p_tenant TEXT, p_reservation UUID
)
RETURNS JSONB
LANGUAGE SQL
STABLE
SET search_path = pg_catalog
AS $$
    SELECT pg_catalog.jsonb_build_object(
        'schema_version', 1,
        'reservation_id', reservation.reservation_id,
        'program_id', reservation.program_id,
        'offer_version_hash', participant.offer_version_hash,
        'state', CASE
            WHEN invalidation.decision_id IS NOT NULL AND award.award_id IS NOT NULL
                THEN 'awarded_invalidated'
            WHEN reservation.state = 'reserved'
                AND reservation.expires_at <= pg_catalog.clock_timestamp()
                THEN 'expired'
            ELSE reservation.state
        END,
        'invalidated', invalidation.decision_id IS NOT NULL,
        'denomination', 'cloud_credits',
        'pinned_units', reservation.award_units,
        'awarded_units', CASE
            WHEN award.award_id IS NULL THEN 0 ELSE award.award_units
        END,
        'expires_at', reservation.expires_at,
        'created_at', reservation.created_at,
        'submitted_at', reservation.submitted_at,
        'terminal_at', reservation.terminal_at,
        'decisions', COALESCE(decisions.items, '[]'::JSONB)
    )
      FROM public.trace_reward_reservations reservation
      JOIN public.trace_reward_participant_reservations participant
        ON participant.tenant_id = reservation.tenant_id
       AND participant.reservation_id = reservation.reservation_id
      LEFT JOIN public.trace_reward_awards award
        ON award.tenant_id = reservation.tenant_id
       AND award.reservation_id = reservation.reservation_id
      LEFT JOIN public.trace_reward_invalidations invalidation
        ON invalidation.tenant_id = reservation.tenant_id
       AND invalidation.evidence_hash = reservation.evidence_hash
      LEFT JOIN LATERAL (
          SELECT pg_catalog.jsonb_agg(
              item.value ORDER BY item.created_at, item.decision_id
          ) AS items
            FROM (
                SELECT decision.created_at, decision.decision_id,
                    pg_catalog.jsonb_build_object(
                        'decision_id', decision.decision_id,
                        'kind', decision.decision_kind,
                        'accepted', decision.accepted,
                        'reason', decision.reason,
                        'created_at', decision.created_at
                    ) AS value
                  FROM public.trace_reward_decisions decision
                 WHERE decision.tenant_id = reservation.tenant_id
                   AND decision.reservation_id = reservation.reservation_id
                UNION ALL
                SELECT decision.created_at, decision.decision_id,
                    pg_catalog.jsonb_build_object(
                        'decision_id', decision.decision_id,
                        'kind', decision.decision_kind,
                        'accepted', decision.accepted,
                        'reason', decision.reason,
                        'created_at', decision.created_at
                    ) AS value
                  FROM public.trace_reward_decisions decision
                 WHERE decision.tenant_id = reservation.tenant_id
                   AND decision.decision_kind = 'invalidation'
                   AND decision.evidence_hash = reservation.evidence_hash
            ) item
      ) decisions ON TRUE
     WHERE reservation.tenant_id = p_tenant
       AND reservation.reservation_id = p_reservation;
$$;

CREATE FUNCTION public.trace_reward_participant_reserve(
    p_tenant TEXT, p_account UUID, p_program UUID, p_reservation UUID,
    p_offer_version_hash TEXT
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_identity RECORD;
    v_program public.trace_reward_programs%ROWTYPE;
    v_offer public.trace_reward_offers%ROWTYPE;
    v_prior public.trace_reward_reservations%ROWTYPE;
    v_prior_participant public.trace_reward_participant_reservations%ROWTYPE;
    v_aliases TEXT[];
    v_suspended BOOLEAN;
    v_used NUMERIC;
    v_participant_used NUMERIC;
    v_now TIMESTAMPTZ;
    v_expires TIMESTAMPTZ;
    v_work_hash TEXT;
    v_consent_hash TEXT;
BEGIN
    PERFORM public.trace_reward_participant_login_authorize(p_tenant);
    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_tenant, 691)
    );
    PERFORM public.trace_reward_participant_authorize(p_tenant, p_account);
    v_now := pg_catalog.clock_timestamp();
    IF p_program IS NULL OR p_reservation IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;

    SELECT * INTO v_identity
      FROM public.trace_reward_participant_identity(p_tenant, p_account, FALSE);
    SELECT * INTO v_prior FROM public.trace_reward_reservations reservation
     WHERE reservation.tenant_id = p_tenant
       AND reservation.reservation_id = p_reservation;
    IF FOUND THEN
        IF v_identity.reward_principal_id IS NULL THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
        END IF;
        SELECT pg_catalog.array_agg(binding.participant_hash)
          INTO v_aliases
          FROM public.trace_reward_principal_accounts binding
         WHERE binding.tenant_id = p_tenant
           AND binding.reward_principal_id = v_identity.reward_principal_id;
        IF NOT COALESCE(v_prior.participant_hash = ANY(v_aliases), FALSE) THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
        END IF;
        SELECT * INTO v_prior_participant
          FROM public.trace_reward_participant_reservations participant
         WHERE participant.tenant_id = p_tenant
           AND participant.reservation_id = p_reservation
           AND participant.participant_hash = v_prior.participant_hash;
        IF NOT FOUND THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
        END IF;
        IF v_prior.program_id IS DISTINCT FROM p_program
            OR v_prior_participant.offer_version_hash IS DISTINCT FROM p_offer_version_hash THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
        RETURN public.trace_reward_participant_reservation_projection(
            p_tenant, p_reservation
        );
    END IF;

    IF p_offer_version_hash IS NULL
        OR p_offer_version_hash !~ '^sha256:[0-9a-f]{64}$' THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;

    IF v_identity.reward_principal_id IS NULL THEN
        SELECT * INTO v_identity
          FROM public.trace_reward_participant_identity(p_tenant, p_account, TRUE);
    END IF;
    SELECT pg_catalog.array_agg(binding.participant_hash)
      INTO v_aliases
      FROM public.trace_reward_principal_accounts binding
     WHERE binding.tenant_id = p_tenant
       AND binding.reward_principal_id = v_identity.reward_principal_id;

    SELECT * INTO v_program
      FROM public.trace_reward_programs program
     WHERE program.tenant_id = p_tenant AND program.program_id = p_program
       AND program.identity_mode = 'account_bound';
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    SELECT * INTO v_offer
      FROM public.trace_reward_offers offer
     WHERE offer.tenant_id = p_tenant AND offer.program_id = p_program;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_state_conflict';
    END IF;
    IF v_offer.offer_version_hash IS DISTINCT FROM p_offer_version_hash THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_offer_changed';
    END IF;
    SELECT control.suspended INTO v_suspended
      FROM public.trace_reward_offer_controls control
     WHERE control.tenant_id = p_tenant AND control.program_id = p_program;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_state_conflict';
    END IF;
    IF v_suspended THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_offer_suspended';
    END IF;
    IF v_program.closes_at <= v_now THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_program_closed';
    END IF;
    IF EXISTS (
        SELECT 1
          FROM public.trace_reward_participant_reservations participant
         WHERE participant.tenant_id = p_tenant
           AND participant.participant_hash = ANY(v_aliases)
           AND participant.work_namespace_hash = v_offer.work_namespace_hash
    ) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_work_duplicate';
    END IF;

    v_used := public.trace_reward_capacity_used(p_tenant, p_program, NULL);
    IF v_used + v_program.award_units::NUMERIC > v_program.capacity_units::NUMERIC THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_capacity_exhausted';
    END IF;
    v_participant_used := public.trace_reward_capacity_used_for_aliases(
        p_tenant, p_program, v_aliases
    );
    IF v_participant_used + v_program.award_units::NUMERIC
        > v_program.participant_cap_units::NUMERIC THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_participant_cap';
    END IF;

    v_expires := LEAST(
        v_now + pg_catalog.make_interval(secs => v_program.reservation_ttl_seconds),
        v_program.closes_at
    );
    v_work_hash := 'sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            pg_catalog.jsonb_build_array(
                'trace-reward-work-v1', v_identity.participant_hash,
                v_offer.work_namespace_hash
            )::TEXT,
            'UTF8'
        )),
        'hex'
    );
    v_consent_hash := 'sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(
            pg_catalog.jsonb_build_array(
                'trace-reward-reservation-ack-v1', v_identity.participant_hash,
                p_offer_version_hash, p_reservation
            )::TEXT,
            'UTF8'
        )),
        'hex'
    );
    INSERT INTO public.trace_reward_reservations(
        tenant_id, reservation_id, program_id, participant_hash, work_hash,
        consent_hash, creator_hash, award_units, state, expires_at, created_at
    ) VALUES (
        p_tenant, p_reservation, p_program, v_identity.participant_hash,
        v_work_hash, v_consent_hash, v_identity.participant_hash,
        v_program.award_units, 'reserved', v_expires, v_now
    );
    INSERT INTO public.trace_reward_participant_reservations(
        tenant_id, reservation_id, participant_hash, offer_version_hash,
        work_namespace_hash, created_at
    ) VALUES (
        p_tenant, p_reservation, v_identity.participant_hash,
        p_offer_version_hash, v_offer.work_namespace_hash, v_now
    );
    RETURN public.trace_reward_participant_reservation_projection(
        p_tenant, p_reservation
    );
END;
$$;

CREATE FUNCTION public.trace_reward_participant_get(
    p_tenant TEXT, p_account UUID, p_reservation UUID
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_principal UUID;
    v_result JSONB;
BEGIN
    PERFORM public.trace_reward_participant_login_authorize(p_tenant);
    PERFORM pg_catalog.pg_advisory_xact_lock_shared(
        pg_catalog.hashtextextended(p_tenant, 691)
    );
    PERFORM public.trace_reward_participant_authorize(p_tenant, p_account);
    IF p_reservation IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT binding.reward_principal_id INTO v_principal
      FROM public.trace_reward_principal_accounts binding
     WHERE binding.tenant_id = p_tenant AND binding.account_id = p_account;
    IF NOT FOUND OR NOT EXISTS (
        SELECT 1
          FROM public.trace_reward_reservations reservation
          JOIN public.trace_reward_participant_reservations participant
            ON participant.tenant_id = reservation.tenant_id
           AND participant.reservation_id = reservation.reservation_id
          JOIN public.trace_reward_principal_accounts binding
            ON binding.tenant_id = reservation.tenant_id
           AND binding.participant_hash = reservation.participant_hash
         WHERE reservation.tenant_id = p_tenant
           AND reservation.reservation_id = p_reservation
           AND binding.reward_principal_id = v_principal
    ) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    v_result := public.trace_reward_participant_reservation_projection(
        p_tenant, p_reservation
    );
    IF v_result IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    RETURN v_result;
END;
$$;

CREATE FUNCTION public.trace_reward_participant_history(
    p_tenant TEXT, p_account UUID, p_limit INTEGER, p_before UUID DEFAULT NULL
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_principal UUID;
    v_before_created_at TIMESTAMPTZ;
    v_entries JSONB;
    v_has_more BOOLEAN;
    v_next_cursor UUID;
    v_awarded_units TEXT;
BEGIN
    PERFORM public.trace_reward_participant_login_authorize(p_tenant);
    PERFORM pg_catalog.pg_advisory_xact_lock_shared(
        pg_catalog.hashtextextended(p_tenant, 691)
    );
    PERFORM public.trace_reward_participant_authorize(p_tenant, p_account);
    IF p_limit IS NULL OR p_limit < 1 OR p_limit > 100 THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT binding.reward_principal_id INTO v_principal
      FROM public.trace_reward_principal_accounts binding
     WHERE binding.tenant_id = p_tenant AND binding.account_id = p_account;
    IF NOT FOUND THEN
        IF p_before IS NOT NULL THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
        END IF;
        RETURN pg_catalog.jsonb_build_object(
            'schema_version', 1,
            'identity_mode', 'account_bound',
            'denomination', 'cloud_credits',
            'limit', p_limit,
            'truncated', FALSE,
            'next_cursor', NULL,
            'awarded_units', '0',
            'entries', '[]'::JSONB
        );
    END IF;

    IF p_before IS NOT NULL THEN
        SELECT reservation.created_at INTO v_before_created_at
          FROM public.trace_reward_reservations reservation
          JOIN public.trace_reward_participant_reservations participant
            ON participant.tenant_id = reservation.tenant_id
           AND participant.reservation_id = reservation.reservation_id
          JOIN public.trace_reward_principal_accounts binding
            ON binding.tenant_id = participant.tenant_id
           AND binding.participant_hash = participant.participant_hash
         WHERE reservation.tenant_id = p_tenant
           AND reservation.reservation_id = p_before
           AND binding.reward_principal_id = v_principal;
        IF NOT FOUND THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
        END IF;
    END IF;

    WITH candidates AS MATERIALIZED (
        SELECT reservation.reservation_id, reservation.created_at
          FROM public.trace_reward_reservations reservation
          JOIN public.trace_reward_participant_reservations participant
            ON participant.tenant_id = reservation.tenant_id
           AND participant.reservation_id = reservation.reservation_id
          JOIN public.trace_reward_principal_accounts binding
            ON binding.tenant_id = participant.tenant_id
           AND binding.participant_hash = participant.participant_hash
         WHERE reservation.tenant_id = p_tenant
           AND binding.reward_principal_id = v_principal
           AND (
               p_before IS NULL
               OR (reservation.created_at, reservation.reservation_id)
                    < (v_before_created_at, p_before)
           )
         ORDER BY reservation.created_at DESC, reservation.reservation_id DESC
         LIMIT p_limit + 1
    ), chosen AS MATERIALIZED (
        SELECT candidate.* FROM candidates candidate
         ORDER BY candidate.created_at DESC, candidate.reservation_id DESC
         LIMIT p_limit
    )
    SELECT COALESCE(pg_catalog.jsonb_agg(
               public.trace_reward_participant_reservation_projection(
                   p_tenant, chosen.reservation_id
               ) ORDER BY chosen.created_at DESC, chosen.reservation_id DESC
           ), '[]'::JSONB),
           (SELECT pg_catalog.count(*) > p_limit FROM candidates),
           CASE WHEN (SELECT pg_catalog.count(*) > p_limit FROM candidates) THEN
               (SELECT last_visible.reservation_id
                  FROM chosen last_visible
                 ORDER BY last_visible.created_at ASC,
                          last_visible.reservation_id ASC
                 LIMIT 1)
               ELSE NULL
           END
      INTO v_entries, v_has_more, v_next_cursor
      FROM chosen;

    SELECT COALESCE(pg_catalog.sum(award.award_units::NUMERIC), 0)::TEXT
      INTO v_awarded_units
      FROM public.trace_reward_awards award
      JOIN public.trace_reward_participant_reservations participant
        ON participant.tenant_id = award.tenant_id
       AND participant.reservation_id = award.reservation_id
      JOIN public.trace_reward_principal_accounts binding
        ON binding.tenant_id = participant.tenant_id
       AND binding.participant_hash = participant.participant_hash
     WHERE award.tenant_id = p_tenant
       AND binding.reward_principal_id = v_principal;

    RETURN pg_catalog.jsonb_build_object(
        'schema_version', 1,
        'identity_mode', 'account_bound',
        'denomination', 'cloud_credits',
        'limit', p_limit,
        'truncated', v_has_more,
        'next_cursor', v_next_cursor,
        'awarded_units', v_awarded_units,
        'entries', v_entries
    );
END;
$$;

CREATE FUNCTION public.trace_reward_accounts_merge(
    p_tenant TEXT, p_surviving_account UUID, p_absorbed_account UUID,
    p_proposal UUID
)
RETURNS VOID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_surviving_principal UUID;
    v_absorbed_principal UUID;
BEGIN
    IF p_tenant IS NULL OR p_tenant = ''
        OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_surviving_account IS NULL OR p_absorbed_account IS NULL
        OR p_surviving_account = p_absorbed_account OR p_proposal IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;

    PERFORM pg_catalog.pg_advisory_xact_lock(
        pg_catalog.hashtextextended(p_tenant, 691)
    );
    SELECT binding.reward_principal_id INTO v_surviving_principal
      FROM public.trace_reward_principal_accounts binding
     WHERE binding.tenant_id = p_tenant
       AND binding.account_id = p_surviving_account;
    SELECT binding.reward_principal_id INTO v_absorbed_principal
      FROM public.trace_reward_principal_accounts binding
     WHERE binding.tenant_id = p_tenant
       AND binding.account_id = p_absorbed_account;

    -- Preserve pre-R1 account merging without requiring participant provisioning.
    IF v_surviving_principal IS NULL AND v_absorbed_principal IS NULL THEN
        RETURN;
    END IF;

    PERFORM public.trace_reward_participant_authorize(
        p_tenant, p_surviving_account
    );
    PERFORM 1 FROM public.trace_accounts account
     WHERE account.tenant_id = p_tenant
       AND account.account_id = p_absorbed_account
       AND account.closed_at IS NULL
     FOR SHARE;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;
    IF NOT EXISTS (
        SELECT 1
          FROM public.trace_account_merge_proposals proposal
          JOIN public.trace_accounts survivor
            ON survivor.tenant_id = proposal.tenant_id
           AND survivor.account_id = proposal.surviving_account_id
          JOIN public.trace_accounts absorbed
            ON absorbed.tenant_id = proposal.tenant_id
           AND absorbed.account_id = proposal.absorbed_account_id
         WHERE proposal.tenant_id = p_tenant
           AND proposal.proposal_id = p_proposal
           AND proposal.surviving_account_id = p_surviving_account
           AND proposal.absorbed_account_id = p_absorbed_account
           AND proposal.consumed_at IS NOT NULL
           AND proposal.xmin::TEXT = pg_catalog.pg_current_xact_id()::TEXT
           AND survivor.closed_at IS NULL
           AND absorbed.closed_at IS NULL
    ) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;

    IF v_surviving_principal IS NULL THEN
        UPDATE public.trace_reward_principal_accounts
           SET account_id = p_surviving_account
         WHERE tenant_id = p_tenant AND account_id = p_absorbed_account;
    ELSIF v_absorbed_principal IS NOT NULL
        AND v_absorbed_principal <> v_surviving_principal THEN
        UPDATE public.trace_reward_principal_accounts
           SET reward_principal_id = v_surviving_principal
         WHERE tenant_id = p_tenant
           AND reward_principal_id = v_absorbed_principal;
    END IF;
END;
$$;

GRANT trace_reward_guard TO CURRENT_USER;
GRANT trace_reward_participant_guard TO CURRENT_USER;
GRANT trace_reward_offer_reader TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_reward_guard, trace_reward_participant_guard;

-- Keep the R0 signature and privileges; all callers now share the array-aware
-- invariant above. This replacement runs after acquiring owner membership.
CREATE OR REPLACE FUNCTION public.trace_reward_capacity_used(
    p_tenant TEXT, p_program UUID, p_participant TEXT
)
RETURNS NUMERIC
LANGUAGE SQL
SET search_path = pg_catalog
AS $$
    SELECT public.trace_reward_capacity_used_for_aliases(
        p_tenant, p_program,
        CASE WHEN p_participant IS NULL THEN NULL::TEXT[]
             ELSE ARRAY[p_participant] END
    );
$$;

ALTER FUNCTION public.trace_reward_manifest_valid(JSONB) OWNER TO trace_reward_guard;
ALTER FUNCTION public.trace_reward_offer_publish(TEXT, UUID, JSONB, JSONB)
    OWNER TO trace_reward_guard;
ALTER FUNCTION public.trace_reward_offer_suspend(TEXT, UUID, BOOLEAN)
    OWNER TO trace_reward_guard;
ALTER FUNCTION public.trace_reward_offer_get(UUID) OWNER TO trace_reward_offer_reader;
ALTER FUNCTION public.trace_reward_participant_login_authorize(TEXT)
    OWNER TO trace_reward_participant_guard;
ALTER FUNCTION public.trace_reward_participant_authorize(TEXT, UUID)
    OWNER TO trace_reward_participant_guard;
ALTER FUNCTION public.trace_reward_participant_identity(TEXT, UUID, BOOLEAN)
    OWNER TO trace_reward_participant_guard;
ALTER FUNCTION public.trace_reward_capacity_used_for_aliases(TEXT, UUID, TEXT[])
    OWNER TO trace_reward_guard;
ALTER FUNCTION public.trace_reward_account_bound_insert_only()
    OWNER TO trace_reward_participant_guard;
ALTER FUNCTION public.trace_reward_participant_reservation_projection(TEXT, UUID)
    OWNER TO trace_reward_participant_guard;
ALTER FUNCTION public.trace_reward_participant_reserve(TEXT, UUID, UUID, UUID, TEXT)
    OWNER TO trace_reward_participant_guard;
ALTER FUNCTION public.trace_reward_participant_get(TEXT, UUID, UUID)
    OWNER TO trace_reward_participant_guard;
ALTER FUNCTION public.trace_reward_participant_history(TEXT, UUID, INTEGER, UUID)
    OWNER TO trace_reward_participant_guard;
ALTER FUNCTION public.trace_reward_accounts_merge(TEXT, UUID, UUID, UUID)
    OWNER TO trace_reward_participant_guard;

REVOKE CREATE ON SCHEMA public FROM trace_reward_guard, trace_reward_participant_guard;

REVOKE ALL ON FUNCTION public.trace_reward_manifest_valid(JSONB)
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_participant_login_authorize(TEXT)
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_participant_authorize(TEXT, UUID)
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_participant_identity(TEXT, UUID, BOOLEAN)
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_capacity_used_for_aliases(TEXT, UUID, TEXT[])
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;
GRANT EXECUTE ON FUNCTION public.trace_reward_capacity_used_for_aliases(TEXT, UUID, TEXT[])
    TO trace_reward_guard, trace_reward_participant_guard, trace_reward_offer_reader;
REVOKE ALL ON FUNCTION public.trace_reward_account_bound_insert_only()
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_participant_reservation_projection(TEXT, UUID)
    FROM PUBLIC, trace_reward_runtime, trace_reward_participant_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_offer_publish(TEXT, UUID, JSONB, JSONB)
    FROM PUBLIC;
REVOKE ALL ON FUNCTION public.trace_reward_offer_suspend(TEXT, UUID, BOOLEAN)
    FROM PUBLIC;
REVOKE ALL ON FUNCTION public.trace_reward_offer_get(UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION public.trace_reward_participant_reserve(
    TEXT, UUID, UUID, UUID, TEXT
) FROM PUBLIC, trace_reward_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_participant_get(TEXT, UUID, UUID)
    FROM PUBLIC, trace_reward_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_participant_history(
    TEXT, UUID, INTEGER, UUID
) FROM PUBLIC, trace_reward_runtime;
REVOKE ALL ON FUNCTION public.trace_reward_accounts_merge(TEXT, UUID, UUID, UUID)
    FROM PUBLIC, trace_reward_runtime;

GRANT EXECUTE ON FUNCTION public.trace_reward_offer_publish(TEXT, UUID, JSONB, JSONB)
    TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION public.trace_reward_offer_suspend(TEXT, UUID, BOOLEAN)
    TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION public.trace_reward_offer_get(UUID)
    TO CURRENT_USER, trace_reward_guard, trace_reward_participant_runtime;
GRANT EXECUTE ON FUNCTION public.trace_reward_participant_reserve(
    TEXT, UUID, UUID, UUID, TEXT
) TO trace_reward_participant_runtime;
GRANT EXECUTE ON FUNCTION public.trace_reward_participant_get(TEXT, UUID, UUID)
    TO trace_reward_participant_runtime;
GRANT EXECUTE ON FUNCTION public.trace_reward_participant_history(
    TEXT, UUID, INTEGER, UUID
) TO trace_reward_participant_runtime;
-- The account facade always invokes this hook. PUBLIC may enter only so a
-- deployment with no R1 mappings preserves its existing merges; the function
-- returns before mutation in that case. Alias-bearing calls still require the
-- explicit participant-login row and the current transaction's consumed proof.
GRANT EXECUTE ON FUNCTION public.trace_reward_accounts_merge(TEXT, UUID, UUID, UUID)
    TO PUBLIC;

REVOKE trace_reward_guard FROM CURRENT_USER;
REVOKE trace_reward_participant_guard FROM CURRENT_USER;
REVOKE trace_reward_offer_reader FROM CURRENT_USER;
