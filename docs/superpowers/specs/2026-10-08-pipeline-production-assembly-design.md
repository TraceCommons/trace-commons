# Pipeline production assembly, promotion checks and gate-decision rows

Status: draft, revised 2026-10-08 after poldsam's review of PR #1292, with the
owner's decisions on R-1, O-A2, O-A3 and O-A4 recorded (section 10). Base:
`origin/main` at `d8f98f389` (PR #1291). Stage plan: PR #1286
(`docs/operator/pipeline-smoke-tests.md` on branch `pipeline-smoke-runbook`).
Implementation plan:
[`docs/superpowers/plans/2026-10-08-pipeline-production-assembly.md`](../plans/2026-10-08-pipeline-production-assembly.md).

The slices are being built while this spec is reviewed: Slice A is PR #1295
(branch `pipeline-production-assembly`), Slice B-1 is PR #1293
(`pipeline-promotion-checks`), Slice C is PR #1294
(`pipeline-gate-decision-rows`). Where this revision describes what a branch
does, it cites `branch@sha path:line`, so the citation survives the branch
moving: `pipeline-production-assembly@b85f09e9a`,
`pipeline-promotion-checks@e889b5f0e`, `pipeline-gate-decision-rows@15cc64951`.

## 1. Goal

Make stages 3 ("one throwaway tenant") and 4 ("first real tenant") of the
versioned-pipeline cutover runnable on a deployment built from this
repository. Today neither can run:

- The shipped binary injects no pipeline runtime: `main()` calls
  `run_ingest(None)`
  (`crates/trace-commons-server/src/bin/trace-commons-ingest.rs:1330`). With
  `None`, `assemble_ingest_pipeline_runtime` returns `Ok(None)`
  (`bin/trace_commons_ingest_internal/pipeline_runtime.rs:133-166`), so no
  tenant can be served on the pipeline.
- No activation can pass the gate. `evaluate_promotion`
  (`src/versioned_pipeline_qualification.rs:582`) needs a passing result for
  each of the 22 ids in `PROMOTION_REQUIRED_CHECKS` (`:134-160`), and nothing
  emits the three promotion-only ids `pipeline_production_adapters`,
  `pipeline_remote_restore`, `pipeline_hf_network_canary` (`:169-173`).
- Pipeline submissions write no `trace_gate_decisions` row, so duplicate
  clustering, the contributor cap, per-author scoring and account trust never
  see them (stage plan, stage 2, "Note the known difference").

This spec covers three slices. Each branches from `origin/main`, builds and
merges on its own, and is listed with the tests that prove it (TDD order) and
the CI job that runs them.

| Slice | Name | Stage it unblocks |
|---|---|---|
| A | Production assembly | 3 (startup preconditions, `pipeline_production_adapters`) |
| B | Operator promotion checks | 3 (activation: `pipeline_remote_restore`, `pipeline_hf_network_canary`, production package checks) |
| C | Gate-decision rows | 3 (3a/3b comparisons), 4 (consumers see pipeline traffic) |

Section 2 is a design gap that cuts across A and B. The owner has ruled on
it (R-1).

## 2. The production package gap (decided: R-1)

**Finding.** As the code stands, no package that a production assembly can
hold can ever receive a ready promotion, so no tenant can ever be activated on
a production runtime, whatever Slices A to C add, unless the package-bearing
checks also run against the production assembly. The chain:

1. Activation refuses a runtime whose dependencies are not the package's.
   `qualify_bundle` refuses `bundle_qualification_profile_mismatch` unless
   `dependencies.bundle.dependency_digest == package_digests(package).dependency_digest`
   (`versioned_pipeline_qualification.rs:1484-1490`), and refuses
   `runtime_dependency_identity_mismatch` unless the deployed profile's
   `runtime_identity_digest()` (`:1180-1196`: scorer, embedder, index reader
   and writer, settlement adapter identities, plus `dependency_digest`)
   matches. `activate_qualified_bundle_in` (`:1735`) repeats both checks
   against the stored package (`:1789-1806`), and refuses
   `bundle_activation_package_mismatch` unless the promotion names exactly the
   stored package (`:1783-1788`).
2. A package's `dependency_digest` is the SHA-256 over the Score phase's
   data-artifact hashes (`package_digests`, `:304-322`), and those are the
   SHA-256 of each dependency's `content_descriptor()` (gate-api
   `dependency.rs:22-25, 41-44`). A NEAR AI scorer and a bge embedder have
   different descriptors from the reference scorer and embedder, so a
   production package has a different `dependency_digest`.
3. The promotion names one package: the one the four `PROMOTION_PACKAGE_CHECKS`
   (`:180-185`: `pipeline_bundle_qualification`,
   `pipeline_http_corpus_compatibility`, `pipeline_http_corpus_hf_local`,
   `pipeline_restore_drill`) all name (`:623-663`).
4. Those four results are produced by `pipeline.py qualify`, which always
   builds its package from the built-in bundles with the reference scorer and
   embedder: `QUALIFY_CORPUS_RUNS` passes no package
   (`scripts/operator/pipeline.py:134-138`, `:845-847`), and the harness
   services are built from `ReferencePerplexityScorer`/`ReferenceEmbedder`
   wrappers (`bin/trace_commons_ingest_internal/tests.rs:10757-10830`,
   `tests/versioned_pipeline_runtime_pg.rs:29716-29760`).
5. A reference package cannot be submitted as a production one either:
   `validate_production_package` refuses any artifact containing
   `local_reference`, `reference_`, `pipeline-test`, `mock_` or `synthetic`
   (`:1261-1293`).

So the 19 results of a CI or lab `qualify` run can never be the evidence of a
production package. The owner's 2026-10-02 Q7 amendment already anticipates
part of this ("They join the package-bearing checks in the promotion work,
when the production assembly exists",
`docs/superpowers/plans/2026-10-02-versioned-pipeline-pr5-activation-plan.md:51`).

**Decision (owner, 2026-10-08): R-1.** Split qualification into two runs of
the same code revision:

- **Mechanics run** (unchanged): `pipeline.py qualify` in CI and the lab,
  reference dependencies, no network. It produces the 15 mechanics results that
  name no package.
- **Production run** (new, operator-only, Slice B): `pipeline.py promote` on
  the operator host, against the production assembly (Slice A), with network.
  It produces the seven package-bearing results for the production package:
  the four existing `PROMOTION_PACKAGE_CHECKS`, re-run through the production
  assembler, plus the three promotion-only checks.

`evaluate_promotion`'s one-run rule (`:664-673`) then becomes: the mechanics
results share one `run_id`, the package-bearing results share another, and all
share one `code_revision_hash` (unchanged). The run-id rule moves from
"everything except `PROMOTION_ONLY_CHECKS`" to "within each of the two groups".

A mechanics `qualify` run emits 19 results; the operator submits the 15
mechanics ones from it and the seven from the production run.
`check_attestation_invalid` refuses two attestations for one check id, so
`pipeline.py promote assemble` (Slice B) builds the 22-file set from the two
run directories and refuses a missing or doubled id; nobody hand-picks files.

Costs of R-1, accepted with it:

- The corpus checks compare each fixture's `consent_state`, `privacy_state`,
  `scoring_state` and `settlement_state` with expectations keyed by fixture
  label in Rust (`pipeline_corpus_pg_tests.rs`, `fixture_mismatches`). Those
  expectations were written for the reference scorer and the reference floors.
  Under real NEAR AI scoring and the deployment's floors, `scoring_state` (and
  so `settlement_state`) can legitimately differ. R-1 therefore needs a
  production expectation set, or a production mode of the corpus check that
  compares only the deterministic fields (consent, privacy, replay,
  changed-content refusal, tenant isolation) and records the scoring fields as
  evidence. The plan takes the second (B3); the owner may prefer the first
  (O-B3).
- The HF corpus result keeps the id `pipeline_http_corpus_hf_local` in the
  production run even though it is fed by the network pin, because that is the
  id `PROMOTION_REQUIRED_CHECKS` names. Its evidence records which pin fed it.

What R-1 does **not** do: build a reference scorer that reports the production
descriptor so the existing harness can name the production package. That would
be a test double answering the question it is supposed to test.

Alternatives considered and not chosen:

- **R-2.** Exclude the Score data-artifact hashes from what the four package
  checks bind, so a reference run can name a production package. Smaller, but
  it removes exactly the binding that says "this scorer was the one tested".
  Not recommended.
- **R-3.** Run the whole `qualify` on the operator host with the production
  assembly. Simplest rule, but makes the 15 mechanics checks depend on network
  dependencies they do not exercise, and CI could no longer produce a
  full-shape run.

## 3. Common facts the slices build on

### 3.1 The injection seam

- `IngestPipelineRuntimeAssembler::assemble(&self, IngestPipelineRuntimeContext)
  -> anyhow::Result<Arc<PipelineService>>` (`pipeline_runtime.rs:85-90`). The
  context carries the PostgreSQL backend, the artifact store and its name, the
  lease config, the NEAR contract, cadence and payout controls, the
  `NoveltyUtility` checks, `unqualified_routing_allowed`, and `main`'s gate
  configuration (`:47-73`).
- `assemble_ingest_pipeline_runtime` (`:133-275`) refuses, in order:
  `pipeline_test_dependencies_not_allowed_when_required`,
  `pipeline_unqualified_routing_not_allowed_when_required`,
  `pipeline_runtime_required_but_not_injected`,
  `pipeline_runtime_database_unavailable`,
  `pipeline_runtime_artifact_store_unavailable`, then after `assemble`:
  `pipeline_runtime_object_store_mismatch`,
  `pipeline_runtime_lease_config_mismatch`,
  `pipeline_runtime_unqualified_routing_mismatch`,
  `pipeline_unqualified_routing_with_production_runtime`, the three NEAR
  payout mismatches, `pipeline_runtime_main_gate_config_mismatch`,
  `pipeline_runtime_novelty_utility_checks_mismatch`,
  `pipeline_credit_issuer_principal_missing`, and
  `pipeline_runtime_dependencies_not_production_qualified`.
- At boot (`trace-commons-ingest.rs:4032-4065`) ingest then runs
  `validate_pipeline_receipt_rollout`,
  `validate_pipeline_privacy_filter_requirement` (`pipeline_privacy_filter_required`,
  `pipeline_runtime.rs:285-294`) and `validate_pipeline_tenant_bundles`
  (`:1474`). `compatibility_zero_floor`
  (`src/versioned_pipeline_compat.rs:60`, raised by
  `CompatibilityBundleConfig::validate`, `:230-268`) fires when the
  compatibility configuration has every floor at zero.
- "Production-qualified" means `pipeline_runtime_is_production_qualified`
  (`pipeline_runtime.rs:316-320`): `PipelineService::bundle_qualification` of
  the default package reports every dependency the default bundle uses as
  `production_qualified()` (named scorer and embedder, index reader and
  writer, one settlement adapter per pinned instrument, authority, privacy,
  payout when enabled), and a compatibility configuration that
  `is_qualifiable()` (`versioned_pipeline_compat.rs:274-277`).

### 3.2 What the test assemblers do

