//! Files and the system credential store may block independently of HTTP.
//! At most eight reads run across all backends. Their permits belong to the
//! blocking jobs, so cancelling an async caller cannot free a slot while its
//! credential process is still running.

use std::sync::{Arc, OnceLock};

const MAX_READS: usize = 8;

pub(crate) async fn read<T: Send + 'static>(
    resolve: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    static SLOTS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let slots = Arc::clone(SLOTS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_READS))));
    read_with_slots(slots, resolve).await
}

async fn read_with_slots<T: Send + 'static>(
    slots: Arc<tokio::sync::Semaphore>,
    resolve: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let permit = slots
        .acquire_owned()
        .await
        .expect("read semaphore is never closed");
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        resolve()
    })
    .await
    .map_err(|_| {
        // A panic payload can contain a credential, so never render JoinError.
        "reading the stored login failed; run `ironwire doctor` and log in again".to_string()
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelling_readers_keeps_their_running_credential_jobs_bounded() {
        let slots = Arc::new(tokio::sync::Semaphore::new(MAX_READS));
        let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let maximum = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (started, mut notifications) = tokio::sync::mpsc::unbounded_channel();
        let mut releases = Vec::new();
        let mut readers = Vec::new();
        for _ in 0..MAX_READS {
            let (release, held) = std::sync::mpsc::channel();
            releases.push(release);
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            let started = started.clone();
            readers.push(tokio::spawn(read_with_slots(
                Arc::clone(&slots),
                move || {
                    use std::sync::atomic::Ordering;
                    let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(count, Ordering::SeqCst);
                    started.send(()).expect("notification");
                    let _ = held.recv();
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                },
            )));
        }
        for _ in 0..MAX_READS {
            notifications.recv().await.expect("reader started");
        }
        for reader in readers {
            reader.abort();
            assert!(reader.await.expect_err("caller aborted").is_cancelled());
        }
        let waiting = tokio::spawn(read_with_slots(slots, {
            let active = Arc::clone(&active);
            let maximum = Arc::clone(&maximum);
            move || {
                use std::sync::atomic::Ordering;
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                maximum.fetch_max(count, Ordering::SeqCst);
                started.send(()).expect("notification");
                active.fetch_sub(1, Ordering::SeqCst);
                Ok(())
            }
        }));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), notifications.recv())
                .await
                .is_err(),
            "cancellation freed an active job's slot"
        );
        for release in releases {
            release.send(()).expect("release reader");
        }
        waiting
            .await
            .expect("waiting caller")
            .expect("waiting read");
        assert!(maximum.load(std::sync::atomic::Ordering::SeqCst) <= MAX_READS);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_slow_login_read_leaves_the_async_runtime_free_to_serve_other_work() {
        let (started, ready) = tokio::sync::oneshot::channel();
        let reading = tokio::spawn(super::read(move || {
            started.send(()).expect("reader started");
            std::thread::sleep(std::time::Duration::from_millis(150));
            Ok(())
        }));
        ready.await.expect("reader running");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(
            !reading.is_finished(),
            "runtime was blocked by the login read"
        );
        reading.await.expect("task").expect("read");
    }

    #[tokio::test]
    async fn a_login_read_panic_is_reported_without_its_payload() {
        let result = super::read::<()>(|| panic!("inert-test-secret")).await;
        let error = result.expect_err("read refused");
        assert!(!error.contains("inert-test-secret"));
        assert!(error.contains("ironwire doctor"));
    }
}
