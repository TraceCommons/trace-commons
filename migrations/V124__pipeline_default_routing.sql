-- V124: the two cross-tenant reads of pipeline default routing
-- (docs/superpowers/specs/2026-10-10-pipeline-default-routing-design.md).
--
-- When an operator arms default routing, ingest routes every tenant that has
-- no pipeline_tenant_routing row to the pipeline, and keeps a short-lived
-- list of the tenants routed to the pipeline (or contained) so that its
-- worker and its upload path serve tenants routed after start. Both lists
-- span tenants, and every table here has forced row-level security keyed on
-- trace_current_tenant_id(). So the two reads go through SECURITY DEFINER
-- functions owned by a NOLOGIN, NOBYPASSRLS guard role whose only privileges
-- are column-scoped SELECTs and a SELECT-only policy that names only it: the
-- V85 enumeration pattern (trace_account_trust_enumeration_guard).
--
-- Each function returns tenant ids (and, for the routed list, the routing
-- state) and nothing else: no actor, reason, record id, or bundle.
--
-- EXECUTE goes directly to trace_ingest_runtime (the V90 convention), so no
-- deployment needs a hand-made grant before default routing can run.
--
-- Idempotent, and appliable by a non-superuser CREATEROLE migrator, as V85
-- and V101 are.

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V124: trace_ingest_runtime is missing; V90 creates it';
    END IF;
END $$;

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles
                    WHERE rolname = 'trace_pipeline_routing_enumeration_guard') THEN
        CREATE ROLE trace_pipeline_routing_enumeration_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
-- NOLOGIN only: changing BYPASSRLS needs a superuser, and the role is
-- created NOBYPASSRLS above (V85 does the same).
ALTER ROLE trace_pipeline_routing_enumeration_guard NOLOGIN;
GRANT trace_pipeline_routing_enumeration_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_pipeline_routing_enumeration_guard;
GRANT SELECT (tenant_id) ON trace_tenants
    TO trace_pipeline_routing_enumeration_guard;
GRANT SELECT (tenant_id, routing_state) ON pipeline_tenant_routing
    TO trace_pipeline_routing_enumeration_guard;

-- SELECT-only, and only for the guard: no other role's reads change.
DROP POLICY IF EXISTS trace_pipeline_routing_enumeration_tenants ON trace_tenants;
CREATE POLICY trace_pipeline_routing_enumeration_tenants ON trace_tenants
    FOR SELECT TO trace_pipeline_routing_enumeration_guard USING (TRUE);
DROP POLICY IF EXISTS trace_pipeline_routing_enumeration_routing ON pipeline_tenant_routing;
CREATE POLICY trace_pipeline_routing_enumeration_routing ON pipeline_tenant_routing
    FOR SELECT TO trace_pipeline_routing_enumeration_guard USING (TRUE);

DROP FUNCTION IF EXISTS trace_pipeline_unrouted_tenants(TEXT, BIGINT);
DROP FUNCTION IF EXISTS trace_pipeline_routed_tenants(TEXT, BIGINT);

-- One page of the tenants with no routing row, in byte order after p_after
-- (NULL: from the start), at most 1000.
CREATE FUNCTION trace_pipeline_unrouted_tenants(p_after TEXT, p_limit BIGINT)
RETURNS TABLE(tenant_id TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
    SELECT t.tenant_id FROM public.trace_tenants t
     WHERE NOT EXISTS (
            SELECT 1 FROM public.pipeline_tenant_routing r
             WHERE r.tenant_id = t.tenant_id)
       AND (p_after IS NULL OR t.tenant_id COLLATE "C" > p_after COLLATE "C")
     ORDER BY t.tenant_id COLLATE "C"
     LIMIT LEAST(GREATEST(COALESCE(p_limit, 0), 0), 1000);
$$;

-- One page of the tenants whose routing row is `pipeline` or `contained`,
-- with that state, in the same order and bound.
CREATE FUNCTION trace_pipeline_routed_tenants(p_after TEXT, p_limit BIGINT)
RETURNS TABLE(tenant_id TEXT, routing_state TEXT)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = pg_catalog AS $$
    SELECT r.tenant_id, r.routing_state FROM public.pipeline_tenant_routing r
     WHERE r.routing_state IN ('pipeline', 'contained')
       AND (p_after IS NULL OR r.tenant_id COLLATE "C" > p_after COLLATE "C")
     ORDER BY r.tenant_id COLLATE "C"
     LIMIT LEAST(GREATEST(COALESCE(p_limit, 0), 0), 1000);
$$;

GRANT CREATE ON SCHEMA public TO trace_pipeline_routing_enumeration_guard;
ALTER FUNCTION trace_pipeline_unrouted_tenants(TEXT, BIGINT)
    OWNER TO trace_pipeline_routing_enumeration_guard;
ALTER FUNCTION trace_pipeline_routed_tenants(TEXT, BIGINT)
    OWNER TO trace_pipeline_routing_enumeration_guard;
REVOKE CREATE ON SCHEMA public FROM trace_pipeline_routing_enumeration_guard;
REVOKE ALL ON FUNCTION trace_pipeline_unrouted_tenants(TEXT, BIGINT) FROM PUBLIC;
REVOKE ALL ON FUNCTION trace_pipeline_routed_tenants(TEXT, BIGINT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_pipeline_unrouted_tenants(TEXT, BIGINT),
    trace_pipeline_routed_tenants(TEXT, BIGINT)
    TO trace_ingest_runtime;
REVOKE trace_pipeline_routing_enumeration_guard FROM CURRENT_USER;
