//! Runtime context for blocking zbus calls made from ordinary OS threads.
//!
//! Linux feature unification enables zbus's Tokio executor. In zbus 5.18,
//! blocking Connection::object_server() synchronously starts an async dispatch
//! task, outside zbus's usual block_on wrapper. Keep a runtime entered for
//! object-server registration and subsequent connection lifetime on this thread.
use std::sync::OnceLock;

pub(crate) fn with_runtime<T>(operation: impl FnOnce() -> anyhow::Result<T>) -> anyhow::Result<T> {
    static RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    let runtime = RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("desktop-dbus-runtime-unavailable"))?;
    let _entered = runtime.enter();
    operation()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_os_thread_supports_spawn_and_cleanup_with_tokio_executor() {
        std::thread::spawn(|| {
            assert!(tokio::runtime::Handle::try_current().is_err());
            struct Cleanup;
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    // The synchronous drop path must have the same context
                    // as zbus connection/proxy cleanup, after its call ends.
                    tokio::spawn(async {});
                }
            }
            let result = with_runtime(|| {
                let _cleanup = Cleanup;
                let handle = tokio::runtime::Handle::current();
                handle.block_on(async {
                    let task = tokio::spawn(async {
                        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                        7
                    });
                    Ok(task.await.unwrap())
                })
            })
            .unwrap();
            assert_eq!(result, 7);
            assert!(tokio::runtime::Handle::try_current().is_err());
        })
        .join()
        .unwrap();
    }
}
