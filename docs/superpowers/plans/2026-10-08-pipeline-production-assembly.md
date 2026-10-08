# Pipeline production assembly: implementation plan

Spec: [`docs/superpowers/specs/2026-10-08-pipeline-production-assembly-design.md`](../specs/2026-10-08-pipeline-production-assembly-design.md).
Base for every slice: `origin/main` (`d8f98f389` or later). Three branches,
three PRs, any merge order; Slice B's operator paths refuse at run time until
Slice A is deployed.

Before starting any slice: the owner has ruled on spec section 2 (R-1 is the
default). Read `CLAUDE.md` and `AGENTS.md` in the worktree. Every task is TDD:
write the test, run it, see it fail for the stated reason, implement, see it
pass, commit.

Gate for each slice's PR (paste real output, never from truncated output):

```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --features near-ai-scorer --all-targets
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity \
  -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching -D warnings
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server
```

pg tests touched by a slice run on a fresh database
(`createdb -h localhost tc_<branch>_<n>`;
`TRACE_COMMONS_PG_TEST_DATABASE_URL=postgresql://localhost/<db>`;
`-- --include-ignored`; `dropdb` afterwards). Slice A also runs
`cargo check -p trace-commons-server --features local-gpu-models` (CI job
`cargo check (local-gpu-models, non-CUDA)`), since it touches gate-api traits.

---

## Slice A: production assembly

Branch `pipeline-production-assembly`. Files:

- Create `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/production_assembly.rs`
  (AGPL header) and its test sibling `production_assembly_tests.rs`, wired with
  `#[cfg(test)] #[path = "production_assembly_tests.rs"] mod tests;`.
