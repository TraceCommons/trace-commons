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
--    carries the source's link to it (owner, 2026-11-03: carry only at
--    events, never on every pass).
-- 4. trace_legacy_device_upload_principals: the decision-5 derivation in
--    one place, now used by V146's claim (redefined here) and by
-- 5. trace_legacy_payout_proven_accounts: the accounts a live, signed
--    device proved; every carry reaches only these (owner, 2026-11-03),
--    and trace_legacy_payout_carry_unproven, which the send-time standing
--    reads as a backstop.
-- 6. trace_legacy_payout_release_unproven_carries: the payout pass ends a
--    carry whose account stopped being proven, at send time (owner,
--    2026-11-03).
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
-- account's carries go in every tenant it linked. A close in a legacy
-- tenant with no live tenant link (merges, the reaper) is one probe of
-- V81's unique live-link index and takes no lock. A tenant with a live link
-- always goes through the locked release, so a carry that holds the lock
-- and read the account as open is released when it commits (review of
-- #1879).
CREATE FUNCTION public.trace_legacy_account_close_releases_carries()
RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_tenant_link RECORD;
BEGIN
    IF OLD.closed_at IS NOT NULL OR NEW.closed_at IS NULL THEN
        RETURN NEW;
    END IF;
    IF NEW.tenant_id !~ '^(near-|nearai-)' THEN
        IF EXISTS (SELECT 1 FROM public.trace_legacy_invite_links l
                    WHERE l.legacy_tenant_id = NEW.tenant_id AND l.revoked_at IS NULL) THEN
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

-- 4. The decision-5 derivation, in one place (review of #1879): both forms
-- of the signed-upload principal (`account_trust::device_upload_principal_refs`)
-- of each actor derivable from `p_device` in `p_tenant`: the bare device
-- key, the app's `user_subject_hash(device_key_id)` subject, and each
-- instance enrollment subject of the tenant enrolled before
-- `p_enrolled_before` (NULL: so far). Sorted and distinct. Asked under
-- `p_tenant`'s context (instance enrollments are read through the guard's
-- tenant policy). The claim calls it with NULL (it runs at the link or
-- attestation itself); only V148's one-time backfill passes a bound.
-- Nothing else in the database spells the formula.
CREATE FUNCTION public.trace_legacy_device_upload_principals(
    p_tenant TEXT, p_device TEXT, p_enrolled_before TIMESTAMPTZ
)
RETURNS TEXT[]
LANGUAGE plpgsql STABLE SET search_path = pg_catalog AS $$
DECLARE
    v_subject TEXT;
    v_actor TEXT;
    v_material TEXT;
    v_principals TEXT[] := ARRAY[]::TEXT[];
BEGIN
    FOREACH v_subject IN ARRAY
        ARRAY['', 'sha256:' || encode(sha256(convert_to('user_subject:' || p_device, 'UTF8')), 'hex')]
        || (SELECT coalesce(array_agg(DISTINCT e.user_subject_hash), ARRAY[]::TEXT[])
              FROM public.trace_instance_enrollments e
             WHERE e.tenant_id = p_tenant
               AND (p_enrolled_before IS NULL OR e.created_at < p_enrolled_before))
    LOOP
        IF v_subject = '' THEN
            v_actor := p_device;
        ELSE
            v_actor := 'instance:' || p_tenant || ':' || p_device || ':user:' || v_subject;
        END IF;
        v_material := 'signed:' || p_tenant || ':' || v_actor;
        v_principals := v_principals || ARRAY[
            'principal_sha256:' || encode(sha256(convert_to(
                '12:signed_claim:' || octet_length(v_material)::TEXT || ':' || v_material, 'UTF8')), 'hex'),
            'principal_sha256:' || encode(sha256(convert_to(v_material, 'UTF8')), 'hex')
        ];
    END LOOP;
    RETURN (SELECT array_agg(DISTINCT x ORDER BY x) FROM unnest(v_principals) AS x);
END $$;
REVOKE ALL ON FUNCTION public.trace_legacy_device_upload_principals(TEXT, TEXT, TIMESTAMPTZ)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_legacy_device_upload_principals(TEXT, TEXT, TIMESTAMPTZ)
    TO trace_legacy_payout_carry_guard;
GRANT SELECT (created_at) ON trace_instance_enrollments TO trace_legacy_payout_carry_guard;

-- What each claim linked: the principals derived for one device under one
-- tenant link, at that device's first claim (the link, or its V91
-- attestation). The row lives in the legacy tenant; the tenant link is
-- named by hash only (`trace_legacy_link_hash`). Owner decision,
-- 2026-11-03: only subjects enrolled by the link or attestation prove, and
-- this record is that snapshot, taken by the database itself.
CREATE TABLE trace_legacy_link_claimed_principals (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    legacy_link_hash TEXT NOT NULL CHECK (legacy_link_hash ~ '^sha256:[0-9a-f]{64}$'),
    device_key_id TEXT NOT NULL CHECK (device_key_id ~ '^sha256:[0-9a-f]{64}$'),
    principal_ref TEXT NOT NULL CHECK (principal_ref ~ '^principal_sha256:[0-9a-f]{64}$'),
    claimed_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, legacy_link_hash, device_key_id, principal_ref),
    CHECK (tenant_id !~ '^(near-|nearai-)')
);
ALTER TABLE trace_legacy_link_claimed_principals ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_link_claimed_principals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_legacy_link_claimed_principals;
CREATE POLICY trace_corpus_tenant_isolation ON trace_legacy_link_claimed_principals
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
GRANT SELECT, INSERT ON trace_legacy_link_claimed_principals TO trace_legacy_payout_carry_guard;

