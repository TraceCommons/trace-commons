# Pipeline qualification

> **Status: the tooling exists.** `scripts/operator/pipeline.py` runs the
> commands this runbook describes today, against a real, disposable
> PostgreSQL server. Production routing of live tenants through the pipeline
> is off until an operator activates a tenant (see
> [pipeline-activation.md](pipeline-activation.md)). Every report `qualify`
> writes says so (`production_promotion_ready: false`), and every pass carries
> the same seven local blockers. Do not treat a passing local qualification as
> a production promotion.

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
check together -- and `pipeline.py keygen` and `pipeline.py revision`, which
the signing of results needs.

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

### `pipeline.py qualify [--archive] [--postgres-admin-url URL] [--signing-key PATH --signing-key-id ID [--evidence-max-age-seconds N]]`

Runs every check `qualify` requires, in one environment, and refuses to
finish unless each one has a **current, passing** result (see "The result
contract" below). With `--signing-key` it then signs each result (see "Signed
check results" below). In order:

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
   - fifteen required database checks -- pipeline storage-upgrade RLS, crash
     matrix, independent-instrument retry, stale-lease fencing, lease
     renewal, exact receipt replay, payout crash recovery, index rebuild,
     the orphan sweep, bundle qualification inspection, the two real-HTTP
     receipt checks (restart recovery, replay ownership), and the three
     activation checks (containment, rollback, and the legacy drain report);
   - the three corpus runs (`minimal`, `compatibility`, `hf_local`), each
     through the same code path `pipeline.py run` uses;
   - the restore drill (below).
5. After the environment closes, `qualify` refuses if cleanup itself failed
   (a leftover scenario database, lock database, or container), then
   requires a current pass result for all nineteen checks.
6. With `--signing-key`, the tool checks that the source tree has not changed
   and signs the nineteen results into a staging directory.
7. The report is written to `.local/pipeline-qualification-report.json`
   (and only archived with `--archive`). With `--signing-key`, the signed files
   are then moved next to the results.

The nineteen checks, each with the test or run that produces its result, and
whether the result names the candidate package. Four checks name it; the other
fifteen are mechanics checks (or, for `pipeline_http_corpus_minimal`, run a test
bundle) and name none:

| Check id | Produced by | Names the package |
| --- | --- | --- |
| `pipeline_storage_upgrade_rls` | library test `pipeline_upgrade_from_v91_installs_forced_rls_storage` | no |
| `pipeline_crash_matrix` | `crash_matrix_produces_one_logical_effect_per_point` | no |
| `pipeline_independent_instruments` | `independent_instruments_retry_without_repeating_a_completed_one` | no |
| `pipeline_stale_lease_fence` | `stale_lease_cannot_commit_after_reclaim` | no |
| `pipeline_lease_renewal` | `a_score_longer_than_its_lease_completes_once_with_two_workers` | no |
| `pipeline_receipt_replay_exact` | `receipt_replay_and_conflict_are_exact` | no |
| `pipeline_payout_recovery` | `payout_crash_between_submit_and_confirm_submits_once` | no |
| `pipeline_index_rebuild` | `index_rebuild_uses_sealed_commands_without_new_credit_or_outcomes` | no |
| `pipeline_orphan_sweep` | `a_crashed_score_attempt_leaves_staged_objects_the_sweep_removes` | no |
| `pipeline_bundle_qualification` | `qualification_inspects_the_objects_the_constructor_receives` | yes |
| `pipeline_http_restart_recovery` | ingest test `real_http_receipt_completes_and_resumes_after_restart` | no |
| `pipeline_http_receipt_ownership` | ingest test `real_http_pipeline_receipt_checks_ownership_on_replay` | no |
| `pipeline_activation_containment` | ingest test `containment_refuses_new_receipts_and_keeps_pending_work` | no |
| `pipeline_activation_rollback` | `rollback_selects_an_earlier_bundle_for_new_runs_only` | no |
| `pipeline_legacy_drain` | ingest test `the_legacy_drain_report_counts_real_pending_work_and_reaches_zero` | no |
| `pipeline_http_corpus_minimal` | the minimal corpus run | no |
| `pipeline_http_corpus_compatibility` | the compatibility corpus run | yes |
| `pipeline_http_corpus_hf_local` | the HF-local corpus run | yes |
| `pipeline_restore_drill` | the restore drill | yes |

The tests without another name are in the `versioned_pipeline_runtime_pg` suite.
The package the four checks name is the compatibility bundle over the local
reference configuration: a development package, which the qualification route
refuses (`bundle_development_dependency`). A local run proves the mechanics. It
does not qualify a production bundle.

`qualify` first removes the previous `.local` report, so an older passing
report never stays the latest one. A failed run writes its report --
`status: fail` with the safe label that failed it -- before the command
exits nonzero. An interrupt (Ctrl-C) writes the report under
`qualify_interrupted` and exits 130. The only failed run that leaves no
report is one whose report itself cannot be written (for example, a full
disk). On success:

```
PipelineQualificationOK: report=.local/pipeline-qualification-report.json checks=19
PipelineQualificationScope: production_promotion_ready=false blockers=local_reference_scorer,local_reference_embedder,synthetic_index,synthetic_settlement,static_bearer_authentication,filesystem_restore_local_only,hf_network_canary_not_run -- local evidence only, not a production promotion
```

A signed run prints `checks=19 attested=19` on the first line. With the
workspace already built, a full local run takes about 2.5 minutes (measured
with the nineteen checks and signing: 2:34). A cold build adds the compile time
on top of that, and a run on a loaded machine can take much longer.

### `pipeline.py restore-drill [--postgres-admin-url URL]`

One local restore drill, standalone (`qualify` runs the same steps as its
last scenario). In order: seed a pipeline database and encrypted artifact
directory with a pending run, plus one completed run for a second tenant,
plus the activation state of a third tenant (a routing row and its event from
a containment, a suspended Score policy and its intervention, a qualification
row, and an index rebuild fence);
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
| Every table in the schema, not only the ones above, has the seed's two RLS flags (`relrowsecurity`, `relforcerowsecurity`), and the set of tables is the seed's. The policy set lives in `pg_policy`, which holds neither flag, so a table outside `TRACE_COMMONS_RLS_TABLES` whose RLS a restore disabled or un-forced would otherwise pass | `restore_rls_flags_changed` |
| The runtime login holds every privilege it held before the dump, of every type: tables, columns, sequences, functions, and the schema | `restore_runtime_privileges_changed` |
| Every tenant's `main` audit chain verifies (`main`'s own verifier). Checked before the fingerprint below, which covers the audit rows too, so a changed audit row reports here | `restore_audit_chain_broken` |
| Every tenant's rows in those tables (count and row hash per table and tenant, read by the owner) equal the seed's | `restore_tenant_fingerprint_mismatch` |
| The audit chains hold the seed's hashed events | `restore_audit_event_count_mismatch` |
| The third tenant's activation state came back: one row in each of `pipeline_tenant_routing`, `pipeline_activation_events`, `pipeline_policy_interventions`, `pipeline_bundle_qualifications`, and `pipeline_index_rebuild_fences`, the routing state `contained`, and the Score policy still suspended. The tenant fingerprint above covers the content of these rows | `restore_activation_state_row_missing`, `restore_routing_state_changed`, `restore_policy_suspension_lost` |

