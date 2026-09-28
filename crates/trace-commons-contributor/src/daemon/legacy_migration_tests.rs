//! The legacy invite migration against a mock ingest that does what #1066
//! does: issues a challenge, verifies the statement against the device key
//! in the request, and countersigns the record.

use super::*;
use crate::daemon::grant_terms::GrantTerms;
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use ring::signature::Ed25519KeyPair;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use trace_commons_protocol::legacy_invite_link::legacy_invite_link_record_bytes;

const INVITE_CODE: &str = "INVITECODE000001";
const PROJECT: &str = "/Users/invitee/code/armed-project";

/// How the mock ingest answers the link.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LinkMode {
    /// As #1066 does.
    Honest,
    /// A countersigned record naming some other account.
    OtherAccount,
    /// A record whose device signature is not this device's.
    ForeignSignature,
    /// 409 `legacy_link_tenant_pooled`, as for a Devfolio shared code.
    Pooled,
}

struct Ingest {
    token: String,
    tenant: String,
    account: Uuid,
    mode: LinkMode,
    ready: bool,
    server_key: Ed25519KeyPair,
    challenges: AtomicUsize,
    links: AtomicUsize,
    /// The last request the link route verified, for assertions.
    linked: Mutex<Option<LegacyInviteLinkRequest>>,
    /// Released by a test to let the link answer.
    gate: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
    /// Bearer tokens presented to `/v1/account/logout`.
    logouts: Mutex<Vec<String>>,
}

async fn logout_route(State(ingest): State<Arc<Ingest>>, headers: HeaderMap) -> impl IntoResponse {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    ingest.logouts.lock().unwrap().push(bearer);
    StatusCode::NO_CONTENT
}

fn authorized(ingest: &Ingest, headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == format!("Bearer {}", ingest.token))
}

async fn status_route(State(ingest): State<Arc<Ingest>>, headers: HeaderMap) -> impl IntoResponse {
    if !authorized(&ingest, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({"error": "unauthorized"})),
        );
    }
    (
        StatusCode::OK,
        Json(serde_json::json!({"authority": "invited", "ready": ingest.ready})),
    )
}

async fn challenge_route(
    State(ingest): State<Arc<Ingest>>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !authorized(&ingest, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "account session required"})),
        );
    }
    ingest.challenges.fetch_add(1, Ordering::SeqCst);
    let now = Utc::now().timestamp();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "nonce": "d".repeat(64),
            "issued_at": now,
            "expires_at": now + 300,
        })),
    )
}

async fn link_route(
    State(ingest): State<Arc<Ingest>>,
    headers: HeaderMap,
    Json(request): Json<LegacyInviteLinkRequest>,
) -> impl IntoResponse {
    if !authorized(&ingest, &headers) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "account session required"})),
        );
    }
    let gate = ingest.gate.lock().unwrap().take();
    if let Some(gate) = gate {
        let _ = tokio::task::spawn_blocking(move || gate.recv()).await;
    }
    ingest.links.fetch_add(1, Ordering::SeqCst);
    if ingest.mode == LinkMode::Pooled {
        return (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": "legacy_link_tenant_pooled"})),
        );
    }
    // The account half comes from the session, never the body.
    let statement = LegacyInviteLinkStatement {
        legacy_tenant_id: request.legacy_tenant_id.clone(),
        device_key_id: request.device_key_id.clone(),
        invite_subject_hash: request.invite_subject_hash.clone(),
        account_tenant_id: ingest.tenant.clone(),
        account_id: ingest.account,
        nonce: request.nonce.clone(),
        issued_at: request.issued_at,
    };
    let engine = base64::engine::general_purpose::STANDARD;
    let public_key = engine.decode(&request.device_public_key).unwrap();
    let signature = engine.decode(&request.signature).unwrap();
    if ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &public_key)
        .verify(&legacy_invite_link_statement_bytes(&statement), &signature)
        .is_err()
    {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "legacy_link_signature_invalid"})),
        );
    }
    *ingest.linked.lock().unwrap() = Some(request.clone());
    let mut record = LegacyInviteLinkRecord {
        link_id: Uuid::new_v4(),
        statement,
        device_signature: request.signature.clone(),
        linked_at: Utc::now().timestamp(),
        server_kid: "ingest-test".into(),
    };
    match ingest.mode {
        LinkMode::OtherAccount => record.statement.account_id = Uuid::new_v4(),
        LinkMode::ForeignSignature => {
            let other = Ed25519KeyPair::from_pkcs8(
                Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                    .unwrap()
                    .as_ref(),
            )
            .unwrap();
            record.device_signature = engine.encode(
                other
                    .sign(&legacy_invite_link_statement_bytes(&record.statement))
                    .as_ref(),
            );
        }
        _ => {}
    }
    let server_signature = engine.encode(
        ingest
            .server_key
            .sign(&legacy_invite_link_record_bytes(&record))
            .as_ref(),
    );
    (
        StatusCode::OK,
        Json(
            serde_json::to_value(LegacyInviteLinkResponse {
                record,
                server_signature,
                authority: "invited".into(),
                trust_version: 1,
            })
            .unwrap(),
        ),
    )
}

