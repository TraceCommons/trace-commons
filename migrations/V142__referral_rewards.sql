-- Referral rewards: codes, who referred whom, the referrer's 10% bonus on
-- the NEAR AI rail, and a second payout stream per link that carries it
-- (docs/superpowers/specs/2026-10-29-referral-rewards-design.md).
--
-- A referrer earns a bonus of `bps` (default 1000, so 10%) of each credit
-- accrual a referred account earns on a rail during the referee's earning
-- period, which starts at the referee's account creation and lasts at least
-- 92 days (owner, 2026-10-29: "3 months promised but it can be extended").
-- The bonus is on top: nothing here reduces or reads back the referee's own
-- accruals, lines or links. Trace Credit is untouched.
--
-- TENANTS. Each NEAR AI sign-in gets its own nearai- tenant, so referrer and
-- referee are in different tenants. Every table here is tenant data under
-- forced RLS and the central trace_corpus_tenant_isolation policy:
--   * trace_referral_codes, trace_referral_referees,
--     trace_referral_bonus_accruals live in the REFERRER's tenant;
--   * trace_referral_attributions lives in the REFEREE's tenant;
--   * trace_referral_network_marks lives in each account's own tenant.
-- A referral is applied in one transaction that writes the attribution
-- under the referee's tenant context and the referee mirror under the
-- referrer's, re-setting trace_commons.trace_tenant_id between statements.
-- The bonus pass reads the referee tenant, then writes the referrer tenant
-- in its own transaction; its insert is idempotent, so a crash between the
-- two re-runs harmlessly.
--
-- CROSS-TENANT READS. Resolving a code to its referrer is the only one. It
-- goes through trace_referral_resolver, a NOLOGIN NOBYPASSRLS role with a
-- column grant and a permissive SELECT policy on trace_referral_codes only,
-- assumed with SET LOCAL ROLE, as V30 does for trace_login_resolver. Like
-- the payout roles (V132) it is not granted to trace_ingest_runtime here;
-- the operator grants it to the ingest login with INHERIT FALSE
-- (docs/operator/referral-resolver-role.md), and ingest refuses to start
-- with referrals on until it can assume it (referral_resolver_unprovisioned).
--
-- THE REFERRER'S TENANT ID on an attribution is stored in the clear: the
-- bonus pass needs it to write the referrer's rows, and a tenant id is a
-- random label, not an identity. It carries no foreign key, so deleting the
-- referrer's tenant is never blocked by another tenant's row; the pass
-- treats a missing referrer as void. Everything a referrer can read about a
-- referee is a domain-separated hash (referee_ref_hash) and keyed blind
-- indexes already used by the payout tables.
--
-- PAYOUT STREAMS. A link's existing program carries the account's own
-- earned credit (stream 'earned'). Referral bonus goes out on a second
-- program on the same link (stream 'referral'), so what came from one's own
-- traces and what came from referrals stay separate on NEAR AI's side too,
-- and a dedicated referral credential can cap the second. One pending line
-- per link per stream; a link closes with each stream's final total.
--
-- NUMBERING. V141 is the highest on main on 2026-10-29; V129 stays reserved
-- by #1415.
--
-- AMOUNTS are rail atomic units, NUMERIC(39,0) bounded to u128::MAX, as V132.
--
-- NETWORK MARKS. The client is open source, so nothing it sends is abuse
-- evidence (owner, 2026-10-29). trace_referral_network_marks records what
-- the server observes itself: a keyed hash of the network prefix (IPv4 /24,
-- IPv6 /48) the trusted edge saw an account in the referral program call
-- from. The key (TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY) never reaches the
-- database, so a database reader cannot brute-force the 2^24 /24s back out
-- of a mark. The bonus pass compares a referee's marks with the referrer's;
-- a match holds the referral stream and is never counted toward a code's
-- suspension. Marks are kept for the earning period plus 30 days.
--
-- RUNTIME GRANTS, on V105's terms: what the store reads and writes, nothing
-- broader. Audit and bonus accruals are append-only (SELECT, INSERT).

DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_ingest_runtime') THEN
        RAISE EXCEPTION 'V142: trace_ingest_runtime is missing; V90 creates it';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_referral_resolver') THEN
        CREATE ROLE trace_referral_resolver NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles
                WHERE rolname = 'trace_referral_resolver'
                  AND (rolsuper OR rolbypassrls OR rolcanlogin)) THEN
        RAISE EXCEPTION 'V142: trace_referral_resolver is privileged or can log in';
    END IF;
END $$;
GRANT USAGE ON SCHEMA public TO trace_referral_resolver;

-- Codes, in the referrer's tenant. The code itself is sealed (as payout
-- recipients are) with a keyed blind index; code_index is unique across
-- every tenant, so one code names one referrer. A code is live while none
-- of disabled_at (its owner turned it off), suspended_at (the abuse check
-- paused it) or voided_at (an operator closed it) is set. A suspension
-- records the signal labels that tripped it.
CREATE TABLE trace_referral_codes (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    code_id UUID NOT NULL,
    account_id UUID NOT NULL,
    code_index TEXT NOT NULL CHECK (code_index ~ '^sha256:[0-9a-f]{64}$'),
    sealed_code JSONB NOT NULL CHECK (jsonb_typeof(sealed_code) = 'object'),
    code_key_ref TEXT NOT NULL CHECK (char_length(code_key_ref) BETWEEN 1 AND 128),
    index_pepper_ref TEXT NOT NULL CHECK (char_length(index_pepper_ref) BETWEEN 1 AND 128),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    disabled_at TIMESTAMPTZ,
    suspended_at TIMESTAMPTZ,
    suspension_labels TEXT[] NOT NULL DEFAULT '{}' CHECK (
        cardinality(suspension_labels) <= 16
        AND array_position(suspension_labels, NULL) IS NULL
    ),
    voided_at TIMESTAMPTZ,
    void_label TEXT CHECK (void_label ~ '^[a-z0-9_.-]{1,64}$'),
    PRIMARY KEY (tenant_id, code_id),
    CONSTRAINT trace_referral_codes_suspension_shape CHECK (
        (suspended_at IS NULL) = (cardinality(suspension_labels) = 0)
    ),
    CONSTRAINT trace_referral_codes_void_shape CHECK (
        (voided_at IS NULL) = (void_label IS NULL)
    ),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE NO ACTION
);
CREATE UNIQUE INDEX trace_referral_codes_one_index ON trace_referral_codes (code_index);
CREATE INDEX trace_referral_codes_account ON trace_referral_codes (tenant_id, account_id);
CREATE INDEX trace_referral_codes_pepper_ref ON trace_referral_codes (index_pepper_ref);

