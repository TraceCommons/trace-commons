// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Internal credit settlement helpers for the versioned pipeline.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use trace_commons_gate_api::pipeline::{InstrumentId, Microcredits};
use trace_commons_gate_api::{
    SettlementAdapter, SettlementError, SettlementReceipt, SettlementRequest,
};
use uuid::Uuid;

use crate::near_credit::{
    NearCreditReceipt, NearCreditReceiptCall, trace_credit_settlement_attestation_hash,
    trace_credit_settlement_issuer_signature_hash,
};
use crate::trace_corpus_storage::TraceCreditSettlementNearStatus;

pub const PIPELINE_SETTLEMENT_POLICY_VERSION: &str = "pipeline-internal-v1";
pub const PIPELINE_CREDIT_REASON: &str = "pipeline_score";
/// The ledger `actor_role` of a minimal-family (`PipelineScore`) Trace
/// Credit event, whose event type is `accepted`.
pub const PIPELINE_CREDIT_ACTOR_ROLE: &str = "pipeline_worker";
/// Ruling T15-1: the ledger `actor_role` of a compatibility `NoveltyUtility`
/// event -- the role `main`'s gate path records for the same event, its
/// caller's (`vector_worker`). `main`'s database credit readers parse the
/// role of every event type they map to a legacy event and refuse the whole
/// read on one that is not a token role, so `pipeline_worker` on this event
/// type would fail every credit, status, and withdrawal read of the tenant.
pub const PIPELINE_NOVELTY_UTILITY_ACTOR_ROLE: &str = "vector_worker";

/// Ruling T15-2: the ledger `reason` of a compatibility `NoveltyUtility`
/// event, in `main`'s shape `novelty_utility:<version>`, with the
/// compatibility Score rule as the version where `main` has its gate policy
/// version.
pub fn pipeline_novelty_utility_reason() -> String {
    format!(
        "novelty_utility:{}",
        crate::versioned_pipeline_compat::COMPATIBILITY_SCORE_RULE
    )
}
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

/// A test settlement adapter with no effect: it records each request and
/// answers the expected result. It is in the library, not behind
/// `#[cfg(test)]`, because the integration tests and the ingest binary's
/// tests link the library built without `cfg(test)`. It is never
/// production-qualified (`production_qualified` keeps the trait's `false`),
/// so the qualification gate refuses a runtime that routes or drains a
/// tenant through it (`each_pipeline_test_double_fails_the_qualification_gate`
/// in the ingest binary's tests).
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

/// One logical NEAR request a `RecordingNearAdapter` received: a repeated
/// idempotency key is the same request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearLogicalRequest {
    pub idempotency_key: String,
    pub method_name: String,
}

/// A test NEAR adapter with no network effect. It records each idempotency
/// key once, answers a repeated key with the same result, and refuses a
/// repeated key with a different method. A confirmation exists only once a
/// test records one (`record_confirmation`) or a failure on chain
/// (`record_failure`); a recorded failure wins over a recorded
/// confirmation. `fail_next` makes the next submit fail. It presents no credential unless built `authenticated`.
///
/// It is in the library, not behind `#[cfg(test)]`, because the integration
/// tests and the ingest binary's tests link the library built without
/// `cfg(test)`. It is never production-qualified, so the qualification gate
/// refuses an enabled payout through it
/// (`pipeline_runtime_requires_a_qualified_payout_adapter_only_when_payout_is_enabled`
/// in the ingest binary's tests).
#[derive(Debug, Default)]
pub struct RecordingNearAdapter {
    requests: Mutex<Vec<NearLogicalRequest>>,
    confirmations: Mutex<BTreeMap<String, NearConfirmationEvidence>>,
    failures: Mutex<BTreeSet<String>>,
    fail_next: AtomicBool,
    authenticated: bool,
}

/// Hash-only evidence that a submitted NEAR call was confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearConfirmationEvidence {
    pub transaction_hash_hash: String,
    pub receipt_hash: String,
}

/// What a NEAR adapter reports for a submitted call: not decided yet,
/// confirmed with hash-only evidence, or failed on chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NearPayoutConfirmation {
    Pending,
    Confirmed(NearConfirmationEvidence),
    /// The result is final on chain and the transfer did not occur. It is
    /// terminal for the line: the pass marks it `failed` and never submits
    /// it again. An error of the lookup, a timeout, or a transaction that is
    /// not known is `Pending`, never `Failed`.
    Failed,
}

