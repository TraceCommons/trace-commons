-- Operator-managed mission and insight reward ledger.
-- INTEGRATION: register V69 in the server migration list. Runtime callers use only
-- trace_reward_program_create/show, trace_reward_reserve, trace_reward_claim_submit,
-- trace_reward_review/cancel/invalidate, and trace_reward_history; operator grants
-- are provisioned separately by a DBA against trace_reward_operators.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'trace_reward_guard') THEN
        CREATE ROLE trace_reward_guard NOLOGIN NOBYPASSRLS;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_catalog.pg_roles WHERE rolname = 'trace_reward_runtime') THEN
        CREATE ROLE trace_reward_runtime NOLOGIN NOBYPASSRLS;
    END IF;
END $$;

ALTER ROLE trace_reward_guard NOSUPERUSER NOLOGIN NOBYPASSRLS;
ALTER ROLE trace_reward_runtime NOSUPERUSER NOLOGIN NOBYPASSRLS;

CREATE TABLE trace_reward_operators (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    login_role NAME NOT NULL,
    actor_hash TEXT NOT NULL CHECK (actor_hash ~ '^sha256:[0-9a-f]{64}$'),
    operator_role TEXT NOT NULL CHECK (operator_role IN ('issuer', 'reviewer')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, login_role),
    UNIQUE (tenant_id, actor_hash)
);

CREATE TABLE trace_reward_programs (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    program_id UUID NOT NULL,
    terms JSONB NOT NULL,
    terms_hash TEXT NOT NULL CHECK (terms_hash ~ '^sha256:[0-9a-f]{64}$'),
    creator_hash TEXT NOT NULL CHECK (creator_hash ~ '^sha256:[0-9a-f]{64}$'),
    sponsor_hash TEXT NOT NULL CHECK (sponsor_hash ~ '^sha256:[0-9a-f]{64}$'),
    award_units BIGINT NOT NULL CHECK (award_units > 0),
    capacity_units BIGINT NOT NULL CHECK (capacity_units > 0),
    participant_cap_units BIGINT NOT NULL CHECK (participant_cap_units > 0),
    closes_at TIMESTAMPTZ NOT NULL,
    reservation_ttl_seconds INTEGER NOT NULL
        CHECK (reservation_ttl_seconds BETWEEN 1 AND 604800),
    created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, program_id),
    CHECK (award_units <= participant_cap_units),
    CHECK (participant_cap_units <= capacity_units)
);

CREATE TABLE trace_reward_reservations (
    tenant_id TEXT NOT NULL,
    reservation_id UUID NOT NULL,
    program_id UUID NOT NULL,
    participant_hash TEXT NOT NULL CHECK (participant_hash ~ '^sha256:[0-9a-f]{64}$'),
    work_hash TEXT NOT NULL CHECK (work_hash ~ '^sha256:[0-9a-f]{64}$'),
    consent_hash TEXT NOT NULL CHECK (consent_hash ~ '^sha256:[0-9a-f]{64}$'),
    creator_hash TEXT NOT NULL CHECK (creator_hash ~ '^sha256:[0-9a-f]{64}$'),
    award_units BIGINT NOT NULL CHECK (award_units > 0),
    state TEXT NOT NULL CHECK (
        state IN ('reserved', 'submitted', 'awarded', 'rejected', 'cancelled', 'invalidated')
    ),
    expires_at TIMESTAMPTZ NOT NULL,
    evidence_hash TEXT CHECK (
        evidence_hash IS NULL OR evidence_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    evaluation_hash TEXT CHECK (
        evaluation_hash IS NULL OR evaluation_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    submitter_hash TEXT CHECK (
        submitter_hash IS NULL OR submitter_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    submitted_at TIMESTAMPTZ,
    terminal_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, reservation_id),
    UNIQUE (tenant_id, work_hash),
    UNIQUE (tenant_id, evidence_hash),
    FOREIGN KEY (tenant_id, program_id)
        REFERENCES trace_reward_programs(tenant_id, program_id) ON DELETE RESTRICT,
    CHECK (
        (evidence_hash IS NULL AND evaluation_hash IS NULL
            AND submitter_hash IS NULL AND submitted_at IS NULL)
        OR
        (evidence_hash IS NOT NULL AND evaluation_hash IS NOT NULL
            AND submitter_hash IS NOT NULL AND submitted_at IS NOT NULL)
    )
);

CREATE TABLE trace_reward_decisions (
    tenant_id TEXT NOT NULL,
    decision_id UUID NOT NULL,
    decision_kind TEXT NOT NULL CHECK (decision_kind IN ('review', 'cancel', 'invalidation')),
    reservation_id UUID,
    evidence_hash TEXT CHECK (
        evidence_hash IS NULL OR evidence_hash ~ '^sha256:[0-9a-f]{64}$'
    ),
    actor_hash TEXT NOT NULL CHECK (actor_hash ~ '^sha256:[0-9a-f]{64}$'),
    accepted BOOLEAN,
    reason TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, decision_id),
    FOREIGN KEY (tenant_id, reservation_id)
        REFERENCES trace_reward_reservations(tenant_id, reservation_id) ON DELETE RESTRICT,
    CHECK (
        (decision_kind = 'review' AND reservation_id IS NOT NULL
            AND evidence_hash IS NULL AND accepted IS NOT NULL AND reason IS NOT NULL)
        OR
        (decision_kind = 'cancel' AND reservation_id IS NOT NULL
            AND evidence_hash IS NULL AND accepted IS NULL AND reason = 'issuer_cancelled')
        OR
        (decision_kind = 'invalidation' AND reservation_id IS NULL
            AND evidence_hash IS NOT NULL AND accepted IS NULL AND reason = 'evidence_invalidated')
    )
);

CREATE TABLE trace_reward_awards (
    tenant_id TEXT NOT NULL,
    award_id UUID NOT NULL,
    reservation_id UUID NOT NULL,
    program_id UUID NOT NULL,
    participant_hash TEXT NOT NULL CHECK (participant_hash ~ '^sha256:[0-9a-f]{64}$'),
    reviewer_hash TEXT NOT NULL CHECK (reviewer_hash ~ '^sha256:[0-9a-f]{64}$'),
    award_units BIGINT NOT NULL CHECK (award_units > 0),
    terms_hash TEXT NOT NULL CHECK (terms_hash ~ '^sha256:[0-9a-f]{64}$'),
    awarded_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, award_id),
    UNIQUE (tenant_id, reservation_id),
    FOREIGN KEY (tenant_id, reservation_id)
        REFERENCES trace_reward_reservations(tenant_id, reservation_id) ON DELETE RESTRICT,
    FOREIGN KEY (tenant_id, program_id)
        REFERENCES trace_reward_programs(tenant_id, program_id) ON DELETE RESTRICT,
    FOREIGN KEY (tenant_id, award_id)
        REFERENCES trace_reward_decisions(tenant_id, decision_id) ON DELETE RESTRICT
);

