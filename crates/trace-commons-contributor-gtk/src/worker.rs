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

use crate::backend::{Backend, DaemonEvent};
use crate::model::PreviewSummary;

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
    Call(Result<serde_json::Value, String>),
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
}

impl Worker {
    pub fn start(dir: std::path::PathBuf) -> Result<Self> {
        let (job_tx, job_rx) = mpsc::channel::<(u64, Job)>();
        let (result_tx, results) = async_channel::unbounded();
        let (event_tx, events) = async_channel::unbounded();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<bool, String>>();

        std::thread::spawn(move || {
            let backend = match Backend::open(dir) {
                Ok(b) => b,
                Err(e) => {
                    let _ = ready_tx.send(Err(e.to_string()));
                    return;
                }
            };
            let _ = ready_tx.send(Ok(backend.hosts_the_loop()));

            if let Ok(mut stream) = backend.events() {
                std::thread::spawn(move || {
                    while let Some(event) = stream.next() {
                        if event_tx.send_blocking(event).is_err() {
                            return;
                        }
                    }
                });
            }

            for (id, job) in job_rx {
                let outcome = match job {
                    Job::Call { method, params } => {
                        Outcome::Call(backend.call(&method, params).map_err(|e| e.to_string()))
                    }
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
            .map_err(|_| anyhow::anyhow!("the daemon worker stopped before it started"))?
            .map_err(|label| anyhow::anyhow!("{label}"))?;

        Ok(Self {
            jobs: job_tx,
            results,
            events,
            hosts_the_loop,
            next_id: std::cell::Cell::new(1),
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
