// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `trace-commons-ingest --prequalify` (spec 2026-11-04, #1907): a process
//! of the new revision records its qualifications beside the running one,
//! and returns before any background work.
//!
//! Every test runs over a real PostgreSQL as the route suite's runtime login
//! (`RouteFixture`), on the state of a "new build": the fixture's state with
//! another code revision, as the staged binary would hold it. Every walk of
//! the routed tenants is restricted to the test's own tenants, because the
//! suite's database is shared.

use super::*;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::pipeline_activation_pg_tests::{ROUTE_CHECK_KEY_ID, RouteFixture};
use super::pipeline_http_pg_tests::write_routing_as_operator;
use crate::ingest::app::pipeline_default_routing::{
    Arming, DEFAULT_ROUTING_ATTESTATION_SUFFIX, DEFAULT_ROUTING_PACKAGE_FILE,
    PIPELINE_DEFAULT_ROUTING_PACKAGE_MISMATCH_LABEL, PIPELINE_DEFAULT_ROUTING_RESULTS_STALE_LABEL,
    PIPELINE_DEFAULT_ROUTING_REVISION_MISMATCH_LABEL, PipelineDefaultRouting,
    PipelineDefaultRoutingConfig, load_arming,
};
use crate::ingest::app::prequalify::{
    IngestMode, PREQUALIFY_ACTOR_REF, PREQUALIFY_BIND, PrequalifyArgs, PrequalifyReport,
    prequalify_tenants,
};
use trace_commons_gate_api::pipeline::BundlePackage;
use trace_commons_server::versioned_pipeline_qualification::{
    PipelineCheckAttestation, ProductionDependencyProfile,
};

/// The route fixture with both of its tenants activated by hand on A, on the
/// fixture's revision (the running build), and a directory for result sets.
struct Prequalify {
    route: RouteFixture,
    /// The new build's revision.
    deployed: String,
    sets: tempfile::TempDir,
}