CREATE TABLE trace_reward_invalidations (
    tenant_id TEXT NOT NULL,
    decision_id UUID NOT NULL,
    evidence_hash TEXT NOT NULL CHECK (evidence_hash ~ '^sha256:[0-9a-f]{64}$'),
    invalidator_hash TEXT NOT NULL CHECK (invalidator_hash ~ '^sha256:[0-9a-f]{64}$'),
    invalidated_at TIMESTAMPTZ NOT NULL DEFAULT pg_catalog.clock_timestamp(),
    PRIMARY KEY (tenant_id, decision_id),
    UNIQUE (tenant_id, evidence_hash),
    FOREIGN KEY (tenant_id, decision_id)
        REFERENCES trace_reward_decisions(tenant_id, decision_id) ON DELETE RESTRICT
);

CREATE INDEX trace_reward_reservations_program_state_idx
    ON trace_reward_reservations(tenant_id, program_id, state, expires_at);
CREATE INDEX trace_reward_reservations_participant_idx
    ON trace_reward_reservations(tenant_id, participant_hash, created_at DESC);
CREATE INDEX trace_reward_awards_participant_idx
    ON trace_reward_awards(tenant_id, participant_hash, program_id);
CREATE INDEX trace_reward_decisions_reservation_idx
    ON trace_reward_decisions(tenant_id, reservation_id);
CREATE INDEX trace_reward_decisions_invalidation_idx
    ON trace_reward_decisions(tenant_id, evidence_hash)
    WHERE decision_kind = 'invalidation';

ALTER TABLE trace_reward_operators ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_operators FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_programs ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_programs FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_reservations ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_reservations FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_decisions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_decisions FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_awards ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_awards FORCE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_invalidations ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_reward_invalidations FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_reward_operators;
CREATE POLICY trace_corpus_tenant_isolation ON trace_reward_operators
    TO trace_reward_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_reward_programs;
CREATE POLICY trace_corpus_tenant_isolation ON trace_reward_programs
    TO trace_reward_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_reward_reservations;
CREATE POLICY trace_corpus_tenant_isolation ON trace_reward_reservations
    TO trace_reward_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_reward_decisions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_reward_decisions
    TO trace_reward_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_reward_awards;
CREATE POLICY trace_corpus_tenant_isolation ON trace_reward_awards
    TO trace_reward_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_reward_invalidations;
CREATE POLICY trace_corpus_tenant_isolation ON trace_reward_invalidations
    TO trace_reward_guard
    USING (tenant_id = public.trace_current_tenant_id())
    WITH CHECK (tenant_id = public.trace_current_tenant_id());

REVOKE ALL ON trace_reward_operators, trace_reward_programs,
    trace_reward_reservations, trace_reward_decisions, trace_reward_awards,
    trace_reward_invalidations FROM PUBLIC, trace_reward_runtime;
GRANT USAGE ON SCHEMA public TO trace_reward_guard, trace_reward_runtime;
GRANT SELECT ON trace_reward_operators, trace_reward_programs,
    trace_reward_reservations, trace_reward_decisions, trace_reward_awards,
    trace_reward_invalidations TO trace_reward_guard;
GRANT INSERT ON trace_reward_programs, trace_reward_reservations,
    trace_reward_decisions, trace_reward_awards, trace_reward_invalidations
    TO trace_reward_guard;
GRANT UPDATE (state, evidence_hash, evaluation_hash, submitter_hash,
    submitted_at, terminal_at) ON trace_reward_reservations TO trace_reward_guard;
GRANT EXECUTE ON FUNCTION public.trace_current_tenant_id() TO trace_reward_guard;

CREATE FUNCTION trace_reward_authorize(p_tenant TEXT, p_required_role TEXT)
RETURNS TEXT
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_role TEXT;
    v_super BOOLEAN;
    v_bypass BOOLEAN;
