-- Bound upload-trust invalidation by the authoritative changed keys.
-- V137 may already be deployed: replace its trigger function in place, keeping
-- ownership discovery, one global account lock order and generation semantics.
CREATE INDEX trace_submissions_tenant_auth_principal
    ON trace_submissions(tenant_id,auth_principal_ref);
CREATE INDEX trace_device_upload_principals_device
    ON trace_device_upload_principals(tenant_id,device_key_id);

GRANT trace_account_trust_input_guard TO CURRENT_USER;
CREATE OR REPLACE FUNCTION trace_account_trust_advance_upload_frontiers() RETURNS TRIGGER
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE v_old_tenant TEXT; v_new_tenant TEXT; v_old_submission UUID; v_new_submission UUID;
        v_old_principal TEXT; v_new_principal TEXT; v_old_device TEXT; v_new_device TEXT;
        v_old_parent TEXT; v_new_parent TEXT; v_account RECORD;
BEGIN
    IF TG_OP='UPDATE' AND OLD IS NOT DISTINCT FROM NEW THEN RETURN NULL; END IF;
    PERFORM 1 FROM public.trace_account_trust_external_growth WHERE enabled FOR SHARE;
    IF NOT FOUND THEN RETURN NULL; END IF;
    IF TG_OP<>'INSERT' THEN
        v_old_tenant:=OLD.tenant_id;
        IF TG_TABLE_NAME='trace_submissions' THEN v_old_submission:=OLD.submission_id;
        ELSIF TG_TABLE_NAME='trace_device_upload_principals' THEN
            v_old_principal:=OLD.upload_principal_ref; v_old_parent:=OLD.device_principal_ref;
        ELSE v_old_device:=OLD.device_key_id;
            v_old_parent:='principal_sha256:' || pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
                'device:' || OLD.tenant_id || ':' || OLD.device_key_id,'UTF8')),'hex'); END IF;
    END IF;
    IF TG_OP<>'DELETE' THEN
        v_new_tenant:=NEW.tenant_id;
        IF TG_TABLE_NAME='trace_submissions' THEN v_new_submission:=NEW.submission_id;
        ELSIF TG_TABLE_NAME='trace_device_upload_principals' THEN
            v_new_principal:=NEW.upload_principal_ref; v_new_parent:=NEW.device_principal_ref;
        ELSE v_new_device:=NEW.device_key_id;
            v_new_parent:='principal_sha256:' || pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
                'device:' || NEW.tenant_id || ':' || NEW.device_key_id,'UTF8')),'hex'); END IF;
    END IF;
    IF TG_TABLE_NAME='trace_submissions' THEN
        PERFORM public.trace_account_trust_lock_fact_dependencies(v_old_tenant,v_old_submission,v_new_tenant,v_new_submission);
    END IF;
    -- ONE ordered account set: discovering ownership only through existing
    -- facts misses the first-fact race. OLD alias parents survive AFTER DELETE;
    -- canonical device parents and provisioned mappings cover device cascades.
    -- The frontier update is shared with fact INSERT and the RR applied writer.
    -- Do not acquire dependency locks after it: existing order is dependencies
    -- then account frontiers, globally sorted, never the reverse.
    FOR v_account IN
        SELECT DISTINCT affected.tenant_id COLLATE "C" AS tenant_id,affected.account_id FROM (
        -- Separate indexed probes: a disjunction across joined relations
        -- forces a full facts scan on each status mutation. Each arm starts
        -- from its authoritative changed key, then joins by submission key.
        SELECT f.tenant_id,f.account_id FROM public.trace_account_trust_facts f
         WHERE (f.tenant_id,f.submission_id) IN ((v_old_tenant,v_old_submission),(v_new_tenant,v_new_submission))
        UNION ALL
        SELECT f.tenant_id,f.account_id FROM public.trace_submissions s
          JOIN public.trace_account_trust_facts f ON f.tenant_id=s.tenant_id AND f.submission_id=s.submission_id
         WHERE (s.tenant_id,s.auth_principal_ref) IN ((v_old_tenant,v_old_principal),(v_new_tenant,v_new_principal))
        UNION ALL
        SELECT f.tenant_id,f.account_id FROM public.trace_device_upload_principals u
          JOIN public.trace_submissions s ON s.tenant_id=u.tenant_id AND s.auth_principal_ref=u.upload_principal_ref
          JOIN public.trace_account_trust_facts f ON f.tenant_id=s.tenant_id AND f.submission_id=s.submission_id
         WHERE (u.tenant_id,u.device_key_id) IN ((v_old_tenant,v_old_device),(v_new_tenant,v_new_device))
        UNION ALL
        SELECT f.tenant_id,f.account_id FROM public.trace_near_provisioned_devices n
          JOIN public.trace_submissions s ON s.tenant_id=n.tenant_id AND s.auth_principal_ref=n.principal_ref
          JOIN public.trace_account_trust_facts f ON f.tenant_id=s.tenant_id AND f.submission_id=s.submission_id
         WHERE (n.tenant_id,n.device_key_id) IN ((v_old_tenant,v_old_device),(v_new_tenant,v_new_device))
        UNION ALL
        SELECT p.tenant_id,p.account_id FROM public.trace_account_principals p
         WHERE (p.tenant_id,p.principal_ref) IN ((v_old_tenant,v_old_parent),(v_new_tenant,v_new_parent))
        UNION ALL
        SELECT n.tenant_id,n.account_id FROM public.trace_near_provisioned_devices n
         WHERE (n.tenant_id,n.device_key_id) IN ((v_old_tenant,v_old_device),(v_new_tenant,v_new_device))
        UNION ALL
        SELECT p.tenant_id,p.account_id FROM public.trace_device_upload_principals u
          JOIN public.trace_account_principals p ON p.tenant_id=u.tenant_id AND p.principal_ref=u.device_principal_ref
         WHERE (u.tenant_id,u.device_key_id) IN ((v_old_tenant,v_old_device),(v_new_tenant,v_new_device))
        ) affected ORDER BY 1,2
    LOOP
        UPDATE public.trace_account_trust_frontiers SET generation=generation+1
         WHERE tenant_id=v_account.tenant_id AND account_id=v_account.account_id;
    END LOOP;
    RETURN NULL;
END $$;
REVOKE trace_account_trust_input_guard FROM CURRENT_USER;