impl Prequalify {
    async fn new() -> Option<Self> {
        let route = RouteFixture::new().await?;
        route.qualify(&route.a).await;
        let (status, body) = route
            .admin_call(
                "POST",
                "/v1/admin/pipeline/activate",
                Some(route.activate_body(&route.a, "operator_activates").await),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = route
            .call(
                &route.state,
                "POST",
                "/v1/admin/pipeline/qualifications",
                Some(&route.other_admin),
                Some(route.qualify_body(&route.a)),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let in_force = route.record_id_in_force(&route.other_tenant).await;
        let (status, body) = route
            .call(
                &route.state,
                "POST",
                "/v1/admin/pipeline/activate",
                Some(&route.other_admin),
                Some(route.activate_body_expecting(&route.a, "operator_activates", &in_force)),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let deployed = sha256_prefixed(&format!("prequalify-deployed-{}", Uuid::new_v4()));
        Some(Self {
            route,
            deployed,
            sets: tempfile::tempdir().expect("sets dir"),
        })
    }

    /// The staged binary's state: the fixture's, on the new revision.
    fn new_build(&self, change: impl FnOnce(&mut AppState)) -> Arc<AppState> {
        let deployed = self.deployed.clone();
        self.route.with(|state| {
            state.pipeline_code_revision_hash = Some(deployed);
            change(state);
        })
    }

    /// Writes `package` signed and `attestations` into `<sets>/<name>`.
    fn write_set(
        &self,
        name: &str,
        package: &BundlePackage,
        attestations: &[PipelineCheckAttestation],
    ) -> PathBuf {
        let dir = self.sets.path().join(name);
        write_set_into(&dir, &self.route.signed(package), attestations);
        dir
    }

    /// A on the new revision: what the operator assembles for the new build.
    fn a_on_new_build(&self) -> PathBuf {
        let attestations = self.route.attestations_on(&self.route.a, &self.deployed);
        self.write_set("a-new", &self.route.a, &attestations)
    }

    fn ours(&self, extra: &[&str]) -> Box<dyn Fn(&str) -> bool + Sync> {
        let mut tenants =
            BTreeSet::from([self.route.tenant.clone(), self.route.other_tenant.clone()]);
        tenants.extend(extra.iter().map(|tenant| tenant.to_string()));
        Box::new(move |tenant: &str| tenants.contains(tenant))
    }

    async fn qualified_on(&self, tenant: &str, bundle: &BundlePackage, revision: &str) -> bool {
        PipelineQualificationStore::new(self.route.runtime.clone())
            .qualification(tenant, &bundle.bundle_id, revision)
            .await
            .expect("the qualification reads")
            .is_some()
    }

    async fn prequalify(
        &self,
        state: &AppState,
        dirs: &[PathBuf],
        extra: &[&str],
    ) -> anyhow::Result<PrequalifyReport> {
        prequalify_tenants(state, dirs, &*self.ours(extra)).await
    }
}

fn write_set_into(
    dir: &Path,
    signed: &trace_commons_server::versioned_pipeline_qualification::SignedBundlePackage,
    attestations: &[PipelineCheckAttestation],
) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join(DEFAULT_ROUTING_PACKAGE_FILE),
        serde_json::to_vec(signed).unwrap(),
    )
    .unwrap();
    for attestation in attestations {
        std::fs::write(
            dir.join(format!(
                "{}{DEFAULT_ROUTING_ATTESTATION_SUFFIX}",
                attestation.result.check_id
            )),
            serde_json::to_vec(attestation).unwrap(),
        )
        .unwrap();
    }
}

/// A gate that counts `flush_vector_index` calls and is otherwise the
/// wrapped one: the probe for the shutdown flush.
struct FlushCountingGate {
    inner: Arc<dyn TraceGateService>,
    flushes: Arc<std::sync::atomic::AtomicUsize>,
}

impl TraceGateService for FlushCountingGate {
    fn evaluate_trace(
        &self,
        tenant_ctx: &trace_commons_server::trace_gate_service::TenantCtx,
        envelope_ciphertext: &[u8],
        wrapped_dek: &WrappedDek,
        object_kind: TraceArtifactKind,
    ) -> anyhow::Result<GateDecision> {
        self.inner
            .evaluate_trace(tenant_ctx, envelope_ciphertext, wrapped_dek, object_kind)
    }

    fn invalidate_vector_entry(
        &self,
        tenant_ctx: &trace_commons_server::trace_gate_service::TenantCtx,
        vector_entry_id: Uuid,
    ) -> anyhow::Result<()> {
        self.inner
            .invalidate_vector_entry(tenant_ctx, vector_entry_id)
    }

    fn safe_status(&self) -> GateServiceStatus {
        self.inner.safe_status()
    }

