# Backend authorization repairs

This change repairs the five finding families from the local backend audit
(`3d2a7b8ab`, separate branch). It does not import the audit's historical route
inventory or change deployment configuration.

- Empty policy/grant scope lists still mean unrestricted, but a restricted
  caller or tenant ceiling can no longer authorize such a target.
- `principal_legacy` is unknown ownership, not contributor authority. Exact
  credential aliases and active same-account membership remain supported;
  tenant-scoped reviewer/admin recovery remains explicit and audited.
- Legacy replay, benchmark and ranker eligibility checks the stored expiry
  deadline. Sources are revalidated after awaited work before publication,
  including file-backed sources. Optional mirrors cannot override reviewer or
  replay metadata authority. Revalidation checks the earliest source
  deadline again after object-reference lookups. Legal hold preserves storage;
  it does not extend export permission past the deadline.
- Verified wallet enrollment uses atomic database admission. Weak sessions
  count and insert under an account FOR UPDATE lock with a fresh READ COMMITTED
  statement snapshot. Immediate account FKs serialize provisioning and merge
  additions; strong-session enrollment remains supported.
- PostgreSQL readiness checks applicable permissive policies and effective role
  reachability. Referral startup also verifies the intended narrow resolver
  grants. Unsafe inherited resolver access, extra widening policies and
  reachable owner/bypass authority refuse readiness.

Details: [scope and legacy ownership](legacy-scope.md),
[bootstrap serialization](bootstrap.md), [RLS readiness](rls.md).

## Models and source correspondence

The five `.qnt` models express the repaired decisions. The source tests are
primary evidence of executable behavior; a modeled predicate is not proof that
every route invokes it correctly. `authorization.qnt` abstracts credential
validation and active principal expansion and checks selected guard families.
Its observations occur at an atomic authorization decision, not the final
network byte or arbitrary asynchronous database commit. Expiry models file/DB authority, optional mirrors, stale-copy deadlines and
revocation explicitly. It uses the same guard-point abstraction; previously
delivered exports are not recalled. Historical wrong-authority and missing-
deadline actions are negative controls excluded from the repaired step.

`scope-escalation.qnt` compares stored effective permissions with the caller's
ceiling. The identity model distinguishes lock acquisition, fresh admission,
commit and a trusted strong writer. It starts after independent wallet proof
verification. The RLS model represents **application access after a required
readiness check**: bad policy/grant configurations still exist in PostgreSQL,
but the application refuses to start under the checked configuration. Existing
opt-in readiness configuration remains unchanged; the model does not assert
every deployment has enabled it.

All checks are finite, **bounded symbolic exploration**, not unbounded source
proofs. The seven selected checks explore up to eight transitions. The explicit
witness/control runner executes every named run and rejects a zero-test run.
The symbolic runner owns its solver, fingerprints the backend/model inputs and
rejects timeouts, unsupported expressions, changed sources and missing solver
completion. No process-success-only proof classification is accepted.

```sh
python3 docs/security/backend-authorization-fixes/test_models.py --out /private/tmp/auth-fix-witnesses.json
python3 -m unittest discover -s docs/security/backend-authorization-fixes -p test_check.py
python3 docs/security/backend-authorization-fixes/check.py --out /private/tmp/auth-fix-symbolic --port 8871
```

Quint 0.33.0, Apalache 0.62.1 and Java 21 were used. Override `--java` and
`--jar` for another host. Output paths must be fresh and outside the repository.
No dependency is added by this PR.

## Compatibility and rollout

Ownerless historical records become unavailable to ordinary contributor reads
and self-service mutation. Establish ownership from trusted evidence before any
separate recovery; never reconstruct authority from envelope attribution. The
repair does not migrate or delete these records. Legal-held expired payloads
remain stored while new exports exclude them.

RLS checks are deliberately conservative: logically equivalent custom policy
expressions that are not the recognized tenant predicate require review rather
than being accepted as equivalent. Test and correct operator role grants before
rollout; [the resolver runbook](../../operator/referral-resolver-role.md) describes
the intended non-inherited transition. Catalog checks assume trusted database
administrators and cannot prevent an administrator changing policies afterward.

No migration, dependency, automatic merge or deployment is included. Production
legacy-record counts and runtime role configuration were not queried. The PR
contains regression tests and bounded model evidence; rollout remains separate.

## Verified result

The final local run passed **1,950 ingest tests** and **2,819 server tests**
across all 60 server test executables, plus two doctests. The 63 existing ignored
tests across these runs remain reported. PostgreSQL suites used fresh local
PostgreSQL 16.15 databases and the fixture variables specified by CI. All five
models typechecked; all **29 named witness/control runs** and **seven bounded
checks through eight transitions** passed. Formatting, bin checks, Clippy with
the repository allow-list, four license-boundary tests and eight evidence-
classifier tests passed.

Independent source and model reviews found and closed a derived-export
metadata-authority gap. The actual PostgreSQL regression demonstrates a live
stale mirror beside an expired/revoked authoritative file, correct behavior in
both authority modes, and refusal after required object refs are invalidated.

[Evidence manifest](evidence/manifest.json), [validation summary](evidence/validation.json)
and [per-server-suite results](evidence/server-suites.json) retain commands,
source/model hashes, individual results and selected compressed logs. These
results concern the local implementation, not a production exposure check or
an unbounded proof of every backend endpoint.
