//! Turning an approved queue entry into an upload.
//!
//! The uploader adds three things to the shared submit pipeline, all of them
//! consequences of the upload being unattended:
//!
//! 1. **A re-hash guard.** The contributor approves a description of a
//!    session -- a project, a size, a time. Digests batch every few hours, so
//!    the file can grow between the offer and the approval. If it did, the
//!    approval does not cover the current content, so nothing is uploaded and
//!    a fresh offer is made instead. This is the central consent property of
//!    the whole daemon.
//! 2. **Revocation checks.** A cached claim stays valid for minutes after a
//!    logout, so enrollment is re-checked immediately before every upload
//!    rather than once at startup.
//! 3. **Volume caps.** A background process spending the contributor's
//!    bandwidth and privacy-filter budget stops at a daily ceiling.

use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::health::{
    HealthState, LABEL_CLAIM_MINT_FAILED, LABEL_DAILY_CAP_REACHED, LABEL_INGEST_UNREACHABLE,
    LABEL_NEAR_AI_NOTICE_PENDING, LABEL_NOT_LOGGED_IN, LABEL_PII_FILTER_UNAVAILABLE,
};
use super::queue::QueueEntry;
use super::settings::DaemonSettings;
use super::state::DaemonState;
use crate::config::ConfigStore;
use crate::source::{SessionRef, TraceSource};
use crate::submit::{SubmitContext, SubmitOutcome};

#[derive(Debug, PartialEq, Eq)]
pub enum UploadDecision {
    Uploaded {
        submission_id: Uuid,
    },
    /// Already delivered previously; nothing sent.
    AlreadySubmitted {
        submission_id: Uuid,
    },
    /// The session changed after it was offered. Nothing was sent, and
    /// `new_hash` describes what is on disk now.
    Superseded {
        new_hash: String,
    },
    /// The pipeline declined to send this, fail-closed.
    Refused {
        reason_label: String,
    },
    /// Network or auth failure.
    Failed {
        reason_label: String,
    },
    /// A daily volume cap is in force.
    CapReached,
}

/// Whether one more upload of `size_bytes` fits inside today's budget.
///
/// Call `DaemonState::roll_day` first; this deliberately does not mutate.
pub fn cap_check(state: &DaemonState, size_bytes: u64, settings: &DaemonSettings) -> bool {
    if state.uploads_today >= settings.max_uploads_per_day {
        return false;
    }
    state.bytes_today.saturating_add(size_bytes) <= settings.max_bytes_per_day
}

/// Map a pipeline outcome onto a daemon decision, so the queue records a
/// fixed label rather than pipeline internals.
fn decision_for(outcome: SubmitOutcome) -> UploadDecision {
    match outcome {
        SubmitOutcome::Submitted { submission_id, .. } => {
            UploadDecision::Uploaded { submission_id }
        }
        SubmitOutcome::AlreadySubmitted { submission_id, .. } => {
            UploadDecision::AlreadySubmitted { submission_id }
        }
        SubmitOutcome::SkippedParseFailure { reason_label } => {
            UploadDecision::Refused { reason_label }
        }
        SubmitOutcome::Refused { reason_label, .. } => UploadDecision::Refused { reason_label },
        SubmitOutcome::Failed { reason_label } => UploadDecision::Failed { reason_label },
    }
}

/// The health label a decision implies, if any.
pub fn health_label_for(decision: &UploadDecision) -> Option<&'static str> {
    match decision {
        UploadDecision::CapReached => Some(LABEL_DAILY_CAP_REACHED),
        UploadDecision::Refused { reason_label } => match reason_label.as_str() {
            "pii-filter-unavailable" => Some(LABEL_PII_FILTER_UNAVAILABLE),
            LABEL_NEAR_AI_NOTICE_PENDING => Some(LABEL_NEAR_AI_NOTICE_PENDING),
            LABEL_NOT_LOGGED_IN => Some(LABEL_NOT_LOGGED_IN),
            _ => None,
        },
        UploadDecision::Failed { reason_label } => match reason_label.as_str() {
            "claim-mint-failed" => Some(LABEL_CLAIM_MINT_FAILED),
            _ => Some(LABEL_INGEST_UNREACHABLE),
        },
        _ => None,
    }
}