/// The NEAR payout rail for settled Trace Credit (P3-D11). Not a gate
/// contract (Ruling T10-1): it stays in the server crate, as `main`'s other
/// NEAR traits do, and the pipeline holds it only as
/// `Arc<dyn NearPayoutAdapter>`.
///
/// `submit` must be idempotent on `call.idempotency_key`: a repeated call
/// with the same key is the same logical request, never a second
/// transaction. The payout records a submit in the outbox after `submit`
/// returns, so a crash between the two repeats the call on the next pass.
///
/// `authenticated` says whether its calls present a credential to the NEAR
/// submitter and confirmer, as `main`'s HTTP adapters do with a bearer
/// token. Under `TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH`, an enabled
/// payout on an adapter that does not is refused at build, as `main` refuses
/// to start its own adapters without their tokens (Zaki review 1, round 2,
/// finding 2).
#[async_trait]
pub trait NearPayoutAdapter: Send + Sync {
    /// The one answer to whether this adapter may pay out for a routed or
    /// drained tenant: `false` by default, and readiness fails closed on it.
    fn production_qualified(&self) -> bool {
        false
    }
    fn authenticated(&self) -> bool {
        false
    }
    async fn submit(&self, call: &NearCreditReceiptCall) -> anyhow::Result<String>;
    /// `Failed` only when the result is final on chain and the transfer did
    /// not occur; it is terminal, and the pass does not submit the line
    /// again. An error of the lookup, a timeout, or an unknown transaction
    /// is `Pending`.
    async fn confirmation(&self, idempotency_key: &str) -> NearPayoutConfirmation;
}

/// The payout adapter `TRACE_COMMONS_NEAR_SETTLEMENT_MODE=dry_run` pays
/// through in place of the injected one, as `main`'s outbox worker does in
/// that mode with its in-process submitter and confirmer: the full outbox
/// state machine runs with no network and no funds. `submit` validates the
/// call and answers a synthetic transaction hash derived from its
/// idempotency key alone, so a repeated key gets the same hash; every
/// submitted call is confirmed, with hash-only evidence derived from the
/// same key (Zaki review 1, round 2, finding 2).
#[derive(Debug, Default, Clone, Copy)]
pub struct DryRunNearPayoutAdapter;

impl DryRunNearPayoutAdapter {
    /// The synthetic NEAR transaction hash for `idempotency_key`: base58 of
    /// its SHA-256, the shape `main`'s dry-run submitter answers.
    pub fn transaction_hash(idempotency_key: &str) -> String {
        bs58::encode(Sha256::digest(idempotency_key.as_bytes())).into_string()
    }
}

#[async_trait]
impl NearPayoutAdapter for DryRunNearPayoutAdapter {
    async fn submit(&self, call: &NearCreditReceiptCall) -> anyhow::Result<String> {
        call.validate()?;
        Ok(Self::transaction_hash(&call.idempotency_key))
    }

    async fn confirmation(&self, idempotency_key: &str) -> NearPayoutConfirmation {
        let transaction_hash = Self::transaction_hash(idempotency_key);
        NearPayoutConfirmation::Confirmed(NearConfirmationEvidence {
            transaction_hash_hash: format!(
                "sha256:{:x}",
                Sha256::digest(transaction_hash.as_bytes())
            ),
            receipt_hash: format!(
                "sha256:{:x}",
                Sha256::digest(format!("near_dry_run_receipt:v1:{idempotency_key}").as_bytes())
            ),
        })
    }
}

impl RecordingNearAdapter {
    pub fn new() -> Self {
        Self::default()
    }

    /// A recording adapter whose calls present a credential
    /// (`NearPayoutAdapter::authenticated`).
    pub fn authenticated() -> Self {
        Self {
            authenticated: true,
            ..Self::default()
        }
    }

    pub fn fail_next(&self) {
        self.fail_next.store(true, Ordering::SeqCst);
    }

    pub fn requests(&self) -> Vec<NearLogicalRequest> {
        self.requests.lock().expect("near adapter mutex").clone()
    }

    pub fn record_confirmation(
        &self,
        idempotency_key: &str,
        transaction_hash_hash: impl Into<String>,
        receipt_hash: impl Into<String>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.requests
                .lock()
                .expect("near adapter mutex")
                .iter()
                .any(|request| request.idempotency_key == idempotency_key),
            "cannot confirm an unsubmitted NEAR request"
        );
        let evidence = NearConfirmationEvidence {
            transaction_hash_hash: transaction_hash_hash.into(),
            receipt_hash: receipt_hash.into(),
        };
        anyhow::ensure!(
            evidence.transaction_hash_hash.starts_with("sha256:")
                && evidence.receipt_hash.starts_with("sha256:"),
            "NEAR confirmation evidence must be hash-only"
        );
        self.confirmations
            .lock()
            .expect("near confirmation mutex")
            .insert(idempotency_key.to_string(), evidence);
        Ok(())
    }

    /// Records that the transaction of a submitted request failed on chain.
    pub fn record_failure(&self, idempotency_key: &str) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.requests
                .lock()
                .expect("near adapter mutex")
                .iter()
                .any(|request| request.idempotency_key == idempotency_key),
            "cannot fail an unsubmitted NEAR request"
        );
        self.failures
            .lock()
            .expect("near failure mutex")
            .insert(idempotency_key.to_string());
        Ok(())
    }

    pub fn confirmation(&self, idempotency_key: &str) -> Option<NearConfirmationEvidence> {
        self.confirmations
            .lock()
            .expect("near confirmation mutex")
            .get(idempotency_key)
            .cloned()
    }
}

