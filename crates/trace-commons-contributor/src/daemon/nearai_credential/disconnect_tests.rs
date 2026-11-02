use super::test_support::{
    ACCESS, KEY, REFRESH, Seen, assert_cleared, assert_revoked_with_old_session, seed, stub,
};
use super::*;
use crate::config::tests_support::temp_store;
use axum::http::StatusCode;
use std::time::Duration;

/// The store-level clear (CLI `logout`'s) leaves nothing behind, and the
/// revoke it answers goes out with the session it cleared.
#[tokio::test]
async fn the_store_clear_removes_everything_and_revokes_with_the_old_session() {
    let (_dir, store) = temp_store();
    let reference = seed(&store);
    let (origin, seen) = stub(StatusCode::OK, StatusCode::NO_CONTENT).await;

    let cleared = clear_store(&store).unwrap();
    cleared.cleaned.unwrap();
    let revocation = cleared.revocation;
    // Cleared before anything was sent.
    assert_cleared(&store, &reference);
    assert!(seen.lock().unwrap().is_empty());

    let api = CloudApi::for_test(&origin).unwrap();
    assert_eq!(
        revocation.expect("a key to revoke").run(Some(&api)).await,
        Some(true)
    );
    assert_revoked_with_old_session(&seen);
    // The rotated refresh token the exchange answered is never stored.
    assert_cleared(&store, &reference);
}

/// A revoke NEAR AI refuses, or a session it no longer accepts, leaves the
/// clear standing and answers `false`.
#[tokio::test]
async fn a_failed_revoke_still_clears() {
    for (refresh, revoke) in [
        (StatusCode::OK, StatusCode::INTERNAL_SERVER_ERROR),
        (StatusCode::UNAUTHORIZED, StatusCode::NO_CONTENT),
    ] {
        let (_dir, store) = temp_store();
        let reference = seed(&store);
        let (origin, _seen) = stub(refresh, revoke).await;
        let cleared = clear_store(&store).unwrap();
        cleared.cleaned.unwrap();
        let revocation = cleared.revocation;
        let api = CloudApi::for_test(&origin).unwrap();
        assert_eq!(
            revocation.unwrap().run(Some(&api)).await,
            Some(false),
            "{refresh} {revoke}"
        );
        assert_cleared(&store, &reference);
    }
}

/// No client, or a NEAR AI that cannot be reached: still cleared.
#[tokio::test]
async fn an_impossible_revoke_still_clears() {
    let (_dir, store) = temp_store();
    let reference = seed(&store);
    let cleared = clear_store(&store).unwrap();
    cleared.cleaned.unwrap();
    let revocation = cleared.revocation;
    assert_eq!(revocation.unwrap().run(None).await, Some(false));
    assert_cleared(&store, &reference);

    let (_dir, store) = temp_store();
    let reference = seed(&store);
    let revocation = clear_store(&store).unwrap().revocation;
    // Nothing listens here.
    let unreachable = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    let api = CloudApi::for_test(&format!("http://{unreachable}")).unwrap();
    assert_eq!(revocation.unwrap().run(Some(&api)).await, Some(false));
    assert_cleared(&store, &reference);
}

/// Nothing stored, nothing to revoke, nothing sent.
#[tokio::test]
async fn nothing_stored_revokes_nothing() {
    let (_dir, store) = temp_store();
    let cleared = clear_store(&store).unwrap();
    cleared.cleaned.unwrap();
    let revocation = cleared.revocation;
    assert!(revocation.is_none());
}

/// Records every event's target and fields.
#[derive(Clone, Default)]
struct Capture(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

struct Fields(String);

impl tracing::field::Visit for Fields {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        use std::fmt::Write;
        let _ = write!(self.0, " {}={value:?}", field.name());
    }
}

