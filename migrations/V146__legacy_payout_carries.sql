-- Legacy account payout migration
-- (docs/superpowers/specs/2026-11-03-legacy-account-payout-migration-design.md,
-- owner decisions of 2026-11-03).
--
-- A NEAR AI account's payout link is carried to every open account of each
-- legacy tenant it has linked through V81, so legacy credit is paid from its
-- own tenant to the organization the NEAR AI login proved. Nothing here
-- writes an anchor or a NEAR AI device into a legacy tenant, and nothing
-- changes readiness.
--
-- 1. trace_legacy_payout_carries: which links in a legacy tenant are
--    carries, of which source link, under which tenant link. Hash-only
--    references to the other tenant's rows.
-- 2. trace_legacy_payout_release_carry: releases one carry in the current
--    tenant context (an active link gets an unlink request; a waiting one,
--    which never paid, closes at zero). Shared by the payout store and the
--    revocation trigger.
-- 3. A carried waiting link that never sent may close at final_paid 0.
--    Before V146 a waiting link could only go active, so a released carry
--    would have paid once before its unlink took effect. Every other link
--    keeps V132's rule.
-- 4. trace_legacy_link_claim_device_principals (decision 5, option A): links
--    the signing device's signed-upload principals to an account in the
--    legacy tenant, so its past credit is claimable.
-- 5. A trigger on trace_legacy_invite_links: revoking a tenant link releases
--    every carry under it, in the revoking transaction.
-- 6. trace_link_legacy_instance_device (decision 3): the V81 link for an
--    instance-enrolled device, which has no invite row. No trust change.

-- The guard that owns the cross-tenant functions. NOLOGIN, NOBYPASSRLS:
-- it reaches a legacy tenant only by setting the tenant context inside its
-- own functions, so forced RLS still applies to every row it touches.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_legacy_payout_carry_guard') THEN
        CREATE ROLE trace_legacy_payout_carry_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_legacy_payout_carry_guard NOLOGIN;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles
                WHERE rolname = 'trace_legacy_payout_carry_guard'
                  AND (rolsuper OR rolbypassrls OR rolcanlogin)) THEN
        RAISE EXCEPTION 'V146: the carry guard is privileged or can log in';
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_legacy_payout_carry_guard;

-- The three hashes V146 matches across tenants, each in one place
-- (#1874). They mirror the Rust `db::payout_link_id_hash`,
-- `db::payout_account_id_hash` and `db::payout_legacy_link_hash`;
-- `legacy_payout_carry_pg` asserts the SQL and Rust spellings agree, that
-- no V146 function spells a formula itself, and that these stay STABLE,
-- inlinable and closed to PUBLIC.
--
-- STABLE, not IMMUTABLE: `convert_to` is STABLE. No `SET search_path`, so
-- the planner can inline them (a `SET` clause forbids it, and costs a GUC
-- save and restore per call on the send-time check); instead every name in
-- the body is schema-qualified, operators and casts included, so the
-- caller's search_path cannot change what they compute. Review of #1878.
CREATE FUNCTION public.trace_payout_link_hash(p_link_id UUID) RETURNS TEXT
LANGUAGE sql STABLE STRICT PARALLEL SAFE AS $$
    SELECT 'sha256:' OPERATOR(pg_catalog.||) pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
        'payout-link-v1:' OPERATOR(pg_catalog.||) CAST(p_link_id AS pg_catalog.text), 'UTF8')), 'hex')
$$;
CREATE FUNCTION public.trace_payout_account_hash(p_account_id UUID) RETURNS TEXT
LANGUAGE sql STABLE STRICT PARALLEL SAFE AS $$
    SELECT 'sha256:' OPERATOR(pg_catalog.||) pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
        'payout-account-v1:' OPERATOR(pg_catalog.||) CAST(p_account_id AS pg_catalog.text), 'UTF8')), 'hex')
$$;
CREATE FUNCTION public.trace_legacy_link_hash(p_link_id UUID) RETURNS TEXT
LANGUAGE sql STABLE STRICT PARALLEL SAFE AS $$
    SELECT 'sha256:' OPERATOR(pg_catalog.||) pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
        'legacy-invite-link-v1:' OPERATOR(pg_catalog.||) CAST(p_link_id AS pg_catalog.text), 'UTF8')), 'hex')
