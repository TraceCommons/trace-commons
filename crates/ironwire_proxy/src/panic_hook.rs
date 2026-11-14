//! Panic reports that say where a panic happened, never what it said.
//!
//! std's default hook prints the panic message to stderr, which is where the
//! daemon's log goes, and it runs before anything can catch the unwind. A
//! message can quote request content: a slicing panic prints `byte index N is
//! not a char boundary; it is inside 'é' ... of `<string>``, with up to 256
//! bytes of the string it was slicing, which is the prompt. Dropping the
//! payload after `catch_unwind` is too late; the hook has already printed it.
//!
//! Two pieces, because a process has one hook and IronWire does not always
//! own the process:
//!
//! - [`install_location_only`] replaces the hook for the whole process. The
//!   `ironwire` binary calls it first thing. Its release profile is
//!   `panic = "abort"`, under which the façade guard never runs and this hook
//!   is the only thing between the message and stderr. A host embedding the
//!   proxy that has no hook of its own may call it too.
//! - [`chain_for_guarded_handlers`] is installed by the façade guard. It takes
//!   over only panics raised while a façade handler is being polled, and hands
//!   every other panic to whatever hook was there before. A host's own hook
//!   (Trace Commons installs a location-only one) is neither replaced nor
//!   called twice, and a test binary keeps its assertion messages. A host that
//!   installs its hook after the first façade is built replaces this one, and
//!   then owns handler panics too.

use std::cell::Cell;
use std::future::Future;
use std::io::Write as _;
use std::panic::PanicHookInfo;
use std::pin::Pin;
use std::task::{Context, Poll};

thread_local! {
    /// How many guarded handler polls are in progress on this thread.
    static GUARDED: Cell<usize> = const { Cell::new(0) };
}

/// Replace the process's panic hook with one that reports the thread and the
/// source location only.
pub fn install_location_only() {
    std::panic::set_hook(Box::new(report));
}

/// Report a façade handler's panic by location only, and leave every other
/// panic to the hook installed before this. Installed once per process.
pub(crate) fn chain_for_guarded_handlers() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // A panic while the thread is being torn down cannot read the
            // counter; no handler is polled then, so it is not a guarded one.
            if GUARDED.try_with(Cell::get).unwrap_or(0) > 0 {
                report(info);
            } else {
                previous(info);
            }
        }));
    });
}

/// `future`, with every panic raised while it is polled reported by location
/// only (once [`chain_for_guarded_handlers`] is installed).
pub(crate) struct Guarded<F> {
    inner: Pin<Box<F>>,
}

impl<F> Guarded<F> {
    pub(crate) fn new(future: F) -> Self {
        Self {
            inner: Box::pin(future),
        }
    }
}

impl<F: Future> Future for Guarded<F> {
    type Output = F::Output;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        let _scope = Scope::enter();
        self.inner.as_mut().poll(cx)
    }
}

/// Marks this thread as polling a guarded handler. Dropped on return and on
/// unwind alike, and the hook runs before the unwind starts, so the hook
/// always sees the mark of the poll that panicked.
struct Scope;

impl Scope {
    fn enter() -> Self {
        GUARDED.with(|depth| depth.set(depth.get() + 1));
        Self
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let _ = GUARDED.try_with(|depth| depth.set(depth.get().saturating_sub(1)));
    }
}

fn report(info: &PanicHookInfo<'_>) {
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("<unnamed>");
    let location = info.location().map_or_else(
        || "an unknown location".to_string(),
        |at| format!("{}:{}:{}", at.file(), at.line(), at.column()),
    );
    // Written directly and with the error ignored, as std's own hook does: a
    // closed stderr must not turn a panic into an abort.
    let _ = writeln!(
        std::io::stderr().lock(),
        "thread '{name}' panicked at {location} (message withheld: it can quote request content)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::extract::Request;
    use axum::http::StatusCode;
    use axum::routing::post;
    use tower::ServiceExt;

    use crate::facade::guard::{Envelope, catch_panics};

    /// Stands for request content quoted by a panic message.
    const SECRET: &str = "prompt text that must not leak";
    /// What the stand-in for a host's own hook prints.
    const HOST_HOOK: &str = "host hook ran";
    /// Set on the child process; the child tests do nothing without it.
    const CHILD_ENV: &str = "IRONWIRE_PANIC_HOOK_CHILD";

    /// Run one of the child tests below in a fresh copy of this test binary:
    /// a hook is process-wide, so installing one here would change every
    /// other test's panic output. Returns the child's stderr.
    fn run_child(name: &str) -> String {
        let exe = std::env::current_exe().expect("test binary");
        let out = std::process::Command::new(exe)
            .args([
                "--exact",
                &format!("panic_hook::tests::{name}"),
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, "1")
            .output()
            .expect("run child");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(out.status.success(), "child failed:\n{stdout}\n{stderr}");
        assert!(stdout.contains("1 passed"), "child did not run:\n{stdout}");
        assert!(
            !stdout.contains(SECRET),
            "panic text reached stdout:\n{stdout}"
        );
        stderr
    }

    #[test]
    fn the_binary_hook_reports_where_a_panic_happened_and_never_its_message() {
        let stderr = run_child("location_only_child");
        assert!(
            !stderr.contains(SECRET),
            "panic text reached stderr:\n{stderr}"
        );
        assert!(stderr.contains("panic_hook.rs:"), "no location:\n{stderr}");
        assert!(
            stderr.contains("thread 'hook-test-worker'"),
            "no thread:\n{stderr}"
        );
    }

    #[test]
    fn a_handler_panic_is_reported_by_location_and_the_hosts_hook_keeps_every_other_panic() {
        let stderr = run_child("guarded_child");
        assert!(
            !stderr.contains(SECRET),
            "panic text reached stderr:\n{stderr}"
        );
        assert!(
            stderr.contains("panic_hook.rs:") && stderr.contains("message withheld"),
            "the handler panic was not reported by location:\n{stderr}"
        );
        assert_eq!(
            stderr.matches(HOST_HOOK).count(),
            1,
            "the host's hook must see the unguarded panic, and only that one:\n{stderr}"
        );
    }

    /// Only does anything as the child of the test above it.
    #[test]
    fn location_only_child() {
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        install_location_only();
        let joined = std::thread::Builder::new()
            .name("hook-test-worker".into())
            .spawn(|| panic!("{SECRET}"))
            .expect("spawn")
            .join();
        assert!(joined.is_err());
    }

    async fn boom() -> StatusCode {
        panic!("{SECRET}")
    }

    /// Only does anything as the child of the test above it.
    #[test]
    fn guarded_child() {
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        // Stands in for a host's own hook, installed before the proxy starts.
        std::panic::set_hook(Box::new(|_| eprintln!("{HOST_HOOK}")));
        let router = catch_panics(
            Router::new().route("/boom", post(boom)),
            Envelope::OpenAi,
            None,
        );
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let status = runtime.block_on(async {
            router
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/boom")
                        .body(Body::empty())
                        .expect("request builds"),
                )
                .await
                .expect("served")
                .status()
        });
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        // A panic outside any handler is the host's to report.
        let joined = std::thread::Builder::new()
            .name("outside-a-handler".into())
            .spawn(|| panic!("{SECRET}"))
            .expect("spawn")
            .join();
        assert!(joined.is_err());
    }
}
