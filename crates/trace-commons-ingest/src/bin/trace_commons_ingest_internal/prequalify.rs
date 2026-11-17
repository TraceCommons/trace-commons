// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `trace-commons-ingest --prequalify` (spec
//! `docs/superpowers/specs/2026-11-04-qualify-before-install-design.md`, #1907):
//! a boot of a new binary beside the live one that records the new revision's
//! qualifications before the install, so the new process starts with every
//! routed tenant already qualified.
//!
//! The mode builds the same `AppState` from the live environment file, on
//! three scratch vector-index roots of its own ([`ScratchIndexRoots`]), and
//! runs the ordinary startup up to `finish_ingest_startup` (which writes
//! `pipeline_production_adapters` when the check variables are set). It then
//! returns from `run_ingest_mode` before `serve_ingest`, where every
//! background task is spawned, so it never runs a worker, a loop, a
//! scheduler, a payout pass or the shutdown flush. With `--results-dir`, it
//! first qualifies, on its own revision, every routed tenant whose active
//! bundle is a given set's bundle ([`prequalify_tenants`]).

use super::*;

use std::path::PathBuf;

use super::pipeline_activation::{
    PIPELINE_CHANGE_COMMITTED_AUDIT_FAILED_LABEL, is_refusal_label, qualify_for_tenant,
};
use super::pipeline_default_routing::{Arming, load_arming};
use trace_commons_server::versioned_pipeline_production::gate_env::{
    TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT, TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT,
    TRACE_COMMONS_VECTOR_INDEX_ROOT, legacy_index_roots,
};

/// The flag that selects the mode. The one argument that still reads the
/// pipeline runtime selection (`run_ingest_main`).
pub(crate) const PREQUALIFY_FLAG: &str = "--prequalify";
/// Names one signed results set to qualify with; repeatable (decision D6).
pub(crate) const PREQUALIFY_RESULTS_DIR_FLAG: &str = "--results-dir";
/// What the mode binds instead of `TRACE_COMMONS_BIND` (decision D8): a
/// loopback port the kernel picks, so it never meets the live listener.
pub(crate) const PREQUALIFY_BIND: &str = "127.0.0.1:0";

/// The label the mode's audit actor is derived from (decision D2; the pin
/// test recomputes `PREQUALIFY_ACTOR_REF` from it).
#[cfg(test)]
pub(crate) const PREQUALIFY_ACTOR_LABEL: &str = "trace_commons.system.pipeline_prequalify";
/// `principal_storage_ref(PREQUALIFY_ACTOR_LABEL)`: the actor of every
/// qualification the mode records, pinned by a test.
pub(crate) const PREQUALIFY_ACTOR_REF: &str =
    "principal_sha256:a4d8219bd22f530b6eb5ff3399ee88d18e01b00a2eb4563ee897ed076e3c8388";

// Refusals of the mode, each a safe label.
pub(crate) const PIPELINE_PREQUALIFY_REQUIRES_PRODUCTION_RUNTIME_LABEL: &str =
    "pipeline_prequalify_requires_production_runtime";
pub(crate) const PIPELINE_PREQUALIFY_ARGUMENT_INVALID_LABEL: &str =
    "pipeline_prequalify_argument_invalid";
pub(crate) const PIPELINE_PREQUALIFY_SCRATCH_ROOTS_FAILED_LABEL: &str =
    "pipeline_prequalify_scratch_roots_failed";
pub(crate) const PIPELINE_PREQUALIFY_ENUMERATION_FAILED_LABEL: &str =
    "pipeline_prequalify_enumeration_failed";
pub(crate) const PIPELINE_PREQUALIFY_INCOMPLETE_LABEL: &str = "pipeline_prequalify_incomplete";
/// A tenant whose routing could not be read: counted as refused.
pub(crate) const PIPELINE_PREQUALIFY_ROUTING_READ_FAILED_LABEL: &str =
    "pipeline_prequalify_routing_read_failed";

/// The page of routed tenants the mode reads at a time: the largest page
/// the V124 enumeration answers.
const PREQUALIFY_PAGE: i64 = 1000;

/// What `run_ingest` does after its startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum IngestMode {
    /// Serve: spawn the background tasks and run the app.
    Serve,
    /// `--prequalify`: stop after the startup, qualify, exit.
    Prequalify(PrequalifyArgs),
}

