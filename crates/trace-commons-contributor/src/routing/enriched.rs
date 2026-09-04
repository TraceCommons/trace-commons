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

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::routing::{RoutedExchange, RoutingLedger};
use crate::source::{SessionRef, SessionTranscript, TraceSource};

/// Wraps a real adapter and attaches the routing overlay to what it loads.
pub struct RoutingEnrichedSource {
    inner: Box<dyn TraceSource>,
    ledger: Arc<dyn RoutingLedger>,
    /// The proxy's verbatim body store, when this deployment carries attested
    /// bodies. `None` -- the default -- attaches routing metadata only, which
    /// is what every deployment does today.
    bodies_dir: Option<PathBuf>,
}

impl RoutingEnrichedSource {
    #[must_use]
    pub fn new(inner: Box<dyn TraceSource>, ledger: Arc<dyn RoutingLedger>) -> Self {
        Self {
            inner,
            ledger,
            bodies_dir: None,
        }
    }

    /// Also carry the final call's verbatim bodies, read from `dir`.
    ///
    /// Opt-in and off by default, deliberately. Carrying bodies is the one
    /// thing this overlay does that puts session *content* into a trace
    /// rather than metadata about it, so it is never switched on implicitly
    /// by a ledger being present.
    #[must_use]
    pub fn with_attested_bodies(mut self, dir: Option<PathBuf>) -> Self {
        self.bodies_dir = dir;
        self
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
            .filter(|row| {
                row.client_session_id
                    .as_deref()
                    .is_some_and(|id| names_the_same_session(id, &session_id))
            })
            .collect::<Vec<RoutedExchange>>();
        // Bodies last, and only from the rows already joined to this session:
        // a body from a hop that belongs to a different session would be
        // attested content this transcript never produced. Every failure
        // resolves to `None`, in keeping with this module's rule that nothing
        // here can fail a load.
        transcript.attested_call = self.bodies_dir.as_deref().and_then(|dir| {
            super::attested::attested_final_call(&transcript.routing, dir)
                .ok()
                .map(Arc::new)
        });
        Ok(transcript)
    }
}

/// Whether a ledger row's session id and our `conversation_id` name the same
/// session.
///
/// Equality first, which is the Claude Code case: it sends its session UUID on
/// `x-claude-code-session-id`, and our transcript is addressed by the same
/// UUID -- the session file's own stem.
///
/// Codex does not line up that way. Our `conversation_id` is the rollout
/// file's stem, `rollout-<timestamp>-<uuid>`, because that is the identifier
/// discovery and the queue already address the session by. The client sends
/// the bare UUID. Two spellings of one session, and an equality join matches
/// nothing.
///
/// **Nothing about that failure is visible.** IronWire cannot see our
/// spelling, so it cannot detect the mismatch; we would see an empty routing
/// list, which is exactly what a contributor who never used Codex sees. The
/// whole path can be correct and produce nothing, permanently.
///
/// So a bare UUID also matches a stem that ends with `-<that UUID>`. The
/// suffix must be a *whole* UUID and must sit on a `-` boundary: 36 characters
/// of `8-4-4-4-12` hex is globally unique, so a full-UUID suffix match cannot
/// collide, while a looser suffix rule could join two unrelated sessions.
///
/// Deliberately not fixed by changing `conversation_id`: it is the address
/// discovery, the queue and the envelope already use, and moving it would move
/// identity for every Codex session ever recorded.
fn names_the_same_session(row_id: &str, conversation_id: &str) -> bool {
    // Neither side may be empty. A client that always sets its session
    // header and leaves it blank records `client_session_id: ""` -- IronWire
    // keys its precedence on the header being *present* -- and an adapter
    // that read an empty id from a session document would hold `Some("")`.
    // Either alone is harmless; together the equality arm below would join
    // them and attribute one session's routing rows and cost to another.
    // The producers are each fixed at their own end, and this is the last
    // line of defence, which should not depend on all of them staying fixed.
    if row_id.is_empty() || conversation_id.is_empty() {
        return false;
    }
    if row_id == conversation_id {
        return true;
    }
    if !is_uuid(row_id) {
        return false;
    }
    conversation_id
        .len()
        .checked_sub(row_id.len() + 1)
        .is_some_and(|boundary| {
            conversation_id.as_bytes()[boundary] == b'-' && conversation_id.ends_with(row_id)
        })
}