BEGIN
    IF p_tenant IS NULL OR p_tenant = ''
        OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_required_role IS NOT NULL
            AND p_required_role NOT IN ('issuer', 'reviewer') THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;

    SELECT r.rolsuper, r.rolbypassrls
      INTO v_super, v_bypass
      FROM pg_catalog.pg_roles r
     WHERE r.rolname = session_user;
    IF NOT FOUND OR v_super OR v_bypass THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;

    SELECT o.actor_hash, o.operator_role
      INTO v_actor, v_role
      FROM public.trace_reward_operators o
     WHERE o.tenant_id = p_tenant
       AND o.login_role = session_user::NAME;
    IF NOT FOUND OR (p_required_role IS NOT NULL AND v_role <> p_required_role) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_unauthorized';
    END IF;
    RETURN v_actor;
END;
$$;

CREATE FUNCTION trace_reward_terms_valid(p_terms JSONB)
RETURNS BOOLEAN
LANGUAGE plpgsql
IMMUTABLE
SET search_path = pg_catalog
AS $$
DECLARE
    v_expected CONSTANT TEXT[] := ARRAY[
        'schema_version', 'activity_kind', 'definition_hash', 'rubric_hash',
        'evaluator_policy_hash', 'required_evidence_hash', 'rights_hash',
        'challenge_policy_hash', 'sponsor_hash', 'award_units', 'capacity_units',
        'participant_cap_units', 'closes_at', 'reservation_ttl_seconds'
    ];
    v_key TEXT;
    v_award NUMERIC;
    v_capacity NUMERIC;
    v_cap NUMERIC;
    v_ttl NUMERIC;
    v_closes TIMESTAMPTZ;
BEGIN
    IF p_terms IS NULL OR pg_catalog.pg_column_size(p_terms) > 16384
        OR pg_catalog.jsonb_typeof(p_terms) <> 'object'
        OR NOT p_terms ?& v_expected
        OR (SELECT pg_catalog.count(*) FROM pg_catalog.jsonb_object_keys(p_terms))
            <> pg_catalog.array_length(v_expected, 1) THEN
        RETURN FALSE;
    END IF;

    IF pg_catalog.jsonb_typeof(p_terms->'schema_version') <> 'number'
        OR (p_terms->>'schema_version')::NUMERIC <> 1
        OR pg_catalog.jsonb_typeof(p_terms->'activity_kind') <> 'string'
        OR p_terms->>'activity_kind' NOT IN ('mission_completion', 'insight_contribution')
        OR pg_catalog.jsonb_typeof(p_terms->'award_units') <> 'number'
        OR pg_catalog.jsonb_typeof(p_terms->'capacity_units') <> 'number'
        OR pg_catalog.jsonb_typeof(p_terms->'participant_cap_units') <> 'number'
        OR pg_catalog.jsonb_typeof(p_terms->'reservation_ttl_seconds') <> 'number'
        OR pg_catalog.jsonb_typeof(p_terms->'closes_at') <> 'string' THEN
        RETURN FALSE;
    END IF;

    FOREACH v_key IN ARRAY ARRAY[
        'definition_hash', 'rubric_hash', 'evaluator_policy_hash',
        'required_evidence_hash', 'rights_hash', 'challenge_policy_hash', 'sponsor_hash'
    ] LOOP
        IF pg_catalog.jsonb_typeof(p_terms->v_key) <> 'string'
            OR p_terms->>v_key !~ '^sha256:[0-9a-f]{64}$' THEN
            RETURN FALSE;
        END IF;
    END LOOP;

    v_award := (p_terms->>'award_units')::NUMERIC;
    v_capacity := (p_terms->>'capacity_units')::NUMERIC;
    v_cap := (p_terms->>'participant_cap_units')::NUMERIC;
    v_ttl := (p_terms->>'reservation_ttl_seconds')::NUMERIC;
    IF p_terms->>'closes_at'
        !~ '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}([.][0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})$' THEN
        RETURN FALSE;
    END IF;
    v_closes := (p_terms->>'closes_at')::TIMESTAMPTZ;
    IF v_award <> pg_catalog.trunc(v_award) OR v_capacity <> pg_catalog.trunc(v_capacity)
        OR v_cap <> pg_catalog.trunc(v_cap) OR v_ttl <> pg_catalog.trunc(v_ttl)
        OR v_award < 1 OR v_capacity < 1 OR v_cap < 1
        OR v_award > 9223372036854775807 OR v_capacity > 9223372036854775807
        OR v_cap > 9223372036854775807 OR v_award > v_cap OR v_cap > v_capacity
        OR v_ttl < 1 OR v_ttl > 604800 OR v_closes IS NULL
        OR NOT pg_catalog.isfinite(v_closes) THEN
        RETURN FALSE;
    END IF;
    RETURN TRUE;
EXCEPTION WHEN OTHERS THEN
    RETURN FALSE;
END;
$$;