    fn flush_vector_index(&self) -> anyhow::Result<()> {
        self.flushes
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.inner.flush_vector_index()
    }
}

/// `run_ingest_mode` in the mode, with no results set, on `state`, within a
/// minute: a process that reached `serve_ingest` would serve until stopped.
async fn run_mode(state: &Arc<AppState>, deployed: &str) -> anyhow::Result<()> {
    tokio::time::timeout(
        StdDuration::from_secs(60),
        run_ingest_mode(
            IngestMode::Prequalify(PrequalifyArgs::default()),
            state.clone(),
            None,
            PREQUALIFY_BIND,
            production_assembly::PipelineCheckVars::default(),
            Some(deployed),
        ),
    )
    .await
    .expect("the mode returns: it never reaches serve_ingest")
}

fn alive_tasks() -> usize {
    tokio::runtime::Handle::current()
        .metrics()
        .num_alive_tasks()
}

/// T1: the mode, through the same `run_ingest_mode` a boot runs, returns
/// after its startup and its qualifications, and starts nothing: no task
/// outlives it (a scheduler, driver, payout worker, pipeline worker or the
/// default-routing loop would), no default bundle is registered for a
/// receipts-list tenant that has none, default routing neither arms nor
/// routes the unrouted tenant it may act on, and the shutdown flush never
/// runs. A process that reached `serve_ingest` would not return at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_mode_returns_before_any_background_work() {
    let Some(fixture) = Prequalify::new().await else {
        return;
    };
    let suffix = Uuid::new_v4().simple().to_string();
    // A receipts-list tenant with no bundle yet: serving registers its
    // default bundle before the worker starts.
    let unregistered = format!("tenant-prequalify-unregistered-{suffix}");
    // A tenant that signed up and has no routing row: an armed loop routes
    // it on its first pass.
    let unrouted = format!("tenant-prequalify-unrouted-{suffix}");
    let mut client = fixture
        .route
        .owner
        .trace_pool_for_test()
        .get()
        .await
        .unwrap();
    let tx = super::pipeline_http_pg_tests::tenant_tx(&mut client, &unrouted).await;
    tx.execute(
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
        &[&unrouted],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    drop(client);

    let set = fixture.a_on_new_build();
    // The same set where default routing would arm from it.
    let routing_dir = fixture.sets.path().join("routing");
    write_set_into(
        &routing_dir,
        &fixture.route.signed(&fixture.route.a),
        &fixture
            .route
            .attestations_on(&fixture.route.a, &fixture.deployed),
    );
    let flushes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let routing = Arc::new(PipelineDefaultRouting::new(PipelineDefaultRoutingConfig {
        results_dir: routing_dir,
        interval: StdDuration::from_secs(60),
        batch: 1000,
    }));
    routing.restrict_loop_for_test(BTreeSet::from([unrouted.clone()]));
    let state = fixture.new_build(|state| {
        state.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
            TraceTenantRolloutFeature::PipelineReceipts,
            &[
                fixture.route.tenant.as_str(),
                fixture.route.other_tenant.as_str(),
                unregistered.as_str(),
            ],
        );
        state.pipeline_default_routing = Some(routing.clone());
        state.gate_service = Arc::new(FlushCountingGate {
            inner: state.gate_service.clone(),
            flushes: flushes.clone(),
        });
    });
    // The mode's walk is restricted the way every test's is;
    // `run_ingest_mode` takes no filter, so it runs here with no results set
    // (the startup alone), and the qualifications are made by the same
    // function it calls.
    // A first run warms the pools, so the second measures only the mode.
    run_mode(&state, &fixture.deployed)
        .await
        .expect("the mode succeeds");
    tokio::time::sleep(StdDuration::from_millis(300)).await;
    let before = alive_tasks();
    run_mode(&state, &fixture.deployed)
        .await
        .expect("the mode succeeds again");
    let report = fixture
        .prequalify(&state, std::slice::from_ref(&set), &[])
        .await
        .expect("the set verifies");
    assert_eq!(report.qualified, 2, "{report:?}");
    tokio::time::sleep(StdDuration::from_millis(500)).await;
    assert!(
        alive_tasks() <= before,
        "no task outlives the mode: {} > {before}",
        alive_tasks()
    );

    let activation = fixture.route.store();
    let view = activation
        .routing_view(&unregistered, 1, Some(&fixture.deployed))
        .await
        .unwrap();
    assert_eq!(view.active_bundle_id, None, "no default bundle registered");
    assert!(
        activation.routing(&unrouted).await.unwrap().is_none(),
        "default routing routed nobody"
    );
    assert!(!routing.is_armed(), "default routing never armed");
    assert_eq!(
        flushes.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "the shutdown flush never ran"
    );

    // The probe sees a task when one is started: the pipeline worker, as
    // `serve_ingest` would start it.
    let worker = pipeline_runtime::spawn_pipeline_worker(state.clone()).expect("a runtime");
    tokio::time::sleep(StdDuration::from_millis(100)).await;
    assert!(alive_tasks() > before, "the probe counts a started task");
    let _ = worker.stop.send(true);
    let _ = worker.join.await;
}

