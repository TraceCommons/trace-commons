// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Internal credit settlement helpers for the versioned pipeline.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::{InstrumentId, Microcredits};
use trace_commons_gate_api::{
    SettlementAdapter, SettlementError, SettlementReceipt, SettlementRequest,
};
use uuid::Uuid;

pub const PIPELINE_SETTLEMENT_POLICY_VERSION: &str = "pipeline-internal-v1";
pub const PIPELINE_CREDIT_REASON: &str = "pipeline_score";
pub const PIPELINE_TEST_CREDIT_CAP_MICROCREDITS: u64 = 10_000_000;

#[derive(Default)]
pub struct SettlementAdapterRegistry {
    adapters: BTreeMap<String, Arc<dyn SettlementAdapter>>,
}

impl SettlementAdapterRegistry {
    pub fn new(adapters: Vec<Arc<dyn SettlementAdapter>>) -> anyhow::Result<Self> {
        let mut registry = Self::default();
        for adapter in adapters {
            let instrument_id = adapter.instrument_id().as_str().to_string();
            anyhow::ensure!(
                !adapter.adapter_identity().trim().is_empty(),
                "settlement adapter identity is empty"
            );
            anyhow::ensure!(
                !adapter.payout_rail().trim().is_empty(),
                "settlement adapter payout rail is empty"
            );
            anyhow::ensure!(
                registry
                    .adapters
                    .insert(instrument_id.clone(), adapter)
                    .is_none(),
                "duplicate settlement adapter for {instrument_id}"
            );
        }
        Ok(registry)
    }

    pub fn get(&self, instrument_id: &InstrumentId) -> Option<&Arc<dyn SettlementAdapter>> {
        self.adapters.get(instrument_id.as_str())
    }

    pub fn payout_rails(&self) -> BTreeMap<String, String> {
        self.adapters
            .iter()
            .map(|(instrument, adapter)| (instrument.clone(), adapter.payout_rail().to_string()))
            .collect()
    }

    pub fn identities(&self) -> BTreeMap<String, String> {
        self.adapters
            .iter()
            .map(|(instrument, adapter)| {
                (instrument.clone(), adapter.adapter_identity().to_string())
            })
            .collect()
    }

    pub fn production_qualifications(&self) -> BTreeMap<String, bool> {
        self.adapters
            .iter()
            .map(|(instrument, adapter)| (instrument.clone(), adapter.production_qualified()))
            .collect()
    }
}

#[derive(Debug)]
pub struct RecordingSettlementAdapter {
    instrument_id: InstrumentId,
    identity: String,
    payout_rail: String,
    requests: Mutex<Vec<SettlementRequest>>,
    fail_next: AtomicBool,
}

impl RecordingSettlementAdapter {
    pub fn new(
        instrument_id: InstrumentId,
        identity: impl Into<String>,
        payout_rail: impl Into<String>,
    ) -> Arc<Self> {
        Arc::new(Self {
            instrument_id,
            identity: identity.into(),
            payout_rail: payout_rail.into(),
            requests: Mutex::new(Vec::new()),
            fail_next: AtomicBool::new(false),
        })
    }

    pub fn fail_next(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }

    pub fn requests(&self) -> Vec<SettlementRequest> {
        self.requests
            .lock()
            .expect("settlement adapter mutex")
            .clone()
    }
}

/// A test adapter with an internal effect. It records each operation once,
/// answers a repeated request with the same receipt, and refuses a repeated
/// operation reference with different content as a `Conflict`, as the
/// adapter contract requires. `fail_next` makes the next call `Unavailable`.
#[async_trait]
impl SettlementAdapter for RecordingSettlementAdapter {
    fn instrument_id(&self) -> &InstrumentId {
        &self.instrument_id
    }

    fn adapter_identity(&self) -> &str {
        &self.identity
    }

    fn payout_rail(&self) -> &str {
        &self.payout_rail
    }