CREATE FUNCTION trace_reward_capacity_used(
    p_tenant TEXT, p_program UUID, p_participant TEXT
)
RETURNS NUMERIC
LANGUAGE SQL
SET search_path = pg_catalog
AS $$
    SELECT COALESCE(pg_catalog.sum(r.award_units::NUMERIC), 0)
      FROM public.trace_reward_reservations r
     WHERE r.tenant_id = p_tenant AND r.program_id = p_program
       AND (p_participant IS NULL OR r.participant_hash = p_participant)
       AND (
           EXISTS(SELECT 1 FROM public.trace_reward_awards a
               WHERE a.tenant_id = r.tenant_id AND a.reservation_id = r.reservation_id)
           OR r.state = 'submitted'
           OR (r.state = 'reserved' AND r.expires_at > pg_catalog.clock_timestamp())
       );
$$;

CREATE FUNCTION trace_reward_program_create(p_tenant TEXT, p_program UUID, p_terms JSONB)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_prior public.trace_reward_programs%ROWTYPE;
    v_closes TIMESTAMPTZ;
    v_hash TEXT;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, 'issuer');
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(p_tenant, 691));
    IF p_program IS NULL OR NOT public.trace_reward_terms_valid(p_terms) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;

    SELECT * INTO v_prior FROM public.trace_reward_programs
     WHERE tenant_id = p_tenant AND program_id = p_program;
    IF FOUND THEN
        IF v_prior.terms IS DISTINCT FROM p_terms THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
        RETURN pg_catalog.jsonb_build_object(
            'program_id', v_prior.program_id, 'terms', v_prior.terms,
            'terms_hash', v_prior.terms_hash, 'creator_hash', v_prior.creator_hash,
            'identity_mode', 'operator_asserted', 'redemption', 'none'
        );
    END IF;

    v_closes := (p_terms->>'closes_at')::TIMESTAMPTZ;
    IF v_closes <= pg_catalog.clock_timestamp() THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_program_closed';
    END IF;
    v_hash := 'sha256:' || pg_catalog.encode(
        pg_catalog.sha256(pg_catalog.convert_to(p_terms::TEXT, 'UTF8')), 'hex'
    );
    INSERT INTO public.trace_reward_programs(
        tenant_id, program_id, terms, terms_hash, creator_hash, sponsor_hash,
        award_units, capacity_units, participant_cap_units, closes_at,
        reservation_ttl_seconds
    ) VALUES (
        p_tenant, p_program, p_terms, v_hash, v_actor, p_terms->>'sponsor_hash',
        ((p_terms->>'award_units')::NUMERIC)::BIGINT,
        ((p_terms->>'capacity_units')::NUMERIC)::BIGINT,
        ((p_terms->>'participant_cap_units')::NUMERIC)::BIGINT, v_closes,
        ((p_terms->>'reservation_ttl_seconds')::NUMERIC)::INTEGER
    );
    RETURN pg_catalog.jsonb_build_object(
        'program_id', p_program, 'terms', p_terms, 'terms_hash', v_hash,
        'creator_hash', v_actor, 'identity_mode', 'operator_asserted',
        'redemption', 'none'
    );
END;
$$;

CREATE FUNCTION trace_reward_program_show(p_tenant TEXT, p_program UUID)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_program public.trace_reward_programs%ROWTYPE;
    v_used NUMERIC;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, NULL);
    IF p_program IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT * INTO v_program FROM public.trace_reward_programs
     WHERE tenant_id = p_tenant AND program_id = p_program;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    v_used := public.trace_reward_capacity_used(p_tenant, p_program, NULL);
    IF v_used < 0 OR v_used > v_program.capacity_units::NUMERIC
        OR v_used > 9223372036854775807 THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_state_conflict';
    END IF;
    RETURN pg_catalog.jsonb_build_object(
        'program_id', v_program.program_id, 'terms', v_program.terms,
        'terms_hash', v_program.terms_hash, 'creator_hash', v_program.creator_hash,
        'capacity_units', v_program.capacity_units,
        'capacity_used_units', v_used::BIGINT,
        'capacity_remaining_units', (v_program.capacity_units::NUMERIC - v_used)::BIGINT,
        'identity_mode', 'operator_asserted', 'redemption', 'none'
    );
END;
$$;

