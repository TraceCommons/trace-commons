//! Routing assertions that need the daemon, kept here because the overlay
//! moved to `trace-commons-contributor-sources`, which must not depend on
//! this crate. Only public APIs are used; the fixtures mirror the ones in
//! that crate's `routing::enriched` tests.

use chrono::TimeZone;

use super::{FixedLedger, RoutedExchange, enriched::RoutingEnrichedSource};
use crate::source::TraceSource;

fn row(session: Option<&str>, offset: i64) -> RoutedExchange {
    RoutedExchange {
        id: None,
        started_at: chrono::Utc
            .timestamp_opt(1_700_000_000 + offset, 0)
            .unwrap(),
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
        ..Default::default()
    }
}

/// A source returning one transcript with a known conversation id.
struct StubSource {
    conversation_id: Option<String>,
}

impl TraceSource for StubSource {
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

/// A transcript whose final call NEAR AI passed on to another provider,
/// loaded through the overlay exactly as discovery loads it, must not
/// come out marked attested. The row is complete in every other respect
/// -- bodies on disk, digests matching -- so the only thing refusing it
/// is the identifier the answer came back under.
///
/// Moved from `routing::enriched::tests` in the sources crate.
#[test]
fn a_transcript_whose_final_call_was_brokered_is_not_marked_attested() {
    use sha2::{Digest as _, Sha256};
    let request = serde_json::json!({
        "model": "anthropic/claude-fable-5",
        "metadata": {
            trace_commons_protocol::admission::REQUEST_METADATA_KEY: "opaque",
        },
    })
    .to_string();
    let response = "data: [DONE]\n\n";
    let reference = "00000000000000000007-000000";
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join(format!("{reference}.req")), &request).expect("req");
    std::fs::write(dir.path().join(format!("{reference}.res")), response).expect("res");
    let mut brokered = row(Some("s-1"), 0);
    brokered.requested_model = Some("anthropic/claude-fable-5".into());
    brokered.served_model = Some("anthropic/claude-fable-5".into());
    brokered.upstream_id = Some("msg_011CejCasYpbXmB5Zqu3fJsc".into());
    brokered.request_sha256 = Some(hex::encode(Sha256::digest(request.as_bytes())));
    brokered.response_sha256 = Some(hex::encode(Sha256::digest(response.as_bytes())));
    brokered.body_ref = Some(reference.into());
    let source = RoutingEnrichedSource::new(
        Box::new(StubSource {
            conversation_id: Some("s-1".into()),
        }),
        std::sync::Arc::new(FixedLedger::new(vec![brokered])),
    )
    .with_attested_bodies(Some(dir.path().to_path_buf()));
    let t = source.load(&a_ref()).expect("loads");
    let mark = crate::daemon::attestation_mark::evaluate(
        &t.routing,
        t.attested_call.as_deref(),
        t.attested_refusal,
    );
    assert_ne!(
        mark.state,
        crate::daemon::attestation_mark::MARK_ATTESTED,
        "a Claude session brokered through NEAR AI was marked attested"
    );
}