$$;
-- Closed to PUBLIC like every other V146 function. Granted to the roles
-- that run their callers: the payout runtime (the invoker release), the
-- carry guard (its triggers, claim and send-time check), and the invite
-- link guard (the instance link's audit row).
REVOKE ALL ON FUNCTION public.trace_payout_link_hash(UUID), public.trace_payout_account_hash(UUID),
    public.trace_legacy_link_hash(UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_payout_link_hash(UUID), public.trace_payout_account_hash(UUID),
    public.trace_legacy_link_hash(UUID)
    TO trace_ingest_runtime, trace_legacy_payout_carry_guard, trace_legacy_invite_link_guard;

-- 1. Carries. The row lives in the legacy tenant beside the link it names.
-- The source link and the tenant link live in the NEAR AI account's tenant,
-- so they are named by hash only: `source_link_hash` is
-- `trace_payout_link_hash` (the payout audit's link hash), and
-- `legacy_link_hash` is `trace_legacy_link_hash`.
CREATE TABLE trace_legacy_payout_carries (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    link_id UUID NOT NULL,
    legacy_link_hash TEXT NOT NULL CHECK (legacy_link_hash ~ '^sha256:[0-9a-f]{64}$'),
    source_link_hash TEXT NOT NULL CHECK (source_link_hash ~ '^sha256:[0-9a-f]{64}$'),
    carried_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    released_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, link_id),
    CHECK (tenant_id !~ '^(near-|nearai-)'),
    FOREIGN KEY (tenant_id, link_id, account_id, rail_id)
        REFERENCES trace_payout_links(tenant_id, link_id, account_id, rail_id)
);
-- One live carry of one source per account and rail.
CREATE UNIQUE INDEX trace_legacy_payout_carries_one_live
    ON trace_legacy_payout_carries (tenant_id, account_id, rail_id, source_link_hash)
    WHERE released_at IS NULL;
CREATE INDEX trace_legacy_payout_carries_legacy_link
    ON trace_legacy_payout_carries (tenant_id, legacy_link_hash) WHERE released_at IS NULL;
ALTER TABLE trace_legacy_payout_carries ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_payout_carries FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_legacy_payout_carries;
CREATE POLICY trace_corpus_tenant_isolation ON trace_legacy_payout_carries
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The payout runtime carries inside a confirm's transaction, switching the
-- tenant context to each linked legacy tenant. It reads the account's own
-- tenant links and pooled markers, and writes carries.
GRANT SELECT, INSERT ON trace_legacy_payout_carries TO trace_ingest_runtime;
GRANT UPDATE (released_at) ON trace_legacy_payout_carries TO trace_ingest_runtime;
GRANT SELECT (tenant_id, account_id, legacy_tenant_id, link_id, revoked_at)
    ON trace_legacy_invite_links TO trace_ingest_runtime;
GRANT SELECT ON trace_legacy_invite_pooled_tenants TO trace_ingest_runtime;

-- 3. A carried waiting link that never sent may close at zero.
CREATE OR REPLACE FUNCTION trace_payout_links_guard_update()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF OLD.state = 'closed' THEN
        RAISE EXCEPTION 'trace_payout_links: a closed link is final'
            USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.state IS DISTINCT FROM OLD.state AND NOT (
           (OLD.state = 'waiting' AND NEW.state = 'active')
        OR (OLD.state = 'active' AND NEW.state = 'closing')
        OR (OLD.state = 'closing' AND NEW.state = 'closed')
        -- V146: a carried waiting link (and only a carried one) closes at
        -- zero, and only if it never had a line (checked below). Every
        -- other link keeps V132's rule: waiting moves only to active.
        OR (OLD.state = 'waiting' AND NEW.state = 'closed' AND NEW.final_paid = 0
            AND EXISTS (SELECT 1 FROM public.trace_legacy_payout_carries c
                         WHERE c.tenant_id = OLD.tenant_id AND c.link_id = OLD.link_id))
    ) THEN
        RAISE EXCEPTION 'trace_payout_links: a link moves only waiting, active, closing, closed'
            USING ERRCODE = 'check_violation';
    END IF;
    IF OLD.state = 'waiting' AND NEW.state = 'closed' AND EXISTS (
        SELECT 1 FROM public.trace_payout_lines
         WHERE tenant_id = OLD.tenant_id AND link_id = OLD.link_id
    ) THEN
        RAISE EXCEPTION 'trace_payout_links: a waiting link with a line does not close'
            USING ERRCODE = 'check_violation';
    END IF;
    -- Table names are schema-qualified since V146: the carry guard's
    -- functions run with search_path pg_catalog.
    -- A plpgsql statement takes a fresh snapshot, so a pending line that
    -- committed while this UPDATE waited on the link row is seen here (the
    -- store's line insert holds the link row FOR SHARE until it commits).
    -- The rail pass's lock serialises these already; this is the backstop.
    IF NEW.state = 'closed' AND EXISTS (
        SELECT 1 FROM public.trace_payout_lines
         WHERE tenant_id = OLD.tenant_id AND link_id = OLD.link_id AND state = 'pending'
    ) THEN
        RAISE EXCEPTION 'trace_payout_links: a link with a pending line does not close'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

-- 2. Release one carry in the current tenant context. SECURITY INVOKER: the
-- payout runtime calls it with its own grants, the revocation trigger as the
-- guard. Hash-only audit rows; the actor is passed in as a hash.
CREATE FUNCTION public.trace_legacy_payout_release_carry(
    p_link_id UUID, p_actor_ref_hash TEXT, p_label TEXT
) RETURNS TEXT
LANGUAGE plpgsql SECURITY INVOKER SET search_path = pg_catalog AS $$
DECLARE
    v_tenant TEXT := public.trace_current_tenant_id();
    v_link RECORD;
    v_outcome TEXT := 'unchanged';
BEGIN
    UPDATE public.trace_legacy_payout_carries c
       SET released_at = clock_timestamp()
     WHERE c.tenant_id = v_tenant AND c.link_id = p_link_id AND c.released_at IS NULL;
    IF NOT FOUND THEN
        RETURN 'not_carried';
    END IF;
    SELECT l.state, l.account_id, l.rail_id, l.unlink_requested_at INTO v_link
      FROM public.trace_payout_links l
     WHERE l.tenant_id = v_tenant AND l.link_id = p_link_id
     FOR UPDATE;
    IF v_link.state = 'waiting' THEN
        UPDATE public.trace_payout_links
           SET state = 'closed', final_paid = 0, version = version + 1,
               updated_at = clock_timestamp()
         WHERE tenant_id = v_tenant AND link_id = p_link_id;
        v_outcome := 'closed';
        INSERT INTO public.trace_payout_audit
            (tenant_id, audit_id, rail_id, action, actor_ref_hash, account_id_hash,
             link_id_hash, prior_state, new_state, label)
        VALUES (v_tenant, gen_random_uuid(), v_link.rail_id, 'link_closed', p_actor_ref_hash,
                public.trace_payout_account_hash(v_link.account_id),
                public.trace_payout_link_hash(p_link_id),
                'waiting', 'closed', p_label);
    ELSIF v_link.state = 'active' AND v_link.unlink_requested_at IS NULL THEN
        UPDATE public.trace_payout_links
           SET unlink_requested_at = clock_timestamp(), version = version + 1,
               updated_at = clock_timestamp()
         WHERE tenant_id = v_tenant AND link_id = p_link_id;
        v_outcome := 'unlink_requested';
        INSERT INTO public.trace_payout_audit
            (tenant_id, audit_id, rail_id, action, actor_ref_hash, account_id_hash,
             link_id_hash, prior_state, new_state, label)
        VALUES (v_tenant, gen_random_uuid(), v_link.rail_id, 'unlink_requested', p_actor_ref_hash,
                public.trace_payout_account_hash(v_link.account_id),
                public.trace_payout_link_hash(p_link_id),
                'active', 'active', p_label);
    END IF;
    RETURN v_outcome;
END $$;
REVOKE ALL ON FUNCTION public.trace_legacy_payout_release_carry(UUID, TEXT, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_legacy_payout_release_carry(UUID, TEXT, TEXT)
    TO trace_ingest_runtime, trace_legacy_payout_carry_guard;

-- What the guard touches, and only that. Forced RLS applies: every write is
-- made under the legacy tenant's own context.
GRANT SELECT (tenant_id, account_id, legacy_tenant_id, link_id, device_key_id, revoked_at)
    ON trace_legacy_invite_links TO trace_legacy_payout_carry_guard;
GRANT SELECT (link_id, device_key_id) ON trace_legacy_invite_link_devices
    TO trace_legacy_payout_carry_guard;
GRANT SELECT ON trace_legacy_invite_pooled_tenants TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id, device_key_id, revoked_at, onboarding_origin)
    ON device_keys TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id, device_key_id) ON trace_near_provisioned_devices
    TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id, upload_principal_ref) ON trace_device_upload_principals
    TO trace_legacy_payout_carry_guard;
GRANT SELECT, INSERT ON trace_accounts TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id, user_subject_hash) ON trace_instance_enrollments
    TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id, credit_account_ref, released_at) ON trace_credit_holds
    TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id, account_id, rail_id, cleared_at) ON trace_payout_holds
    TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id, account_id, credit_account_hash) ON trace_payout_accruals
    TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id, device_principal_ref) ON trace_device_upload_principals
    TO trace_legacy_payout_carry_guard;
