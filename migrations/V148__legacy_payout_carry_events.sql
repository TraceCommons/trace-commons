-- Legacy payout carries follow their events (#1872, #1873; legacy account
-- payout migration spec, section 13).
--
-- V146 released a tenant's carries only when its tenant link was revoked,
-- or at the NEAR AI account's next confirm. Anything else that made a carry
-- invalid (a pooled mark, a closed account, a deleted tenant link) left it
-- held by the send-time check until then, pausing the legacy account's
-- other streams with it. Here each of those events releases the carries it
-- invalidates, in its own transaction, the way revocation already does. The
-- send-time check (`trace_legacy_payout_carry_invalid`) stays as the
-- backstop, and still holds a carry while its source account is on a
-- credit hold, which is temporary and so is not a release.
--
-- 1. trace_legacy_payout_release_tenant_carries: releases live carries in
--    one legacy tenant (all of them, one tenant link's, or one account's),
--    under that tenant's context and the tenant link's lock.
-- 2. Triggers: a pooled mark, an account closing (a NEAR AI account's
--    carries in every tenant it linked, or a legacy account's own), and a
--    tenant link deleted rather than revoked.
-- 3. trace_legacy_payout_source_account (#1873): which open NEAR AI account
--    a legacy tenant is linked to, so opening an account in that tenant
--    carries the source's link to it in the same transaction (owner,
--    2026-11-03: carry only at events, never on every pass).
--
-- Every function is owned by V146's NOLOGIN, NOBYPASSRLS guard and reaches
-- a legacy tenant only by setting the tenant context, so forced RLS applies
-- throughout. Audit rows are hash-only: each release names its link by
-- `trace_payout_link_hash` and its actor by the carried link's own id.

GRANT trace_legacy_payout_carry_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id) ON trace_tenants TO trace_legacy_payout_carry_guard;

-- 1. The shared release. `p_legacy_link_hash` and `p_account` narrow it; NULL
-- means every live carry in the tenant. The caller's tenant context is
-- restored before returning.
CREATE FUNCTION public.trace_legacy_payout_release_tenant_carries(
    p_legacy_tenant TEXT, p_legacy_link_hash TEXT, p_account UUID, p_label TEXT
) RETURNS INTEGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_prior TEXT := current_setting('trace_commons.trace_tenant_id', true);
    v_link UUID;
    v_released INTEGER := 0;
