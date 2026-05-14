//! Integration test for the pilot-bootstrap submitter + sidecar pair. Spins
//! up a `wiremock` server that mimics the `trace-commons-ingest` `/v1/traces`
//! endpoint, points the submitter at it, runs a small batch of drafts, and
//! asserts the sidecar captures one row per submission with the expected
//! gate_decision values.

#[path = "../src/bin/pilot_bootstrap/hf_dataset.rs"]
mod hf_dataset;

#[path = "../src/bin/pilot_bootstrap/translators.rs"]
mod translators;

#[path = "../src/bin/pilot_bootstrap/submitter.rs"]
mod submitter;

#[path = "../src/bin/pilot_bootstrap/sidecar.rs"]
mod sidecar;

use serde_json::json;
use sidecar::{Sidecar, SidecarRecord};
use submitter::Submitter;
use translators::SubmissionDraft;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn draft(seed: u8) -> SubmissionDraft {
    let mut id = [0u8; 16];
    id[15] = seed;
    SubmissionDraft {
        submission_id: hex::encode(id),
        trace_body: format!("trace body {seed}"),
        source_dataset: "test/dataset".into(),
        source_row_id: format!("row-{seed}"),
        source_domain_tag: "test/synthetic".into(),
    }
}

#[tokio::test]
async fn submitter_records_accepted_outcomes_in_sidecar() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/traces"))
        .and(header("authorization", "Bearer test-token"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({
                "status": "accepted",
                "credit_points_pending": 0.0,
                "explanation": []
            })),
        )
        .mount(&server)
        .await;

    let tmp = tempfile::tempdir().unwrap();
    let sidecar_path = tmp.path().join("sidecar.jsonl");
    let sidecar = Sidecar::open(&sidecar_path).unwrap();
    let submitter = Submitter::new(
        server.uri(),
        "test-token".to_string(),
        // 0.0 rate disables sleep so the test runs fast.
        0.0,
    )
    .unwrap();

    for i in 0..10u8 {
        let d = draft(i);
        let outcome = submitter.submit(&d).await.unwrap();
        assert_eq!(outcome.http_status, Some(200));
        assert_eq!(outcome.gate_decision, "accepted");
        let record = SidecarRecord {
            submission_id: d.submission_id.clone(),
            source_dataset: d.source_dataset.clone(),
            source_row_id: d.source_row_id.clone(),
            source_domain_tag: d.source_domain_tag.clone(),
            http_status: outcome.http_status,
            gate_decision: outcome.gate_decision.clone(),
            elapsed_ms: outcome.elapsed_ms,
            timestamp: chrono::Utc::now(),
        };
        sidecar.write(&record).unwrap();
    }

    let contents = std::fs::read_to_string(&sidecar_path).unwrap();
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 10, "one sidecar line per submission");
    for line in &lines {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(v["gate_decision"], "accepted");
        assert_eq!(v["http_status"], 200);
        assert_eq!(v["source_dataset"], "test/dataset");
    }
}

#[tokio::test]
async fn submitter_records_rejected_for_4xx() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/traces"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let submitter = Submitter::new(server.uri(), "test-token".into(), 0.0).unwrap();
    let outcome = submitter.submit(&draft(1)).await.unwrap();
    assert_eq!(outcome.http_status, Some(403));
    assert_eq!(outcome.gate_decision, "rejected");
}
