-- Legacy invite identities coexist with account admission, and may be linked
-- to a NEAR account at the tenant level.
--
-- Decisions of 2026-09-27 (docs/operator/legacy-invite-migration.md):
--   * Coexistence. `tenant-…` invite devices and tenants keep working exactly
--     as before; account admission governs only `near-`/`nearai-`. Nobody is
--     forced to migrate.
--   * A link is per TENANT: one legacy invite tenant links to one NEAR
--     account. A second account claiming the same tenant is refused and
--     recorded for the operator.
--   * Pooled tenants (one shared, multi-use invite code serving many people)
--     are never linkable. A tenant is pooled only because an operator said
--     so, in `trace_legacy_invite_pooled_tenants`; nothing is inferred from
--     `max_uses`.
--   * A revoked invite does not carry over. An expired or exhausted one that
--     was already redeemed does.
--
-- Nothing here reads the file allowlist. A file invite that should stop
-- carrying over must also be revoked in `onboarding_invites` (see the runbook).

-- Operator-set marker: this legacy tenant is shared by many people.
CREATE TABLE trace_legacy_invite_pooled_tenants (
    tenant_id TEXT PRIMARY KEY REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    reason_label TEXT NOT NULL CHECK (length(reason_label) BETWEEN 1 AND 128),
    marked_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Single-use challenges for the link statement, per NEAR account.
CREATE TABLE trace_legacy_invite_link_challenges (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    nonce_hash TEXT NOT NULL CHECK (nonce_hash ~ '^sha256:[0-9a-f]{64}$'),
    issued_at BIGINT NOT NULL,
    expires_at BIGINT NOT NULL CHECK (expires_at > issued_at),
    consumed_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, nonce_hash),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
CREATE INDEX trace_legacy_invite_link_challenges_open
    ON trace_legacy_invite_link_challenges (tenant_id, account_id)
    WHERE consumed_at IS NULL;

-- The link record. `tenant_id`/`account_id` are the NEAR account (the row is
-- scoped to it); `legacy_tenant_id` is the invite tenant it absorbed. The
-- statement fields, the device signature and the server countersignature are
-- stored exactly as returned, so the record can be re-verified later.
CREATE TABLE trace_legacy_invite_links (
    link_id UUID PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    legacy_tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    device_key_id TEXT NOT NULL CHECK (device_key_id ~ '^sha256:[0-9a-f]{64}$'),
    invite_subject_hash TEXT NOT NULL CHECK (invite_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    nonce TEXT NOT NULL CHECK (nonce ~ '^[0-9a-f]{64}$'),
    issued_at BIGINT NOT NULL,
    device_signature TEXT NOT NULL,
    linked_at BIGINT NOT NULL,
    server_kid TEXT NOT NULL,
    server_signature TEXT NOT NULL,
    revoked_at TIMESTAMPTZ,
    CHECK (legacy_tenant_id !~ '^(near-|nearai-)'),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);
-- One live link per legacy tenant, across every account tenant.
CREATE UNIQUE INDEX trace_legacy_invite_links_one_per_tenant
    ON trace_legacy_invite_links (legacy_tenant_id) WHERE revoked_at IS NULL;
CREATE INDEX trace_legacy_invite_links_account
    ON trace_legacy_invite_links (tenant_id, account_id);

-- A second account claimed an already-linked, non-pooled tenant. Each row is
-- an ambiguity the operator resolves; unresolved rows block account
-- admission (trace_account_admission_linkage_ready below).
CREATE TABLE trace_legacy_invite_link_conflicts (
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    legacy_tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts BIGINT NOT NULL DEFAULT 1 CHECK (attempts > 0),
    resolved_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, account_id, legacy_tenant_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);

ALTER TABLE trace_legacy_invite_pooled_tenants ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_invite_pooled_tenants FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_legacy_invite_pooled_tenants;
CREATE POLICY trace_corpus_tenant_isolation ON trace_legacy_invite_pooled_tenants
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_legacy_invite_link_challenges ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_invite_link_challenges FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_legacy_invite_link_challenges;
CREATE POLICY trace_corpus_tenant_isolation ON trace_legacy_invite_link_challenges
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_legacy_invite_links ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_invite_links FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_legacy_invite_links;
CREATE POLICY trace_corpus_tenant_isolation ON trace_legacy_invite_links
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_legacy_invite_link_conflicts ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_invite_link_conflicts FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_legacy_invite_link_conflicts;
CREATE POLICY trace_corpus_tenant_isolation ON trace_legacy_invite_link_conflicts
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The ingest login already holds trace_account_invite_runtime (V75). It may
-- issue challenges for its own tenant and call the link function; it gains no
-- read of another tenant's devices, invites or links.
GRANT SELECT, INSERT ON trace_legacy_invite_link_challenges TO trace_account_invite_runtime;

-- The link function's owner. NOLOGIN and NOBYPASSRLS: the SELECT policies
-- below, scoped to this role alone, are what let it read across tenants, and
-- only through the one function it owns.
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_legacy_invite_link_guard') THEN
        CREATE ROLE trace_legacy_invite_link_guard NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
-- Only NOLOGIN is re-asserted: changing BYPASSRLS needs a superuser, and the
-- migrating owner is not one (migration_atomicity_pg).
ALTER ROLE trace_legacy_invite_link_guard NOLOGIN;
GRANT trace_legacy_invite_link_guard TO CURRENT_USER;
GRANT USAGE ON SCHEMA public TO trace_legacy_invite_link_guard;
GRANT SELECT (tenant_id, device_key_id, public_key, invite_subject_hash, revoked_at, onboarding_origin)
    ON device_keys TO trace_legacy_invite_link_guard;
GRANT SELECT (tenant_id, invite_subject_hash, revoked_at)
    ON onboarding_invites TO trace_legacy_invite_link_guard;
GRANT SELECT (invite_subject_hash, revoked_at)
    ON onboarding_invite_grants TO trace_legacy_invite_link_guard;
GRANT SELECT ON trace_legacy_invite_pooled_tenants TO trace_legacy_invite_link_guard;
GRANT SELECT (tenant_id, account_id, closed_at) ON trace_accounts TO trace_legacy_invite_link_guard;
GRANT SELECT (tenant_id, account_id) ON trace_near_account_anchors TO trace_legacy_invite_link_guard;
GRANT SELECT, UPDATE (consumed_at) ON trace_legacy_invite_link_challenges TO trace_legacy_invite_link_guard;
GRANT SELECT, INSERT ON trace_legacy_invite_links TO trace_legacy_invite_link_guard;
GRANT SELECT, INSERT, UPDATE (last_seen_at, attempts)
    ON trace_legacy_invite_link_conflicts TO trace_legacy_invite_link_guard;
GRANT SELECT, INSERT ON trace_account_invite_grants TO trace_legacy_invite_link_guard;
GRANT SELECT, INSERT, UPDATE (authority, trust_version, updated_at)
    ON trace_account_trust TO trace_legacy_invite_link_guard;
GRANT INSERT ON trace_account_audit TO trace_legacy_invite_link_guard;
GRANT USAGE ON SEQUENCE trace_account_audit_audit_sequence_seq TO trace_legacy_invite_link_guard;

CREATE POLICY legacy_invite_link_devices ON device_keys
    FOR SELECT TO trace_legacy_invite_link_guard USING (TRUE);
CREATE POLICY legacy_invite_link_invites ON onboarding_invites
    FOR SELECT TO trace_legacy_invite_link_guard USING (TRUE);
CREATE POLICY legacy_invite_link_invite_grants ON onboarding_invite_grants
    FOR SELECT TO trace_legacy_invite_link_guard USING (TRUE);
CREATE POLICY legacy_invite_link_pooled ON trace_legacy_invite_pooled_tenants
    FOR SELECT TO trace_legacy_invite_link_guard USING (TRUE);
CREATE POLICY legacy_invite_link_links ON trace_legacy_invite_links
    FOR SELECT TO trace_legacy_invite_link_guard USING (TRUE);

-- Link one legacy invite tenant to the calling NEAR account.
--
-- The caller has already verified the device's Ed25519 signature over the
-- statement against `p_public_key` and countersigned the record; this
-- function proves that key IS the registered, unrevoked, invite-onboarded
-- device of `p_legacy_tenant`, and decides. It returns a label and, only for
-- 'linked' and 'already_linked', the caller's own stored record. It never
-- returns another tenant's data.
--
-- Labels: challenge_invalid, account_ineligible, device_not_eligible,
-- tenant_pooled, invite_revoked, tenant_claimed, already_linked, linked.
CREATE FUNCTION public.trace_link_legacy_invite(
    p_account_tenant TEXT, p_account UUID, p_legacy_tenant TEXT,
    p_device_key_id TEXT, p_public_key BYTEA, p_invite_hash TEXT,
    p_nonce TEXT, p_issued_at BIGINT, p_link_id UUID, p_linked_at BIGINT,
    p_device_signature TEXT, p_server_kid TEXT, p_server_signature TEXT
) RETURNS TABLE (
    outcome TEXT, link_id UUID, device_key_id TEXT, invite_subject_hash TEXT,
    nonce TEXT, issued_at BIGINT, device_signature TEXT, linked_at BIGINT,
    server_kid TEXT, server_signature TEXT, trust_version BIGINT
)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_nonce_hash TEXT;
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
        OR p_nonce IS NULL OR p_nonce !~ '^[0-9a-f]{64}$' THEN
        RETURN QUERY SELECT 'account_ineligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT, NULL::BIGINT;
        RETURN;
    END IF;

    -- The challenge is spent first and stays spent whatever follows.
    v_nonce_hash := 'sha256:' || encode(sha256(convert_to(p_nonce, 'UTF8')), 'hex');
    UPDATE public.trace_legacy_invite_link_challenges c
       SET consumed_at = clock_timestamp()
     WHERE c.tenant_id = p_account_tenant AND c.account_id = p_account
       AND c.nonce_hash = v_nonce_hash AND c.consumed_at IS NULL
       AND c.issued_at = p_issued_at
       AND c.expires_at > extract(epoch FROM clock_timestamp())::BIGINT;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'challenge_invalid'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT, NULL::BIGINT;
        RETURN;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM public.trace_accounts a
          JOIN public.trace_near_account_anchors n
            ON n.tenant_id = a.tenant_id AND n.account_id = a.account_id
         WHERE a.tenant_id = p_account_tenant AND a.account_id = p_account
           AND a.closed_at IS NULL
    ) OR p_legacy_tenant ~ '^(near-|nearai-)' THEN
        RETURN QUERY SELECT 'account_ineligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT, NULL::BIGINT;
        RETURN;
    END IF;

    -- The signing key must be this tenant's live, invite-onboarded device,
    -- registered under the invite the statement names. Unknown, revoked,
    -- mismatched and non-invite devices all read the same.
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
    -- The invite must be one this tenant actually redeemed (V29 records every
    -- file- and registry-path redemption per tenant; an instance enrollment
    -- has no such row and is not an invite identity).
    IF NOT v_key_matches OR NOT EXISTS (
        SELECT 1 FROM public.onboarding_invites i
         WHERE i.tenant_id = p_legacy_tenant AND i.invite_subject_hash = p_invite_hash
    ) THEN
        RETURN QUERY SELECT 'device_not_eligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT, NULL::BIGINT;
        RETURN;
    END IF;

    -- Checked only after the device proved control of the tenant, so a probe
    -- cannot learn which tenants are pooled.
    IF EXISTS (SELECT 1 FROM public.trace_legacy_invite_pooled_tenants p
                WHERE p.tenant_id = p_legacy_tenant) THEN
        RETURN QUERY SELECT 'tenant_pooled'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT, NULL::BIGINT;
        RETURN;
    END IF;

    -- A revoked invite does not carry over, from either registry. Expiry and
    -- exhaustion are deliberately not checked: the invite was redeemed.
    IF EXISTS (SELECT 1 FROM public.onboarding_invites i
                WHERE i.tenant_id = p_legacy_tenant AND i.invite_subject_hash = p_invite_hash
                  AND i.revoked_at IS NOT NULL)
       OR EXISTS (SELECT 1 FROM public.onboarding_invite_grants g
                   WHERE g.invite_subject_hash = p_invite_hash AND g.revoked_at IS NOT NULL)
       OR EXISTS (SELECT 1 FROM public.trace_account_invite_grants ag
                   WHERE ag.tenant_id = p_account_tenant AND ag.account_id = p_account
                     AND ag.invite_subject_hash = p_invite_hash AND ag.revoked_at IS NOT NULL)
    THEN
        RETURN QUERY SELECT 'invite_revoked'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT, NULL::BIGINT;
        RETURN;
    END IF;

    -- Serialize every claim on this legacy tenant, from any account tenant.
    PERFORM pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || p_legacy_tenant, 0));

    SELECT * INTO v_existing FROM public.trace_legacy_invite_links l
     WHERE l.legacy_tenant_id = p_legacy_tenant AND l.revoked_at IS NULL;
    IF FOUND THEN
        IF v_existing.tenant_id = p_account_tenant AND v_existing.account_id = p_account THEN
            SELECT t.trust_version INTO v_version FROM public.trace_account_trust t
             WHERE t.tenant_id = p_account_tenant AND t.account_id = p_account;
            RETURN QUERY SELECT 'already_linked'::TEXT, v_existing.link_id,
                v_existing.device_key_id, v_existing.invite_subject_hash, v_existing.nonce,
                v_existing.issued_at, v_existing.device_signature, v_existing.linked_at,
                v_existing.server_kid, v_existing.server_signature, v_version;
            RETURN;
        END IF;
        INSERT INTO public.trace_legacy_invite_link_conflicts AS c
            (tenant_id, account_id, legacy_tenant_id)
        VALUES (p_account_tenant, p_account, p_legacy_tenant)
        ON CONFLICT (tenant_id, account_id, legacy_tenant_id) DO UPDATE
           SET last_seen_at = clock_timestamp(), attempts = c.attempts + 1;
        INSERT INTO public.trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
        VALUES (p_account_tenant, 'legacy_invite_link_refused',
                'account-actor:' || p_account::TEXT, 'tenant_claimed',
                jsonb_build_object(
                    'legacy_tenant_hash',
                    'sha256:' || encode(sha256(convert_to(p_legacy_tenant, 'UTF8')), 'hex'),
                    'invite_subject_hash', p_invite_hash));
        RETURN QUERY SELECT 'tenant_claimed'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT, NULL::BIGINT;
        RETURN;
    END IF;

    INSERT INTO public.trace_legacy_invite_links
        (link_id, tenant_id, account_id, legacy_tenant_id, device_key_id,
         invite_subject_hash, nonce, issued_at, device_signature, linked_at,
         server_kid, server_signature)
    VALUES (p_link_id, p_account_tenant, p_account, p_legacy_tenant, p_device_key_id,
            p_invite_hash, p_nonce, p_issued_at, p_device_signature, p_linked_at,
            p_server_kid, p_server_signature);

    -- Invite trust, exactly as a redemption confers it, without spending a
    -- use: the legacy tenant already spent one when it onboarded.
    SELECT t.authority, t.trust_version INTO v_authority, v_version
      FROM public.trace_account_trust t
     WHERE t.tenant_id = p_account_tenant AND t.account_id = p_account
     FOR UPDATE;
    IF NOT FOUND THEN
        v_version := 1;
        INSERT INTO public.trace_account_trust (tenant_id, account_id, authority, trust_version)
        VALUES (p_account_tenant, p_account, 'invited', v_version);
    ELSIF v_authority IS DISTINCT FROM 'invited' THEN
        v_version := v_version + 1;
        UPDATE public.trace_account_trust
           SET authority = 'invited', trust_version = v_version,
               updated_at = clock_timestamp()
         WHERE tenant_id = p_account_tenant AND account_id = p_account;
    END IF;
    INSERT INTO public.trace_account_invite_grants
        (tenant_id, account_id, invite_subject_hash, trust_version)
    VALUES (p_account_tenant, p_account, p_invite_hash, v_version)
    ON CONFLICT DO NOTHING;

    INSERT INTO public.trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
    VALUES (p_account_tenant, 'legacy_invite_linked',
            'account-actor:' || p_account::TEXT, 'invited',
            jsonb_build_object(
                'legacy_tenant_hash',
                'sha256:' || encode(sha256(convert_to(p_legacy_tenant, 'UTF8')), 'hex'),
                'device_key_id', p_device_key_id,
                'invite_subject_hash', p_invite_hash,
                'trust_version', v_version));

    RETURN QUERY SELECT 'linked'::TEXT, p_link_id, p_device_key_id, p_invite_hash, p_nonce,
        p_issued_at, p_device_signature, p_linked_at, p_server_kid, p_server_signature, v_version;
