# Pipeline qualification

> **Status: the tooling exists.** `scripts/operator/pipeline.py` runs the
> commands this runbook describes today, against a real, disposable
> PostgreSQL server. Production routing of live tenants through the pipeline
> is still off; it arrives with the activation work in PR 5. Every report
> `qualify` writes says so (`production_promotion_ready: false`), and every
> pass carries the same seven local blockers. Do not treat a passing local
> qualification as a production promotion.

`scripts/operator/pipeline.py` is the operator entrypoint for the versioned
pipeline's local test tooling. Python 3 standard library only; it starts and
tears down its own PostgreSQL server, drives the Rust test suites and the
shared `trace-commons-ingest` app (hosted in-process by an ignored Rust
test -- no launcher binary, no cargo feature), and writes bounded,
hash-only reports. It never prints a child process's stdout or stderr,
only the step label, the exit code, and the path to that step's own
protected log file.

See [pipeline-lab.md](pipeline-lab.md) for `pipeline.py run` and
`pipeline.py package` (one corpus, one bundle, at a time). This runbook
covers `pipeline.py test`, `pipeline.py qualify`, and
`pipeline.py restore-drill` -- the commands that exercise every required
check together.

## What each command checks

### `pipeline.py test [--check contracts|runtime|postgres] [--postgres-admin-url URL]`

Runs one or more of three check groups (default: `contracts` and `runtime`;
repeat `--check` for more than one):

- **`contracts`** -- no database. The gate-API and server-crate pipeline unit
  tests (`trace-commons-gate-api --lib pipeline::`,
  `versioned_pipeline_bundle`, `versioned_pipeline_qualification`), and
  `scripts/operator/pipeline-deployment-inventory.py --check`, which
  classifies routes, operations, adapters, tables, object namespaces,
  telemetry destinations, and roles.
- **`runtime`** -- no database. The server-crate's own `versioned_pipeline`
  unit tests, the `trace-commons-ingest` bin's `pipeline_runtime` tests, and
  `scripts/operator/test_pipeline_tooling.py` (the tooling's own self-tests;
  a binding check like every other step in this group).
- **`postgres`** -- one `Environment` (below), one scenario per step: pipeline
  storage upgrade RLS, migration atomicity, the `versioned_pipeline_runtime_pg`
  integration suite, the real HTTP receipt-restart test, the compatibility
  bundle over HTTP, and legacy/pipeline parity.

### `pipeline.py qualify [--archive] [--postgres-admin-url URL]`

