-- Organizations: one tenant per org, a seat per member and per server, and a
-- holder account that owns the org's settled credit
-- (docs/superpowers/specs/2026-11-11-enterprise-gateway-design.md, 4.1-4.2).
--
-- trace_orgs                      one row per org tenant: status, the holder
--                                 account, the bound NEAR AI organization's
--                                 blind index, the current policy and limits.
-- trace_org_seats                 one row per member, server or holder
--                                 account in the org.
-- trace_org_enrolment_authorities how seats are admitted.
-- trace_org_seat_devices          which registered device key belongs to
--                                 which seat. Org seats have no NEAR login
--                                 anchor, so trace_near_provisioned_devices
--                                 (keyed to an anchor) cannot hold them.
--
-- FAIL CLOSED. A tenant in the org- namespace with no trace_orgs row, or one
-- whose status is not 'active', is refused by the issuer, by admission and at
-- landing. Nothing here is read for any other tenant.
--
-- HASH ONLY. No column holds an org name, a NEAR AI org id, a person's
-- identity or a label: near_ai_org_index and identity_index are keyed blind
-- indexes under their own domains (`org::NEAR_AI_ORG_BINDING_DOMAIN`,
-- `org::ORG_MEMBER_IDENTITY_DOMAIN`), so they cannot be joined to a login
-- anchor or a payout subject index. Seat and org labels are not stored in
-- this version; they arrive sealed, with the sealing.
--
-- ROLES. org_role is a seat role read from this table on every org console
-- request. It is deliberately not a value in the trace_tenant_access_grants
-- role CHECK and not a bearer-token role.
--
-- RUNTIME. trace_ingest_runtime may SELECT and INSERT, and UPDATE only the
-- listed lifecycle columns. It has no DELETE: a seat leaves by removed_at, an
-- authority by revoked_at, and rows leave only with their tenant.
--
-- Applied by a non-superuser CREATEROLE migrator: it creates no role
-- (trace_ingest_runtime exists since V90), and GRANT on a table needs only
-- its ownership.

CREATE TABLE IF NOT EXISTS trace_orgs (
    tenant_id             TEXT PRIMARY KEY
                          REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE
                          CHECK (tenant_id ~ '^org-[0-9a-f]{64}$'),
    status                TEXT NOT NULL DEFAULT 'active'
                          CHECK (status IN ('active', 'suspended', 'closing')),
    holder_account_id     UUID,
    credit_mode           TEXT NOT NULL DEFAULT 'org' CHECK (credit_mode IN ('org')),
    near_ai_org_index     TEXT UNIQUE
                          CHECK (near_ai_org_index ~ '^sha256:[0-9a-f]{64}$'),
    policy                JSONB NOT NULL,
    policy_version        BIGINT NOT NULL DEFAULT 1 CHECK (policy_version >= 1),
    policy_digest         TEXT NOT NULL CHECK (policy_digest ~ '^sha256:[0-9a-f]{64}$'),
    seat_limit            INTEGER NOT NULL CHECK (seat_limit >= 0),
    server_limit          INTEGER NOT NULL CHECK (server_limit >= 0),
    settlement_cap_micros BIGINT CHECK (settlement_cap_micros > 0),
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY (tenant_id, holder_account_id)
        REFERENCES trace_accounts(tenant_id, account_id)
);

CREATE TABLE IF NOT EXISTS trace_org_enrolment_authorities (
    tenant_id      TEXT NOT NULL REFERENCES trace_orgs(tenant_id) ON DELETE CASCADE,
    authority_id   UUID NOT NULL,
    kind           TEXT NOT NULL CHECK (kind IN (
                       'near_ai_org_membership', 'enrolment_token',
                       'oidc_issuer', 'workload_issuer')),
    seat_kind      TEXT NOT NULL CHECK (seat_kind IN ('member', 'server')),
    binding_hash   TEXT CHECK (binding_hash ~ '^sha256:[0-9a-f]{64}$'),
    max_seats      INTEGER CHECK (max_seats > 0),
    seats_admitted INTEGER NOT NULL DEFAULT 0 CHECK (seats_admitted >= 0),
    expires_at     TIMESTAMPTZ,
    revoked_at     TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, authority_id),
    -- An external issuer names what it binds; the other kinds bind nothing.
    CHECK ((kind IN ('oidc_issuer', 'workload_issuer')) = (binding_hash IS NOT NULL)),
    CHECK (max_seats IS NULL OR seats_admitted <= max_seats)
);

