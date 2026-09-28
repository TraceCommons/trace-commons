-- A second device of an already-linked legacy invite tenant attests to the
-- same link (decision of 2026-09-27, follow-up to V81).
--
-- V81 links a legacy invite TENANT to one NEAR account, and a re-link by the
-- same account from any of the tenant's devices returns the original record.
-- That record carries the first device's signature, so a second device can
-- never verify it against its own key -- which the client requires before it
-- re-baselines -- and could never migrate. Here the second device's own
-- statement, signed with its own key over a fresh challenge and countersigned
-- by ingest, is recorded under the existing link, for the SAME account only,
-- idempotently per device. Nothing about the link, the account's trust or
-- its invite grant changes: the link already carried them.

CREATE TABLE trace_legacy_invite_link_devices (
    -- The id of the countersigned record returned to this device.
    attestation_id UUID PRIMARY KEY,
    link_id UUID NOT NULL REFERENCES trace_legacy_invite_links(link_id) ON DELETE CASCADE,
    tenant_id TEXT NOT NULL,
    account_id UUID NOT NULL,
    device_key_id TEXT NOT NULL CHECK (device_key_id ~ '^sha256:[0-9a-f]{64}$'),
    invite_subject_hash TEXT NOT NULL CHECK (invite_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    nonce TEXT NOT NULL CHECK (nonce ~ '^[0-9a-f]{64}$'),
    issued_at BIGINT NOT NULL,
    device_signature TEXT NOT NULL,
    attested_at BIGINT NOT NULL,
    server_kid TEXT NOT NULL,
    server_signature TEXT NOT NULL,
    UNIQUE (link_id, device_key_id),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
);

ALTER TABLE trace_legacy_invite_link_devices ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_legacy_invite_link_devices FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_legacy_invite_link_devices;
CREATE POLICY trace_corpus_tenant_isolation ON trace_legacy_invite_link_devices
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- Only the guard writes it, through the function below; the runtime reads
-- nothing here directly.
GRANT SELECT, INSERT ON trace_legacy_invite_link_devices TO trace_legacy_invite_link_guard;

-- Record `p_device_key_id`'s attestation under the live link of
-- `p_legacy_tenant`, which must belong to the calling account. Called by
-- ingest in the same transaction as `trace_link_legacy_invite`, right after
-- it answered `already_linked` with another device's record: that call spent
-- this statement's challenge and proved the device, and both are checked
-- again here rather than trusted. Returns the device's stored attestation
-- ('attested', or 'already_attested' for a repeat, which returns the first
-- one unchanged), or a refusal label: account_ineligible, challenge_invalid,
-- device_not_eligible, tenant_claimed.
CREATE FUNCTION public.trace_attest_legacy_invite_device(
    p_account_tenant TEXT, p_account UUID, p_legacy_tenant TEXT,
    p_device_key_id TEXT, p_public_key BYTEA, p_invite_hash TEXT,
    p_nonce TEXT, p_issued_at BIGINT, p_attestation_id UUID, p_attested_at BIGINT,
    p_device_signature TEXT, p_server_kid TEXT, p_server_signature TEXT
) RETURNS TABLE (
    outcome TEXT, attestation_id UUID, device_key_id TEXT, invite_subject_hash TEXT,
    nonce TEXT, issued_at BIGINT, device_signature TEXT, attested_at BIGINT,
    server_kid TEXT, server_signature TEXT
)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = pg_catalog AS $$
DECLARE
    v_stored_key TEXT;
    v_key_matches BOOLEAN;
    v_link public.trace_legacy_invite_links%ROWTYPE;
    v_existing public.trace_legacy_invite_link_devices%ROWTYPE;
BEGIN
    IF p_account_tenant IS NULL
        OR p_account_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_account_tenant !~ '^(near-|nearai-)[0-9a-f]{64}$'
        OR p_account IS NULL OR p_legacy_tenant IS NULL
        OR p_legacy_tenant ~ '^(near-|nearai-)'
        OR p_nonce IS NULL OR p_nonce !~ '^[0-9a-f]{64}$' THEN
        RETURN QUERY SELECT 'account_ineligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

    -- The statement's challenge must have been issued to this account and
    -- already spent: by `trace_link_legacy_invite`, in this transaction.
    IF NOT EXISTS (
        SELECT 1 FROM public.trace_legacy_invite_link_challenges c
         WHERE c.tenant_id = p_account_tenant AND c.account_id = p_account
           AND c.nonce_hash = 'sha256:' || encode(sha256(convert_to(p_nonce, 'UTF8')), 'hex')
           AND c.issued_at = p_issued_at AND c.consumed_at IS NOT NULL
    ) THEN
        RETURN QUERY SELECT 'challenge_invalid'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

    -- The same device test as the link: this tenant's live, invite-onboarded
    -- device, under the named invite, holding this key.
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
    IF NOT v_key_matches THEN
        RETURN QUERY SELECT 'device_not_eligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

    PERFORM pg_advisory_xact_lock(hashtextextended('trace_legacy_invite_link:' || p_legacy_tenant, 0));

    -- The live link must be this account's. Anything else is not this
    -- function's to decide: the link function already refused and recorded
    -- another account's claim.
    SELECT * INTO v_link FROM public.trace_legacy_invite_links l
     WHERE l.legacy_tenant_id = p_legacy_tenant AND l.revoked_at IS NULL;
    IF NOT FOUND OR v_link.tenant_id <> p_account_tenant OR v_link.account_id <> p_account
        OR v_link.invite_subject_hash <> p_invite_hash THEN
        RETURN QUERY SELECT 'tenant_claimed'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

    SELECT * INTO v_existing FROM public.trace_legacy_invite_link_devices a
     WHERE a.link_id = v_link.link_id AND a.device_key_id = p_device_key_id;
    IF FOUND THEN
        RETURN QUERY SELECT 'already_attested'::TEXT, v_existing.attestation_id,
            v_existing.device_key_id, v_existing.invite_subject_hash, v_existing.nonce,
            v_existing.issued_at, v_existing.device_signature, v_existing.attested_at,
            v_existing.server_kid, v_existing.server_signature;
        RETURN;
    END IF;

    INSERT INTO public.trace_legacy_invite_link_devices
        (attestation_id, link_id, tenant_id, account_id, device_key_id, invite_subject_hash,
         nonce, issued_at, device_signature, attested_at, server_kid, server_signature)
    VALUES (p_attestation_id, v_link.link_id, p_account_tenant, p_account, p_device_key_id,
            p_invite_hash, p_nonce, p_issued_at, p_device_signature, p_attested_at,
            p_server_kid, p_server_signature);

    INSERT INTO public.trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
    VALUES (p_account_tenant, 'legacy_invite_device_attested',
            'account-actor:' || p_account::TEXT, 'attested',
            jsonb_build_object(
                'legacy_tenant_hash',
                'sha256:' || encode(sha256(convert_to(p_legacy_tenant, 'UTF8')), 'hex'),
                'device_key_id', p_device_key_id,
                'invite_subject_hash', p_invite_hash));

    RETURN QUERY SELECT 'attested'::TEXT, p_attestation_id, p_device_key_id, p_invite_hash,
        p_nonce, p_issued_at, p_device_signature, p_attested_at, p_server_kid,
        p_server_signature;
END $$;
GRANT trace_legacy_invite_link_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_legacy_invite_link_guard;
ALTER FUNCTION public.trace_attest_legacy_invite_device(TEXT, UUID, TEXT, TEXT, BYTEA, TEXT,
    TEXT, BIGINT, UUID, BIGINT, TEXT, TEXT, TEXT) OWNER TO trace_legacy_invite_link_guard;
REVOKE CREATE ON SCHEMA public FROM trace_legacy_invite_link_guard;
REVOKE ALL ON FUNCTION public.trace_attest_legacy_invite_device(TEXT, UUID, TEXT, TEXT, BYTEA,
    TEXT, TEXT, BIGINT, UUID, BIGINT, TEXT, TEXT, TEXT) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION public.trace_attest_legacy_invite_device(TEXT, UUID, TEXT, TEXT,
    BYTEA, TEXT, TEXT, BIGINT, UUID, BIGINT, TEXT, TEXT, TEXT) TO trace_account_invite_runtime;
REVOKE trace_legacy_invite_link_guard FROM CURRENT_USER;