Runs every check `qualify` requires, in one environment, and refuses to
finish unless each one has a **current, passing** result (see "The result
contract" below). In order:

1. The contract test manifest's digest
   (`docs/superpowers/specs/2026-09-11-versioned-pipeline-contract-test-manifest.json`).
2. Every binding check by exit status: `contracts` and `runtime` from
   `pipeline.py test` above, including the tooling self-tests. The
   deployment inventory writes its output to the run directory and its
   `inventory_digest` becomes a report input.
3. Three corpus runs are prepared (corpus digest and, for the third, the HF
   export -- see [pipeline-lab.md](pipeline-lab.md)) before any database
   starts.
4. One `Environment` opens, and each of these gets its own fresh scenario
   database and artifact root, in order:
   - migration atomicity;
   - twelve required database checks -- pipeline storage-upgrade RLS, crash
     matrix, independent-instrument retry, stale-lease fencing, lease
     renewal, exact receipt replay, payout crash recovery, index rebuild,
     the orphan sweep, bundle qualification inspection, and the two real-HTTP
     receipt checks (restart recovery, replay ownership);
   - the three corpus runs (`minimal`, `compatibility`, `hf_local`), each
     through the same code path `pipeline.py run` uses;
   - the restore drill (below).
5. After the environment closes, `qualify` refuses if cleanup itself failed
   (a leftover scenario database, lock database, or container), then
   requires a current pass result for all sixteen checks.
6. The report is written to `.local/pipeline-qualification-report.json`
   (and only archived with `--archive`).

`qualify` first removes the previous `.local` report, so an older passing
report never stays the latest one. A failed run writes its report --
`status: fail` with the safe label that failed it -- before the command
exits nonzero. An interrupt (Ctrl-C) writes the report under
`qualify_interrupted` and exits 130. The only failed run that leaves no
report is one whose report itself cannot be written (for example, a full
disk). On success:

```
PipelineQualificationOK: report=.local/pipeline-qualification-report.json checks=16
PipelineQualificationScope: production_promotion_ready=false blockers=local_reference_scorer,local_reference_embedder,synthetic_index,synthetic_settlement,static_bearer_authentication,filesystem_restore_local_only,hf_network_canary_not_run -- local evidence only, not a production promotion
```

With the workspace already built, a full local run takes about 1.5 minutes
(measured: 1:27). A cold build adds the compile time on top of that.

### `pipeline.py restore-drill [--postgres-admin-url URL]`

One local restore drill, standalone (`qualify` runs the same steps as its
last scenario). In order: seed a pipeline database and encrypted artifact
directory with a pending run, plus one completed run for a second tenant;
dump the seed database with `pg_dump`; create a sibling database in the
same cluster and restore into it with `pg_restore`; copy the artifact
directory and compare every file byte for byte; check the restored
database before anything resumes; resume the pending run against the
restored database and artifacts, and require it to reach the same
settlement legs and Trace Credit ledger event the seed produced, with no
duplicate effect.

The checks before the resume, each with its own failure label in the
resume's protected log:

| Check | Label |
| --- | --- |
| Every table in `TRACE_COMMONS_RLS_TABLES` has its `trace_corpus_tenant_isolation` policy, and the policy's `USING` and `WITH CHECK` expressions are the tenant predicate on `trace_current_tenant_id()` (the diagnostic `trace_corpus_pg_rls.rs` reads) | `restore_rls_policy_predicate_mismatch` |
| Every one of those tables enables and forces RLS | `restore_rls_not_enabled_and_forced` |
| Every RLS policy in the schema is the seed's, field for field: table, name, command, permissive or restrictive, roles, and both expressions. PostgreSQL ORs permissive policies, so a policy added beside the tenant policy, or one whose roles widen, would open a table the two checks above call isolated | `restore_rls_policy_set_changed` |
| The runtime login holds every privilege it held before the dump, of every type: tables, columns, sequences, functions, and the schema | `restore_runtime_privileges_changed` |
| Every tenant's `main` audit chain verifies (`main`'s own verifier). Checked before the fingerprint below, which covers the audit rows too, so a changed audit row reports here | `restore_audit_chain_broken` |
| Every tenant's rows in those tables (count and row hash per table and tenant, read by the owner) equal the seed's | `restore_tenant_fingerprint_mismatch` |
| The audit chains hold the seed's hashed events | `restore_audit_event_count_mismatch` |

On success it prints a second line with this evidence (the table count,
the policy set's hash and size, the privilege set's hash and size, the
tenant count and fingerprint, and the hashed audit events verified):

```
PipelineRestoreOK: database=sha256:... artifacts=sha256:... index=sha256:... legs_per_run=1 credit_events_per_run=1 pending_runs_resumed=1 duplicate_effects=0
PipelineRestoreChecks: rls_tables=96 rls_policies=sha256:... rls_policy_count=165 runtime_privileges=sha256:... runtime_privilege_count=412 tenants=2 tenant_fingerprint=sha256:... audit_events_verified=2
PipelineRestoreScope: filesystem_restore_local_only -- the artifact restore is a local filesystem copy, local evidence only, not a remote object-store restore
```

