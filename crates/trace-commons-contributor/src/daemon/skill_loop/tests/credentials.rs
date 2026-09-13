//! INTEGRATION: verifies credential snapshots and active-operation cancellation.

use std::cell::{Cell, RefCell};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;

struct DropSignal(Arc<AtomicBool>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[tokio::test]
async fn credential_snapshot_reconciles_a_change_after_the_first_revision_read() {
    let revision = Cell::new(0_u64);
    let loaded_key = RefCell::new(Some("prior-fixture-key".to_string()));
    let disk_key = "current-fixture-key".to_string();
    let key_reads = Cell::new(0_usize);
    let reconcile_calls = Cell::new(0_usize);
    let absorbed_revision = Cell::new(0_u64);

    let snapshot = consistent_credential_snapshot(
        || revision.get(),
        || {
            reconcile_calls.set(reconcile_calls.get() + 1);
            if revision.get() == 1 {
                loaded_key.replace(Some(disk_key.clone()));
            }
            absorbed_revision.set(revision.get());
            std::future::ready(())
        },
        || absorbed_revision.get(),
        || {
            key_reads.set(key_reads.get() + 1);
            if key_reads.get() == 1 {
                revision.set(1);
            }
            loaded_key.borrow().clone()
        },
    )
    .await
    .expect("stable credential snapshot");

    assert_eq!(snapshot.0, 1);
    assert_eq!(snapshot.1.as_deref(), Some("current-fixture-key"));
    assert_eq!(reconcile_calls.get(), 2);
}

#[tokio::test]
async fn credential_snapshot_fails_closed_after_bounded_churn() {
    let revision = Cell::new(0_u64);
    let reconcile_calls = Cell::new(0_usize);

    let snapshot = consistent_credential_snapshot(
        || {
            let value = revision.get();
            revision.set(value + 1);
            value
        },
        || {
            reconcile_calls.set(reconcile_calls.get() + 1);
            std::future::ready(())
        },
        || revision.get(),
        || Some("fixture-key".to_string()),
    )
    .await;

    assert!(snapshot.is_none());
    assert_eq!(reconcile_calls.get(), CREDENTIAL_SNAPSHOT_ATTEMPTS);
}

#[tokio::test]
async fn credential_change_cancels_and_drops_the_active_operation() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().to_path_buf();
    let observed = crate::daemon::nearai_credential::ceremony::change_count(&path);
    let dropped = Arc::new(AtomicBool::new(false));
    let operation_drop = Arc::clone(&dropped);
    let operation = async move {
        let _signal = DropSignal(operation_drop);
        std::future::pending::<()>().await;
    };
    let task = tokio::spawn(run_until_credential_change(
        path.clone(),
        observed,
        operation,
    ));
    tokio::task::yield_now().await;
    crate::daemon::nearai_credential::ceremony::record_change_for_test(&path);

    let outcome = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("credential watcher completion")
        .expect("credential watcher task");
    assert!(outcome.is_none());
    assert!(dropped.load(Ordering::Acquire));
}
