# Pipeline production assembly, promotion checks and gate-decision rows

Status: draft for owner review, 2026-10-08. Base: `origin/main` at
`d8f98f389` (PR #1291). Stage plan: PR #1286
(`docs/operator/pipeline-smoke-tests.md` on branch `pipeline-smoke-runbook`).
Implementation plan:
[`docs/superpowers/plans/2026-10-08-pipeline-production-assembly.md`](../plans/2026-10-08-pipeline-production-assembly.md).

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

Section 2 is a design gap that cuts across A and B. It is the first thing the
owner needs to rule on.

## 2. The production package gap (owner decision)

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

**Recommended resolution (R-1; proceeding on it unless overruled).** Split
qualification into two runs of the same code revision:

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

Costs of R-1 the owner should weigh:

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

Alternatives the owner may prefer:

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

## 4. Slice A: production assembly

### 4.1 Decisions

- **A-D1. Where it lives.** A new module
  `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/production_assembly.rs`
  (AGPL header), declared beside `pipeline_runtime` in the ingest binary. The
  adapter types and the assembler are **not** feature-gated: they hold
  `Arc<dyn PerplexityScorer>`, `Arc<dyn Embedder>` and a usearch-backed index
  behind traits, so the default-features `cargo test` exercises them with
  qualified doubles. Only the function that constructs the NEAR AI scorer,
  fastembed embedder and usearch index from the environment is
  `#[cfg(feature = "near-ai-scorer")]`.
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
  stored artifact; whether the attestation host should be bound is an open
  question, O-A3). Because qualification binds per code revision and per
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
  `src/versioned_pipeline.rs:10126`) into an emptied root. Every call returns
  well within `PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS` (60 s,
  `versioned_pipeline.rs:218`), including a flush. Identities
  `usearch_pipeline_index_reader`/`_writer`, both qualified `true`. Namespace
  = `pipeline:<index_id>:<tenant_storage_ref>`.
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
  couple the pipeline runtime to every legacy tenant's payout. Consequence the
  owner should see: the stage plan's stage 3 precondition "NEAR settlement mode
  is `disabled`", read literally, turns legacy NEAR payout off for the whole
  deployment during stage 3. With this slice it is not needed for the pipeline
  tenant, which cannot pay out; whether to keep it is the owner's call, and
  PR #1286 should say which (O-A4).
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
  managed EdDSA tokens. After
  `validate_pipeline_privacy_filter_requirement` and
  `validate_pipeline_tenant_bundles` pass (`trace-commons-ingest.rs:4051-4065`),
  and only when the three `TRACE_COMMONS_PIPELINE_CHECK_*` variables are set,
  ingest emits one result with `PipelineCheckEmitter::emit` (never
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
  (`report.py:342`) and holds no copy of its own.

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
    blockers.
13. `promotion_package_checks_include_the_promotion_only_three` and
    `promotion_run_rule_is_per_group` in `versioned_pipeline_qualification.rs`
    unit tests; Python `test_pipeline_tooling.py` list-agreement tests updated.
14. pg (ingest binary, `--include-ignored`):
    `production_assembly_serves_a_routed_tenant_end_to_end` -- the production
    assembler over qualified scorer and embedder doubles and a temp-root
    usearch index (under `near-ai-scorer`) or a qualified wrapper over
    `IsolatedPipelineIndex` (default), receipt to Settle, credit row written,
    no payout.

### 4.3 CI

- Default-features tests (1-3, 5-13) run in `cargo test (default features)`
  and `trace-commons-ingest tests, whole bin, against PostgreSQL`
  (`ingest-bin-postgres`, `.github/workflows/ci.yml:595`) for 14.
- `cargo check (near-ai-scorer)` (`ci.yml:954-1001`) today only checks the
  bins and runs gate-enclave's NEAR AI lib tests. Add one step:
  `cargo test -p trace-commons-server --features near-ai-scorer --bin trace-commons-ingest production_assembly`
  with the existing "`[1-9][0-9]* passed`" grep guard (copy of the
  gcs-client step at `:992-997`), so tests 4 and the `near-ai-scorer` arm of
  14 run somewhere. No network: tests construct no `NearAiPerplexityScorer`
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
    verifies every digest. Evidence:

    ```json
    {
      "schema": "trace_commons.pipeline_hf_network_canary.v1",
      "repository": "jedisct1/security-audits",
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
    secrets, so they appear in clear. A digest mismatch is `fail` with
    `hf_pin_digest_mismatch:<field>`. Whether the revision in `pin-local.json`
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
   `test_hf_canary_evidence_shape`, `test_remote_restore_evidence_shape`,
   `test_remote_restore_evidence_rejects_url_like_values`
   (reuse `_reject_url_like_arguments`, `pipeline.py:152`),
   `test_promote_sign_signs_exactly_seven`,
   `test_promote_adapters_refuses_another_run_or_revision`,
   `test_promote_assemble_builds_22_and_refuses_doubles`. All offline: the download and the
   bucket calls are injected callables in the tests.
3. Rust, harness assembly switch (B-2):
   `harness_production_assembly_requires_the_feature` (default build refuses
   with `harness_production_assembly_unavailable`) and
   `production_corpus_mode_compares_only_deterministic_fields`.
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

- **C-D1. Where.** In `PgPipelineStore::commit_settle`
  (`src/versioned_pipeline.rs:4677`), after `ensure_current_lease` and
  `lock_runnable_policy` and before the run's state change, on the same `tx`,
  for runs whose bundle is the compatibility bundle (the only bundle whose
  Score evidence has the legacy fields). A stale lease or a suspended policy
  therefore writes no row. A run rejected in Admission or Review never reaches
  Settle and gets no row, as the legacy gate driver writes none for a
  submission it does not score.
- **C-D2. Idempotency.** `decision_id` = UUIDv5 over
  `trace_commons.pipeline_gate_decision.v1\n<tenant_id>\n<run_id>`
  (the same `Uuid::new_v5(&Uuid::NAMESPACE_URL, ..)` pattern as
  `versioned_pipeline.rs:8737-8740`), inserted with
  `ON CONFLICT (tenant_id, decision_id) DO NOTHING`. Plus V116's partial unique
  index (C-D4) so a second run for the same submission cannot add a second
  pipeline row.
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
  have a value (to verify in the implementation, O-C3); if rescoring a
  pipeline row would overwrite the pipeline's own quality, the sweep must
  exclude `source = 'pipeline_settle'`, which is the one consumer change this
  slice allows.
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
  so `WITH CHECK (tenant_id = trace_current_tenant_id())` holds. The ingest
  runtime group `trace_ingest_runtime` received table-wide grants at V62
  (`migrations/V90__ingest_runtime_grants.sql` header), which covers INSERT on
  `trace_gate_decisions`; V116 adds `GRANT SELECT (source, pipeline_run_id) ON
  trace_gate_decisions TO trace_gate_driver` (that role holds column grants,
  V47/V48/V57) only if a sweep selects them. A pg test proves the insert under
  `SET ROLE trace_ingest_runtime` on a fresh database; if it is denied, V116
  also grants `INSERT` and `UPDATE (dedup_simhash, dedup_cluster_id,
  dedup_cluster_size, dedup_signal_version, credit_withheld_reason)` (O-C4).
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
- **C-D7. What the consumers then do, unmodified.** Dedup sweeps compute
  simhash from the submitted envelope (`list_dedup_rederive_rows`), so a
  pipeline row gains `dedup_simhash` and a cluster on the next sweep;
  contributor cap and account trust read the row as written. No legacy
  credit is awarded from the row: legacy `NoveltyUtility` credit is written
  inline by the legacy gate path, not by the sweeps (O-C3 confirms).

### 6.2 Tests (TDD order)

All pg tests on a fresh database with `--include-ignored`.

1. `v116_adds_source_and_pipeline_run_id_with_defaults` (in
   `db/postgres/pipeline_upgrade_tests.rs`): legacy insert unchanged, CHECKs
   enforced, partial unique index present.
2. `settle_writes_one_gate_decision_row_per_submission`
   (`tests/versioned_pipeline_runtime_pg.rs`): a compatibility run reaching
   Settle writes exactly one row with the columns of C-D3; re-running Settle
   (crash after commit, retried with a new lease) writes none.
3. `stale_settle_writes_no_gate_decision_row` and
   `suspended_settle_policy_writes_no_gate_decision_row`.
4. `rejected_run_writes_no_gate_decision_row`.
5. `pipeline_gate_decision_insert_works_under_the_runtime_role` --
   `SET ROLE trace_ingest_runtime`.
6. `withdrawal_clears_pipeline_row_dedup_columns` and the drain report's
   `withdrawal_completion_pending` reads zero afterwards.
7. `consumers_see_pipeline_rows` -- the dedup rederive, contributor-cap and
   account-trust fact paths each pick up a pipeline row (one assertion each,
   calling the existing functions).
8. `credit_quality_sweep_does_not_overwrite_pipeline_rows` (O-C3).
9. `legacy_gate_driver_still_skips_pipeline_submissions` (regression on
   `db/postgres.rs:5333-5336`).

### 6.3 CI

`database suites against a real PostgreSQL` (`postgres-suites`, `ci.yml:78`)
runs `versioned_pipeline_runtime_pg` and the upgrade tests; with #737's
`#[ignore]` selector semantics the new tests must be added to that job's
selector list, or they never run (project memory, "Ignored pg suites").

## 7. What stays the owner's

- Ruling on section 2 (R-1, R-2 or R-3) and on every open question below.
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
  design builds on (base of all three slices).
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
- **R2. Novelty history restarts.** The pipeline index starts empty, so the
  first pipeline traces of a real tenant score novel against nothing the
  legacy index holds. Stage 3's legacy comparison will show it; stage 4 needs
  an owner decision (seed from legacy, or accept) (O-A2).
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
- **R7. Overrun of the 60 s fence margin** by a usearch flush on a large
  shard; A-D6 measures flush time in the integration test and refuses
  `flush_every` settings that exceed it.

## 10. Open questions

- **O-A1.** Legacy tenant authority semantics: how the tenant allowlist
  combines with the token-claim allowlist, and what a tenant absent from
  `TRACE_COMMONS_TENANT_POLICIES` gets. Must be read from the admission code
  before A-D8 is written.
- **O-A2.** Seed the pipeline index from the legacy novelty index for the
  stage 4 tenant, or accept a cold start?
- **O-A3.** Should the scorer descriptor bind the NEAR AI attestation host or
  measurement pin, so a change of the serving TEE is a new package?
- **O-A4.** The stage 3d payout slice: a production `NearPayoutAdapter` over
  `main`'s HTTP NEAR submitter and confirmer does not exist; is it a fourth
  slice? And does stage 3 keep the deployment-wide "settlement mode
  `disabled`" precondition now that the pipeline tenant cannot pay out (A-D9)?
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
- **O-C3.** Whether the credit-quality sweep skips rows that already hold a
  value, and whether any sweep writes a credit event.
- **O-C4.** Whether `trace_ingest_runtime` on a fresh (non-pilot) database
  holds INSERT on `trace_gate_decisions`.
- **O-C5.** Whether the legacy withdrawal route already runs
  `clear_trace_dedup_cluster_for_submission` for pipeline-owned submissions.
- **O-C6.** Whether `trace_submissions.status` for a pipeline submission
  reaches `accepted`, which V85's `evaluated_passed` outcome requires.