Every implementation of the trait is test code: `NameAssembler`
(`tests.rs:10601`), `FlagAssembler` (`:10685`), `CompatibilityAssembler`
(`:11359-11384`), `UnqualifiedAssembler` (`:11632`), `QualifiedAssembler`
(`:11648-11670`) and its variants (`:11674-11788`), `PayoutAssembler`
(`:12378`), `NoveltyUtilityChecksAssembler` (`:12477`), `TestAssembler` and
`CompatibilityTestAssembler` (`pipeline_http_pg_tests.rs:246, 6202`),
`CorpusAssembler` (`pipeline_corpus_pg_tests.rs:534`), `QualifiedRouteAssembler`
(`pipeline_activation_pg_tests.rs:2838`).

`QualifiedAssembler` passes the qualification check only because
`qualified_pipeline_service_with_privacy` (`tests.rs:11078-11160`) wraps the
reference implementations in doubles whose `production_qualified()` returns
`true`: `QualifiedTestScorer` (`:10757`, identity
`qualified_test_perplexity_scorer`), `QualifiedTestEmbedder` (`:10806`),
`QualifiedTestIndex` over `IsolatedPipelineIndex` (`:10834`),
`QualifiedTestSettlementAdapter` (`:10935-10965`), `QualifiedTestAuthority`
(`:10973-10995`) and `QualifiedTestPrivacy` (`:11001-11017`), and pins one
`trace_credit` award (`qualified_test_trace_credit_award`, `:11030-11046`).
`QualifiedProductionScorer` in `tests/versioned_pipeline_runtime_pg.rs:29716`
has identity `qualified_production_scorer_test_only` and descriptor
`trace-commons-qualified-production-scorer.v1`; it is chosen to avoid the five
markers `validate_production_package` refuses (comment at `:29696-29712`).

### 3.3 The gate-api seams the assembly holds

All held as trait objects, per the repository rule:

| Dependency | Trait | Where |
|---|---|---|
| Scorer | `IdentifiedPerplexityScorer: PerplexityScorer` | `crates/trace-commons-gate-api/src/dependency.rs:18-31`, `perplexity.rs:57` |
| Embedder | `IdentifiedEmbedder: Embedder` | `dependency.rs:34-50`, `embedder.rs:18` |
| Index | `IdentifiedIndexReader: VectorIndexReader`, `IdentifiedIndexWriter: VectorIndexWriter` | `dependency.rs:53-72`, `vector_index.rs:99-144` |
| Settlement | `SettlementAdapter` | `settlement.rs:196-232` |
| Payout | `NearPayoutAdapter` (server crate, not gate-api, Ruling T10-1) | `src/versioned_pipeline_credit.rs:246-256` |
| Authority | `PipelineAuthorityProvider` | `src/versioned_pipeline_authority.rs:21-28` |
| Privacy | `PipelinePrivacyBoundary` | `src/versioned_pipeline_authority.rs:71-90` |

`dependency_identity()` and `adapter_identity()` must be safe labels
(`^[a-z0-9_]{1,64}$`, `settlement.rs:200-202`); a hosted model name such as
`Qwen/Qwen3.6-35B-A3B-FP8` cannot be one and goes into
`content_descriptor()` bytes instead.

### 3.4 How the legacy path builds its components today

`build_enclave_near_ai_gate_service_from_env`
(`trace-commons-ingest.rs:6227-6476`, behind
`#[cfg(feature = "near-ai-scorer")]`, selected by
`TRACE_COMMONS_GATE_SERVICE=enclave_near_ai`, `:5923-5930`):

- **Scorer.** `NearAiPerplexityScorer::try_new(NearAiScorerConfig { base_url,
  model, api_key, tail_logprob_cutoff, logprobs_top_k, timeout })`
  (`:6300-6314`), built on the blocking pool because its reqwest client owns a
  runtime. Env: `TRACE_COMMONS_NEAR_AI_BASE_URL`, `TRACE_COMMONS_NEAR_AI_MODEL`,
  `TRACE_COMMONS_NEAR_AI_API_KEY`, `TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS`
  (`:549-555`), `TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF`;
  `logprobs_top_k` is the constant `TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K`.
  It implements `PerplexityScorer` including a lossless `score_chunk`
  (`crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs:541-557`).
- **Embedder.** `FastEmbedTextEmbedder::try_new(model_id, cache_dir,
  matryoshka_dim, max_tokens).await` (`:6316-6357`). Env:
  `TRACE_COMMONS_EMBEDDER_MODEL_ID` (default bge-large-en-v1.5),
  `TRACE_COMMONS_EMBEDDER_CACHE_DIR`, `TRACE_COMMONS_EMBEDDER_MAX_TOKENS`,
  `TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM`.
- **Index.** `UsearchVectorIndex::try_new(root, UsearchVectorIndexConfig {..})`
  (`:6359-6404`). Env: `TRACE_COMMONS_VECTOR_INDEX_ROOT`, `_DIM`, `_MAX_OPEN`,
  `_FLUSH_EVERY`, `_HNSW_M`, `_EF_CONSTRUCTION`, `_EF_SEARCH`, flush interval.
  It implements the legacy `VectorIndex` trait (`gate-api vector_index.rs:147`),
  not the pipeline's `VectorIndexReader`/`Writer`.
- All three are moved by value into
  `EnclaveGateOrchestrator::new(scorer, embedder, vector_index, cfg)`
  (`:6470`; `gate-enclave/src/orchestrator.rs:20-41`, generic `<P, E, V>`).
  Neither gate-api nor gate-enclave has a forwarding impl for `Arc<T>`, so
  today the components cannot be shared.
- **Privacy filter.** Not built by the gate service.
  `trace_commons_protocol::trace_contribution::privacy_filter_adapter_from_env()`
  (`crates/trace-commons-protocol/src/trace_contribution.rs:2835-2860`) returns
  `(Arc<dyn PrivacyFilterAdapter>, PrivacyFilterBackendTag)` from
  `TRACE_PRIVACY_FILTER_BACKEND` (`sidecar`, `near-ai`, `self-hosted`);
  `PiiClassifyPolicy::from_env()` reads `TRACE_COMMONS_PII_CLASSIFY_POLICY`
  (`:2895-2902`). The pipeline already has the production boundary:
  `ClassifierRedactorPipelinePrivacyBoundary::new(adapter, tag, policy)`
  (`versioned_pipeline_authority.rs:111-170`), qualified only over a real
  backend.
- **Gate floors.** `main`'s floors, top-k and chunk knobs are parsed for the
  pipeline by `pipeline_main_gate_config_from_env(assembled, delta)`
  (`trace-commons-ingest.rs:6857`), which reads the floors only when an
  assembler is present.
- **Settlement.** Legacy NEAR settlement mode is `NearSettlementMode`
  (`trace-commons-ingest.rs:26196-26206`, `for_pipeline()`); the pipeline
  already has `DryRunNearPayoutAdapter` (`versioned_pipeline_credit.rs:268-310`).
  No production `SettlementAdapter` for `trace_credit` exists outside tests.

### 3.5 How check results are produced and signed

- `PipelineCheckResult` (`versioned_pipeline_qualification.rs:249-263`):
  schema, `run_id`, `check_id`, status, `code_revision_hash`, optional
  `package_hash`/`configuration_digest`/`dependency_digest`, `observed_at`,
  `evidence_hash` (canonical-JSON SHA-256 of the observed value, `:209-226`),
  `safe_blockers`.
- `PipelineCheckEmitter::emit` (`:364-427`) writes
  `<check_id>.evidence.json` and `<check_id>.result.json` into
  `TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR`, keyed by
  `TRACE_COMMONS_PIPELINE_CHECK_RUN_ID` and
  `TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH`; it refuses a repeat id
  (`pipeline_check_already_emitted`). `emit_from_env` panics on any error
  (`:432-450`): fine in a test, fatal in a server.
- `pipeline.py qualify --signing-key --signing-key-id` signs each accepted
  result through the Rust writer
  `tests::pipeline_corpus_pg_tests::pipeline_check_attestations_write`
  (`scripts/operator/pipeline.py:72, 970-1018`) into
  `<check_id>.attestation.json` (`trace_commons.pipeline_check_attestation.v1`).
- `POST /v1/admin/pipeline/qualifications` takes `{signed_package,
  attestations}` and calls `qualify_bundle_attested` (`:1617`), which verifies
  them against the check trust store and runs `evaluate_promotion`.
- `evaluate_promotion` adds every result's own `safe_blockers` whatever its
  status (`:613-622`). The local restore drill always carries
  `filesystem_restore_local_only`, and `pipeline.py` *requires* it
  (`pipeline.py:100, 740`). So even with all three promotion-only results, a
  set containing a local restore drill is never ready. Slice B resolves this.

### 3.6 What evidence the three promotion checks are meant to carry

From `docs/operator/pipeline-qualification.md:449-480`,
`docs/operator/pipeline-lab.md:62-70` and the PR 4 plan
(`docs/superpowers/plans/2026-09-29-versioned-pipeline-pr4-qualification-plan.md:108, 1118`):

- `pipeline_production_adapters`: "real production adapters standing in for
  the reference scorer, embedder, index, and settlement adapter" -- the
  closure of the local blockers `local_reference_scorer`,
  `local_reference_embedder`, `synthetic_index`, `synthetic_settlement`.
- `pipeline_remote_restore`: "a remote-provider restore drill (GCS or another
  configured object store, not a local directory copy)" -- the closure of
  `filesystem_restore_local_only`.
- `pipeline_hf_network_canary`: "a real download against the pinned dataset
  (`jedisct1/security-audits`), which arrives with the promotion work that
  needs it (ruling HF-1)" -- the closure of `hf_network_canary_not_run`.

### 3.7 Consumers of `trace_gate_decisions`

Schema: `migrations/V23__novelty_utility_credit_and_gate_decisions.sql:34-63`
(PK `(tenant_id, decision_id)`, RLS forced, policy
`trace_corpus_tenant_isolation` on `trace_current_tenant_id()`), extended by
V24, V25, V37, V39, V40, V41, V47, V48, V53, V54, V57, V73. The legacy writer
is `insert_trace_gate_decision[_with_chunk_entries]`
(`src/db/trace_corpus_pg.rs:6765-6890`) on the trace pool inside
`begin_trace_tenant_transaction`. Multiple rows per submission are legal (the
`Cached` outcome, `:6965`). Readers:

