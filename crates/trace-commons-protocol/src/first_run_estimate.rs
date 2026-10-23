//! The first run's dollar estimate: what the commons says one accepted trace
//! is worth in NEAR AI credit, as a range the operator sets.
//!
//! `GET /v1/public/first-run-estimate` on `trace-commons-ingest` is
//! unauthenticated and outside tenant context, like
//! `/v1/credit-estimate/table`: it is public, non-personal configuration, and
//! the first run reads it before the contributor has an account. The
//! operator sets the range with `TRACE_COMMONS_FIRST_RUN_USD_ESTIMATE_MICROS_MIN`
//! and `..._MAX`; with neither set the document carries no range, and a
//! client words its first-run estimate in points, as before.
//!
//! The range is an estimate, not a promise and not a rate. It is shown once,
//! on the first run, multiplied by the traces found on the machine and worded
//! "an estimated $X–$Y in NEAR AI credit". It is never a balance, never
//! settled, and nothing else reads it.
//!
//! The response type deliberately does not use `deny_unknown_fields`, so a
//! field added later does not break clients already shipped.

use serde::{Deserialize, Serialize};

/// Ingest route.
pub const FIRST_RUN_ESTIMATE_PATH: &str = "/v1/public/first-run-estimate";

/// The largest document a client reads. A valid one is well under 200
/// bytes; this bounds the read, it is not a policy value.
pub const FIRST_RUN_ESTIMATE_MAX_BYTES: usize = 4 * 1024;

/// One cent, in micro-dollars. Range ends are whole cents, so a figure is
/// never rounded on the way to the screen.
pub const USD_MICROS_PER_CENT: u64 = 10_000;

/// The largest per-trace figure accepted at either end: $100. A bound on a
/// typo (micros entered as dollars times a million twice), not a policy
/// value.
pub const FIRST_RUN_USD_MICROS_MAX: u64 = 100_000_000;

/// `GET /v1/public/first-run-estimate` body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FirstRunEstimate {
    /// Estimated NEAR AI credit per accepted trace, in micro-dollars,
    /// inclusive on both ends. Absent when the operator set none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usd_micros_per_accepted_trace: Option<UsdMicrosRange>,
}

/// A range in micro-dollars, `min <= max`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsdMicrosRange {
    pub min: u64,
    pub max: u64,
}

impl UsdMicrosRange {
    /// Whether the range is one a server may publish and a client may show:
    /// both ends positive, whole cents, at most [`FIRST_RUN_USD_MICROS_MAX`],
    /// and `min <= max`. Server boot and the client apply the same rule.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let end_ok = |v: u64| {
            v > 0 && v <= FIRST_RUN_USD_MICROS_MAX && v.is_multiple_of(USD_MICROS_PER_CENT)
        };
        end_ok(self.min) && end_ok(self.max) && self.min <= self.max
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_range_is_an_empty_document() {
        let doc = FirstRunEstimate::default();
        assert_eq!(serde_json::to_value(&doc).unwrap(), serde_json::json!({}));
        let parsed: FirstRunEstimate = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(parsed, doc);
    }

    #[test]
    fn a_document_with_an_extra_field_still_deserializes() {
        let parsed: FirstRunEstimate = serde_json::from_value(serde_json::json!({
            "usd_micros_per_accepted_trace": {"min": 10_000, "max": 20_000},
            "later": true,
        }))
        .unwrap();
        assert_eq!(
            parsed.usd_micros_per_accepted_trace,
            Some(UsdMicrosRange {
                min: 10_000,
                max: 20_000
            })
        );
    }

    #[test]
    fn a_range_is_positive_whole_cents_bounded_and_ordered() {
        let r = |min, max| UsdMicrosRange { min, max };
        assert!(r(10_000, 10_000).is_valid());
        assert!(r(10_000, FIRST_RUN_USD_MICROS_MAX).is_valid());
        assert!(!r(0, 10_000).is_valid(), "zero");
        assert!(!r(20_000, 10_000).is_valid(), "min above max");
        assert!(!r(10_001, 20_000).is_valid(), "sub-cent min");
        assert!(!r(10_000, 20_001).is_valid(), "sub-cent max");
        assert!(
            !r(10_000, FIRST_RUN_USD_MICROS_MAX + USD_MICROS_PER_CENT).is_valid(),
            "above the bound"
        );
    }
}
