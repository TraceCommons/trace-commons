# Export fixture baseline and repair

The optional PostgreSQL failure was reproduced on main commit
`f310792f4a6ca93d03204d05b7865e09083bbed2` (the merged #603 base), before the
onboarding changes. The detached baseline checkout was restored clean afterward.

Test: `store_facade_preserves_export_grant_job_scope_and_updates` in
`crates/trace-commons-server/tests/trace_corpus_pg_rls.rs`.

The baseline failed at line 3165, `insert stale alpha export job`, with PostgreSQL
SQLSTATE `23503`: `trace_export_jobs_tenant_id_grant_id_fkey`. The newly generated
stale job's grant ID did not exist in `trace_export_access_grants`. The failure
therefore predates V58's device-key changes; it is not an inferred attribution.
The full local output was retained at `/tmp/onboarding-export-main-baseline.log`.

Command, against a fresh isolated loopback database named
`admission_test_export_main` (set `ISOLATED_EXPORT_TEST_DATABASE_URL` to its URL):

```sh
RUSTFLAGS='-D warnings' \
CARGO_TARGET_DIR=/tmp/trace-commons-inference-funding-target \
TRACE_COMMONS_PG_TEST_DATABASE_URL="$ISOLATED_EXPORT_TEST_DATABASE_URL" \
cargo test -p trace-commons-server --test trace_corpus_pg_rls \
  store_facade_preserves_export_grant_job_scope_and_updates --locked --offline
```

Build-only variation: unrelated binary target registration was temporarily
removed (`autobins=false`, explicit `[[bin]]` entries omitted) to avoid linking
all server executables. Library, test and migration source, dependency versions
and feature settings were unchanged. The original manifest was then restored
byte-for-byte; no baseline edit is committed.

The repair creates the stale job's matching tenant-scoped export grant, then
uses that grant ID in the stale job. It preserves every existing assertion,
recovery operation and schema constraint. It does not weaken the foreign key.

Final review-tree validation (2026-09-05), compiled with `RUSTFLAGS=-D warnings`:

- Entire `trace_corpus_pg_rls` suite against a fresh real PostgreSQL database:
  **32 passed**, zero failures/ignored (12.74 seconds, `--test-threads=1`).
- `restricted_migrator_loses_guard_membership_and_retention_preserves_replay`:
  **1 passed**, including runtime retention-guard grant/refusal/revoke/recovery.
- `atomic_admission_replay_budget_recovery_and_rls`: **1 passed**.

Final harness builds used the same temporary exclusion of unrelated binary
registration, restored afterward. After switching the shared isolated target cache
from main back to the review tree, stale package artifacts were cleared and the
three final harnesses rebuilt successfully before execution. This avoided treating
a stale main-library import failure as a source regression.

Local result logs: `/tmp/onboarding-final-rls-pg.log`,
`/tmp/onboarding-final-retention-pg.log`, `/tmp/onboarding-final-admission-pg.log`.
No live provider call, deployment or production database was involved.