/// T3: qualifications recorded by a process of the new revision are rows of
/// that revision only. The running build keeps its own rows, serves as
/// before, and reads nothing new; the new build serves once they exist. No
/// routing row changes, and the actor is the mode's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_new_revision_is_qualified_beside_the_running_one() {
    let Some(fixture) = Prequalify::new().await else {
        return;
    };
    let route = &fixture.route;
    let new_build = fixture.new_build(|_| {});
    let (status, body, _) = route.upload(&new_build, "before_prequalify").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], "pipeline_bundle_not_qualified");
    let before = [
        route
            .store()
            .routing_view(&route.tenant, 100, None)
            .await
            .unwrap(),
        route
            .store()
            .routing_view(&route.other_tenant, 100, None)
            .await
            .unwrap(),
    ];
    let running_rows = [
        route.qualification(&route.tenant, &route.a).await.unwrap(),
        route
            .qualification(&route.other_tenant, &route.a)
            .await
            .unwrap(),
    ];

    let report = fixture
        .prequalify(&new_build, &[fixture.a_on_new_build()], &[])
        .await
        .expect("the set verifies");
    assert_eq!(
        report,
        PrequalifyReport {
            sets: 1,
            qualified: 2,
            ..Default::default()
        }
    );
    for tenant in [&route.tenant, &route.other_tenant] {
        assert!(
            fixture
                .qualified_on(tenant, &route.a, &fixture.deployed)
                .await
        );
        assert!(
            fixture
                .qualified_on(tenant, &route.a, &route.revision)
                .await
        );
    }
    let after_rows = [
        route.qualification(&route.tenant, &route.a).await.unwrap(),
        route
            .qualification(&route.other_tenant, &route.a)
            .await
            .unwrap(),
    ];
    for (running, after) in running_rows.iter().zip(after_rows.iter()) {
        assert_eq!(
            running.metadata, after.metadata,
            "the running row is untouched"
        );
        assert_eq!(running.qualified_at, after.qualified_at);
    }
    for (tenant, before) in [&route.tenant, &route.other_tenant]
        .iter()
        .zip(before.iter())
    {
        let after = route.store().routing_view(tenant, 100, None).await.unwrap();
        assert_eq!(
            after.routing.as_ref().map(|r| r.activation_record_id),
            before.routing.as_ref().map(|r| r.activation_record_id),
            "the routing row is unchanged"
        );
        assert_eq!(after.events.len(), before.events.len(), "no routing event");
    }
    let running_view = route
        .store()
        .routing_view(&route.tenant, 1, Some(&route.revision))
        .await
        .unwrap();
    assert_eq!(running_view.active_bundle_qualified_on_revision, Some(true));
    let (status, body, _) = route.upload(&route.state, "running_build").await;
    assert_eq!(status, StatusCode::OK, "the running build serves: {body}");
    let (status, body, _) = route.upload(&new_build, "after_prequalify").await;
    assert_eq!(status, StatusCode::OK, "the new build serves: {body}");

    let audited = read_all_audit_events(route.dir.path(), &route.tenant)
        .expect("the audit log reads")
        .into_iter()
        .filter(|event| event.kind == "pipeline_activation")
        .filter(|event| event.actor_principal_ref.as_deref() == Some(PREQUALIFY_ACTOR_REF))
        .count();
    assert_eq!(
        audited, 1,
        "the qualification is audited under the mode's actor"
    );
}