| Consumer | Reader | What it reads |
|---|---|---|
| Duplicate clustering | `list_dedup_signals`, `list_dedup_rederive_rows` (`db/postgres.rs:5588, 5630`); sweeps at `trace-commons-ingest.rs:57313, 58058` | `dedup_simhash`, `dedup_cluster_*`, `dedup_signal_version` |
| Contributor cap | `list_contributor_cap_signals` (`db/postgres.rs:5704`); sweep at `trace-commons-ingest.rs:58319` | `credit_quality_micros`, `dedup_cluster_size`, joined to `auth_principal_ref` |
| Credit quality | `list_gate_decisions_for_credit_scoring` (`db/postgres.rs:5545`); sweep at `trace-commons-ingest.rs:57187` | rows with `perplexity_micros > 0` |
| Per-author scoring | V73 columns, read by `find_gate_decision_by_canonical_hash` and the score listing (`trace_corpus_pg.rs:7343-7370`) | `agent_prose_*`, `tool_result_*`, `attributed_token_fraction_micros` |
| Account trust | `trace_record_account_trust_fact` `gate_evaluation` branch (`migrations/V85__account_trust_fact_recorder.sql:105-118, 180-185`), cluster guard (`V86:72-100`), V114 input guard (`V114:299`) | `gate_policy_version <> ''`, `perplexity_passed`, `novelty_passed`, `decided_at`, `dedup_cluster_id` |
| Scores by submission | `list_scores_by_submission_ids` (`db/postgres.rs:5756`) | credit quality, perplexity, novelty, chunk fields |
| Legacy drain report | `versioned_pipeline_activation.rs:566-645` (excludes `pipeline_runs`), withdrawal completeness `:862-866` | presence; dedup columns NULL after withdrawal |
| Gate driver selection | `count/list_submissions_needing_gate_decision` (`db/postgres.rs:5316-5400`, excludes `pipeline_runs`) | presence |

**A write to the table also takes account-trust locks (V114).** The trigger
`account_trust_gate_frontier` (`migrations/V114__external_account_trust_evaluations.sql:403`)
fires `AFTER INSERT OR UPDATE OR DELETE` on every row.
`trace_account_trust_advance_gate_frontiers` (`:339-402`) returns at once for
an UPDATE that changes none of `tenant_id`, `submission_id`, `decision_id`,
`credit_quality_micros`, `credit_quality_calibration_version`,
`dedup_signal_version`, `dedup_cluster_id`, `decided_at` (`:345-351`), and
when external growth is off (`:353-356`). When it is on, it takes, in this
order:

1. `trace_account_trust_external_growth`'s single row `FOR SHARE` (`:353`);
   `trace_account_trust_enable_external_growth` (`:190-204`) updates that row,
   so turning growth on waits for every gate write in flight.
2. One `trace_account_trust_dependency_locks` row per key, by
   `INSERT ... ON CONFLICT DO UPDATE` in `trace_account_trust_lock_dependencies`
   (`:309-321`, called at `:375`), held exclusively until commit. Keys are
   `submission:<tenant>:<submission>` for the old and new submission, and
   `cluster:<id>` for the row's old and new `dedup_cluster_id` and any
   cluster already on another gate row of that submission (`:365-374`),
   locked in `COLLATE "C"` order within the call.
3. `trace_account_trust_frontiers` rows, one `UPDATE ... generation+1` per
   account that has a `gate_evaluation` fact on that submission, that
   decision, or any submission in an affected cluster, in
   `(tenant_id, account_id)` order (`:380-400`).

It reads `trace_gate_decisions` and `trace_account_trust_facts` without
locking them, and takes no account or gate-row lock after step 2 (comment at
`:376`). The fact-table trigger (`:107`, through
`trace_account_trust_lock_fact_dependencies`, `:322-338`) takes the switch
row and submission and cluster keys the same way, then advances the fact's
account frontier (`:93-104`). Slice C's lock order is in C-D1.

## 4. Slice A: production assembly

### 4.1 Decisions

- **A-D1. Where it lives: the library crate.** The descriptors
  (`NearAiScorerDescriptor`, the fastembed descriptor), the pipeline adapters
  (scorer, embedder, index, Trace Credit settlement, tenant-policy
  authority), `PipelineGateComponents` and the production assembler live in a
  module of the `trace-commons-server` library crate (`src/`, AGPL header).
  Only the builder that constructs the NEAR AI scorer, the fastembed embedder
  and the usearch indexes from the environment stays in the ingest binary,
  behind `#[cfg(feature = "near-ai-scorer")]`. The reason is R-1: the
  `pipeline_bundle_qualification` result is emitted only from the integration
  target `tests/versioned_pipeline_runtime_pg.rs`
  (`qualification_inspects_the_objects_the_constructor_receives`,
  `origin/main` `:2769`, emit at `:2898`), and `checks.py` runs it as
  `--test versioned_pipeline_runtime_pg`. An integration target links the
  library crate only, so an assembler kept in the binary could never be used
  by that check, and the production run could never produce one of its seven
  package-bearing results. At `pipeline-production-assembly@b85f09e9a` the
  whole module is still
  `src/bin/trace_commons_ingest_internal/production_assembly.rs`; #1295 is
  moving everything except the env builder into the library. The adapter
  types and the assembler are not feature-gated: they hold
  `Arc<dyn PerplexityScorer>`, `Arc<dyn Embedder>` and the index behind the
  gate-api traits, so the default-features `cargo test` exercises them with
  qualified doubles. The usearch-backed index adapter compiles only with
  `near-ai-scorer` (usearch is feature-gated in gate-enclave), wherever it
  sits. Switching the bundle-qualification check to the production assembly
  is B-2's work, not #1295's (B-D1, and section 5.2 item 3 for which
  assertions change).
- **A-D2. Opt-in, not implied by the feature.** `main()` passes an assembler
  only when compiled with `near-ai-scorer` **and**
  `TRACE_COMMONS_PIPELINE_RUNTIME=production`. Unset (or empty) keeps today's
  `run_ingest(None)` exactly, so stage 1 ("deploy with the pipeline off") on
  the pilot's `near-ai-scorer` build boots as it does now. Any other value
  refuses the start with `pipeline_runtime_selection_unknown`. A default build
  with `TRACE_COMMONS_PIPELINE_RUNTIME=production` refuses with
  `pipeline_runtime_production_requires_near_ai_scorer` (mirroring the
  `enclave_near_ai` arm at `trace-commons-ingest.rs:5925-5930`). The
  production selection also requires `TRACE_COMMONS_GATE_SERVICE=enclave_near_ai`
  (`pipeline_runtime_production_requires_enclave_near_ai`), because the legacy
  path's components are what the pipeline reuses. `GET /v1/admin/config-status`
  reports `pipeline_runtime_selection` as one of `none`/`production`.
- **A-D3. Shared components, one stack.** Refactor
  `build_enclave_near_ai_gate_service_from_env` into two steps:
  `near_ai_gate_components_from_env() -> NearAiGateComponents` (parse env,
  construct the scorer, embedder and novelty index once, held as `Arc`s) and
  the existing orchestrator construction over those `Arc`s. Add forwarding
  impls in `trace-commons-gate-api` (AGPL; no license boundary crossed):
  `impl<T: PerplexityScorer + ?Sized> PerplexityScorer for Arc<T>`,
  the same for `Embedder` and `VectorIndex`, each forwarding every method
  including the defaulted ones (`score_chunk`, `snapshot`, `flush`). The
  forwarding impls must override every default method, or an `Arc` silently
  drops the NEAR AI scorer's lossless `score_chunk`; a unit test pins it.
  The bundle the pipeline sees holds trait objects and descriptors only,
  never the gated concrete types:
  `PipelineGateComponents { scorer: Arc<dyn PerplexityScorer>, scorer_descriptor:
  NearAiScorerDescriptor, embedder: Arc<dyn Embedder>, embedder_descriptor:
  FastEmbedDescriptor, index_reader: Arc<dyn IdentifiedIndexReader>,
  index_writer: Arc<dyn IdentifiedIndexWriter>, privacy: Option<Arc<dyn
  PipelinePrivacyBoundary>> }`. Only the env builder that fills it from
  `NearAiPerplexityScorer`, `FastEmbedTextEmbedder` and `UsearchVectorIndex`
  is `#[cfg(feature = "near-ai-scorer")]`; a default-features test fills it
  with qualified doubles. `AppState` keeps the components it built, and the
  assembler receives them through a new field
  `IngestPipelineRuntimeContext::components: Option<Arc<PipelineGateComponents>>`
  (`None` refuses with `pipeline_production_components_missing`), so
  `assemble` stays the only entry.
  Memory matters: the pilot is CPU- and RAM-constrained by bge-large on ONNX;
  loading a second embedder is not acceptable.

  **The component build moves ahead of pipeline assembly.** On `origin/main`
  `assemble_ingest_pipeline_runtime` runs at `trace-commons-ingest.rs:4032`
  and the gate service is built later, at `:4746`, inside the
  `Ok(Self { .. })` literal, returning only `Arc<dyn TraceGateService>`. An
  assembler there would always refuse with
  `pipeline_production_components_missing`. Under the production selection
  the legacy gate and the pipeline components are therefore built together,
  before assembly. Boot order in `from_env_with_pipeline_runtime_assembler`
  as #1295 implements it (`pipeline-production-assembly@b85f09e9a`,
  `trace-commons-ingest.rs`):

  1. `trace_corpus_db_mirror_from_env()` connects the database mirror
     (`:3825`) and `trace_artifact_store_from_env` builds the artifact store
     (`:3996`), as today.
  2. `pipeline_main_gate_config_from_env` parses `main`'s floors (`:4041`).
  3. Production selection only: `build_near_ai_gate_service_with_pipeline_components`
     (`:4047-4073`) builds the NEAR AI client (`near_ai_gate_parts_from_env`,
     `NearAiPerplexityScorer::try_new` on the blocking pool, `:6422`), loads
     the fastembed model (`:6462`), opens the legacy novelty index, runs the
     legacy all-zero-floor refusal ("cannot all be zero", `:6527-6533`),
     then builds the legacy orchestrator and the pipeline components over
     the same `Arc`s, including the pipeline index on its own root.
  4. `assemble_ingest_pipeline_runtime_with_components` (`:4076`) runs every
     refusal of 3.1 in its existing order, including
     `pipeline_runtime_database_unavailable` and
     `pipeline_runtime_artifact_store_unavailable`
     (`pipeline_runtime.rs:220` on that branch) and, through
     `CompatibilityBundleConfig::validate`, `compatibility_zero_floor`.
  5. `validate_pipeline_receipt_rollout`, `validate_pipeline_privacy_filter_requirement`,
     `validate_pipeline_tenant_bundles` (`:4095-4110`).
  6. The rest of `AppState`'s locals, then the struct (`:4640`), whose
     `gate_service` field takes the prebuilt gate, or calls
     `build_trace_gate_service_from_env()` as before when there is none
     (`:4794`).

  What this changes at boot, under the production selection only:

  - The NEAR AI client and the fastembed model load now come before the
    pipeline's own refusals and before every `AppState` local built between
    `:4110` and `:4640`. The database mirror and the artifact store are still
    constructed first (step 1), and anything that fails there fails as it
    does today. What moves behind the model load is the pipeline's refusal
    of an absent database or artifact store (step 4) and every refusal after
    step 3: a deployment misconfigured in any of those ways now loads the
    embedder and builds the NEAR AI client before it refuses. Nothing is
    served in that window, and the refusal labels are unchanged.
  - The legacy all-zero-floor refusal moves from the end of `AppState`
    construction to step 3, ahead of the pipeline's checks. An all-zero
    floor configuration is therefore refused with the legacy message, after
    the model load, and `compatibility_zero_floor` is never reached for it.
    The two refusals guard the same three variables, so the configurations
    refused are the same; only the label differs.
  - With the selection unset (`None`) nothing moves: the gate is built at
    `:4794` exactly as on `main`, and no model loads before step 4.
