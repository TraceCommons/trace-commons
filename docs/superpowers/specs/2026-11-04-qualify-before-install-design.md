# Qualify the new revision before the install: design

Status: approved 2026-11-04. The owner accepted all nine recommendations in
section 8 (D1-D9), recorded on PR #1910, and the implementation is in the same
PR: `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/prequalify.rs`,
`run_ingest_mode` and `serve_ingest` in `trace-commons-ingest.rs`, and
`results_dir_for_revision` in `pipeline_default_routing.rs`. Section 11 lists
where the build differs from the text above it. The operator procedure is
`docs/operator/pipeline-activation.md`, "Prequalify the new revision".
Part of #1907.
Date: 2026-11-04. Code read at `main` e44b9710e.

## 1. Summary

A deploy today installs the new binaries first and qualifies the new
revision afterwards. Between the restart and the requalification of each
tenant, every new upload of a pipeline-routed tenant answers `503
pipeline_bundle_not_qualified`, because the upload path checks for a
qualification on the revision the running binary was built from.

**The qualification itself can move before the install.** Qualification rows
are keyed by revision, and a process of revision N reads only its own rows.
Rows for N+1 written while N serves change nothing for N (section 3).

**The blocker is R1.** The 22-check set needs `pipeline_production_adapters`,
which only the deployed binary can emit, at boot, with the production
configuration. Today that means restarting the live service on the new
binary. A side boot of the new binary on the same host can emit it. But a
plain second `trace-commons-ingest` is not safe to run beside the live one:

- it starts every scheduler, the payout rail workers, the pipeline worker and
  the default-routing loop, each of which is turned off only by its own
  environment variable;
- it opens the live vector-index roots and can write to them at open and at
  exit.

Section 4 has the details.

**Recommendation: a dedicated `--prequalify` mode of `trace-commons-ingest`.**
It builds the same `AppState` from the live environment file, against its
own scratch index roots. It returns right after `finish_ingest_startup`, the
point where the adapters result is written today. It never reaches the code
that spawns schedulers, workers or loops. With a results directory, it then
records an N+1 qualification for every routed tenant in-process and exits.

The second change is that the default-routing results set for N+1 sits beside
N's instead of replacing it. The new process then starts already armed.

Together these remove the 503 window for every tenant that is routed and on
the set's bundle when the mode runs. Section 7 lists what stays.

## 2. Today's order and where the window opens

Phase A runs from source on the host: `qualify` (run M), then `promote init`
(run R), `package-checks`, `hf-canary` and `remote-restore`. Then:

1. Install the issuer, then the ingest binaries. Ingest restarts on N+1.
2. R1: restart ingest once more with `TRACE_COMMONS_PIPELINE_CHECK_*`. It
   writes `pipeline_production_adapters` just before it serves.
3. R2: `promote adapters`, `promote sign`, `promote assemble`, which write
   the 22 attestations and the signed package.
4. R3: swap the default-routing results directory. The loop re-arms and
   requalifies default-routed tenants, one page per pass.
5. R4: requalify hand-activated tenants through `POST
   /v1/admin/pipeline/qualifications`.

The window opens at step 1. It closes for each tenant at R3 or R4. The
upload-path check is at `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:15807-15829`.

## 3. Question 1: can N+1 be qualified while N runs?

**Yes. The rows are keyed by revision, and nothing N does reads another
revision's rows.**

- `pipeline_bundle_qualifications` is append-only (V107). V112 changed its
  key to `(tenant_id, bundle_id, code_revision_hash)`
  (`migrations/V112__versioned_pipeline_activation_gate.sql:9`). The store
  inserts with `ON CONFLICT (tenant_id, bundle_id, code_revision_hash) DO
  NOTHING`. A repeat with different metadata is
  `bundle_qualification_identity_conflict`
  (`crates/trace-commons-server/src/versioned_pipeline_qualification.rs:1466-1475`,
  `1586`, `1677-1710`).
- The upload path checks only for a row on its own revision.
  `active_bundle_qualified` is computed against
  `state.pipeline_code_revision_hash` (`trace-commons-ingest.rs:15822-15829`).
  `GET /v1/admin/pipeline/routing` answers for its own revision the same way
  (`pipeline_activation.rs:1136-1150`).
