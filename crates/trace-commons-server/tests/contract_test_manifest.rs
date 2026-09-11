use std::collections::BTreeSet;

#[test]
fn redesign_contract_manifest_is_complete_and_well_formed() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/redesign/contract-test-manifest.json"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).expect("manifest exists"))
            .expect("manifest is valid JSON");
    assert_eq!(value["schema"], "trace_commons.contract_test_manifest.v1");
    let statuses = ["planned", "partial", "passing", "deferred"];
    let mut actual_contracts = BTreeSet::new();
    for group in value["contract_groups"]
        .as_array()
        .expect("contract_groups is an array")
    {
        let status = group["evidence_status"]
            .as_str()
            .expect("group has evidence_status");
        assert!(statuses.contains(&status));
        for id in group["contract_ids"]
            .as_array()
            .expect("contract_ids is an array")
        {
            assert!(
                actual_contracts.insert(id.as_str().expect("contract ID is a string")),
                "contract ID is unique"
            );
        }
    }
    let expected_contracts = [
        "SYS-001", "SYS-002", "SYS-003", "SYS-004", "SYS-005", "SYS-006", "SYS-007", "SYS-008",
        "SYS-009", "SYS-010", "AUTH-001", "AUTH-002", "AUTH-003", "AUTH-004", "AUTH-005",
        "SUB-001", "SUB-002", "SUB-003", "SUB-004", "SUB-005", "SUB-006", "SUB-007", "BND-001",
        "BND-002", "BND-003", "BND-004", "REV-001", "REV-002", "REV-003", "REV-004", "SCR-001",
        "SCR-002", "SCR-003", "SCR-004", "SCR-005", "STL-001", "STL-002", "STL-003", "STL-004",
        "STL-005", "RUN-001", "RUN-002", "RUN-003", "RUN-004", "RUN-005", "GRD-001", "GRD-002",
        "GRD-003", "GRD-004", "STA-001", "STA-002", "STA-003", "STA-004", "STA-005", "CRD-001",
        "CRD-002", "CRD-003", "CRD-004", "CRD-005", "LIF-001", "LIF-002", "LIF-003", "EXP-001",
        "EXP-002", "EXP-003", "EXP-004", "COM-001", "COM-002", "OPS-001", "OPS-002", "OPS-003",
        "OPS-004", "OPS-005", "OPS-006", "LAB-001", "LAB-002", "LAB-003", "CMP-001", "CMP-002",
        "CMP-003", "CMP-004",
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    assert_eq!(actual_contracts, expected_contracts);

    let scenarios = value["scenarios"]
        .as_array()
        .expect("scenarios is an array");
    assert_eq!(scenarios.len(), 15);
    for (index, scenario) in scenarios.iter().enumerate() {
        assert_eq!(scenario["scenario_id"], format!("SCN-{:03}", index + 1));
        assert!(
            statuses.contains(
                &scenario["evidence_status"]
                    .as_str()
                    .expect("scenario has evidence_status")
            )
        );
    }
}

#[test]
fn legacy_baseline_records_fixed_inputs_and_expected_results() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../docs/redesign/legacy-baseline-v1.json"
    );
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).expect("baseline exists"))
            .expect("baseline is valid JSON");
    assert_eq!(value["schema"], "trace_commons.legacy_baseline.v1");
    assert_eq!(value["input_identities"]["fixture_count"], 10);
    assert_eq!(
        value["input_identities"]["ordered_fixture_sha256"]
            .as_array()
            .expect("fixture hashes are an array")
            .len(),
        10
    );
    assert_eq!(
        value["expected_results"]["first_run"]["receipt_counts"]["accepted"],
        10
    );
    assert_eq!(
        value["expected_results"]["exact_replay"]["distinct_submission_count"],
        10
    );
}