/// The arguments after `--prequalify`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PrequalifyArgs {
    /// Each `--results-dir`, in order. Empty: only the startup runs (the
    /// adapters check, when its variables are set).
    pub(crate) results_dirs: Vec<PathBuf>,
}

impl PrequalifyArgs {
    /// Parses the arguments that follow `--prequalify`. Anything but
    /// `--results-dir <dir>` refuses, as an unknown flag does for a plain
    /// boot.
    pub(crate) fn parse(args: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut parsed = Self::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            if arg != PREQUALIFY_RESULTS_DIR_FLAG {
                anyhow::bail!(PIPELINE_PREQUALIFY_ARGUMENT_INVALID_LABEL);
            }
            match args.next() {
                Some(dir) if !dir.trim().is_empty() && !dir.starts_with("--") => {
                    parsed.results_dirs.push(PathBuf::from(dir));
                }
                _ => anyhow::bail!(PIPELINE_PREQUALIFY_ARGUMENT_INVALID_LABEL),
            }
        }
        Ok(parsed)
    }
}

/// Whether a boot whose first argument is `first_arg` reads the pipeline
/// runtime selection: a plain boot and the mode do; every other argument
/// (help, version, keypair, TLS self-check) reads no configuration.
pub(crate) fn reads_runtime_selection(first_arg: Option<&str>) -> bool {
    matches!(first_arg, None | Some(PREQUALIFY_FLAG))
}

/// Three empty vector-index roots under one temporary directory (mode
/// 0700), removed when this is dropped. The mode opens these instead of the
/// live roots: opening an index can complete logged removals and compact its
/// manifest in place, and dropping one saves it, so the live roots must
/// never be opened by a second process.
pub(crate) struct ScratchIndexRoots {
    dir: tempfile::TempDir,
}

impl ScratchIndexRoots {
    pub(crate) fn create() -> anyhow::Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("trace-commons-prequalify-")
            .tempdir()
            .map_err(|_| anyhow::anyhow!(PIPELINE_PREQUALIFY_SCRATCH_ROOTS_FAILED_LABEL))?;
        for name in ["novelty", "dedup", "pipeline"] {
            std::fs::create_dir(dir.path().join(name))
                .map_err(|_| anyhow::anyhow!(PIPELINE_PREQUALIFY_SCRATCH_ROOTS_FAILED_LABEL))?;
        }
        Ok(Self { dir })
    }

    #[cfg(test)]
    pub(crate) fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    /// The scratch value of one of the three root variables; `None` for any
    /// other variable.
    fn var(&self, name: &str) -> Option<String> {
        let leaf = match name {
            TRACE_COMMONS_VECTOR_INDEX_ROOT => "novelty",
            TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT => "dedup",
            TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT => "pipeline",
            _ => return None,
        };
        Some(self.dir.path().join(leaf).to_string_lossy().into_owned())
    }
}

/// How this process resolves the vector-index root variables: from the
/// environment, or, for the mode, from its scratch roots. Every other
/// variable reads through unchanged.
#[derive(Default)]
pub(crate) struct IndexRoots<'a> {
    scratch: Option<&'a ScratchIndexRoots>,
}

impl<'a> IndexRoots<'a> {
    /// The live roots, from the environment.
    pub(crate) fn environment() -> Self {
        Self { scratch: None }
    }

    /// The mode's roots.
    pub(crate) fn scratch(scratch: &'a ScratchIndexRoots) -> Self {
        Self {
            scratch: Some(scratch),
        }
    }

    /// `name` through the scratch roots, else through `base`.
    pub(crate) fn var_over(
        &self,
        name: &str,
        base: &(dyn Fn(&str) -> Option<String> + Sync),
    ) -> Option<String> {
        self.scratch
            .and_then(|scratch| scratch.var(name))
            .or_else(|| base(name))
    }

    /// `name` through the scratch roots, else the process environment.
    pub(crate) fn var(&self, name: &str) -> Option<String> {
        self.var_over(name, &|name: &str| std::env::var(name).ok())
    }
}

/// The novelty index root the legacy gate opens, through `lookup`.
#[cfg_attr(not(feature = "near-ai-scorer"), allow(dead_code))]
pub(crate) fn novelty_index_root(lookup: &dyn Fn(&str) -> Option<String>) -> String {
    legacy_index_roots(lookup).0.to_string_lossy().into_owned()
}