async fn spawn_ingest(mode: LinkMode, ready: bool) -> (String, Arc<Ingest>) {
    let tenant = format!("nearai-{}", "3c".repeat(32));
    let ingest = Arc::new(Ingest {
        token: crate::daemon::account_admission::native_token_for_test(&tenant, "secret"),
        tenant,
        account: Uuid::new_v4(),
        mode,
        ready,
        server_key: Ed25519KeyPair::from_pkcs8(
            Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .unwrap()
                .as_ref(),
        )
        .unwrap(),
        challenges: AtomicUsize::new(0),
        links: AtomicUsize::new(0),
        linked: Mutex::new(None),
        gate: Mutex::new(None),
        logouts: Mutex::new(Vec::new()),
    });
    let app = Router::new()
        .route("/v1/account/contribution-status", get(status_route))
        .route(CHALLENGE_PATH, post(challenge_route))
        .route(LINK_PATH, post(link_route))
        .route("/v1/account/logout", post(logout_route))
        .with_state(ingest.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), ingest)
}

/// Stands in for the NEAR AI ceremony: the staged key is "provisioned" into
/// the mock ingest's account. What it may not do is run before the staged
/// key exists, so it checks that.
struct FakeProvisioner {
    ingest: Arc<Ingest>,
    /// Runs before provisioning completes, to simulate something happening
    /// mid-migration (a logout).
    during: Option<Box<dyn Fn(&DaemonShared) + Send + Sync>>,
    fail: bool,
}

impl AccountProvisioner for FakeProvisioner {
    async fn provision(
        &self,
        shared: &DaemonShared,
        cfg: &ContributorConfig,
        staged: &DeviceIdentity,
    ) -> Result<ProvisionedAccount> {
        assert_eq!(
            commons_credentials::load_staged_device_key(&shared.store)
                .unwrap()
                .map(|k| k.device_key_id),
            Some(staged.device_key_id.clone()),
            "the staged key exists before the ceremony runs"
        );
        if let Some(during) = &self.during {
            during(shared);
        }
        if self.fail {
            bail!("near_ai_enroll_verification_failed");
        }
        Ok(ProvisionedAccount {
            tenant_id: self.ingest.tenant.clone(),
            account_id: self.ingest.account,
            access_token: self.ingest.token.clone(),
            expires_in_secs: 3600,
            issuer_url: cfg.issuer_url.clone(),
            audience: cfg.audience.clone(),
        })
    }
}

