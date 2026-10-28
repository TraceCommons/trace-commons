//! Subscription usage (PR #1608 design, first slice): the contributor's
//! ChatGPT plan and how much of each Codex limit they have used, read from
//! their own Codex on this Mac, for the Insights "Subscriptions" card.
//!
//! Built only with the `subscription-usage` cargo feature, and off at run
//! time until the contributor turns on the `subscription_usage.codex`
//! setting. Nothing polls: a probe runs only when the contributor chooses
//! Refresh (`subscription_usage` with `refresh: true`), at most once a
//! minute, and never while Codex is already running.
//!
//! In this slice: Codex only, through `codex app-server` (design 2.2). Not
//! here: Codex's `auth.json` and the usage endpoint (2.1), Claude (2.3,
//! 2.4), web extras (2.5), history (3.5) and the Private AI row (5.2).
//!
//! # What crosses the socket
//!
//! Fixed labels, per-mille figures, durations, reset times and a credit
//! balance. Never an email, an account id, a path, a token or upstream text.
//! Nothing here is logged.

pub mod codex_app_server;
pub mod snapshot;

use chrono::{DateTime, Utc};

use crate::daemon::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
pub use snapshot::{Reading, Reason, Scheduled, SubscriptionUsage};

/// The IPC method.
pub const METHOD: &str = "subscription_usage";
/// `refresh` present and not a boolean.
pub const ERR_REFRESH_INVALID: &str = "refresh-invalid";

/// Where a reading comes from. Tests use fakes; a later source replaces one
/// implementation without touching the IPC surface.
pub trait UsageSource: Send + Sync {
    /// Checks made before a launch. An `Err` here launches nothing and does
    /// not start the probe floor.
    fn ready(&self) -> Result<(), Reason>;

    /// One blocking probe, run off the IPC thread.
    fn probe(&self, now: DateTime<Utc>) -> Result<Reading, Reason>;
}

