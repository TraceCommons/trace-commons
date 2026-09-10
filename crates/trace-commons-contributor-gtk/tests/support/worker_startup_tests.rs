//! Startup readiness yields while the backend waits, and failures allow retry.

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crate::backend::Backend;
use crate::worker::Worker;

const DEADLINE: Duration = Duration::from_secs(10);

fn poll<T>(future: Pin<&mut impl Future<Output = T>>) -> Poll<T> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

fn finish<T>(mut future: Pin<&mut impl Future<Output = T>>) -> T {
    let deadline = Instant::now() + DEADLINE;
    loop {
        if let Poll::Ready(result) = poll(future.as_mut()) {
            return result;
        }
        assert!(Instant::now() < deadline, "startup did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn readiness_yields_while_backend_is_blocked_and_failure_allows_retry() {
    let (entered, entering) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let caller = std::thread::current().id();
    let mut start = Box::pin(Worker::start_with("unused".into(), move |_| {
        assert_ne!(std::thread::current().id(), caller);
        entered.send(()).unwrap();
        released.recv_timeout(DEADLINE).unwrap();
        anyhow::bail!("synthetic-start-failed")
    }));
    assert!(poll(start.as_mut()).is_pending());
    entering.recv_timeout(DEADLINE).unwrap();
    // Other event-loop work can run: polling again must still yield, not
    // synchronously receive the blocked worker's result.
    assert!(poll(start.as_mut()).is_pending());
    release.send(()).unwrap();
    let error = match finish(start.as_mut()) {
        Ok(_) => panic!("backend failure became a successful start"),
        Err(error) => error,
    };
    assert_eq!(error.to_string(), "synthetic-start-failed");

    let dir = std::env::temp_dir().join(format!("tc-worker-{}", uuid::Uuid::new_v4()));
    let mut retry = Box::pin(Worker::start_with(dir.clone(), |dir| {
        // An attached backend with no socket cannot subscribe or read a
        // source. It exercises readiness and ownership without starting a
        // daemon or accessing the developer's real state.
        Ok(Backend::Attached {
            store: trace_commons_contributor::config::ConfigStore::open(dir)?,
        })
    }));
    let worker = finish(retry.as_mut()).unwrap();
    assert!(!worker.hosts_the_loop());
    assert_eq!(worker.dir, dir);
    drop(worker);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn abandoning_a_pending_start_retires_the_late_daemon() {
    use trace_commons_contributor::config::{ConfigStore, DAEMON_LOCK_FILE};
    use trace_commons_contributor::daemon::settings::{DaemonSettings, SourceDeclaration};

    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let dir = std::env::temp_dir().join(format!("tc-{}", &suffix[..8]));
    let store = ConfigStore::open(dir.clone()).unwrap();
    DaemonSettings {
        claude_source: Some(SourceDeclaration::Off),
        codex_source: Some(SourceDeclaration::Off),
        ..Default::default()
    }
    .save(&store)
    .unwrap();
    let (entered, entering) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let mut start = Box::pin(Worker::start_with(dir.clone(), move |dir| {
        let backend = Backend::open(dir)?;
        assert!(backend.hosts_the_loop());
        entered.send(()).unwrap();
        released.recv_timeout(DEADLINE).unwrap();
        Ok(backend)
    }));
    assert!(poll(start.as_mut()).is_pending());
    entering.recv_timeout(DEADLINE).unwrap();
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(store.daemon_path(DAEMON_LOCK_FILE))
        .unwrap();
    assert!(
        lock.try_lock().is_err(),
        "the pending backend owns the daemon"
    );
    drop(start);
    release.send(()).unwrap();
    let deadline = Instant::now() + DEADLINE;
    while lock.try_lock().is_err() {
        assert!(
            Instant::now() < deadline,
            "abandoned startup retained its daemon"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    lock.unlock().unwrap();
    drop(lock);
    std::fs::remove_dir_all(dir).unwrap();
}
