//! Everything that can block, off the GTK thread.
//!
//! A `preview` runs the whole redaction pipeline and, under an external
//! privacy filter, a network round trip -- the contract allows a daemon 60
//! seconds to answer one. Running that on the main loop would freeze the
//! window mid-decision, which on this particular product means freezing it
//! while someone is deciding whether to hand over their employer's
//! transcripts.
//!
//! So the backend lives on a worker thread, jobs go to it over a channel,
//! and results come back over an `async_channel` the GTK main context awaits.
//! Daemon events arrive on a second channel from a third thread holding the
//! subscription, so a slow preview cannot stall the event stream.

use std::sync::mpsc;

use anyhow::Result;

use crate::backend::{Backend, DaemonEvent, DaemonFailure};
use crate::model::PreviewSummary;

#[cfg(test)]
#[path = "../tests/support/worker_startup_tests.rs"]
mod startup_tests;

pub enum Job {
    Call {
        method: String,
        params: serde_json::Value,
    },
    Preview {
        entry_id: String,
    },
    /// How many times a needle appears in an entry's PRE-redaction session
    /// text. A count and nothing else; see `Backend::search_original`.
    SearchOriginal {
        entry_id: String,
        needle: String,
    },
}

pub enum Outcome {
    /// The daemon's answer, or its failure with any payload the daemon sent
    /// beside the label -- see [`DaemonFailure`].
    Call(Result<serde_json::Value, DaemonFailure>),
    /// The summary, and the redacted body when this deployment can serve
    /// one. `None` for the body is "not available here", never "empty".
    Preview(Result<(PreviewSummary, Option<String>), String>),
    /// `None` for any failure. Not a `Result`: the caller must never round a
    /// failure off to a clean answer, and one shape for "did not find it"
    /// and "could not look" would invite exactly that. See
    /// `crate::original_search::Outcome::Unknown`.
    SearchOriginal(Option<u32>),
}

pub struct Worker {
    jobs: mpsc::Sender<(u64, Job)>,
    pub results: async_channel::Receiver<(u64, Outcome)>,
    pub events: async_channel::Receiver<DaemonEvent>,
    hosts_the_loop: bool,
    next_id: std::cell::Cell<u64>,
    /// The contributor state directory this shell was started against.
    ///
    /// Held because two things live in `config.json` rather than in daemon
    /// settings -- the witness among them -- and the IPC contract has no
    /// method for either, so the settings screen reads them through
    /// `ConfigStore` directly. Taken from what `Backend::open` was given,
    /// never re-resolved: `--state-dir` exists so a container run does not
    /// touch a real one, and a second `ConfigStore::resolve` here would
    /// quietly ignore it and edit the contributor's own configuration.
    pub dir: std::path::PathBuf,
}

impl Worker {
    /// Channels for widget tests; no daemon or credential store is opened.
    #[cfg(all(test, target_os = "linux"))]
    pub(crate) fn fixture(
        dir: std::path::PathBuf,
    ) -> (
        Self,
        mpsc::Receiver<(u64, Job)>,
        async_channel::Sender<(u64, Outcome)>,
    ) {
        let (jobs, requests) = mpsc::channel();
        let (responses, results) = async_channel::unbounded();
        let (_, events) = async_channel::unbounded();
        (
            Self {
                jobs,
                results,
                events,
                hosts_the_loop: false,
                next_id: std::cell::Cell::new(1),
                dir,
            },
            requests,
            responses,
        )
    }

    pub async fn start(dir: std::path::PathBuf) -> Result<Self> {
        Self::start_with(dir, Backend::open).await
    }

    async fn start_with(
        dir: std::path::PathBuf,
        open: impl FnOnce(std::path::PathBuf) -> Result<Backend> + Send + 'static,
    ) -> Result<Self> {
        let (job_tx, job_rx) = mpsc::channel::<(u64, Job)>();
        let (result_tx, results) = async_channel::unbounded();
        let (event_tx, events) = async_channel::unbounded();
        let (ready_tx, ready_rx) = async_channel::bounded::<Result<bool, String>>(1);
        let held = dir.clone();

        std::thread::spawn(move || {
            let backend = match open(dir) {
                Ok(b) => b,
                Err(e) => {
                    let _ = ready_tx.send_blocking(Err(e.to_string()));
                    return;
                }
            };
            if ready_tx
                .send_blocking(Ok(backend.hosts_the_loop()))
                .is_err()
            {
                return;
            }

            match backend.attached_socket().map(std::path::Path::to_path_buf) {
                Some(socket) => {
                    std::thread::spawn(move || {
                        pump_events(
                            || Backend::events_at(&socket),
                            &event_tx,
                            Some(EVENTS_RECONNECT),
                        )
                    });
                }
                None => {
                    if let Ok(stream) = backend.events() {
                        let mut stream = Some(stream);
                        std::thread::spawn(move || {
                            pump_events(
                                || stream.take().ok_or_else(|| anyhow::anyhow!("opened once")),
                                &event_tx,
                                None,
                            )
                        });
                    }
                }
            }

            for (id, job) in job_rx {
                let outcome = match job {
                    Job::Call { method, params } => Outcome::Call(
                        backend
                            .call(&method, params)
                            .map_err(DaemonFailure::from_error),
                    ),
                    Job::Preview { entry_id } => {
                        Outcome::Preview(backend.preview(&entry_id).map_err(|e| e.to_string()))
                    }
                    Job::SearchOriginal { entry_id, needle } => {
                        Outcome::SearchOriginal(backend.search_original(&entry_id, &needle))
                    }
                };
                if result_tx.send_blocking((id, outcome)).is_err() {
                    return;
                }
            }
        });

        let hosts_the_loop = ready_rx
            .recv()
            .await
            .map_err(|_| anyhow::anyhow!("the daemon worker stopped before it started"))?
            .map_err(|label| anyhow::anyhow!("{label}"))?;

        Ok(Self {
            jobs: job_tx,
            results,
            events,
            hosts_the_loop,
            next_id: std::cell::Cell::new(1),
            dir: held,
        })
    }

