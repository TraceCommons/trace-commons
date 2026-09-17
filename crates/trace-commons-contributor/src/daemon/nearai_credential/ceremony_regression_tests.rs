// INTEGRATION: register as a child of ceremony.rs with
// #[cfg(test)] #[path = "ceremony_regression_tests.rs"] mod regression_tests;
// Requires the current persist_attempt guard and serialized forget implementation.
// Tests hold a synthetic completed mint at a channel barrier; no browser, Cloud
// request, real credential, or runtime background ceremony is started.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use crate::config::{ConfigStore, tests_support::temp_store};
use crate::daemon::cloud_credential_test_support::{MemoryBackend, install_backend};
use crate::daemon::credential_store::{CredentialError, CredentialReference, SecretBackend};
use crate::daemon::nearai_credential::api::MintedKey;
use crate::daemon::nearai_credential::ceremony::{
    Attempt, Status, attempt_status, attempts, begin, browser_urls_served, cancel, change_count,
    changes, forget, persist, persist_attempt,
};
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};

const DEADLINE: Duration = Duration::from_secs(10);

struct Snapshot {
    inference: Option<NearAiInferenceCredential>,
    session: Option<NearAiSession>,
    changes: u64,
}

/// Wraps the shared in-memory double so this file can force the one outcome
/// production code distinguishes -- `Unentitled` -- without adding a flag to
/// `MemoryBackend` itself, which every other test in the suite also shares.
#[derive(Default)]
struct FailingBackend {
    inner: MemoryBackend,
    fail_next: AtomicBool,
}

impl FailingBackend {
    fn fail_next_with_unentitled(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }
}

impl SecretBackend for FailingBackend {
    fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(CredentialError::Unentitled);
        }
        self.inner.read(reference)
    }
    fn write(&self, reference: &CredentialReference, bytes: &[u8]) -> Result<(), CredentialError> {
        self.inner.write(reference, bytes)
    }
    fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
        self.inner.delete(reference)
    }
}

struct Fixture {
    store: ConfigStore,
    backend: Arc<FailingBackend>,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let (directory, store) = temp_store();
        let backend = Arc::new(FailingBackend::default());
        install_backend(&store, backend.clone());
        let mut settings =
            DaemonSettings::load_with_cloud_credentials(&store).expect("load synthetic settings");
        settings.max_uploads_per_day = 7;
        settings
            .save_for_test(&store)
            .expect("save synthetic preference");
        // Seed the already-connected account, not the attempted publication.
        // Every publication under test below goes through persist_attempt.
        persist(
            store.dir(),
            minted("previous"),
            refresh("previous"),
            "Mozilla/5.0 Test".into(),
        )
        .expect("seed previous synthetic connection");
        Self {
            store,
            backend,
            _directory: directory,
        }
    }

    fn browser_urls_served(&self) -> u64 {
        browser_urls_served(self.store.dir())
    }

    fn waiting(&self, id: &str) {
        attempts().lock().expect("attempt registry").insert(
            self.store.dir().to_path_buf(),
            Attempt {
                state: Status {
                    attempt_id: id.into(),
                    status: "waiting_for_browser",
                },
                abort: None,
            },
        );
    }

    fn snapshot(&self) -> Snapshot {
        let settings = DaemonSettings::load_with_cloud_credentials(&self.store)
            .expect("load persisted credentials");
        Snapshot {
            inference: settings.near_ai_inference,
            session: settings.near_ai_session,
            changes: change_count(self.store.dir()),
        }
    }

    fn assert_unchanged(&self, before: &Snapshot) {
        let after = self.snapshot();
        assert_eq!(after.inference, before.inference, "inference key changed");
        assert_eq!(after.session, before.session, "retained session changed");
        assert_eq!(after.changes, before.changes, "change counter advanced");
    }

    /// Model an old Cloud response finishing only after the competing local
    /// action. Bounded channels make ordering deterministic without sleeps.
    fn finish_after<T>(
        &self,
        attempt_id: &str,
        before_release: impl FnOnce() -> T,
    ) -> (anyhow::Result<()>, T) {
        let directory = self.store.dir().to_path_buf();
        let attempt_id = attempt_id.to_owned();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        std::thread::scope(|scope| {
            let publication = scope.spawn(move || {
                let key = minted("late");
                let session = refresh("late");
                ready_tx.send(()).expect("signal completed synthetic mint");
                release_rx
                    .recv_timeout(DEADLINE)
                    .expect("release late publication");
                persist_attempt(
                    &directory,
                    &attempt_id,
                    key,
                    session,
                    None,
                    "Mozilla/5.0 Test".into(),
                )
            });
            ready_rx
                .recv_timeout(DEADLINE)
                .expect("synthetic mint reached publication barrier");
            let expected = before_release();
            release_tx.send(()).expect("allow late publication");
            (
                publication.join().expect("publication worker completed"),
                expected,
            )
        })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Remove only this fixture's registry slots, including during unwind.
        // No worker remains after finish_after's scoped join.
        if let Ok(mut entries) = attempts().lock() {
            entries.remove(self.store.dir());
        }
        if let Ok(mut entries) = changes().lock() {
            entries.remove(self.store.dir());
        }
    }
}