-- Attributions, in the referee's tenant: at most one referrer per account,
-- ever. referee_account_created_at is copied from trace_accounts when the
-- code is applied; the earning period runs from it, and its length is a
-- setting (at least 92 days) read when the bonus is priced, so an extension
-- needs no data change. A referral within one tenant is refused here as
-- well as in the store.
CREATE TABLE trace_referral_attributions (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id UUID NOT NULL,
    code_index TEXT NOT NULL CHECK (code_index ~ '^sha256:[0-9a-f]{64}$'),
    referrer_tenant_id TEXT NOT NULL CHECK (char_length(referrer_tenant_id) BETWEEN 1 AND 128),
    referrer_account_id UUID NOT NULL,
    referee_ref_hash TEXT NOT NULL CHECK (referee_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    referee_account_created_at TIMESTAMPTZ NOT NULL,
    applied_via TEXT NOT NULL CHECK (applied_via IN ('signup', 'later')),
    applied_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    voided_at TIMESTAMPTZ,
    void_label TEXT CHECK (void_label ~ '^[a-z0-9_.-]{1,64}$'),
    PRIMARY KEY (tenant_id, account_id),
    CONSTRAINT trace_referral_attributions_not_self CHECK (referrer_tenant_id <> tenant_id),
    CONSTRAINT trace_referral_attributions_void_shape CHECK (
        (voided_at IS NULL) = (void_label IS NULL)
    ),
    CONSTRAINT trace_referral_attributions_applied_after_creation CHECK (
        applied_at >= referee_account_created_at
    ),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE NO ACTION
);
CREATE UNIQUE INDEX trace_referral_attributions_one_ref
    ON trace_referral_attributions (referee_ref_hash);

-- The referrer's view of each referee, in the referrer's tenant: a hash,
-- never the referee's tenant or account. The bonus pass refreshes the
-- referee's abuse signal labels and the blind indexes of their payout link
-- (recipient_index, subject_index, as on trace_payout_links), which the
-- per-code abuse check reads to find clusters among a code's referees.
CREATE TABLE trace_referral_referees (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    referee_ref_hash TEXT NOT NULL CHECK (referee_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    account_id UUID NOT NULL,
    code_index TEXT NOT NULL CHECK (code_index ~ '^sha256:[0-9a-f]{64}$'),
    referee_account_created_at TIMESTAMPTZ NOT NULL,
    applied_at TIMESTAMPTZ NOT NULL,
    signal_labels TEXT[] NOT NULL DEFAULT '{}' CHECK (
        cardinality(signal_labels) <= 16
        AND array_position(signal_labels, NULL) IS NULL
    ),
    recipient_index TEXT CHECK (recipient_index ~ '^sha256:[0-9a-f]{64}$'),
    subject_index TEXT CHECK (subject_index ~ '^sha256:[0-9a-f]{64}$'),
    signals_updated_at TIMESTAMPTZ,
    voided_at TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, referee_ref_hash),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE NO ACTION
);
CREATE INDEX trace_referral_referees_code
    ON trace_referral_referees (tenant_id, code_index, applied_at);
CREATE INDEX trace_referral_referees_account
    ON trace_referral_referees (tenant_id, account_id);

-- Bonus accruals, in the referrer's tenant: one row per referee credit
-- accrual (bonus) or reversal accrual (reversal) on a rail, priced once and
-- never re-priced, plus a void row per referee that an operator voided,
-- which takes back the referee's unsent bonus. base_units is the referee
-- accrual's rail_units; bonus_units = floor(|base_units| * bps / 10000),
-- negated for a reversal or a void. A void row has no source accrual and
-- base_units 0.
CREATE TABLE trace_referral_bonus_accruals (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    bonus_id UUID NOT NULL,
    rail_id TEXT NOT NULL CHECK (rail_id ~ '^[a-z0-9_.-]{1,64}$'),
    account_id UUID NOT NULL,
    referee_ref_hash TEXT NOT NULL CHECK (referee_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    kind TEXT NOT NULL CHECK (kind IN ('bonus', 'reversal', 'void')),
    source_accrual_id UUID,
    bps INTEGER NOT NULL CHECK (bps BETWEEN 1 AND 10000),
    base_units NUMERIC(39, 0) NOT NULL,
    bonus_units NUMERIC(39, 0) NOT NULL,
    rate_version TEXT NOT NULL CHECK (rate_version ~ '^[A-Za-z0-9_.-]{1,64}$'),
    instrument_id TEXT NOT NULL CHECK (instrument_id ~ '^[a-z0-9_.-]{1,64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, bonus_id),
    CONSTRAINT trace_referral_bonus_accruals_key_shape CHECK (
        (kind IN ('bonus', 'reversal')) = (source_accrual_id IS NOT NULL)
    ),
    CONSTRAINT trace_referral_bonus_accruals_sign_shape CHECK (
        (kind = 'bonus' AND base_units > 0 AND bonus_units >= 0)
        OR (kind = 'reversal' AND base_units < 0 AND bonus_units <= 0)
        OR (kind = 'void' AND base_units = 0 AND bonus_units <= 0)
    ),
    CONSTRAINT trace_referral_bonus_accruals_bound CHECK (
        abs(base_units) <= 340282366920938463463374607431768211455
        AND abs(bonus_units) <= 340282366920938463463374607431768211455
    ),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE NO ACTION,
    FOREIGN KEY (tenant_id, referee_ref_hash)
        REFERENCES trace_referral_referees(tenant_id, referee_ref_hash) ON DELETE NO ACTION,
    FOREIGN KEY (rail_id, rate_version)
        REFERENCES trace_payout_rate_epochs(rail_id, version) ON DELETE NO ACTION
);
CREATE UNIQUE INDEX trace_referral_bonus_accruals_one_source
    ON trace_referral_bonus_accruals (tenant_id, rail_id, referee_ref_hash, source_accrual_id)
    WHERE kind IN ('bonus', 'reversal');
CREATE UNIQUE INDEX trace_referral_bonus_accruals_one_void
    ON trace_referral_bonus_accruals (tenant_id, rail_id, referee_ref_hash)
    WHERE kind = 'void';
CREATE INDEX trace_referral_bonus_accruals_owed
    ON trace_referral_bonus_accruals (tenant_id, rail_id, account_id);

-- Audit: append-only and hash-only, in whichever tenant the action touched.
CREATE TABLE trace_referral_audit (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    audit_id UUID NOT NULL,
    action TEXT NOT NULL CHECK (action IN (
        'code_created', 'code_disabled', 'code_suspended', 'code_resumed', 'code_voided',
        'referral_applied', 'referral_refused', 'referral_voided'
    )),
    actor_ref_hash TEXT NOT NULL CHECK (actor_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    code_index TEXT CHECK (code_index ~ '^sha256:[0-9a-f]{64}$'),
    referee_ref_hash TEXT CHECK (referee_ref_hash ~ '^sha256:[0-9a-f]{64}$'),
    label TEXT CHECK (label ~ '^[a-z0-9_.-]{1,64}$'),
    created_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, audit_id)
);
CREATE INDEX trace_referral_audit_time ON trace_referral_audit (tenant_id, created_at);

-- Network marks, in the account's own tenant: one row per account and mark,
-- with when the edge first and last saw it. The mark is
-- HMAC-SHA256(key, domain || canonical prefix) as sha256: + hex, so it is
-- comparable across tenants and opaque without the key.
CREATE TABLE trace_referral_network_marks (
    tenant_id TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
    account_id UUID NOT NULL,
    mark TEXT NOT NULL CHECK (mark ~ '^sha256:[0-9a-f]{64}$'),
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (tenant_id, account_id, mark),
    CONSTRAINT trace_referral_network_marks_seen_order CHECK (last_seen_at >= first_seen_at),
    FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id)
        ON DELETE NO ACTION
);
CREATE INDEX trace_referral_network_marks_seen
    ON trace_referral_network_marks (tenant_id, last_seen_at);

-- A second payout stream per link. Existing lines are the earned stream.
ALTER TABLE trace_payout_lines
    ADD COLUMN stream TEXT NOT NULL DEFAULT 'earned' CHECK (stream IN ('earned', 'referral'));
DROP INDEX trace_payout_lines_one_pending;
CREATE UNIQUE INDEX trace_payout_lines_one_pending
    ON trace_payout_lines (tenant_id, link_id, stream) WHERE state = 'pending';

-- The referral program of a link, set when its first referral line is
-- written, and the referral total recorded when the link closes. A link
-- that never carried referral credit closes with neither.
ALTER TABLE trace_payout_links
    ADD COLUMN referral_program TEXT CHECK (
        octet_length(referral_program) BETWEEN 1 AND 100
        AND referral_program ~ '^[A-Za-z0-9_.:-]+$'
    ),
    ADD COLUMN final_paid_referral NUMERIC(39, 0) CHECK (
        final_paid_referral >= 0
        AND final_paid_referral <= 340282366920938463463374607431768211455
    ),
    ADD CONSTRAINT trace_payout_links_final_paid_referral_shape CHECK (
        final_paid_referral IS NULL OR (state = 'closed' AND referral_program IS NOT NULL)
    ),
    ADD CONSTRAINT trace_payout_links_referral_program_distinct CHECK (
        referral_program IS NULL OR referral_program <> program
    );

-- Row-level security: the central predicate on every new table.
ALTER TABLE trace_referral_codes ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_referral_codes FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_referral_codes;
CREATE POLICY trace_corpus_tenant_isolation ON trace_referral_codes
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_referral_attributions ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_referral_attributions FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_referral_attributions;
CREATE POLICY trace_corpus_tenant_isolation ON trace_referral_attributions
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_referral_referees ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_referral_referees FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_referral_referees;
CREATE POLICY trace_corpus_tenant_isolation ON trace_referral_referees
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_referral_bonus_accruals ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_referral_bonus_accruals FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_referral_bonus_accruals;
CREATE POLICY trace_corpus_tenant_isolation ON trace_referral_bonus_accruals
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_referral_audit ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_referral_audit FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_referral_audit;
CREATE POLICY trace_corpus_tenant_isolation ON trace_referral_audit
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_referral_network_marks ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_referral_network_marks FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_referral_network_marks;
CREATE POLICY trace_corpus_tenant_isolation ON trace_referral_network_marks
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- The resolver reads which tenant and account own a code index, and whether
-- the code is live. Nothing else: not the sealed code, not other tables.
-- Permissive policies OR together, so for this role the effective USING is
-- (central predicate) OR true, and for every other role it is unchanged.
GRANT SELECT (tenant_id, account_id, code_index, disabled_at, suspended_at, voided_at)
    ON trace_referral_codes TO trace_referral_resolver;
DROP POLICY IF EXISTS trace_referral_resolver_cross_tenant_read ON trace_referral_codes;
CREATE POLICY trace_referral_resolver_cross_tenant_read ON trace_referral_codes
    FOR SELECT TO trace_referral_resolver
    USING (true);
-- No ALTER ROLE ... SET statement_timeout here: the role is NOLOGIN and
-- assumed only with SET LOCAL ROLE, which never loads role settings. Every
-- transaction that assumes it sets SET LOCAL statement_timeout itself
-- (REFERRAL_RESOLVER_ASSUME_SQL in db/postgres_referral.rs).
-- The RESET clears the setting an earlier draft of this file left on a
-- cluster's (global) role.
ALTER ROLE trace_referral_resolver RESET statement_timeout;

-- Runtime grants.
REVOKE ALL ON trace_referral_codes, trace_referral_attributions, trace_referral_referees,
    trace_referral_bonus_accruals, trace_referral_audit, trace_referral_network_marks
    FROM PUBLIC;
GRANT SELECT, INSERT ON trace_referral_codes TO trace_ingest_runtime;
GRANT UPDATE (disabled_at, suspended_at, suspension_labels, voided_at, void_label)
    ON trace_referral_codes TO trace_ingest_runtime;
GRANT SELECT, INSERT ON trace_referral_attributions TO trace_ingest_runtime;
GRANT UPDATE (voided_at, void_label) ON trace_referral_attributions TO trace_ingest_runtime;
GRANT SELECT, INSERT ON trace_referral_referees TO trace_ingest_runtime;
GRANT UPDATE (signal_labels, recipient_index, subject_index, signals_updated_at, voided_at)
    ON trace_referral_referees TO trace_ingest_runtime;
GRANT SELECT, INSERT ON trace_referral_bonus_accruals TO trace_ingest_runtime;
GRANT SELECT, INSERT ON trace_referral_audit TO trace_ingest_runtime;
GRANT UPDATE (referral_program, final_paid_referral) ON trace_payout_links
    TO trace_ingest_runtime;
-- Recording a mark refreshes last_seen_at; retention deletes old marks.
GRANT SELECT, INSERT, DELETE ON trace_referral_network_marks TO trace_ingest_runtime;
GRANT UPDATE (last_seen_at) ON trace_referral_network_marks TO trace_ingest_runtime;
-- The pass compares the duplicate clusters of a referee's and a referrer's
-- traces (V40's dedup_cluster_id, assigned from the cross-tenant simhash
-- scan, so one cluster id means the same content in any tenant). It reads
-- these columns of each side's own decisions, under that side's tenant.
GRANT SELECT (tenant_id, submission_id, dedup_cluster_id)
    ON trace_gate_decisions TO trace_ingest_runtime;