GRANT SELECT ON trace_account_owned_principals TO trace_legacy_payout_carry_guard;
-- Instance enrollments are isolated by instance subject, not tenant; the
-- guard reads one tenant's rows under that tenant's context only.
CREATE POLICY legacy_payout_carry_instance_enrollments ON trace_instance_enrollments
    FOR SELECT TO trace_legacy_payout_carry_guard
    USING (tenant_id = trace_current_tenant_id());
-- The send-time check finds a legacy tenant's link from the legacy side.
CREATE POLICY legacy_payout_carry_tenant_links ON trace_legacy_invite_links
    FOR SELECT TO trace_legacy_payout_carry_guard USING (TRUE);
GRANT SELECT, INSERT ON trace_account_principals TO trace_legacy_payout_carry_guard;
GRANT INSERT ON trace_account_audit TO trace_legacy_payout_carry_guard;
GRANT USAGE ON SEQUENCE trace_account_audit_audit_sequence_seq TO trace_legacy_payout_carry_guard;
GRANT SELECT, UPDATE (released_at) ON trace_legacy_payout_carries TO trace_legacy_payout_carry_guard;
GRANT SELECT ON trace_payout_links TO trace_legacy_payout_carry_guard;
GRANT UPDATE (state, final_paid, unlink_requested_at, version, updated_at)
    ON trace_payout_links TO trace_legacy_payout_carry_guard;
