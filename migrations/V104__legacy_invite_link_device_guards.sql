-- A second device's attestation under a legacy invite link (V91), made to
-- stand on its own checks. Follow-up to the review of #1097.
--
-- V91's `trace_attest_legacy_invite_device` relied on `trace_link_legacy_invite`
-- (V81) running first in the same transaction, and checked less than its
-- comment said: any consumed challenge of the account passed, however old,
-- whichever device it was spent for. Called directly by the runtime role it
-- would record an attestation on a long-spent nonce. V91 is recorded and
-- cannot change (#1028), so this replaces the function's body; its signature,
-- owner and grants stay as V91 left them.
--
-- The function now checks, itself, everything V81 checks:
--   * the account tenant is the tenant context, and the account is open and
--     NEAR-anchored;
--   * the statement's challenge was issued to this account, was spent IN THIS
--     TRANSACTION (the row's xmin is the current transaction id, which is
--     true only after `trace_link_legacy_invite` spent it here), is still
--     unexpired, and has not already backed another attestation. The spend is
--     recorded against this device in `attested_device_key_id`, so one
--     challenge backs at most one attestation;
--   * the device is the legacy tenant's live, invite-onboarded device under
--     the named invite, holding this key, and the tenant redeemed that invite;
--   * the tenant is not pooled, and the invite is not revoked in either
--     registry or in the account's own grant;
--   * the live link belongs to this account, under this same invite.
--
-- A device of the linked tenant that joined under a DIFFERENT invite of the
-- same tenant is refused with its own label, `invite_not_linked`, and an
-- operator-visible, hash-only `legacy_invite_device_attest_refused` audit row.
-- Only the invite the link carries was granted to the account; another invite
-- of the same tenant never was, so refusing is the rule (documented in
-- docs/operator/legacy-invite-migration.md). V91 answered `tenant_claimed`
-- there, which told the contributor the tenant belonged to someone else.
-- No conflict row is written: `trace_legacy_invite_link_conflicts` records
-- two accounts claiming one tenant and blocks account admission until
-- resolved, and this is one account, not an ambiguity.
--
-- The challenge stays spent on that refusal, as on every refusal after the
-- spend (V81: "spent first and stays spent whatever follows"). Keeping it
-- unspent would mean rolling back the transaction, which would also roll back
-- the audit row; and the answer is a rule, not a transient, so the same
-- challenge would only be refused again. The device asks for a new challenge
-- if the operator changes anything.
--
-- The attestation is countersigned by ingest under its own domain
-- (`trace-commons.legacy-invite-device-attestation-record.v1`) and returned
-- with `kind = device_attestation`, so it cannot be mistaken for the link
-- record. `countersign_domain` says which domain a stored row's
-- countersignature is under. A row V91 recorded before this migration is
-- under the link domain; it is superseded in place by the device's next
-- attestation, which is the only way that device can get a record a V92-era
-- client accepts.
--
-- V91's own advisory lock is dropped: on the path ingest takes, V81 has
-- already taken the same transaction-scoped lock and holds it to commit.
-- Off that path, UNIQUE (link_id, device_key_id) and the row lock taken by
-- the supersede UPDATE are what serialise two attestations of one device.

ALTER TABLE trace_legacy_invite_link_challenges
    ADD COLUMN attested_device_key_id TEXT
        CHECK (attested_device_key_id ~ '^sha256:[0-9a-f]{64}$');

ALTER TABLE trace_legacy_invite_link_devices
    ADD COLUMN countersign_domain TEXT NOT NULL
        DEFAULT 'trace-commons.legacy-invite-link-record.v1'
        CHECK (countersign_domain IN (
            'trace-commons.legacy-invite-link-record.v1',
            'trace-commons.legacy-invite-device-attestation-record.v1'));
-- Rows that exist now were countersigned under the link domain; every new row
-- names its domain explicitly.
ALTER TABLE trace_legacy_invite_link_devices ALTER COLUMN countersign_domain DROP DEFAULT;

GRANT UPDATE (attested_device_key_id)
    ON trace_legacy_invite_link_challenges TO trace_legacy_invite_link_guard;
GRANT UPDATE (attestation_id, invite_subject_hash, nonce, issued_at, device_signature,
              attested_at, server_kid, server_signature, countersign_domain)
    ON trace_legacy_invite_link_devices TO trace_legacy_invite_link_guard;

-- Record `p_device_key_id`'s attestation under the live link of
-- `p_legacy_tenant`, which must belong to the calling account under the same
-- invite. Ingest calls it in the same transaction as `trace_link_legacy_invite`,
-- right after that answered `already_linked` with another device's record;
-- every check that call made is made again here (see the header), so a
-- direct call as `trace_account_invite_runtime` gets no further. Returns the
-- device's stored attestation ('attested', or 'already_attested' for a
-- repeat, which returns the first one unchanged), or a refusal label:
-- account_ineligible, challenge_invalid, device_not_eligible, tenant_pooled,
-- invite_revoked, tenant_claimed, invite_not_linked.
GRANT trace_legacy_invite_link_guard TO CURRENT_USER;
GRANT CREATE ON SCHEMA public TO trace_legacy_invite_link_guard;
CREATE OR REPLACE FUNCTION public.trace_attest_legacy_invite_device(
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
    v_domain CONSTANT TEXT := 'trace-commons.legacy-invite-device-attestation-record.v1';
    v_legacy_tenant_hash TEXT;
    v_stored_key TEXT;
    v_key_matches BOOLEAN;
    v_link public.trace_legacy_invite_links%ROWTYPE;
    v_existing public.trace_legacy_invite_link_devices%ROWTYPE;
    v_superseded BOOLEAN;
BEGIN
    IF p_account_tenant IS NULL
        OR p_account_tenant IS DISTINCT FROM public.trace_current_tenant_id()
        OR p_account_tenant !~ '^(near-|nearai-)[0-9a-f]{64}$'
        OR p_account IS NULL OR p_legacy_tenant IS NULL
        OR p_legacy_tenant ~ '^(near-|nearai-)'
        OR p_device_key_id IS NULL OR p_invite_hash IS NULL
        OR p_nonce IS NULL OR p_nonce !~ '^[0-9a-f]{64}$' THEN
        RETURN QUERY SELECT 'account_ineligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;
    v_legacy_tenant_hash :=
        'sha256:' || encode(sha256(convert_to(p_legacy_tenant, 'UTF8')), 'hex');

    -- The statement's challenge: issued to this account, spent by
    -- `trace_link_legacy_invite` in this very transaction, unexpired, and not
    -- yet behind any attestation. Claimed for this device here, so the spend
    -- is recorded and cannot back a second one.
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
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
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
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

    -- The same device test as the link: this tenant's live, invite-onboarded
    -- device, under the named invite, holding this key, for an invite the
    -- tenant actually redeemed.
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
    IF NOT v_key_matches OR NOT EXISTS (
        SELECT 1 FROM public.onboarding_invites i
         WHERE i.tenant_id = p_legacy_tenant AND i.invite_subject_hash = p_invite_hash
    ) THEN
        RETURN QUERY SELECT 'device_not_eligible'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

    IF EXISTS (SELECT 1 FROM public.trace_legacy_invite_pooled_tenants p
                WHERE p.tenant_id = p_legacy_tenant) THEN
        RETURN QUERY SELECT 'tenant_pooled'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

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
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

    -- The live link must be this account's. Another account's claim is not
    -- this function's to record: the link function already refused it and
    -- wrote the conflict row. No live link at all means an operator revoked
    -- it after the link function read it.
    SELECT * INTO v_link FROM public.trace_legacy_invite_links l
     WHERE l.legacy_tenant_id = p_legacy_tenant AND l.revoked_at IS NULL;
    IF NOT FOUND OR v_link.tenant_id <> p_account_tenant OR v_link.account_id <> p_account THEN
        RETURN QUERY SELECT 'tenant_claimed'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

    -- This account's link, but carried by another invite of the same tenant.
    IF v_link.invite_subject_hash <> p_invite_hash THEN
        INSERT INTO public.trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
        VALUES (p_account_tenant, 'legacy_invite_device_attest_refused',
                'account-actor:' || p_account::TEXT, 'invite_not_linked',
                jsonb_build_object(
                    'legacy_tenant_hash', v_legacy_tenant_hash,
                    'device_key_id', p_device_key_id,
                    'invite_subject_hash', p_invite_hash,
                    'linked_invite_subject_hash', v_link.invite_subject_hash));
        RETURN QUERY SELECT 'invite_not_linked'::TEXT, NULL::UUID, NULL::TEXT, NULL::TEXT,
            NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::BIGINT, NULL::TEXT, NULL::TEXT;
        RETURN;
    END IF;

    SELECT * INTO v_existing FROM public.trace_legacy_invite_link_devices a
     WHERE a.link_id = v_link.link_id AND a.device_key_id = p_device_key_id
     FOR UPDATE;
    IF FOUND AND v_existing.countersign_domain = v_domain THEN
        RETURN QUERY SELECT 'already_attested'::TEXT, v_existing.attestation_id,
            v_existing.device_key_id, v_existing.invite_subject_hash, v_existing.nonce,
            v_existing.issued_at, v_existing.device_signature, v_existing.attested_at,
            v_existing.server_kid, v_existing.server_signature;
        RETURN;
    END IF;

    v_superseded := FOUND;
    IF v_superseded THEN
        -- Recorded by V91 under the link domain: supersede it in place.
        UPDATE public.trace_legacy_invite_link_devices a
           SET attestation_id = p_attestation_id, invite_subject_hash = p_invite_hash,
               nonce = p_nonce, issued_at = p_issued_at,
               device_signature = p_device_signature, attested_at = p_attested_at,
               server_kid = p_server_kid, server_signature = p_server_signature,
               countersign_domain = v_domain
         WHERE a.attestation_id = v_existing.attestation_id;
    ELSE
        INSERT INTO public.trace_legacy_invite_link_devices
            (attestation_id, link_id, tenant_id, account_id, device_key_id, invite_subject_hash,
             nonce, issued_at, device_signature, attested_at, server_kid, server_signature,
             countersign_domain)
        VALUES (p_attestation_id, v_link.link_id, p_account_tenant, p_account, p_device_key_id,
                p_invite_hash, p_nonce, p_issued_at, p_device_signature, p_attested_at,
                p_server_kid, p_server_signature, v_domain);
    END IF;

    INSERT INTO public.trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
    VALUES (p_account_tenant, 'legacy_invite_device_attested',
            'account-actor:' || p_account::TEXT, 'attested',
            jsonb_build_object(
                'legacy_tenant_hash', v_legacy_tenant_hash,
                'device_key_id', p_device_key_id,
                'invite_subject_hash', p_invite_hash,
                'superseded_link_domain_record', v_superseded));

    RETURN QUERY SELECT 'attested'::TEXT, p_attestation_id, p_device_key_id, p_invite_hash,
        p_nonce, p_issued_at, p_device_signature, p_attested_at, p_server_kid,
        p_server_signature;
END $$;
REVOKE CREATE ON SCHEMA public FROM trace_legacy_invite_link_guard;
REVOKE trace_legacy_invite_link_guard FROM CURRENT_USER;