CREATE FUNCTION trace_reward_reserve(
    p_tenant TEXT, p_program UUID, p_reservation UUID, p_participant_hash TEXT,
    p_work_hash TEXT, p_consent_hash TEXT
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_program public.trace_reward_programs%ROWTYPE;
    v_prior public.trace_reward_reservations%ROWTYPE;
    v_used NUMERIC;
    v_participant_used NUMERIC;
    v_expires TIMESTAMPTZ;
    v_now TIMESTAMPTZ;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, 'issuer');
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(p_tenant, 691));
    v_now := pg_catalog.clock_timestamp();
    IF p_program IS NULL OR p_reservation IS NULL
        OR p_participant_hash IS NULL OR p_participant_hash !~ '^sha256:[0-9a-f]{64}$'
        OR p_work_hash IS NULL OR p_work_hash !~ '^sha256:[0-9a-f]{64}$'
        OR p_consent_hash IS NULL OR p_consent_hash !~ '^sha256:[0-9a-f]{64}$' THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;

    SELECT * INTO v_prior FROM public.trace_reward_reservations
     WHERE tenant_id = p_tenant AND reservation_id = p_reservation;
    IF FOUND THEN
        IF v_prior.program_id IS DISTINCT FROM p_program
            OR v_prior.participant_hash IS DISTINCT FROM p_participant_hash
            OR v_prior.work_hash IS DISTINCT FROM p_work_hash
            OR v_prior.consent_hash IS DISTINCT FROM p_consent_hash
            OR v_prior.creator_hash IS DISTINCT FROM v_actor THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
        RETURN pg_catalog.jsonb_build_object(
            'reservation_id', v_prior.reservation_id, 'program_id', v_prior.program_id,
            'participant_hash', v_prior.participant_hash, 'work_hash', v_prior.work_hash,
            'state', CASE WHEN v_prior.state = 'reserved'
                    AND v_prior.expires_at <= v_now
                THEN 'expired' ELSE v_prior.state END,
            'expires_at', v_prior.expires_at, 'award_units', v_prior.award_units
        );
    END IF;

    IF EXISTS(SELECT 1 FROM public.trace_reward_reservations
        WHERE tenant_id = p_tenant AND work_hash = p_work_hash) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_work_duplicate';
    END IF;
    SELECT * INTO v_program FROM public.trace_reward_programs
     WHERE tenant_id = p_tenant AND program_id = p_program;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    IF v_program.closes_at <= v_now THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_program_closed';
    END IF;

    v_used := public.trace_reward_capacity_used(p_tenant, p_program, NULL);
    IF v_used + v_program.award_units::NUMERIC > v_program.capacity_units::NUMERIC THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_capacity_exhausted';
    END IF;
    v_participant_used := public.trace_reward_capacity_used(
        p_tenant, p_program, p_participant_hash
    );
    IF v_participant_used + v_program.award_units::NUMERIC
        > v_program.participant_cap_units::NUMERIC THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_participant_cap';
    END IF;

    v_expires := LEAST(
        v_now + pg_catalog.make_interval(secs => v_program.reservation_ttl_seconds),
        v_program.closes_at
    );
    INSERT INTO public.trace_reward_reservations(
        tenant_id, reservation_id, program_id, participant_hash, work_hash,
        consent_hash, creator_hash, award_units, state, expires_at, created_at
    ) VALUES (
        p_tenant, p_reservation, p_program, p_participant_hash, p_work_hash,
        p_consent_hash, v_actor, v_program.award_units, 'reserved', v_expires, v_now
    );
    RETURN pg_catalog.jsonb_build_object(
        'reservation_id', p_reservation, 'program_id', p_program,
        'participant_hash', p_participant_hash, 'work_hash', p_work_hash,
        'state', 'reserved', 'expires_at', v_expires,
        'award_units', v_program.award_units
    );
END;
$$;

CREATE FUNCTION trace_reward_claim_submit(
    p_tenant TEXT, p_reservation UUID, p_evidence_hash TEXT, p_evaluation_hash TEXT
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_reservation public.trace_reward_reservations%ROWTYPE;
    v_now TIMESTAMPTZ;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, 'issuer');
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(p_tenant, 691));
    v_now := pg_catalog.clock_timestamp();
    IF p_reservation IS NULL
        OR p_evidence_hash IS NULL OR p_evidence_hash !~ '^sha256:[0-9a-f]{64}$'
        OR p_evaluation_hash IS NULL OR p_evaluation_hash !~ '^sha256:[0-9a-f]{64}$' THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT * INTO v_reservation FROM public.trace_reward_reservations
     WHERE tenant_id = p_tenant AND reservation_id = p_reservation;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    IF v_reservation.evidence_hash IS NOT NULL THEN
        IF v_reservation.evidence_hash IS DISTINCT FROM p_evidence_hash
            OR v_reservation.evaluation_hash IS DISTINCT FROM p_evaluation_hash
            OR v_reservation.submitter_hash IS DISTINCT FROM v_actor THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
        RETURN pg_catalog.jsonb_build_object(
            'reservation_id', v_reservation.reservation_id,
            'program_id', v_reservation.program_id, 'state', v_reservation.state,
            'submitted_at', v_reservation.submitted_at,
            'award_units', v_reservation.award_units
        );
    END IF;
    IF v_reservation.state <> 'reserved' THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_state_conflict';
    END IF;
    IF v_reservation.expires_at <= v_now THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_reservation_expired';
    END IF;
    IF EXISTS(SELECT 1 FROM public.trace_reward_invalidations
        WHERE tenant_id = p_tenant AND evidence_hash = p_evidence_hash) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_evidence_invalidated';
    END IF;
    IF EXISTS(SELECT 1 FROM public.trace_reward_reservations
        WHERE tenant_id = p_tenant AND evidence_hash = p_evidence_hash) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_evidence_duplicate';
    END IF;
    UPDATE public.trace_reward_reservations
       SET state = 'submitted', evidence_hash = p_evidence_hash,
           evaluation_hash = p_evaluation_hash, submitter_hash = v_actor,
           submitted_at = v_now
     WHERE tenant_id = p_tenant AND reservation_id = p_reservation
     RETURNING * INTO v_reservation;
    RETURN pg_catalog.jsonb_build_object(
        'reservation_id', v_reservation.reservation_id,
        'program_id', v_reservation.program_id, 'state', v_reservation.state,
        'submitted_at', v_reservation.submitted_at,
        'award_units', v_reservation.award_units
    );
END;
$$;