GRANT SELECT (tenant_id, link_id, state) ON trace_payout_lines TO trace_legacy_payout_carry_guard;
GRANT INSERT ON trace_payout_audit TO trace_legacy_payout_carry_guard;

GRANT trace_legacy_payout_carry_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_legacy_payout_carry_guard;

-- 4. Decision 5, option A. Called by the link store in the link's own
-- transaction, under the NEAR AI account's tenant context, after a link or
-- an attestation. The device must be the live link's own device or one
-- attested under it. Links both forms of the signed-upload principal of the
-- device's actors (`account_trust::device_upload_principal_refs`) to one
-- account in the legacy tenant: the live account one of them already
-- belongs to, or a new one. A principal that was unlinked stays unlinked.
-- The actors are the ones the server can derive from the device key alone:
-- the bare key (device-JWT uploads), the app's per-user subject when it is
-- the key itself (`user_subject_hash(device_key_id)`, every invite device
-- without a contributor label), and each instance enrollment's subject for
-- this tenant. Any other subject cannot be derived and its credit stays
-- unowned (the spec's known limit). Returns how many principals it linked.
CREATE FUNCTION public.trace_legacy_link_claim_device_principals(
    p_account_tenant TEXT, p_account UUID, p_legacy_tenant TEXT, p_device_key_id TEXT
) RETURNS INTEGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_material TEXT;
    v_actor TEXT;
    v_subjects TEXT[];
    v_subject TEXT;
    v_principals TEXT[] := ARRAY[]::TEXT[];
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
    IF NOT EXISTS (
        SELECT 1 FROM public.trace_legacy_invite_links l
         WHERE l.tenant_id = p_account_tenant AND l.account_id = p_account
           AND l.legacy_tenant_id = p_legacy_tenant AND l.revoked_at IS NULL
           AND (l.device_key_id = p_device_key_id
                OR EXISTS (SELECT 1 FROM public.trace_legacy_invite_link_devices d
                            WHERE d.link_id = l.link_id AND d.device_key_id = p_device_key_id))
    ) THEN
        RETURN 0;
    END IF;

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

    -- account_trust::device_claim_actor and device_upload_principal_refs,
    -- for every subject derivable here ('' is the bare actor).
    SELECT ARRAY['',
                 'sha256:' || encode(sha256(convert_to('user_subject:' || p_device_key_id, 'UTF8')), 'hex')]
           || coalesce(array_agg(DISTINCT e.user_subject_hash), ARRAY[]::TEXT[])
      INTO v_subjects
      FROM public.trace_instance_enrollments e
     WHERE e.tenant_id = p_legacy_tenant;
    FOREACH v_subject IN ARRAY v_subjects LOOP
        IF v_subject = '' THEN
            v_actor := p_device_key_id;
        ELSE
            v_actor := 'instance:' || p_legacy_tenant || ':' || p_device_key_id || ':user:' || v_subject;
        END IF;
        v_material := 'signed:' || p_legacy_tenant || ':' || v_actor;
        v_principals := v_principals || ARRAY[
            'principal_sha256:' || encode(sha256(convert_to(
                '12:signed_claim:' || octet_length(v_material)::TEXT || ':' || v_material, 'UTF8')), 'hex'),
            'principal_sha256:' || encode(sha256(convert_to(v_material, 'UTF8')), 'hex')
        ];
    END LOOP;
    SELECT array_agg(DISTINCT x ORDER BY x) INTO v_principals FROM unnest(v_principals) AS x;
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
ALTER FUNCTION public.trace_legacy_link_claim_device_principals(TEXT, UUID, TEXT, TEXT)
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_link_claim_device_principals(TEXT, UUID, TEXT, TEXT)
    FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_legacy_link_claim_device_principals(TEXT, UUID, TEXT, TEXT)
    TO trace_account_invite_runtime;

-- 5. Revoking a tenant link releases its carries, in the revoking
-- transaction. SECURITY DEFINER: the operator revokes under the NEAR AI
-- account's tenant context, and an invoker trigger would match no legacy
-- row under forced RLS and do nothing.
CREATE FUNCTION public.trace_legacy_invite_links_release_carries()
RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_prior TEXT := current_setting('trace_commons.trace_tenant_id', true);
    v_hash TEXT;
    v_link UUID;
BEGIN
    IF OLD.revoked_at IS NOT NULL OR NEW.revoked_at IS NULL THEN
        RETURN NEW;
    END IF;
    -- The tenant link's own lock, which the carry takes too: a carry that
    -- committed first is seen below, and one that waits re-reads
    -- `revoked_at` and carries nothing.
    PERFORM pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || NEW.legacy_tenant_id, 0));
    v_hash := public.trace_legacy_link_hash(NEW.link_id);
    PERFORM set_config('trace_commons.trace_tenant_id', NEW.legacy_tenant_id, true);
    FOR v_link IN
        SELECT c.link_id FROM public.trace_legacy_payout_carries c
         WHERE c.tenant_id = NEW.legacy_tenant_id AND c.legacy_link_hash = v_hash
           AND c.released_at IS NULL
         ORDER BY c.link_id
    LOOP
        PERFORM public.trace_legacy_payout_release_carry(
            v_link,
            'sha256:' || encode(sha256(convert_to('legacy-invite-link-revocation', 'UTF8')), 'hex'),
            'legacy_tenant_link_released');
    END LOOP;
    PERFORM set_config('trace_commons.trace_tenant_id', coalesce(v_prior, ''), true);
    RETURN NEW;
