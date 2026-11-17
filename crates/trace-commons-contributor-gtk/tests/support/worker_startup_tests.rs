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
            socket: trace_commons_contributor::config::ConfigStore::open(dir)?
                .daemon_path(trace_commons_contributor::config::DAEMON_SOCK_FILE),
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

/// A reopen replaces the backend on the worker thread: later calls reach the
/// new one, and the window learns whether it now hosts the loop.
#[test]
fn a_reopen_replaces_the_backend_and_reports_the_attachment() {
    use std::io::{BufRead, BufReader, Write};
    // Short: a socket path must fit in `sun_path`.
    let suffix = uuid::Uuid::new_v4().simple().to_string();
    let dir = std::env::temp_dir().join(format!("tc-ro-{}", &suffix[..8]));
    std::fs::create_dir(&dir).unwrap();
    let first = dir.join("a.sock");
    let second = dir.join("b.sock");
    let listener = std::os::unix::net::UnixListener::bind(&second).unwrap();
    let opens = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&opens);
    let (first_c, second_c) = (first.clone(), second.clone());
    let mut start = Box::pin(Worker::start_with(dir.clone(), move |_| {
        let n = counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Backend::Attached {
            socket: if n == 0 {
                first_c.clone()
            } else {
                second_c.clone()
            },
        })
    }));
    let worker = finish(start.as_mut()).unwrap();
    let id = worker.reopen(false);
    let (got, outcome) = worker.results.recv_blocking().unwrap();
    assert_eq!(got, id);
    match outcome {
        crate::worker::Outcome::Reopened { attachment, error } => {
            assert!(!attachment.hosts);
            assert_eq!(error, None);
        }
        _ => panic!("expected a reopen outcome"),
    }
    assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 2);
    // The new backend answers: the event subscription connects first, then
    // the call.
    let server = std::thread::spawn(move || {
        for stream in listener.incoming() {
            let stream = stream.unwrap();
            let mut line = String::new();
            BufReader::new(&stream).read_line(&mut line).unwrap();
            if line.contains("\"status\"") {
                (&stream)
                    .write_all(b"{\"id\":1,\"result\":{\"from\":\"second\"}}\n")
                    .unwrap();
                return;
            }
        }
    });
    let id = worker.call("status", serde_json::json!({}));
    let (got, outcome) = worker.results.recv_blocking().unwrap();
    assert_eq!(got, id);
    server.join().unwrap();
    match outcome {
        crate::worker::Outcome::Call(Ok(value)) => assert_eq!(value["from"], "second"),
        _ => panic!("expected the second backend's answer"),
    }
    drop(worker);
    let _ = std::fs::remove_dir_all(dir);
}