impl tracing::Subscriber for Capture {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        let mut fields = Fields(span.metadata().name().to_string());
        span.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, values: &tracing::span::Record<'_>) {
        let mut fields = Fields(String::new());
        values.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = Fields(event.metadata().target().to_string());
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// A failed revoke writes its label and nothing that names the key, the
/// session, the access token or the key's ids.
#[test]
fn a_failed_revoke_logs_a_label_and_no_secret() {
    let capture = Capture::default();
    // A dedicated thread with its own runtime: tracing's callsite interest
    // cache is process-wide, see `mission_matching`'s capture test.
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                tracing::subscriber::with_default(capture.clone(), || {
                    let _second =
                        tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
                    tracing::callsite::rebuild_interest_cache();
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(async {
                            let (_dir, store) = temp_store();
                            let reference = seed(&store);
                            let (origin, seen): (String, Seen) =
                                stub(StatusCode::OK, StatusCode::INTERNAL_SERVER_ERROR).await;
                            let revocation = clear_store(&store).unwrap().revocation;
                            let api = CloudApi::for_test(&origin).unwrap();
                            assert_eq!(revocation.unwrap().run(Some(&api)).await, Some(false));
                            assert!(!seen.lock().unwrap().is_empty());
                            assert_cleared(&store, &reference);
                        });
                })
            })
            .join()
            .unwrap()
    });
    let lines = capture.0.lock().unwrap().clone();
    assert!(
        lines.iter().any(|line| line.contains(LABEL_NOT_REVOKED)),
        "{lines:?}"
    );
    // This crate's lines. The HTTP client's and the stub server's own debug
    // lines are theirs, and the daemon's subscriber never writes their
    // fields (`diagnostics_log::MetadataOnlySubscriber`).
    let ours = lines.iter().filter(|line| {
        ![
            "reqwest", "hyper", "h2", "axum", "tower", "tokio", "mio", "rustls",
        ]
        .iter()
        .any(|external| line.starts_with(external))
    });
    for line in ours {
        for secret in [
            KEY,
            REFRESH,
            ACCESS,
            "rt_rotated-secret",
            "key-1",
            "ws-1",
            "key-old",
        ] {
            assert!(!line.contains(secret), "{secret} in {line}");
        }
    }
}

/// A daemon holding a connected NEAR AI half, with its revokes pointed at a
/// stub NEAR AI.
async fn connected_daemon(
    revoke: StatusCode,
) -> (
    tempfile::TempDir,
    crate::daemon::ipc::DaemonShared,
    crate::daemon::credential_store::CredentialReference,
    Seen,
) {
    let (dir, store) = temp_store();
    let reference = seed(&store);
    let (origin, seen) = stub(StatusCode::OK, revoke).await;
    test_api::install(&store, &origin);
    let shared = crate::daemon::ipc::DaemonShared::load(store).unwrap();
    assert!(shared.settings.lock().unwrap().near_ai_inference.is_some());
    (dir, shared, reference, seen)
}

fn request(method: &str) -> crate::daemon::ipc::Request {
    crate::daemon::ipc::Request {
        id: 1,
        method: method.into(),
        params: serde_json::json!({}),
    }
}

/// The running daemon holds none of the NEAR AI half either.
fn assert_memory_cleared(shared: &crate::daemon::ipc::DaemonShared) {
    let memory = shared.settings.lock().unwrap();
    assert!(memory.near_ai_inference.is_none());
    assert!(memory.near_ai_session.is_none());
    assert!(memory.cloud_credentials.is_none());
}

/// Forget clears, and now revokes the key with the session it cleared.
#[tokio::test]
async fn forget_clears_and_revokes() {
    let (_dir, shared, reference, seen) = connected_daemon(StatusCode::NO_CONTENT).await;
    let response =
        crate::daemon::ipc::handle_request_async(&shared, &request("near_ai_credential_forget"))
            .await;
    let body = response.result.expect("forget answers");
    assert_eq!(body["removed"], true);
    assert_eq!(body["revoked"], true);
    assert_cleared(&shared.store, &reference);
    assert_memory_cleared(&shared);
    assert_revoked_with_old_session(&seen);
}

/// A revoke NEAR AI refuses does not undo or fail the forget.
#[tokio::test]
async fn forget_with_a_failed_revoke_still_clears() {
    let (_dir, shared, reference, seen) = connected_daemon(StatusCode::INTERNAL_SERVER_ERROR).await;
    let response =
        crate::daemon::ipc::handle_request_async(&shared, &request("near_ai_credential_forget"))
            .await;
    let body = response.result.expect("forget answers");
    assert_eq!(body["removed"], true);
    assert_eq!(body["revoked"], false);
    assert_cleared(&shared.store, &reference);
    assert_memory_cleared(&shared);
    assert!(!seen.lock().unwrap().is_empty());
}

