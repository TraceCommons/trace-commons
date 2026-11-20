//! The window's reads of the daemon's feature-flag snapshot, as the Mac's
//! `AppModel+FeatureFlags.swift` makes them: at start, on
//! `feature_flags_changed`, and (here) on `resync_required`, which is how
//! this shell learns its daemon was replaced.
//!
//! Neither feature the Mac gates has a surface in this shell yet -- there
//! is no Skills tab in Insights and no Claude Desktop setup -- so nothing
//! reads [`App::feature_is_enabled`] today. A surface ported later gates on
//! it, never on a decision of its own.

use std::rc::Rc;

use super::App;
use crate::feature_flags;

impl App {
    /// Read the daemon's snapshot again. Only the latest read's answer is
    /// applied; a failed read, or one about another build's profile, leaves
    /// every flag off.
    pub fn refresh_feature_flags(self: &Rc<Self>) {
        let read = self.feature_flags.begin_read();
        self.call(
            feature_flags::METHOD_SNAPSHOT,
            serde_json::json!({}),
            move |app, answer| {
                let identity = feature_flags::build_identity();
                let snapshot = feature_flags::read_answer(answer, identity.as_ref());
                app.feature_flags.finish_read(read, snapshot);
            },
        );
    }

    /// Whether the daemon resolved `id` on. Off when unknown, and while no
    /// snapshot is held.
    pub fn feature_is_enabled(&self, id: &str) -> bool {
        self.feature_flags.is_enabled(id)
    }
}

// Under `#[cfg(test)]` alone, so the wording scan (`tests/shell_wording.rs`)
// skips it as it skips every test module; the mounted test is Linux-only.
#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    mod mounted {
        use super::super::*;
        use crate::backend::DaemonEvent;
        use crate::worker::{Job, Outcome, Worker};
        use gtk::gio::prelude::ApplicationExt;

        /// The recorded daemon answer, about this build's profile, with
        /// `insights_skills` as given and the generation given.
        fn answer(generation: u64, skills: bool) -> serde_json::Value {
            let identity = feature_flags::build_identity().unwrap();
            let mut value: serde_json::Value = serde_json::from_str(include_str!(
                "../../tests/fixtures/feature_flags_snapshot.json"
            ))
            .unwrap();
            value["environment"] = serde_json::json!(identity.environment.as_str());
            value["profile_digest"] = serde_json::json!(identity.profile_digest);
            value["generation"] = serde_json::json!(generation);
            value["flags"]["insights_skills"]["enabled"] = serde_json::json!(skills);
            value
        }

        /// A mounted window reads the snapshot when it starts, reads it again
        /// when the daemon says it changed, and holds nothing once the daemon
        /// answers about another build's profile.
        #[test]
        #[ignore = "requires a Linux GTK display; run alone with --ignored --test-threads=1"]
        fn mounted_the_window_reads_flags_at_start_and_again_on_a_change() {
            let context = gtk::glib::MainContext::default();
            let _owner = context.acquire().unwrap();
            adw::init().expect("GTK display unavailable");
            let application = adw::Application::builder()
                .application_id("ai.tracecommons.FeatureFlagsTest")
                .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
                .build();
            application
                .register(None::<&gtk::gio::Cancellable>)
                .unwrap();
            let dir = std::env::temp_dir().join(format!("tc-flags-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let (worker, jobs, results, events) = Worker::fixture_with_events(dir.clone());
            let app = App::build(&application, worker);

            // Answer every snapshot read with `next`, refuse every other call,
            // until `done` holds. How many snapshot reads were answered.
            let pump = |next: &dyn Fn() -> serde_json::Value, done: &dyn Fn(&App) -> bool| {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                let mut reads = 0;
                loop {
                    while let Ok((id, job)) = jobs.try_recv() {
                        let reply = match job {
                            Job::Call { method, .. }
                                if method == feature_flags::METHOD_SNAPSHOT =>
                            {
                                reads += 1;
                                Ok(next())
                            }
                            _ => Err("synthetic-refused".to_string()),
                        };
                        let _ = results.try_send((id, Outcome::Call(reply.map_err(Into::into))));
                    }
                    if done(app.as_ref()) {
                        return reads;
                    }
                    assert!(
                        std::time::Instant::now() < deadline,
                        "the window never reached the expected flags"
                    );
                    context.iteration(false);
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
            };

            // Before any answer, every flag is off.
            assert!(!app.feature_is_enabled("insights_skills"));
            let reads = pump(&|| answer(1, true), &|app| {
                app.feature_is_enabled("insights_skills")
            });
            assert!(reads >= 1, "the window read the snapshot at start");
            assert!(!app.feature_is_enabled("no_such_flag"));

            let change = || DaemonEvent {
                name: feature_flags::EVENT_FEATURE_FLAGS_CHANGED.to_string(),
                entry_id: None,
                digest: None,
            };
            events.try_send(change()).unwrap();
            let reads = pump(&|| answer(2, false), &|app| {
                app.feature_flags
                    .snapshot()
                    .is_some_and(|s| s.generation == 2)
            });
            assert_eq!(reads, 1, "one change, one read");
            assert!(!app.feature_is_enabled("insights_skills"));

            // An answer about another profile is not this build's: nothing held.
            events.try_send(change()).unwrap();
            pump(
                &|| {
                    let mut other = answer(3, true);
                    other["profile_digest"] = serde_json::json!("0".repeat(64));
                    other
                },
                &|app| app.feature_flags.snapshot().is_none(),
            );
            assert!(!app.feature_is_enabled("insights_skills"));
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}
