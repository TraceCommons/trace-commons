// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pipeline default routing (spec 2026-10-10): the operator-armed mode that
//! routes every tenant with no routing row to the versioned pipeline, through
//! the same qualification and activation the admin routes use.
//!
//! Every test runs over a real PostgreSQL as the suite's runtime login
//! (`runtime_backend`: `NOSUPERUSER`, `NOBYPASSRLS`, a member of
//! `trace_ingest_runtime` only), so the row-level security and the grants of
//! the V124 enumeration are the ones the pilot's ingest runs under.
//!
//! Nested inside `tests` beside `pipeline_activation_pg_tests`, whose
//! `pub(super)` route fixture it reuses.

use super::*;

use std::sync::Arc;

use super::pipeline_activation_pg_tests::{RouteFixture, clean_envelope};
use super::pipeline_http_pg_tests::{
    account_owner_backend, route_request, route_trace, runtime_backend, tenant_tx,
    wait_for_run_state, write_routing_as_operator,
};
use crate::pipeline_default_routing::{
    DEFAULT_ROUTING_ACTOR_REF, DEFAULT_ROUTING_ATTESTATION_SUFFIX, DEFAULT_ROUTING_PACKAGE_FILE,
    DefaultRouteOutcome, DefaultRoutingPassReport, PIPELINE_DEFAULT_ROUTING_REASON_CODE,
    PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL, PIPELINE_DEFAULT_ROUTING_RESULTS_STALE_LABEL,
    PIPELINE_DEFAULT_ROUTING_REVISION_MISMATCH_LABEL, PipelineDefaultRouting,
    PipelineDefaultRoutingConfig,
};
use trace_commons_server::versioned_pipeline_activation::{PipelineActivationStore, RoutingState};

/// Inserts `tenant_id` into `trace_tenants` as the database owner: a tenant
/// that signed up and has not uploaded yet.
async fn seed_tenant(owner: &PgBackend, tenant_id: &str) {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant_id).await;
    tx.execute(
        "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
        &[&tenant_id],
    )
    .await
    .expect("seed the tenant");
    tx.commit().await.unwrap();
}

/// Every unrouted tenant whose id starts with `prefix`, read in pages of
/// `page` from the cursor `prefix` (every id with the prefix sorts after it).
async fn unrouted_with_prefix(
    store: &PipelineActivationStore,
    prefix: &str,
    page: i64,
) -> Vec<String> {
    let mut found = Vec::new();
    let mut after = prefix.to_string();
    loop {
        let rows = store
            .unrouted_tenants(Some(&after), page)
            .await
            .expect("the unrouted page reads as the runtime role");
        let Some(last) = rows.last().cloned() else {
            return found;
        };
        for tenant_id in rows {
            if !tenant_id.starts_with(prefix) {
                return found;
            }
            found.push(tenant_id);
        }
        after = last;
    }
}

