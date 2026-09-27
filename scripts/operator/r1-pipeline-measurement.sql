-- R1 measurement: how many submissions carry a certified redaction pipeline
-- on the published allowlist.
--
-- R1 (docs/superpowers/specs/2026-09-23-connect-and-forget-consent-design.md,
-- "R1. A complete redaction pipeline actually ran, per session") is met by a
-- verified witness certificate whose `redaction_policy_version` -- the value
-- the witness computes with `redaction_pipeline_version()` -- is EXACTLY one
-- of FULL_REDACTION_PIPELINE_VERSIONS (crates/trace-commons-protocol/src/
-- trace_contribution.rs). No prefix match, no normalisation. `full-pipeline`
-- is the witness's startup mode name, not a certified value, and never passes.
--
-- READ-ONLY. Counts only: no submission ids, tenant ids, account ids, paths
-- or content reach the output. A version string is printed only when it is
-- on the allowlist, is `full-pipeline`, or has the exact shape of a known
-- pipeline identifier; anything else is counted as `unrecognised`, because the
-- self-reported column holds whatever a contributor's envelope said.
--
-- Run (see docs/operator/r1-pipeline-measurement.md):
--
--   PGOPTIONS='-c default_transaction_read_only=on' \
--   psql "$DATABASE_URL" -X -v ON_ERROR_STOP=1 \
--     [-v window_start=2026-09-01T00:00:00Z] [-v window_end=2026-10-01T00:00:00Z] \
--     -f scripts/operator/r1-pipeline-measurement.sql
--
-- The whole measurement runs inside one READ ONLY, REPEATABLE READ
-- transaction that ends in ROLLBACK.
--
-- Sources, chosen by what the schema has (the first result set says which):
--
--   * trace_witness_certificate_evidence (V76+): the certificate the server
--     verified and stored, byte for byte. The ONLY source of a certified
--     version. Without it every R1 figure is NULL (not measurable), never 0.
--   * trace_submissions.redaction_pipeline_version (V1+): the envelope's
--     SELF-REPORTED version with the server's own suffixes appended. Reported
--     under `claimed_version_not_evidence`, with those suffixes stripped. It
--     is not R1 evidence: a client can run a classifier locally, or type any
--     string.
--   * trace_submissions.last_status_reason (V49+): `witness_admitted` marks a
--     submission a verified certificate kept out of the PII-backstop hold. A
--     LOWER BOUND on witnessed submissions: it is written only when the
--     certificate changed the outcome, and a later transition overwrites it.
--   * trace_submission_sessions (V78+): submission -> source session, for the
--     per-session figures. Without it only per-submission figures exist.
--
-- The three lists below are copies. tests/r1_pipeline_measurement.rs in the
-- server crate fails when one drifts from its source; update both together.

\set ON_ERROR_STOP on
\pset footer off

\if :{?window_start}
\else
\set window_start '-infinity'
\endif
\if :{?window_end}
\else
\set window_end 'infinity'
\endif

BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;

-- Every Trace Commons table is FORCE ROW LEVEL SECURITY. A role that does not
-- bypass it sees no rows without a tenant context, and the report would read
-- "nothing passes". Refuse instead of reporting zero.
SELECT coalesce((SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname = current_user), false)
    AS r1_sees_every_tenant
\gset
\if :r1_sees_every_tenant
\else
DO $$ BEGIN
    RAISE EXCEPTION 'R1MeasurementRoleSubjectToRls: connect as a role with BYPASSRLS (or the superuser); an RLS-bound role sees no rows and would report zero';
END $$;
\endif

SELECT
    to_regclass('trace_witness_certificate_evidence') IS NOT NULL AS r1_has_evidence,
    to_regclass('trace_submission_sessions') IS NOT NULL AS r1_has_sessions,
    to_regclass('_trace_commons_migrations') IS NOT NULL AS r1_has_ledger,
    EXISTS (
        SELECT 1 FROM pg_attribute
        WHERE attrelid = 'trace_submissions'::regclass
          AND attname = 'last_status_reason'
          AND NOT attisdropped
    ) AS r1_has_status_reason
\gset

\if :r1_has_evidence
\set r1_evidence 'SELECT tenant_id, submission_id, convert_from(certificate_json, ''UTF8'')::jsonb ->> ''redaction_policy_version'' AS certified_version FROM trace_witness_certificate_evidence'
\else
\set r1_evidence 'SELECT NULL::text AS tenant_id, NULL::uuid AS submission_id, NULL::text AS certified_version WHERE false'
\endif

\if :r1_has_sessions
\set r1_sessions 'SELECT tenant_id, submission_id, account_id::text || '':'' || encode(session_digest, ''hex'') AS session_key FROM trace_submission_sessions'
\else
\set r1_sessions 'SELECT NULL::text AS tenant_id, NULL::uuid AS submission_id, NULL::text AS session_key WHERE false'
\endif