- **A-D4. Scorer adapter and identity.** `NearAiPipelineScorer { inner:
  Arc<dyn PerplexityScorer>, descriptor: NearAiScorerDescriptor }` implements
  `PerplexityScorer` (forwarding `score` and `score_chunk`) and
  `IdentifiedPerplexityScorer` with:
  - `dependency_identity()` = `near_ai_perplexity_scorer` (fixed safe label);
  - `content_descriptor()` = canonical JSON (sorted keys, the same
    `to_canonical_vec` as `evidence_hash`) of
    `{"schema":"trace_commons.near_ai_scorer_descriptor.v1","model":<TRACE_COMMONS_NEAR_AI_MODEL>,"tail_logprob_cutoff_micros":<round(cutoff*1e6)>,"logprobs_top_k":<u32>}`;
  - `production_qualified()` = `true`.
  The descriptor is a pure function of the model pin and the scorer's own
  configuration. Chunking knobs are Score-policy configuration, already bound
  by the package's `configuration_hash` through `main_gate`, so they are not
  repeated here. The API key, timeout and base URL are excluded (the base URL
  is operator configuration and the hash-only convention keeps it out of a
  stored artifact). The descriptor does not bind the NEAR AI attestation host
  or a measurement pin (O-A3, decided 2026-10-08): a change of the serving
  TEE alone is not a new package. Because qualification binds per code revision and per
  package, rotating the model or any scoring knob produces a new package and
  needs a new qualification, which is the intended effect.
  `NearAiScorerDescriptor::from_env()` reads only the descriptor inputs, with
  no network, so `pipeline.py package` can build the production package on a
  workstation (Slice B).
- **A-D5. Embedder adapter.** `FastEmbedPipelineEmbedder { inner: Arc<dyn
  Embedder>, model_id, output_dim, max_tokens, matryoshka_dim }`: identity
  `fastembed_text_embedder`, `model_id()` the configured model id,
  descriptor canonical JSON of
  `{"schema":"trace_commons.fastembed_embedder_descriptor.v1","model_id","output_dim","max_tokens","matryoshka_dim"}`,
  qualified `true`.
- **A-D6. Index: same implementation, its own root.** The pipeline index must
  not share the legacy novelty root: the dedup index comment
  (`trace-commons-ingest.rs:6490-6505`) already records that two indexes on
  one root corrupt each other's `nearest`. `UsearchPipelineIndex` opens a
  second `UsearchVectorIndex` at `TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT`
  (required under the production selection; refused when equal to, or nested
  in, `TRACE_COMMONS_VECTOR_INDEX_ROOT` or the dedup root, label
  `pipeline_vector_index_root_shared`), with the same dimension knobs. Usearch
  is keyed by an entry UUID and a namespace string; the pipeline contract also
  needs content-conflict detection, `exclude_revision` on `nearest`,
  `invalidate_revision`, and a `snapshot_hash`. The adapter keeps a manifest
  per `(tenant_storage_ref, index_id)` namespace: `entry_id -> (revision_id,
  content_digest, content_hash)`, persisted beside the usearch files
  (`<root>/<namespace>.manifest.json`, written atomically on every flush) and
  checked against the usearch entry count on open. A manifest that does not
  match its usearch file refuses the start
  (`pipeline_vector_index_manifest_mismatch`); the operator then runs
  `POST /v1/workers/pipeline/index-rebuild` per tenant
  (`rebuild_index_from_authoritative_commands`,
  `src/versioned_pipeline.rs:10126`) into an emptied root. Identities
  `usearch_pipeline_index_reader`/`_writer`, both qualified `true`. Namespace
  = `pipeline:<index_id>:<tenant_storage_ref>`.

  **Flushing.** Every pipeline index write flushes usearch and then writes
  the namespace's manifest atomically before it returns (`persist`,
  `pipeline-production-assembly@b85f09e9a`
  `production_assembly.rs:1201-1218`), and the pipeline index is opened with
  `flush_interval = None` (`:723-726`), so no periodic flusher runs and the
  legacy `TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY` setting has nothing to
  bound: no `flush_every` refusal exists and none is specified. What must fit
  inside `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS` (60 s,
  `versioned_pipeline.rs:218`) is one write including its flush and manifest
  rewrite; `flush_returns_within_the_fence_margin`
  (`production_assembly_tests.rs:609` on that branch) measures it against a
  generous bound. The flush and the manifest rewrite grow with the shard, so
  this is measured again before stage 4 (R7).

  **Routing is not held off during recovery.** Nothing in #1295 pauses a
  routed tenant between a boot on an emptied root and the end of its
  rebuild. In that window Score runs against an empty index, so novelty is
  computed against nothing and the run settles on that verdict; the V113
  fence covers only the rebuild's own writes against invalidation claims. So
  the recovery procedure, which the operator runbook states, is: suspend the
  tenant's Score policy (`POST /v1/admin/pipeline/policy-interventions`,
  action `suspend`), empty the root, boot, run the per-tenant rebuild, check
  its report, then resume. A suspended Score policy starts no Score attempt
  (`policy_is_runnable`, `versioned_pipeline.rs:1645`), and the rebuild
  evaluates no policy, so it runs while the policy is suspended. A
  code-enforced hold (refuse to serve a routed tenant whose namespace has no
  manifest while its index commands say it should) is not part of Slice A.
- **A-D7. Privacy.** `ClassifierRedactorPipelinePrivacyBoundary::new(adapter,
  tag, PiiClassifyPolicy::from_env())` over `privacy_filter_adapter_from_env()`,
  built once at boot. An unset backend leaves the boundary out, and the
  runtime is then not production-qualified; with
  `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER` set, startup also refuses with
  `pipeline_privacy_filter_required`. No new code beyond the call.
- **A-D8. Authority.** `TenantPolicyPipelineAuthorityProvider` over the
  `TRACE_COMMONS_TENANT_POLICIES` map ingest already parses
  (`parse_tenant_submission_policies_from_env`, `trace-commons-ingest.rs:11862`;
  `AppState.tenant_policies`, `:1684`), returning `SubmissionAuthority {
  tenant, policy: None, require_policy }` exactly as the legacy admission
  computes it for a tenant. `production_qualified()` = `true`. How the legacy
  path combines the tenant allowlist with the per-token claim allowlist
  (`claims.allowed_consent_scopes`, `:11857`) and what it does for a tenant
  missing from the map must be mirrored exactly (O-A1); a parity test pins it.
- **A-D9. Settlement disabled until 3d.** The compatibility bundle pins one
  `trace_credit` award, so the runtime needs one qualified `SettlementAdapter`
  for it. `InternalTraceCreditSettlementAdapter`: `instrument_id` =
  `InstrumentId::trace_credit()`, `adapter_identity` =
  `internal_trace_credit_ledger`, `payout_rail` = `none`, `settle` answers
  `SettlementReceipt::internal(request.expected_result_ref_hash())` with no
  external effect (the pipeline's own transaction writes the
  `trace_credit_ledger` row, `versioned_pipeline.rs:12753`), qualified `true`.
  This is the production adapter, not a synthetic one: Trace Credit has no
  external effect beyond the ledger. NEAR payout stays off: the assembler
  never calls `with_payout` in this slice, so `payout_enabled()` is false and
  `PipelineNearPayoutControls` are not consulted. That makes pipeline payout
  structurally impossible, and `live_external_payout_enabled` in
  `infrastructure_profile_from_state`
  (`bin/trace_commons_ingest_internal/pipeline_activation.rs:355-410`) reads
  the pipeline service's own payout controls, so it stays false whatever the
  legacy mode is. No boot refusal on `TRACE_COMMONS_NEAR_SETTLEMENT_MODE` is
  added: that variable is one deployment-wide value, and refusing on it would
  couple the pipeline runtime to every legacy tenant's payout. The stage plan's
  stage 3 precondition "NEAR settlement mode is `disabled`"
  (`TRACE_COMMONS_NEAR_SETTLEMENT_MODE=disabled`) stays: NEAR payout is off
  deployment-wide for stage 3 (O-A4, decided 2026-10-08). It is an operator
  precondition in PR #1286's stage plan, not a boot refusal, and the pipeline
  tenant could not pay out without it either.
- **A-D10. Bundle.** The assembler binds the compatibility bundle:
  `CompatibilityBundleConfig::production_compatible(scorer_model_id,
  projection_id, index_id, &context.main_gate)`
  (`versioned_pipeline_compat.rs:177`), with `scorer_model_id` =
  `near_ai:<sha256 of the scorer descriptor>`, `projection_id` =
  `compatibility_projection_v1`, `index_id` =
  `compatibility_index_v1`, none containing `test` or `reference`
  (`validate_production_package`, `:1270-1276`), then
  `MinimalPolicyBundle::compatibility_package(&config, scorer, embedder)`
  (`versioned_pipeline_bundle.rs:597`). Builder calls: `with_scorer`,
  `with_embedder`, `with_authority`, `with_privacy`,
  `with_object_store_name(context.object_store_name)`,
  `with_lease_config(context.lease_config)`,
  `with_novelty_utility_checks(context.novelty_utility_checks)`,
  `with_unqualified_routing(context.unqualified_routing_allowed)`
  (`versioned_pipeline.rs:7496-7616`). Every refusal in 3.1 then passes for a
  correctly configured deployment, and each one still fires for a
  misconfigured one (tests below).