/// Unenroll clears the NEAR AI half too, enrolled or not, and revokes.
#[tokio::test]
async fn unenroll_clears_and_revokes() {
    let (_dir, shared, reference, seen) = connected_daemon(StatusCode::NO_CONTENT).await;
    crate::identity::DeviceIdentity::load_or_generate(&shared.store).unwrap();
    shared
        .store
        .save_config(&crate::commands::unenrolled_preview_config())
        .unwrap();
    let response = crate::daemon::ipc::handle_request_async(&shared, &request("unenroll")).await;
    let body = response.result.expect("unenroll answers");
    assert_eq!(body["removed"], true);
    assert_eq!(body["near_ai_revoked"], true);
    assert!(shared.store.load_config().unwrap().is_none());
    assert_cleared(&shared.store, &reference);
    assert_memory_cleared(&shared);
    assert_revoked_with_old_session(&seen);
}

/// A device that holds only the NEAR AI sign-in is cleared by unenroll too.
#[tokio::test]
async fn unenroll_without_an_enrollment_still_clears_near_ai() {
    let (_dir, shared, reference, seen) = connected_daemon(StatusCode::NO_CONTENT).await;
    let response = crate::daemon::ipc::handle_request_async(&shared, &request("unenroll")).await;
    assert_eq!(response.result.expect("unenroll answers")["removed"], false);
    assert_cleared(&shared.store, &reference);
    assert_memory_cleared(&shared);
    assert_revoked_with_old_session(&seen);
}

/// Native sign-out clears the NEAR AI half with the Commons session.
#[tokio::test]
async fn native_sign_out_clears_and_revokes() {
    let (_dir, shared, reference, seen) = connected_daemon(StatusCode::NO_CONTENT).await;
    let response =
        crate::daemon::ipc::handle_request_async(&shared, &request("account_sign_out")).await;
    assert_eq!(
        response.result.expect("sign-out answers")["signed_out"],
        true
    );
    assert_cleared(&shared.store, &reference);
    assert_memory_cleared(&shared);
    assert_revoked_with_old_session(&seen);
}

/// Native sign-out with a revoke NEAR AI refuses still signs out.
#[tokio::test]
async fn native_sign_out_with_a_failed_revoke_still_clears() {
    let (_dir, shared, reference, _seen) =
        connected_daemon(StatusCode::INTERNAL_SERVER_ERROR).await;
    let response =
        crate::daemon::ipc::handle_request_async(&shared, &request("account_sign_out")).await;
    let body = response.result.expect("sign-out answers");
    assert_eq!(body["signed_out"], true);
    assert_eq!(body["near_ai_revoked"], false);
    assert_cleared(&shared.store, &reference);
    assert_memory_cleared(&shared);
}

/// CLI `logout` clears the NEAR AI half through its journal before the wipe,
/// and revokes with the old session.
#[tokio::test]
async fn cli_logout_clears_and_revokes() {
    let (_dir, store) = temp_store();
    let reference = seed(&store);
    let (origin, seen) = stub(StatusCode::OK, StatusCode::NO_CONTENT).await;
    let api = CloudApi::for_test(&origin).unwrap();
    let mut out = Vec::new();
    crate::commands::logout_with(&store, true, &mut "".as_bytes(), &mut out, Some(&api))
        .await
        .unwrap();
    assert_cleared(&store, &reference);
    assert_revoked_with_old_session(&seen);
}

/// CLI `logout` with a revoke NEAR AI refuses still logs out.
#[tokio::test]
async fn cli_logout_with_a_failed_revoke_still_clears() {
    let (_dir, store) = temp_store();
    let reference = seed(&store);
    let (origin, _seen) = stub(StatusCode::OK, StatusCode::INTERNAL_SERVER_ERROR).await;
    let api = CloudApi::for_test(&origin).unwrap();
    let mut out = Vec::new();
    crate::commands::logout_with(&store, true, &mut "".as_bytes(), &mut out, Some(&api))
        .await
        .unwrap();
    assert_cleared(&store, &reference);
    assert!(String::from_utf8(out).unwrap().contains("logged out"));
}

