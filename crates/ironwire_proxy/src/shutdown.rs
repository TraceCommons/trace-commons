//! Telling a response that the daemon is stopping.
//!
//! Graceful shutdown means "stop accepting, then let what is in flight
//! finish", and that is exactly right for the thing it was built for: a
//! streamed model response mid-turn, which is the outage IronWire exists to
//! prevent. It is exactly wrong for a response that is *designed* never to
//! finish. `/_ironwire/events` is held open for as long as a client cares to
//! hold it — `ironwire watch` all day, the menu bar app for the life of the
//! login session — so waiting for it is waiting forever, and a plain `kill`,
//! `systemctl --user stop` or `brew services restart` hangs for as long as
//! anyone has a client open.
//!
//! So the server says so, and the handler ends its own stream. This is the
//! announcement.
//!
//! ```text
//!   SIGTERM ─→ Shutdown::begin() ─→ /_ironwire/events yields ": closing", returns
//!                    │                            │
//!                    └─→ axum stops accepting ────┴─→ drains what is left ─→ exit
//! ```
//!
//! Level-triggered, not edge-triggered: [`Shutdown::begins`] returns
//! immediately once it has been announced. A stream that opened during the
//! shutdown would otherwise wait for a second announcement that never comes.

use std::sync::Arc;

use tokio::sync::watch;

/// A one-way announcement that the daemon is stopping.
///
/// Cloneable and cheap: it lives in [`crate::state::AppState`], which every
/// handler has.
#[derive(Clone, Debug)]
pub struct Shutdown {
    /// Held in an `Arc` so the announcement survives every clone of the state.
    /// A dropped `Sender` would make the receivers fire on their own, which
    /// would end every event stream for the wrong reason.
    tx: Arc<watch::Sender<bool>>,
}

impl Default for Shutdown {
    fn default() -> Self {
        Self::new()
    }
}

impl Shutdown {
    /// A daemon that is running.
    #[must_use]
    pub fn new() -> Self {
        let (tx, _) = watch::channel(false);
        Self { tx: Arc::new(tx) }
    }

    /// Announce it. Idempotent, and fine to call with nobody listening.
    pub fn begin(&self) {
        // `send_replace` rather than `send`, which *fails without storing the
        // value* when there are no receivers — and no receivers is the normal
        // case, because most daemons shut down with nothing streaming. The
        // value has to be stored anyway: a stream that opens a moment later
        // reads it and leaves instead of waiting for the next announcement,
        // which is the whole reason this is a flag and not a notification.
        self.tx.send_replace(true);
    }

    /// Whether it has been announced already.
    #[must_use]
    pub fn begun(&self) -> bool {
        *self.tx.borrow()
    }

    /// Resolves when the daemon starts shutting down, or immediately if it
    /// already has.
    pub async fn begins(&self) {
        let mut rx = self.tx.subscribe();
        // Read before waiting. `changed()` only fires on a value that arrives
        // *after* the receiver was made, so a stream that opened one
        // instruction after `begin()` would hang on a change that has already
        // happened. The borrow is dropped before the await on purpose.
        let already = *rx.borrow_and_update();
        if already {
            return;
        }
        // The error is a dropped sender, which cannot happen while `self` is
        // alive and, if it somehow did, means the daemon is gone anyway.
        let _ = rx.changed().await;
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_waiter_is_released_by_the_announcement() {
        let shutdown = Shutdown::new();
        assert!(!shutdown.begun());

        let waiting = tokio::spawn({
            let shutdown = shutdown.clone();
            async move { shutdown.begins().await }
        });
        // Nothing has been announced, so it must still be waiting.
        assert!(!waiting.is_finished());

        shutdown.begin();
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the waiter was never released")
            .expect("the waiting task panicked");
        assert!(shutdown.begun());
    }

    /// The bug this type exists to prevent, in miniature: a stream that opens
    /// after the announcement must not wait for the next one.
    #[tokio::test]
    async fn a_waiter_that_arrives_late_does_not_wait_at_all() {
        let shutdown = Shutdown::new();
        shutdown.begin();
        tokio::time::timeout(Duration::from_secs(5), shutdown.begins())
            .await
            .expect("a late waiter is waiting for an announcement already made");
    }

    /// Every handler holds a clone, and a clone that has been dropped must not
    /// look like a shutdown to anyone still listening.
    #[tokio::test]
    async fn a_dropped_clone_announces_nothing() {
        let shutdown = Shutdown::new();
        let clone = shutdown.clone();
        drop(clone);
        assert!(!shutdown.begun());
        assert!(
            tokio::time::timeout(Duration::from_millis(50), shutdown.begins())
                .await
                .is_err(),
            "dropping a clone released a waiter"
        );
    }
}