CREATE TABLE IF NOT EXISTS trace_org_seats (
    tenant_id            TEXT NOT NULL REFERENCES trace_orgs(tenant_id) ON DELETE CASCADE,
    account_id           UUID NOT NULL,
    kind                 TEXT NOT NULL CHECK (kind IN ('member', 'server', 'holder')),
    org_role             TEXT CHECK (org_role IN ('member', 'org_admin')),
    consent_basis        TEXT CHECK (consent_basis IN ('member_grant', 'org_policy')),
    identity_index       TEXT CHECK (identity_index ~ '^sha256:[0-9a-f]{64}$'),
    authority_id         UUID,
    membership_proved_at TIMESTAMPTZ,
    joined_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    removed_at           TIMESTAMPTZ,
    removed_by_ref       TEXT CHECK (removed_by_ref ~ '^sha256:[0-9a-f]{64}$'),
    PRIMARY KEY (tenant_id, account_id),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_accounts(tenant_id, account_id) ON DELETE CASCADE,
    FOREIGN KEY (tenant_id, authority_id)
        REFERENCES trace_org_enrolment_authorities(tenant_id, authority_id),
    UNIQUE (tenant_id, identity_index),
    -- Only a member seat has an org role, and every member seat has one.
    CHECK ((kind = 'member') = (org_role IS NOT NULL)),
    -- A seat sends under exactly one basis: a member under its own grant, a
    -- server under the org's policy, the holder never.
    CHECK ((kind = 'member' AND consent_basis = 'member_grant')
        OR (kind = 'server' AND consent_basis = 'org_policy')
        OR (kind = 'holder' AND consent_basis IS NULL)),
    -- A seat that sends was admitted by a proof; the holder was not admitted.
    CHECK ((kind = 'holder') = (identity_index IS NULL)),
    CHECK (kind = 'member' OR membership_proved_at IS NULL),
    CHECK ((removed_at IS NULL) = (removed_by_ref IS NULL))
);

CREATE TABLE IF NOT EXISTS trace_org_seat_devices (
    tenant_id     TEXT NOT NULL REFERENCES trace_orgs(tenant_id) ON DELETE CASCADE,
    device_key_id TEXT NOT NULL REFERENCES device_keys(device_key_id),
    account_id    UUID NOT NULL,
    principal_ref TEXT NOT NULL,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Set for a short-lived key under a workload seat (spec 7.1); NULL for a
    -- key that lives until it is revoked.
    expires_at    TIMESTAMPTZ,
    PRIMARY KEY (tenant_id, device_key_id),
    UNIQUE (tenant_id, principal_ref),
    FOREIGN KEY (tenant_id, account_id)
        REFERENCES trace_org_seats(tenant_id, account_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS trace_org_seat_devices_account
    ON trace_org_seat_devices (tenant_id, account_id);

-- One holder per org.
CREATE UNIQUE INDEX IF NOT EXISTS trace_org_seats_one_holder
    ON trace_org_seats (tenant_id) WHERE kind = 'holder';

ALTER TABLE trace_orgs ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_orgs FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_orgs;
CREATE POLICY trace_corpus_tenant_isolation ON trace_orgs
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_org_enrolment_authorities ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_org_enrolment_authorities FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_org_enrolment_authorities;
CREATE POLICY trace_corpus_tenant_isolation ON trace_org_enrolment_authorities
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_org_seat_devices ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_org_seat_devices FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_org_seat_devices;
CREATE POLICY trace_corpus_tenant_isolation ON trace_org_seat_devices
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

ALTER TABLE trace_org_seats ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_org_seats FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_org_seats;
CREATE POLICY trace_corpus_tenant_isolation ON trace_org_seats
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

REVOKE ALL ON trace_orgs, trace_org_enrolment_authorities, trace_org_seats,
              trace_org_seat_devices FROM PUBLIC;

GRANT SELECT, INSERT ON trace_orgs TO trace_ingest_runtime;
GRANT UPDATE (status, holder_account_id, near_ai_org_index, policy, policy_version,
              policy_digest, updated_at)
    ON trace_orgs TO trace_ingest_runtime;

GRANT SELECT, INSERT ON trace_org_enrolment_authorities TO trace_ingest_runtime;
GRANT UPDATE (seats_admitted, revoked_at)
    ON trace_org_enrolment_authorities TO trace_ingest_runtime;

GRANT SELECT, INSERT ON trace_org_seats TO trace_ingest_runtime;
GRANT UPDATE (org_role, membership_proved_at, removed_at, removed_by_ref)
    ON trace_org_seats TO trace_ingest_runtime;

GRANT SELECT, INSERT ON trace_org_seat_devices TO trace_ingest_runtime;
