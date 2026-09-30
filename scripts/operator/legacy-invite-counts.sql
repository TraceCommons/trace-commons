-- Legacy invite identity counts, before and after account admission.
--
-- The inventory docs/operator/account-trust.md asks for before enabling
-- account admission, under the coexistence rules of V81
-- (docs/operator/legacy-invite-migration.md): legacy invite tenants (pooled
-- and individual separately), devices per tenant, wallet and NEAR AI
-- accounts, linked versus unlinked tenants, ambiguous claims, and what would
-- block `trace_account_admission_linkage_ready()`.
--
-- READ-ONLY. Counts only: no tenant ids, account ids, device ids, invite
-- hashes or labels reach the output.
--
-- Run (see docs/operator/legacy-invite-counts.md):
--
--   PGOPTIONS='-c default_transaction_read_only=on' \
--   psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 \
--     -f scripts/operator/legacy-invite-counts.sql
--
-- The whole measurement runs inside one READ ONLY, REPEATABLE READ
-- transaction that ends in ROLLBACK.
--
-- Works on a schema before V81 too, so it can be run before that migration
-- is deployed. Without V81 there is no pooled marker, no link and no claim:
-- every figure that depends on them is NULL (not measurable), never 0, and
-- every legacy invite tenant is reported as `unclassified`.
--
-- Definitions:
--   * Account namespaces: `near-` (wallet) and `nearai-` (NEAR AI). The list
--     below is a copy; tests/legacy_invite_counts.rs in the server crate fails
--     when it drifts from trace_commons_protocol::admission::ANCHOR_NAMESPACES.
--   * Legacy invite tenant: a tenant outside those namespaces with a V29
--     `onboarding_invites` row, i.e. one that redeemed an invite through
--     `/v1/onboard` (file allowlist or registry).
--   * Instance-enrolled tenant: outside those namespaces, `invite`-origin
--     devices, but no `onboarding_invites` row. Not an invite identity.
--   * Pooled: marked by an operator in trace_legacy_invite_pooled_tenants.
--   * Review before linking: an UNMARKED legacy invite tenant whose invite
--     allows more than the individual shape's 3 uses, or that more than one
--     invite routes to. A prompt for a decision, never a classification.
--   * Ambiguous: a non-pooled legacy tenant with an unresolved claim by a
--     second account.

\set ON_ERROR_STOP on
\pset footer off

BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;

-- Every Trace Commons table is FORCE ROW LEVEL SECURITY. A role that does not
-- bypass it sees no rows without a tenant context and would report zero.
SELECT coalesce((SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname = current_user), false)
    AS lic_sees_every_tenant
\gset
\if :lic_sees_every_tenant
\else
DO $$ BEGIN
    RAISE EXCEPTION 'LegacyInviteCountsRoleSubjectToRls: connect as a role with BYPASSRLS (or the superuser); an RLS-bound role sees no rows and would report zero';
END $$;
\endif

SELECT
    to_regclass('trace_legacy_invite_pooled_tenants') IS NOT NULL
        AND to_regclass('trace_legacy_invite_links') IS NOT NULL
        AND to_regclass('trace_legacy_invite_link_conflicts') IS NOT NULL AS lic_has_v81,
    to_regclass('_trace_commons_migrations') IS NOT NULL AS lic_has_ledger
\gset

\if :lic_has_v81
\set lic_pooled 'SELECT tenant_id FROM trace_legacy_invite_pooled_tenants'
\set lic_links 'SELECT legacy_tenant_id, tenant_id, account_id FROM trace_legacy_invite_links WHERE revoked_at IS NULL'
\set lic_conflicts 'SELECT legacy_tenant_id, tenant_id, account_id FROM trace_legacy_invite_link_conflicts WHERE resolved_at IS NULL'
\else
\set lic_pooled 'SELECT NULL::text AS tenant_id WHERE false'
\set lic_links 'SELECT NULL::text AS legacy_tenant_id, NULL::text AS tenant_id, NULL::uuid AS account_id WHERE false'
\set lic_conflicts 'SELECT NULL::text AS legacy_tenant_id, NULL::text AS tenant_id, NULL::uuid AS account_id WHERE false'
\endif

