//! Server-advertised daily client upload allowances. These never grant consent
//! or bypass server admission; clients retain their own explicit limits.

use serde::{Deserialize, Serialize};

#[deprecated(note = "the client daily upload limit is retired; nothing reads it")]
pub const DEFAULT_MAX_UPLOADS_PER_DAY: u32 = 50;
#[deprecated(note = "the client daily upload limit is retired; nothing reads it")]
pub const DEFAULT_MAX_BYTES_PER_DAY: u64 = 209_715_200;

#[deprecated(note = "the client daily upload limit is retired; nothing reads it")]
pub const MAX_UPLOADS_PER_DAY: u32 = 1_000;
#[deprecated(note = "the client daily upload limit is retired; nothing reads it")]
pub const MAX_BYTES_PER_DAY: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyUploadAllowance {
    pub schema_version: u32,
    pub max_uploads_per_day: u32,
    pub max_bytes_per_day: u64,
}

impl DailyUploadAllowance {
    /// Reject unsupported versions and the whole allowance if either limit
    /// exceeds the client's runaway-upload safeguards.
    #[deprecated(note = "the client daily upload limit is retired; nothing reads it")]
    #[allow(deprecated)]
    pub fn validate(&self) -> bool {
        self.schema_version == 1
            && (1..=MAX_UPLOADS_PER_DAY).contains(&self.max_uploads_per_day)
            && (1..=MAX_BYTES_PER_DAY).contains(&self.max_bytes_per_day)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UploadAllowanceResponse {
    // Explicit null withdraws the allowance; a missing field is malformed and
    // must not erase a client's previously valid allowance.
    #[serde(deserialize_with = "deserialize_allowance")]
    pub allowance: Option<DailyUploadAllowance>,
}

fn deserialize_allowance<'de, D>(deserializer: D) -> Result<Option<DailyUploadAllowance>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<DailyUploadAllowance>::deserialize(deserializer)
}
