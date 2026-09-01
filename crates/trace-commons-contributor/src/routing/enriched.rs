//! A [`TraceSource`](crate::source::TraceSource) that decorates another.
//!
//! The ledger is not a session. Registering it as a fourth source would make
//! it independently discoverable, queueable, previewable and submittable,
//! because everything downstream keys on a `SessionRef` with a path and a
//! size -- and all three trait methods would have to lie. It is an overlay on
//! sessions the real adapters already build, so it decorates them.
//!
//! Wrapping rather than threading a parameter: there are three production
//! `load` call sites and four public envelope builders, and a parameter would
//! touch all of them. This has one insertion point, `all_sources`.

use std::path::Path;
use std::sync::Arc;

use crate::routing::{RoutedExchange, RoutingLedger};
use crate::source::{SessionRef, SessionTranscript, TraceSource};

/// Wraps a real adapter and attaches the routing overlay to what it loads.
pub struct RoutingEnrichedSource {
    inner: Box<dyn TraceSource>,
    ledger: Arc<dyn RoutingLedger>,
}

impl RoutingEnrichedSource {
    #[must_use]
    pub fn new(inner: Box<dyn TraceSource>, ledger: Arc<dyn RoutingLedger>) -> Self {
        Self { inner, ledger }
    }
}

impl TraceSource for RoutingEnrichedSource {
    fn name(&self) -> &'static str {
        // The adapter's own name. This decorates a source; it is not one, and
        // a session's provenance is the adapter that read it.
        self.inner.name()
    }

    fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
        // Untouched, deliberately. `size_bytes` and `group_modified_at` drive
        // quiescence and eligibility, and a session must be eligible on its
        // own bytes -- never on whether a proxy happened to answer.
        self.inner.discover()
    }

    fn session_for_path(&self, path: &Path) -> Option<std::path::PathBuf> {
        self.inner.session_for_path(path)
    }

    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript> {
        let mut transcript = self.inner.load(r)?;
        // Only ever additive, and only after the real load succeeded. Nothing
        // below can fail the load: no `?`, no early return with an error.
        let Some(session_id) = transcript.conversation_id.clone() else {
            return Ok(transcript);
        };
        let from = transcript
            .started_at
            .unwrap_or(chrono::DateTime::UNIX_EPOCH);
        transcript.routing = self
            .ledger
            .exchanges_since(from)
            .into_iter()
            .filter(|row| row.client_session_id.as_deref() == Some(session_id.as_str()))
            .collect::<Vec<RoutedExchange>>();
        Ok(transcript)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::{FixedLedger, RoutedExchange};
    use chrono::TimeZone;

    fn at(offset: i64) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc
            .timestamp_opt(1_700_000_000 + offset, 0)
            .unwrap()
    }

    fn row(session: Option<&str>, offset: i64) -> RoutedExchange {
        RoutedExchange {
            id: None,
            started_at: at(offset),
            client_session_id: session.map(str::to_string),
            total_ms: Some(1200),
            facade: "anthropic".to_string(),
            backend: "claude-sub".to_string(),
            requested_model: None,
            served_model: None,
            rung: "same_model".to_string(),
            attempts: 1,
            input_tokens: Some(1000),
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: Some(200),
            cost_usd: Some(0.02),
            status: 200,
        }
    }

    /// A source returning one transcript with a known conversation id.
    struct StubSource {
        conversation_id: Option<String>,
    }

    impl crate::source::TraceSource for StubSource {
        fn name(&self) -> &'static str {
            "stub"
        }
        fn discover(&self) -> anyhow::Result<Vec<crate::source::SessionRef>> {
            Ok(Vec::new())
        }
        fn load(
            &self,
            _r: &crate::source::SessionRef,
        ) -> anyhow::Result<crate::source::SessionTranscript> {
            Ok(crate::source::SessionTranscript {
                conversation_id: self.conversation_id.clone(),
                ..Default::default()
            })
        }
    }

    fn a_ref() -> crate::source::SessionRef {
        // Built the way the existing adapter tests build a synthetic ref
        // (see `commands.rs::submit_scope_tests::a_ref`); this type has no
        // `Default` and none is added here.
        crate::source::SessionRef {
            source: crate::source::SOURCE_CLAUDE_CODE,
            path: std::path::Path::new("/store/s.jsonl").to_path_buf(),
            project: None,
            cwd: None,
            started_at: None,
            size_bytes: 0,
            group_modified_at: None,
            group_member_count: 0,
        }
    }

    #[test]
    fn only_the_rows_for_this_session_are_attached() {
        let ledger = FixedLedger::new(vec![
            row(Some("s-1"), 0),
            row(Some("s-2"), 10),
            row(Some("s-1"), 20),
        ]);
        let source = RoutingEnrichedSource::new(
            Box::new(StubSource {
                conversation_id: Some("s-1".into()),
            }),
            std::sync::Arc::new(ledger),
        );
        let t = source.load(&a_ref()).expect("loads");
        assert_eq!(t.routing.len(), 2, "the other session's rows are not ours");
    }

    #[test]
    fn a_session_we_cannot_identify_gets_no_overlay() {
        // A transcript with no conversation id cannot be joined to anything,
        // and guessing would attribute another session's cost to this trace.
        let ledger = FixedLedger::new(vec![row(Some("s-1"), 0)]);
        let source = RoutingEnrichedSource::new(
            Box::new(StubSource {
                conversation_id: None,
            }),
            std::sync::Arc::new(ledger),
        );
        assert!(source.load(&a_ref()).expect("loads").routing.is_empty());
    }

    #[test]
    fn rows_that_name_no_session_are_never_attached() {
        // A proxy older than the release that records the session id, and the
        // proxy's own auxiliary requests, both land here.
        let ledger = FixedLedger::new(vec![row(None, 0), row(None, 10)]);
        let source = RoutingEnrichedSource::new(
            Box::new(StubSource {
                conversation_id: Some("s-1".into()),
            }),
            std::sync::Arc::new(ledger),
        );
        assert!(source.load(&a_ref()).expect("loads").routing.is_empty());
    }

    #[test]
    fn an_empty_overlay_leaves_the_transcript_as_the_inner_source_built_it() {
        let source = RoutingEnrichedSource::new(
            Box::new(StubSource {
                conversation_id: Some("s-1".into()),
            }),
            std::sync::Arc::new(FixedLedger::new(Vec::new())),
        );
        let t = source.load(&a_ref()).expect("loads");
        assert!(t.routing.is_empty());
        assert_eq!(t.conversation_id.as_deref(), Some("s-1"));
    }
}