/// The dedup index root, through `lookup` (by default a sibling of the
/// novelty root).
#[cfg_attr(
    not(any(feature = "local-gpu-models", feature = "near-ai-scorer")),
    allow(dead_code)
)]
pub(crate) fn dedup_index_root(lookup: &dyn Fn(&str) -> Option<String>) -> String {
    legacy_index_roots(lookup).1.to_string_lossy().into_owned()
}

/// What one run of the mode did, as counts (never a tenant id or a path).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct PrequalifyReport {
    /// Results sets verified.
    pub(crate) sets: u64,
    /// Qualifications recorded on this revision.
    pub(crate) qualified: u64,
    /// Tenants whose active bundle already had a qualification here.
    pub(crate) already_qualified: u64,
    /// Tenants whose active bundle is no given set's bundle.
    pub(crate) bundle_differs: u64,
    /// Routed tenants with no active bundle.
    pub(crate) no_active_bundle: u64,
    /// Qualifications refused (each logged with its label).
    pub(crate) refused: u64,
    /// Qualifications recorded whose audit row failed.
    pub(crate) audit_failed: u64,
}

impl PrequalifyReport {
    /// One line for the operator's terminal.
    pub(crate) fn line(&self) -> String {
        format!(
            "PipelinePrequalify: sets={} qualified={} already_qualified={} bundle_differs={} \
             no_active_bundle={} refused={} audit_failed={}",
            self.sets,
            self.qualified,
            self.already_qualified,
            self.bundle_differs,
            self.no_active_bundle,
            self.refused,
            self.audit_failed
        )
    }

    /// Decision D5: a run that refused a tenant, or recorded one without its
    /// audit row, exits non-zero. A tenant on another bundle is reported, not
    /// failed: the operator decides with a set for that bundle.
    pub(crate) fn ensure_complete(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.refused == 0 && self.audit_failed == 0,
            PIPELINE_PREQUALIFY_INCOMPLETE_LABEL
        );
        Ok(())
    }
}

/// Refuses the mode before anything is built unless the boot assembles the
/// production pipeline: only that path opens the index roots through
/// [`IndexRoots`], and only a production runtime can qualify.
pub(crate) fn require_production_runtime(
    pipeline_runtime_assembler: Option<&dyn IngestPipelineRuntimeAssembler>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        pipeline_runtime_assembler.is_some_and(|assembler| assembler.needs_gate_components()),
        PIPELINE_PREQUALIFY_REQUIRES_PRODUCTION_RUNTIME_LABEL
    );
    Ok(())
}

/// The mode after the startup: qualify with each results set, print the
/// counts, and fail when the run was incomplete. Spawns nothing.
pub(crate) async fn run_prequalify(state: &AppState, args: &PrequalifyArgs) -> anyhow::Result<()> {
    let report = prequalify_tenants(state, &args.results_dirs, &|_| true).await?;
    tracing::info!(
        sets = report.sets,
        qualified = report.qualified,
        already_qualified = report.already_qualified,
        bundle_differs = report.bundle_differs,
        no_active_bundle = report.no_active_bundle,
        refused = report.refused,
        audit_failed = report.audit_failed,
        "pipeline_prequalify_finished"
    );
    println!("{}", report.line());
    report.ensure_complete()
}