/// Spec test 8: both V124 functions answer the non-superuser runtime role,
/// across tenants, with tenant ids (and the routing state) only, in byte
/// order from a cursor; and the runtime role still cannot read
/// `trace_tenants` across tenants itself.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_enumeration_works_as_the_runtime_role_and_returns_ids_only() {
    let Some(runtime) = runtime_backend(2).await else {
        return;
    };
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let prefix = format!("tenant-defroute-enum-{}-", Uuid::new_v4().simple());
    let unrouted_a = format!("{prefix}a");
    let unrouted_b = format!("{prefix}b");
    let contained = format!("{prefix}c");
    let legacy = format!("{prefix}l");
    let pipeline = format!("{prefix}p");
    seed_tenant(&owner, &unrouted_a).await;
    seed_tenant(&owner, &unrouted_b).await;
    write_routing_as_operator(&contained, "contained").await;
    write_routing_as_operator(&legacy, "legacy").await;
    write_routing_as_operator(&pipeline, "pipeline").await;

    let store = PipelineActivationStore::new(runtime.clone());
    assert!(
        store
            .default_routing_enumeration_ready()
            .await
            .expect("the privilege check reads"),
        "the runtime role may execute both V124 functions"
    );
    // Pages of one prove the cursor; a page of 1000 the bound.
    for page in [1, 1000] {
        assert_eq!(
            unrouted_with_prefix(&store, &prefix, page).await,
            vec![unrouted_a.clone(), unrouted_b.clone()],
            "only the tenants with no routing row, in byte order (page {page})"
        );
    }
    let mut routed = Vec::new();
    let mut after = prefix.clone();
    loop {
        let rows = store
            .routed_tenants(Some(&after), 1)
            .await
            .expect("the routed page reads as the runtime role");
        let Some((last, _)) = rows.last().cloned() else {
            break;
        };
        if !last.starts_with(&prefix) {
            break;
        }
        routed.extend(rows);
        after = last;
    }
    assert_eq!(
        routed,
        vec![
            (contained.clone(), RoutingState::Contained),
            (pipeline.clone(), RoutingState::Pipeline),
        ],
        "the pipeline and contained rows, never a legacy one"
    );
    // A limit outside the bound is clamped, never an error.
    assert!(store.unrouted_tenants(None, -5).await.unwrap().is_empty());

    // The functions return the declared columns and nothing else.
    let client = runtime.trace_pool_for_test().get().await.unwrap();
    let columns = |sql: &'static str| {
        let client = &client;
        async move {
            client
                .prepare(sql)
                .await
                .expect("prepare as the runtime role")
                .columns()
                .iter()
                .map(|column| column.name().to_string())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        columns("SELECT * FROM trace_pipeline_unrouted_tenants(NULL, 1)").await,
        vec!["tenant_id"]
    );
    assert_eq!(
        columns("SELECT * FROM trace_pipeline_routed_tenants(NULL, 1)").await,
        vec!["tenant_id", "routing_state"]
    );

    // Without a tenant context the runtime role reads no tenant directly:
    // the tenant policy either refuses or matches nothing.
    let direct = client
        .query_one(
            "SELECT COUNT(*) FROM trace_tenants WHERE tenant_id LIKE $1 || '%'",
            &[&prefix],
        )
        .await;
    if let Ok(row) = direct {
        assert_eq!(row.get::<_, i64>(0), 0, "no cross-tenant read");
    }
    let direct_routing = client
        .query_one(
            "SELECT COUNT(*) FROM pipeline_tenant_routing WHERE tenant_id LIKE $1 || '%'",
            &[&prefix],
        )
        .await;
    if let Ok(row) = direct_routing {
        assert_eq!(row.get::<_, i64>(0), 0, "no cross-tenant read");
    }

    // The owner of the functions cannot log in or bypass row-level
    // security, and the runtime role is not a member of it.
    let owner_client = owner.trace_pool_for_test().get().await.unwrap();
    let guard = owner_client
        .query_one(
            "SELECT rolcanlogin, rolbypassrls, rolsuper,
                    pg_has_role($1, 'trace_pipeline_routing_enumeration_guard', 'USAGE')
               FROM pg_roles WHERE rolname = 'trace_pipeline_routing_enumeration_guard'",
            &[&super::pipeline_http_pg_tests::PIPELINE_HTTP_RUNTIME_ROLE],
        )
        .await
        .expect("the guard role exists");
    assert!(!guard.get::<_, bool>(0), "the guard cannot log in");
    assert!(!guard.get::<_, bool>(1), "the guard cannot bypass RLS");
    assert!(!guard.get::<_, bool>(2), "the guard is no superuser");
    assert!(
        !guard.get::<_, bool>(3),
        "the runtime role does not hold the guard's privileges"
    );
}

// ---------------------------------------------------------------------------
// The mode itself, through the plain router over the route suite's
// production-qualified state (`RouteFixture`).
// ---------------------------------------------------------------------------

/// The route suite's fixture, a tenant that signed up and is on no list
/// (`newcomer`, with a contributor and an admin credential), and a results
/// directory holding a signed package (A) and its full set of check results
/// for the fixture's revision: what an operator places on the host to arm.
/// `state` is the fixture's state with those credentials and the mode `all`
/// over that directory; it is not armed until `rearm` is called.
struct DefaultRoutingFixture {
    route: RouteFixture,
    results: tempfile::TempDir,
    newcomer: String,
    newcomer_contributor: String,
    newcomer_admin: String,
    state: Arc<AppState>,
}