-- V146's decision-5 claim, now recording its snapshot and reusing it.
CREATE OR REPLACE FUNCTION public.trace_legacy_link_claim_device_principals(
    p_account_tenant TEXT, p_account UUID, p_legacy_tenant TEXT, p_device_key_id TEXT
) RETURNS INTEGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_principals TEXT[];
    v_link_id UUID;
    v_link_hash TEXT;
    v_free TEXT[];
    v_principal TEXT;
    v_account UUID;
    v_linked INTEGER := 0;
BEGIN
    IF p_account_tenant IS NULL
        OR p_account_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_account_tenant !~ '^(near-|nearai-)[0-9a-f]{64}$'
        OR p_account IS NULL OR p_legacy_tenant IS NULL
        OR p_legacy_tenant ~ '^(near-|nearai-)'
        OR p_device_key_id IS NULL OR p_device_key_id !~ '^sha256:[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'legacy_link_claim_refused' USING ERRCODE = '42501';
    END IF;
    SELECT l.link_id INTO v_link_id
      FROM public.trace_legacy_invite_links l
     WHERE l.tenant_id = p_account_tenant AND l.account_id = p_account
       AND l.legacy_tenant_id = p_legacy_tenant AND l.revoked_at IS NULL
       AND (l.device_key_id = p_device_key_id
            OR EXISTS (SELECT 1 FROM public.trace_legacy_invite_link_devices d
                        WHERE d.link_id = l.link_id AND d.device_key_id = p_device_key_id));
    IF NOT FOUND THEN
        RETURN 0;
    END IF;
    v_link_hash := public.trace_legacy_link_hash(v_link_id);

    PERFORM set_config('trace_commons.trace_tenant_id', p_legacy_tenant, true);
    IF EXISTS (SELECT 1 FROM public.trace_legacy_invite_pooled_tenants p
                WHERE p.tenant_id = p_legacy_tenant)
       OR NOT EXISTS (SELECT 1 FROM public.device_keys d
                       WHERE d.tenant_id = p_legacy_tenant AND d.device_key_id = p_device_key_id
                         AND d.revoked_at IS NULL AND d.onboarding_origin = 'invite')
       OR EXISTS (SELECT 1 FROM public.trace_near_provisioned_devices n
                   WHERE n.tenant_id = p_legacy_tenant AND n.device_key_id = p_device_key_id) THEN
        PERFORM set_config('trace_commons.trace_tenant_id', p_account_tenant, true);
        RETURN 0;
    END IF;

    -- The claim's snapshot (review of #1879). The first claim for this
    -- device under this tenant link derives its principals and records
    -- them; every later claim (a repeated link or attestation) uses that
    -- record, so a subject enrolled after it never joins. The proof set is
    -- a join over these records, so the claim and the proof cannot differ,
    -- and no clock is compared with another.
    SELECT array_agg(c.principal_ref ORDER BY c.principal_ref) INTO v_principals
      FROM public.trace_legacy_link_claimed_principals c
     WHERE c.tenant_id = p_legacy_tenant AND c.legacy_link_hash = v_link_hash
       AND c.device_key_id = p_device_key_id;
    IF v_principals IS NULL THEN
        v_principals := public.trace_legacy_device_upload_principals(
            p_legacy_tenant, p_device_key_id, NULL);
        INSERT INTO public.trace_legacy_link_claimed_principals
            (tenant_id, legacy_link_hash, device_key_id, principal_ref)
        SELECT p_legacy_tenant, v_link_hash, p_device_key_id, x FROM unnest(v_principals) AS x
        ON CONFLICT DO NOTHING;
    END IF;
    -- The same lock both V135 namespace-guard triggers take per principal.
    FOREACH v_principal IN ARRAY v_principals LOOP
        PERFORM pg_advisory_xact_lock(hashtextextended(p_legacy_tenant || ':' || v_principal, 135));
    END LOOP;

    SELECT p.account_id INTO v_account
      FROM public.trace_account_principals p
      JOIN public.trace_accounts a ON a.tenant_id = p.tenant_id AND a.account_id = p.account_id
     WHERE p.tenant_id = p_legacy_tenant AND p.principal_ref = ANY (v_principals)
       AND p.unlinked_at IS NULL AND a.closed_at IS NULL
     ORDER BY p.linked_at, p.principal_ref
     LIMIT 1;
    SELECT coalesce(array_agg(x), ARRAY[]::TEXT[]) INTO v_free
      FROM unnest(v_principals) AS x
     WHERE NOT EXISTS (SELECT 1 FROM public.trace_account_principals p
                        WHERE p.tenant_id = p_legacy_tenant AND p.principal_ref = x)
       AND NOT EXISTS (SELECT 1 FROM public.trace_device_upload_principals u
                        WHERE u.tenant_id = p_legacy_tenant AND u.upload_principal_ref = x);
    IF cardinality(v_free) = 0 THEN
        PERFORM set_config('trace_commons.trace_tenant_id', p_account_tenant, true);
        RETURN 0;
    END IF;
    IF v_account IS NULL THEN
        v_account := gen_random_uuid();
        INSERT INTO public.trace_accounts (tenant_id, account_id) VALUES (p_legacy_tenant, v_account);
    END IF;
    FOREACH v_principal IN ARRAY v_free LOOP
        INSERT INTO public.trace_account_principals (tenant_id, account_id, principal_ref)
        VALUES (p_legacy_tenant, v_account, v_principal)
        ON CONFLICT DO NOTHING;
        IF FOUND THEN
            v_linked := v_linked + 1;
        END IF;
    END LOOP;
    IF v_linked > 0 THEN
        INSERT INTO public.trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
        VALUES (p_legacy_tenant, 'legacy_link_principals_claimed',
                public.trace_payout_account_hash(v_account),
                'success',
                jsonb_build_object('principals', v_linked, 'device_key_id', p_device_key_id));
    END IF;
    PERFORM set_config('trace_commons.trace_tenant_id', p_account_tenant, true);
    RETURN v_linked;
