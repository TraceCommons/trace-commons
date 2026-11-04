// The daily-cap items are deprecated with the retired limit; these tests
// keep their behaviour pinned until they are removed.
#![allow(deprecated)]

use trace_commons_protocol::upload_allowance::{DailyUploadAllowance, UploadAllowanceResponse};

#[test]
fn upload_allowance_accepts_only_supported_bounded_complete_values() {
    let allowance = DailyUploadAllowance {
        schema_version: 1,
        max_uploads_per_day: 1_000,
        max_bytes_per_day: 5 * 1024 * 1024 * 1024,
    };
    assert!(allowance.validate());
    for invalid in [
        DailyUploadAllowance {
            schema_version: 2,
            ..allowance
        },
        DailyUploadAllowance {
            max_uploads_per_day: 0,
            ..allowance
        },
        DailyUploadAllowance {
            max_uploads_per_day: 1_001,
            ..allowance
        },
        DailyUploadAllowance {
            max_bytes_per_day: 0,
            ..allowance
        },
        DailyUploadAllowance {
            max_bytes_per_day: 5 * 1024 * 1024 * 1024 + 1,
            ..allowance
        },
    ] {
        assert!(!invalid.validate());
    }
    assert!(
        serde_json::from_str::<DailyUploadAllowance>(
            r#"{"schema_version":1,"max_uploads_per_day":200}"#
        )
        .is_err()
    );
}

#[test]
fn upload_allowance_wire_round_trip_and_explicit_absence() {
    let body = r#"{"allowance":{"schema_version":1,"max_uploads_per_day":200,"max_bytes_per_day":2147483648},"future_field":true}"#;
    let response: UploadAllowanceResponse = serde_json::from_str(body).unwrap();
    assert!(response.allowance.unwrap().validate());
    let absent: UploadAllowanceResponse = serde_json::from_str(r#"{"allowance":null}"#).unwrap();
    assert!(absent.allowance.is_none());
    assert!(serde_json::from_str::<UploadAllowanceResponse>("{}").is_err());
    assert_eq!(
        serde_json::to_string(&absent).unwrap(),
        r#"{"allowance":null}"#
    );
}