impl DefaultRoutingFixture {
    async fn new() -> Option<Self> {
        let route = RouteFixture::new().await?;
        let results = tempfile::tempdir().expect("results dir");
        let suffix = Uuid::new_v4().simple().to_string();
        let newcomer = format!("tenant-defroute-new-{suffix}");
        let newcomer_contributor = format!("token-defroute-new-contributor-{suffix}");
        let newcomer_admin = format!("token-defroute-new-admin-{suffix}");
        let placeholder = route.state.clone();
        let mut fixture = Self {
            route,
            results,
            newcomer,
            newcomer_contributor,
            newcomer_admin,
            state: placeholder,
        };
        fixture.write_results(&fixture.route.attestations(&fixture.route.a));
        fixture.state = fixture.process(|_| {});
        Some(fixture)
    }

    /// A process over the fixture's database with its own default routing
    /// (its own arming, cache, and locks), the newcomer's credentials, and
    /// `change` applied: a second replica when called twice.
    fn process(&self, change: impl FnOnce(&mut AppState)) -> Arc<AppState> {
        let results_dir = self.results.path().to_path_buf();
        self.route.with(|state| {
            let mut tokens = (*state.tokens).clone();
            insert_token(
                &mut tokens,
                &self.newcomer,
                &self.newcomer_contributor,
                TokenRole::Contributor,
            );
            insert_token(
                &mut tokens,
                &self.newcomer,
                &self.newcomer_admin,
                TokenRole::Admin,
            );
            for index in 0..CONCURRENT_UPLOADS {
                insert_token(
                    &mut tokens,
                    &self.newcomer,
                    &format!("{}-{index}", self.newcomer_contributor),
                    TokenRole::Contributor,
                );
            }
            state.tokens = Arc::new(tokens);
            let routing = PipelineDefaultRouting::new(PipelineDefaultRoutingConfig {
                results_dir,
                interval: StdDuration::from_secs(60),
                batch: 1000,
            });
            routing.restrict_loop_for_test(BTreeSet::from([
                self.newcomer.clone(),
                self.route.tenant.clone(),
            ]));
            state.pipeline_default_routing = Some(Arc::new(routing));
            change(state);
        })
    }

    /// Replaces the results directory's content with the fixture's signed A
    /// and `attestations`.
    fn write_results(
        &self,
        attestations: &[trace_commons_server::versioned_pipeline_qualification::PipelineCheckAttestation],
    ) {
        for entry in std::fs::read_dir(self.results.path()).unwrap() {
            std::fs::remove_file(entry.unwrap().path()).unwrap();
        }
        std::fs::write(
            self.results.path().join(DEFAULT_ROUTING_PACKAGE_FILE),
            serde_json::to_vec(&self.route.signed(&self.route.a)).unwrap(),
        )
        .unwrap();
        for attestation in attestations {
            std::fs::write(
                self.results.path().join(format!(
                    "{}{DEFAULT_ROUTING_ATTESTATION_SUFFIX}",
                    attestation.result.check_id
                )),
                serde_json::to_vec(attestation).unwrap(),
            )
            .unwrap();
        }
    }

    fn routing_of(state: &AppState) -> &PipelineDefaultRouting {
        state
            .pipeline_default_routing
            .as_deref()
            .expect("the mode is on")
    }

    fn routing(&self) -> &PipelineDefaultRouting {
        Self::routing_of(&self.state)
    }

    /// Reads the results directory into the fixture's process.
    async fn rearm(&self) {
        self.routing().rearm(&self.state).await;
    }

    /// `POST /v1/traces` of a fresh clean envelope by `token` through `state`.
    async fn upload_as(
        &self,
        state: &Arc<AppState>,
        token: &str,
        tag: &str,
    ) -> (StatusCode, serde_json::Value) {
        let envelope = clean_envelope(&format!("default_routing_{tag}")).await;
        route_trace(state, token, &serde_json::to_vec(&envelope).unwrap()).await
    }

    /// The newcomer's upload through the fixture's process.
    async fn newcomer_upload(&self, tag: &str) -> (StatusCode, serde_json::Value) {
        self.upload_as(&self.state, &self.newcomer_contributor, tag)
            .await
    }

    /// `tenant`'s routing row and its events, newest first.
    async fn view(
        &self,
        tenant: &str,
    ) -> trace_commons_server::versioned_pipeline_activation::TenantRoutingView {
        self.route
            .store()
            .routing_view(tenant, 100, Some(&self.route.revision))
            .await
            .expect("the routing view reads")
    }