On success it prints a second line with this evidence (the table count,
the policy set's hash and size, the RLS flag set's hash and table count,
the privilege set's hash and size, the tenant count and fingerprint, and
the hashed audit events verified):

```
PipelineRestoreOK: database=sha256:... artifacts=sha256:... index=sha256:... legs_per_run=1 credit_events_per_run=1 pending_runs_resumed=1 duplicate_effects=0
PipelineRestoreChecks: rls_tables=96 rls_policies=sha256:... rls_policy_count=165 rls_flags=sha256:... rls_flag_tables=114 runtime_privileges=sha256:... runtime_privilege_count=412 tenants=2 tenant_fingerprint=sha256:... audit_events_verified=2
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
(`crates/trace-commons-server/src/versioned_pipeline_qualification.rs`). The
qualification and activation routes call it over signed results (see
[pipeline-activation.md](pipeline-activation.md)). It needs one current,
passing result for each of the 22 checks in `PROMOTION_REQUIRED_CHECKS`: the 19
that `qualify` produces, and three promotion-only checks
(`pipeline_production_adapters`, `pipeline_remote_restore`,
`pipeline_hf_network_canary`) that need the production assembly. No code in
this repository emits those three (see "What local evidence is not"). It
also requires:

- every result to carry the same `code_revision_hash`
  (`qualification_evidence_mixed_revision` otherwise);
- the 19 results that `qualify` produces to carry the same `run_id`
  (`qualification_evidence_mixed_run` otherwise). The evidence of a bundle is
  the output of one `qualify` run plus the three promotion-only results. A set
  cannot take one result from one `qualify` run and the rest from another run,
  on the same revision or not. The three promotion-only results can come from
  other runs: their run ids are not compared. `PROMOTION_ONLY_CHECKS` holds
  the three ids. The rule applies at the qualification, at the activation, and
  at the rollback;
- no result to carry a safe blocker, a passing one included. Each blocker is
  listed as `<label>:<check_id>`. A local restore drill passes with the blocker
  `filesystem_restore_local_only`, so a local result set is never ready;
- each of the four checks that test the candidate package to name it, with all
  three digests. `PROMOTION_PACKAGE_CHECKS` holds them:
  `pipeline_bundle_qualification`, `pipeline_http_corpus_compatibility`,
  `pipeline_http_corpus_hf_local`, and `pipeline_restore_drill`. A candidate
  check that names fewer digests adds
  `qualification_evidence_package_missing:<check_id>`;
- every other check, a mechanics check or a promotion-only check, to name no
  package. A result that carries any digest adds
  `qualification_evidence_package_unexpected:<check_id>`;
- the packages that the results name to be one
  (`qualification_evidence_mixed_package` otherwise).

Its decision names the one revision and the one package, and its
`evidence_hash` covers each result's run id, code revision, package digests and
evidence hash (not the evaluation time, so the same evidence gives the same
hash). Three consequences:

- A decision is ready only when the four candidate checks name one package and
  no other result names any. A decision in which no check names a package is
  not ready: it carries four `qualification_evidence_package_missing`
  blockers.
- A `qualify` run names exactly one package. The corpus runs for the
  compatibility and HF-local checks and the restore drill serve it, and so does
  the bundle qualification test. The mechanics checks (crash matrix, leases,
  sweep, the activation checks, and the like) name none, and so does
  `pipeline_http_corpus_minimal`, which serves a test bundle. The report's
  `inputs.corpus_runs` still lists the minimal bundle's digests beside the
  candidate's, for the record: they are the corpus run's own, not a result's.
- A refused decision names its blockers to a route's caller. A qualification
  answers `409` `bundle_qualification_promotion_not_ready`, and an activation or
  a rollback answers `409` `bundle_activation_promotion_not_ready`. The body has
  a second field, `blockers`: the decision's blockers as labels, each
  `<label>:<check_id>` for one check (for example
  `qualification_evidence_stale:pipeline_crash_matrix`), or a label alone for
  the whole set (`qualification_evidence_mixed_revision`,
  `qualification_evidence_mixed_run`). A ready decision needs each of the 22
  ids once, each `pass`, each inside its maximum age, one revision, one run
  for the 19 results of `qualify`, the package named by the four candidate
  checks only, and no safe blocker.

## Signed check results

A result file is a file that anyone who can write one can make, so the server
counts a result only when a trusted key signed it. `pipeline.py qualify
--signing-key PATH --signing-key-id ID` signs every result of a run that
passed.

- `PATH` is an Ed25519 PKCS#8 DER private key. `ID` is the key id under which
  the server's check trust store holds the public key
  (`TRACE_COMMONS_PIPELINE_CHECK_TRUSTED_KEYS_PATH`, see
  [pipeline-activation.md](pipeline-activation.md)). The two flags go together
  (`signing_key_incomplete`), the id is 1 to 128 characters of `A-Za-z0-9_.:-`
  (`signing_key_id_invalid`), and the key file must be readable
  (`signing_key_unreadable`). The tool checks the flags before anything runs.
  The key's path goes to the signing step's environment only: no label, log, or
  report names it.
- `--evidence-max-age-seconds N` is how long a signed result stays current. It
  is under the signature, so the signer chooses it. The default is 86400 (one
  day) and the most is 604800 (seven days); a value outside 1 to 604800 is
  refused (`evidence_max_age_invalid`, `evidence_max_age_above_ceiling`). The
  server refuses an age of more than seven days too, on every route that
  verifies signed results (the qualification, the activation, and the
  rollback): `409` `bundle_qualification_evidence_age_above_ceiling`.
- `pipeline.py keygen --output PATH --key-id ID --trusted-key-output PATH`
  makes the key pair. It writes the private key (mode 0600; it never overwrites
  a file) and the trusted key, one JSON object `{"key_id", "public_key_base64url"}`.
  It prints `PipelineKeygenOK: key_id=ID` and no path. It trusts the key
  nowhere: put the object in a JSON array in the file that the check trust store
  variable names. Keep this key apart from the package signing key. The server
  refuses to start when one key is in both stores.
- A holder of the check-signing key can vouch for any result. The key must not be
  held by anyone who holds the tenant's admin credential. Nothing in the server
  enforces this. Who holds it is decided when the deployment is promoted. This
  repository's CI signs its `qualify` run with a key that the job makes with
  `keygen` and discards. No trust store holds that key.
- `pipeline.py revision` prints the code revision hash of the working tree: the
  `code_revision_hash` that every result of a run carries, and the value to give
  the build as `TRACE_COMMONS_BUILD_CODE_REVISION_HASH`. It hashes the path and
  content of every file that git tracks, and of every untracked file that the
  repository's own `.gitignore` files do not ignore, except the top-level
  `.local`, `.vscode`, and `target` directories, so any edit changes it. It
  also leaves out an untracked `.cargo` directory at any depth (a local cargo
  configuration); a `.cargo` file that git tracks is part of it. The
  repository's `.gitignore` has no `.cargo/` line, so a host with a local
  `.cargo/config.toml` in its checkout sees it in `git status` until that host
  adds `.cargo/` to its own `.git/info/exclude`. A
  host's `.git/info/exclude` and a user's global excludes file do not change
  it. Compute the revision, qualify, and build on the same clean checkout: a
  stray untracked file changes the revision.

An attestation is one file, `<check_id>.attestation.json`, next to the result it
signs. It holds the schema `trace_commons.pipeline_check_attestation.v1`, the
`result` (unchanged), `maximum_age_seconds`, a `corpus_digest` and an
`input_digest` (the three corpus checks carry them from their evidence; the
others hold null), and a `signature`: the algorithm `Ed25519`, the `key_id`, the
`attestation_hash`, and `signature_base64url`. The signature covers every other
field, so any change after signing breaks it. A route takes the attestations as
a JSON array of these objects, for example `jq -s . results/*.attestation.json`.

How a run signs, and what it leaves:

- The tool checks that the source tree has not changed before the signing step
  and again after it. It signs into `attestations-staging/` in the run directory,
  and moves the files into `results/` only after the report is written (and
  before the catalog is archived). So a failed run leaves no attestation file. If
  it cannot remove one, it prints `PipelineFailure:
  check_attestation_discard_failed` and writes `check_attestation_discard_failed`
  beside its own label in the failed report.
- Only the `results/*.attestation.json` files of a run that printed
  `PipelineQualificationOK ... attested=19` count. A run that was killed in a way
  the tool cannot catch can leave `attestations-staging/` holding a signed set that
  did not pass the second tree check, and no run removes it. Delete it by hand.
- The report keeps the schema `v1`. `attested` and `attestation_count` are
  optional on read: a report without both reads as unsigned, a report with one of
  them must have both, `attested` is true exactly when `attestation_count` is
  above 0, and a signed pass report counts 19. A pass report that an earlier
  version of the tool wrote, before the one-package rule, does not validate: it
  names many packages. A fail report of that version does.

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
  `results/` (every result and evidence file the run produced, and, for a
  signed `qualify`, its `<check id>.attestation.json` files), and
  `artifacts/` (one encrypted artifact root per scenario the run opened). A
  signed run also holds `attestations-staging/` while it runs.
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

Closing these is promotion work, not part of `qualify`:

- A remote-provider restore drill (GCS or another configured object store,
  not a local directory copy).
- Real production adapters standing in for the reference scorer, embedder,
  index, and settlement adapter.
- The HF network canary: a real download against the pinned dataset
  (`jedisct1/security-audits`), which arrives with the promotion work that needs
  it (ruling HF-1). No check that `qualify` runs hits the network.

`evaluate_promotion` also requires three promotion-only checks:
`pipeline_production_adapters`, `pipeline_remote_restore`, and
`pipeline_hf_network_canary`. Their results come from a production run, not a
local one: `pipeline.py promote` (below) writes the last two on the operator
host, and no local or CI run passes any of them. So a decision over the results
of a local `qualify` run is never ready: it lacks these three results, and its
restore drill carries the blocker `filesystem_restore_local_only`.

## The production run: `pipeline.py promote`

Spec: `docs/superpowers/specs/2026-10-08-pipeline-production-assembly-design.md`
(Slice B). Qualification is two runs of one code revision: the mechanics run
(`qualify`, above) and a production run on the operator host, against the
production package, with network. Every `promote` subcommand and `hf-pin
record` refuses to start when `CI` is set (`promote_refused_in_ci`).

| Command | What it does |
|---|---|
| `promote init --package P --trusted-key K` | Starts the production run: prints its `run_id` and code revision, records the package's three digests. Every later subcommand takes `--run-id` and refuses a changed tree (`promote_code_revision_changed`). |
| `promote package-checks --run-id R` | Refuses with `harness_production_assembly_unavailable` until the qualification harness can run on the production assembly. |
| `promote hf-canary --run-id R [--pin PATH]` | `pipeline_hf_network_canary`: downloads the network pin's revision into a fresh cache inside the run and compares all five digests (`hf_pin_digest_mismatch_<field>` on a moved one). The pin is the committed `crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/pin-network.json`, which the run's code revision covers; `hf_network_pin_missing` when it is absent. A `--pin` whose bytes differ from it is refused with `hf_network_pin_not_committed` before anything is downloaded, so an uncommitted pin (one `hf-pin record` just wrote, for example) is never certified. |
| `promote remote-restore --run-id R --source-store B[/prefix] --scratch-store B[/prefix]` | `pipeline_remote_restore`. Refuses a scratch store that is, contains, or sits inside the live one. Store names appear only as hashes. The restore harness it drives is not built yet: today it refuses with `remote_restore_harness_unavailable`. |
| `promote adapters --run-id R` | Checks the `pipeline_production_adapters` result the deployed ingest wrote at boot (copied into the run's `results/`): this run, this revision, this package, a pass with no blocker. |
| `promote sign --run-id R --signing-key KEY --signing-key-id ID` | Signs exactly the production run's seven results (the four package checks and the three promotion-only checks), on the pilot's feature set. |
| `promote assemble --run-id R --mechanics-run-id M --output DIR` | Writes the 22 attestations (15 from the mechanics run, 7 from this one) and the signed package into a new directory. Refuses a missing id, a mechanics id signed in the production run, and two code revisions. |
| `hf-pin record --revision COMMIT --output PATH` | Downloads one dataset commit and writes `pin-network.json` (the local pin's fields less `local_jsonl_dir`, with the computed digests). Never overwrites; the owner commits the file in a PR. |

`evaluate_promotion` discharges exactly one blocker on a production run's
evidence: `filesystem_restore_local_only` on `pipeline_restore_drill` is not a
blocker when the set holds a passing `pipeline_remote_restore` with no blocker of
its own, from the same code revision, naming the same package. Every other
blocker still blocks.

The child processes `promote` starts see only the allowlisted environment
(`child_environment`): no cloud credential variable reaches them. On the pilot
host, Application Default Credentials come from the metadata server.

## Package trust and `qualify_bundle`

`pipeline.py package` (see [pipeline-lab.md](pipeline-lab.md)) builds and
signs a `BundlePackage` with the existing canonical-hashing and Ed25519
signature format, verified against a `BundlePackageTrustStore`. Signing a
package with this command does not qualify it for production; it only
proves the package is well-formed and correctly signed.

The production path is `POST /v1/admin/pipeline/qualifications` (see
[pipeline-activation.md](pipeline-activation.md)). It takes a signed package and
the signed check results, `{signed_package, attestations}`, and calls
`PipelineQualificationStore::qualify_bundle_attested`
(`crates/trace-commons-server/src/versioned_pipeline_qualification.rs`). The bare
`PipelineQualificationStore::qualify_bundle` stays a library API that takes the
metadata and the check results as its caller gives them. No route calls it, and
a caller of it vouches for its own evidence.

The route verifies the package's signature against the package trust store
before it does any other work on the package. `qualify_bundle_attested` then
verifies every attestation against the check trust store, evaluates the
promotion over the verified results at the time of the call, and builds the
qualification's metadata itself. Nothing in the metadata comes from the request:

- `corpus_digest` and `input_digest` are the two digests that the verified
  attestations name: the canonical hash of the object `{check_id: digest}` over
  the attestations that carry that digest. A check result carries only its
  evidence's hash, so the digests come from the signed attestations and not from
  the result files.
- `configuration_digest` is the signed package's own.
- `runtime_dependency_digest` is the runtime identity of the dependency profile
  that the server built for the package from its own configuration.
- `evidence_hash` is the promotion's, and `code_revision_hash` is the build's
  (`TRACE_COMMONS_BUILD_CODE_REVISION_HASH`; see
  [pipeline-activation.md](pipeline-activation.md)).

It fails closed on each of these:

- an untrusted or tampered package (`bundle_package_signature_invalid`,
  `bundle_package_signer_untrusted`);
- a package outside the compatibility family, or with a development or synthetic
  marker (`bundle_implementation_unknown`, `bundle_development_dependency`);
- a package whose own configuration is not qualifiable
  (`bundle_configuration_not_qualifiable`, read from the signed package, not from
  the profile);
- a package that names a dependency that the running service does not hold
  (`bundle_dependency_missing`);
- a profile built for a different bundle (`bundle_qualification_profile_mismatch`);
- a package that pins an instrument with another descriptor than a package
  already registered for the tenant (`bundle_instrument_conflict`);
- a runtime identity that is not the profile's
  (`runtime_dependency_identity_mismatch`);
- any blocked dependency or infrastructure control (the first blocker's label);
- an attestation that is not valid, is signed by an untrusted key, or was altered
  (`check_attestation_invalid`, `check_attestation_signer_untrusted`,
  `check_attestation_signature_invalid`);
- a maximum age above seven days
  (`bundle_qualification_evidence_age_above_ceiling`);
- a promotion that is not ready (`bundle_qualification_promotion_not_ready`);
- a revision or a package that the promotion does not share with the
  qualification (`bundle_qualification_code_revision_mismatch`,
  `bundle_qualification_package_mismatch`);
- a second call for the same bundle and revision with different metadata
  (`bundle_qualification_identity_conflict`).

The evidence is evaluated on every call. A repeat of a call that already
recorded its row, made after the evidence went stale, is refused as
`bundle_qualification_promotion_not_ready` instead of answering the existing row.

A successful call records one append-only row for each `(tenant_id, bundle_id,
code_revision_hash)` in `pipeline_bundle_qualifications` (migration V107, with
the key widened to the code revision by V112). The same bundle on another
revision records another row, and a repeat with exactly the same inputs answers
the row that exists. The call also registers the package for the tenant. So a
qualification covers one code revision: after a deploy to a new revision, a
bundle needs a new qualification on it, an earlier bundle included, before an
activation or a rollback can use it. A tenant's active bundle needs it too
before a process of the new revision takes that tenant's new uploads: see
"After a deploy: the qualification is read again for each new upload" in
[pipeline-activation.md](pipeline-activation.md), which has the deploy
procedure.

A recorded qualification covers the signed package and its dependency
profile, not the deployment's bindings. `main`'s gate configuration
(`pipeline_runtime_main_gate_config_mismatch`) and the pipeline credit
issuer (`pipeline_credit_issuer_principal_missing`) are startup checks of
every bundle a routed or drained tenant may run (see
[pipeline-activation.md](pipeline-activation.md), "Each tenant's bundles at
startup"), not terms of the record. The activation and rollback routes keep
them: they run the same checks on the bundle they select, before their gate.

Activating a bundle for a tenant's live traffic uses this record. See
[pipeline-activation.md](pipeline-activation.md), "Activate, roll back, contain,
deactivate".
