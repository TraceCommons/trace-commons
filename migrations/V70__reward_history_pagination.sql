-- Persist pilot identity provenance and allow complete reward history traversal.
-- Preserve V69 for databases that already installed the operator pilot.

ALTER TABLE public.trace_reward_programs
    ADD COLUMN identity_mode TEXT NOT NULL DEFAULT 'operator_asserted'
        CHECK (identity_mode = 'operator_asserted');

DROP INDEX public.trace_reward_reservations_participant_idx;
CREATE INDEX trace_reward_reservations_participant_idx
    ON public.trace_reward_reservations(
        tenant_id, participant_hash, created_at DESC, reservation_id DESC
    );

GRANT trace_reward_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_reward_guard;

CREATE OR REPLACE FUNCTION trace_reward_program_create(p_tenant TEXT, p_program UUID, p_terms JSONB)
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
            'identity_mode', v_prior.identity_mode, 'redemption', 'none'
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
    ) RETURNING * INTO v_prior;
    RETURN pg_catalog.jsonb_build_object(
        'program_id', v_prior.program_id, 'terms', v_prior.terms,
        'terms_hash', v_prior.terms_hash, 'creator_hash', v_prior.creator_hash,
        'identity_mode', v_prior.identity_mode,
        'redemption', 'none'
    );
END;
$$;

CREATE OR REPLACE FUNCTION trace_reward_program_show(p_tenant TEXT, p_program UUID)
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
        'identity_mode', v_program.identity_mode, 'redemption', 'none'
    );
END;
$$;

-- The default cursor preserves three-argument SQL calls without an ambiguous overload.
DROP FUNCTION public.trace_reward_history(TEXT, TEXT, INTEGER);

CREATE FUNCTION trace_reward_history(
    p_tenant TEXT, p_participant_hash TEXT, p_limit INTEGER,
    p_before UUID DEFAULT NULL
)
RETURNS JSONB
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = pg_catalog
AS $$
DECLARE
    v_actor TEXT;
    v_before_created_at TIMESTAMPTZ;
    v_entries JSONB;
    v_has_more BOOLEAN;
    v_next_cursor UUID;
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
    IF p_before IS NOT NULL THEN
        SELECT r.created_at INTO v_before_created_at
          FROM public.trace_reward_reservations r
         WHERE r.tenant_id = p_tenant
           AND r.participant_hash = p_participant_hash
           AND r.reservation_id = p_before;
        IF NOT FOUND THEN
            RAISE EXCEPTION USING MESSAGE = 'reward_not_found';
        END IF;
    END IF;

    WITH candidates AS MATERIALIZED (
        SELECT r.*
          FROM public.trace_reward_reservations r
         WHERE r.tenant_id = p_tenant AND r.participant_hash = p_participant_hash
           AND (p_before IS NULL OR (r.created_at, r.reservation_id)
               < (v_before_created_at, p_before))
         ORDER BY r.created_at DESC, r.reservation_id DESC
         LIMIT p_limit + 1
    ), chosen AS MATERIALIZED (
        SELECT candidate.*
          FROM candidates candidate
         ORDER BY candidate.created_at DESC, candidate.reservation_id DESC
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
    COALESCE(pg_catalog.array_agg(DISTINCT c.program_id), ARRAY[]::UUID[]),
    (SELECT pg_catalog.count(*) > p_limit FROM candidates),
    CASE WHEN (SELECT pg_catalog.count(*) > p_limit FROM candidates) THEN
        (SELECT last_visible.reservation_id
           FROM chosen last_visible
          ORDER BY last_visible.created_at ASC, last_visible.reservation_id ASC
          LIMIT 1)
        ELSE NULL
    END
      INTO v_entries, v_program_ids, v_has_more, v_next_cursor
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
            'identity_mode', totals.identity_mode,
            'awarded_units', totals.awarded_units
        ) ORDER BY totals.program_id
    ), '[]'::JSONB)
      INTO v_programs
      FROM (
          SELECT visible.program_id, p.identity_mode,
              COALESCE(pg_catalog.sum(a.award_units::NUMERIC), 0) AS awarded_units
            FROM pg_catalog.unnest(v_program_ids) AS visible(program_id)
            JOIN public.trace_reward_programs p
              ON p.tenant_id = p_tenant AND p.program_id = visible.program_id
            LEFT JOIN public.trace_reward_awards a
              ON a.tenant_id = p_tenant AND a.participant_hash = p_participant_hash
             AND a.program_id = visible.program_id
           GROUP BY visible.program_id, p.identity_mode
      ) totals;

    RETURN pg_catalog.jsonb_build_object(
        'participant_hash', p_participant_hash,
        'identity_mode', 'operator_asserted', 'redemption', 'none',
        'limit', p_limit, 'truncated', v_has_more,
        'next_cursor', v_next_cursor,
        'programs_scope', 'visible_entries',
        'programs', v_programs, 'entries', v_entries
    );
END;
$$;

ALTER FUNCTION public.trace_reward_history(TEXT, TEXT, INTEGER, UUID)
    OWNER TO trace_reward_guard;
REVOKE ALL ON FUNCTION public.trace_reward_history(TEXT, TEXT, INTEGER, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_reward_history(TEXT, TEXT, INTEGER, UUID)
    TO trace_reward_runtime;

REVOKE CREATE ON SCHEMA public FROM trace_reward_guard;
REVOKE trace_reward_guard FROM CURRENT_USER;