- Modify `crates/trace-commons-gate-api/src/{perplexity,embedder,vector_index}.rs`
  (forwarding impls), `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
  (`main`, component builder split, boot emit, config-status),
  `bin/trace_commons_ingest_internal/pipeline_runtime.rs` (context field),
  `src/versioned_pipeline_qualification.rs` (Q7 lists, run rule),
  `scripts/operator/pipeline_tooling/checks.py`,
  `scripts/operator/test_pipeline_tooling.py`, `.github/workflows/ci.yml`
  (one step), `docs/operator/pipeline-activation.md` and
  `pipeline-qualification.md` ("Current completion", the env vars).

### A1. `Arc<T>` forwarding impls in gate-api

1. Test (gate-api `perplexity.rs` tests): a scorer whose `score_chunk` returns
   non-empty `logprobs`; through `Arc<dyn PerplexityScorer>` the logprobs
   survive. Same for `Embedder::embed` and `VectorIndex::{snapshot, flush}`.
   Fails: no impl.
2. Implement `impl<T: PerplexityScorer + ?Sized> PerplexityScorer for Arc<T>`
   forwarding `score` and `score_chunk`; `Embedder` forwarding `embed`;
   `VectorIndex` forwarding all five methods.
3. `cargo test -p trace-commons-gate-api`; `cargo check` all four feature
   configurations (the forwarding impl must not conflict with any existing
   impl under `local-gpu-models`).
4. Commit: "Forward the gate traits through Arc".

### A2. Split the NEAR AI gate builder into components and orchestrator

1. Test (ingest tests, `near-ai-scorer` only):
   `near_ai_gate_components_parse_without_network` -- with every env var set
   and an unreachable base URL, `NearAiGateComponentsConfig::from_env` parses
   and `NearAiScorerDescriptor::from_env` matches; nothing connects.
2. Extract `NearAiGateComponentsConfig::from_env()` (pure parse) and
   `build_near_ai_gate_components(config)` from
   `build_enclave_near_ai_gate_service_from_env`
   (`trace-commons-ingest.rs:6227`), feature-gated, returning `NearAiGateComponents`; it constructs the NEAR
   AI scorer, the fastembed embedder and the legacy novelty index once as
   `Arc`s. The orchestrator now takes those `Arc`s (A1). The pipeline-facing
   `PipelineGateComponents` (spec A-D3) holds only trait objects and
   descriptors and is not gated; the pipeline index is added to it in A4. Behaviour of the legacy gate is unchanged: the existing
   `compute_gate_version_hash` inputs are byte-identical (pin with a test that
   the hash for a fixed config is the value `main` computes today).
3. Store `Option<Arc<PipelineGateComponents>>` on `AppState` (set only by the
   `enclave_near_ai` arm when the production selection is on).
4. Commit: "Build the NEAR AI gate components once".

### A3. Scorer and embedder adapters with deterministic identities

1. Tests: `scorer_descriptor_is_a_pure_function_of_the_pin`,
   `embedder_descriptor_changes_with_model_dim_and_tokens`,
   `adapter_identities_are_safe_labels`,
   `descriptors_carry_no_development_marker` (the five strings of
   `validate_production_package`).
2. Implement `NearAiScorerDescriptor`, `NearAiPipelineScorer`,
   `FastEmbedPipelineEmbedder` (spec A-D4, A-D5) over `Arc<dyn
   PerplexityScorer>` / `Arc<dyn Embedder>`, not feature-gated.
3. Commit: "Identify the NEAR AI scorer and bge embedder for the pipeline".

### A4. Usearch-backed pipeline index

1. Tests (`near-ai-scorer` only, temp-dir roots): the writer contract cases of
   `IsolatedPipelineIndex`'s tests re-run against `UsearchPipelineIndex`;
   `manifest_survives_reopen`; `truncated_manifest_refuses_open`
   (`pipeline_vector_index_manifest_mismatch`);
   `pipeline_index_root_must_not_be_the_legacy_root`
   (`pipeline_vector_index_root_shared`, equal and nested);
   `flush_returns_within_the_fence_margin` (measured, generous bound).
2. Implement per spec A-D6. Calls stay synchronous; the pipeline already moves
   index calls to the blocking pool (N-6). Add a test with
   `refuse_calls_on_runtime_workers`-style detection if the adapter exposes
   one.
3. Commit: "Back the pipeline index with usearch on its own root".

### A5. Internal Trace Credit settlement adapter and tenant-policy authority

1. Read the legacy admission code for O-A1 first and write the finding into
   the PR body.
2. Tests: `internal_trace_credit_adapter_is_idempotent_and_qualified`;
   `tenant_policy_authority_matches_legacy_admission` (table-driven).
3. Implement `InternalTraceCreditSettlementAdapter`,
   `TenantPolicyPipelineAuthorityProvider` (spec A-D8, A-D9).
4. Commit: "Add the production Trace Credit adapter and tenant authority".

### A6. The production assembler and opt-in selection

1. Tests: `production_assembler_passes_every_startup_refusal`,
   `production_assembler_refuses_what_it_must` (one case per label of spec
   4.2 item 9), `runtime_selection_is_opt_in` (item 10),
   `default_build_serves_no_pipeline` (item 11).
2. Implement `ProductionPipelineAssembler` (spec A-D10) and
   `pipeline_runtime_selection_from_env()`; add
   `IngestPipelineRuntimeContext::components`. `main()`:

   ```rust
   #[tokio::main]
   async fn main() -> anyhow::Result<()> {
       match pipeline_runtime_selection_from_env()? {
           PipelineRuntimeSelection::None => run_ingest(None).await,
           #[cfg(feature = "near-ai-scorer")]
           PipelineRuntimeSelection::Production => {
               run_ingest(Some(&ProductionPipelineAssembler)).await
           }
       }
   }
   ```

   with the refusals of spec A-D2 inside the selection parser. Report
   `pipeline_runtime_selection` on `GET /v1/admin/config-status`.
3. Commit: "Assemble the production pipeline runtime behind an opt-in".

### A7. Emit `pipeline_production_adapters` at startup; Q7 lists

1. Tests: `production_adapters_check_is_emitted_once_with_its_evidence`
   (spec 4.2 item 12); `promotion_package_checks_include_the_promotion_only_three`;
   `promotion_run_rule_is_per_group`; Python list-agreement tests in
   `test_pipeline_tooling.py` updated to the seven-id package list.
2. Implement spec A-D11 and A-D12. The emit runs after
   `validate_pipeline_tenant_bundles` (`trace-commons-ingest.rs:4057-4064`) and
   uses `PipelineCheckEmitter::emit`, never `emit_from_env`.
3. Commit: "Emit the production adapters check from startup".

### A8. End-to-end and CI

1. pg test `production_assembly_serves_a_routed_tenant_end_to_end` in
   `pipeline_http_pg_tests.rs` (default features, qualified doubles in `PipelineGateComponents` behind the
   production assembler); fresh DB, `--include-ignored`. Confirm the
   `ingest-bin-postgres` job's selector includes it.
2. Add to `cargo-check-near-ai-scorer` (`ci.yml:954`):

   ```yaml
   - name: production assembly tests (near-ai-scorer)
     timeout-minutes: 20
     run: |
       set -euo pipefail
       cargo test -p trace-commons-server --features near-ai-scorer --bin trace-commons-ingest production_assembly 2>&1 | tee production-assembly-tests.log
       grep -qE 'test result: ok\. [1-9][0-9]* passed; 0 failed' production-assembly-tests.log
   ```
3. Update the operator docs: new env vars
   (`TRACE_COMMONS_PIPELINE_RUNTIME`, `TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT`),
   new labels, and "Current completion".
4. Commit: "Run the production assembly tests in CI".

---

## Slice B: operator promotion checks

Two PRs (spec section 8). **B-1**, branch `pipeline-promotion-checks`, from
`origin/main`: tasks B1, B2, B4, B5. **B-2**, branch
`pipeline-promotion-production-harness`, from `origin/main` after Slice A
merges: tasks B3, B6. Files: `scripts/operator/pipeline.py`,
`scripts/operator/pipeline_tooling/{checks,corpus,results}.py`, new
`scripts/operator/pipeline_tooling/promote.py`,
`scripts/operator/test_pipeline_tooling.py`,
`src/versioned_pipeline_qualification.rs`,
`bin/trace_commons_ingest_internal/pipeline_corpus_pg_tests.rs` and
`pipeline_restore_pg_tests.rs` (harness assembly switch, remote store seam),
`crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/pin-network.json`
(added only after the owner's `record` run), `docs/operator/pipeline-qualification.md`,
`docs/operator/pipeline-lab.md`.

### B1. The remote-restore discharge rule in Rust

1. Test: `remote_restore_discharges_only_the_local_restore_blocker` (spec 5.2
   item 1). Fails: today the blocker always blocks.
2. Implement in `evaluate_promotion` with a named constant pair
   (`LOCAL_RESTORE_BLOCKER_LABEL`, `REMOTE_RESTORE_CHECK_ID`).
3. Commit: "Let a passing remote restore discharge the local restore blocker".

### B2. `promote` scaffolding and refusals

1. Tests: `test_promote_refuses_in_ci`, `test_promote_requires_near_ai_build`,
   `test_promote_sign_signs_exactly_seven`,
   `test_promote_adapters_refuses_another_run_or_revision`,
   `test_promote_assemble_builds_22_and_refuses_doubles`.
2. Implement the `promote` subparser (`init`, `package-checks`,
   `remote-restore`, `hf-canary`, `adapters`, `sign`, `assemble`) in
   `promote.py` (spec B-D1; `adapters` collects the result the deployed boot
   wrote, it never starts ingest); reuse `attest_results`
   and the staging/publish logic (`pipeline.py:919-1018`).
3. Commit: "Add the operator promote command".

### B3. Harness production assembly switch

1. Tests: `harness_production_assembly_requires_the_feature`,
   `production_corpus_mode_compares_only_deterministic_fields` (spec section
   2: consent, privacy, replay, changed-content refusal and tenant isolation
   compared; scoring and settlement states recorded as evidence only).
2. Read `TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY` in `CorpusAssembler`,
   `pipeline_bundle_qualification`'s harness and the restore seed/resume; under
   `production` and `near-ai-scorer`, build through Slice A's assembler from
   the operator's env file. Refuse otherwise.
3. Commit: "Let the qualification harness run on the production assembly".

### B4. HF network pin and canary

1. Tests (offline, injected downloader): `test_hf_pin_network_has_no_local_dir`,
   `test_hf_pin_record_writes_every_digest`, `test_hf_canary_evidence_shape`,
   `test_hf_canary_fails_on_digest_mismatch`.
2. Implement `pipeline.py hf-pin record` and `promote hf-canary` (spec B-D3);
   cache dir inside the run directory.
3. Commit: "Add the HF network pin recorder and canary". The pin file itself
   lands in a follow-up commit or PR after the owner runs `record` and pastes
   its output.

### B5. Remote restore drill

1. Tests (offline, injected object-store client):
   `test_remote_restore_evidence_shape`,
   `test_remote_restore_evidence_rejects_url_like_values`,
   `test_remote_restore_fails_on_fingerprint_mismatch`,
   `test_remote_restore_refuses_the_live_prefix_as_target`.
2. Implement `promote remote-restore` (spec B-D2). The object-store client is
   the configured `ConfiguredTraceArtifactStore`, driven through a small Rust
   harness test (`pipeline_remote_restore_run`, `#[ignore]`, run only by
   `promote`), so the Python side never holds bucket credentials.