END $$;
GRANT CREATE ON SCHEMA public TO trace_legacy_invite_link_guard;
ALTER FUNCTION public.trace_link_legacy_invite(TEXT, UUID, TEXT, TEXT, BYTEA, TEXT, TEXT,
    BIGINT, UUID, BIGINT, TEXT, TEXT, TEXT) OWNER TO trace_legacy_invite_link_guard;
REVOKE CREATE ON SCHEMA public FROM trace_legacy_invite_link_guard;
REVOKE ALL ON FUNCTION public.trace_link_legacy_invite(TEXT, UUID, TEXT, TEXT, BYTEA, TEXT,
    TEXT, BIGINT, UUID, BIGINT, TEXT, TEXT, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_link_legacy_invite(TEXT, UUID, TEXT, TEXT, BYTEA, TEXT,
    TEXT, BIGINT, UUID, BIGINT, TEXT, TEXT, TEXT) TO trace_account_invite_runtime;
REVOKE trace_legacy_invite_link_guard FROM CURRENT_USER;

-- Readiness with coexistence (replaces V77's body; same signature, owner and
-- grants). Blocks account admission only on what is genuinely ambiguous:
--   1. an active device in the `near-`/`nearai-` namespace that is not a
--      NEAR/NEAR AI device with live account linkage (unchanged from V77);
--   2. an active device OUTSIDE that namespace that is not invite-onboarded
--      (unchanged in effect: V58/V63 allow no other origin there today);
--   3. an open account OUTSIDE that namespace whose tenant is neither pooled
--      nor has any invite-onboarded device -- an identity with no invite
--      behind it;
--   4. an unresolved claim of a non-pooled legacy tenant by a second account.
-- Coexisting, and therefore NOT blocking: every invite-onboarded legacy
-- device and its tenant's accounts, linked or not, and every pooled tenant.
GRANT trace_account_readiness_guard TO CURRENT_USER;
-- device_keys (tenant_id, device_key_id, revoked_at, onboarding_origin) and
-- trace_accounts were granted to this guard by V77; nothing more is read.
GRANT SELECT ON trace_legacy_invite_pooled_tenants TO trace_account_readiness_guard;
GRANT SELECT (legacy_tenant_id, resolved_at)
    ON trace_legacy_invite_link_conflicts TO trace_account_readiness_guard;
CREATE POLICY account_readiness_pooled ON trace_legacy_invite_pooled_tenants
    FOR SELECT TO trace_account_readiness_guard USING (TRUE);
CREATE POLICY account_readiness_link_conflicts ON trace_legacy_invite_link_conflicts
    FOR SELECT TO trace_account_readiness_guard USING (TRUE);
GRANT CREATE ON SCHEMA public TO trace_account_readiness_guard;
CREATE OR REPLACE FUNCTION trace_account_admission_linkage_ready() RETURNS BOOLEAN
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
    SELECT NOT EXISTS (
        SELECT 1 FROM public.device_keys d
         WHERE d.revoked_at IS NULL
           AND d.tenant_id ~ '^(near-|nearai-)[0-9a-f]{64}$'
           AND (d.onboarding_origin NOT IN ('near','near_ai')
                OR NOT EXISTS (
                    SELECT 1 FROM public.trace_near_provisioned_devices n
                    JOIN public.trace_accounts a ON a.tenant_id=n.tenant_id AND a.account_id=n.account_id
                    JOIN public.trace_account_principals p ON p.tenant_id=n.tenant_id
                        AND p.account_id=n.account_id AND p.principal_ref=n.principal_ref
                    WHERE n.tenant_id=d.tenant_id AND n.device_key_id=d.device_key_id
                        AND a.closed_at IS NULL AND p.unlinked_at IS NULL))
    ) AND NOT EXISTS (
        SELECT 1 FROM public.device_keys d
         WHERE d.revoked_at IS NULL
           AND d.tenant_id !~ '^(near-|nearai-)[0-9a-f]{64}$'
           AND d.onboarding_origin <> 'invite'
    ) AND NOT EXISTS (
        SELECT 1 FROM public.trace_accounts a
         WHERE a.closed_at IS NULL
           AND a.tenant_id !~ '^(near-|nearai-)[0-9a-f]{64}$'
           AND NOT EXISTS (SELECT 1 FROM public.trace_legacy_invite_pooled_tenants p
                            WHERE p.tenant_id = a.tenant_id)
           AND NOT EXISTS (SELECT 1 FROM public.device_keys d
                            WHERE d.tenant_id = a.tenant_id AND d.onboarding_origin = 'invite')
    ) AND NOT EXISTS (
        SELECT 1 FROM public.trace_legacy_invite_link_conflicts c
         WHERE c.resolved_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM public.trace_legacy_invite_pooled_tenants p
                            WHERE p.tenant_id = c.legacy_tenant_id)
    );
$$;
REVOKE CREATE ON SCHEMA public FROM trace_account_readiness_guard;
REVOKE ALL ON FUNCTION trace_account_admission_linkage_ready() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION trace_account_admission_linkage_ready() TO trace_account_admission_runtime;
REVOKE trace_account_readiness_guard FROM CURRENT_USER;