CREATE FUNCTION trace_reward_review(
    p_tenant TEXT, p_reservation UUID, p_decision UUID,
    p_accept BOOLEAN, p_reason TEXT
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_prior public.trace_reward_decisions%ROWTYPE;
    v_reservation public.trace_reward_reservations%ROWTYPE;
    v_program public.trace_reward_programs%ROWTYPE;
    v_state TEXT;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, 'reviewer');
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(p_tenant, 691));
    IF p_reservation IS NULL OR p_decision IS NULL OR p_accept IS NULL OR p_reason IS NULL
        OR (p_accept AND p_reason NOT IN (
            'completion_verified', 'qualified_negative', 'qualified_inconclusive'))
        OR (NOT p_accept AND p_reason NOT IN (
            'evidence_incomplete', 'rubric_not_met', 'conflict_unresolved',
            'consent_withdrawn')) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT * INTO v_prior FROM public.trace_reward_decisions
     WHERE tenant_id = p_tenant AND decision_id = p_decision;
    IF FOUND THEN
        IF v_prior.decision_kind <> 'review'
            OR v_prior.reservation_id IS DISTINCT FROM p_reservation
            OR v_prior.actor_hash IS DISTINCT FROM v_actor
            OR v_prior.accepted IS DISTINCT FROM p_accept
            OR v_prior.reason IS DISTINCT FROM p_reason THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
        SELECT r.state INTO v_state FROM public.trace_reward_reservations r
         WHERE r.tenant_id = p_tenant AND r.reservation_id = p_reservation;
        RETURN pg_catalog.jsonb_build_object(
            'decision_id', p_decision, 'reservation_id', p_reservation,
            'accepted', p_accept, 'reason', p_reason, 'state', v_state,
            'award_units', CASE WHEN p_accept THEN
                (SELECT a.award_units FROM public.trace_reward_awards a
                  WHERE a.tenant_id = p_tenant AND a.award_id = p_decision)
                ELSE 0 END
        );
    END IF;

    SELECT * INTO v_reservation FROM public.trace_reward_reservations
     WHERE tenant_id = p_tenant AND reservation_id = p_reservation;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    SELECT * INTO v_program FROM public.trace_reward_programs
     WHERE tenant_id = p_tenant AND program_id = v_reservation.program_id;
    IF v_actor IN (v_program.creator_hash, v_program.sponsor_hash,
        v_reservation.participant_hash, v_reservation.creator_hash,
        v_reservation.submitter_hash) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_self_review';
    END IF;
    IF v_reservation.state <> 'submitted' THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_state_conflict';
    END IF;
    IF EXISTS(SELECT 1 FROM public.trace_reward_invalidations
        WHERE tenant_id = p_tenant AND evidence_hash = v_reservation.evidence_hash) THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_evidence_invalidated';
    END IF;

    INSERT INTO public.trace_reward_decisions(
        tenant_id, decision_id, decision_kind, reservation_id,
        actor_hash, accepted, reason
    ) VALUES (
        p_tenant, p_decision, 'review', p_reservation, v_actor, p_accept, p_reason
    );
    IF p_accept THEN
        INSERT INTO public.trace_reward_awards(
            tenant_id, award_id, reservation_id, program_id, participant_hash,
            reviewer_hash, award_units, terms_hash
        ) VALUES (
            p_tenant, p_decision, p_reservation, v_program.program_id,
            v_reservation.participant_hash, v_actor, v_reservation.award_units,
            v_program.terms_hash
        );
        v_state := 'awarded';
    ELSE
        v_state := 'rejected';
    END IF;
    UPDATE public.trace_reward_reservations
       SET state = v_state, terminal_at = pg_catalog.clock_timestamp()
     WHERE tenant_id = p_tenant AND reservation_id = p_reservation;
    RETURN pg_catalog.jsonb_build_object(
        'decision_id', p_decision, 'reservation_id', p_reservation,
        'accepted', p_accept, 'reason', p_reason, 'state', v_state,
        'award_units', CASE WHEN p_accept THEN v_reservation.award_units ELSE 0 END
    );
END;
$$;

CREATE FUNCTION trace_reward_cancel(
    p_tenant TEXT, p_reservation UUID, p_decision UUID
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_prior public.trace_reward_decisions%ROWTYPE;
    v_reservation public.trace_reward_reservations%ROWTYPE;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, 'issuer');
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(p_tenant, 691));
    IF p_reservation IS NULL OR p_decision IS NULL THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT * INTO v_prior FROM public.trace_reward_decisions
     WHERE tenant_id = p_tenant AND decision_id = p_decision;
    IF FOUND THEN
        IF v_prior.decision_kind <> 'cancel'
            OR v_prior.reservation_id IS DISTINCT FROM p_reservation
            OR v_prior.actor_hash IS DISTINCT FROM v_actor THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
        RETURN pg_catalog.jsonb_build_object(
            'decision_id', p_decision, 'reservation_id', p_reservation,
            'state', 'cancelled'
        );
    END IF;
    SELECT * INTO v_reservation FROM public.trace_reward_reservations
     WHERE tenant_id = p_tenant AND reservation_id = p_reservation;
    IF NOT FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
    END IF;
    IF v_reservation.state <> 'reserved' THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_state_conflict';
    END IF;
    INSERT INTO public.trace_reward_decisions(
        tenant_id, decision_id, decision_kind, reservation_id,
        actor_hash, accepted, reason
    ) VALUES (
        p_tenant, p_decision, 'cancel', p_reservation,
        v_actor, NULL, 'issuer_cancelled'
    );
    UPDATE public.trace_reward_reservations
       SET state = 'cancelled', terminal_at = pg_catalog.clock_timestamp()
     WHERE tenant_id = p_tenant AND reservation_id = p_reservation;
    RETURN pg_catalog.jsonb_build_object(
        'decision_id', p_decision, 'reservation_id', p_reservation,
        'state', 'cancelled'
    );