3. Commit: "Add the remote restore drill".

### B6. Offline production package build and docs

1. Test: `production_package_build_needs_no_network` (Rust, under
   `near-ai-scorer`): `pipeline_package_write` with `--bundle production`
   from a fixture env file produces a package that passes
   `validate_production_package`.
2. Implement; update `pipeline-qualification.md` ("What local evidence is
   not", the 22-check paragraph) and `pipeline-lab.md` (the HF pin paragraph).
3. Commit: "Build the production package offline and document promote".

CI: B1 and B6 in `cargo test (default features)` / `cargo check
(near-ai-scorer)`; B2, B4, B5 tooling tests in `pipeline qualification and
restore`. No CI step calls the network.

---

## Slice C: gate-decision rows

Branch `pipeline-gate-decision-rows`. Files:
`migrations/V116__pipeline_gate_decision_rows.sql` (confirm V116 is still free
with `git ls-tree origin/main migrations/` at branch time; renumber if not),
`src/db/postgres.rs` (migration registration), `src/versioned_pipeline.rs`
(`commit_settle`, withdrawal/revocation/retention follow-ups),
`src/db/postgres/pipeline_upgrade_tests.rs`,
`tests/versioned_pipeline_runtime_pg.rs`, `.github/workflows/ci.yml`
(selector lists), `docs/operator/pipeline-activation.md` (remove the "known
difference").

### C0. Answer the open questions first

Read and record in the PR body, before any code: O-C3 (credit-quality sweep and
any sweep that writes a credit event), O-C4 (runtime-role INSERT on a fresh
database), O-C5 (legacy withdrawal route for pipeline-owned submissions),
O-C6 (pipeline submission status). If O-C3 finds a sweep that writes credit
from a row, stop and report: the slice needs an owner decision.

### C1. V116

1. Test: `v116_adds_source_and_pipeline_run_id_with_defaults`. Fails: no
   migration.
2. Write the migration (spec C-D4), register it, and run the full
   `pipeline_upgrade_tests` on a fresh DB.
3. Commit: "Mark pipeline-written gate decision rows".

### C2. Settle writes the row

1. Tests: `settle_writes_one_gate_decision_row_per_submission`,
   `stale_settle_writes_no_gate_decision_row`,
   `suspended_settle_policy_writes_no_gate_decision_row`,
   `rejected_run_writes_no_gate_decision_row`,
   `pipeline_gate_decision_insert_works_under_the_runtime_role`.
2. Implement in `commit_settle` (spec C-D1 to C-D3). If the runtime-role test
   fails, add the grant to V116 (spec C-D5) rather than widening anything else.
3. Commit: "Write a gate decision row from Settle".

### C3. Withdrawal and revocation

1. Tests: `withdrawal_clears_pipeline_row_dedup_columns` (and the drain
   report's `withdrawal_completion_pending` reads zero),
   `revocation_clears_pipeline_row_dedup_columns`,
   `withdrawal_leaves_credit_withheld_reason_as_legacy_does`.
2. Implement per spec C-D6, or, if O-C5 shows the legacy route already clears
   the row, keep only the tests.
3. Commit: "Clear pipeline gate decision rows on withdrawal".

### C4. Consumers and regressions

1. Tests: `consumers_see_pipeline_rows`,
   `credit_quality_sweep_does_not_overwrite_pipeline_rows`,
   `legacy_gate_driver_still_skips_pipeline_submissions`.
2. Only if the credit-quality test fails: exclude `source = 'pipeline_settle'`
   in `list_gate_decisions_for_credit_scoring`, the single consumer change the
   spec allows.
3. Add every new `#[ignore]` pg test to the `postgres-suites` job's selector
   lists in `ci.yml` (`ci.yml:78`); an unlisted ignored test never runs.
4. Commit: "Prove the gate decision consumers see pipeline rows".