/// Whether this store still holds a usable enrollment.
///
/// Checked immediately before every upload, not once at startup: a cached
/// claim outlives a logout by minutes, and the receipts file it would append
/// to is gone.
pub fn enrollment_is_live(store: &ConfigStore) -> bool {
    match store.load_config() {
        Ok(Some(_)) => store.load_device_key().ok().flatten().is_some(),
        _ => false,
    }
}

pub struct Uploader<'a, 'ctx> {
    pub ctx: &'a mut SubmitContext<'ctx>,
    pub store: &'a ConfigStore,
    pub settings: &'a DaemonSettings,
    pub state: &'a mut DaemonState,
    pub health: &'a mut HealthState,
}

impl Uploader<'_, '_> {
    /// Upload one queue entry, or explain why not.
    pub async fn upload_entry(
        &mut self,
        source: &dyn TraceSource,
        session_ref: &SessionRef,
        entry: &QueueEntry,
        now: DateTime<Utc>,
    ) -> Result<UploadDecision> {
        if !enrollment_is_live(self.store) {
            self.health.fail(LABEL_NOT_LOGGED_IN, now);
            return Ok(UploadDecision::Refused {
                reason_label: LABEL_NOT_LOGGED_IN.to_string(),
            });
        }
        // Enrollment is live, so retract the not-logged-in condition if it was set.
        self.health.resolve(LABEL_NOT_LOGGED_IN);

        if self.settings.near_ai.is_some() && !self.store.near_ai_notice_shown() {
            // The one-time notice is delivered interactively. Under a service
            // manager its output goes to a log nobody reads, so a daemon that
            // consumed the marker would send the contributor's text to a third
            // party with the notice never actually delivered.
            self.health.fail(LABEL_NEAR_AI_NOTICE_PENDING, now);
            return Ok(UploadDecision::Refused {
                reason_label: LABEL_NEAR_AI_NOTICE_PENDING.to_string(),
            });
        }
        // Near-AI notice requirement is met (either not configured or already shown),
        // so retract the notice-pending condition if it was set.
        self.health.resolve(LABEL_NEAR_AI_NOTICE_PENDING);

        self.state.roll_day(now);
        if !cap_check(self.state, entry.size_bytes, self.settings) {
            self.health.fail(LABEL_DAILY_CAP_REACHED, now);
            return Ok(UploadDecision::CapReached);
        }
        // Cap check passed, so retract the daily-cap-reached condition if it was set.
        self.health.resolve(LABEL_DAILY_CAP_REACHED);

        // Re-read and re-hash. The approval was for the content described by
        // entry.session_hash; if the file has moved on, that approval does not
        // transfer to the new content.
        // `source.load` reads the whole session file and hashes it -- blocking,
        // non-yielding work with no `.await` of its own, run once per approved
        // entry from inside the supervisor's task. Off-worker for the same
        // reason `watcher::tick`'s scan is; see `super::run_blocking`'s doc.
        let transcript = match super::run_blocking(|| source.load(session_ref)) {
            Ok(t) => t,
            Err(_) => {
                return Ok(UploadDecision::Refused {
                    reason_label: "parse-failed".to_string(),
                });
            }
        };
        if transcript.session_hash != entry.session_hash {
            return Ok(UploadDecision::Superseded {
                new_hash: transcript.session_hash,
            });
        }

        let outcome = self.ctx.submit_one(source, session_ref).await?;
        let decision = decision_for(outcome);

        match &decision {
            UploadDecision::Uploaded { .. } => {
                self.state
                    .record_upload(&entry.path, &entry.session_hash, entry.size_bytes, now);
                self.health.clear();
            }
            other => {
                if let Some(label) = health_label_for(other) {
                    self.health.fail(label, now);
                }
            }
        }
        Ok(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;
    use crate::daemon::queue::{QueueEntry, QueueState, entry_id_for};
    use crate::source::claude_code::ClaudeCodeSource;
    use std::path::PathBuf;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    fn settings() -> DaemonSettings {
        DaemonSettings::default()
    }

    #[test]
    fn cap_check_rejects_past_the_daily_upload_count() {
        let mut st = DaemonState::new();
        st.uploads_today = 50;
        assert!(!cap_check(&st, 10, &settings()));
    }

    #[test]
    fn cap_check_rejects_past_the_daily_byte_budget() {
        let mut st = DaemonState::new();
        st.bytes_today = 209_715_200;
        assert!(!cap_check(&st, 1, &settings()));
    }

    #[test]
    fn cap_check_rejects_an_upload_that_would_cross_the_byte_budget() {
        let mut st = DaemonState::new();
        st.bytes_today = 209_715_100;
        assert!(!cap_check(&st, 200, &settings()));
    }

    #[test]
    fn cap_check_allows_a_normal_upload() {
        assert!(cap_check(&DaemonState::new(), 1024, &settings()));
    }

    #[test]
    fn a_failed_upload_maps_to_an_ingest_health_label() {
        let d = UploadDecision::Failed {
            reason_label: "upload-failed".into(),
        };
        assert_eq!(health_label_for(&d), Some(LABEL_INGEST_UNREACHABLE));
    }

    #[test]
    fn a_claim_failure_is_distinguished_from_an_ingest_failure() {
        let d = UploadDecision::Failed {
            reason_label: "claim-mint-failed".into(),
        };
        assert_eq!(health_label_for(&d), Some(LABEL_CLAIM_MINT_FAILED));
    }

    #[test]
    fn a_successful_upload_implies_no_health_failure() {
        let d = UploadDecision::Uploaded {
            submission_id: Uuid::nil(),
        };
        assert_eq!(health_label_for(&d), None);
    }

    /// A claude-code session in a tempdir that can be grown on demand.
    struct GrowingSession {
        _dir: tempfile::TempDir,
        root: PathBuf,
        path: PathBuf,
    }

    impl GrowingSession {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("projects");
            let project = root.join("-Users-testuser-code-myproj");
            std::fs::create_dir_all(&project).unwrap();
            let path = project.join("33333333-3333-3333-3333-333333333333.jsonl");
            std::fs::write(
                &path,
                "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"first question\"},\
                 \"cwd\":\"/Users/testuser/code/myproj\",\"timestamp\":\"2026-08-08T10:00:00Z\",\
                 \"version\":\"2.0.1\",\"sessionId\":\"33333333-3333-3333-3333-333333333333\",\
                 \"uuid\":\"a1\"}\n",
            )
            .unwrap();
            Self {
                _dir: dir,
                root,
                path,
            }
        }

        fn source(&self) -> ClaudeCodeSource {
            ClaudeCodeSource::new(self.root.clone())
        }

        fn session_ref(&self) -> SessionRef {
            self.source().discover().unwrap().remove(0)
        }

        fn current_hash(&self) -> String {
            self.source()
                .load(&self.session_ref())
                .unwrap()
                .session_hash
        }

        fn append_more_events(&self) {
            use std::io::Write as _;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&self.path)
                .unwrap();
            f.write_all(
                b"{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"much later work\"},\
                  \"cwd\":\"/Users/testuser/code/myproj\",\"timestamp\":\"2026-08-08T18:00:00Z\",\
                  \"version\":\"2.0.1\",\"sessionId\":\"33333333-3333-3333-3333-333333333333\",\
                  \"uuid\":\"a2\"}\n",
            )
            .unwrap();
        }

        fn entry(&self, hash: &str) -> QueueEntry {
            QueueEntry {
                entry_id: entry_id_for(hash),
                session_hash: hash.to_string(),
                source: "claude-code".into(),
                project_key: "/Users/testuser/code/myproj".into(),
                project_label: "myproj".into(),
                path: self.path.clone(),
                size_bytes: std::fs::metadata(&self.path).unwrap().len(),
                discovered_at: at("2026-08-08T12:00:00Z"),
                state: QueueState::Approved,
                reason_label: None,
                attempts: 0,
                retry_after: None,
                submission_id: None,
            }
        }
    }