\if :lic_has_ledger
\set lic_v81_applied '(SELECT applied_at FROM _trace_commons_migrations WHERE version = 81)'
\else
\set lic_v81_applied 'NULL::timestamptz'
\endif

WITH params AS (
    SELECT
        :'lic_has_v81'::boolean AS has_v81,
        ARRAY[
            -- BEGIN ANCHOR_NAMESPACES
            'near-',
            'nearai-'
            -- END ANCHOR_NAMESPACES
        ]::text[] AS namespaces
),
tenant_class AS (
    SELECT t.tenant_id,
           coalesce(
               -- A namespace followed by a 64-hex hash, exactly as
               -- is_anchored_tenant() and the V81 readiness function decide.
               (SELECT ns FROM params p, unnest(p.namespaces) AS ns
                 WHERE starts_with(t.tenant_id, ns)
                   AND substr(t.tenant_id, length(ns) + 1) ~ '^[0-9a-f]{64}$'
                 ORDER BY length(ns) DESC LIMIT 1),
               'legacy') AS ns_class
    FROM trace_tenants t
),
pooled AS (:lic_pooled),
links AS (:lic_links),
conflicts AS (:lic_conflicts),
invite_tenants AS (
    SELECT tc.tenant_id,
           EXISTS (SELECT 1 FROM pooled p WHERE p.tenant_id = tc.tenant_id) AS is_pooled,
           (SELECT max(i.max_uses) FROM onboarding_invites i WHERE i.tenant_id = tc.tenant_id) AS max_uses,
           (SELECT count(DISTINCT i.invite_subject_hash) FROM onboarding_invites i
             WHERE i.tenant_id = tc.tenant_id) AS invites,
           (SELECT count(*) FROM device_keys d
             WHERE d.tenant_id = tc.tenant_id AND d.revoked_at IS NULL) AS active_devices,
           (SELECT count(*) FROM device_keys d
             WHERE d.tenant_id = tc.tenant_id AND d.revoked_at IS NOT NULL) AS revoked_devices,
           EXISTS (SELECT 1 FROM links l WHERE l.legacy_tenant_id = tc.tenant_id) AS is_linked,
           (SELECT count(DISTINCT (c.tenant_id, c.account_id)) FROM conflicts c
             WHERE c.legacy_tenant_id = tc.tenant_id) AS open_claims
    FROM tenant_class tc
    WHERE tc.ns_class = 'legacy'
      AND EXISTS (SELECT 1 FROM onboarding_invites i WHERE i.tenant_id = tc.tenant_id)
),
classified AS (
    SELECT it.*,
           CASE WHEN NOT (SELECT has_v81 FROM params) THEN 'unclassified'
                WHEN is_pooled THEN 'pooled'
                ELSE 'individual' END AS class
    FROM invite_tenants it
),
instance_tenants AS (
    SELECT tc.tenant_id FROM tenant_class tc
     WHERE tc.ns_class = 'legacy'
       AND NOT EXISTS (SELECT 1 FROM onboarding_invites i WHERE i.tenant_id = tc.tenant_id)
       AND EXISTS (SELECT 1 FROM device_keys d
                    WHERE d.tenant_id = tc.tenant_id AND d.onboarding_origin = 'invite')
),
accounts AS (
    SELECT tc.ns_class, a.tenant_id, a.account_id,
           EXISTS (SELECT 1 FROM trace_near_account_anchors n
                    WHERE n.tenant_id = a.tenant_id AND n.account_id = a.account_id) AS anchored
    FROM trace_accounts a JOIN tenant_class tc ON tc.tenant_id = a.tenant_id
    WHERE a.closed_at IS NULL
),
device_buckets AS (
    SELECT class,
           CASE WHEN active_devices = 0 THEN '0'
                WHEN active_devices = 1 THEN '1'
                WHEN active_devices = 2 THEN '2'
                WHEN active_devices = 3 THEN '3'
                WHEN active_devices <= 10 THEN '4-10'
                WHEN active_devices <= 100 THEN '11-100'
                ELSE '101+' END AS bucket,
           CASE WHEN active_devices = 0 THEN 0
                WHEN active_devices <= 3 THEN active_devices
                WHEN active_devices <= 10 THEN 4
                WHEN active_devices <= 100 THEN 11
                ELSE 101 END AS bucket_ord
    FROM classified
),
-- The readiness rules of V81's trace_account_admission_linkage_ready(), as
-- counts. Rule 1 is unchanged from V77.
readiness AS (
    SELECT
        (SELECT count(*) FROM device_keys d JOIN tenant_class tc ON tc.tenant_id = d.tenant_id
          WHERE d.revoked_at IS NULL AND tc.ns_class <> 'legacy'
            AND (d.onboarding_origin NOT IN ('near', 'near_ai')
                 OR NOT EXISTS (
                     SELECT 1 FROM trace_near_provisioned_devices n
                     JOIN trace_accounts a ON a.tenant_id = n.tenant_id AND a.account_id = n.account_id
                     JOIN trace_account_principals pr ON pr.tenant_id = n.tenant_id
                         AND pr.account_id = n.account_id AND pr.principal_ref = n.principal_ref
                     WHERE n.tenant_id = d.tenant_id AND n.device_key_id = d.device_key_id
                       AND a.closed_at IS NULL AND pr.unlinked_at IS NULL))) AS unlinked_account_devices,
        (SELECT count(*) FROM device_keys d JOIN tenant_class tc ON tc.tenant_id = d.tenant_id
          WHERE d.revoked_at IS NULL AND tc.ns_class = 'legacy'
            AND d.onboarding_origin <> 'invite') AS legacy_devices_not_invite,
        (SELECT count(*) FROM accounts a
          WHERE a.ns_class = 'legacy'
            AND NOT EXISTS (SELECT 1 FROM pooled p WHERE p.tenant_id = a.tenant_id)
            AND NOT EXISTS (SELECT 1 FROM device_keys d
                             WHERE d.tenant_id = a.tenant_id AND d.onboarding_origin = 'invite'))
            AS legacy_accounts_without_invite,
        CASE WHEN (SELECT has_v81 FROM params) THEN
            (SELECT count(DISTINCT c.legacy_tenant_id) FROM conflicts c
              WHERE NOT EXISTS (SELECT 1 FROM pooled p WHERE p.tenant_id = c.legacy_tenant_id))
        END AS ambiguous_tenants
),
output(ord, section, bucket, value) AS (
    SELECT 1, 'source', 'v81_legacy_invite_link', CASE WHEN (SELECT has_v81 FROM params) THEN 1 ELSE 0 END
    UNION ALL
    SELECT 10, 'legacy_invite_tenants', 'total', (SELECT count(*) FROM classified)
    UNION ALL
    SELECT 11, 'legacy_invite_tenants', 'pooled',
           CASE WHEN (SELECT has_v81 FROM params)
                THEN (SELECT count(*) FROM classified WHERE class = 'pooled') END
    UNION ALL
    SELECT 12, 'legacy_invite_tenants', 'individual',
           CASE WHEN (SELECT has_v81 FROM params)
                THEN (SELECT count(*) FROM classified WHERE class = 'individual') END
    UNION ALL
    SELECT 13, 'legacy_invite_tenants', 'unclassified',
           (SELECT count(*) FROM classified WHERE class = 'unclassified')
    UNION ALL
    SELECT 14, 'legacy_invite_tenants', 'review_before_linking',
           (SELECT count(*) FROM classified
             WHERE class <> 'pooled' AND (max_uses > 3 OR invites > 1))
    UNION ALL
    SELECT 15, 'legacy_invite_tenants', 'instance_enrolled_not_invite',
           (SELECT count(*) FROM instance_tenants)
    UNION ALL
    SELECT 20, 'devices', c.class || ':active',
           (SELECT coalesce(sum(active_devices), 0) FROM classified x WHERE x.class = c.class)
    FROM (SELECT DISTINCT class FROM classified) c
    UNION ALL
    SELECT 21, 'devices', c.class || ':revoked',
           (SELECT coalesce(sum(revoked_devices), 0) FROM classified x WHERE x.class = c.class)
    FROM (SELECT DISTINCT class FROM classified) c
    UNION ALL
    SELECT 22 + b.bucket_ord, 'devices_per_tenant', b.class || ':' || b.bucket, count(*)
    FROM device_buckets b GROUP BY b.class, b.bucket, b.bucket_ord
    UNION ALL
    SELECT 300, 'accounts', 'wallet:open',
           (SELECT count(*) FROM accounts WHERE ns_class = 'near-')
    UNION ALL
    SELECT 301, 'accounts', 'wallet:anchored',
           (SELECT count(*) FROM accounts WHERE ns_class = 'near-' AND anchored)
    UNION ALL
    SELECT 302, 'accounts', 'near_ai:open',
           (SELECT count(*) FROM accounts WHERE ns_class = 'nearai-')
    UNION ALL
    SELECT 303, 'accounts', 'near_ai:anchored',
           (SELECT count(*) FROM accounts WHERE ns_class = 'nearai-' AND anchored)
    UNION ALL
    SELECT 304, 'accounts', 'legacy:open',
           (SELECT count(*) FROM accounts WHERE ns_class = 'legacy')
    UNION ALL
    SELECT 400, 'links', 'individual:linked',
           CASE WHEN (SELECT has_v81 FROM params)
                THEN (SELECT count(*) FROM classified WHERE class = 'individual' AND is_linked) END
    UNION ALL
    SELECT 401, 'links', 'individual:unlinked',
           CASE WHEN (SELECT has_v81 FROM params)
                THEN (SELECT count(*) FROM classified WHERE class = 'individual' AND NOT is_linked) END
    UNION ALL
    SELECT 402, 'links', 'pooled:linked',
           CASE WHEN (SELECT has_v81 FROM params)
                THEN (SELECT count(*) FROM classified WHERE class = 'pooled' AND is_linked) END
    UNION ALL
    SELECT 403, 'links', 'accounts_holding_a_link',
           CASE WHEN (SELECT has_v81 FROM params)
                THEN (SELECT count(DISTINCT (tenant_id, account_id)) FROM links) END
    UNION ALL
    SELECT 500, 'ambiguous', 'tenants_claimed_by_more_than_one_account',
           (SELECT ambiguous_tenants FROM readiness)
    UNION ALL
    SELECT 501, 'ambiguous', 'open_second_claims',
           CASE WHEN (SELECT has_v81 FROM params)
                THEN (SELECT count(*) FROM conflicts c
                       WHERE NOT EXISTS (SELECT 1 FROM pooled p WHERE p.tenant_id = c.legacy_tenant_id)) END
    UNION ALL
    SELECT 600, 'readiness_blockers', '1_account_namespace_devices_without_linkage',
           (SELECT unlinked_account_devices FROM readiness)
    UNION ALL
    SELECT 601, 'readiness_blockers', '2_legacy_devices_not_invite_onboarded',
           (SELECT legacy_devices_not_invite FROM readiness)
    UNION ALL
    SELECT 602, 'readiness_blockers', '3_legacy_accounts_without_an_invite',
           (SELECT legacy_accounts_without_invite FROM readiness)
    UNION ALL
    SELECT 603, 'readiness_blockers', '4_ambiguous_tenants',
           (SELECT ambiguous_tenants FROM readiness)
    UNION ALL
    SELECT 604, 'readiness_blockers', 'total',
           CASE WHEN (SELECT has_v81 FROM params)
                THEN (SELECT unlinked_account_devices + legacy_devices_not_invite
                             + legacy_accounts_without_invite + ambiguous_tenants FROM readiness) END
)
SELECT section, bucket, value
FROM output
ORDER BY ord, bucket;

ROLLBACK;