- **A-D11. `pipeline_production_adapters` from the startup path.** This
  result can only pass on the deployed host: the infrastructure profile is
  production only with the GCS artifact store, the Cloud KMS key wrapper and
  managed EdDSA tokens.

  **Where it is emitted.** The pass rule needs
  `infrastructure_profile_from_state(&AppState)`
  (`bin/trace_commons_ingest_internal/pipeline_activation.rs:355`), which
  reads `artifact_store`, `signed_token_verifier`, `db_mirror`,
  `require_db_mirror_writes`, `require_managed_eddsa_signed_tokens` and
  `tokens` off the built state, so the emit runs over the built `AppState`,
  not from the locals after `validate_pipeline_tenant_bundles`. Building the
  profile from the locals was rejected: it would be a second copy of the
  profile's rules, and the activation route reads the state version. At
  `pipeline-production-assembly@b85f09e9a` the emit is at the end of
  `from_env_with_pipeline_runtime_assembler`
  (`trace-commons-ingest.rs:4829-4844`, after `let state = Self { .. }` at
  `:4640` and before `Ok(state)` at `:4845`). That is still ahead of
  `run_ingest`'s scheduler validators (`:1442-1489`) and the listener bind
  (`:1546`), so a boot those refuse could already have written a passing
  result. The decided placement, being made on #1295, is after the scheduler
  validators and after `TcpListener::bind` succeeds, and before
  `axum::serve` (`:1609-1610`): the result then records a process that
  passed every startup refusal. The emit returns its error to `run_ingest`
  with `?`, so a failure there (`pipeline_check_revision_mismatch`, or an
  emitter I/O error) still stops the start, with the port bound but nothing
  served.

  Only when the three `TRACE_COMMONS_PIPELINE_CHECK_*` variables are set
  does ingest emit one result, with `PipelineCheckEmitter::emit` (never
  `emit_from_env`, which panics). Rules:
  - the variables' `code_revision_hash` must equal `DEPLOYED_CODE_REVISION_HASH`
    (`versioned_pipeline_qualification.rs:110-111`), else refuse the start
    with `pipeline_check_revision_mismatch`;
  - an already-emitted result in that directory is logged
    (`pipeline_production_adapters_already_emitted`) and the start continues;
  - status `pass` only when `pipeline_runtime_is_production_qualified` is true,
    the infrastructure profile (`infrastructure_profile_from_state`,
    `bin/trace_commons_ingest_internal/pipeline_activation.rs:357`) has no
    blockers, and the privacy boundary classifies prose PII; otherwise `fail`
    with those blockers as `safe_blockers`;
  - it names the default package (`package: Some(service.default_package())`),
    per R-1 and the Q7 amendment;
  - evidence (all labels, digests, counts; no URL, key, tenant id or model
    name in clear):

    ```json
    {
      "schema": "trace_commons.pipeline_production_adapters.v1",
      "runtime_identity_digest": "sha256:...",
      "scorer": {"identity": "near_ai_perplexity_scorer", "descriptor_hash": "sha256:...", "qualified": true},
      "embedder": {"identity": "fastembed_text_embedder", "descriptor_hash": "sha256:...", "output_dim": 1024, "qualified": true},
      "index_reader": {"identity": "usearch_pipeline_index_reader", "qualified": true},
      "index_writer": {"identity": "usearch_pipeline_index_writer", "qualified": true},
      "index_root_shared_with_legacy": false,
      "settlement_adapters": {"trace_credit": {"identity": "internal_trace_credit_ledger", "payout_rail": "none", "qualified": true}},
      "payout_enabled": false,
      "near_settlement_mode": "disabled",
      "authority": {"qualified": true, "tenant_policy_count": 3},
      "privacy": {"backend": "near_ai", "classifies_prose_pii": true, "qualified": true},
      "compatibility_configuration_qualifiable": true,
      "infrastructure_blockers": [],
      "bundle_blockers": []
    }
    ```

    `runtime_identity_digest` is
    `ProductionDependencyProfile::runtime_identity_digest()` (`:1180-1196`), the
    value activation later compares, so the signed result binds the exact
    dependency set that ran.
- **A-D12. Q7 lands here.** `PROMOTION_PACKAGE_CHECKS` grows from four to
  seven (adds the three promotion-only ids), and `evaluate_promotion`'s
  run-id rule becomes per group (R-1). The mirrored lists must change in the
  same commit: `scripts/operator/pipeline_tooling/checks.py`
  (`digests_required`) and `scripts/operator/test_pipeline_tooling.py`
  (`_PROMOTION_ONLY`, `:2456`), whose self-tests require agreement.
  `pipeline_tooling/report.py` reads `digests_required` from `checks.py`
  (`report.py:342`) and holds no copy of its own. #1295 carries the
  seven-id list (`pipeline-production-assembly@b85f09e9a`
  `versioned_pipeline_qualification.rs:186-194`).

### 4.2 Tests (TDD order)

Each test is written first and seen failing. Unit tests live in
`bin/trace_commons_ingest_internal/production_assembly_tests.rs` (a sibling
`#[path]` file, as the ingest tests already are) unless noted.

1. gate-api: `arc_forwarding_keeps_score_chunk_logprobs` and
   `arc_forwarding_keeps_index_snapshot_and_flush` -- an `Arc<T>` forwards
   every default-overridden method (fails today: no impl).
2. `scorer_descriptor_is_a_pure_function_of_the_pin` -- same inputs, same
   bytes; each of model, cutoff and top-k changes the hash; API key,
   timeout, base URL and chunk knobs do not; descriptor contains none of the five
   development markers.
3. `embedder_descriptor_changes_with_model_dim_and_tokens`.
4. `usearch_pipeline_index_meets_the_writer_contract` -- the same cases
   `IsolatedPipelineIndex`'s tests cover: `Inserted`/`Unchanged`,
   `ContentConflict`, `exclude_revision`, `invalidate_revision` idempotence,
   snapshot hash order-independence; plus manifest round-trip across reopen
   and `pipeline_vector_index_manifest_mismatch` on a truncated manifest.
   Under `near-ai-scorer` only (usearch is feature-gated in gate-enclave).
5. `pipeline_index_root_must_not_be_the_legacy_root` --
   `pipeline_vector_index_root_shared` for equal and nested roots.
6. `internal_trace_credit_adapter_is_idempotent_and_qualified`.
7. `tenant_policy_authority_matches_legacy_admission` -- table-driven parity
   with the legacy admission check for present, absent and empty-policy
   tenants (O-A1 resolved by reading the legacy code first).
8. `production_assembler_passes_every_startup_refusal` -- through
   `assemble_ingest_pipeline_runtime` with `PipelineGateComponents` filled
   by qualified doubles (default features): production-qualified, no main-gate mismatch, no zero floor,
   privacy required and met.
9. `production_assembler_refuses_what_it_must` -- one case per label:
   all-zero floors (`compatibility_zero_floor`), floors differing from `main`
   (`pipeline_runtime_main_gate_config_mismatch`), no privacy backend under
   the requirement (`pipeline_privacy_filter_required`), privacy backend
   `None` (`pipeline_runtime_dependencies_not_production_qualified` with tenants
   routed), unqualified routing set (`pipeline_unqualified_routing_with_production_runtime`).
10. `runtime_selection_is_opt_in` -- unset: `None`; `production` on a default
    build: `pipeline_runtime_production_requires_near_ai_scorer`; other value:
    `pipeline_runtime_selection_unknown`; production without
    `enclave_near_ai`: `pipeline_runtime_production_requires_enclave_near_ai`;
    any NEAR settlement mode leaves `payout_enabled()` false.
11. `default_build_serves_no_pipeline` -- default-features `main` path still
    passes `None` and `/v1/pipeline/readiness` answers `pipeline_runtime_absent`.
12. `production_adapters_check_is_emitted_once_with_its_evidence` -- emitter
    in a temp dir: pass result names the package, evidence matches the schema
    above, `evidence_hash` recomputes, a second boot does not refuse, a
    revision mismatch refuses, a missing privacy backend emits `fail` with
    blockers, and a boot refused after `AppState` is built (a scheduler
    validator, or a bind failure) writes no result.
13. `promotion_package_checks_include_the_promotion_only_three` and
    `promotion_run_rule_is_per_group` in `versioned_pipeline_qualification.rs`
    unit tests; Python `test_pipeline_tooling.py` list-agreement tests updated.
14. pg (ingest binary; not `#[ignore]`, so it runs whenever
    `TRACE_COMMONS_PG_TEST_DATABASE_URL` is set and returns early, reporting
    `passed`, when it is not):
    `production_assembly_serves_a_routed_tenant_end_to_end` -- the production
    assembler over qualified scorer and embedder doubles and a temp-root
    usearch index (under `near-ai-scorer`) or a qualified wrapper over
    `IsolatedPipelineIndex` (default), receipt to Settle, credit row written,
    no payout.

### 4.3 CI

- Default-features tests (1-3, 5-13) run in `cargo test (default features)`.
  The default arm of 14 runs in `trace-commons-ingest tests, whole bin,
  against PostgreSQL` (`ingest-bin-postgres`, `.github/workflows/ci.yml:595`),
  which builds default features only and keeps its xact_commit floor guard.
- `cargo check (near-ai-scorer)` (`ci.yml:954-1001`) gets one step,
  `cargo test -p trace-commons-server --features near-ai-scorer --bin trace-commons-ingest production_assembly`
  with the existing "`[1-9][0-9]* passed`" grep guard (copy of the
  gcs-client step at `:992-997`); #1295 adds it
  (`pipeline-production-assembly@b85f09e9a` `ci.yml:998-1008`). That job has
  no PostgreSQL service and no `TRACE_COMMONS_PG_TEST_DATABASE_URL`, so this
  step runs test 4 and the other non-pg tests only. The `near-ai-scorer` arm
  of 14 would return early there and still count as passed, so it is not
  claimed for that job.
- The `near-ai-scorer` arm of 14 (the usearch-backed end-to-end path) runs
  in a `near-ai-scorer` variant of `ingest-bin-postgres`: the same
  PostgreSQL service, database URL and xact_commit floor guard, building the
  ingest binary with `--features near-ai-scorer`. #1295 is adding it; it is
  not on `b85f09e9a`. Without it the usearch path runs nowhere in CI.
- No network in either job: tests construct no `NearAiPerplexityScorer`
  and load no fastembed model.
- `cargo check (default features)` and `cargo check (local-gpu-models,
  non-CUDA)` must stay green; the forwarding impls and the cfg split are what
  they check.

## 5. Slice B: operator promotion checks

### 5.1 Decisions

- **B-D1. A `promote` command, operator-only.** `pipeline.py promote
  --package <signed production package> --trusted-key <path>
  --results-dir <dir>` runs on the operator host against a build of the
  deployed revision with the pilot's feature set (`near-ai-scorer`, plus
  `gcs-client` and `gcp-kms` for the remote store and the key wrapper). It refuses to start when
  `CI` is set in the environment (`promote_refused_in_ci`), so no CI job makes
  a network call. Subcommands (each one result, each re-runnable alone into
  the same run directory, signed at the end with `--signing-key`):
  - `promote package-checks`: the four `PROMOTION_PACKAGE_CHECKS` re-run with
    the harness assembler switched to the Slice A production assembler
    (`TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY=production`, honoured only in a
    `near-ai-scorer` test build; any other build refuses with
    `harness_production_assembly_unavailable`). Real NEAR AI scoring, real
    bge embedding, usearch index in the run directory. The HF corpus run in
    this mode uses the network pin (B-D3), so `pipeline_http_corpus_hf_local`
    here is produced from a real download.
    The switch is read in four harnesses: `CorpusAssembler`
    (`pipeline_corpus_pg_tests.rs`), the restore seed and resume, and the
    bundle-qualification test in `tests/versioned_pipeline_runtime_pg.rs`.
    The last one reaches the assembler only because A-D1 puts it in the
    library crate. In that test the switch replaces the candidate service
    (`compatibility_test_service(..., qualification_candidate_config())`,
    `origin/main` `:2866-2871`) with one built by the production assembler
    from the operator's env file, and changes these assertions
    (`:2880-2890`):

    | Assertion today (reference mode) | Production mode |
    |---|---|
    | `candidate_scorer_is_reference` (identity `reference_perplexity_test_only`) | identity is `near_ai_perplexity_scorer` |
    | `candidate_embedder_is_reference` (identity `reference_embedder_test_only`) | identity is `fastembed_text_embedder` |
    | `!candidate_qualification.scorer.production_qualified` | `production_qualified` |
    | `!candidate_qualification.embedder.production_qualified` | `production_qualified` |
    | `!candidate_qualification.configuration_qualifiable` | `configuration_qualifiable` |

    Unchanged in both modes: `candidate_resolved`, the
    `assert_ne!(candidate.bundle_id, package.bundle_id)` at `:2891`, and the
    constructor-object proof over the Q/U counting scorers (earlier in the
    same test),
    which is about `construct`, not about the candidate. The emitted evidence
    (`emit_pass_from_env`, `:2898-2912`) replaces the two `_is_reference`
    booleans with the two identity labels and adds
    `"harness_assembly": "reference"|"production"`, so a reference result can
    never be read as a production one. In reference mode the test behaves
    exactly as today.
  - `promote remote-restore` -> `pipeline_remote_restore` (B-D2).
  - `promote hf-canary` -> `pipeline_hf_network_canary` (B-D3).
  - `promote adapters` collects `pipeline_production_adapters`; it does not
    produce it. Only the deployed ingest's boot can pass it (A-D11). Handoff:
    `promote init` chooses the production run's `run_id` and prints it with the
    code revision; the owner sets `TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR`,
    `TRACE_COMMONS_PIPELINE_CHECK_RUN_ID` (that id) and
    `TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH` on the service for one
    boot, then copies the two files into the run directory; `promote adapters`
    verifies run id, revision and package and refuses otherwise
    (`promote_adapters_result_mismatch`, `promote_adapters_result_missing`).
  - `promote assemble` builds the 22-file submission from the mechanics run
    and this run (section 2).
  - `promote sign` signs the seven results with the operator's check key
    (`attest_results`, `pipeline.py:970`).