BEGIN
    IF p_legacy_tenant IS NULL OR p_legacy_tenant ~ '^(near-|nearai-)'
        OR p_label IS NULL OR p_label !~ '^[a-z0-9_.-]{1,64}$' THEN
        RETURN 0;
    END IF;
    -- The tenant link's own lock, as the carry and the revocation take it.
    PERFORM pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || p_legacy_tenant, 0));
    PERFORM set_config('trace_commons.trace_tenant_id', p_legacy_tenant, true);
    -- A tenant being deleted (its tenant links cascade from it) has nothing
    -- left to release and must not be written to (review of #1879).
    IF NOT EXISTS (SELECT 1 FROM public.trace_tenants t WHERE t.tenant_id = p_legacy_tenant) THEN
        PERFORM set_config('trace_commons.trace_tenant_id', coalesce(v_prior, ''), true);
        RETURN 0;
    END IF;
    FOR v_link IN
        SELECT c.link_id FROM public.trace_legacy_payout_carries c
         WHERE c.tenant_id = p_legacy_tenant AND c.released_at IS NULL
           AND (p_legacy_link_hash IS NULL OR c.legacy_link_hash = p_legacy_link_hash)
           AND (p_account IS NULL OR c.account_id = p_account)
         ORDER BY c.link_id
    LOOP
        PERFORM public.trace_legacy_payout_release_carry(
            v_link,
            'sha256:' || encode(sha256(convert_to(
                'legacy-payout-carry-actor-v1:' || v_link::TEXT, 'UTF8')), 'hex'),
            p_label);
        v_released := v_released + 1;
    END LOOP;
    PERFORM set_config('trace_commons.trace_tenant_id', coalesce(v_prior, ''), true);
    RETURN v_released;
END $$;
ALTER FUNCTION public.trace_legacy_payout_release_tenant_carries(TEXT, TEXT, UUID, TEXT)
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_payout_release_tenant_carries(TEXT, TEXT, UUID, TEXT)
    FROM PUBLIC;

-- 2a. A tenant marked pooled is never paid (spec section 7).
CREATE FUNCTION public.trace_legacy_pooled_tenant_releases_carries()
RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    PERFORM public.trace_legacy_payout_release_tenant_carries(
        NEW.tenant_id, NULL, NULL, 'legacy_tenant_pooled');
    RETURN NEW;
END $$;
ALTER FUNCTION public.trace_legacy_pooled_tenant_releases_carries()
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_pooled_tenant_releases_carries() FROM PUBLIC;
CREATE TRIGGER trace_legacy_pooled_tenant_releases_carries
    AFTER INSERT ON trace_legacy_invite_pooled_tenants
    FOR EACH ROW EXECUTE FUNCTION public.trace_legacy_pooled_tenant_releases_carries();

-- 2b. An account closing. A legacy account's own carries go; a NEAR AI
-- account's carries go in every tenant it linked. The common close (an
-- account with no carry and no tenant link: merges, the reaper) reads one
-- index and returns without taking a lock (review of #1879). A carry that
-- races the close is released by the carry's own closed-account check.
CREATE FUNCTION public.trace_legacy_account_close_releases_carries()
RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_prior TEXT := current_setting('trace_commons.trace_tenant_id', true);
    v_carried BOOLEAN;
    v_tenant_link RECORD;
BEGIN
    IF OLD.closed_at IS NOT NULL OR NEW.closed_at IS NULL THEN
        RETURN NEW;
    END IF;
    IF NEW.tenant_id !~ '^(near-|nearai-)' THEN
        PERFORM set_config('trace_commons.trace_tenant_id', NEW.tenant_id, true);
        v_carried := EXISTS (
            SELECT 1 FROM public.trace_legacy_payout_carries c
             WHERE c.tenant_id = NEW.tenant_id AND c.account_id = NEW.account_id
               AND c.released_at IS NULL);
        PERFORM set_config('trace_commons.trace_tenant_id', coalesce(v_prior, ''), true);
        IF v_carried THEN
            PERFORM public.trace_legacy_payout_release_tenant_carries(
                NEW.tenant_id, NULL, NEW.account_id, 'legacy_account_closed');
        END IF;
        RETURN NEW;
    END IF;
    FOR v_tenant_link IN
        SELECT l.link_id, l.legacy_tenant_id FROM public.trace_legacy_invite_links l
         WHERE l.tenant_id = NEW.tenant_id AND l.account_id = NEW.account_id
           AND l.revoked_at IS NULL
         ORDER BY l.legacy_tenant_id
    LOOP
        PERFORM public.trace_legacy_payout_release_tenant_carries(
            v_tenant_link.legacy_tenant_id,
            public.trace_legacy_link_hash(v_tenant_link.link_id),
            NULL,
            'legacy_source_account_closed');
    END LOOP;
    RETURN NEW;
END $$;
ALTER FUNCTION public.trace_legacy_account_close_releases_carries()
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_account_close_releases_carries() FROM PUBLIC;
CREATE TRIGGER trace_legacy_account_close_releases_carries
    AFTER UPDATE OF closed_at ON trace_accounts
    FOR EACH ROW EXECUTE FUNCTION public.trace_legacy_account_close_releases_carries();

-- 2c. A tenant link deleted rather than revoked (an operator delete, or a
-- cascade) releases its carries as a revocation does.
CREATE FUNCTION public.trace_legacy_invite_link_delete_releases_carries()
RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    IF OLD.revoked_at IS NULL THEN
        PERFORM public.trace_legacy_payout_release_tenant_carries(
            OLD.legacy_tenant_id, public.trace_legacy_link_hash(OLD.link_id), NULL,
            'legacy_tenant_link_deleted');
    END IF;
    RETURN OLD;
END $$;
ALTER FUNCTION public.trace_legacy_invite_link_delete_releases_carries()
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_invite_link_delete_releases_carries() FROM PUBLIC;
CREATE TRIGGER trace_legacy_invite_link_delete_releases_carries
    AFTER DELETE ON trace_legacy_invite_links
    FOR EACH ROW EXECUTE FUNCTION public.trace_legacy_invite_link_delete_releases_carries();

-- 3. The open NEAR AI account a legacy tenant is linked to (#1873). Asked
-- under the legacy tenant's own context by the transaction that opens an
-- account there; refused under any other. No row when the tenant has no
-- live tenant link or the account is closed.
CREATE FUNCTION public.trace_legacy_payout_source_account(p_legacy_tenant TEXT)
RETURNS TABLE (tenant_id TEXT, account_id UUID)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_prior TEXT := current_setting('trace_commons.trace_tenant_id', true);
    v_link RECORD;
BEGIN
    IF p_legacy_tenant IS NULL
        OR p_legacy_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION 'legacy_payout_source_refused' USING ERRCODE = '42501';
    END IF;
    IF p_legacy_tenant ~ '^(near-|nearai-)' THEN
        RETURN;
    END IF;
    SELECT l.tenant_id, l.account_id INTO v_link
      FROM public.trace_legacy_invite_links l
     WHERE l.legacy_tenant_id = p_legacy_tenant AND l.revoked_at IS NULL;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    PERFORM set_config('trace_commons.trace_tenant_id', v_link.tenant_id, true);
    IF EXISTS (SELECT 1 FROM public.trace_accounts a
                WHERE a.tenant_id = v_link.tenant_id AND a.account_id = v_link.account_id
                  AND a.closed_at IS NULL) THEN
        PERFORM set_config('trace_commons.trace_tenant_id', coalesce(v_prior, ''), true);
        RETURN QUERY SELECT v_link.tenant_id, v_link.account_id;
        RETURN;
    END IF;
    PERFORM set_config('trace_commons.trace_tenant_id', coalesce(v_prior, ''), true);
END $$;
ALTER FUNCTION public.trace_legacy_payout_source_account(TEXT)
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_payout_source_account(TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_legacy_payout_source_account(TEXT)
    TO trace_ingest_runtime;

REVOKE CREATE ON SCHEMA public FROM trace_legacy_payout_carry_guard;
REVOKE trace_legacy_payout_carry_guard FROM CURRENT_USER;