/// CLI `account logout` with no daemon running clears the NEAR AI half and
/// revokes with the old session.
#[tokio::test]
async fn cli_account_logout_without_a_daemon_clears_and_revokes() {
    let (_dir, store) = temp_store();
    let reference = seed(&store);
    let cfg = crate::commands::unenrolled_preview_config();
    store.save_config(&cfg).unwrap();
    let (origin, seen) = stub(StatusCode::OK, StatusCode::NO_CONTENT).await;
    let api = CloudApi::for_test(&origin).unwrap();
    let outcome = crate::commands::account_logout_local(&store, &cfg, Some(&api)).await;
    outcome.result.unwrap();
    assert_eq!(outcome.near_ai_revoked, Some(true));
    assert_cleared(&store, &reference);
    assert_revoked_with_old_session(&seen);
}

/// CLI `logout` says so when the revoke failed, in the owner's words.
#[tokio::test]
async fn cli_logout_names_a_failed_revoke() {
    let (_dir, store) = temp_store();
    seed(&store);
    let (origin, _seen) = stub(StatusCode::OK, StatusCode::INTERNAL_SERVER_ERROR).await;
    let api = CloudApi::for_test(&origin).unwrap();
    let mut out = Vec::new();
    crate::commands::logout_with(&store, true, &mut "".as_bytes(), &mut out, Some(&api))
        .await
        .unwrap();
    assert!(
        String::from_utf8(out)
            .unwrap()
            .contains("Couldn't revoke the key. Delete it in your NEAR AI account."),
    );
}

/// A stored key whose secret cannot be read (a locked or declined keychain)
/// is cleared, and answers "not revoked", never "nothing was stored".
#[tokio::test]
async fn a_stored_key_whose_secret_cannot_be_read_answers_not_revoked() {
    let (_dir, store) = temp_store();
    let reference = seed(&store);
    crate::daemon::cloud_credential_test_support::backend(store.dir())
        .unwrap()
        .delete(&reference)
        .unwrap();
    let (origin, seen) = stub(StatusCode::OK, StatusCode::NO_CONTENT).await;
    let cleared = clear_store(&store).unwrap();
    let api = CloudApi::for_test(&origin).unwrap();
    assert_eq!(
        cleared
            .revocation
            .expect("a stored key")
            .run(Some(&api))
            .await,
        Some(false)
    );
    // No session could be read, so nothing was sent.
    assert!(seen.lock().unwrap().is_empty());
    assert_cleared(&store, &reference);
}

/// The same in the daemon: a running daemon that cannot vouch for its copy
/// still clears, and forget answers `revoked: false`.
#[tokio::test]
async fn forget_with_an_unreadable_copy_answers_not_revoked() {
    let (_dir, shared, reference, seen) = connected_daemon(StatusCode::NO_CONTENT).await;
    shared.settings.lock().unwrap().cloud_storage_unavailable = true;
    let response =
        crate::daemon::ipc::handle_request_async(&shared, &request("near_ai_credential_forget"))
            .await;
    let body = response.result.expect("forget answers");
    assert_eq!(body["removed"], true);
    assert_eq!(body["revoked"], false);
    assert_cleared(&shared.store, &reference);
    assert!(seen.lock().unwrap().is_empty());
}