See [backup-restore.md](backup-restore.md#versioned-pipeline) for what this
does and does not prove about a real restore.

## The environment: container digest, `--postgres-admin-url`, one server at a time

Every command above that touches a database opens one `Environment`. With
no `--postgres-admin-url`, it starts its own disposable PostgreSQL 16
container, pinned by digest so every run uses the same bytes
(`postgres@sha256:a3b7f434b2dc57ce85a67e171163eb8ab1a1ebcb39d27484661f26b1dfbe30d6`),
on a loopback port Docker assigns, and removes the container on exit --
confirmed removed with `docker ps -a`, not just asked for, so a container
that outlives its process fails the run's own cleanup check.

With `--postgres-admin-url postgres://trace@127.0.0.1:<port>/postgres` (host
must be the literal `127.0.0.1`, user must be `trace`), the command uses
that server instead. It creates a `pipeline_tooling_lock` database as an
exclusive lock for the run's duration: a second `pipeline.py` command
against the same server fails closed with `pipeline_tooling_server_busy`
rather than racing the first one's scenario databases. Run one
`pipeline.py` command at a time against a given PostgreSQL server.

Every check that needs a database gets its own scenario: a fresh
`admission_test_<run>_<NN>` (and, where needed, `pipeline_test_<run>_<NN>`)
database, dropped when the environment closes, plus its own encrypted
artifact directory under the run's own directory. Nothing shares a database
across scenarios, so one check's rows can never leak into another's
transaction-count guard.

## The result contract, and what makes a result invalid

Every check that passes writes a `trace_commons.pipeline_check_result.v1`
JSON file (`<check id>.result.json`) plus a paired `<check id>.evidence.json`
in the run's `results/` directory. A result carries: `schema`, `run_id`,
`check_id`, `status` (`pass`, `fail`, or `blocked`), `code_revision_hash`,
`package_hash`/`configuration_digest`/`dependency_digest` (present when the
check binds a package), `observed_at`, `evidence_hash`, and `safe_blockers`.

`qualify` (and `run`, for its own corpus check) requires every check it
lists to have a result that is **current**, meaning all of:

- present at all (`check_result_missing:<id>` otherwise);
- from this exact run (`check_result_foreign_run` otherwise -- a stray
  result left over from another `pipeline.py` invocation never counts);
- from this exact code revision (`check_result_foreign_revision`);
- observed between the run's start and now (`check_result_stale`);
- `status: pass` (`check_result_blocked:<id>` or `check_result_failed:<id>`
  otherwise);
- carrying its required digests, for checks that must bind a package
  (`check_result_digest_missing:<id>`).

A result file that is empty or not valid JSON (a test that died while it
wrote its result leaves an empty one) fails with
`check_result_schema_invalid`, the same label as a result of the wrong
shape; an evidence file that is not valid JSON fails with
`check_evidence_malformed`. `observed_at` may carry up to nine fractional
digits (Rust writes nine on Linux); the tooling reads it to the microsecond
on every Python 3 it supports, including 3.9 and 3.10.

The evidence file must independently hash to the result's `evidence_hash`
and pass the evidence validator (labels, `sha256:` hashes, and ISO
timestamps only -- no URL, tenant id, trace text, or secret-shaped value).
Both `pipeline.py test --check postgres` and `qualify` also require the
scenario database it just ran in to show at least 5 committed transactions
(`database_check_executed_nothing:<step>`) -- a pass count alone is not
evidence the check reached PostgreSQL.

Promotion reads results through `evaluate_promotion`
(`crates/trace-commons-server/src/versioned_pipeline_qualification.rs`),
which has no caller in the running service yet. Beyond one current, passing
result per required check, it requires every result to carry the same
`code_revision_hash` (`qualification_evidence_mixed_revision` otherwise) and
every result that names a package to name the same one, all three digests
together (`qualification_evidence_mixed_package` otherwise). Its decision
names that revision and package, and its `evidence_hash` covers each
result's run id, code revision, package digests and evidence hash. A
`qualify` run's checks bind many packages (most harnesses build their own
test bundle; the last run's 15 package-bearing results named 10), so its
results as a whole are mixed: evidence that promotes one package has to
come from runs of the package-bearing checks against that package.

## Outputs under `.local/`

- `.local/pipeline-qualification-report.json` -- the latest qualification
  report, `pass` or `fail`.
- `.local/pipeline-<label>-corpus-report.json` / `.md` -- the latest corpus
  report of each label (`minimal`, `compatibility`, `hf_local`, `package`)
  from `pipeline.py run`. See [pipeline-lab.md](pipeline-lab.md).
- `.local/pipeline-lab-catalog.json` -- written only by `--archive` (either
  command). See "`--archive` and the catalog" below.
- `.local/pipeline/hf-cache/` -- the shared, git-ignored HuggingFace cache
  directory `export_hf_corpus` uses. Harmless and untouched by a local
  fixture pin, since that mode never downloads anything.
- `.local/pipeline/runs/<run id>/` -- one directory per invocation, mode
  `0700`, always holding `logs/` (one protected log per step, mode `0600`),
  `results/` (every result and evidence file the run produced), and
  `artifacts/` (one encrypted artifact root per scenario the run opened).
  Depending on the command: `qualify` also writes `inventory.json` and its
  own copy of the latest report as `qualification-report.json`;
  `restore-drill` (and the last scenario of `qualify`) writes
  `restore-fingerprint.json`; `run` (and `qualify`'s corpus scenarios) write
  `corpus-report-<label>.json`. Nothing here is deleted automatically; clean
  it up like any other build output when disk space matters.

In CI, a failed `pipeline qualification and restore` job uploads
`.local/pipeline/runs/` -- every run directory except its encrypted
`artifacts/` -- as the workflow artifact `pipeline-qualification-runs`,
kept for 7 days, so the failing step's protected log, the results, the
evidence, and the report can be read without reproducing the run. Every
value in them comes from the synthetic fixtures and test constants; the
artifact master key only ever reaches a child process's environment.

## `--archive` and the catalog

`pipeline.py run --archive` and `pipeline.py qualify --archive` are the only
commands that write `.local/pipeline-lab-catalog.json`
(`trace_commons.pipeline_lab_catalog.v1`). A routine run without `--archive`
never touches it.

- A corpus report (`run --archive`) is added under `bundles`, keyed by
  `bundle_id`, as before.
- A qualification report (`qualify --archive`) is added under a
  `qualifications` list, together with its records: the run's three corpus
  reports and the HF export's `source-manifest.json`. Each record is
  validated against its own schema, and each must belong to the report it
  is archived with (a corpus report by check id, bundle, package, and report
  digest; the manifest by its two corpus digests matching the `hf_local`
  run) -- `qualification_corpus_mismatch` or `qualification_manifest_mismatch`
  otherwise.

Every archived file is written once under `lab-records/`, named by its own
content digest. Archiving the same report and records again is a no-op: the
catalog comes out byte-identical. A different report that happens to land
on the same digest is refused (`immutable_record_conflict`) rather than
silently overwriting the first one. Two different real qualification runs
each add their own entry -- the catalog is append-only, and an earlier
entry is never rewritten by a later archive.

## What local evidence is not

Every qualification report always carries `production_promotion_ready:
false`, `external_payout_enabled: false`, and these seven blockers, because
every check runs against local or synthetic dependencies:

- `local_reference_scorer`, `local_reference_embedder` -- the reference
  perplexity scorer and embedder, not a production-qualified model.
- `synthetic_index` -- the in-memory index, not a durable production one.
- `synthetic_settlement` -- the recording settlement adapter, not a real
  payout rail.
- `static_bearer_authentication` -- the harnesses configure static test
  tenant tokens, not the production credential path.
- `filesystem_restore_local_only` -- the restore drill's artifact recovery
  is a local filesystem copy, never a remote object-store restore.
- `hf_network_canary_not_run` -- the HF corpus run uses the local JSONL
  fixture pin (`pin-local.json`); no traces were actually downloaded from
  Hugging Face.

Closing these is promotion work, not part of this PR:

- A remote-provider restore drill (GCS or another configured object store,
  not a local directory copy).
- Real production adapters standing in for the reference scorer, embedder,
  index, and settlement adapter.
- The HF network canary: a real download against the pinned dataset
  (`jedisct1/security-audits`), which arrives with the PR that needs it
  (ruling HF-1). No PR 4 check hits the network.

## Package trust and `qualify_bundle`

`pipeline.py package` (see [pipeline-lab.md](pipeline-lab.md)) builds and
signs a `BundlePackage` with the existing canonical-hashing and Ed25519
signature format, verified against a `BundlePackageTrustStore`. Signing a
package with this command does not qualify it for production; it only
proves the package is well-formed and correctly signed.

The production path is `PipelineQualificationStore::qualify_bundle`
(`crates/trace-commons-server/src/versioned_pipeline_qualification.rs`), a
Rust API this repository exposes today with no admin HTTP route in front of
it. It takes a signed package, a trust store, qualification metadata, a
`ProductionDependencyProfile`, and the `evaluate_promotion` decision over the
evidence the metadata records. It fails closed on an untrusted or tampered
package, a non-production or development package, a profile built for a
different bundle, a configuration or dependency-digest mismatch, any
blocked dependency (a compatibility configuration that is not qualifiable,
such as all three floors zero, blocks as
`bundle_configuration_not_qualifiable`), a decision that does not back the
metadata, or a second
call for the same bundle with different metadata. The decision must be
ready (`bundle_qualification_promotion_not_ready`), its evidence hash must
be the metadata's (`bundle_qualification_evidence_mismatch`), its one code
revision the metadata's (`bundle_qualification_code_revision_mismatch`),
and its one package the signed package
(`bundle_qualification_package_mismatch`). A successful call records one
append-only row per `(tenant_id, bundle_id)` in
`pipeline_bundle_qualifications` (migration V107).

Calling `qualify_bundle` over HTTP, and using its record to activate a
bundle for a tenant's live traffic, are PR 5 work. See
[pipeline-activation.md](pipeline-activation.md) for what activation adds
once that lands.
