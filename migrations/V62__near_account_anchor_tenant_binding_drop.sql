-- Actually drop V58's tenant/anchor binding, which V61 only appeared to drop.
--
-- V61 decoupled `tenant_id` from `anchor_hash`: the anchor became a blind index
-- and the tenant id became random, so that holding a NEAR account name no
-- longer yields that contributor's tenant id. V58's
-- `CHECK (tenant_id = 'near-' || substring(anchor_hash from 8))` is the
-- constraint that has to go for a random tenant id to be insertable at all, and
-- V61 went looking for it by rendered definition:
--
--     AND pg_get_constraintdef(oid) LIKE '%substring(anchor_hash%'
--
-- Postgres does not render it that way. It normalises the keyword and prints
-- `SUBSTRING(anchor_hash FROM 8)`, so the LIKE is case-sensitively looking for
-- text that never appears. The SELECT found nothing, `constraint_name` stayed
-- NULL, the `IF` guarded the drop away, and the DO block completed without
-- error. V61 reported success on every database it has ever run against while
-- changing nothing about the constraint.
--
-- The consequence is not cosmetic. The constraint still forces the tenant id to
-- be a substring of the anchor -- exactly the derivation V61 exists to remove --
-- so wallet provisioning cannot insert its row:
--
--     ERROR:  new row for relation "trace_near_account_anchors" violates
--             check constraint "trace_near_account_anchors_check"
--
-- Wallet provisioning is default-disabled, which is why this has not been seen
-- in service.
--
-- This migration does not repeat V61's mistake of matching on rendered SQL.
-- `pg_get_constraintdef` output is a formatting decision that belongs to the
-- server version, not an interface: it is free to change keyword case, add
-- parentheses, or expand a function name in any release. The constraint is
-- identified structurally instead, by what it is attached to -- a table-level
-- CHECK (`contype = 'c'`) over exactly the two columns `tenant_id` and
-- `anchor_hash` (`conkey`) -- which is a property of the catalogue rather than
-- of a printer.
DO $$
DECLARE
    doomed text;
BEGIN
    SELECT c.conname INTO doomed
    FROM pg_constraint AS c
    WHERE c.conrelid = 'trace_near_account_anchors'::regclass
      AND c.contype = 'c'
      AND (
          SELECT array_agg(a.attname::text ORDER BY a.attname)
          FROM unnest(c.conkey) AS k(attnum)
          JOIN pg_attribute AS a
            ON a.attrelid = c.conrelid AND a.attnum = k.attnum
      ) = ARRAY['anchor_hash', 'tenant_id'];

    IF doomed IS NOT NULL THEN
        EXECUTE format(
            'ALTER TABLE trace_near_account_anchors DROP CONSTRAINT %I', doomed);
    END IF;
END $$;

-- Assert the outcome rather than the attempt.
--
-- This is the half V61 lacked, and the reason its failure was silent: a drop
-- guarded by `IF found THEN` is indistinguishable, on success, from a drop that
-- found nothing. Re-reading the catalogue afterwards and refusing makes the
-- next such mistake loud at migration time instead of at a contributor's first
-- wallet sign-in. It is also what makes this migration safe to re-run and safe
-- on a database where V61 somehow did work: finding nothing to drop is fine,
-- finding something still there afterwards is not.
DO $$
DECLARE
    surviving text;
BEGIN
    SELECT c.conname INTO surviving
    FROM pg_constraint AS c
    WHERE c.conrelid = 'trace_near_account_anchors'::regclass
      AND c.contype = 'c'
      AND (
          SELECT array_agg(a.attname::text ORDER BY a.attname)
          FROM unnest(c.conkey) AS k(attnum)
          JOIN pg_attribute AS a
            ON a.attrelid = c.conrelid AND a.attnum = k.attnum
      ) = ARRAY['anchor_hash', 'tenant_id'];

    IF surviving IS NOT NULL THEN
        RAISE EXCEPTION
            'V62 could not remove the tenant/anchor binding: constraint % still constrains (tenant_id, anchor_hash), so a random tenant id remains uninsertable',
            surviving;
    END IF;
END $$;