#[async_trait]
impl NearPayoutAdapter for RecordingNearAdapter {
    fn authenticated(&self) -> bool {
        self.authenticated
    }

    async fn submit(&self, call: &NearCreditReceiptCall) -> anyhow::Result<String> {
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

    async fn confirmation(&self, idempotency_key: &str) -> NearPayoutConfirmation {
        if self
            .failures
            .lock()
            .expect("near failure mutex")
            .contains(idempotency_key)
        {
            return NearPayoutConfirmation::Failed;
        }
        match Self::confirmation(self, idempotency_key) {
            Some(evidence) => NearPayoutConfirmation::Confirmed(evidence),
            None => NearPayoutConfirmation::Pending,
        }
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

pub fn pipeline_near_outbox_line_id(
    tenant_id: &str,
    settlement_batch_id: Uuid,
    credit_account_hash: &str,
) -> Uuid {
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!(
            "tracecommons:pipeline-near:{tenant_id}:{settlement_batch_id}:{credit_account_hash}"
        )
        .as_bytes(),
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

pub fn microcredits_to_settled_i64(amount: Microcredits) -> anyhow::Result<i64> {
    i64::try_from(amount.get()).map_err(|_| anyhow::anyhow!("credit_amount_overflow"))
}

/// The NEAR `settle_credit_receipt` call that pays one batch line out, on
/// `near_contract_id` -- the contract `main`'s legacy NEAR path is
/// configured with (Ruling T10-4). The contract is part of the call's
/// idempotency key. (The port's name is kept.) The attestation and
/// signature hashes are `main`'s for a settlement that named no issuer
/// approval evidence (Zaki review 1, item 2): a pipeline batch has none, and
/// a payout is refused while `main` requires one.
pub fn disabled_near_call(
    near_contract_id: &str,
    settlement_batch_id: Uuid,
    credit_account_hash: &str,
    source_list_hash: &str,
    amount_micros: i64,
) -> anyhow::Result<NearCreditReceiptCall> {
    NearCreditReceiptCall::settle(
        near_contract_id,
        NearCreditReceipt {
            settlement_batch_id,
            credit_account_hash: credit_account_hash.to_string(),
            policy_version: PIPELINE_SETTLEMENT_POLICY_VERSION.to_string(),
            source_list_hash: source_list_hash.to_string(),
            attestation_hash: trace_credit_settlement_attestation_hash(source_list_hash, None),
            amount_micros,
            issuer_signature_hash: trace_credit_settlement_issuer_signature_hash(
                settlement_batch_id,
                source_list_hash,
                None,
            ),
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

    #[tokio::test]
    async fn recording_near_adapter_collapses_a_repeated_key_and_confirms_only_a_submitted_one() {
        let adapter = RecordingNearAdapter::new();
        let call = disabled_near_call(
            "trace-credits.testnet",
            Uuid::from_u128(1),
            &format!("sha256:{}", "1".repeat(64)),
            &format!("sha256:{}", "2".repeat(64)),
            5,
        )
        .unwrap();
        let tx_hash = format!("sha256:{}", "a".repeat(64));
        let receipt_hash = format!("sha256:{}", "b".repeat(64));
        assert!(
            adapter
                .record_confirmation(&call.idempotency_key, &tx_hash, &receipt_hash)
                .is_err(),
            "an unsubmitted request cannot be confirmed"
        );

        adapter.fail_next();
        assert!(NearPayoutAdapter::submit(&adapter, &call).await.is_err());
        assert!(adapter.requests().is_empty());
        let first = NearPayoutAdapter::submit(&adapter, &call).await.unwrap();
        assert_eq!(
            NearPayoutAdapter::submit(&adapter, &call).await.unwrap(),
            first
        );
        assert_eq!(
            adapter.requests().len(),
            1,
            "a repeated key is one logical request"
        );

        assert!(
            adapter
                .record_confirmation(&call.idempotency_key, "tx-plain", &receipt_hash)
                .is_err(),
            "confirmation evidence is hash-only"
        );
        assert_eq!(
            NearPayoutAdapter::confirmation(&adapter, &call.idempotency_key).await,
            NearPayoutConfirmation::Pending
        );
        adapter
            .record_confirmation(&call.idempotency_key, &tx_hash, &receipt_hash)
            .unwrap();
        assert_eq!(
            NearPayoutAdapter::confirmation(&adapter, &call.idempotency_key).await,
            NearPayoutConfirmation::Confirmed(NearConfirmationEvidence {
                transaction_hash_hash: tx_hash,
                receipt_hash,
            })
        );
        adapter.record_failure(&call.idempotency_key).unwrap();
        assert_eq!(
            NearPayoutAdapter::confirmation(&adapter, &call.idempotency_key).await,
            NearPayoutConfirmation::Failed
        );
        assert!(adapter.record_failure("unsubmitted").is_err());
    }
}
