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
    tokio::join!(
        poll(&shared.feature_flags, fetch, Utc::now, |snapshot| {
            shared.publish(EVENT_FEATURE_FLAGS_CHANGED, serde_json::json!({"instance_id":snapshot.instance_id,"generation":snapshot.generation}));
        }),
        crate::daemon::claude_desktop::run_rollout(shared),
    );
}
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
    let mut expiry = tokio::time::interval(Duration::from_secs(1));
    expiry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pending = Some(start_fetch(runtime.environment()));
    let mut published = runtime.snapshot().generation;
    loop {
        tokio::select! {
            _ = expiry.tick() => runtime.tick(now()),
            _ = refresh.tick() => { if pending.is_none() { pending = Some(start_fetch(runtime.environment())); } },
            _ = runtime.wake.notified() => { if pending.is_none() { pending = Some(start_fetch(runtime.environment())); } },
            response = async { match pending.as_mut() { Some(f) => f.await, None => std::future::pending().await } } => {
                pending = None;
                if let Ok(bytes) = response { let _ = crate::daemon::run_blocking(|| runtime.accept(&bytes, now())); }
                runtime.tick(now());
            }
        }
        let snapshot = runtime.snapshot();
        if snapshot.generation != published {
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
}