    /// `GET /v1/admin/config-status` through `state` with `admin`.
    async fn config_status(&self, state: &Arc<AppState>, admin: &str) -> serde_json::Value {
        let (status, body) = route_request(
            state.clone(),
            "GET",
            "/v1/admin/config-status",
            auth_headers(admin),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    }
}

/// The concurrent first uploads of `concurrent_first_uploads_activate_the_tenant_once`,
/// each by its own contributor credential (the submit rate limit is per
/// principal).
const CONCURRENT_UPLOADS: usize = 8;

/// A receipt the pipeline took.
fn is_pipeline_receipt(receipt: &serde_json::Value) -> bool {
    receipt["status"] == "processing"
}

/// Spec test 1: without the mode, nothing changes. A tenant with no routing
/// row that is on no list uploads on the legacy path, config-status says
/// `off`, no loop starts, and no routing row or event is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_the_mode_a_new_tenant_stays_on_the_legacy_path() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    let off = fixture.process(|state| state.pipeline_default_routing = None);
    let (status, receipt) = fixture
        .upload_as(&off, &fixture.newcomer_contributor, "off")
        .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert!(
        !is_pipeline_receipt(&receipt),
        "a legacy receipt: {receipt}"
    );
    let view = fixture.view(&fixture.newcomer).await;
    assert!(view.routing.is_none(), "no routing row is written");
    assert!(view.events.is_empty(), "no event is written");
    assert!(
        pipeline_default_routing::spawn_default_routing_loop(off.clone()).is_none(),
        "no loop runs without the mode"
    );
    let status = fixture.config_status(&off, &fixture.newcomer_admin).await;
    assert_eq!(status["pipeline_default_routing_mode"], "off");
    assert_eq!(status["pipeline_default_routing_armed"], false);
    assert!(status["pipeline_default_routing_label"].is_null());
    assert_eq!(status["pipeline_default_routing_routed_tenant_count"], 0);
}

/// Spec test 2: armed with a valid set, a new tenant's first upload is a
/// pipeline receipt, and the one activation event names the system actor
/// and the reason `pipeline_default_routing`. Config-status says armed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_armed_process_routes_a_new_tenants_first_upload_to_the_pipeline() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    fixture.rearm().await;
    assert!(
        fixture.routing().is_armed(),
        "{:?}",
        fixture.routing().status()
    );
    let status = fixture
        .config_status(&fixture.state, &fixture.newcomer_admin)
        .await;
    assert_eq!(status["pipeline_default_routing_mode"], "all");
    assert_eq!(status["pipeline_default_routing_armed"], true, "{status}");
    assert!(status["pipeline_default_routing_label"].is_null());
    let expires = status["pipeline_default_routing_expires_in_seconds"]
        .as_i64()
        .expect("an armed set has an expiry");
    assert!(expires > 0 && expires <= 3_600, "{expires}");

    let (status, receipt) = fixture.newcomer_upload("first").await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert!(
        is_pipeline_receipt(&receipt),
        "a pipeline receipt: {receipt}"
    );

    let view = fixture.view(&fixture.newcomer).await;
    let routing = view.routing.expect("a routing row");
    assert_eq!(routing.routing_state, RoutingState::Pipeline);
    assert_eq!(routing.actor_principal_ref, DEFAULT_ROUTING_ACTOR_REF);
    assert_eq!(routing.reason_code, PIPELINE_DEFAULT_ROUTING_REASON_CODE);
    assert_eq!(
        view.active_bundle_id.as_deref(),
        Some(fixture.route.a.bundle_id.as_str())
    );
    assert_eq!(view.active_bundle_qualified_on_revision, Some(true));
    let events = serde_json::to_value(&view.events).unwrap();
    let events = events.as_array().unwrap();
    assert_eq!(events.len(), 1, "one activation: {events:?}");
    assert_eq!(events[0]["action"], "activate");
    assert_eq!(events[0]["actor_principal_ref"], DEFAULT_ROUTING_ACTOR_REF);
    assert_eq!(
        events[0]["reason_code"],
        PIPELINE_DEFAULT_ROUTING_REASON_CODE
    );

    // A second upload goes to the pipeline with no further activation.
    let (status, receipt) = fixture.newcomer_upload("second").await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert!(is_pipeline_receipt(&receipt), "{receipt}");
    assert_eq!(fixture.view(&fixture.newcomer).await.events.len(), 1);
    assert!(fixture.routing().serves(&fixture.newcomer));
}