\if :r1_has_status_reason
\set r1_status_reason 's.last_status_reason'
\else
\set r1_status_reason 'NULL::text'
\endif

\if :r1_has_ledger
\set r1_v76_applied '(SELECT applied_at FROM _trace_commons_migrations WHERE version = 76)'
\else
\set r1_v76_applied 'NULL::timestamptz'
\endif

-- Result set 1: which sources were used, and the window.
SELECT 'source' AS section, 'witness_evidence_table' AS bucket,
       CASE WHEN :'r1_has_evidence'::boolean THEN 'present' ELSE 'absent' END AS value
UNION ALL
SELECT 'source', 'session_mapping_table',
       CASE WHEN :'r1_has_sessions'::boolean THEN 'present' ELSE 'absent' END
UNION ALL
SELECT 'source', 'status_reason_column',
       CASE WHEN :'r1_has_status_reason'::boolean THEN 'present' ELSE 'absent' END
UNION ALL
SELECT 'source', 'v76_applied_at', coalesce((:r1_v76_applied)::text, 'n/a')
UNION ALL
SELECT 'source', 'window_start', :'window_start'::timestamptz::text
UNION ALL
SELECT 'source', 'window_end', :'window_end'::timestamptz::text;

-- Result set 2: counts.
WITH params AS (
    SELECT
        :'window_start'::timestamptz AS ws,
        :'window_end'::timestamptz AS we,
        :'r1_has_evidence'::boolean AS has_evidence,
        :'r1_has_sessions'::boolean AS has_sessions,
        :'r1_has_status_reason'::boolean AS has_status_reason,
        (:r1_v76_applied)::timestamptz AS v76_applied_at,
        ARRAY[
            -- BEGIN FULL_REDACTION_PIPELINE_VERSIONS
            'ironclaw-deterministic-secret-path-v3+privacy-filter-near-ai-v1',
            'ironclaw-deterministic-secret-path-v3+privacy-filter-self-hosted-v1',
            'ironclaw-deterministic-secret-path-v3+privacy-filter-sidecar-v2'
            -- END FULL_REDACTION_PIPELINE_VERSIONS
        ]::text[] AS allowlist,
        ARRAY[
            -- BEGIN ANCHOR_NAMESPACES
            'near-',
            'nearai-'
            -- END ANCHOR_NAMESPACES
        ]::text[] AS namespaces,
        -- Suffixes the SERVER appends to the stored version after receipt;
        -- stripped to recover what the client claimed. Historical spellings
        -- stay so older rows strip too.
        ARRAY[
            -- BEGIN SERVER_PIPELINE_SUFFIXES
            'server-rescrub-v1',
            'server-rescrub-v2',
            'near-ai-pii-backstop-v1'
            -- END SERVER_PIPELINE_SUFFIXES
        ]::text[] AS server_suffixes
),
subs AS (
    SELECT
        s.trace_id,
        s.received_at,
        coalesce(
            (SELECT ns FROM unnest(p.namespaces) AS ns
             WHERE starts_with(s.tenant_id, ns)
             ORDER BY length(ns) DESC LIMIT 1),
            'other') AS ns_class,
        ev.submission_id IS NOT NULL AS has_witness,
        ev.certified_version,
        CASE WHEN p.has_evidence
             THEN ev.submission_id IS NOT NULL
                  AND ev.certified_version = ANY (p.allowlist)
        END AS r1_pass,
        (SELECT string_agg(part, '+' ORDER BY i)
         FROM unnest(string_to_array(s.redaction_pipeline_version, '+'))
              WITH ORDINALITY AS t(part, i)
         WHERE part <> ALL (p.server_suffixes)) AS claimed_version,
        coalesce(:r1_status_reason = 'witness_admitted', false) AS witness_admitted_reason,
        ss.session_key
    FROM trace_submissions s
    CROSS JOIN params p
    LEFT JOIN (:r1_evidence) ev
        ON ev.tenant_id = s.tenant_id AND ev.submission_id = s.submission_id
    LEFT JOIN (:r1_sessions) ss
        ON ss.tenant_id = s.tenant_id AND ss.submission_id = s.submission_id
    WHERE s.received_at >= p.ws AND s.received_at < p.we
),
labelled AS (
    SELECT
        subs.*,
        CASE
            WHEN certified_version = ANY (p.allowlist) THEN 'allowlisted:' || certified_version
            WHEN certified_version = 'full-pipeline'
              OR certified_version ~ '^ironclaw-deterministic-secret-path-v[0-9]{1,3}(\+privacy-filter-(sidecar|near-ai|self-hosted)-v[0-9]{1,3})?$'
                THEN 'not_allowlisted:' || certified_version
            ELSE 'unrecognised'
        END AS certified_label,
        CASE
            WHEN claimed_version = ANY (p.allowlist) THEN 'allowlisted:' || claimed_version
            WHEN claimed_version = 'full-pipeline'
              OR claimed_version ~ '^ironclaw-deterministic-secret-path-v[0-9]{1,3}(\+privacy-filter-(sidecar|near-ai|self-hosted)-v[0-9]{1,3})?$'
                THEN 'not_allowlisted:' || claimed_version
            ELSE 'unrecognised'
        END AS claimed_label,
        coalesce(claimed_version = ANY (p.allowlist), false) AS claimed_allowlisted
    FROM subs CROSS JOIN params p
),
classes AS (
    SELECT unnest(namespaces) AS ns_class FROM params
    UNION ALL SELECT 'other'
),
sessions AS (
    SELECT
        session_key,
        bool_and(coalesce(r1_pass, false)) AS every_pass,
        bool_or(coalesce(r1_pass, false)) AS some_pass
    FROM subs
    WHERE session_key IS NOT NULL
    GROUP BY session_key
),
output(ord, section, bucket, value) AS (
    SELECT 10, 'totals', 'submissions', (SELECT count(*) FROM subs)
    UNION ALL
    SELECT 11, 'totals', 'distinct_trace_ids', (SELECT count(DISTINCT trace_id) FROM subs)
    UNION ALL
    SELECT 12, 'totals', 'r1_pass',
           CASE WHEN (SELECT has_evidence FROM params)
                THEN (SELECT count(*) FROM subs WHERE r1_pass) END
    UNION ALL
    SELECT 13, 'totals', 'r1_fail',
           CASE WHEN (SELECT has_evidence FROM params)
                THEN (SELECT count(*) FROM subs WHERE NOT r1_pass) END
    UNION ALL
    SELECT 14, 'totals', 'claimed_allowlisted_not_evidence',
           (SELECT count(*) FROM labelled WHERE claimed_allowlisted)
    UNION ALL
    SELECT 15, 'totals', 'received_before_v76_applied',
           CASE WHEN (SELECT has_evidence FROM params)
                THEN (SELECT count(*) FROM subs, params
                      WHERE subs.received_at < params.v76_applied_at) END
    UNION ALL
    SELECT 20, 'witness', 'evidence_present',
           CASE WHEN (SELECT has_evidence FROM params)
                THEN (SELECT count(*) FROM subs WHERE has_witness) END
    UNION ALL
    SELECT 21, 'witness', 'evidence_present_r1_pass',
           CASE WHEN (SELECT has_evidence FROM params)
                THEN (SELECT count(*) FROM subs WHERE has_witness AND r1_pass) END
    UNION ALL
    SELECT 22, 'witness', 'evidence_absent',
           CASE WHEN (SELECT has_evidence FROM params)
                THEN (SELECT count(*) FROM subs WHERE NOT has_witness) END
    UNION ALL
    SELECT 23, 'witness', 'status_reason_witness_admitted',
           CASE WHEN (SELECT has_status_reason FROM params)
                THEN (SELECT count(*) FROM subs WHERE witness_admitted_reason) END
    UNION ALL
    SELECT 30, 'certified_version', certified_label, count(*)
    FROM labelled WHERE has_witness GROUP BY certified_label
    UNION ALL
    SELECT 40, 'namespace', c.ns_class || ':submissions',
           (SELECT count(*) FROM subs WHERE subs.ns_class = c.ns_class)
    FROM classes c
    UNION ALL
    SELECT 41, 'namespace', c.ns_class || ':evidence_present',
           CASE WHEN (SELECT has_evidence FROM params)
                THEN (SELECT count(*) FROM subs WHERE subs.ns_class = c.ns_class AND has_witness) END
    FROM classes c
    UNION ALL
    SELECT 42, 'namespace', c.ns_class || ':r1_pass',
           CASE WHEN (SELECT has_evidence FROM params)
                THEN (SELECT count(*) FROM subs WHERE subs.ns_class = c.ns_class AND r1_pass) END
    FROM classes c
    UNION ALL
    SELECT 50, 'claimed_version_not_evidence', claimed_label, count(*)
    FROM labelled GROUP BY claimed_label
    UNION ALL
    SELECT 60, 'sessions', 'sessions', (SELECT count(*) FROM sessions)
    WHERE (SELECT has_sessions FROM params)
    UNION ALL
    SELECT 61, 'sessions', 'every_submission_r1_pass',
           (SELECT count(*) FROM sessions WHERE every_pass)
    WHERE (SELECT has_sessions FROM params)
    UNION ALL
    SELECT 62, 'sessions', 'some_submission_r1_pass',
           (SELECT count(*) FROM sessions WHERE some_pass)
    WHERE (SELECT has_sessions FROM params)
    UNION ALL
    SELECT 63, 'sessions', 'submissions_without_session',
           (SELECT count(*) FROM subs WHERE session_key IS NULL)
    WHERE (SELECT has_sessions FROM params)
)
SELECT section, bucket, value
FROM output
ORDER BY ord, bucket;

ROLLBACK;