/// Verifies each set in `results_dirs` as arming does (on this process's
/// revision: a set for another revision, a stale set or a package mismatch
/// refuses with the arming label and records nothing), then reads every
/// routed tenant (`pipeline` and `contained`, whoever routed it) for which
/// `may_act_on` holds, and qualifies each whose active bundle is a set's
/// bundle and has no qualification on this revision, under
/// `PREQUALIFY_ACTOR_REF`. A qualification changes no routing row.
pub(crate) async fn prequalify_tenants(
    state: &AppState,
    results_dirs: &[PathBuf],
    may_act_on: &(dyn Fn(&str) -> bool + Sync),
) -> anyhow::Result<PrequalifyReport> {
    let mut report = PrequalifyReport::default();
    if results_dirs.is_empty() {
        return Ok(report);
    }
    let activation = state
        .pipeline_activation
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!(PIPELINE_PREQUALIFY_REQUIRES_PRODUCTION_RUNTIME_LABEL))?;
    let revision = state
        .pipeline_code_revision_hash
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!(PIPELINE_PREQUALIFY_REQUIRES_PRODUCTION_RUNTIME_LABEL))?;
    let now = Utc::now();
    let mut sets = Vec::with_capacity(results_dirs.len());
    for dir in results_dirs {
        match load_arming(state, dir, now).await {
            Arming::Armed(set) => sets.push(set),
            Arming::Disarmed { label } => anyhow::bail!(label),
        }
    }
    report.sets = sets.len() as u64;

    let mut after: Option<String> = None;
    loop {
        let page = activation
            .routed_tenants(after.as_deref(), PREQUALIFY_PAGE)
            .await
            .map_err(|_| anyhow::anyhow!(PIPELINE_PREQUALIFY_ENUMERATION_FAILED_LABEL))?;
        let full = page.len() as i64 >= PREQUALIFY_PAGE;
        after = page.last().map(|(tenant_id, _)| tenant_id.clone());
        for (tenant_id, _) in page.iter().filter(|(tenant_id, _)| may_act_on(tenant_id)) {
            let Ok(view) = activation.routing_view(tenant_id, 1, Some(revision)).await else {
                report.refused += 1;
                tracing::warn!(
                    tenant_storage_ref = %tenant_storage_ref(tenant_id),
                    label = PIPELINE_PREQUALIFY_ROUTING_READ_FAILED_LABEL,
                    "pipeline_prequalify_tenant_refused"
                );
                continue;
            };
            match view.active_bundle_qualified_on_revision {
                Some(true) => {
                    report.already_qualified += 1;
                    continue;
                }
                None => {
                    report.no_active_bundle += 1;
                    continue;
                }
                Some(false) => {}
            }
            let Some(set) = sets
                .iter()
                .find(|set| view.active_bundle_id.as_deref() == Some(set.bundle_id.as_str()))
            else {
                report.bundle_differs += 1;
                tracing::warn!(
                    tenant_storage_ref = %tenant_storage_ref(tenant_id),
                    "pipeline_prequalify_bundle_differs"
                );
                continue;
            };
            let actor = system_audit_tenant(tenant_id, PREQUALIFY_ACTOR_REF);
            match qualify_for_tenant(state, &actor, &set.signed_package, &set.attestations).await {
                Ok(_) => report.qualified += 1,
                Err((_, error))
                    if error.0.error == PIPELINE_CHANGE_COMMITTED_AUDIT_FAILED_LABEL =>
                {
                    report.audit_failed += 1;
                    tracing::warn!(
                        tenant_storage_ref = %tenant_storage_ref(tenant_id),
                        label = PIPELINE_CHANGE_COMMITTED_AUDIT_FAILED_LABEL,
                        "pipeline_prequalify_tenant_refused"
                    );
                }
                Err((_, error)) => {
                    report.refused += 1;
                    let label = if is_refusal_label(&error.0.error) {
                        error.0.error
                    } else {
                        PIPELINE_PREQUALIFY_INCOMPLETE_LABEL.to_string()
                    };
                    tracing::warn!(
                        tenant_storage_ref = %tenant_storage_ref(tenant_id),
                        label = %label,
                        "pipeline_prequalify_tenant_refused"
                    );
                }
            }
        }
        if !full {
            break;
        }
    }
    Ok(report)
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    /// T6: the mode reads the pipeline runtime selection, as a plain boot
    /// does; every other first argument still reads none.
    #[test]
    fn only_a_plain_boot_and_the_mode_read_the_runtime_selection() {
        assert!(reads_runtime_selection(None));
        assert!(reads_runtime_selection(Some(PREQUALIFY_FLAG)));
        for other in [
            "--help",
            "-h",
            "--version",
            "-V",
            "--tls-selfcheck",
            "--generate-attestation-keypair",
            "--prequalify=1",
            "--results-dir",
        ] {
            assert!(!reads_runtime_selection(Some(other)), "{other}");
        }
    }

    #[test]
    fn the_results_dir_flag_is_repeatable_and_anything_else_refuses() {
        assert_eq!(
            PrequalifyArgs::parse(args(&[])).unwrap(),
            PrequalifyArgs::default()
        );
        assert_eq!(
            PrequalifyArgs::parse(args(&["--results-dir", "/a", "--results-dir", "/b"]))
                .unwrap()
                .results_dirs,
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
        for bad in [
            args(&["--results-dir"]),
            args(&["--results-dir", ""]),
            args(&["--results-dir", "--results-dir"]),
            args(&["/a"]),
            args(&["--serve"]),
        ] {
            assert_eq!(
                PrequalifyArgs::parse(bad.clone()).unwrap_err().to_string(),
                PIPELINE_PREQUALIFY_ARGUMENT_INVALID_LABEL,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_actor_ref_is_the_principal_ref_of_its_label() {
        assert_eq!(
            PREQUALIFY_ACTOR_REF,
            principal_storage_ref(PREQUALIFY_ACTOR_LABEL)
        );
        assert_ne!(
            PREQUALIFY_ACTOR_REF,
            super::super::pipeline_default_routing::DEFAULT_ROUTING_ACTOR_REF
        );
    }

    #[test]
    fn a_report_with_a_refusal_or_a_missing_audit_row_is_incomplete() {
        assert!(PrequalifyReport::default().ensure_complete().is_ok());
        let differs = PrequalifyReport {
            bundle_differs: 2,
            ..Default::default()
        };
        assert!(differs.ensure_complete().is_ok());
        for report in [
            PrequalifyReport {
                refused: 1,
                ..Default::default()
            },
            PrequalifyReport {
                audit_failed: 1,
                ..Default::default()
            },
        ] {
            assert_eq!(
                report.ensure_complete().unwrap_err().to_string(),
                PIPELINE_PREQUALIFY_INCOMPLETE_LABEL
            );
        }
        assert!(!PrequalifyReport::default().line().contains('/'));
    }

    /// T2, the resolution half: with the mode's roots, each of the three
    /// root variables resolves inside the scratch directory and never to the
    /// configured value; every other variable reads through.
    #[test]
    fn the_mode_resolves_every_index_root_inside_its_scratch_directory() {
        let scratch = ScratchIndexRoots::create().unwrap();
        let live = |name: &str| match name {
            TRACE_COMMONS_VECTOR_INDEX_ROOT => Some("/live/novelty".to_string()),
            TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT => Some("/live/dedup".to_string()),
            TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT => Some("/live/pipeline".to_string()),
            "OTHER" => Some("kept".to_string()),
            _ => None,
        };
        let roots = IndexRoots::scratch(&scratch);
        let mut resolved = Vec::new();
        for name in [
            TRACE_COMMONS_VECTOR_INDEX_ROOT,
            TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT,
            TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT,
        ] {
            let value = PathBuf::from(roots.var_over(name, &live).unwrap());
            assert!(value.starts_with(scratch.path()), "{name}");
            assert!(value.is_dir(), "{name}");
            assert!(!value.starts_with("/live"), "{name}");
            resolved.push(value);
        }
        resolved.sort();
        resolved.dedup();
        assert_eq!(resolved.len(), 3, "three separate roots");
        assert_eq!(roots.var_over("OTHER", &live).as_deref(), Some("kept"));
        assert_eq!(
            IndexRoots::environment()
                .var_over(TRACE_COMMONS_VECTOR_INDEX_ROOT, &live)
                .as_deref(),
            Some("/live/novelty")
        );
        let path = scratch.path().to_path_buf();
        drop(scratch);
        assert!(!path.exists(), "the scratch roots are removed");
    }

    /// Every file under `dir`, by path, with the digest of its bytes.
    #[cfg(feature = "near-ai-scorer")]
    fn snapshot(dir: &std::path::Path) -> BTreeMap<PathBuf, String> {
        let mut files = BTreeMap::new();
        let mut pending = vec![dir.to_path_buf()];
        while let Some(next) = pending.pop() {
            for entry in std::fs::read_dir(&next).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    files.insert(
                        path.clone(),
                        format!("{:x}", Sha256::digest(std::fs::read(&path).unwrap())),
                    );
                }
            }
        }
        files
    }

    /// T2: the configured (live) roots hold a pipeline index whose manifest
    /// log is not compacted (an open rewrites it) and a novelty and a dedup
    /// index. Resolved the way the startup resolves them under the mode,
    /// every index the startup opens is opened, written and dropped (a
    /// drop-time flush), and the live roots stay byte-identical. The same
    /// open through the environment's roots rewrites the live manifest, so
    /// the comparison would see it.
    #[cfg(feature = "near-ai-scorer")]
    #[test]
    fn the_mode_never_writes_under_the_configured_index_roots() {
        use trace_commons_gate_api::pipeline::TenantStorageRef;
        use trace_commons_gate_api::{IndexEntryKey, VectorIndex, VectorIndexWriter};
        use trace_commons_gate_enclave::vector_index_usearch::{
            UsearchVectorIndex, UsearchVectorIndexConfig,
        };
        use trace_commons_server::versioned_pipeline_production::gate_env::pipeline_index_root_from;
        use trace_commons_server::versioned_pipeline_production::{
            PRODUCTION_COMPATIBILITY_INDEX_ID, PRODUCTION_COMPATIBILITY_PROJECTION_ID,
            UsearchPipelineIndex,
        };

        let config = || UsearchVectorIndexConfig {
            dim: 4,
            hnsw_m: 16,
            ef_construction: 64,
            ef_search: 64,
            max_open: 8,
            flush_every: 1_000,
            flush_interval: None,
        };
        let tenant_ref =
            TenantStorageRef::new(format!("tenant_sha256:{}", "ab".repeat(16))).unwrap();
        let key = |revision: u128| IndexEntryKey {
            tenant_storage_ref: tenant_ref.clone(),
            index_id: PRODUCTION_COMPATIBILITY_INDEX_ID.to_string(),
            revision_id: Uuid::from_u128(revision),
            projection_id: PRODUCTION_COMPATIBILITY_PROJECTION_ID.to_string(),
            model_id: "BAAI/bge-large-en-v1.5".to_string(),
            chunk: 0,
        };
        let unit = |axis: usize| {
            let mut vector = vec![0.0_f32; 4];
            vector[axis] = 1.0;
            vector
        };

        let live = tempfile::tempdir().unwrap();
        let live_novelty = live.path().join("novelty");
        let live_dedup = live.path().join("dedup");
        let live_pipeline = live.path().join("pipeline");
        for dir in [&live_novelty, &live_dedup, &live_pipeline] {
            std::fs::create_dir(dir).unwrap();
        }
        {
            let index = UsearchPipelineIndex::open(&live_pipeline, config()).unwrap();
            index.upsert(&key(1), &unit(0), "sha256:one").unwrap();
            index.upsert(&key(2), &unit(1), "sha256:two").unwrap();
            index
                .invalidate_revision(
                    &tenant_ref,
                    PRODUCTION_COMPATIBILITY_INDEX_ID,
                    Uuid::from_u128(1),
                )
                .unwrap();
        }
        for root in [&live_novelty, &live_dedup] {
            let index = UsearchVectorIndex::try_new(root, config()).unwrap();
            index
                .insert(Uuid::new_v4(), tenant_ref.as_str(), &unit(2))
                .unwrap();
            index.flush().unwrap();
        }
        let before = snapshot(live.path());
        assert!(!before.is_empty());

        let base = {
            let (novelty, dedup, pipeline) = (
                live_novelty.to_string_lossy().into_owned(),
                live_dedup.to_string_lossy().into_owned(),
                live_pipeline.to_string_lossy().into_owned(),
            );
            move |name: &str| match name {
                TRACE_COMMONS_VECTOR_INDEX_ROOT => Some(novelty.clone()),
                TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT => Some(dedup.clone()),
                TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT => Some(pipeline.clone()),
                _ => None,
            }
        };
        let scratch = ScratchIndexRoots::create().unwrap();
        let roots = IndexRoots::scratch(&scratch);
        let lookup = |name: &str| roots.var_over(name, &base);
        {
            let pipeline =
                UsearchPipelineIndex::open(&pipeline_index_root_from(&lookup).unwrap(), config())
                    .unwrap();
            pipeline.upsert(&key(3), &unit(3), "sha256:three").unwrap();
        }
        for root in [novelty_index_root(&lookup), dedup_index_root(&lookup)] {
            let index = UsearchVectorIndex::try_new(&root, config()).unwrap();
            index
                .insert(Uuid::new_v4(), tenant_ref.as_str(), &unit(3))
                .unwrap();
            drop(index);
        }
        assert_eq!(
            snapshot(live.path()),
            before,
            "the live roots are byte-identical"
        );
        assert!(
            !snapshot(scratch.path()).is_empty(),
            "the indexes were opened and written, in the scratch roots"
        );

        let environment = IndexRoots::environment();
        let live_lookup = |name: &str| environment.var_over(name, &base);
        drop(
            UsearchPipelineIndex::open(&pipeline_index_root_from(&live_lookup).unwrap(), config())
                .unwrap(),
        );
        assert_ne!(
            snapshot(live.path()),
            before,
            "an open of the live roots rewrites the manifest, and the comparison sees it"
        );
    }
}