- **B-D2. `pipeline_remote_restore` evidence.** The drill restores the
  production artifact store's objects for one throwaway tenant from the
  versioned remote bucket into a scratch prefix (or scratch bucket) the
  operator names, and checks them against a fingerprint taken from the live
  store before the restore. It does not restore into the live prefix. Evidence:

  ```json
  {
    "schema": "trace_commons.pipeline_remote_restore.v1",
    "object_store_kind": "gcs",
    "source_store_name_hash": "sha256:...",
    "scratch_store_name_hash": "sha256:...",
    "object_count": 42,
    "artifact_fingerprint": "sha256:...",
    "restored_artifact_fingerprint": "sha256:...",
    "versioning_enabled": true,
    "kek_unwrap_verified_count": 42,
    "database_fingerprint": "sha256:...",
    "pending_runs_resumed": 1,
    "duplicate_effects": 0
  }
  ```

  `artifact_fingerprint` uses the same per-object SHA-256 listing
  `artifact_fingerprint` uses locally (`pipeline.py:625`), computed over
  ciphertext as stored; `kek_unwrap_verified_count` proves the restored
  objects decrypt under the configured key wrapper (GCP KMS on the pilot)
  without writing plaintext anywhere. The database half reuses
  `pipeline_restore_seed`/`pipeline_restore_resume` against a restored
  dump with `TRACE_COMMONS_PIPELINE_ARTIFACT_ROOT` replaced by the scratch
  remote store. Pass requires the two fingerprints equal, unwrap count equal
  to object count, versioning on, and the resume evidence equal to the seed's
  (the same equality `run_restore_drill` asserts, `pipeline.py:742-761`).
  It names the production package (A-D12).
- **B-D3. The real HF pin and the canary.** Add
  `crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/pin-network.json`:
  `pin-local.json`'s schema and fields minus `local_jsonl_dir`. Its digests
  can only come from a live download, so:
  - `pipeline.py hf-pin record --revision <commit> --output pin-network.json`
    downloads the pinned revision through `trace-commons-pipeline-corpus-export`
    (`HfJsonlDataset::open_at_revision`,
    `src/bin/trace-commons-pipeline-corpus-export.rs:145`) with
    `--cache-dir` inside the run directory (`corpus.py` already keeps every HF
    download inside the worktree), and writes the computed `source_digest`,
    `configuration_digest`, `order_digest`, `bootstrap_corpus_digest`,
    `holdout_corpus_digest`. The owner commits the file in a PR; it is never
    regenerated by a canary.
  - `promote hf-canary` downloads again against `pin-network.json` and
    verifies every digest. Its result names the production package: all
    three of `package_hash`, `configuration_digest` and `dependency_digest`,
    the triple `promote init` recorded for the run, because A-D12 makes
    `pipeline_hf_network_canary` package-bearing and `evaluate_promotion`
    adds `qualification_evidence_package_missing` to a package-bearing
    result without them. #1293 does this: every `promote` result goes
    through `write_result`, which writes the run's triple
    (`pipeline-promotion-checks@e889b5f0e`
    `scripts/operator/pipeline_tooling/promote.py:354-391`, the triple at
    `:369` and `:376-378`), `hf_canary` calls it (`:551`), and
    `test_hf_canary_evidence_shape` asserts the triple
    (`scripts/operator/test_pipeline_tooling.py:4923-4930`). Evidence, as
    #1293 builds it:

    ```json
    {
      "schema": "trace_commons.pipeline_hf_network_canary.v1",
      "repository_owner": "jedisct1",
      "repository_name": "security-audits",
      "revision": "<40-hex commit>",
      "pin_hash": "sha256:...",
      "source_digest": "sha256:...",
      "order_digest": "sha256:...",
      "bootstrap_corpus_digest": "sha256:...",
      "holdout_corpus_digest": "sha256:...",
      "downloaded_file_count": 2,
      "cache_dir_inside_run": true
    }
    ```

    The repository and revision are public dataset coordinates, not operator
    secrets, so they appear in clear; the repository is split at the `/`,
    which the evidence alphabet does not allow. A digest mismatch is `fail`
    with `hf_pin_digest_mismatch_<field>`; a download that left no JSONL file
    is `fail` with `hf_network_download_missing`; a cache outside the run
    directory is `fail` with `hf_network_cache_outside_run`. Whether the revision in `pin-local.json`
    (`6d527ff0...`) is a real commit of that dataset is unknown; the first
    `record` run answers it (O-B2).
  - CI keeps `pin-local.json` and makes no network call.
- **B-D4. How qualify consumes them.** `qualify` itself is unchanged except
  that it stops claiming to be the whole set. `evaluate_promotion` (Rust) gets
  one new, narrow rule: `filesystem_restore_local_only` on
  `pipeline_restore_drill` is discharged when the set holds a passing
  `pipeline_remote_restore` with no safe blockers, the same
  `code_revision_hash`, and the same package. Every other safe blocker still
  blocks. With R-1, the production run's own `pipeline_restore_drill` (from
  `promote package-checks`) is the one in the set, and it still carries the
  local blocker because its artifact copy is local; the remote check is what
  discharges it. The 22 ids, the trust store, the age ceiling
  (`EVIDENCE_AGE_CEILING_SECONDS`, `pipeline.py:82`) and the
  `{signed_package, attestations}` route body are unchanged; the operator
  submits the union of the mechanics run's 15 attestations and the
  production run's seven.
- **B-D5. `pipeline.py package` builds the production package offline.**
  `pipeline.py package --bundle production` builds the compatibility package
  with the production descriptors (A-D4, A-D5) from the deployment's env file,
  with no network and no model load, through `pipeline_package_write`
  (`pipeline.py:69`). The package hash is then known before any network run.

### 5.2 Tests (TDD order)

1. Rust unit, `versioned_pipeline_qualification.rs`:
   `remote_restore_discharges_only_the_local_restore_blocker` (passes with a
   matching remote result; still blocked with a failing one, one from another
   revision or package, or one carrying its own blocker; other blockers on the
   restore drill still block).
2. Python, `scripts/operator/test_pipeline_tooling.py`:
   `test_promote_refuses_in_ci`, `test_promote_requires_near_ai_build`,
   `test_hf_pin_network_has_no_local_dir`,
   `test_hf_canary_evidence_shape` (also asserts the result names the run's
   package triple), `test_remote_restore_evidence_shape`,
   `test_remote_restore_evidence_rejects_url_like_values`
   (reuse `_reject_url_like_arguments`, `pipeline.py:152`),
   `test_promote_sign_signs_exactly_seven`,
   `test_promote_adapters_refuses_another_run_or_revision`,
   `test_promote_assemble_builds_22_and_refuses_doubles`. All offline: the download and the
   bucket calls are injected callables in the tests.
3. Rust, harness assembly switch (B-2):
   `harness_production_assembly_requires_the_feature` (default build refuses
   with `harness_production_assembly_unavailable`),
   `production_corpus_mode_compares_only_deterministic_fields`, and, in
   `tests/versioned_pipeline_runtime_pg.rs`,
   `bundle_qualification_production_mode_inverts_the_reference_assertions`
   (`near-ai-scorer` only; the production assembler over qualified doubles,
   no network): the five assertions of the B-D1 table hold in their
   production form, the evidence carries `"harness_assembly": "production"`,
   and the emitted result names the production package. The reference-mode
   test is unchanged and must still pass.
4. Manual, operator host, recorded in the PR that lands the pin: one
   `hf-pin record` run, one `promote hf-canary`, one `promote remote-restore`
   against a scratch bucket, with output pasted.

### 5.3 CI

Rust unit tests in `cargo test (default features)`; Python tooling tests in
the `pipeline qualification and restore` job (`ci.yml:710-776`), which already
runs the tooling self-tests. Nothing in CI calls `promote` except its refusal
test. The `pipeline qualification and restore` job stays non-required.

## 6. Slice C: gate-decision rows

**Decision (Recommended; proceeding unless overruled):** the pipeline writes a
`trace_gate_decisions` row from its Settle phase, inside Settle's commit
transaction, idempotent per submission, so the four consumers stay
unmodified.

### 6.1 Decisions

