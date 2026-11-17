//! The witness's submit-then-poll wire contract.
//!
//! One spelling of every path, header, label and body shape, shared by the
//! witness and the contributor so the two cannot drift. See
//! `docs/superpowers/specs/2026-11-05-witness-submit-then-poll.md`.

use serde::{Deserialize, Serialize};

/// `POST` here to submit a job. The body is the one `POST /v1/witness` took.
pub const WITNESS_JOBS_PATH: &str = "/v1/witness/jobs";

/// `sha256(fetch_secret)`, 64 lowercase hex characters, on the submit.
pub const WITNESS_FETCH_SECRET_SHA256_HEADER: &str = "x-trace-witness-fetch-secret-sha256";

/// The fetch secret itself, 64 hex characters, on every `GET` and `DELETE`.
pub const WITNESS_FETCH_SECRET_HEADER: &str = "x-trace-witness-fetch-secret";

/// Bytes in a fetch secret.
pub const WITNESS_FETCH_SECRET_LEN: usize = 32;

/// Bytes in a job id (128 bits).
pub const WITNESS_JOB_ID_LEN: usize = 16;

/// The single answer for every miss: unknown id, wrong secret, expired,
/// already fetched, unreadable. `404`.
pub const WITNESS_JOB_GONE_ERROR: &str = "witness_job_gone";

/// The submit's secret hash is absent or not 64 hex characters. `400`.
pub const WITNESS_FETCH_SECRET_MALFORMED_ERROR: &str = "witness_fetch_secret_malformed";

/// The `state` a not-yet-finished job reports.
pub const WITNESS_JOB_PENDING_STATE: &str = "pending";

/// The shortest and longest wait a client honours between polls.
pub const MIN_POLL_AFTER_SECS: u32 = 1;
pub const MAX_POLL_AFTER_SECS: u32 = 60;

/// The `202` a submit answers.
///
/// `instance_id` names the instance holding the job, so the client can poll
/// it directly through the gateway. Absent when the witness could not read
/// its own instance id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessJobAccepted {
    pub job_id: String,
    pub poll_after_secs: u32,
    pub queue_position: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_id: Option<String>,
}

/// The `202` a poll answers while the job has not finished.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WitnessJobPending {
    pub state: String,
    pub poll_after_secs: u32,
    pub queue_position: u32,
}

/// Whether `value` is exactly `bytes` bytes of lowercase or uppercase hex.
pub fn is_hex_of_len(value: &str, bytes: usize) -> bool {
    value.len() == bytes * 2 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether `value` can be an instance id the client will put in a host
/// name: 8 to 64 lowercase hex characters, nothing else.
pub fn is_instance_id(value: &str) -> bool {
    (8..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_are_exact() {
        assert_eq!(WITNESS_JOBS_PATH, "/v1/witness/jobs");
        assert_eq!(
            WITNESS_FETCH_SECRET_SHA256_HEADER,
            "x-trace-witness-fetch-secret-sha256"
        );
        assert_eq!(WITNESS_FETCH_SECRET_HEADER, "x-trace-witness-fetch-secret");
        assert_eq!(WITNESS_JOB_GONE_ERROR, "witness_job_gone");
        assert_eq!(
            WITNESS_FETCH_SECRET_MALFORMED_ERROR,
            "witness_fetch_secret_malformed"
        );
    }

    #[test]
    fn accepted_omits_an_absent_instance_and_refuses_unknown_fields() {
        let accepted = WitnessJobAccepted {
            job_id: "ab".repeat(16),
            poll_after_secs: 5,
            queue_position: 0,
            instance_id: None,
        };
        let json = serde_json::to_value(&accepted).unwrap();
        assert!(json.get("instance_id").is_none());
        assert!(
            serde_json::from_str::<WitnessJobAccepted>(
                r#"{"job_id":"a","poll_after_secs":1,"queue_position":0,"extra":1}"#
            )
            .is_err()
        );
    }

    #[test]
    fn instance_ids_are_bounded_lowercase_hex() {
        assert!(is_instance_id("0123456789abcdef"));
        assert!(!is_instance_id("0123456"));
        assert!(!is_instance_id(&"a".repeat(65)));
        assert!(!is_instance_id("ABCDEF0123"));
        assert!(!is_instance_id("abcdef01.evil"));
        assert!(!is_instance_id("abcdef01-8088"));
    }

    #[test]
    fn hex_of_len_checks_both_length_and_alphabet() {
        assert!(is_hex_of_len(&"0f".repeat(32), 32));
        assert!(!is_hex_of_len(&"0f".repeat(31), 32));
        assert!(!is_hex_of_len(&"zz".repeat(32), 32));
    }
}