/// T4: the counts. A set for another bundle skips every tenant on A; a
/// contained tenant is qualified; a routed tenant with no active bundle is
/// counted; a re-run is a no-op; a qualification whose audit row fails is
/// counted and makes the run incomplete.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_mode_counts_what_it_did_and_a_rerun_does_nothing() {
    let Some(fixture) = Prequalify::new().await else {
        return;
    };
    let route = &fixture.route;
    let (status, body) = route
        .call(
            &route.state,
            "POST",
            "/v1/admin/pipeline/contain",
            Some(&route.other_admin),
            Some(serde_json::json!({ "reason_code": "contain_for_incident" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let bundleless = format!("tenant-prequalify-bundleless-{}", Uuid::new_v4().simple());
    write_routing_as_operator(&bundleless, "pipeline").await;
    let new_build = fixture.new_build(|_| {});

    let b_only = fixture.write_set(
        "b-new",
        &route.b,
        &route.attestations_on(&route.b, &fixture.deployed),
    );
    let report = fixture
        .prequalify(&new_build, &[b_only], &[&bundleless])
        .await
        .unwrap();
    assert_eq!(
        report,
        PrequalifyReport {
            sets: 1,
            bundle_differs: 2,
            no_active_bundle: 1,
            ..Default::default()
        }
    );
    assert!(
        report.ensure_complete().is_ok(),
        "a different bundle is reported, not failed"
    );

    // The audit of the fixture tenant cannot be written: its log path is a
    // directory for the length of the run.
    let log = route
        .dir
        .path()
        .join("tenants")
        .join(tenant_storage_key(&route.tenant))
        .join("audit")
        .join("events.jsonl");
    let aside = log.with_extension("aside");
    std::fs::rename(&log, &aside).unwrap();
    std::fs::create_dir(&log).unwrap();
    let a = fixture.a_on_new_build();
    let report = fixture
        .prequalify(&new_build, std::slice::from_ref(&a), &[&bundleless])
        .await
        .unwrap();
    std::fs::remove_dir(&log).unwrap();
    std::fs::rename(&aside, &log).unwrap();
    assert_eq!(
        report,
        PrequalifyReport {
            sets: 1,
            qualified: 1,
            no_active_bundle: 1,
            audit_failed: 1,
            ..Default::default()
        }
    );
    assert!(
        report.ensure_complete().is_err(),
        "a missing audit row fails the run"
    );
    assert!(
        fixture
            .qualified_on(&route.other_tenant, &route.a, &fixture.deployed)
            .await,
        "the contained tenant is qualified"
    );
    assert!(
        fixture
            .qualified_on(&route.tenant, &route.a, &fixture.deployed)
            .await,
        "the row commits before its audit"
    );

    let report = fixture
        .prequalify(&new_build, &[a], &[&bundleless])
        .await
        .unwrap();
    assert_eq!(
        report,
        PrequalifyReport {
            sets: 1,
            already_qualified: 2,
            no_active_bundle: 1,
            ..Default::default()
        }
    );
}

/// T5: a set that does not verify for the new build refuses the whole run
/// with the arming label, before any tenant is qualified, even beside a set
/// that does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_set_that_does_not_verify_records_nothing() {
    let Some(fixture) = Prequalify::new().await else {
        return;
    };
    let route = &fixture.route;
    let new_build = fixture.new_build(|_| {});
    let good = fixture.a_on_new_build();
    let other_revision = fixture.write_set("a-running", &route.a, &route.attestations(&route.a));
    let stale = fixture.write_set(
        "a-stale",
        &route.a,
        &route.attestations_with(
            &route.a,
            &fixture.deployed,
            ROUTE_CHECK_KEY_ID,
            &route.check_pkcs8,
            3_600,
            Some("pipeline_crash_matrix"),
            &[],
        ),
    );
    let mismatch = fixture.sets.path().join("b-with-a");
    write_set_into(
        &mismatch,
        &route.signed(&route.b),
        &route.attestations_on(&route.a, &fixture.deployed),
    );
    for (dir, label) in [
        (
            other_revision,
            PIPELINE_DEFAULT_ROUTING_REVISION_MISMATCH_LABEL,
        ),
        (stale, PIPELINE_DEFAULT_ROUTING_RESULTS_STALE_LABEL),
        (mismatch, PIPELINE_DEFAULT_ROUTING_PACKAGE_MISMATCH_LABEL),
    ] {
        let error = fixture
            .prequalify(&new_build, &[good.clone(), dir], &[])
            .await
            .expect_err("the run is refused");
        assert_eq!(error.to_string(), label);
    }
    for tenant in [&route.tenant, &route.other_tenant] {
        assert!(
            !fixture
                .qualified_on(tenant, &route.a, &fixture.deployed)
                .await,
            "nothing was recorded"
        );
    }
}

/// T7 (decision D3): a process arms from `<dir>/<its revision hex>/` when
/// that directory exists, and from the top level otherwise; the
/// subdirectory of another revision never arms it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn arming_reads_the_subdirectory_of_its_own_revision() {
    let Some(fixture) = Prequalify::new().await else {
        return;
    };
    let route = &fixture.route;
    let dir = fixture.sets.path().join("default-routing");
    // The running build's set at the top level, as today.
    write_set_into(&dir, &route.signed(&route.a), &route.attestations(&route.a));
    // The new build's set beside it.
    let hex = fixture.deployed.strip_prefix("sha256:").unwrap();
    write_set_into(
        &dir.join(hex),
        &route.signed(&route.a),
        &route.attestations_on(&route.a, &fixture.deployed),
    );
    let now = Utc::now();
    let armed = |arming: Arming| matches!(arming, Arming::Armed(_));
    assert!(
        armed(load_arming(&fixture.new_build(|_| {}), &dir, now).await),
        "the new build arms from its subdirectory"
    );
    assert!(
        armed(load_arming(&route.state, &dir, now).await),
        "the running build still arms from the top level"
    );
    let third = route.with(|state| {
        state.pipeline_code_revision_hash = Some(sha256_prefixed("prequalify-third-revision"));
    });
    match load_arming(&third, &dir, now).await {
        Arming::Disarmed { label } => {
            assert_eq!(label, PIPELINE_DEFAULT_ROUTING_REVISION_MISMATCH_LABEL)
        }
        Arming::Armed(_) => panic!("another revision's subdirectory never arms"),
    }
}

/// T8: the runtime identity the mode records is the one the serving process
/// of the same build computes, so its `activate` passes the gate's term 6
/// on the mode's qualification.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_serving_process_of_the_build_activates_on_the_modes_qualification() {
    let Some(fixture) = Prequalify::new().await else {
        return;
    };
    let route = &fixture.route;
    let mode_state = fixture.new_build(|_| {});
    let report = fixture
        .prequalify(&mode_state, &[fixture.a_on_new_build()], &[])
        .await
        .unwrap();
    assert_eq!(report.qualified, 2, "{report:?}");
    let serving = fixture.new_build(|_| {});
    let recorded = PipelineQualificationStore::new(route.runtime.clone())
        .qualification(&route.tenant, &route.a.bundle_id, &fixture.deployed)
        .await
        .unwrap()
        .expect("the mode's row");
    let computed = ProductionDependencyProfile::for_bundle(
        serving.pipeline_service.as_deref().unwrap(),
        &route.a,
        pipeline_activation::route_infrastructure_profile(&serving),
    )
    .unwrap()
    .runtime_identity_digest()
    .unwrap();
    assert_eq!(recorded.metadata.runtime_dependency_digest, computed);

    let in_force = route.record_id_in_force(&route.tenant).await;
    let (status, body) = route
        .call(
            &serving,
            "POST",
            "/v1/admin/pipeline/activate",
            Some(&route.admin),
            Some(serde_json::json!({
                "bundle_id": route.a.bundle_id,
                "reason_code": "operator_activates",
                "attestations": route.attestations_on(&route.a, &fixture.deployed),
                "expected_record_id": in_force,
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