- **C-D1. Where, and the lock order.** In `PgPipelineStore::commit_settle`
  (`src/versioned_pipeline.rs:4677` on `main`;
  `pipeline-gate-decision-rows@15cc64951` `:4782`, the write at `:4820`),
  after `ensure_current_lease` and `lock_runnable_policy` and before the
  run's state change, on the same `tx`, for runs whose bundle is the
  compatibility bundle (the only bundle whose Score evidence has the legacy
  fields). A stale lease or a suspended policy therefore writes no row. A run
  rejected in Admission or Review never reaches Settle and gets no row, as
  the legacy gate driver writes none for a submission it does not score.

  Locks, in the order `commit_settle` takes them (line numbers on
  `pipeline-gate-decision-rows@15cc64951` `versioned_pipeline.rs`, V114
  numbers from 3.7):

  1. the run's `pipeline_runs` row, `FOR UPDATE` (`ensure_current_lease`,
     `:4797`, statement at `:6618-6621`);
  2. the Settle policy's `pipeline_bundle_policy_status` row, `FOR SHARE`
     (`lock_runnable_policy`, `:4803`, statement at `:6659-6661`);
  3. the new `phase_outcomes` row (`insert_outcome`);
  4. the new `trace_gate_decisions` row and its keys in the primary key and
     in `trace_gate_decisions_one_pipeline_row` (`:4927-4939`); a concurrent
     insert of the same key waits on this transaction;
  5. only when external growth is enabled, through the V114 trigger:
     the switch row `FOR SHARE`, then the dependency-lock row
     `submission:<tenant>:<submission>` (plus `cluster:<id>` for a cluster
     already on another gate row of the submission), then the frontier rows
     of accounts with a `gate_evaluation` fact on the submission, the
     decision or such a cluster;
  6. the `UPDATE pipeline_runs` that completes the run, on the row step 1
     already holds.

  This adds an edge from the pipeline's locks to the account-trust locks; it
  adds none the other way. Every path that takes a dependency-lock or
  frontier row while holding a pipeline lock takes the pipeline lock first:
  the withdrawal (`withdraw_submission`: `pipeline_runs` `FOR UPDATE` at
  `:4112`, `trace_submissions` `FOR UPDATE` at `:4126`, then the dedup clear
  through `withdraw_pipeline_content_on_tx` `:6870` and
  `invalidate_pipeline_exports_on_tx` `:6920`), the revocation and
  withdrawal follow-ups (`follow_up_inoperable_submission`: runs `:4554`,
  submission `:4570`, clear through `withdraw_pipeline_content_on_tx` at
  `:4575`) and the retention follow-up (runs `:4633`, submission `:4648`,
  clear at `:4670`). Every other writer that
  reaches the V114 locks -- `main`'s `insert_trace_gate_decision`, the dedup
  and credit-quality sweeps, `trace_record_account_trust_fact`
  (`db/postgres_account_trust.rs:28`), `trace_account_trust_merge`
  (`db/postgres.rs:5241`), `trace_account_trust_enable_external_growth`
  (`admission_ledger.rs:644`) and the external evaluation recorder -- is in
  code that takes no `pipeline_runs` or `pipeline_bundle_policy_status` lock:
  outside `versioned_pipeline.rs` those tables are only read without a
  locking clause (`db/postgres.rs:5344, 5403`,
  `versioned_pipeline_activation.rs:553-765, 1294-1309`,
  `versioned_pipeline_product.rs`). The one exclusive lock on a policy row,
  `intervene_policy` (`:1820`; `FOR UPDATE` at `:1865`, update at `:1904`),
  locks no other row in this order. So the order is
  strictly layered -- pipeline run rows, then submission and policy rows,
  then gate-decision rows, then the growth switch, then dependency keys in
  `COLLATE "C"` order, then frontiers in `(tenant_id, account_id)` order --
  and no cycle through a pipeline lock exists. Enabling growth while a
  Settle commit is in flight waits for that commit (the switch row), and a
  Settle that starts after it waits for the enable to commit; the enable
  holds no pipeline lock.

  Two consequences that are not inversions. `commit_settle` runs at READ
  COMMITTED (`tenant_transaction`, `:1543-1560`), so a dependency-lock row
  another writer holds makes it wait, not fail with a serialization error.
  And for a submission's first pipeline row, no `gate_evaluation` fact can
  exist yet (the fact names a decision that did not exist), so step 5 takes
  the switch row and one dependency key and no frontier row; once it holds
  the key it waits on nothing else, so it cannot close a cycle among the
  account-trust locks either. The multi-row dedup clear in
  `withdraw_submission` (several submissions of one source session) fires
  the trigger once per row, each with its own sorted key set, so across rows
  its keys are not in one global order; it can deadlock against another
  multi-row account-trust writer touching the same clusters in the other
  order. PostgreSQL detects that and aborts one transaction (`40P01`); the
  withdrawal returns an error and writes nothing. That is the same exposure
  `main`'s multi-row writers have, and it involves no pipeline lock.
- **C-D2. Idempotency and the second-run conflict.** `decision_id` =
  UUIDv5 over `trace_commons.pipeline_gate_decision.v1\n<tenant_id>\n<run_id>`
  (`pipeline_gate_decision_id`, `pipeline-gate-decision-rows@15cc64951`
  `versioned_pipeline.rs:220`), inserted with a **targetless**
  `ON CONFLICT DO NOTHING` (`:4939`). V116's partial unique index (C-D4) allows
  one pipeline row per submission. V92 allows several runs per submission
  (the primary key is `(tenant_id, run_id)` and `idx_pipeline_runs_submission`
  is not unique), and a second run has a new `run_id` and so a new
  `decision_id`. With a conflict target of `(tenant_id, decision_id)` its
  insert would miss that arbiter, hit the partial index, raise
  `unique_violation` and roll back Settle on every retry. With no target,
  both the primary key and the partial index are arbiters, so the insert
  writes nothing instead. After a no-op insert, an owner check reads the
  submission's pipeline row (`:4978-4993`): when it names this run (a
  retried commit) the commit proceeds; when it names another run, the
  commit refuses with `pipeline_gate_decision_conflict` and writes nothing.
  That label, and `pipeline_gate_decision_evidence_incomplete`, are
  classified as terminal: `process_claimed_run` fails the run instead of
  spending its attempts (`:10796-10805`), and the Settle commit path
  re-raises them bare (`:12740-12744`). The run is failed, not left wedged
  in Settle.

  The evidence check runs before any leg pays: Settle's Step 1 builds
  `PipelineGateDecisionScoreValues::from_evidence` from the committed Score
  evidence for a compatibility bundle (`:11845-11851`), so evidence the row
  cannot be built from fails the run with nothing settled. The commit
  repeats the check and refuses with the same label only if the outcome
  changed in between. A value too large for its column saturates
  (`i64::MAX`, `i32::MAX`) as `main`'s gate writer stores it, rather than
  refusing (`from_evidence`, `:175-210`): the chunk aggregate saturates a
  perplexity to `u64::MAX` on purpose.