fn provisioner(ingest: &Arc<Ingest>) -> FakeProvisioner {
    FakeProvisioner {
        ingest: ingest.clone(),
        during: None,
        fail: false,
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    shared: Arc<DaemonShared>,
    legacy: DeviceIdentity,
    cfg: ContributorConfig,
    ingest: Arc<Ingest>,
}

fn legacy_config(ingest_url: &str, device_key_id: &str) -> ContributorConfig {
    let mut cfg = crate::commands::unenrolled_preview_config();
    cfg.ingest_url = ingest_url.to_string();
    cfg.issuer_url = "https://issuer.example".to_string();
    cfg.tenant_id = "tenant-invitee".to_string();
    cfg.instance_id = String::new();
    cfg.user_subject = "invitee".to_string();
    cfg.device_key_id = device_key_id.to_string();
    cfg.consent_scopes_chosen = true;
    cfg
}

/// An enrolled legacy invitee with one armed folder, granted under the
/// terms in force, and a legacy account session.
async fn fixture(mode: LinkMode, ready: bool) -> Fixture {
    let (dir, store) = crate::config::tests_support::temp_store();
    let (url, ingest) = spawn_ingest(mode, ready).await;
    let legacy = DeviceIdentity::load_or_generate(&store).unwrap();
    let cfg = legacy_config(&url, &legacy.device_key_id);
    store.save_config(&cfg).unwrap();
    store
        .write_daemon_file(
            crate::config::ACCOUNT_SESSION_FILE,
            &serde_json::to_vec(&crate::account_auth::AccountSession {
                access_token: "tcn1_legacy".into(),
                expires_at: Utc::now() + chrono::Duration::hours(1),
                account_id: "legacy-account".into(),
            })
            .unwrap(),
        )
        .unwrap();
    let shared = Arc::new(DaemonShared::load(store).unwrap());
    let terms = GrantTerms::in_force(&shared).unwrap();
    {
        let mut policy = shared.policy.lock().unwrap();
        policy
            .set_mode(PROJECT, ProjectMode::AutoUpload, Utc::now())
            .unwrap();
        assert!(policy.record_grant_terms(PROJECT, terms));
        policy.save(&shared.store).unwrap();
    }
    Fixture {
        _dir: dir,
        shared,
        legacy,
        cfg,
        ingest,
    }
}

impl Fixture {
    async fn migrate(&self, provisioner: &FakeProvisioner) -> Result<serde_json::Value> {
        migrate(&self.shared, provisioner, Some(INVITE_CODE)).await
    }

    fn project(&self) -> super::super::policy::ProjectEntry {
        self.shared.policy.lock().unwrap().projects[PROJECT].clone()
    }

    /// What a watcher pass would do now: sweep the grants against the terms
    /// the config on disk gives.
    fn sweep(&self) -> super::super::policy::GrantSweep {
        let current = GrantTerms::in_force(&self.shared).unwrap();
        self.shared
            .policy
            .lock()
            .unwrap()
            .sweep_grants(&current, Utc::now())
    }

    /// The legacy identity, untouched and fully working.
    fn assert_legacy_intact(&self) {
        let store = &self.shared.store;
        let on_disk = store.load_config().unwrap().unwrap();
        assert_eq!(on_disk.tenant_id, self.cfg.tenant_id);
        assert_eq!(on_disk.device_key_id, self.legacy.device_key_id);
        assert_eq!(
            DeviceIdentity::load(store).unwrap().unwrap().device_key_id,
            self.legacy.device_key_id,
            "the legacy key still signs"
        );
        assert_eq!(
            crate::account_auth::try_load_token(store)
                .unwrap()
                .as_deref(),
            Some("tcn1_legacy"),
            "the legacy session is still usable"
        );
        assert!(
            commons_credentials::load_staged_device_key(store)
                .unwrap()
                .is_none(),
            "no staged key left behind"
        );
        assert!(
            commons_credentials::pending_switch(store)
                .unwrap()
                .is_none()
        );
        assert!(load_link(store).is_none());
        let project = self.project();
        assert_eq!(project.mode, ProjectMode::AutoUpload);
        assert_eq!(
            project.armed_under.as_ref().unwrap().identity(),
            IdentityTerm::of_config(&self.cfg),
            "still armed under the legacy identity"
        );
        let sweep = self.sweep();
        assert!(sweep.voided.is_empty(), "a pass voids nothing: {sweep:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_migration_switches_identity_keeps_armed_folders_and_retires_the_legacy_key() {
    let f = fixture(LinkMode::Honest, true).await;
    let before = f.project().armed_under.unwrap();
    let answer = f.migrate(&provisioner(&f.ingest)).await.unwrap();
    assert_eq!(answer["migrated"], true);
    assert_eq!(answer["folders_kept"], 1);

    let store = &f.shared.store;
    let cfg = store.load_config().unwrap().unwrap();
    assert_eq!(cfg.tenant_id, f.ingest.tenant);
    assert_ne!(cfg.device_key_id, f.legacy.device_key_id);
    assert_eq!(
        DeviceIdentity::load(store).unwrap().unwrap().device_key_id,
        cfg.device_key_id,
        "the staged key is now the device key"
    );
    assert_eq!(
        crate::account_auth::try_load_token(store)
            .unwrap()
            .as_deref(),
        Some(f.ingest.token.as_str()),
        "the NEAR AI session is stored"
    );
    assert!(
        commons_credentials::pending_switch(store)
            .unwrap()
            .is_none()
    );
    // Everything but the identity term is as it was, and the folder is armed.
    let project = f.project();
    assert_eq!(project.mode, ProjectMode::AutoUpload);
    let after = project.armed_under.unwrap();
    assert_eq!(after.identity(), IdentityTerm::of_config(&cfg));
    assert_eq!(after.with_identity(&before.identity()), before);
    // The next pass voids nothing.
    assert!(f.sweep().voided.is_empty());
    // The statement was signed with the legacy key, naming its invite.
    let linked = f.ingest.linked.lock().unwrap().clone().unwrap();
    assert_eq!(linked.device_key_id, f.legacy.device_key_id);
    assert_eq!(linked.legacy_tenant_id, "tenant-invitee");
    assert_eq!(linked.invite_subject_hash, invite_subject_hash(INVITE_CODE));
    // The link and notice are kept, and the rebaseline audited, label-only.
    let link = load_link(store).unwrap();
    assert_eq!(link.folders_kept, 1);
    assert!(!link.notice_acknowledged);
    let audit = crate::daemon::audit::load(store).unwrap();
    assert!(
        audit
            .iter()
            .any(|e| e.action == "auto-upload-rebaselined"
                && e.detail.as_deref() == Some("identity"))
    );
    assert!(audit.iter().any(|e| e.action == "legacy-invite-migrated"));
    let rendered = serde_json::to_string(&audit).unwrap();
    for secret in [
        f.ingest.tenant.as_str(),
        "tenant-invitee",
        f.legacy.device_key_id.as_str(),
    ] {
        assert!(!rendered.contains(secret), "audit carries {secret}");
    }
}

/// The control for the test above: the same identity change without the
/// re-baseline voids the folder. This is what the migration exists to avoid,
/// and what any other identity change still does.
#[tokio::test(flavor = "multi_thread")]
async fn without_the_rebaseline_the_same_switch_voids_the_folder() {
    let f = fixture(LinkMode::Honest, true).await;
    let mut other = f.cfg.clone();
    other.tenant_id = f.ingest.tenant.clone();
    f.shared.store.save_config(&other).unwrap();
    let sweep = f.sweep();
    assert_eq!(sweep.voided.len(), 1);
    assert_eq!(
        sweep.voided[0].reasons,
        [super::super::grant_terms::VOID_IDENTITY]
    );
}

/// A different change made at the same time still voids: only the identity
/// term is carried over.
#[test]
fn only_the_identity_term_is_rebaselined() {
    let mut cfg = legacy_config("https://ingest.example", "sha256:aa");
    let granted = GrantTerms::current(&cfg, None, false, "none");
    let mut policy = ProjectPolicy::new();
    policy
        .set_mode(PROJECT, ProjectMode::AutoUpload, Utc::now())
        .unwrap();
    policy.record_grant_terms(PROJECT, granted.clone());
    let from = IdentityTerm::of_config(&cfg);
    cfg.tenant_id = format!("nearai-{}", "3c".repeat(32));
    cfg.device_key_id = "sha256:bb".into();
    cfg.user_subject = "sha256:bb".into();
    let to = IdentityTerm::of_config(&cfg);
    let out = rebaseline_identity(&mut policy, &from, &to);
    assert_eq!(out.project_labels.len(), 1);
    // Identity alone: nothing voids.
    let now_terms = GrantTerms::current(&cfg, None, false, "none");
    assert!(
        now_terms
            .widening_from(&policy.projects[PROJECT].armed_under.clone().unwrap())
            .is_empty()
    );
    // Identity plus a destination: the destination still voids.
    cfg.ingest_url = "https://elsewhere.example".into();
    let moved = GrantTerms::current(&cfg, None, false, "none");
    assert_eq!(
        moved.widening_from(&policy.projects[PROJECT].armed_under.clone().unwrap()),
        [super::super::grant_terms::VOID_DESTINATION]
    );
    // A folder armed under some other identity is not carried over.
    let stranger = IdentityTerm {
        tenant_id: "tenant-else".into(),
        ..from.clone()
    };
    let mut other = ProjectPolicy::new();
    other
        .set_mode(PROJECT, ProjectMode::AutoUpload, Utc::now())
        .unwrap();
    other.record_grant_terms(PROJECT, granted);
    assert!(
        rebaseline_identity(&mut other, &stranger, &to)
            .project_labels
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_record_for_another_account_changes_nothing() {
    let f = fixture(LinkMode::OtherAccount, true).await;
    let error = f.migrate(&provisioner(&f.ingest)).await.unwrap_err();
    assert_eq!(label(&error), "legacy_migration_verification_failed");
    f.assert_legacy_intact();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_record_not_signed_by_this_device_changes_nothing() {
    let f = fixture(LinkMode::ForeignSignature, true).await;
    let error = f.migrate(&provisioner(&f.ingest)).await.unwrap_err();
    assert_eq!(label(&error), "legacy_migration_verification_failed");
    f.assert_legacy_intact();
}

/// A shared event invite is refused by the server; the reason is named and
/// the device keeps contributing under it exactly as before.
#[tokio::test(flavor = "multi_thread")]
async fn a_pooled_invite_is_refused_by_name_and_keeps_working() {
    let f = fixture(LinkMode::Pooled, true).await;
    let error = f.migrate(&provisioner(&f.ingest)).await.unwrap_err();
    assert_eq!(label(&error), "legacy_migration_tenant_pooled");
    f.assert_legacy_intact();
    assert_eq!(
        status_value(&f.shared.store, Some(&f.cfg))["offered"],
        true,
        "still offered: nothing was recorded"
    );
}

/// No switch before ingest admits the account by account: the link is not
/// even asked for.
#[tokio::test(flavor = "multi_thread")]
async fn nothing_moves_until_ingest_answers_ready() {
    let f = fixture(LinkMode::Honest, false).await;
    let error = f.migrate(&provisioner(&f.ingest)).await.unwrap_err();
    assert_eq!(label(&error), "legacy_migration_admission_not_ready");
    assert_eq!(f.ingest.challenges.load(Ordering::SeqCst), 0);
    assert_eq!(f.ingest.links.load(Ordering::SeqCst), 0);
    f.assert_legacy_intact();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_failed_ceremony_changes_nothing() {
    let f = fixture(LinkMode::Honest, true).await;
    let mut p = provisioner(&f.ingest);
    p.fail = true;
    let error = f.migrate(&p).await.unwrap_err();
    assert_eq!(label(&error), "legacy_migration_account_unavailable");
    f.assert_legacy_intact();
}

/// A failure between any two steps of the switch leaves the legacy identity
/// fully working, and a pass after it voids nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_failure_between_the_steps_leaves_the_legacy_identity_working() {
    for step in 1..=5u8 {
        let f = fixture(LinkMode::Honest, true).await;
        if step <= 4 {
            commons_credentials::fail_switch_after_for_test(&f.shared.store, step);
        }
        if step == 5 {
            fail_after_rebaseline_for_test(&f.shared.store);
        }
        let error = f.migrate(&provisioner(&f.ingest)).await.unwrap_err();
        assert!(
            label(&error).starts_with("legacy_migration_"),
            "step {step}: {error}"
        );
        f.assert_legacy_intact();
    }
}

/// The logout guard: a sign-out while the ceremony runs discards the legacy
/// key, and the migration does not pass through it.
#[tokio::test(flavor = "multi_thread")]
async fn a_logout_mid_migration_ends_it_and_restores_nothing() {
    let f = fixture(LinkMode::Honest, true).await;
    let mut p = provisioner(&f.ingest);
    p.during = Some(Box::new(|shared: &DaemonShared| {
        shared.store.wipe().unwrap()
    }));
    let error = f.migrate(&p).await.unwrap_err();
    assert_eq!(label(&error), "legacy_migration_identity_changed");
    let store = &f.shared.store;
    assert!(store.load_config().unwrap().is_none(), "still logged out");
    assert!(DeviceIdentity::load(store).unwrap().is_none());
    assert!(
        crate::account_auth::try_load_token(store)
            .unwrap()
            .is_none()
    );
    assert!(
        commons_credentials::pending_switch(store)
            .unwrap()
            .is_none()
    );
    assert!(load_link(store).is_none());
}

/// The void sweep and the switch never interleave: while a pass holds the
/// pass lock, the switch waits, so no pass reads the old config and then
/// sweeps grants already re-recorded under the new identity.
#[tokio::test(flavor = "multi_thread")]
async fn the_switch_waits_for_a_pass_in_flight() {
    let f = fixture(LinkMode::Honest, true).await;
    let (hold_tx, hold_rx) = std::sync::mpsc::channel::<()>();
    let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
    let pass_shared = f.shared.clone();
    // A "pass" that has read the legacy config and holds the lock.
    let pass = std::thread::spawn(move || {
        let _pass = pass_shared.pass_lock.lock().unwrap();
        let terms = GrantTerms::in_force(&pass_shared).unwrap();
        held_tx.send(()).unwrap();
        hold_rx.recv().unwrap();
        // Its sweep, against the config it read, before releasing.
        let sweep = pass_shared
            .policy
            .lock()
            .unwrap()
            .sweep_grants(&terms, Utc::now());
        sweep.voided.len()
    });
    held_rx.recv().unwrap();
    let shared = f.shared.clone();
    let p = provisioner(&f.ingest);
    let migration =
        tokio::spawn(async move { migrate(&shared, &p, Some(INVITE_CODE)).await.map(|_| ()) });
    // The link lands, but nothing is switched while the pass holds the lock.
    for _ in 0..200 {
        if f.ingest.links.load(Ordering::SeqCst) == 1 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(f.ingest.links.load(Ordering::SeqCst), 1);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(
        f.shared.store.load_config().unwrap().unwrap().tenant_id,
        "tenant-invitee",
        "switched while a pass was in flight"
    );
    hold_tx.send(()).unwrap();
    assert_eq!(pass.join().unwrap(), 0, "the in-flight pass voided nothing");
    migration.await.unwrap().unwrap();
    assert_eq!(
        f.shared.store.load_config().unwrap().unwrap().tenant_id,
        f.ingest.tenant
    );
    assert!(f.sweep().voided.is_empty(), "nor does the pass after it");
    assert_eq!(f.project().mode, ProjectMode::AutoUpload);
}

/// A daemon that died after the switch wrote its files but before it
/// committed comes back as the legacy identity.
#[tokio::test(flavor = "multi_thread")]
async fn an_uncommitted_switch_is_rolled_back_at_the_next_start() {
    let f = fixture(LinkMode::Honest, true).await;
    let store = f.shared.store.clone();
    let legacy_snapshot = commons_credentials::snapshot(&store, Kind::Device).unwrap();
    let staged = commons_credentials::stage_device_key(&store).unwrap();
    let mut next = f.cfg.clone();
    next.tenant_id = f.ingest.tenant.clone();
    next.device_key_id = staged.device_key_id.clone();
    next.user_subject = staged.device_key_id.clone();
    let context = SwitchContext {
        old: IdentityTerm::of_config(&f.cfg),
        new: IdentityTerm::of_config(&next),
        link: StoredLink {
            record: LegacyInviteLinkRecord {
                link_id: Uuid::nil(),
                statement: LegacyInviteLinkStatement {
                    legacy_tenant_id: f.cfg.tenant_id.clone(),
                    device_key_id: f.legacy.device_key_id.clone(),
                    invite_subject_hash: invite_subject_hash(INVITE_CODE),
                    account_tenant_id: f.ingest.tenant.clone(),
                    account_id: f.ingest.account,
                    nonce: "d".repeat(64),
                    issued_at: 1,
                },
                device_signature: String::new(),
                linked_at: 1,
                server_kid: "k".into(),
            },
            server_signature: String::new(),
            migrated_at: Utc::now(),
            folders_kept: 1,
            automatic_grant_kept: false,
            notice_acknowledged: false,
        },
    };
    commons_credentials::switch_identity(
        &store,
        &legacy_snapshot,
        &next,
        b"{}",
        serde_json::to_value(&context).unwrap(),
    )
    .unwrap();
    {
        let mut policy = f.shared.policy.lock().unwrap();
        rebaseline_identity(&mut policy, &context.old, &context.new);
        policy.save(&store).unwrap();
    }
    // "Crash": a fresh daemon on the same directory.
    let restarted = DaemonShared::load(store.clone()).unwrap();
    let on_disk = store.load_config().unwrap().unwrap();
    assert_eq!(on_disk.tenant_id, f.cfg.tenant_id);
    assert_eq!(
        DeviceIdentity::load(&store).unwrap().unwrap().device_key_id,
        f.legacy.device_key_id
    );
    assert!(
        commons_credentials::pending_switch(&store)
            .unwrap()
            .is_none()
    );
    let policy = restarted.policy.lock().unwrap();
    assert_eq!(
        policy.projects[PROJECT]
            .armed_under
            .as_ref()
            .unwrap()
            .identity(),
        IdentityTerm::of_config(&f.cfg)
    );
}

/// The re-baseline has no IPC surface: nothing a shell or the CLI can call
/// re-records a grant's terms, and the migration itself is async-only.
#[test]
fn the_rebaseline_is_not_reachable_over_ipc() {
    use crate::daemon::ipc::METHODS;
    for method in METHODS {
        assert!(
            !method.contains("rebaseline") && !method.contains("grant_terms"),
            "{method}"
        );
    }
    let migration: Vec<_> = METHODS
        .iter()
        .filter(|m| m.contains("legacy_invite"))
        .collect();
    assert_eq!(
        migration,
        [
            &"acknowledge_legacy_invite_migration",
            &"legacy_invite_migrate"
        ]
    );
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = DaemonShared::load(store).unwrap();
    let response = crate::daemon::ipc::handle_request(
        &shared,
        &Request {
            id: 1,
            method: "legacy_invite_migrate".into(),
            params: serde_json::json!({}),
        },
    );
    assert_eq!(
        response.error.map(|e| e.message),
        Some("legacy-migration-requires-async".to_string())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn status_offers_the_move_then_shows_the_notice_until_acknowledged() {
    let f = fixture(LinkMode::Honest, true).await;
    let status = f.shared.status_value();
    assert_eq!(status["legacy_invite_migration"]["offered"], true);
    assert!(status["legacy_invite_migration"]["notice"].is_null());
    f.migrate(&provisioner(&f.ingest)).await.unwrap();
    let status = f.shared.status_value();
    assert_eq!(status["legacy_invite_migration"]["offered"], false);
    assert_eq!(
        status["legacy_invite_migration"]["notice"],
        serde_json::json!({"folders_kept": 1, "automatic_grant_kept": false})
    );
    let rendered = status["legacy_invite_migration"].to_string();
    assert!(!rendered.contains("tenant-"), "{rendered}");
    let response = crate::daemon::ipc::handle_request(
        &f.shared,
        &Request {
            id: 1,
            method: "acknowledge_legacy_invite_migration".into(),
            params: serde_json::json!({}),
        },
    );
    assert!(response.error.is_none());
    assert!(f.shared.status_value()["legacy_invite_migration"]["notice"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn only_a_legacy_invite_identity_is_offered_the_move() {
    let f = fixture(LinkMode::Honest, true).await;
    let mut near = f.cfg.clone();
    near.tenant_id = format!("nearai-{}", "ab".repeat(32));
    f.shared.store.save_config(&near).unwrap();
    let error = f.migrate(&provisioner(&f.ingest)).await.unwrap_err();
    assert_eq!(label(&error), "legacy_migration_not_applicable");
    let mut instance = f.cfg.clone();
    instance.instance_id = "instance-1".into();
    assert!(!offered_for(&instance));
    assert!(offered_for(&f.cfg));
}

#[test]
fn a_pasted_invite_must_be_this_commons_s() {
    let cfg = legacy_config("https://ingest.example", "sha256:aa");
    assert_eq!(
        invite_hash_from_paste("https://issuer.example/onboard#ABCD1234ABCD1234", &cfg).unwrap(),
        invite_subject_hash("ABCD1234ABCD1234")
    );
    assert_eq!(
        invite_hash_from_paste("ABCD1234ABCD1234", &cfg).unwrap(),
        invite_subject_hash("ABCD1234ABCD1234")
    );
    assert_eq!(
        label(&invite_hash_from_paste("https://other.example/onboard#ABCD", &cfg).unwrap_err()),
        "legacy_migration_invite_other_commons"
    );
    assert_eq!(
        label(&invite_hash_from_paste("not a code!", &cfg).unwrap_err()),
        "legacy_migration_invite_invalid"
    );
    // Same derivation as the issuer's `hash_invite_code`.
    assert_eq!(
        invite_subject_hash("INV-PILOT-001"),
        format!(
            "sha256:{}",
            hex::encode(Sha256::digest(b"invite:INV-PILOT-001"))
        )
    );
}

/// With nothing pasted and nothing remembered, the issuer is asked, signed by
/// the legacy key; an issuer older than the route leads to asking the
/// contributor for their invite.
#[tokio::test(flavor = "multi_thread")]
async fn the_issuer_is_asked_first_and_an_old_issuer_falls_back_to_the_paste() {
    use trace_commons_protocol::device_invite_subject::DEVICE_INVITE_SUBJECT_PATH;
    let f = fixture(LinkMode::Honest, true).await;
    let hash = invite_subject_hash("FROMTHEISSUER001");
    let seen = Arc::new(Mutex::new(None::<(String, String)>));
    let seen_route = seen.clone();
    let answer = hash.clone();
    let issuer = Router::new().route(
        DEVICE_INVITE_SUBJECT_PATH,
        post(move |headers: HeaderMap, body: axum::body::Bytes| {
            let seen = seen_route.clone();
            let answer = answer.clone();
            async move {
                let id = headers["x-trace-device-key-id"]
                    .to_str()
                    .unwrap()
                    .to_string();
                let sig = headers["x-trace-device-signature"]
                    .to_str()
                    .unwrap()
                    .to_string();
                *seen.lock().unwrap() = Some((id, sig));
                let _ = body;
                Json(serde_json::json!({ "invite_subject_hash": answer }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, issuer).await.unwrap() });
    let mut cfg = f.cfg.clone();
    cfg.issuer_url = issuer_url.clone();
    assert_eq!(
        resolve_invite_subject(&f.shared.store, &cfg, &f.legacy, None)
            .await
            .unwrap(),
        hash
    );
    assert_eq!(
        seen.lock().unwrap().clone().unwrap().0,
        f.legacy.device_key_id,
        "asked as the legacy device"
    );

    // An issuer without the route.
    let old = Router::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, old).await.unwrap() });
    cfg.issuer_url = old_url;
    assert_eq!(
        label(
            &resolve_invite_subject(&f.shared.store, &cfg, &f.legacy, None)
                .await
                .unwrap_err()
        ),
        "legacy_migration_invite_needed"
    );
    // A hash remembered at enrollment is used before either.
    remember_invite_subject(&f.shared.store, "REMEMBERED000001");
    assert_eq!(
        resolve_invite_subject(&f.shared.store, &cfg, &f.legacy, None)
            .await
            .unwrap(),
        invite_subject_hash("REMEMBERED000001")
    );
}

/// Completing the move revokes the legacy account session on the server,
/// not only locally: the old session can no longer read or withdraw the
/// invite tenant's traces once the device has left it.
#[tokio::test(flavor = "multi_thread")]
async fn completing_the_move_revokes_the_legacy_session_on_the_server() {
    let f = fixture(LinkMode::Honest, true).await;
    let answer = f.migrate(&provisioner(&f.ingest)).await.unwrap();
    assert_eq!(
        *f.ingest.logouts.lock().unwrap(),
        ["Bearer tcn1_legacy".to_string()],
        "the legacy session, and only it, is revoked"
    );
    assert_eq!(answer["legacy_session_revoked"], true);
    let audit = crate::daemon::audit::load(&f.shared.store).unwrap();
    assert!(
        audit
            .iter()
            .any(|e| e.action == "legacy-account-session-revoked")
    );
}

/// A move that does not complete revokes nothing: the legacy identity,
/// session included, stays exactly as it was.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_that_does_not_complete_revokes_nothing() {
    for mode in [LinkMode::Pooled, LinkMode::OtherAccount] {
        let f = fixture(mode, true).await;
        f.migrate(&provisioner(&f.ingest)).await.unwrap_err();
        assert!(f.ingest.logouts.lock().unwrap().is_empty());
        f.assert_legacy_intact();
    }
    let f = fixture(LinkMode::Honest, true).await;
    commons_credentials::fail_switch_after_for_test(&f.shared.store, 3);
    f.migrate(&provisioner(&f.ingest)).await.unwrap_err();
    assert!(f.ingest.logouts.lock().unwrap().is_empty());
    f.assert_legacy_intact();
}

/// A legacy identity with no account session has nothing to revoke, and the
/// move still completes.
#[tokio::test(flavor = "multi_thread")]
async fn a_move_without_a_legacy_session_revokes_nothing_and_completes() {
    let f = fixture(LinkMode::Honest, true).await;
    crate::account_auth::clear_token(&f.shared.store).unwrap();
    let answer = f.migrate(&provisioner(&f.ingest)).await.unwrap();
    assert_eq!(answer["migrated"], true);
    assert_eq!(answer["legacy_session_revoked"], false);
    assert!(f.ingest.logouts.lock().unwrap().is_empty());
}