END;
$$;

CREATE FUNCTION trace_reward_invalidate(
    p_tenant TEXT, p_evidence_hash TEXT, p_decision UUID
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_prior public.trace_reward_decisions%ROWTYPE;
    v_existing UUID;
    v_affected UUID;
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, NULL);
    PERFORM pg_catalog.pg_advisory_xact_lock(pg_catalog.hashtextextended(p_tenant, 691));
    IF p_decision IS NULL OR p_evidence_hash IS NULL
        OR p_evidence_hash !~ '^sha256:[0-9a-f]{64}$' THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT * INTO v_prior FROM public.trace_reward_decisions
     WHERE tenant_id = p_tenant AND decision_id = p_decision;
    IF FOUND THEN
        IF v_prior.decision_kind <> 'invalidation'
            OR v_prior.evidence_hash IS DISTINCT FROM p_evidence_hash
            OR v_prior.actor_hash IS DISTINCT FROM v_actor THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_payload_conflict';
        END IF;
        SELECT r.reservation_id INTO v_affected
          FROM public.trace_reward_reservations r
         WHERE r.tenant_id = p_tenant AND r.evidence_hash = p_evidence_hash
           AND r.state = 'invalidated';
        RETURN pg_catalog.jsonb_build_object(
            'decision_id', p_decision, 'evidence_invalidated', TRUE,
            'reservation_id', v_affected
        );
    END IF;
    SELECT i.decision_id INTO v_existing
      FROM public.trace_reward_invalidations i
     WHERE i.tenant_id = p_tenant AND i.evidence_hash = p_evidence_hash;
    IF FOUND THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_state_conflict';
    END IF;
    INSERT INTO public.trace_reward_decisions(
        tenant_id, decision_id, decision_kind, evidence_hash,
        actor_hash, accepted, reason
    ) VALUES (
        p_tenant, p_decision, 'invalidation', p_evidence_hash,
        v_actor, NULL, 'evidence_invalidated'
    );
    INSERT INTO public.trace_reward_invalidations(
        tenant_id, decision_id, evidence_hash, invalidator_hash
    ) VALUES (p_tenant, p_decision, p_evidence_hash, v_actor);
    UPDATE public.trace_reward_reservations
       SET state = 'invalidated', terminal_at = pg_catalog.clock_timestamp()
     WHERE tenant_id = p_tenant AND evidence_hash = p_evidence_hash
       AND state = 'submitted'
     RETURNING reservation_id INTO v_affected;
    RETURN pg_catalog.jsonb_build_object(
        'decision_id', p_decision, 'evidence_invalidated', TRUE,
        'reservation_id', v_affected
    );
END;
$$;