- **C-D3. Columns from pipeline evidence.** From the Score outcome's
  `ScoreEvidence` (`gate-api pipeline.rs:1703`; set by
  `CompatibilityScorePolicy::evaluate`, `versioned_pipeline_compat.rs:500-530`):

  | Column | Source |
  |---|---|
  | `submission_id` | run |
  | `gate_policy_version` | `pipeline:<bundle_id>` (non-empty; V85 drops facts with an empty version) |
  | `gate_version_hash` | the Score policy's `configuration_hash` |
  | `perplexity_micros`, `tail_fraction_micros`, `peak_perplexity_micros` | evidence |
  | `perplexity_passed` | `quality_passed` (O-C1: confirm it is the legacy perplexity-and-tail predicate) |
  | `novelty_score_micros`, `peak_novelty_micros`, `novelty_passed` | evidence |
  | `nearest_neighbor_hash` | evidence |
  | `embedding_evidence_hash` | `embedding_artifact_hash`, or the SHA-256 of `pipeline_no_index_command` when Score sealed none |
  | `attestation_chain_hash` | the Score phase outcome's content hash |
  | `decided_at` | Settle commit time (`NOW()` in the transaction) |
  | `chunk_count`, `total_chunk_count`, `chunks_capped` | evidence |
  | `credit_quality_micros`, `credit_quality_calibration_version` | evidence `credit_quality_micros`, `credit_quality_version` |
  | `index_cardinality_at_scoring` | evidence `index_cardinality` |
  | `credit_withheld_reason` | the Settle leg's `error_label` when the `NoveltyUtility` checks withheld it, else NULL |
  | `vector_entry_id`, `vector_index_snapshot_id` | NULL (pipeline entries are revision-keyed; the snapshot id is a hash, not a UUID) |
  | `agent_prose_*`, `tool_result_*`, `attributed_token_fraction_micros` | NULL (O-C2) |
  | `dedup_*`, `contributor_*`, `correction_*`, `composite_score_micros`, `qualifying_token_fraction_micros` | NULL; filled by the existing sweeps |

  Pre-filling `credit_quality_micros` matters: the credit-quality sweep
  (`list_gate_decisions_for_credit_scoring`, `db/postgres.rs:5545`) selects
  every row with `perplexity_micros > 0` and does not skip rows that already
  have a value, so it would overwrite the pipeline's own quality. #1294
  excludes `source = 'pipeline_settle'` there
  (`pipeline-gate-decision-rows@15cc64951` `db/postgres.rs:5578`). The admin
  perplexity re-score would likewise rewrite a pipeline row's verdict, which
  the row's `attestation_chain_hash` covers, so its enumeration leaves out
  any submission with a pipeline row (`db/postgres.rs:5520-5532`) and its
  writers refuse one (`trace_corpus_pg.rs:6992, 7038`). Those two exclusions
  are the consumer changes this slice makes (O-C3, answered by #1294).
- **C-D4. Migration V116** (`V115` is the highest on `origin/main`, verified
  with `git ls-tree origin/main migrations/`):
  `migrations/V116__pipeline_gate_decision_rows.sql` adds
  `source TEXT NOT NULL DEFAULT 'legacy_gate' CHECK (source IN ('legacy_gate','pipeline_settle'))`
  and `pipeline_run_id UUID NULL`, a `CHECK ((source = 'pipeline_settle') = (pipeline_run_id IS NOT NULL))`,
  and `CREATE UNIQUE INDEX ... ON trace_gate_decisions (tenant_id, submission_id) WHERE source = 'pipeline_settle'`.
  Both columns default so the legacy writers are unchanged. Registered in
  `db/postgres.rs`'s migration list like V92 (`:1593`).
- **C-D5. RLS and grants.** RLS is already forced with the tenant policy
  (V23:57-63); the Settle transaction is a tenant transaction
  (`Self::tenant_transaction`, `versioned_pipeline.rs:4689`) on the trace
  pool, the same pool and login the legacy `insert_trace_gate_decision` uses,
  so `WITH CHECK (tenant_id = trace_current_tenant_id())` holds.

  The privilege to insert is not a migration grant. The V90 header
  (`migrations/V90__ingest_runtime_grants.sql:1-25`) says the table-wide
  grant to `trace_ingest_runtime` was made once, by hand, on the pilot when
  its schema was at V62; elsewhere V90 creates the group with no table-wide
  grant, and no migration grants it INSERT on `trace_gate_decisions` or
  UPDATE on its dedup columns. So on a fresh database whose ingest login
  holds only `trace_ingest_runtime`, both `main`'s gate writer and the
  pipeline's are denied; a deployment that is not the pilot runs ingest as
  the owning login, or grants by hand as V90's header tells the operator to.
  V116 as built (`pipeline-gate-decision-rows@15cc64951`) grants only
  `SELECT (source)` to `trace_gate_driver`, for the credit-quality filter.
  `pipeline_gate_decision_insert_works_under_the_runtime_role`
  (`tests/versioned_pipeline_runtime_pg.rs:43532` on that branch) passes
  because its harness migrates the pilot's way: `migrate_like_the_pilot`
  (`tests/support/pilot_runtime_login.rs:42`) applies the pilot's
  `GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO
  trace_ingest_runtime` at V62 (`pilot_runtime_grants.rs:13-15`). It proves
  the pilot's shape, not a fresh one. O-C4 is answered: no. The pipeline
  writer needs exactly the privilege `main`'s writer needs, so this slice
  opens no new gap. Whether V116 should grant `INSERT` and `UPDATE
  (dedup_simhash, dedup_cluster_id, dedup_cluster_size, dedup_signal_version)`
  to `trace_ingest_runtime` explicitly, so the pipeline path does not rest on
  the pilot's one-time grant, is left to the owner (O-C4).
- **C-D6. Withdrawal and revocation.** The legacy withdrawal clears the
  dedup columns by submission (`clear_trace_dedup_cluster_for_submission`,
  `trace_corpus_pg.rs:4865-4888`), and the drain report's
  `withdrawal_completion_pending` requires that
  (`versioned_pipeline_activation.rs:862-866`). The pipeline's withdrawal and
  revocation follow-ups (`follow_up_withdrawal`, `follow_up_revocation`,
  `versioned_pipeline.rs:4238-4300`; retention `follow_up_retention`, `:4516`)
  run the same UPDATE on the pipeline row in their own tenant transaction and
  nothing else: the legacy path writes no withdrawal value into
  `credit_withheld_reason` (its only writer is the gate path,
  `trace_corpus_pg.rs:6943`), and `list_scores_by_submission_ids` returns that
  column, so inventing one would be a consumer-visible divergence. The row is
  not deleted: account trust records
  `submission_withdrawn` from `trace_submissions`, and the gate evaluation fact
  stays true history. Whether the legacy HTTP withdrawal already runs its
  mirror functions for pipeline-owned submissions (in which case the pipeline
  needs no extra UPDATE) is to be confirmed first (O-C5); the test below
  decides.
- **C-D7. What the consumers then do, unmodified.** A pipeline row is
  written with `dedup_simhash` NULL, and the periodic sweep does not derive
  one: `run_recluster_dedup_pass` (`trace-commons-ingest.rs:57304` on
  `main`) clusters only rows that already have a simhash
  (`.filter(|row| row.dedup_simhash.is_some())`, `:57320`). Only the
  operator-run `POST /v1/admin/rederive-dedup` (`:8930`;
  `run_rederive_dedup_pass`, `:58047`, through `list_dedup_rederive_rows`)
  derives a simhash for such a row from its envelope. Until an operator
  runs it, pipeline rows are not clustered and `dedup_cluster_size` stays
  NULL for the contributor cap. #1294's operator doc says so. The
  contributor cap and account trust otherwise read the row as written. No legacy
  credit is awarded from the row: legacy `NoveltyUtility` credit is written
  inline by the legacy gate path, not by the sweeps (O-C3 confirms).

### 6.2 Tests (TDD order)

All pg tests on a fresh database. They are not `#[ignore]`: each returns
early when `TRACE_COMMONS_PG_TEST_DATABASE_URL` is unset, so a run without a
database reports them as passed and proves nothing. Names below are as
#1294 built them (`pipeline-gate-decision-rows@15cc64951`).

1. `v116_adds_source_and_pipeline_run_id_with_defaults` (in
   `db/postgres/pipeline_upgrade_tests.rs`): legacy insert unchanged, CHECKs
   enforced, partial unique index present.
2. `settle_writes_one_gate_decision_row_per_submission`
   (`tests/versioned_pipeline_runtime_pg.rs`): a compatibility run reaching
   Settle writes exactly one row with the columns of C-D3; and
   `settle_gate_decision_row_is_idempotent_per_submission`: Settle's commit
   cannot run twice for one run (transition guard and lease), so a retry
   after a crash after commit amounts to the same row offered again, which
   is a no-op.
   `a_pipeline_row_naming_another_run_refuses_settle` (`:44221`) covers the
   second run: a pipeline row for the same submission naming another run
   fails Settle with `pipeline_gate_decision_conflict`, terminally, and
   leaves the rows as they were. It seeds the other run's row directly
   rather than driving a second run through Settle, because
   `insert_pipeline_receipt` is meant to keep a second run from being
   created. `saturated_score_evidence_saturates_on_the_gate_decision_row`
   (`:44126`) covers the saturation in C-D2.
3. `stale_settle_writes_no_gate_decision_row` and
   `suspended_settle_policy_writes_no_gate_decision_row`.
4. `rejected_run_writes_no_gate_decision_row`.
5. `pipeline_gate_decision_insert_works_under_the_runtime_role` -- the
   runtime login, a member of `trace_ingest_runtime` only, on the pilot's
   grant shape (C-D5).
6. `withdrawal_clears_pipeline_row_dedup_columns`,
   `revocation_clears_pipeline_row_dedup_columns`,
   `retention_clears_pipeline_row_dedup_columns`,
   `withdrawal_leaves_credit_withheld_reason_as_legacy_does`; the drain
   report's `withdrawal_completion_pending` reads zero afterwards.
7. `consumers_see_pipeline_rows` -- the dedup rederive, contributor-cap and
   account-trust fact paths each pick up a pipeline row (one assertion each,
   calling the existing functions).
8. `credit_quality_sweep_does_not_overwrite_pipeline_rows`,
   `perplexity_rescore_enumeration_skips_pipeline_submissions`,
   `perplexity_rescore_writers_leave_pipeline_rows_alone` (O-C3).
9. `legacy_gate_driver_still_skips_pipeline_submissions` (regression on
   `db/postgres.rs:5333-5336`).

### 6.3 CI

`database suites against a real PostgreSQL` (`postgres-suites`, `ci.yml:78`)
runs `cargo test -p trace-commons-server --test versioned_pipeline_runtime_pg`
(`ci.yml:420`) with a database and checks that the suite provisioned its
runtime role (`:429`), and runs the upgrade tests. The new tests are not
`#[ignore]`, so they run there with no selector change.

## 7. What stays the owner's

- Ruling on every open question in section 10 (section 2, O-A2, O-A3 and the
  stage 3 settlement precondition of O-A4 are decided).
- Merging each slice. No agent merges, approves or arms the queue.
- Every pilot write: deploying a build, setting
  `TRACE_COMMONS_PIPELINE_RUNTIME=production` and
  `TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT`, running V116, creating the
  throwaway tenants, any bucket or KMS operation of the remote-restore drill.
- Holding the check-signing key and signing the seven production results.
- Running `hf-pin record` and committing `pin-network.json`.
- Activation, rollback, containment and deactivation of any tenant.
- Choosing the stage 4 tenant.
- Signing the stage 4 abort rule in the change record before activation.
- Deciding when stage 3d (payout) starts, and the slice that builds it.

## 8. Dependencies

- PR #1291, merged as `d8f98f389`: the compatibility-merge review points this
  design builds on (base of every slice).
- PR #1286 (`pipeline-smoke-runbook`): the stage plan. Not required to merge
  first; when it merges, its "What is not possible yet" section should point
  here.
- Slice B is two PRs. **B-1** (plan tasks B1, B2, B4, B5: the
  `evaluate_promotion` discharge rule, `promote` scaffolding, the HF pin and
  canary, the remote restore drill) branches from `origin/main` and builds
  alone. **B-2** (plan tasks B3, B6: the harness production-assembly switch
  and the offline production package) uses Slice A's descriptor and assembler
  types, so it branches from `origin/main` after Slice A merges. Slice C is
  independent of A and B.
- TraceCommons/trace-commons#1233 (60 s index write margin) bounds A-D6.

## 9. Risks

- **R1. Shared scorer load.** Legacy and pipeline traffic share one NEAR AI
  quota and one embedder. A routed tenant adds load to the CPU-starved host.
  Mitigation: stage 3 traffic is synthetic and small; measure before stage 4.
- **R2. Novelty history restarts.** The pipeline index starts empty and is
  not seeded from the legacy novelty index (O-A2, decided: cold start), so
  the first pipeline traces of a real tenant score novel against nothing the
  legacy index holds. Stage 3's legacy comparison will show it, and the
  stage 4 change record states it.
- **R3. Manifest drift.** A crash between a usearch write and its manifest
  flush leaves them inconsistent; the start then refuses, and the rebuild
  route is the recovery. A slow rebuild extends downtime.
- **R4. Two credit writers.** If any legacy sweep turns a pipeline row into a
  legacy credit event, a pipeline submission would be credited twice. Tests
  C-7/C-8 and O-C3 exist for this; a failing one blocks Slice C.
- **R5. Descriptor drift.** Changing the descriptor format after a
  qualification invalidates it (new package hash). Version the schema string
  and never edit a released one.
- **R6. Evidence leakage.** Evidence and logs must stay hash-only; a review
  pass greps evidence fixtures for URL-like values and model names.
- **R7. Overrun of the 60 s fence margin** by one pipeline index write on a
  large shard: every write flushes usearch and rewrites the namespace's
  manifest (A-D6), and both grow with the shard. There is no `flush_every`
  setting to refuse. `flush_returns_within_the_fence_margin` measures a
  write against a generous bound in CI; the operator measures it again on
  the stage 4 tenant's shard size before stage 4.

## 10. Decisions and open questions

Decided by the owner, 2026-10-08:

- **Section 2: R-1.** The package-bearing checks are re-run against the
  production assembly, operator-only (`pipeline.py promote`, Slice B).
- **O-A2: cold start.** The pipeline index is not seeded from the legacy
  novelty index, for stage 3 or stage 4 (R2).
- **O-A3: not bound.** The scorer descriptor does not bind the NEAR AI
  attestation host or measurement pin for now (A-D4).
- **O-A4, stage 3 precondition: kept.** Stage 3 keeps NEAR payout disabled
  deployment-wide; `TRACE_COMMONS_NEAR_SETTLEMENT_MODE=disabled` stays a
  stage 3 precondition (A-D9).

Answered by what was built:

- **O-C3** (#1294): the credit-quality sweep did not skip rows with a value,
  and the admin perplexity re-score reached pipeline rows; both now leave
  pipeline rows out (C-D3).
- **O-C4** (C-D5): no, a fresh database's `trace_ingest_runtime` holds no
  INSERT on `trace_gate_decisions`; neither does it for `main`'s writer.
  Still open: whether V116 grants it explicitly.

Open:

- **O-A1.** Legacy tenant authority semantics: how the tenant allowlist
  combines with the token-claim allowlist, and what a tenant absent from
  `TRACE_COMMONS_TENANT_POLICIES` gets. Must be read from the admission code
  before A-D8 is written.
- **O-A4, payout slice.** The stage 3d payout slice: a production
  `NearPayoutAdapter` over `main`'s HTTP NEAR submitter and confirmer does
  not exist; is it a fourth slice?
- **O-B3.** Production corpus expectations: a separate expectation set, or
  the deterministic-fields mode the plan picks (section 2)?
- **O-B1.** The remote-restore scratch target: a prefix in the production
  bucket or a separate bucket, and who creates it.
- **O-B2.** Whether `6d527ff0081eec6704c2a4f00e1ef8d308ae7366` is a real
  revision of `jedisct1/security-audits`.
- **O-C1.** Whether `quality_passed` equals the legacy `perplexity_passed`
  predicate for every floor combination.
- **O-C2.** Per-author perplexity for pipeline rows: compute it in the
  compatibility Score (changes Score output and so the package), or leave
  NULL and accept that per-author scoring skips pipeline traffic.
- **O-C5.** Whether the legacy withdrawal route already runs
  `clear_trace_dedup_cluster_for_submission` for pipeline-owned submissions.
- **O-C6.** Whether `trace_submissions.status` for a pipeline submission
  reaches `accepted`, which V85's `evaluated_passed` outcome requires.