    async fn settle(
        &self,
        request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError> {
        if request.instrument_id() != &self.instrument_id {
            return Err(SettlementError::Rejected);
        }
        if self.fail_next.swap(false, Ordering::SeqCst) {
            return Err(SettlementError::Unavailable);
        }
        let mut requests = self.requests.lock().expect("settlement adapter mutex");
        if let Some(existing) = requests
            .iter()
            .find(|existing| existing.operation_ref_hash() == request.operation_ref_hash())
        {
            if existing != request {
                return Err(SettlementError::Conflict);
            }
        } else {
            requests.push(request.clone());
        }
        SettlementReceipt::internal(request.expected_result_ref_hash())
            .map_err(|_| SettlementError::Rejected)
    }
}

pub fn pipeline_credit_event_id(tenant_id: &str, run_id: Uuid, score_outcome_id: Uuid) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:pipeline-credit:{tenant_id}:{run_id}:{score_outcome_id}").as_bytes(),
    )
}

pub fn pipeline_ledger_source_key(tenant_id: &str, request_idempotency_key: &str) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(
            format!("tracecommons:ledger-source:{tenant_id}:{request_idempotency_key}").as_bytes()
        )
    )
}

pub fn pipeline_settlement_batch_id(tenant_id: &str, source_list_hash: &str) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:pipeline-batch:{tenant_id}:{source_list_hash}").as_bytes(),
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

pub fn microcredits_to_settled_i64(amount: Microcredits) -> anyhow::Result<i64> {
    i64::try_from(amount.get()).map_err(|_| anyhow::anyhow!("credit_amount_overflow"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_gate_api::pipeline::AtomicUnits;

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

    #[test]
    fn registry_refuses_two_adapters_for_one_instrument() {
        let first = RecordingSettlementAdapter::new(
            InstrumentId::trace_credit(),
            "first_test_only",
            "none",
        );
        let second = RecordingSettlementAdapter::new(
            InstrumentId::trace_credit(),
            "second_test_only",
            "none",
        );
        let error = SettlementAdapterRegistry::new(vec![first, second])
            .err()
            .unwrap();
        assert!(error.to_string().contains("duplicate settlement adapter"));
    }

    #[tokio::test]
    async fn recording_adapter_fails_once_then_returns_the_same_receipt_for_a_retry() {
        let adapter = RecordingSettlementAdapter::new(
            InstrumentId::new("storage_rebate").unwrap(),
            "rebate_test_only",
            "none",
        );
        let request_with = |atomic_units: u128| {
            SettlementRequest::new(
                crate::versioned_pipeline::pipeline_tenant_storage_ref("tenant-a"),
                Uuid::from_u128(1),
                InstrumentId::new("storage_rebate").unwrap(),
                AtomicUnits::from_raw(atomic_units),
                format!("sha256:{}", "1".repeat(64)),
                format!("sha256:{}", "2".repeat(64)),
            )
            .unwrap()
        };
        let request = request_with(5);
        adapter.fail_next();
        assert_eq!(
            adapter.settle(&request).await,
            Err(SettlementError::Unavailable)
        );
        let receipt = adapter.settle(&request).await.unwrap();
        assert!(receipt.answers(&request));
        assert_eq!(receipt.external_receipt_hash(), None);
        assert_eq!(adapter.settle(&request).await.unwrap(), receipt);
        assert_eq!(
            adapter.requests().len(),
            1,
            "a retry is one logical request"
        );
        assert_eq!(
            adapter.settle(&request_with(6)).await,
            Err(SettlementError::Conflict),
            "same operation with changed content is a conflict"
        );
        assert_eq!(adapter.requests().len(), 1);
    }

    #[tokio::test]
    async fn recording_adapter_rejects_another_instrument() {
        let adapter = RecordingSettlementAdapter::new(
            InstrumentId::new("storage_rebate").unwrap(),
            "rebate_test_only",
            "none",
        );
        let request = SettlementRequest::new(
            crate::versioned_pipeline::pipeline_tenant_storage_ref("tenant-a"),
            Uuid::from_u128(1),
            InstrumentId::trace_credit(),
            AtomicUnits::from_raw(5),
            format!("sha256:{}", "1".repeat(64)),
            format!("sha256:{}", "2".repeat(64)),
        )
        .unwrap();
        assert_eq!(
            adapter.settle(&request).await,
            Err(SettlementError::Rejected)
        );
        assert!(adapter.requests().is_empty());
    }
}