END $$;
ALTER FUNCTION public.trace_legacy_invite_links_release_carries()
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_invite_links_release_carries() FROM PUBLIC;
CREATE TRIGGER trace_legacy_invite_links_release_carries
    AFTER UPDATE OF revoked_at ON trace_legacy_invite_links
    FOR EACH ROW EXECUTE FUNCTION public.trace_legacy_invite_links_release_carries();

-- The account a settlement key names, in every spelling
-- `db::settlement_key_account_id` accepts: `account:` in any case, then a
-- UUID hyphenated or compact, braced or not, optionally `urn:uuid:`.
-- NULL for anything else, including a bare principal.
CREATE FUNCTION public.trace_legacy_payout_settlement_key_account(p_ref TEXT)
RETURNS UUID
LANGUAGE plpgsql IMMUTABLE SET search_path = pg_catalog AS $$
DECLARE
    v_raw TEXT;
BEGIN
    IF p_ref IS NULL OR lower(left(p_ref, 8)) <> 'account:' THEN
        RETURN NULL;
    END IF;
    v_raw := btrim(substr(p_ref, 9));
    IF lower(left(v_raw, 9)) = 'urn:uuid:' THEN
        v_raw := substr(v_raw, 10);
    END IF;
    RETURN v_raw::UUID;
EXCEPTION WHEN invalid_text_representation THEN
    RETURN NULL;