/// Spec test 3: a tenant an operator deactivated to `legacy` is never
/// activated again, by its uploads or by the loop.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tenant_deactivated_to_legacy_is_never_activated_again() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    fixture.rearm().await;
    let (_, receipt) = fixture.newcomer_upload("before").await;
    assert!(is_pipeline_receipt(&receipt), "{receipt}");
    let record = fixture
        .view(&fixture.newcomer)
        .await
        .routing
        .expect("activated")
        .activation_record_id;
    let (status, body) = route_request(
        fixture.state.clone(),
        "POST",
        "/v1/admin/pipeline/deactivate",
        auth_headers(&fixture.newcomer_admin),
        Some(serde_json::json!({
            "reason_code": "operator_returns_tenant_to_legacy",
            "expected_record_id": record,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let events_after_deactivate = fixture.view(&fixture.newcomer).await.events.len();

    let (status, receipt) = fixture.newcomer_upload("after").await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert!(!is_pipeline_receipt(&receipt), "legacy again: {receipt}");
    assert_eq!(
        fixture
            .routing()
            .default_route_tenant(&fixture.state, &fixture.newcomer)
            .await,
        DefaultRouteOutcome::AlreadyDecided
    );
    fixture.routing().run_pass(&fixture.state).await;
    let view = fixture.view(&fixture.newcomer).await;
    assert_eq!(
        view.routing.map(|routing| routing.routing_state),
        Some(RoutingState::Legacy)
    );
    assert_eq!(
        view.events.len(),
        events_after_deactivate,
        "nothing written"
    );
}

/// Spec test 4: a contained tenant is untouched: its uploads are refused as
/// before, and neither the upload nor the loop writes an event.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_contained_tenant_is_left_contained() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    write_routing_as_operator(&fixture.newcomer, "contained").await;
    fixture.rearm().await;
    let before = fixture.view(&fixture.newcomer).await.events.len();
    let (status, body) = fixture.newcomer_upload("contained").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], "pipeline_receipt_intake_contained");
    fixture.routing().run_pass(&fixture.state).await;
    let view = fixture.view(&fixture.newcomer).await;
    assert_eq!(
        view.routing.map(|routing| routing.routing_state),
        Some(RoutingState::Contained)
    );
    assert_eq!(view.events.len(), before, "nothing written");
}

/// Spec test 5: a result set for another revision arms nothing, says
/// `pipeline_default_routing_revision_mismatch`, and the upload is legacy.
/// A missing set, a stale one, and one an untrusted key signed each say
/// their own label, and none of them activates anything either.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_set_that_does_not_verify_for_this_process_arms_nothing() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    let other_revision = sha256_prefixed("default-routing-another-revision");
    let untrusted =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap()
            .as_ref()
            .to_vec();
    let cases = [
        (
            fixture.route.attestations_on(&fixture.route.a, &other_revision),
            PIPELINE_DEFAULT_ROUTING_REVISION_MISMATCH_LABEL,
        ),
        (
            fixture
                .route
                .attestations_aged(&fixture.route.a, 3_600, Some("pipeline_restore_drill")),
            PIPELINE_DEFAULT_ROUTING_RESULTS_STALE_LABEL,
        ),
        (
            fixture.route.attestations_signed_by(
                &fixture.route.a,
                "untrusted-check-key",
                &untrusted,
            ),
            trace_commons_server::versioned_pipeline_qualification::CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL,
        ),
        (Vec::new(), PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL),
    ];
    for (attestations, label) in cases {
        fixture.write_results(&attestations);
        fixture.rearm().await;
        assert!(!fixture.routing().is_armed(), "{label}");
        let status = fixture
            .config_status(&fixture.state, &fixture.newcomer_admin)
            .await;
        assert_eq!(status["pipeline_default_routing_armed"], false);
        assert_eq!(status["pipeline_default_routing_label"], label, "{status}");
        let (code, receipt) = fixture.newcomer_upload(label).await;
        assert_eq!(code, StatusCode::OK, "{receipt}");
        assert!(!is_pipeline_receipt(&receipt), "{label}: legacy: {receipt}");
        fixture.routing().run_pass(&fixture.state).await;
        assert!(
            fixture.view(&fixture.newcomer).await.routing.is_none(),
            "{label}: no tenant is activated"
        );
        assert!(
            fixture
                .route
                .qualification(&fixture.newcomer, &fixture.route.a)
                .await
                .is_none(),
            "{label}: nothing is qualified"
        );
    }
    // A directory that is not there at all.
    let gone = fixture.process(|state| {
        let routing = PipelineDefaultRouting::new(PipelineDefaultRoutingConfig {
            results_dir: fixture.results.path().join("absent"),
            interval: StdDuration::from_secs(60),
            batch: 50,
        });
        routing.restrict_loop_for_test(BTreeSet::new());
        state.pipeline_default_routing = Some(Arc::new(routing));
    });
    DefaultRoutingFixture::routing_of(&gone).rearm(&gone).await;
    assert_eq!(
        DefaultRoutingFixture::routing_of(&gone)
            .arming_label()
            .as_deref(),
        Some(PIPELINE_DEFAULT_ROUTING_RESULTS_MISSING_LABEL)
    );
    // And the valid set arms again, with no restart.
    fixture.write_results(&fixture.route.attestations(&fixture.route.a));
    fixture.rearm().await;
    assert!(fixture.routing().is_armed());
}