END $$;

-- 5. The accounts a signed device proved (owner, 2026-11-03, #1879): open
-- accounts of the legacy tenant holding a principal that a claim under the
-- live tenant link recorded for a device still live in the tenant
-- (unrevoked, `invite` origin, not NEAR-provisioned; the claim's own device
-- test). A join over the claims' records: what a claim linked is exactly
-- what proves. An account opened by any other device or subject waits.
-- Asked under the legacy tenant's own context; refused under any other.
CREATE FUNCTION public.trace_legacy_payout_proven_accounts(p_legacy_tenant TEXT)
RETURNS SETOF UUID
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_link_id UUID;
BEGIN
    IF p_legacy_tenant IS NULL
        OR p_legacy_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION 'legacy_payout_proven_refused' USING ERRCODE = '42501';
    END IF;
    IF p_legacy_tenant ~ '^(near-|nearai-)' THEN
        RETURN;
    END IF;
    SELECT l.link_id INTO v_link_id
      FROM public.trace_legacy_invite_links l
     WHERE l.legacy_tenant_id = p_legacy_tenant AND l.revoked_at IS NULL;
    IF NOT FOUND THEN
        RETURN;
    END IF;
    RETURN QUERY
        SELECT DISTINCT p.account_id
          FROM public.trace_legacy_link_claimed_principals c
          JOIN public.trace_account_principals p
            ON p.tenant_id = c.tenant_id AND p.principal_ref = c.principal_ref
           AND p.unlinked_at IS NULL
          JOIN public.trace_accounts a
            ON a.tenant_id = p.tenant_id AND a.account_id = p.account_id AND a.closed_at IS NULL
         WHERE c.tenant_id = p_legacy_tenant
           AND c.legacy_link_hash = public.trace_legacy_link_hash(v_link_id)
           AND EXISTS (SELECT 1 FROM public.device_keys d
                        WHERE d.tenant_id = c.tenant_id AND d.device_key_id = c.device_key_id
                          AND d.revoked_at IS NULL AND d.onboarding_origin = 'invite')
           AND NOT EXISTS (SELECT 1 FROM public.trace_near_provisioned_devices n
                            WHERE n.tenant_id = c.tenant_id AND n.device_key_id = c.device_key_id);
END $$;
ALTER FUNCTION public.trace_legacy_payout_proven_accounts(TEXT)
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_payout_proven_accounts(TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_legacy_payout_proven_accounts(TEXT)
    TO trace_ingest_runtime;

-- Links made before V148 have no record: their claims ran under V146. Record
-- them once now, for each live tenant link's device and attested devices,
-- with the subjects enrolled by the link or attestation as nearly as the
-- stored times allow (whole seconds from the app host; one-time only, every
-- later claim records itself). Run as the guard, then dropped.
GRANT SELECT (linked_at) ON trace_legacy_invite_links TO trace_legacy_payout_carry_guard;
GRANT SELECT (attested_at) ON trace_legacy_invite_link_devices TO trace_legacy_payout_carry_guard;
CREATE FUNCTION public.trace_legacy_link_claimed_principals_backfill()
RETURNS INTEGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_prior TEXT := current_setting('trace_commons.trace_tenant_id', true);
    v_link RECORD;
    v_device RECORD;
    v_principals TEXT[];
    v_rows INTEGER := 0;
    v_count INTEGER;
BEGIN
    FOR v_link IN
        SELECT l.link_id, l.tenant_id, l.legacy_tenant_id, l.device_key_id, l.linked_at
          FROM public.trace_legacy_invite_links l
         WHERE l.revoked_at IS NULL AND l.legacy_tenant_id !~ '^(near-|nearai-)'
    LOOP
        PERFORM set_config('trace_commons.trace_tenant_id', v_link.tenant_id, true);
        FOR v_device IN
            SELECT v_link.device_key_id AS device_key_id, v_link.linked_at AS at
            UNION
            SELECT d.device_key_id, d.attested_at FROM public.trace_legacy_invite_link_devices d
             WHERE d.link_id = v_link.link_id
        LOOP
            PERFORM set_config('trace_commons.trace_tenant_id', v_link.legacy_tenant_id, true);
            v_principals := public.trace_legacy_device_upload_principals(
                v_link.legacy_tenant_id, v_device.device_key_id, to_timestamp(v_device.at + 1));
            INSERT INTO public.trace_legacy_link_claimed_principals
                (tenant_id, legacy_link_hash, device_key_id, principal_ref)
            SELECT v_link.legacy_tenant_id, public.trace_legacy_link_hash(v_link.link_id),
                   v_device.device_key_id, x
              FROM unnest(v_principals) AS x
            ON CONFLICT DO NOTHING;
            GET DIAGNOSTICS v_count = ROW_COUNT;
            v_rows := v_rows + v_count;
            PERFORM set_config('trace_commons.trace_tenant_id', v_link.tenant_id, true);
        END LOOP;
    END LOOP;
    PERFORM set_config('trace_commons.trace_tenant_id', coalesce(v_prior, ''), true);
    RETURN v_rows;
END $$;
ALTER FUNCTION public.trace_legacy_link_claimed_principals_backfill()
    OWNER TO trace_legacy_payout_carry_guard;
SELECT public.trace_legacy_link_claimed_principals_backfill();
DROP FUNCTION public.trace_legacy_link_claimed_principals_backfill();

-- The send-time half of the proof rule (review of #1879): TRUE when the
-- account holds a live carry but is no longer proven (its proving principal
-- was unlinked, or its device revoked). The payout store's account standing
-- reads it beside `trace_legacy_payout_carry_invalid` before every send, so
-- such a carry is held at once and released at the next carry event.
CREATE FUNCTION public.trace_legacy_payout_carry_unproven(p_tenant TEXT, p_account UUID)
RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
BEGIN
    IF p_tenant IS NULL OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION 'legacy_payout_carry_check_refused' USING ERRCODE = '42501';
    END IF;
    IF p_tenant ~ '^(near-|nearai-)' OR NOT EXISTS (
        SELECT 1 FROM public.trace_legacy_payout_carries c
          JOIN public.trace_payout_links l ON l.tenant_id = c.tenant_id AND l.link_id = c.link_id
         WHERE c.tenant_id = p_tenant AND c.account_id = p_account AND c.released_at IS NULL
           AND l.state IN ('waiting', 'active')) THEN
        RETURN FALSE;
    END IF;
    RETURN NOT EXISTS (
        SELECT 1 FROM public.trace_legacy_payout_proven_accounts(p_tenant) AS proven(account_id)
         WHERE proven.account_id = p_account);
END $$;
ALTER FUNCTION public.trace_legacy_payout_carry_unproven(TEXT, UUID)
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_payout_carry_unproven(TEXT, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_legacy_payout_carry_unproven(TEXT, UUID)
    TO trace_ingest_runtime;

-- 6. Release at send time (owner, 2026-11-03, V148:444). The payout pass
-- calls this for a legacy tenant before it reads that tenant's links: every
-- live carry on an account that is no longer proven (its proving principal
-- unlinked, its device revoked) is released, an active one with an unlink
-- request and a waiting one closed at zero, audited hash-only as
-- `legacy_carry_unproven`. The account's standing then no longer holds it,
-- so its other streams resume. This only ends carries; the pass never
-- creates one. Under the tenant-link lock, so it cannot interleave with a
-- carry or a claim. Asked under the tenant's own context; refused under
-- any other. Returns how many carries it released.
CREATE FUNCTION public.trace_legacy_payout_release_unproven_carries(p_tenant TEXT)
RETURNS INTEGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_account UUID;
    v_released INTEGER := 0;
BEGIN
    IF p_tenant IS NULL OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION 'legacy_payout_release_refused' USING ERRCODE = '42501';
    END IF;
    IF p_tenant ~ '^(near-|nearai-)' OR NOT EXISTS (
        SELECT 1 FROM public.trace_legacy_payout_carries c
         WHERE c.tenant_id = p_tenant AND c.released_at IS NULL) THEN
        RETURN 0;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || p_tenant, 0));
    FOR v_account IN
        SELECT DISTINCT c.account_id FROM public.trace_legacy_payout_carries c
         WHERE c.tenant_id = p_tenant AND c.released_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM public.trace_legacy_payout_proven_accounts(p_tenant)
                                       AS proven(account_id)
                            WHERE proven.account_id = c.account_id)
         ORDER BY c.account_id
    LOOP
        v_released := v_released + public.trace_legacy_payout_release_tenant_carries(
            p_tenant, NULL, v_account, 'legacy_carry_unproven');
    END LOOP;
    RETURN v_released;
END $$;
ALTER FUNCTION public.trace_legacy_payout_release_unproven_carries(TEXT)
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_payout_release_unproven_carries(TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_legacy_payout_release_unproven_carries(TEXT)
    TO trace_ingest_runtime;

REVOKE CREATE ON SCHEMA public FROM trace_legacy_payout_carry_guard;
REVOKE trace_legacy_payout_carry_guard FROM CURRENT_USER;