- The route cannot name a revision. `qualify_for_tenant`
  (`crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/pipeline_activation.rs:1199-1235`)
  passes `trust.revision`, which `trust_and_revision` reads from the build
  (`pipeline_activation.rs:719-738`). `qualify_bundle_attested`
  (`versioned_pipeline_qualification.rs:1750-1786`) refuses a set that names
  another revision with `bundle_qualification_code_revision_mismatch`. So
  only a process **of revision N+1** can record an N+1 row. That is the
  property worth keeping (section 6, alternative E).
- Recording a qualification also runs `register_bundle`. That inserts the
  package and its policy-status rows with `ON CONFLICT DO NOTHING`, and
  selects no active bundle (`crates/trace-commons-server/src/versioned_pipeline.rs:2158-2260`).
  N's startup checks cover each tenant's active bundle and the bundles of
  runs in flight, not every registered package
  (`docs/operator/pipeline-activation.md`, "Each tenant's bundles at
  startup"). So a new package registered for N+1 does not change what N
  checks or runs, even across a restart of N.
- The row records the qualifying process's `runtime_dependency_digest`. Later
  `activate` and `rollback` on the live N+1 compare against it (gate term 6).
  The digest covers only the dependency identities and the bundle's
  dependency digest (`versioned_pipeline_qualification.rs:1260-1276`). The
  usearch identities are constants
  (`crates/trace-commons-server/src/versioned_pipeline_production/usearch_pipeline_index.rs:762-779`).
  A side process of the same binary with the same environment file therefore
  records the digest the live N+1 computes, and scratch index roots do not
  change it.

The runbook already relies on this. "The deploy procedure" in
`pipeline-activation.md` (lines 988-1022) starts a process of build B outside
client traffic and posts the qualifications to it before the roll. This
design makes that step safe and moves R1 into it.

## 4. Question 2: can the adapters check run in a side process?

**Yes, but not as an ordinary second ingest. It has to stop before the point
where ingest starts its background work.**

### 4.1 What happens before the result is written

`run_ingest` (`trace-commons-ingest.rs:1362-1544`) builds `AppState`, then
calls `finish_ingest_startup` (1467). That function runs the scheduler config
validations, parses `TRACE_COMMONS_BIND`, binds, and only then calls
`emit_production_adapters_check` (1553-1635; the emission is at 1622-1633).
Everything that does background work comes **after** it returns:

| Started at | What |
|---|---|
| `trace-commons-ingest.rs:1475` | managed EdDSA keyset refresh |
| 1488-1530 | community snapshot recompute, export jobs, NEAR credit outbox, retention maintenance, vector index scheduler, perplexity and PII-backstop drivers, benchmark registry and pipeline, credit cycle, credit settlement, process evaluation, revocation propagation, account trust, community invalidation |
| 1531 | payout rail workers |
| `pipeline_runtime.rs:2809` | default bundle registration for receipts-list tenants (a DB write) |
| `pipeline_runtime.rs:2813-2817` | default-routing arming and cache fill |
| `pipeline_runtime.rs:2818-2820` | the pipeline worker and the default-routing loop |
| `trace-commons-ingest.rs:1542` | `flush_vector_indexes_on_shutdown`, on the way out |

A grep of `tokio::spawn` and `std::thread::spawn` in the ingest crate finds
no other spawn on the path up to the emission. The remaining spawns are in
the `spawn_*` functions above or in request handlers. The plan should still
keep the test in section 9 (T1), which proves this rather than relying on a
grep.

### 4.2 What a side boot shares with the live process before that point

- **Migrations.** `trace_corpus_db_mirror_from_env` runs `run_migrations`
  unconditionally (`trace-commons-ingest.rs:4097`, `8239-8266`), under a
  session advisory lock (`crates/trace-commons-server/src/db/postgres.rs:2757-2810`).
  On a deployment with the role split, the runtime role cannot run DDL, so a
  migration that is not applied fails the side boot. That failure is safe.
  `docs/operator/deployment.md:405-419` already requires applying migrations
  as the migrator before the install. This design moves that step before
  the side boot. N then runs against N+1's schema for longer: from the
  prequalify run rather than from the issuer restart. The rule there still
  holds: "usually safe", except for a migration that changes state an old
  process acts on (V147). A build that carries such a migration needs both
  units stopped, so it has a window this design does not remove.
- **Idempotent boot writes.** `enable_external_account_trust_growth` when
  external growth is configured (`trace-commons-ingest.rs:4766-4770`;
  `crates/trace-commons-server/src/admission_ledger.rs:838-849`). The live
  process made the same call at its own boot.
- **Reads.** The tenant bundle checks for the receipts and drain lists
  (`trace-commons-ingest.rs:4416`), the default-routing enumeration grant
  check (4478), and the start-time qualification warning (4493). The warning
  logs `pipeline_active_bundle_not_qualified` for every listed tenant, which
  is expected on a revision nobody has qualified yet.
- **Vector-index roots.** This is the one hazard that changes the design.
  Opening the pipeline index completes logged removals and compacts the
  manifest log in place (`usearch_pipeline_index.rs:355-371`, `478-495`). It
  takes no lock. Against the live root, that rewrites files the live process
  appends to. Separately, `flush_vector_indexes_on_shutdown`
  (`trace-commons-ingest.rs:1721-1743`) saves the side process's novelty and
  dedup indexes over the live files when it exits. **The side boot must use
  its own empty roots for all three indexes.** That changes nothing in the
  adapters result. The only root-dependent blocker is
  `index_root_shared_with_legacy` (`versioned_pipeline_production.rs:917-920`),
  and three separate scratch roots do not trip it. `promote package-checks`
  already never passes the live roots (`pipeline-qualification.md`,
  "The production run").
- **Listeners.** Only `TRACE_COMMONS_BIND` (1616-1621). No other port or
  socket is opened.
- **Connections and memory.** A second PostgreSQL pool of
  `DATABASE_POOL_SIZE`, and a second load of the embedder model. Section 10
  lists the headroom as not measured.

### 4.3 Why not a plain second ingest with variables removed

The runbook's side process today runs all of `run_ingest`. Each scheduler is
gated only by its own variable, for example
`TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_ENABLED` (7310-7316),
`TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_ENABLED` (7792-7797),
`TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ENABLED` (7859-7862),
`TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` (7680-7683), and
`TRACE_COMMONS_PII_BACKSTOP_ENABLED` (7737-7739). Payout rails have their
own switches. The pipeline worker has no switch at all: it spawns whenever a
runtime exists (`pipeline_runtime.rs:2661-2662`). It drains the receipts
list, the drain list and, under `all`, every default-routed tenant
(`pipeline_runtime.rs:2150-2158`).

So a safe side process needs "the live environment minus about fifteen
variables, and minus default routing, and minus both lists". That list grows
with every scheduler someone adds. Getting it wrong runs a second payout pass
or a second settlement scheduler against production. Removing the lists also
removes the one useful preflight the side boot could give: checking that
N+1 can run every routed tenant's bundles.

## 5. Question 3: can the N+1 default-routing set be armed before the install?

**Yes. A process arms before its listener opens, and the set for N+1 can
coexist with N's if each binary reads its own.**

- `prepare_default_routing` re-arms and fills the cache before the listener
  opens (`pipeline_runtime.rs:2813-2817`;
  `pipeline_default_routing.rs:1390-1406`). `verify_armed_set` checks that the
  set names the running revision (`pipeline_default_routing.rs:365-383`). So a
  new process whose directory already holds the N+1 set starts armed.
- The catch is that the live process reads the **same directory** at the
  start of every pass. Putting the N+1 set there before the install makes N
  report `pipeline_default_routing_revision_mismatch` and disarm. A disarmed
  N still serves the tenants it already routed, but it routes no new signup.
- The directory's **path** is read once, at start
  (`default_routing_config_from_env`, `pipeline_default_routing.rs:205-214`,
  called at `trace-commons-ingest.rs:4296`). Only its contents are read
  every pass.

Two ways to let the sets coexist:

- **(A) No code.** Keep one directory per revision, `results.<revision>`.
  Edit `TRACE_COMMONS_PIPELINE_DEFAULT_ROUTING_RESULTS_DIR` in the unit's
  environment file to N+1's directory just before the install. N never
  re-reads the variable. The risk: if N restarts for any other reason
  between the edit and the install, it comes up unarmed. That fails closed:
  routed tenants are still served, and new signups stay on the legacy path.
  A binary rollback has to revert the line.
- **(B) Small code change, recommended.** Arming first looks for a
  subdirectory named for the running revision
  (`<dir>/<revision hex>/`). If it is there, that is the set. If not, it
  reads the directory's top level as today. Both sets live under the one
  configured path, each binary arms itself, and neither a restart nor a
  binary rollback needs an edit. This touches only `read_results_dir` and
  `load_arming`, not the loop's pass cadence that #1906 is changing.

Arming alone does not close the window. An armed N+1 still answers 503 for a
routed tenant until its loop reaches that tenant's page. The prequalification
in section 6 is what closes it. The loop then finds each tenant already
qualified and skips it (`pipeline_default_routing.rs:1005`).

## 6. Question 5 and the recommended design

### 6.1 Alternatives

| | Option | Verdict |
|---|---|---|
| A | Side serving process with the scheduler, payout, routing and list variables removed (today's runbook step, extended to R1) | Unsafe by default: section 4.3. |
| B | **Dedicated `--prequalify` mode** that returns after `finish_ingest_startup`, with forced scratch index roots and an optional in-process qualification step | **Recommended.** Nothing to subtract: the code that starts background work is never reached. |
| C | Boot N+1 on a separate VM against the production database and stores | Passes the adapters check only if that VM has the full production profile (GCS, KMS, EdDSA). More infrastructure, and the same index and scheduler hazards. |
| D | Two ingest instances behind the proxy (blue/green) | Removes the window for any deploy, but both instances run workers against one database. Much larger change. |
| E | Let `POST qualifications` name a target revision, so N records N+1's rows | Rejected. A qualification would no longer prove that a process of that revision, with this configuration, checked the package and the runtime. That binding is the point of the rule. |
| F | Contain before the deploy | A contained tenant is also refused with 503, so this does not help. |

### 6.2 The `--prequalify` mode

`trace-commons-ingest --prequalify [--results-dir DIR]...`

1. **Startup.** `run_ingest_main` today gives a subcommand no pipeline
   runtime: any argument selects `PipelineRuntimeSelection::None`
   (`trace-commons-ingest.rs:1322-1331`). This one flag must read the
   selection as a plain boot does.
2. **Forced overrides, set by the mode and not by the operator:**
   - the three index roots (`TRACE_COMMONS_VECTOR_INDEX_ROOT`,
     `TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT`,
     `TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT`) become three fresh `0700`
     directories under one temporary directory the mode creates and removes;
   - the bind becomes `127.0.0.1:0`.
   The plan chooses how to apply them. Either set them before the Tokio
   runtime starts any thread, or pass them through the lookup seam
   `PipelineGateComponents::from_lookup` already has. Do not call
   `std::env::set_var` once other threads are running.
3. **Run the normal startup.** Build `AppState` from the live environment
   (`AppState::from_env_with_pipeline_runtime_assembler`), then call
   `finish_ingest_startup`. With `TRACE_COMMONS_PIPELINE_CHECK_*` set, this
   writes `pipeline_production_adapters` exactly as the R1 boot does today,
   with the same revision check
   (`emit_production_adapters_check`, `versioned_pipeline_production.rs:823-880`).
   The mode refuses to start without a production runtime
   (`pipeline_prequalify_requires_production_runtime`). Every refusal is a
   safe label.
4. **Prequalify**, only with `--results-dir`. For each directory:
   - verify it as arming does (`verify_armed_set`). A set that names another
     revision, a stale result or a package mismatch stops the mode with the
     arming label and records nothing;
   - walk **every** page of `PipelineActivationStore::routed_tenants`
     (`crates/trace-commons-server/src/versioned_pipeline_activation.rs:1295`).
     That covers `pipeline` and `contained`, default-routed and hand-activated;
   - for each tenant, read `routing_view` on this revision:
     - already qualified: skip, counted `already_qualified`;
     - active bundle is not this set's bundle: skip, counted
       `bundle_differs`;
     - otherwise call `qualify_for_tenant` under a system audit actor
       (decision D2), counted `qualified` or `refused` with its label.
   Unlike `requalify_batch`, this step does not filter on who wrote the
   routing row. A qualification changes no routing row, so default routing's
   "ours" test (`pipeline_default_routing.rs:970`) is unaffected.
5. **Exit.** Print the counts (no tenant id, no path) and remove the scratch
   roots. Skip `flush_vector_indexes_on_shutdown`, so nothing is saved
   anywhere. Exit non-zero if any directory was refused or any tenant was
   `refused`.

The mode never calls `run_pipeline_app` or any `spawn_*` function. So it
registers no default bundle, arms nothing, and runs no worker, loop,
scheduler or payout pass. Rows it writes are qualification rows, the
package and policy-status rows `register_bundle` adds, and one audit row per
qualification.

### 6.3 The resulting deploy order

| Step | What | Live process | Rollback |
|---|---|---|---|
| A | Phase A from source at the new commit, as today (run M, run R and its package checks) | untouched | none needed |
| P0 | Apply the build's migrations as the migrator, as `deployment.md` already requires | runs against N+1's schema | the migration's own rollback. A state-changing migration (like V147) needs both units stopped, and that deploy keeps a window |
| P1 | Fetch the N+1 binaries into a staging directory and verify them (`pull-and-install.sh` with `TC_FETCH_ONLY_DIR`) | untouched | delete the directory |
| P2 | Run the staged ingest with `--prequalify` and the three `TRACE_COMMONS_PIPELINE_CHECK_*` variables, as the service user with the unit's environment file. It writes the adapters result and exits. This replaces R1 | untouched | nothing to undo: the result is a file in the run |
| P3 | `promote adapters`, `promote sign`, `promote assemble --output <results>/<revision>` (R2) | untouched | discard the directory |
| P4 | Run the staged ingest with `--prequalify --results-dir <results>/<revision>`. Every routed tenant on the set's bundle gets its N+1 row. Read the counts; resolve any `refused` or `bundle_differs` | untouched (N+1 rows are invisible to N) | none needed: the N+1 rows are inert if N+1 is never installed |
| P5 | (B) nothing more: the subdirectory is already in place. (A) Edit the results path in the environment file | (A) only: an unplanned restart of N leaves it unarmed, which fails closed | (A) revert the line |
| P6 | Re-run P4 just before the install, to cover tenants N routed since then. Tenants already done are skipped | untouched | as P4 |
| P7 | Install the issuer, then ingest. N+1 starts armed, and every routed tenant already has its N+1 row | replaced | reinstall N's backup. N's rows are intact (append-only), and N re-arms with its own set under (B), or after the revert under (A) |
| P8 | Check: `config-status` shows armed; the start log has no `pipeline_active_bundle_not_qualified`; the loop's `requalified` stays near 0; `GET routing` shows `active_bundle_qualified_on_revision: true` for each hand-activated tenant | | |

R3 and R4 disappear from the deploy. The loop's requalification stays as a
backstop, and so does the admin route.

### 6.4 What fails closed

- An adapters result for another revision refuses the side boot
  (`pipeline_check_revision_mismatch`). A result already present is kept
  (`pipeline_production_adapters_already_emitted`), so emit into a fresh run
  directory.
- A results set that names another revision, is stale, names another package
  or does not verify stops P4 with the arming label. It records nothing.
- A migration that is not applied fails the side boot. The runtime role
  cannot run DDL.
- A wrong set in the arming directory leaves N+1 unarmed. Its routed tenants
  are then served only if P4 qualified them, and the rest answer 503 as they
  do today. Nothing is routed under a wrong bundle.
- The mode cannot reach the live index roots, the bind address, or any
  background task. These are enforced in code, not left to the operator.

## 7. What stays open after this design

- **Tenants routed in the last moments before P7.** A signup N activates
  after P6 has no N+1 row. N+1's loop requalifies it on its first pass,
  because the loop runs a pass as soon as it starts
  (`pipeline_default_routing.rs:1367-1378`). That is a window of one pass for
  a handful of tenants, not for all of them.
- **Tenants whose active bundle is not the set's bundle** (`bundle_differs`).
  An example is a deploy that ships a new package while tenants are still on
  the old one, for the "raising the award" move. Covering them needs a set
  for the old package on N+1 too, which means a second `qualify` and
  production run over that package. `--results-dir` is repeatable for that
  reason (decision D6). Without it, those tenants keep today's window until
  the loop migrates them.
- **A build carrying a state-changing migration** still needs both units
  stopped.
- **The issuer restart** in P7 is unchanged by this design.

## 8. Owner decisions (approved 2026-11-04)

The owner approved every recommendation below on 2026-11-04.

| # | Decision | Recommendation (approved) |
|---|---|---|
| D1 | Side boot as a dedicated `--prequalify` mode (6.2), or the runbook's side serving process with a list of variables removed (alternative A) | **The mode.** The variable list is open-ended, and a mistake runs a second payout or settlement pass against production. |
| D2 | Which audit actor records the prequalification, and does the mode also qualify hand-activated tenants | **A new system principal**, derived from the label `trace_commons.system.pipeline_prequalify` the way default routing's is. **Yes, qualify hand-activated tenants too.** A qualification changes no routing row, and the evidence is the same signed set an operator would post. The alternative, a loopback listener for the admin route, needs every hand-activated tenant's admin credential at deploy time and a restricted router. |
| D3 | How the N+1 and N default-routing sets coexist: (A) edit the environment path before the install, or (B) a per-revision subdirectory read first by arming | **(B).** Small, outside #1906's loop code, and binary rollback re-arms N with no edit. Use (A) until (B) ships. |
| D4 | Does the mode qualify `contained` tenants | **Yes.** Reopening a contained tenant needs a qualification on the running revision anyway. Writing it ahead changes no routing. |
| D5 | What a `refused` tenant does to the mode's exit | **Exit non-zero with the counts.** Whether to go ahead with those tenants still in the window is the operator's call. |
| D6 | `--results-dir` repeatable, one set per package | **Yes.** It is the only way to cover tenants on an older bundle across a package change. |
| D7 | Keep the R1 restart path (`TRACE_COMMONS_PIPELINE_CHECK_*` on the live service) | **Keep it working, and change the runbook to use the mode.** Remove nothing in this change. |
| D8 | The mode binds `127.0.0.1:0` instead of `TRACE_COMMONS_BIND` | **Yes.** The adapters check has no blocker that depends on the bind. The real bind is proved by the restart in P7, as for any deploy. |
| D9 | Close the residual window of section 7 inside the new process (requalify before the listener opens) or by re-running the mode just before the install (P6) | **P6 for now.** Doing it inside `prepare_default_routing` would touch the code #1906 is changing. Revisit after #1906 lands. |

## 9. Tests the plan should write first

- **T1 (no background work).** Run the mode against a migrated PostgreSQL
  fixture whose environment enables every scheduler, the payout rails, both
  lists and default routing `all`. Assert that it returns, and that the
  database has no lease, outbox, settlement, scheduler or worker effect, no
  default-bundle registration and no routing write.
- **T2 (index roots).** Point the configured roots at directories holding a
  manifest log with a logged removal and an uncompacted log. Assert that
  they are byte-identical after the mode exits.
- **T3 (revision isolation).** Prequalify under revision B on a database
  where tenants are qualified on A. Assert that every tenant gets a B row,
  and that A's `routing_view` and the upload-path check for A are unchanged.
- **T4 (counts and skips).** Cover `already_qualified`, `bundle_differs`,
  `refused`, and a contained tenant. A re-run is a no-op.
- **T5 (fail closed).** A set that names another revision, a stale set, and
  a package mismatch each exit non-zero and write no row.
- **T6 (selection).** `--prequalify` reads the pipeline runtime selection.
  Every other argument still selects `None`.
- **T7 (D3 B).** A process arms from `<dir>/<its revision>/` when that
  directory exists, and from the top level otherwise. A subdirectory for
  another revision never arms it.
- **T8 (digest parity).** The `runtime_dependency_digest` the mode records
  equals what a plain boot of the same binary and environment computes.
  `activate` on that boot passes term 6.

## 10. Not established

- Memory and CPU headroom on the host for a second process that loads the
  embedder model while the live one serves. Measure before relying on P2 and
  P4 under load.
- PostgreSQL connection headroom for a second pool.
- Whether any NEAR AI or privacy-filter call at boot is billed or
  rate-limited enough to matter for two extra boots per deploy.
- The duration of P4 at a large tenant count. Each qualification verifies the
  22 attestations and the package again.

## 11. As built

- **Structure.** `run_ingest` now ends in `run_ingest_mode`, which calls
  `finish_ingest_startup` and then either returns into
  `prequalify::run_prequalify` or calls `serve_ingest`. `serve_ingest` holds
  everything a serving process starts: `spawn_ingest_background_tasks` (the
  key refresh, every scheduler and driver, the payout rail workers),
  `run_pipeline_app` (default bundle registration, arming, the worker, the
  loop, the listener) and the shutdown flush. The mode's early return is the
  shape of the code, not a flag inside each spawn.
- **Scratch roots.** No environment variable is set at run time. The three
  root variables are resolved through `prequalify::IndexRoots`, which
  `AppState::from_env_with_pipeline_runtime_assembler` now takes and passes to
  the production gate builder (`PipelineGateComponents::from_lookup`, the
  novelty root of `near_ai_gate_parts`) and to the dedup index builder. The
  mode refuses before `AppState` is built unless the boot assembles the
  production pipeline, the only path that resolves roots this way.
- **Subdirectory name (D3).** The revision's hex digest without `sha256:`,
  so `<dir>/<64 hex>/`. `--results-dir` accepts that directory or its parent,
  through the same resolution.
- **Counts.** Besides section 6.2's, the report counts `no_active_bundle` (a
  routed tenant with no active bundle) and `audit_failed` (a qualification
  recorded whose audit row failed). Both `refused` and `audit_failed` make
  the exit non-zero (D5). `bundle_differs` does not.
- **A shared resource the spec missed.** A qualification appends an audit
  event to the tenant's file log under `TRACE_COMMONS_DATA_DIR`, and the
  append lock is per process (`audit_append_lock`). With the database mirror
  required, the row is written first and the database refuses a stale
  previous hash, so a collision with the running service's append for the
  same tenant fails the mode's audit, not the chain. It shows as
  `audit_failed`; the qualification is recorded, and a re-run finds it.
- **Tests** (all in the ingest library; the first ten were run against stubs
  of today's behaviour and failed before the code):
  - T1 `prequalify_pg_tests::the_mode_returns_before_any_background_work`:
    `run_ingest_mode` returns (a process in `serve_ingest` would not); the
    runtime's alive-task count does not rise across a run, and does when the
    pipeline worker is started (the probe's control); no default bundle is
    registered for a receipts-list tenant; default routing neither arms nor
    routes; the gate's flush is never called.
  - T2 `prequalify::unit_tests::the_mode_never_writes_under_the_configured_index_roots`
    (`near-ai-scorer`) and `the_mode_resolves_every_index_root_inside_its_scratch_directory`:
    the live roots, holding an uncompacted pipeline manifest and novelty and
    dedup indexes, are byte-identical after every index is opened, written
    and dropped through the mode's resolution; the same open through the
    environment's resolution changes them.
  - T3 `the_new_revision_is_qualified_beside_the_running_one`, T4
    `the_mode_counts_what_it_did_and_a_rerun_does_nothing`, T5
    `a_set_that_does_not_verify_records_nothing`, T7
    `arming_reads_the_subdirectory_of_its_own_revision`, T8
    `a_serving_process_of_the_build_activates_on_the_modes_qualification`.
  - T6 `prequalify::unit_tests::only_a_plain_boot_and_the_mode_read_the_runtime_selection`,
    plus argument parsing, the actor pin and the report rule.
- **Not tested end to end.** No test boots the binary with a production
  environment (it needs NEAR AI, GCS and KMS). T1 runs the mode from
  `run_ingest_mode` on a production-qualified test state; T2 runs the root
  resolution the startup uses. The first real run is on the host, and section
  10 still applies.