/// The canonical `8-4-4-4-12` hyphenated form, lowercase or upper.
///
/// Strict on purpose: this is what makes a suffix match safe, so a looser
/// reading here would quietly widen the join.
fn is_uuid(value: &str) -> bool {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    let mut parts = value.split('-');
    for width in GROUPS {
        let Some(part) = parts.next() else {
            return false;
        };
        if part.len() != width || !part.chars().all(|c| c.is_ascii_hexdigit()) {
            return false;
        }
    }
    parts.next().is_none()
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
            upstream_id: None,
            request_sha256: None,
            response_sha256: None,
            body_ref: None,
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
            declared_source: None,
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
    fn a_row_naming_an_empty_session_is_never_attached() {
        // An empty `client_session_id` is not "no session": it is `Some("")`,
        // and it reaches the join by equality against any transcript whose
        // own id is also empty. Both halves are reachable from real
        // producers -- IronWire's session-id precedence is keyed on the
        // header being present, so a client that always sends it and leaves
        // it blank records an empty id. Joining them would put another
        // session's routing rows and cost on this trace.
        for id in ["", "s-1"] {
            let ledger = FixedLedger::new(vec![row(Some(""), 0), row(Some("  "), 10)]);
            let source = RoutingEnrichedSource::new(
                Box::new(StubSource {
                    conversation_id: Some(id.into()),
                }),
                std::sync::Arc::new(ledger),
            );
            let t = source.load(&a_ref()).expect("loads");
            assert!(
                t.routing.is_empty(),
                "an empty ledger session id joined a transcript with id {id:?}"
            );
        }
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

    /// A page a real IronWire (0.1.0, commit 4024619) served on 2026-09-03,
    /// captured from `GET /_ironwire/log` with one request per facade: an
    /// Anthropic Messages call carrying `x-claude-code-session-id` and a Chat
    /// Completions call carrying `session-id`. Not hand-written, so the
    /// field names and shapes are the proxy's own -- every other test in
    /// this module builds its rows from our side of the contract.
    const REAL_PAGE: &str = include_str!("../../tests/fixtures/ironwire/log-page-2026-09-03.json");

    fn real_rows() -> Vec<RoutedExchange> {
        #[derive(serde::Deserialize)]
        struct Page {
            exchanges: Vec<RoutedExchange>,
        }
        serde_json::from_str::<Page>(REAL_PAGE)
            .expect("the proxy's page parses as our row type")
            .exchanges
    }

    #[test]
    fn a_page_a_real_proxy_served_joins_each_native_spelling_to_its_own_session() {
        let rows = real_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, Some(1), "the cursor is present");
        assert_eq!(rows[0].facade, "anthropic");
        assert_eq!(rows[1].facade, "openai");
        assert_eq!(rows[0].input_tokens, Some(7));
        assert_eq!(rows[0].output_tokens, Some(3));

        // Claude Code: the transcript stem is the UUID the client sent.
        let claude = RoutingEnrichedSource::new(
            Box::new(StubSource {
                conversation_id: Some("79f2f947-522e-4780-8518-33155a18152e".into()),
            }),
            std::sync::Arc::new(FixedLedger::new(rows.clone())),
        );
        let t = claude.load(&a_ref()).expect("loads");
        assert_eq!(t.routing.len(), 1);
        assert_eq!(t.routing[0].id, Some(1));

        // Codex: the transcript stem is the rollout file name, and the client
        // sent only the UUID at its end.
        let codex = RoutingEnrichedSource::new(
            Box::new(StubSource {
                conversation_id: Some(
                    "rollout-2026-09-03T11-56-43-019921c3-6a5c-7d4e-9f00-aaaaaaaaaaaa".into(),
                ),
            }),
            std::sync::Arc::new(FixedLedger::new(rows)),
        );
        let t = codex.load(&a_ref()).expect("loads");
        assert_eq!(t.routing.len(), 1);
        assert_eq!(t.routing[0].id, Some(2));
    }
}

#[cfg(test)]
mod session_id_matching {
    use super::{is_uuid, names_the_same_session};

    const UUID: &str = "5db811ed-ce4a-45a7-ab00-56890e111668";
    const STEM: &str = "rollout-2026-09-02T10-14-22-5db811ed-ce4a-45a7-ab00-56890e111668";

    /// Claude Code: the client's header and our transcript stem are the same
    /// UUID, so equality already worked and must keep working.
    #[test]
    fn an_identical_id_matches() {
        assert!(names_the_same_session(UUID, UUID));
    }

    /// Codex: the bug this exists for. The client sends the bare UUID; we
    /// address the session by the rollout stem.
    #[test]
    fn a_bare_uuid_matches_the_rollout_stem_that_ends_with_it() {
        assert!(
            names_the_same_session(UUID, STEM),
            "a Codex row must join the session it came from"
        );
    }

    /// The safety property. A full UUID is globally unique, so a whole-UUID
    /// suffix cannot collide -- but a partial one could, and must not match.
    #[test]
    fn a_partial_uuid_suffix_does_not_match() {
        assert!(!names_the_same_session("56890e111668", STEM));
        assert!(!names_the_same_session("ab00-56890e111668", STEM));
    }

    /// The suffix has to sit on a separator, so a stem that merely *ends with*
    /// the characters does not join.
    #[test]
    fn a_suffix_that_is_not_on_a_boundary_does_not_match() {
        let glued = format!("rollout-2026x{UUID}");
        assert!(!names_the_same_session(UUID, &glued));
    }

    /// A different session's UUID inside a stem is still a different session.
    #[test]
    fn a_different_uuid_does_not_match() {
        assert!(!names_the_same_session(
            "00000000-0000-0000-0000-000000000000",
            STEM
        ));
    }

    /// Only a UUID earns the suffix rule. An arbitrary short id must not join
    /// every stem that happens to end with it.
    #[test]
    fn a_non_uuid_row_id_never_suffix_matches() {
        assert!(!names_the_same_session("22", "rollout-2026-09-02T10-14-22"));
        assert!(!names_the_same_session("session", "a-session"));
    }

    #[test]
    fn uuid_recognition_is_strict() {
        assert!(is_uuid(UUID));
        assert!(
            is_uuid("5DB811ED-CE4A-45A7-AB00-56890E111668"),
            "upper case"
        );
        assert!(
            !is_uuid("5db811ed-ce4a-45a7-ab00-56890e11166"),
            "short group"
        );
        assert!(!is_uuid("5db811edce4a45a7ab0056890e111668"), "unhyphenated");
        assert!(!is_uuid(&format!("{UUID}-extra")), "trailing group");
        assert!(!is_uuid("5db811ed-ce4a-45a7-ab00-56890e11166g"), "non-hex");
    }
}