/// The revoke's exchange waits for an exchange already in flight: NEAR AI
/// rotates the refresh token on use, and two uses at once would present a
/// rotated token.
#[tokio::test]
async fn the_revoke_exchange_takes_the_refresh_lock() {
    let (_dir, store) = temp_store();
    seed(&store);
    let (origin, seen) = stub(StatusCode::OK, StatusCode::NO_CONTENT).await;
    let revocation = clear_store(&store).unwrap().revocation.unwrap();
    let locks = super::super::session::coordination(store.dir()).unwrap();
    let held = locks.refresh_lock().await;
    let api = CloudApi::for_test(&origin).unwrap();
    let task = tokio::spawn(async move { revocation.run(Some(&api)).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        seen.lock().unwrap().is_empty(),
        "exchanged while another exchange held the lock"
    );
    drop(held);
    assert_eq!(task.await.unwrap(), Some(true));
    assert_revoked_with_old_session(&seen);
}

/// Native sign-out whose NEAR AI half cannot be cleared still signs out of
/// the account, and says the NEAR AI half on its own.
#[tokio::test]
async fn native_sign_out_reports_a_near_ai_clear_failure_separately() {
    let (_dir, shared, _reference, _seen) = connected_daemon(StatusCode::NO_CONTENT).await;
    shared
        .store
        .write_daemon_file(crate::config::DAEMON_SETTINGS_FILE, b"not json")
        .unwrap();
    let response =
        crate::daemon::ipc::handle_request_async(&shared, &request("account_sign_out")).await;
    let body = response.result.expect("sign-out answers");
    assert_eq!(body["signed_out"], true);
    assert_eq!(body["near_ai_cleared"], false);
    assert!(body["near_ai_revoked"].is_null(), "{body}");
    // No account session was stored, so there was nothing to revoke there.
    assert!(body["server_revoked"].is_null(), "{body}");
}

fn outcome(response: crate::daemon::ipc::Response) -> crate::commands::AccountLogoutOutcome {
    match crate::commands::daemon_sign_out_outcome(response) {
        crate::commands::DaemonSignOut::Answered(outcome) => outcome,
        crate::commands::DaemonSignOut::DryRun => panic!("read as a dry run"),
    }
}

/// `account logout` through a daemon fails as the local path does, and never
/// reads an old daemon's silence as a cleared NEAR AI sign-in.
#[test]
fn account_logout_reads_the_daemons_answer_strictly() {
    use crate::daemon::ipc::Response;
    use serde_json::json;
    let ok = outcome(Response::ok(
        1,
        json!({"signed_out": true, "server_revoked": true, "near_ai_cleared": true, "near_ai_revoked": false}),
    ));
    assert!(ok.result.is_ok());
    assert_eq!(ok.near_ai_revoked, Some(false));

    let server = outcome(Response::ok(
        1,
        json!({"signed_out": true, "server_revoked": false, "near_ai_cleared": true, "near_ai_revoked": true}),
    ));
    assert_eq!(
        server.result.unwrap_err().to_string(),
        "revoking the account session"
    );
    assert_eq!(server.near_ai_revoked, Some(true));

    let not_cleared = outcome(Response::ok(
        1,
        json!({"signed_out": true, "server_revoked": null, "near_ai_cleared": false, "near_ai_revoked": null}),
    ));
    assert!(not_cleared.result.is_err());

    // An older daemon: no NEAR AI fields at all.
    let old = outcome(Response::ok(1, json!({"signed_out": true})));
    assert!(old.result.is_err());

    assert!(matches!(
        crate::commands::daemon_sign_out_outcome(Response::err(
            1,
            crate::daemon::ipc::ERR_BAD_PARAMS,
            crate::daemon::ipc::ERR_DEV_DRY_RUN,
        )),
        crate::commands::DaemonSignOut::DryRun
    ));
}

/// A pending key tagged with another NEAR AI identity is never sent with
/// this session; an untagged one is (#1842's rule, spec B8).
#[tokio::test]
async fn another_identitys_pending_key_is_not_sent() {
    let (_dir, store) = temp_store();
    seed(&store);
    super::super::spend_org::record_identity(&store, "key-1", "sha256:mine");
    let mut theirs = super::super::spend_org::PendingRevocation::new("ws-9", "key-theirs");
    theirs.identity = Some("sha256:theirs".into());
    super::super::spend_org::record_pending(&store, &[theirs]).unwrap();
    let (origin, seen) = stub(StatusCode::OK, StatusCode::NO_CONTENT).await;
    let revocation = clear_store(&store).unwrap().revocation.unwrap();
    let api = CloudApi::for_test(&origin).unwrap();
    assert_eq!(revocation.run(Some(&api)).await, Some(true));
    assert_revoked_with_old_session(&seen);
    assert!(
        !seen
            .lock()
            .unwrap()
            .iter()
            .any(|line| line.contains("key-theirs")),
        "{seen:?}"
    );
}