END $$;
REVOKE ALL ON FUNCTION public.trace_legacy_payout_settlement_key_account(TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_legacy_payout_settlement_key_account(TEXT)
    TO trace_legacy_payout_carry_guard;

-- 7. The send-time check (spec section 7): whether a legacy account holds a
-- live carry that should not be paid. Called by the payout store's account
-- standing, under the legacy tenant's context, before every send; a TRUE
-- answer holds the account. Invalid when the tenant is pooled; when no
-- unrevoked tenant link names the carry (revoked, deleted, or relinked);
-- when the NEAR AI account is closed or has an open account credit hold;
-- or when the source link is no longer live there. FALSE, cheaply, for an
-- account with no live carry.
CREATE FUNCTION public.trace_legacy_payout_carry_invalid(p_tenant TEXT, p_account UUID)
RETURNS BOOLEAN
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_carry RECORD;
    v_link RECORD;
    v_invalid BOOLEAN := FALSE;
BEGIN
    -- Misuse holds rather than pays: a caller under another tenant's
    -- context is refused, never answered "valid".
    IF p_tenant IS NULL OR p_tenant IS DISTINCT FROM public.trace_current_tenant_id() THEN
        RAISE EXCEPTION 'legacy_payout_carry_check_refused' USING ERRCODE = '42501';
    END IF;
    IF p_tenant ~ '^(near-|nearai-)' THEN
        RETURN FALSE;
    END IF;
    FOR v_carry IN
        SELECT c.rail_id, c.legacy_link_hash, c.source_link_hash
          FROM public.trace_legacy_payout_carries c
          JOIN public.trace_payout_links l ON l.tenant_id = c.tenant_id AND l.link_id = c.link_id
         WHERE c.tenant_id = p_tenant AND c.account_id = p_account AND c.released_at IS NULL
           AND l.state IN ('waiting', 'active')
    LOOP
        IF EXISTS (SELECT 1 FROM public.trace_legacy_invite_pooled_tenants p
                    WHERE p.tenant_id = p_tenant) THEN
            RETURN TRUE;
        END IF;
        SELECT l.tenant_id, l.account_id INTO v_link
          FROM public.trace_legacy_invite_links l
         WHERE l.legacy_tenant_id = p_tenant AND l.revoked_at IS NULL
           AND public.trace_legacy_link_hash(l.link_id) = v_carry.legacy_link_hash;
        IF NOT FOUND THEN
            RETURN TRUE;
        END IF;
        PERFORM set_config('trace_commons.trace_tenant_id', v_link.tenant_id, true);
        -- The source account's standing, as `payout_account_standing`
        -- reads it for that account (review of #1865): closed; an open
        -- credit hold on its `account:` key in any spelling settlement
        -- accepts, on a principal it owns, or matched through its accruals;
        -- or an open payout hold on this rail.
        v_invalid := NOT EXISTS (
                SELECT 1 FROM public.trace_accounts a
                 WHERE a.tenant_id = v_link.tenant_id AND a.account_id = v_link.account_id
                   AND a.closed_at IS NULL)
            OR EXISTS (
                SELECT 1 FROM public.trace_credit_holds h
                 WHERE h.tenant_id = v_link.tenant_id AND h.released_at IS NULL
                   AND (public.trace_legacy_payout_settlement_key_account(h.credit_account_ref)
                            = v_link.account_id
                        OR h.credit_account_ref IN (
                            SELECT p.principal_ref FROM public.trace_account_owned_principals p
                             WHERE p.tenant_id = v_link.tenant_id
                               AND p.account_id = v_link.account_id)
                        OR EXISTS (
                            SELECT 1 FROM public.trace_payout_accruals accrued
                             WHERE accrued.tenant_id = v_link.tenant_id
                               AND accrued.account_id = v_link.account_id
                               AND accrued.credit_account_hash = 'sha256:' ||
                                   encode(sha256(convert_to(h.credit_account_ref, 'UTF8')), 'hex'))))
            OR EXISTS (
                SELECT 1 FROM public.trace_payout_holds ph
                 WHERE ph.tenant_id = v_link.tenant_id AND ph.account_id = v_link.account_id
                   AND ph.rail_id = v_carry.rail_id AND ph.cleared_at IS NULL)
            OR NOT EXISTS (
                SELECT 1 FROM public.trace_payout_links s
                 WHERE s.tenant_id = v_link.tenant_id AND s.account_id = v_link.account_id
                   AND s.rail_id = v_carry.rail_id
                   AND s.state IN ('waiting', 'active') AND s.unlink_requested_at IS NULL
                   AND public.trace_payout_link_hash(s.link_id) = v_carry.source_link_hash);
        PERFORM set_config('trace_commons.trace_tenant_id', p_tenant, true);
        IF v_invalid THEN
            RETURN TRUE;
        END IF;
    END LOOP;
    RETURN FALSE;
END $$;
ALTER FUNCTION public.trace_legacy_payout_carry_invalid(TEXT, UUID)
    OWNER TO trace_legacy_payout_carry_guard;
REVOKE ALL ON FUNCTION public.trace_legacy_payout_carry_invalid(TEXT, UUID) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_legacy_payout_carry_invalid(TEXT, UUID)
    TO trace_ingest_runtime;

REVOKE CREATE ON SCHEMA public FROM trace_legacy_payout_carry_guard;
REVOKE trace_legacy_payout_carry_guard FROM CURRENT_USER;

-- 6. Decision 3: the V81 link for an instance-enrolled device. Called by the
-- link store in the same transaction, only after `trace_link_legacy_invite`
-- answered `device_not_eligible` (which spent the challenge). The same
-- statement, key check, pooled check and one-link-per-tenant rule; the
-- device must be an `invite`-origin device of the tenant with NO
-- onboarding_invites row for its subject hash (an instance enrollment). No
-- invite grant and no trust change: there is no invite to carry over.
--
-- Labels: challenge_invalid, account_ineligible, device_not_eligible,
-- tenant_pooled, tenant_claimed, already_linked, linked. Returns the
-- account's current authority (or 'none') and trust version (or 0).
GRANT SELECT (created_at) ON device_keys TO trace_legacy_invite_link_guard;
GRANT trace_legacy_invite_link_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_legacy_invite_link_guard;
CREATE FUNCTION public.trace_link_legacy_instance_device(
    p_account_tenant TEXT, p_account UUID, p_legacy_tenant TEXT,
    p_device_key_id TEXT, p_public_key BYTEA, p_invite_hash TEXT,
    p_nonce TEXT, p_issued_at BIGINT, p_link_id UUID, p_linked_at BIGINT,
    p_device_signature TEXT, p_server_kid TEXT, p_server_signature TEXT
) RETURNS TABLE (
    outcome TEXT, link_id UUID, device_key_id TEXT, invite_subject_hash TEXT,
    nonce TEXT, issued_at BIGINT, device_signature TEXT, linked_at BIGINT,
    server_kid TEXT, server_signature TEXT, trust_version BIGINT, authority TEXT
)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_stored_key TEXT;
    v_key_matches BOOLEAN;
    v_existing public.trace_legacy_invite_links%ROWTYPE;
    v_version BIGINT;
    v_authority TEXT;
BEGIN
    IF p_account_tenant IS NULL
        OR p_account_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_account_tenant !~ '^(near-|nearai-)[0-9a-f]{64}$'
        OR p_account IS NULL OR p_legacy_tenant IS NULL
        OR p_legacy_tenant ~ '^(near-|nearai-)'
        OR p_nonce IS NULL OR p_nonce !~ '^[0-9a-f]{64}$' THEN
        RETURN QUERY SELECT 'account_ineligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT,
            NULL::BIGINT, NULL::TEXT;
        RETURN;
    END IF;

    -- The challenge was spent by trace_link_legacy_invite in this very
    -- transaction and backs nothing else; claimed here for this device.
    UPDATE public.trace_legacy_invite_link_challenges c
       SET attested_device_key_id = p_device_key_id
     WHERE c.tenant_id = p_account_tenant AND c.account_id = p_account
       AND c.nonce_hash = 'sha256:' || encode(sha256(convert_to(p_nonce, 'UTF8')), 'hex')
       AND c.issued_at = p_issued_at
       AND c.consumed_at IS NOT NULL
       AND c.xmin = pg_catalog.pg_current_xact_id()::xid
       AND c.attested_device_key_id IS NULL
       AND c.expires_at > extract(epoch FROM clock_timestamp())::BIGINT;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'challenge_invalid'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT,
            NULL::BIGINT, NULL::TEXT;
        RETURN;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM public.trace_accounts a
          JOIN public.trace_near_account_anchors n
            ON n.tenant_id = a.tenant_id AND n.account_id = a.account_id
         WHERE a.tenant_id = p_account_tenant AND a.account_id = p_account
           AND a.closed_at IS NULL
    ) THEN
        RETURN QUERY SELECT 'account_ineligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT,
            NULL::BIGINT, NULL::TEXT;
        RETURN;
    END IF;

    SELECT d.public_key INTO v_stored_key
      FROM public.device_keys d
     WHERE d.tenant_id = p_legacy_tenant AND d.device_key_id = p_device_key_id
       AND d.revoked_at IS NULL AND d.onboarding_origin = 'invite'
       AND d.invite_subject_hash = p_invite_hash;
    v_key_matches := FALSE;
    IF FOUND THEN
        BEGIN
            v_key_matches := decode(btrim(v_stored_key), 'base64') = p_public_key;
        EXCEPTION WHEN OTHERS THEN
            v_key_matches := FALSE;
        END;
    END IF;
    -- Instance-enrolled only: an invite row means the V81 link applies.
    -- And only the tenant's earliest live device: whoever holds the
    -- instance key can enroll a later device into this tenant, and must not
    -- be able to claim it with that one (review of #1865).
    IF NOT v_key_matches OR EXISTS (
        SELECT 1 FROM public.onboarding_invites i
         WHERE i.tenant_id = p_legacy_tenant AND i.invite_subject_hash = p_invite_hash
    ) OR EXISTS (
        SELECT 1 FROM public.device_keys older, public.device_keys this
         WHERE this.tenant_id = p_legacy_tenant AND this.device_key_id = p_device_key_id
           AND older.tenant_id = p_legacy_tenant AND older.revoked_at IS NULL
           AND older.device_key_id <> p_device_key_id
           AND (older.created_at, older.device_key_id) < (this.created_at, this.device_key_id)
    ) THEN
        RETURN QUERY SELECT 'device_not_eligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT,
            NULL::BIGINT, NULL::TEXT;
        RETURN;
    END IF;

    IF EXISTS (SELECT 1 FROM public.trace_legacy_invite_pooled_tenants p
                WHERE p.tenant_id = p_legacy_tenant) THEN
        RETURN QUERY SELECT 'tenant_pooled'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT,
            NULL::BIGINT, NULL::TEXT;
        RETURN;
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || p_legacy_tenant, 0));

    SELECT t.trust_version, t.authority INTO v_version, v_authority
      FROM public.trace_account_trust t
     WHERE t.tenant_id = p_account_tenant AND t.account_id = p_account;
    v_version := coalesce(v_version, 0);
    v_authority := coalesce(v_authority, 'none');

    SELECT * INTO v_existing FROM public.trace_legacy_invite_links l
     WHERE l.legacy_tenant_id = p_legacy_tenant AND l.revoked_at IS NULL;
    IF FOUND THEN
        IF v_existing.tenant_id = p_account_tenant AND v_existing.account_id = p_account THEN
            RETURN QUERY SELECT 'already_linked'::TEXT, v_existing.link_id,
                v_existing.device_key_id, v_existing.invite_subject_hash, v_existing.nonce,
                v_existing.issued_at, v_existing.device_signature, v_existing.linked_at,
                v_existing.server_kid, v_existing.server_signature, v_version, v_authority;
            RETURN;
        END IF;
        INSERT INTO public.trace_legacy_invite_link_conflicts AS c
            (tenant_id, account_id, legacy_tenant_id)
        VALUES (p_account_tenant, p_account, p_legacy_tenant)
        ON CONFLICT (tenant_id, account_id, legacy_tenant_id) DO UPDATE
           SET last_seen_at = clock_timestamp(), attempts = c.attempts + 1;
        INSERT INTO public.trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
        VALUES (p_account_tenant, 'legacy_instance_link_refused',
                public.trace_payout_account_hash(p_account),
                'tenant_claimed',
                jsonb_build_object(
                    'legacy_tenant_hash',
                    'sha256:' || encode(sha256(convert_to(p_legacy_tenant, 'UTF8')), 'hex')));
        RETURN QUERY SELECT 'tenant_claimed'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT,
            NULL::BIGINT, NULL::TEXT;
        RETURN;
    END IF;

    INSERT INTO public.trace_legacy_invite_links
        (link_id, tenant_id, account_id, legacy_tenant_id, device_key_id,
         invite_subject_hash, nonce, issued_at, device_signature, linked_at,
         server_kid, server_signature)
    VALUES (p_link_id, p_account_tenant, p_account, p_legacy_tenant, p_device_key_id,
            p_invite_hash, p_nonce, p_issued_at, p_device_signature, p_linked_at,
            p_server_kid, p_server_signature);

    INSERT INTO public.trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
    VALUES (p_account_tenant, 'legacy_instance_linked',
            public.trace_payout_account_hash(p_account),
            'success',
            jsonb_build_object(
                'legacy_tenant_hash',
                'sha256:' || encode(sha256(convert_to(p_legacy_tenant, 'UTF8')), 'hex'),
                'device_key_id', p_device_key_id));

    RETURN QUERY SELECT 'linked'::TEXT, p_link_id, p_device_key_id, p_invite_hash, p_nonce,
        p_issued_at, p_device_signature, p_linked_at, p_server_kid, p_server_signature,
        v_version, v_authority;
END $$;
ALTER FUNCTION public.trace_link_legacy_instance_device(TEXT, UUID, TEXT, TEXT, BYTEA, TEXT,
    TEXT, BIGINT, UUID, BIGINT, TEXT, TEXT, TEXT) OWNER TO trace_legacy_invite_link_guard;
REVOKE CREATE ON SCHEMA public FROM trace_legacy_invite_link_guard;
REVOKE ALL ON FUNCTION public.trace_link_legacy_instance_device(TEXT, UUID, TEXT, TEXT, BYTEA,
    TEXT, TEXT, BIGINT, UUID, BIGINT, TEXT, TEXT, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_link_legacy_instance_device(TEXT, UUID, TEXT, TEXT,
    BYTEA, TEXT, TEXT, BIGINT, UUID, BIGINT, TEXT, TEXT, TEXT) TO trace_account_invite_runtime;
REVOKE trace_legacy_invite_link_guard FROM CURRENT_USER;
