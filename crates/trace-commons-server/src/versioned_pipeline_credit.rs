//! Internal credit settlement helpers for the versioned pipeline.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::Microcredits;
use uuid::Uuid;

use crate::near_credit::{NearCreditReceipt, NearCreditReceiptCall};
use crate::trace_corpus_storage::TraceCreditSettlementNearStatus;

pub const PIPELINE_SETTLEMENT_POLICY_VERSION: &str = "pipeline-internal-v1";
pub const PIPELINE_CREDIT_REASON: &str = "pipeline_score";
pub const PIPELINE_TEST_CREDIT_CAP_MICROCREDITS: u64 = 10_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearLogicalRequest {
    pub idempotency_key: String,
    pub method_name: String,
}

#[derive(Debug, Default)]
pub struct RecordingNearAdapter {
    requests: Mutex<Vec<NearLogicalRequest>>,
    fail_next: AtomicBool,
}

impl RecordingNearAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fail_next(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }

    pub fn submit(&self, call: &NearCreditReceiptCall) -> anyhow::Result<String> {
        call.validate()?;
        if self.fail_next.swap(false, Ordering::SeqCst) {
            anyhow::bail!("near_adapter_unavailable");
        }
        let mut requests = self.requests.lock().expect("near adapter mutex");
        if let Some(existing) = requests
            .iter()
            .find(|request| request.idempotency_key == call.idempotency_key)
        {
            anyhow::ensure!(
                existing.method_name == call.method_name,
                "NEAR idempotency key reused with a different method"
            );
            return Ok(call.idempotency_key.clone());
        }
        requests.push(NearLogicalRequest {
            idempotency_key: call.idempotency_key.clone(),
            method_name: call.method_name.clone(),
        });
        Ok(call.idempotency_key.clone())
    }

    pub fn requests(&self) -> Vec<NearLogicalRequest> {
        self.requests.lock().expect("near adapter mutex").clone()
    }
}

pub fn pipeline_credit_event_id(tenant_id: &str, run_id: Uuid, score_outcome_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:pipeline-credit:{tenant_id}:{run_id}:{score_outcome_id}").as_bytes(),
    )
}

pub fn pipeline_settlement_batch_id(tenant_id: &str, source_list_hash: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:pipeline-batch:{tenant_id}:{source_list_hash}").as_bytes(),
    )
}

pub fn pipeline_near_outbox_id(tenant_id: &str, settlement_batch_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:pipeline-near:{tenant_id}:{settlement_batch_id}").as_bytes(),
    )
}

pub fn credit_account_hash(principal_ref: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(principal_ref.as_bytes()))
}

pub fn source_list_hash(event_ids: &[Uuid]) -> String {
    let mut ids = event_ids.to_vec();
    ids.sort_unstable();
    let canonical = ids
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
}

pub fn issuer_approval_hash(source_list_hash: &str) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("pipeline-issuer:{source_list_hash}").as_bytes())
    )
}

pub fn settlement_batch_ref_hash(settlement_batch_id: Uuid, source_list_hash: &str) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("{settlement_batch_id}\n{source_list_hash}").as_bytes())
    )
}

pub fn microcredits_to_settled_i64(amount: Microcredits) -> anyhow::Result<i64> {
    i64::try_from(amount.get()).map_err(|_| anyhow::anyhow!("credit_amount_overflow"))
}

pub fn disabled_near_call(
    settlement_batch_id: Uuid,
    credit_account_hash: &str,
    source_list_hash: &str,
    amount_micros: i64,
) -> anyhow::Result<NearCreditReceiptCall> {
    NearCreditReceiptCall::settle(
        "pipeline.test.near",
        NearCreditReceipt {
            settlement_batch_id,
            credit_account_hash: credit_account_hash.to_string(),
            policy_version: PIPELINE_SETTLEMENT_POLICY_VERSION.to_string(),
            source_list_hash: source_list_hash.to_string(),
            attestation_hash: issuer_approval_hash(source_list_hash),
            amount_micros,
            issuer_signature_hash: issuer_approval_hash(source_list_hash),
        },
    )
}

pub fn payout_state_label(status: TraceCreditSettlementNearStatus) -> &'static str {
    match status {
        TraceCreditSettlementNearStatus::Disabled => "disabled",
        TraceCreditSettlementNearStatus::Pending => "pending",
        TraceCreditSettlementNearStatus::Submitted => "submitted",
        TraceCreditSettlementNearStatus::Confirmed => "confirmed",
        TraceCreditSettlementNearStatus::Failed => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_list_hash_is_order_independent() {
        let left = Uuid::from_u128(1);
        let right = Uuid::from_u128(2);
        assert_eq!(
            source_list_hash(&[left, right]),
            source_list_hash(&[right, left])
        );
        assert_ne!(source_list_hash(&[left]), source_list_hash(&[right]));
    }

    #[test]
    fn microcredit_storage_conversion_rejects_overflow() {
        assert!(microcredits_to_settled_i64(Microcredits::from_raw(i64::MAX as u64)).is_ok());
        assert!(
            microcredits_to_settled_i64(Microcredits::from_raw(u64::MAX))
                .unwrap_err()
                .to_string()
                .contains("credit_amount_overflow")
        );
    }
}