fn minted(account: &str) -> MintedKey {
    MintedKey {
        key: format!("synthetic-key-{account}"),
        key_id: format!("key-{account}"),
        key_prefix: "synthetic".into(),
        organization_id: format!("organization-{account}"),
        workspace_id: format!("workspace-{account}"),
    }
}

fn refresh(account: &str) -> String {
    format!("synthetic-refresh-{account}")
}

fn assert_cancelled(result: anyhow::Result<()>) {
    assert_eq!(
        result
            .expect_err("late credentials must be refused")
            .to_string(),
        "near_ai_credential_cancelled"
    );
}

#[test]
fn the_current_waiting_attempt_publishes_both_credentials_once() {
    let fixture = Fixture::new();
    fixture.waiting("current");
    let before = fixture.snapshot();
    let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
    persist_attempt(
        fixture.store.dir(),
        "current",
        minted("current"),
        refresh("current"),
        Some(expires_at),
        "Mozilla/5.0 Test".into(),
    )
    .expect("the current waiting attempt must publish");

    let after = fixture.snapshot();
    let key = after.inference.expect("published inference key");
    assert_eq!(key.key, "synthetic-key-current");
    assert_eq!(key.organization_id, "organization-current");
    assert_eq!(key.workspace_id, "workspace-current");
    let session = after.session.expect("published retained session");
    assert_eq!(session.refresh_token, refresh("current"));
    assert_eq!(session.refresh_token_expires_at, Some(expires_at));
    assert_eq!(after.changes, before.changes + 1);
    assert_eq!(
        DaemonSettings::load_with_cloud_credentials(&fixture.store)
            .expect("reload persisted preference")
            .max_uploads_per_day,
        7
    );
}

#[test]
fn a_late_mint_after_cancel_preserves_both_credentials_and_the_change_count() {
    let fixture = Fixture::new();
    fixture.waiting("cancelled");
    let before = fixture.snapshot();
    let (result, ()) = fixture.finish_after("cancelled", || {
        let status =
            cancel(fixture.store.dir(), Some("cancelled")).expect("cancel current attempt");
        assert_eq!(status.status, "cancelled");
        fixture.assert_unchanged(&before);
    });
    assert_cancelled(result);
    fixture.assert_unchanged(&before);
}

#[test]
fn a_late_mint_after_forget_cannot_restore_either_credential_or_signal_a_change() {
    let fixture = Fixture::new();
    fixture.waiting("forgotten");
    let before = fixture.snapshot();
    let (result, forgotten) = fixture.finish_after("forgotten", || {
        assert!(forget(&fixture.store).expect("forget synthetic connection"));
        assert_eq!(attempt_status(fixture.store.dir()), Some("cancelled"));
        let forgotten = fixture.snapshot();
        assert!(forgotten.inference.is_none());
        assert!(forgotten.session.is_none());
        assert_eq!(forgotten.changes, before.changes + 1);
        forgotten
    });
    assert_cancelled(result);
    fixture.assert_unchanged(&forgotten);
}

#[test]
fn a_late_mint_cannot_replace_a_new_waiting_attempts_credentials() {
    let fixture = Fixture::new();
    fixture.waiting("replaced");
    let (result, replacement) = fixture.finish_after("replaced", || {
        cancel(fixture.store.dir(), Some("replaced")).expect("retire old attempt");
        fixture.waiting("replacement");
        persist_attempt(
            fixture.store.dir(),
            "replacement",
            minted("replacement"),
            refresh("replacement"),
            None,
            "Mozilla/5.0 Test".into(),
        )
        .expect("publish replacement account");
        // Keep the new entry waiting: only its different ID can reject the old
        // response, so checking the status alone cannot make this test pass.
        assert_eq!(
            attempt_status(fixture.store.dir()),
            Some("waiting_for_browser")
        );
        fixture.snapshot()
    });
    assert_cancelled(result);
    fixture.assert_unchanged(&replacement);
}

/// A ceremony that cannot store its result must say so before it opens a
/// browser. The contributor authenticates at NEAR AI, which mints a real
/// session; failing after that point spends a sign-in and strands a session
/// at the service that this machine will never hold.
#[tokio::test]
async fn an_unstorable_ceremony_refuses_before_opening_a_browser() {
    let fixture = Fixture::new();
    fixture.backend.fail_next_with_unentitled();
    let error = begin(&fixture.store, "github").await.unwrap_err();
    assert_eq!(error.to_string(), "near_ai_credential_storage_unentitled");
    assert_eq!(
        fixture.browser_urls_served(),
        0,
        "no browser URL was minted"
    );
}