CREATE FUNCTION trace_reward_history(
    p_tenant TEXT, p_participant_hash TEXT, p_limit INTEGER
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_total BIGINT;
    v_entries JSONB;
    v_programs JSONB;
    v_program_ids UUID[];
BEGIN
    v_actor := public.trace_reward_authorize(p_tenant, NULL);
    PERFORM pg_catalog.pg_advisory_xact_lock_shared(pg_catalog.hashtextextended(p_tenant, 691));
    IF p_participant_hash IS NULL
        OR p_participant_hash !~ '^sha256:[0-9a-f]{64}$'
        OR p_limit IS NULL OR p_limit < 1 OR p_limit > 100 THEN
        RAISE EXCEPTION USING MESSAGE = 'reward_request_invalid';
    END IF;
    SELECT pg_catalog.count(*) INTO v_total
      FROM public.trace_reward_reservations r
     WHERE r.tenant_id = p_tenant AND r.participant_hash = p_participant_hash;

    WITH chosen AS (
        SELECT r.*
          FROM public.trace_reward_reservations r
         WHERE r.tenant_id = p_tenant AND r.participant_hash = p_participant_hash
         ORDER BY r.created_at DESC, r.reservation_id DESC
         LIMIT p_limit
    )
    SELECT COALESCE(pg_catalog.jsonb_agg(
        pg_catalog.jsonb_build_object(
            'reservation_id', c.reservation_id,
            'work_hash', c.work_hash,
            'program_id', c.program_id,
            'state', CASE
                WHEN inv.decision_id IS NOT NULL AND a.award_id IS NOT NULL
                    THEN 'awarded_invalidated'
                WHEN c.state = 'reserved' AND c.expires_at <= pg_catalog.clock_timestamp()
                    THEN 'expired'
                ELSE c.state
            END,
            'invalidated', inv.decision_id IS NOT NULL,
            'pinned_units', c.award_units,
            'awarded_units', CASE WHEN a.award_id IS NULL THEN 0 ELSE a.award_units END,
            'terms_hash', p.terms_hash,
            'created_at', c.created_at,
            'decisions', COALESCE(d.items, '[]'::JSONB)
        ) ORDER BY c.created_at DESC, c.reservation_id DESC
    ), '[]'::JSONB),
    COALESCE(pg_catalog.array_agg(DISTINCT c.program_id), ARRAY[]::UUID[])
      INTO v_entries, v_program_ids
      FROM chosen c
      JOIN public.trace_reward_programs p
        ON p.tenant_id = c.tenant_id AND p.program_id = c.program_id
      LEFT JOIN public.trace_reward_awards a
        ON a.tenant_id = c.tenant_id AND a.reservation_id = c.reservation_id
      LEFT JOIN public.trace_reward_invalidations inv
        ON inv.tenant_id = c.tenant_id AND inv.evidence_hash = c.evidence_hash
      LEFT JOIN LATERAL (
          SELECT pg_catalog.jsonb_agg(x.item ORDER BY x.created_at, x.decision_id) AS items
            FROM (
                SELECT d.created_at, d.decision_id,
                    pg_catalog.jsonb_build_object(
                        'decision_id', d.decision_id, 'kind', d.decision_kind,
                        'accepted', d.accepted, 'reason', d.reason,
                        'actor_hash', d.actor_hash, 'created_at', d.created_at
                    ) AS item
                  FROM (
                      SELECT decision.* FROM public.trace_reward_decisions decision
                       WHERE decision.tenant_id = c.tenant_id
                         AND decision.reservation_id = c.reservation_id
                      UNION ALL
                      SELECT decision.* FROM public.trace_reward_decisions decision
                       WHERE decision.tenant_id = c.tenant_id
                         AND decision.decision_kind = 'invalidation'
                         AND decision.evidence_hash = c.evidence_hash
                  ) d
            ) x
      ) d ON TRUE;

    SELECT COALESCE(pg_catalog.jsonb_agg(
        pg_catalog.jsonb_build_object(
            'program_id', totals.program_id,
            'awarded_units', totals.awarded_units
        ) ORDER BY totals.program_id
    ), '[]'::JSONB)
      INTO v_programs
      FROM (
          SELECT visible.program_id,
              COALESCE(pg_catalog.sum(a.award_units::NUMERIC), 0) AS awarded_units
            FROM pg_catalog.unnest(v_program_ids) AS visible(program_id)
            LEFT JOIN public.trace_reward_awards a
              ON a.tenant_id = p_tenant AND a.participant_hash = p_participant_hash
             AND a.program_id = visible.program_id
           GROUP BY visible.program_id
      ) totals;

    RETURN pg_catalog.jsonb_build_object(
        'participant_hash', p_participant_hash,
        'identity_mode', 'operator_asserted', 'redemption', 'none',
        'limit', p_limit, 'truncated', v_total > p_limit,
        'programs_scope', 'visible_entries',
        'programs', v_programs, 'entries', v_entries
    );
END;
$$;

GRANT trace_reward_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_reward_guard;
ALTER FUNCTION trace_reward_authorize(TEXT, TEXT) OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_terms_valid(JSONB) OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_capacity_used(TEXT, UUID, TEXT) OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_program_create(TEXT, UUID, JSONB) OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_program_show(TEXT, UUID) OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_reserve(TEXT, UUID, UUID, TEXT, TEXT, TEXT)
    OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_claim_submit(TEXT, UUID, TEXT, TEXT) OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_review(TEXT, UUID, UUID, BOOLEAN, TEXT)
    OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_cancel(TEXT, UUID, UUID) OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_invalidate(TEXT, TEXT, UUID) OWNER TO trace_reward_guard;
ALTER FUNCTION trace_reward_history(TEXT, TEXT, INTEGER) OWNER TO trace_reward_guard;
REVOKE CREATE ON SCHEMA public FROM trace_reward_guard;

REVOKE ALL ON FUNCTION trace_reward_authorize(TEXT, TEXT) FROM PUBLIC, trace_reward_runtime;
REVOKE ALL ON FUNCTION trace_reward_terms_valid(JSONB) FROM PUBLIC, trace_reward_runtime;
REVOKE ALL ON FUNCTION trace_reward_capacity_used(TEXT, UUID, TEXT)
    FROM PUBLIC, trace_reward_runtime;
REVOKE ALL ON FUNCTION trace_reward_program_create(TEXT, UUID, JSONB) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_reward_program_show(TEXT, UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_reward_reserve(TEXT, UUID, UUID, TEXT, TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_reward_claim_submit(TEXT, UUID, TEXT, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_reward_review(TEXT, UUID, UUID, BOOLEAN, TEXT) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_reward_cancel(TEXT, UUID, UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_reward_invalidate(TEXT, TEXT, UUID) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_reward_history(TEXT, TEXT, INTEGER) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION trace_reward_program_create(TEXT, UUID, JSONB)
    TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION trace_reward_program_show(TEXT, UUID) TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION trace_reward_reserve(TEXT, UUID, UUID, TEXT, TEXT, TEXT)
    TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION trace_reward_claim_submit(TEXT, UUID, TEXT, TEXT)
    TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION trace_reward_review(TEXT, UUID, UUID, BOOLEAN, TEXT)
    TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION trace_reward_cancel(TEXT, UUID, UUID) TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION trace_reward_invalidate(TEXT, TEXT, UUID) TO trace_reward_runtime;
GRANT EXECUTE ON FUNCTION trace_reward_history(TEXT, TEXT, INTEGER) TO trace_reward_runtime;

GRANT trace_reward_runtime TO CURRENT_USER WITH ADMIN OPTION;
REVOKE trace_reward_guard FROM CURRENT_USER;
