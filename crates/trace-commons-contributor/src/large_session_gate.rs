//! One large session at a time.
//!
//! Building an envelope for a session costs about three times the session's
//! size in memory at the peak (read, parse, raw contribution, redacted
//! envelope, serialization), measured in #1742 once the per-session read
//! limit (`source::MAX_SESSION_BYTES`) went to 512 MiB. Two such sessions
//! built side by side -- a preview and an upload, or two previews -- would
//! hold about 3 GB between them.
//!
//! So a session over [`LARGE_SESSION_BYTES`] takes the single permit of a
//! process-wide gate before it is read, and gives it back once the envelope
//! is built and measured: before the network upload, never after it. A
//! second large session waits its turn rather than failing. A session at or
//! under the threshold never touches the gate, so ordinary previews and
//! uploads are never queued behind a large one.
//!
//! The size used is `SessionRef::size_bytes` (or the queue entry's copy of
//! it), which covers every file the ref loads -- the primary transcript plus
//! any subagent transcripts -- and is known before anything is read.

use tokio::sync::{Semaphore, SemaphorePermit};

/// Sessions larger than this many bytes build one at a time.
pub const LARGE_SESSION_BYTES: u64 = 128 * 1024 * 1024;

/// A one-permit gate for large session builds.
///
/// The daemon uses the single process-wide instance behind [`admit`]; tests
/// make their own so they cannot contend with each other.
pub(crate) struct LargeSessionGate {
    permits: Semaphore,
}

/// Held while a large session is read, redacted and serialized. Dropping it
/// lets the next large session in. `None` inside means the session was small
/// and never took the gate.
#[must_use = "the gate is released as soon as this is dropped"]
pub struct LargeSessionPermit {
    permit: Option<SemaphorePermit<'static>>,
}

impl LargeSessionPermit {
    /// Whether this build holds the gate (the session was large).
    pub fn holds_gate(&self) -> bool {
        self.permit.is_some()
    }
}

impl LargeSessionGate {
    pub(crate) const fn new() -> Self {
        Self {
            permits: Semaphore::const_new(1),
        }
    }

    /// Wait for the gate if `size_bytes` is over [`LARGE_SESSION_BYTES`];
    /// return at once otherwise.
    pub(crate) async fn admit(&'static self, size_bytes: u64) -> LargeSessionPermit {
        if size_bytes <= LARGE_SESSION_BYTES {
            return LargeSessionPermit { permit: None };
        }
        // The semaphore is never closed, so `acquire` cannot fail; a
        // permit-less build would only lose the memory bound, not
        // correctness, so that is the fallback rather than a panic.
        LargeSessionPermit {
            permit: self.permits.acquire().await.ok(),
        }
    }
}

static GATE: LargeSessionGate = LargeSessionGate::new();

/// Take the process-wide gate for a session of `size_bytes`, waiting if
/// another large session is building. Small sessions return at once.
pub async fn admit(size_bytes: u64) -> LargeSessionPermit {
    GATE.admit(size_bytes).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fresh_gate() -> &'static LargeSessionGate {
        Box::leak(Box::new(LargeSessionGate::new()))
    }

    const LARGE: u64 = LARGE_SESSION_BYTES + 1;

    #[tokio::test]
    async fn two_large_builds_do_not_overlap() {
        let gate = fresh_gate();
        let first = gate.admit(LARGE).await;
        assert!(first.holds_gate());

        // The second large session waits while the first holds the gate...
        let waiting = tokio::spawn(async move { gate.admit(LARGE).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !waiting.is_finished(),
            "a second large build must wait for the first"
        );

        // ...and gets in once the first lets go, rather than failing.
        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("the waiting build is admitted once the gate is free")
            .unwrap();
        assert!(second.holds_gate());
    }

    #[tokio::test]
    async fn a_small_build_is_not_blocked_by_a_large_one() {
        let gate = fresh_gate();
        let _large = gate.admit(LARGE).await;
        for size in [0, 1, LARGE_SESSION_BYTES] {
            let small = tokio::time::timeout(Duration::from_millis(100), gate.admit(size))
                .await
                .expect("a session at or under the threshold never waits");
            assert!(!small.holds_gate());
        }
    }

    #[tokio::test]
    async fn concurrent_large_builds_never_run_two_at_once() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let gate = fresh_gate();
        let inside = std::sync::Arc::new(AtomicUsize::new(0));
        let peak = std::sync::Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..4 {
            let inside = inside.clone();
            let peak = peak.clone();
            tasks.push(tokio::spawn(async move {
                let _permit = gate.admit(LARGE).await;
                let now = inside.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(now, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(10)).await;
                inside.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }
}
