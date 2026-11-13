use super::*;
use crate::daemon::ipc::{DaemonShared, EVENT_FEATURE_FLAGS_CHANGED};
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
type Fetch = Pin<Box<dyn Future<Output = anyhow::Result<Vec<u8>>> + Send>>;
fn fetch(environment: Environment) -> Fetch {
    Box::pin(async move {
        tokio::time::timeout(Duration::from_secs(5), transport::fetch(environment)).await?
    })
}
/// Owned by the supervisor's select: dropping it cancels fetch/timers. No detached tasks.
pub async fn run(shared: &Arc<DaemonShared>, dry_run: bool) {
    if dry_run || shared.dev_dry_run || !shared.feature_flags.has_flags() {
        std::future::pending::<()>().await;
        return;
    }
    poll(&shared.feature_flags, fetch, Utc::now, |snapshot| {
        shared.publish(EVENT_FEATURE_FLAGS_CHANGED, serde_json::json!({"instance_id":snapshot.instance_id,"generation":snapshot.generation}));
    }).await;
}
/// A wake (system wake, `feature_flags_refresh`) starts a fetch at most this
/// often; one arriving sooner is deferred to the end of the window, not lost.
const WAKE_FLOOR: Duration = Duration::from_secs(30);
/// Longest sleep while a manifest applies. The monotonic deadline does not
/// advance while a Mac sleeps, so the wall-clock expiry is also checked at
/// least this often.
const EXPIRY_CHECK_CAP: Duration = Duration::from_secs(60);
async fn poll(
    runtime: &Runtime,
    mut start_fetch: impl FnMut(Environment) -> Fetch,
    now: impl Fn() -> DateTime<Utc>,
    mut publish: impl FnMut(Snapshot),
) {
    let seconds = 285 + (uuid::Uuid::new_v4().as_u128() % 16) as u64;
    let mut refresh = tokio::time::interval_at(
        tokio::time::Instant::now() + Duration::from_secs(seconds),
        Duration::from_secs(seconds),
    );
    refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pending = Some(start_fetch(runtime.environment()));
    let mut last_started = tokio::time::Instant::now();
    let mut wake_deferred = false;
    let mut published = runtime.generation();
    loop {
        let expiry = runtime.next_expiry().map(|deadline| {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            tokio::time::Instant::now() + remaining.min(EXPIRY_CHECK_CAP)
        });
        let deferred_until = last_started + WAKE_FLOOR;
        tokio::select! {
            _ = async { match expiry { Some(at) => tokio::time::sleep_until(at).await, None => std::future::pending().await } } => runtime.tick(now()),
            _ = refresh.tick() => {
                if pending.is_none() {
                    pending = Some(start_fetch(runtime.environment()));
                    last_started = tokio::time::Instant::now();
                    wake_deferred = false;
                }
            },
            _ = runtime.wake.notified() => {
                // A wake may follow a sleep that stopped the monotonic clock.
                runtime.tick(now());
                if pending.is_none() {
                    if tokio::time::Instant::now() >= deferred_until {
                        pending = Some(start_fetch(runtime.environment()));
                        last_started = tokio::time::Instant::now();
                    } else {
                        wake_deferred = true;
                    }
                }
            },
            _ = tokio::time::sleep_until(deferred_until), if wake_deferred && pending.is_none() => {
                pending = Some(start_fetch(runtime.environment()));
                last_started = tokio::time::Instant::now();
                wake_deferred = false;
            },
            response = async { match pending.as_mut() { Some(f) => f.await, None => std::future::pending().await } } => {
                pending = None;
                if let Ok(bytes) = response { let _ = crate::daemon::run_blocking(|| runtime.accept(&bytes, now())); }
                runtime.tick(now());
            }
        }
        if runtime.generation() != published {
            let snapshot = runtime.snapshot();
            published = snapshot.generation;
            publish(snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Active(Arc<AtomicUsize>);
    impl Drop for Active {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }
    #[tokio::test(start_paused = true)]
    async fn refresh_is_single_flight_and_cancelled_with_its_owner() {
        let temp = tempfile::tempdir().unwrap();
        let (registry, profile) = super::super::tests::fixtures();
        let time = super::super::tests::now();
        let runtime = Arc::new(Runtime::open(registry, profile, temp.path(), time).unwrap());
        let active = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let (send, receive) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let receive = Arc::new(tokio::sync::Mutex::new(receive));
        let (events, mut changed) = tokio::sync::mpsc::unbounded_channel();
        let cloned = Arc::clone(&runtime);
        let count = Arc::clone(&active);
        let calls = Arc::clone(&total);
        let owner = tokio::spawn(async move {
            poll(
                &cloned,
                move |_| {
                    let count = Arc::clone(&count);
                    let calls = Arc::clone(&calls);
                    let receive = Arc::clone(&receive);
                    Box::pin(async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        assert_eq!(count.fetch_add(1, Ordering::SeqCst), 0);
                        let _active = Active(count);
                        Ok(receive.lock().await.recv().await.unwrap())
                    })
                },
                move || time,
                move |snapshot| {
                    let _ = events.send(snapshot);
                },
            )
            .await;
        });
        tokio::task::yield_now().await;
        assert_eq!(active.load(Ordering::SeqCst), 1);
        for _ in 0..5 {
            runtime.request_refresh();
            tokio::task::yield_now().await;
        }
        tokio::time::advance(Duration::from_secs(301)).await;
        tokio::task::yield_now().await;
        assert_eq!(
            total.load(Ordering::SeqCst),
            1,
            "wake/timer cannot overlap an active fetch"
        );
        send.send(super::super::tests::doc(1, true)).unwrap();
        let snapshot = changed.recv().await.unwrap();
        assert!(snapshot.is_enabled("live"));
        assert!(!snapshot.is_enabled("blocked"));
        assert!(!snapshot.is_enabled("restart"));
        assert!(snapshot.flags["restart"].pending);
        runtime.request_refresh();
        tokio::task::yield_now().await;
        assert_eq!(active.load(Ordering::SeqCst), 1);
        send.send(super::super::tests::doc(2, false)).unwrap();
        assert!(!changed.recv().await.unwrap().is_enabled("live"));
        runtime.request_refresh();
        tokio::task::yield_now().await;
        owner.abort();
        let _ = owner.await;
        assert_eq!(
            active.load(Ordering::SeqCst),
            0,
            "dropping the owner drops an in-flight fetch"
        );
    }

    fn run_poll(
        runtime: &Arc<Runtime>,
        clock: impl Fn() -> DateTime<Utc> + Send + 'static,
    ) -> (
        tokio::task::JoinHandle<()>,
        Arc<AtomicUsize>,
        tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
        tokio::sync::mpsc::UnboundedReceiver<Snapshot>,
    ) {
        let total = Arc::new(AtomicUsize::new(0));
        let (send, receive) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let receive = Arc::new(tokio::sync::Mutex::new(receive));
        let (events, changed) = tokio::sync::mpsc::unbounded_channel();
        let cloned = Arc::clone(runtime);
        let calls = Arc::clone(&total);
        let owner = tokio::spawn(async move {
            poll(
                &cloned,
                move |_| {
                    let calls = Arc::clone(&calls);
                    let receive = Arc::clone(&receive);
                    Box::pin(async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        Ok(receive.lock().await.recv().await.unwrap())
                    })
                },
                clock,
                move |snapshot| {
                    let _ = events.send(snapshot);
                },
            )
            .await;
        });
        (owner, total, send, changed)
    }
    async fn settle() {
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
    }
    #[tokio::test(start_paused = true)]
    async fn wake_refreshes_have_a_floor_and_are_deferred_not_lost() {
        let temp = tempfile::tempdir().unwrap();
        let (registry, profile) = super::super::tests::fixtures();
        let time = super::super::tests::now();
        let runtime = Arc::new(Runtime::open(registry, profile, temp.path(), time).unwrap());
        let (owner, total, send, mut changed) = run_poll(&runtime, move || time);
        settle().await;
        send.send(super::super::tests::doc(1, true)).unwrap();
        changed.recv().await.unwrap();
        assert_eq!(total.load(Ordering::SeqCst), 1);
        for _ in 0..50 {
            runtime.request_refresh();
            settle().await;
        }
        assert_eq!(
            total.load(Ordering::SeqCst),
            1,
            "a burst of wakes inside the floor started a fetch"
        );
        tokio::time::advance(WAKE_FLOOR).await;
        settle().await;
        assert_eq!(
            total.load(Ordering::SeqCst),
            2,
            "the deferred wake runs once the floor has passed"
        );
        owner.abort();
        let _ = owner.await;
    }
    #[tokio::test(start_paused = true)]
    async fn expiry_is_published_without_a_once_a_second_tick() {
        let temp = tempfile::tempdir().unwrap();
        let (registry, profile) = super::super::tests::fixtures();
        let base = super::super::tests::now();
        let runtime = Arc::new(Runtime::open(registry, profile, temp.path(), base).unwrap());
        let started = tokio::time::Instant::now();
        let clock = move || {
            base + chrono::Duration::from_std(tokio::time::Instant::now() - started).unwrap()
        };
        let (owner, _total, send, mut changed) = run_poll(&runtime, clock);
        settle().await;
        send.send(super::super::tests::doc_until(
            1,
            true,
            base + chrono::Duration::seconds(90),
        ))
        .unwrap();
        assert!(changed.recv().await.unwrap().is_enabled("live"));
        let expired = tokio::time::timeout(Duration::from_secs(200), changed.recv())
            .await
            .expect("expiry was not published")
            .unwrap();
        assert!(!expired.is_enabled("live"));
        owner.abort();
        let _ = owner.await;
    }
}
