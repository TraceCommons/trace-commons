/// Declares cooperative background witness work without carrying identity.
pub const WITNESS_WORKLOAD_HEADER: &str = "x-trace-witness-workload";
pub const WITNESS_BACKGROUND_WORKLOAD: &str = "background";
pub const WITNESS_SATURATED_ERROR: &str = "witness_saturated";
pub const WITNESS_SATURATED_RETRY_AFTER_SECS: u32 = 30;

/// Only capacity refusal is retryable; policy and timeout refusals are distinct.
pub fn is_witness_saturation(status: u16, error: &str) -> bool {
    status == 503 && error == WITNESS_SATURATED_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_contract_is_exact_and_policy_refusals_are_not_saturation() {
        assert_eq!(WITNESS_WORKLOAD_HEADER, "x-trace-witness-workload");
        assert_eq!(WITNESS_BACKGROUND_WORKLOAD, "background");
        assert_eq!(WITNESS_SATURATED_ERROR, "witness_saturated");
        assert_eq!(WITNESS_SATURATED_RETRY_AFTER_SECS, 30);
        assert!(is_witness_saturation(503, "witness_saturated"));
        assert!(!is_witness_saturation(403, "witness_saturated"));
        assert!(!is_witness_saturation(504, "witness_saturated"));
        assert!(!is_witness_saturation(503, "witness_saturated_extra"));
    }
}
