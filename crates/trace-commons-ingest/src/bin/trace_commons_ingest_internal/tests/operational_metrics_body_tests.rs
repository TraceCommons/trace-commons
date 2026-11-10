// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Prometheus body served at the operational metrics route, pinned byte
//! for byte. Metric names, help text, label sets and order are scraped by
//! operators' dashboards, so a change to any of them must show up here as a
//! deliberate edit to the golden file. The tenant storage ref is replaced by
//! a placeholder so the file does not depend on the hashing key.

use super::*;

const GOLDEN: &str = include_str!("operational_metrics_body.golden.txt");

#[tokio::test]
async fn operational_metrics_body_matches_the_golden_file() {
    let temp = tempfile::tempdir().expect("temp dir");
    let mut state = test_state(temp.path().to_path_buf());
    {
        let state = Arc::make_mut(&mut state);
        state.credit_settlement_require_central_issuer_profile = true;
        state.export_job_scheduler = Some(TraceExportJobSchedulerConfig {
            worker_token: SecretString::from("golden-export-token".to_string()),
            interval: StdDuration::from_secs(90),
            dataset_kind: Some("ranker_training_candidates".to_string()),
            run_queued_max_jobs: 3,
            retry_failed_max_jobs: 2,
            retry_failed_max_retry_count: 4,
            retry_failed_base_delay_seconds: 5,
            retry_failed_max_delay_seconds: 60,
            retry_failed_reason: "golden export retry".to_string(),
        });
    }
    for _ in 0..2 {
        let envelope = low_risk_envelope().await;
        let _ = test_submit(state.clone(), "token-a", envelope)
            .await
            .expect("submit");
    }

    let Json(response) =
        operational_summary_handler(State(state.clone()), auth_headers("admin-token-a"))
            .await
            .expect("admin can read the operational summary");
    let (body, metric_count) = trace_operational_metrics_body(&response);

    let samples = body.lines().filter(|line| !line.starts_with('#')).count();
    assert_eq!(metric_count, samples, "the count is the number of samples");

    let actual = body.replace(&tenant_storage_ref("tenant-a"), "<tenant>");
    if actual != GOLDEN {
        let path = std::env::temp_dir().join("operational_metrics_body.actual.txt");
        std::fs::write(&path, &actual).expect("write the actual body");
        panic!(
            "the operational metrics body changed; the new body is at {}",
            path.display()
        );
    }
}
