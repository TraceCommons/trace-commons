-- Salt the NEAR account anchor (#716).
--
-- Before this migration `trace_near_account_anchors.anchor_hash` was
-- sha256(domain || network || account_name). Every input is public and the
-- domain prefix is in open source, so the digest took no secret, and V58's
-- `CHECK (tenant_id = 'near-' || substring(anchor_hash from 8))` made the
-- tenant id a function of that digest. Anyone holding a NEAR account name
-- could compute that contributor's tenant id offline.
--
-- The fix decouples the two. `anchor_hash` becomes a blind index --
-- HMAC(pepper, network || account_name) -- whose only job is to find an
-- existing anchor when a contributor signs in again, and `tenant_id` becomes
-- random. Peppering the old derivation in place was rejected: `anchor_hash` is
-- half a primary key and the target of a foreign key, so a peppered primary key
-- could never be rotated, and losing the pepper would orphan every wallet
-- account silently.

-- Legacy rows cannot be carried across. A pre-V61 row's anchor was computed
-- from an account name that was never stored, so there is nothing to recompute
-- the blind index from: the row would survive but its owner would never match
-- it again and would silently be given a second tenant. Refusing here is loud
-- and reversible; the alternative is a silent identity split. No such rows are
-- expected -- wallet provisioning is default-disabled -- and an operator who
-- has them needs a decision, not a migration.
DO $$
DECLARE
    legacy bigint;
BEGIN
    SELECT count(*) INTO legacy FROM trace_near_account_anchors;
    IF legacy > 0 THEN
        RAISE EXCEPTION 'V61 refuses to run: % pre-salting anchor rows exist and cannot be re-indexed (their account names were never stored)', legacy;
    END IF;
END $$;

-- Drop V58's tenant/anchor binding. It is unnamed in V58, so Postgres generated
-- the name; find it by definition rather than guessing at the generated form.
DO $$
DECLARE
    constraint_name text;
BEGIN
    SELECT conname INTO constraint_name
    FROM pg_constraint
    WHERE conrelid = 'trace_near_account_anchors'::regclass
      AND contype = 'c'
      AND pg_get_constraintdef(oid) LIKE '%substring(anchor_hash%';
    IF constraint_name IS NOT NULL THEN
        EXECUTE format('ALTER TABLE trace_near_account_anchors DROP CONSTRAINT %I', constraint_name);
    END IF;
END $$;

-- The sealed account name is the only copy of the name we keep, and it exists
-- solely so the pepper can be rotated. It is AES-256-GCM ciphertext under a
-- per-row DEK wrapped by the configured KMS key -- the same KEK/DEK envelope
-- the trace artifact store uses -- so a database disclosure without application
-- secrets yields ciphertext and a keyed index, and neither key is in the
-- database. Storing the name in plaintext would defeat the pepper outright,
-- since a leaked backup is exactly the scenario the pepper answers.
--
-- NOT NULL with no default, which the emptiness check above makes safe. There
-- is deliberately no nullable path: a row that could be written without a seal
-- is a row that can be written without a key, and that is the unsalted defect
-- returning by the back door.
ALTER TABLE trace_near_account_anchors
    ADD COLUMN sealed_account_name JSONB NOT NULL,
    -- Which pepper and which key this row is indexed and sealed under, so
    -- rotation can select the rows still under a superseded one. Both are
    -- hash-shaped labels, not references to key material.
    ADD COLUMN index_pepper_ref TEXT NOT NULL,
    ADD COLUMN account_name_key_ref TEXT NOT NULL;

CREATE INDEX trace_near_account_anchor_pepper_ref
    ON trace_near_account_anchors(index_pepper_ref);

-- Re-assert the tenant isolation V58 established. The new columns inherit the
-- table's forced RLS, but stating it here keeps the guarantee readable in the
-- migration that introduced them.
ALTER TABLE trace_near_account_anchors ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_near_account_anchors FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON trace_near_account_anchors;
CREATE POLICY trace_corpus_tenant_isolation ON trace_near_account_anchors
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());

-- A returning contributor is now recognised by their blind index and by nothing
-- else, so the index must be resolved to a tenant before a tenant exists to
-- scope the query to. This is the same shape as V30's login-resolver grant on
-- trace_login_links and V33's on trace_near_identities: a role-scoped
-- permissive SELECT policy, because a column GRANT alone still leaves the
-- restrictive tenant policy denying every row to a session with no tenant
-- context. Permissive policies OR together, so the PUBLIC path keeps full
-- tenant isolation -- this policy is `TO trace_login_resolver` only.
--
-- Safe without a tenant predicate because anchor_hash is globally UNIQUE: the
-- resolver can learn at most one tenant per index it already holds, and the
-- index is not computable without the pepper. Do NOT widen this grant to a
-- non-unique column, and do NOT add sealed_account_name to it -- the resolver
-- role has no tenant context and must never be able to read the sealed names.
GRANT SELECT (tenant_id, anchor_hash) ON trace_near_account_anchors TO trace_login_resolver;
DROP POLICY IF EXISTS trace_login_resolver_near_anchor_read ON trace_near_account_anchors;
CREATE POLICY trace_login_resolver_near_anchor_read ON trace_near_account_anchors
    FOR SELECT TO trace_login_resolver
    USING (true);
