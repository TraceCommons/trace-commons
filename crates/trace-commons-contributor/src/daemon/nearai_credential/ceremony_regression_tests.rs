// INTEGRATION: register as a child of ceremony.rs with
// #[cfg(test)] #[path = "ceremony_regression_tests.rs"] mod regression_tests;
// Requires the current persist_attempt guard and serialized forget implementation.
// Tests hold a synthetic completed mint at a channel barrier; no browser, Cloud
// request, real credential, or runtime background ceremony is started.

use std::sync::mpsc;
use std::time::Duration;

use crate::config::{ConfigStore, tests_support::temp_store};
use crate::daemon::nearai_credential::api::MintedKey;
use crate::daemon::nearai_credential::ceremony::{
    Attempt, Status, attempt_status, attempts, cancel, change_count, changes, forget, persist,
    persist_attempt,
};
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};

const DEADLINE: Duration = Duration::from_secs(10);

struct Snapshot {
    inference: Option<NearAiInferenceCredential>,
    session: Option<NearAiSession>,
    changes: u64,
}

struct Fixture {
    store: ConfigStore,
    _directory: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let (directory, store) = temp_store();
        let mut settings = DaemonSettings::load(&store).expect("load synthetic settings");
        settings.max_uploads_per_day = 7;
        settings.save(&store).expect("save synthetic preference");
        // Seed the already-connected account, not the attempted publication.
        // Every publication under test below goes through persist_attempt.
        persist(store.dir(), minted("previous"), refresh("previous"))
            .expect("seed previous synthetic connection");
        Self {
            store,
            _directory: directory,
        }
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
        let settings = DaemonSettings::load(&self.store).expect("load persisted credentials");
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
                persist_attempt(&directory, &attempt_id, key, session)
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
    persist_attempt(
        fixture.store.dir(),
        "current",
        minted("current"),
        refresh("current"),
    )
    .expect("the current waiting attempt must publish");

    let after = fixture.snapshot();
    let key = after.inference.expect("published inference key");
    assert_eq!(key.key, "synthetic-key-current");
    assert_eq!(key.organization_id, "organization-current");
    assert_eq!(key.workspace_id, "workspace-current");
    assert_eq!(
        after
            .session
            .expect("published retained session")
            .refresh_token,
        refresh("current")
    );
    assert_eq!(after.changes, before.changes + 1);
    assert_eq!(
        DaemonSettings::load(&fixture.store)
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