    fn dry_run_opts() -> crate::submit::SubmitOptions {
        crate::submit::SubmitOptions {
            dry_run: true,
            pii_filter: None,
            no_reasoning: false,
            machine_readable: true,
            unenrolled_preview: false,
            remediate_quarantined: false,
        }
    }

    #[tokio::test]
    async fn upload_refuses_when_the_session_grew_after_it_was_offered() {
        // The central consent property: approve a 1-event session, never
        // ship one that has since gained an afternoon of work.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        };
        store.save_config(&cfg).unwrap();

        let offered_hash = session.current_hash();
        let entry = session.entry(&offered_hash);
        session.append_more_events();

        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };

        let decision = up
            .upload_entry(
                &session.source(),
                &session.session_ref(),
                &entry,
                at("2026-08-08T16:00:00Z"),
            )
            .await
            .unwrap();

        match decision {
            UploadDecision::Superseded { new_hash } => {
                assert_ne!(new_hash, entry.session_hash);
                assert_eq!(new_hash, session.current_hash());
            }
            other => panic!("expected Superseded, got {other:?}"),
        }
        assert_eq!(
            state.uploads_today, 0,
            "nothing may be uploaded when the hash no longer matches"
        );
    }

    #[tokio::test]
    async fn upload_proceeds_when_the_hash_still_matches() {
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        };
        store.save_config(&cfg).unwrap();

        let entry = session.entry(&session.current_hash());
        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };

        let decision = up
            .upload_entry(
                &session.source(),
                &session.session_ref(),
                &entry,
                at("2026-08-08T16:00:00Z"),
            )
            .await
            .unwrap();

        assert!(
            matches!(decision, UploadDecision::Uploaded { .. }),
            "got {decision:?}"
        );
        assert_eq!(state.uploads_today, 1);
        assert!(health.ok());
    }

    #[tokio::test]
    async fn upload_refuses_once_the_enrollment_is_gone() {
        // A cached claim outlives a logout by minutes.
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        };
        store.save_config(&cfg).unwrap();
        let entry = session.entry(&session.current_hash());
        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();

        // Log out underneath the running context.
        store.wipe().unwrap();

        let mut state = DaemonState::new();
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        let decision = up
            .upload_entry(
                &session.source(),
                &session.session_ref(),
                &entry,
                at("2026-08-08T16:00:00Z"),
            )
            .await
            .unwrap();

        assert_eq!(
            decision,
            UploadDecision::Refused {
                reason_label: LABEL_NOT_LOGGED_IN.to_string()
            }
        );
        assert_eq!(
            health.last_error_label.as_deref(),
            Some(LABEL_NOT_LOGGED_IN)
        );
        assert_eq!(state.uploads_today, 0);
    }

    #[tokio::test]
    async fn upload_stops_at_the_daily_cap() {
        let session = GrowingSession::new();
        let (_d, store) = temp_store();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "http://issuer.invalid".into(),
            ingest_url: "http://ingest.invalid".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        };
        store.save_config(&cfg).unwrap();

        let entry = session.entry(&session.current_hash());
        let opts = dry_run_opts();
        let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
        let mut state = DaemonState::new();
        state.roll_day(at("2026-08-08T16:00:00Z"));
        state.uploads_today = 50;
        let mut health = HealthState::default();
        let settings = settings();
        let mut up = Uploader {
            ctx: &mut ctx,
            store: &store,
            settings: &settings,
            state: &mut state,
            health: &mut health,
        };
        let decision = up
            .upload_entry(
                &session.source(),
                &session.session_ref(),
                &entry,
                at("2026-08-08T16:00:00Z"),
            )
            .await
            .unwrap();

        assert_eq!(decision, UploadDecision::CapReached);
        assert_eq!(
            health.last_error_label.as_deref(),
            Some(LABEL_DAILY_CAP_REACHED)
        );
    }
}