/// `subscription_usage`. Synchronous: it answers from the held snapshot,
/// and `refresh: true` only schedules a probe, whose answer arrives as
/// `status_changed`.
pub fn handle(shared: &DaemonShared, req: &Request) -> Response {
    let refresh = match req.params.get("refresh") {
        None | Some(serde_json::Value::Null) => false,
        Some(serde_json::Value::Bool(refresh)) => *refresh,
        Some(_) => return Response::err(req.id, ERR_BAD_PARAMS, ERR_REFRESH_INVALID),
    };
    // The settings lock is held across the check and the scheduling, so a
    // `set_settings` that turns the switch off (and clears the snapshot)
    // lands either before, and nothing is scheduled, or after, and the
    // probe sees the cleared generation before it launches. `held` is a
    // leaf lock, so taking it under this one is safe. A poisoned lock reads
    // as off: fail closed, never a guess.
    {
        let Ok(settings) = shared.settings.lock() else {
            return Response::ok(req.id, serde_json::json!({ "enabled": false }));
        };
        if !settings.subscription_usage.codex {
            return Response::ok(req.id, serde_json::json!({ "enabled": false }));
        }
        if refresh {
            shared.subscription_usage.refresh();
        }
    }
    Response::ok(req.id, shared.subscription_usage.value())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::ipc::{ERR_UNKNOWN_METHOD, METHODS, advertised_methods, handle_request};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct Fake {
        launches: AtomicUsize,
    }

    impl UsageSource for Fake {
        fn ready(&self) -> Result<(), Reason> {
            Ok(())
        }

        fn probe(&self, now: DateTime<Utc>) -> Result<Reading, Reason> {
            self.launches.fetch_add(1, Ordering::SeqCst);
            Ok(Reading {
                at: now,
                plan: snapshot::Plan::Pro,
                limit_reached: None,
                windows: Vec::new(),
                credits: None,
                reset_credits: None,
            })
        }
    }

    fn shared() -> (tempfile::TempDir, DaemonShared, Arc<Fake>) {
        let (dir, store) = crate::config::tests_support::temp_store();
        let shared = DaemonShared::load(store).unwrap();
        let fake = Arc::new(Fake {
            launches: AtomicUsize::new(0),
        });
        shared.subscription_usage.set_source(fake.clone());
        (dir, shared, fake)
    }

    fn call(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    #[test]
    fn off_by_default_and_off_launches_nothing() {
        let (_dir, s, fake) = shared();
        assert!(!s.settings.lock().unwrap().subscription_usage.codex);
        let r = handle_request(&s, &call(METHOD, serde_json::json!({"refresh": true})));
        assert_eq!(r.result.unwrap(), serde_json::json!({"enabled": false}));
        assert!(s.subscription_usage.wait_idle(Duration::from_secs(5)));
        assert_eq!(fake.launches.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn refresh_answers_at_once_and_the_probe_lands_after() {
        let (_dir, s, fake) = shared();
        let mut events = s.events.subscribe();
        let set = handle_request(
            &s,
            &call(
                "set_settings",
                serde_json::json!({"subscription_usage": {"codex": true}}),
            ),
        );
        assert_eq!(
            set.result.unwrap()["subscription_usage"],
            serde_json::json!({"codex": true})
        );
        let plain = handle_request(&s, &call(METHOD, serde_json::json!({})));
        assert_eq!(
            plain.result.unwrap()["logins"][0]["reason"],
            "never_refreshed"
        );
        assert_eq!(
            fake.launches.load(Ordering::SeqCst),
            0,
            "no refresh, no probe"
        );
        let r = handle_request(&s, &call(METHOD, serde_json::json!({"refresh": true})));
        assert!(r.error.is_none());
        assert!(s.subscription_usage.wait_idle(Duration::from_secs(5)));
        assert_eq!(fake.launches.load(Ordering::SeqCst), 1);
        let after = handle_request(&s, &call(METHOD, serde_json::json!({})));
        let login = after.result.unwrap()["logins"][0].clone();
        assert_eq!(login["state"], "observed");
        assert_eq!(login["plan"], "pro");
        assert_eq!(login["refreshing"], false);
        let mut saw_status = false;
        while let Ok(event) = events.try_recv() {
            saw_status |= event.event == crate::daemon::ipc::EVENT_STATUS_CHANGED;
        }
        assert!(saw_status, "the answer is announced as status_changed");
    }

    #[test]
    fn turning_the_switch_off_clears_the_held_reading() {
        let (_dir, s, _fake) = shared();
        let on = serde_json::json!({"subscription_usage": {"codex": true}});
        let off = serde_json::json!({"subscription_usage": {"codex": false}});
        assert!(
            handle_request(&s, &call("set_settings", on.clone()))
                .error
                .is_none()
        );
        handle_request(&s, &call(METHOD, serde_json::json!({"refresh": true})));
        assert!(s.subscription_usage.wait_idle(Duration::from_secs(5)));
        assert!(
            handle_request(&s, &call("set_settings", off))
                .error
                .is_none()
        );
        let r = handle_request(&s, &call(METHOD, serde_json::json!({})));
        assert_eq!(r.result.unwrap(), serde_json::json!({"enabled": false}));
        assert!(
            handle_request(&s, &call("set_settings", on))
                .error
                .is_none()
        );
        let r = handle_request(&s, &call(METHOD, serde_json::json!({})));
        assert_eq!(
            r.result.unwrap()["logins"][0]["reason"],
            "never_refreshed",
            "the old reading did not come back"
        );
    }

    #[test]
    fn a_refresh_that_is_not_a_boolean_is_refused() {
        let (_dir, s, _fake) = shared();
        let r = handle_request(&s, &call(METHOD, serde_json::json!({"refresh": "yes"})));
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_BAD_PARAMS);
        assert_eq!(err.message, ERR_REFRESH_INVALID);
        assert!(
            handle_request(&s, &call(METHOD, serde_json::Value::Null))
                .error
                .is_none()
        );
    }

    #[test]
    fn the_settings_key_takes_only_a_codex_boolean() {
        let (_dir, s, _fake) = shared();
        for bad in [
            serde_json::json!({"subscription_usage": {"codex": "on"}}),
            serde_json::json!({"subscription_usage": {"claude": true}}),
            serde_json::json!({"subscription_usage": true}),
        ] {
            assert!(
                handle_request(&s, &call("set_settings", bad.clone()))
                    .error
                    .is_some(),
                "{bad}"
            );
        }
        let got = handle_request(&s, &call("get_settings", serde_json::json!({})));
        assert_eq!(
            got.result.unwrap()["subscription_usage"],
            serde_json::json!({"codex": false})
        );
    }

    #[test]
    fn the_method_is_advertised_dispatched_and_not_dry_run_local() {
        assert!(advertised_methods().contains(&METHOD));
        assert!(
            !METHODS.contains(&METHOD),
            "advertised through the feature list"
        );
        assert!(!crate::daemon::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&METHOD));
        let (_dir, s, _fake) = shared();
        let r = handle_request(&s, &call(METHOD, serde_json::json!({})));
        assert!(r.error.is_none(), "not {ERR_UNKNOWN_METHOD}");
        let hello = handle_request(&s, &call("hello", serde_json::json!({})));
        assert!(
            hello.result.unwrap()["methods"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(METHOD))
        );
    }
}