/// An armed set disarms itself when its earliest result expires.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_armed_set_stops_activating_when_it_expires() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    fixture.rearm().await;
    assert!(fixture.routing().is_armed());
    fixture
        .routing()
        .set_now_for_test(Some(Utc::now() + chrono::Duration::hours(2)));
    assert!(!fixture.routing().is_armed());
    assert_eq!(
        fixture.routing().arming_label().as_deref(),
        Some(PIPELINE_DEFAULT_ROUTING_RESULTS_STALE_LABEL)
    );
    assert_eq!(
        fixture
            .routing()
            .default_route_tenant(&fixture.state, &fixture.newcomer)
            .await,
        DefaultRouteOutcome::NotArmed
    );
    assert!(fixture.view(&fixture.newcomer).await.routing.is_none());
}

/// Spec test 6: concurrent first uploads, on one process and on two
/// processes over one database, write exactly one activation, and every
/// upload succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_first_uploads_activate_the_tenant_once() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    let fixture = Arc::new(fixture);
    let second = fixture.process(|_| {});
    fixture.rearm().await;
    DefaultRoutingFixture::routing_of(&second)
        .rearm(&second)
        .await;
    let mut uploads = Vec::new();
    for index in 0..CONCURRENT_UPLOADS {
        let fixture = fixture.clone();
        let state = if index % 2 == 0 {
            fixture.state.clone()
        } else {
            second.clone()
        };
        uploads.push(tokio::spawn(async move {
            fixture
                .upload_as(
                    &state,
                    &format!("{}-{index}", fixture.newcomer_contributor),
                    &format!("concurrent_{index}"),
                )
                .await
        }));
    }
    for upload in uploads {
        let (status, receipt) = upload.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{receipt}");
        assert!(is_pipeline_receipt(&receipt), "{receipt}");
    }
    let events = serde_json::to_value(fixture.view(&fixture.newcomer).await.events).unwrap();
    let activations = events
        .as_array()
        .unwrap()
        .iter()
        .filter(|event| event["action"] == "activate")
        .count();
    assert_eq!(activations, 1, "exactly one activation: {events}");
}

/// Spec test 7: the worker drains a tenant routed after it started, with no
/// restart: its tenant list is read again on every pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_worker_processes_a_tenant_routed_after_it_started() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    let worker = pipeline_runtime::spawn_pipeline_worker(fixture.state.clone())
        .expect("a runtime, so a worker");
    tokio::time::sleep(StdDuration::from_millis(500)).await;
    assert!(!fixture.routing().serves(&fixture.newcomer));
    fixture.rearm().await;
    let envelope = clean_envelope("default_routing_worker").await;
    let (status, receipt) = route_trace(
        &fixture.state,
        &fixture.newcomer_contributor,
        &serde_json::to_vec(&envelope).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert!(is_pipeline_receipt(&receipt), "{receipt}");
    wait_for_run_state(
        &fixture.route.runtime,
        &fixture.newcomer,
        envelope.submission_id,
        "complete",
    )
    .await;
    assert!(
        fixture
            .state
            .pipeline_worker_pass_stats
            .tenant_count
            .load(std::sync::atomic::Ordering::Relaxed)
            >= 1
    );
    let _ = worker.stop.send(true);
    pipeline_runtime::join_or_abort(worker.join, StdDuration::from_secs(10)).await;
}