    /// Whether this process is the watcher, or is attached to one.
    ///
    /// This decides two user-visible things: whether the preview sheet can
    /// show the transcript at all, and which of the two quit warnings is
    /// the true one.
    pub fn hosts_the_loop(&self) -> bool {
        self.hosts_the_loop
    }

    pub fn submit(&self, job: Job) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let _ = self.jobs.send((id, job));
        id
    }

    pub fn call(&self, method: &str, params: serde_json::Value) -> u64 {
        self.submit(Job::Call {
            method: method.to_string(),
            params,
        })
    }

    pub fn preview(&self, entry_id: &str) -> u64 {
        self.submit(Job::Preview {
            entry_id: entry_id.to_string(),
        })
    }

    pub fn search_original(&self, entry_id: &str, needle: &str) -> u64 {
        self.submit(Job::SearchOriginal {
            entry_id: entry_id.to_string(),
            needle: needle.to_string(),
        })
    }
}

/// How long the event thread waits before opening the subscription again
/// after it could not, or after the daemon it was attached to went away.
const EVENTS_RECONNECT: std::time::Duration = std::time::Duration::from_secs(2);

/// Feed daemon events into `tx` until its receiver is gone.
///
/// `reconnect` is `Some` for a window attached over a socket: the daemon
/// there can stop and come back -- the host daemon a Flatpak window attached
/// to while it was down, or a unit restarting -- so a failed open or an ended
/// stream is retried after that wait. Every subscription opened after the
/// first attempt starts with `resync_required`, because events were missed
/// in between. `None` (hosting the loop in-process) opens once: that stream
/// ends only when this process stops the daemon.
pub(crate) fn pump_events(
    mut open: impl FnMut() -> Result<Box<dyn crate::backend::EventStream>>,
    tx: &async_channel::Sender<DaemonEvent>,
    reconnect: Option<std::time::Duration>,
) {
    let mut first = true;
    loop {
        if let Ok(mut stream) = open() {
            if !first {
                let resync = DaemonEvent {
                    name: "resync_required".to_string(),
                    entry_id: None,
                    digest: None,
                };
                if tx.send_blocking(resync).is_err() {
                    return;
                }
            }
            while let Some(event) = stream.next() {
                if tx.send_blocking(event).is_err() {
                    return;
                }
            }
        }
        first = false;
        let Some(wait) = reconnect else {
            return;
        };
        if tx.is_closed() {
            return;
        }
        std::thread::sleep(wait);
    }
}

#[cfg(test)]
mod event_tests {
    use super::*;
    use crate::backend::EventStream;

    struct Scripted(std::vec::IntoIter<&'static str>);

    impl EventStream for Scripted {
        fn next(&mut self) -> Option<DaemonEvent> {
            self.0.next().map(|name| DaemonEvent {
                name: name.to_string(),
                entry_id: None,
                digest: None,
            })
        }
    }

    fn stream(names: &[&'static str]) -> Result<Box<dyn EventStream>> {
        Ok(Box::new(Scripted(names.to_vec().into_iter())))
    }

    #[test]
    fn an_attached_window_picks_events_up_again_when_its_daemon_returns() {
        // Attached while the host daemon was down; it comes up, runs, stops,
        // and comes up again.
        let (tx, rx) = async_channel::unbounded();
        let mut attempt = 0;
        let pump = std::thread::spawn(move || {
            pump_events(
                move || {
                    attempt += 1;
                    match attempt {
                        1 | 3 => Err(anyhow::anyhow!("daemon-not-running")),
                        2 => stream(&["queue_changed"]),
                        4 => stream(&["status_changed"]),
                        _ => Err(anyhow::anyhow!("daemon-not-running")),
                    }
                },
                &tx,
                Some(std::time::Duration::ZERO),
            )
        });
        let got: Vec<String> = (0..4).map(|_| rx.recv_blocking().unwrap().name).collect();
        drop(rx);
        pump.join().unwrap();
        assert_eq!(
            got,
            [
                "resync_required",
                "queue_changed",
                "resync_required",
                "status_changed"
            ]
        );
    }

    #[test]
    fn the_first_subscription_needs_no_resync() {
        let (tx, rx) = async_channel::unbounded();
        let mut opened = false;
        let pump = std::thread::spawn(move || {
            pump_events(
                move || {
                    if opened {
                        return Err(anyhow::anyhow!("daemon-not-running"));
                    }
                    opened = true;
                    stream(&["queue_changed"])
                },
                &tx,
                Some(std::time::Duration::ZERO),
            )
        });
        assert_eq!(rx.recv_blocking().unwrap().name, "queue_changed");
        drop(rx);
        pump.join().unwrap();
    }

    #[test]
    fn hosting_opens_once_and_stops_with_its_stream() {
        let (tx, rx) = async_channel::unbounded();
        let mut opens = 0;
        pump_events(
            || {
                opens += 1;
                stream(&["queue_changed"])
            },
            &tx,
            None,
        );
        assert_eq!(opens, 1);
        assert_eq!(rx.recv_blocking().unwrap().name, "queue_changed");
        assert!(rx.try_recv().is_err());
    }
}