/// Spec test 9: after a deploy, the loop re-qualifies a tenant this mode
/// routed on the new revision, and leaves its routing row as it is. A
/// tenant an operator activated by hand is not re-qualified (owner
/// decision: only rows default routing wrote).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_loop_requalifies_its_tenants_after_a_deploy() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    fixture.rearm().await;
    let (_, receipt) = fixture.newcomer_upload("before_deploy").await;
    assert!(is_pipeline_receipt(&receipt), "{receipt}");
    let before = fixture.view(&fixture.newcomer).await;
    // The fixture's listed tenant, activated by hand through the routes.
    fixture.route.qualify(&fixture.route.a).await;
    let (status, body) = fixture
        .route
        .admin_call(
            "POST",
            "/v1/admin/pipeline/activate",
            Some(
                fixture
                    .route
                    .activate_body(&fixture.route.a, "operator_activates")
                    .await,
            ),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // The new build: another revision, and the operator's set for it.
    let deployed = sha256_prefixed("default-routing-deployed-revision");
    let new_build = fixture.process(|state| {
        state.pipeline_code_revision_hash = Some(deployed.clone());
    });
    fixture.write_results(&fixture.route.attestations_on(&fixture.route.a, &deployed));
    let (status, body) = fixture
        .upload_as(
            &new_build,
            &fixture.newcomer_contributor,
            "deployed_unqualified",
        )
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");

    let report = DefaultRoutingFixture::routing_of(&new_build)
        .run_pass(&new_build)
        .await;
    assert_eq!(report.requalified, 1, "only the newcomer: {report:?}");
    let qualified = |tenant: String| {
        let store = PipelineQualificationStore::new(fixture.route.runtime.clone());
        let bundle = fixture.route.a.bundle_id.clone();
        let deployed = deployed.clone();
        async move {
            store
                .qualification(&tenant, &bundle, &deployed)
                .await
                .unwrap()
                .is_some()
        }
    };
    assert!(qualified(fixture.newcomer.clone()).await);
    assert!(
        !qualified(fixture.route.tenant.clone()).await,
        "a hand-activated tenant is not re-qualified"
    );
    let after = fixture.view(&fixture.newcomer).await;
    assert_eq!(
        after
            .routing
            .as_ref()
            .map(|routing| routing.activation_record_id),
        before
            .routing
            .as_ref()
            .map(|routing| routing.activation_record_id),
        "the routing row is unchanged"
    );
    assert_eq!(after.events.len(), before.events.len(), "no routing event");
    let (status, receipt) = fixture
        .upload_as(
            &new_build,
            &fixture.newcomer_contributor,
            "deployed_requalified",
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert!(is_pipeline_receipt(&receipt), "{receipt}");
}

/// The loop routes a tenant that signed up and has not uploaded yet, before
/// its first upload.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_loop_routes_a_tenant_that_has_not_uploaded_yet() {
    let Some(fixture) = DefaultRoutingFixture::new().await else {
        return;
    };
    seed_tenant(&fixture.route.owner, &fixture.newcomer).await;
    fixture.rearm().await;
    // Other suites leave tenants with no row in the shared database; pass
    // until the cursor reaches the newcomer.
    let mut report = DefaultRoutingPassReport::default();
    for _ in 0..1_000 {
        report = fixture.routing().run_pass(&fixture.state).await;
        if fixture.view(&fixture.newcomer).await.routing.is_some() {
            break;
        }
    }
    let routing = fixture
        .view(&fixture.newcomer)
        .await
        .routing
        .expect("the loop activated the tenant");
    assert_eq!(routing.routing_state, RoutingState::Pipeline);
    assert_eq!(routing.actor_principal_ref, DEFAULT_ROUTING_ACTOR_REF);
    let _ = report;
    assert!(fixture.routing().serves(&fixture.newcomer));
    let status = fixture
        .config_status(&fixture.state, &fixture.newcomer_admin)
        .await;
    assert!(
        status["pipeline_default_routing_last_pass"].is_object(),
        "{status}"
    );
    let (_, receipt) = fixture.newcomer_upload("after_loop").await;
    assert!(is_pipeline_receipt(&receipt), "{receipt}");
}
