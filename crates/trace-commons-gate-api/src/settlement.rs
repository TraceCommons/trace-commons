// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The settlement adapter seam for the versioned pipeline (#971).
//!
//! [`SettlementAdapter`] is the trait object a proprietary backend
//! implements to perform one instrument's external effect for a settlement
//! operation. The runner holds it as a trait object, never a concrete
//! type, so a substituted backend can slot in at this seam.

use async_trait::async_trait;
use uuid::Uuid;

use crate::pipeline::{
    AtomicUnits, ContractError, InstrumentId, TenantStorageRef, is_sha256,
    require_trace_credit_range,
};

/// One settlement operation: enough for an adapter to perform the
/// instrument's external effect and to prove, on retry, that a repeated
/// request carries the same content.
///
/// `tenant_storage_ref` is the tenant's derived storage reference, not the
/// raw tenant identifier — a gate-api seam never carries the raw tenant id.
/// The fields are private: `new` refuses a request that no settlement leg
/// could hold, so an adapter never receives a malformed reference, a zero
/// amount, or a Trace Credit amount above the ledger's range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementRequest {
    tenant_storage_ref: TenantStorageRef,
    run_id: Uuid,
    instrument_id: InstrumentId,
    atomic_units: AtomicUnits,
    operation_ref_hash: String,
    expected_result_ref_hash: String,
}

impl SettlementRequest {
    /// Refuses a request unless `operation_ref_hash` and
    /// `expected_result_ref_hash` are lowercase SHA-256 references and
    /// `atomic_units` is an amount that `InstrumentSettlement` accepts for
    /// `instrument_id`.
    pub fn new(
        tenant_storage_ref: TenantStorageRef,
        run_id: Uuid,
        instrument_id: InstrumentId,
        atomic_units: AtomicUnits,
        operation_ref_hash: impl Into<String>,
        expected_result_ref_hash: impl Into<String>,
    ) -> Result<Self, ContractError> {
        let operation_ref_hash = operation_ref_hash.into();
        let expected_result_ref_hash = expected_result_ref_hash.into();
        if atomic_units == AtomicUnits::ZERO {
            return Err(ContractError::ZeroInstrumentAward);
        }
        require_trace_credit_range(&instrument_id, atomic_units)?;
        if !is_sha256(&operation_ref_hash) || !is_sha256(&expected_result_ref_hash) {
            return Err(ContractError::InvalidSettlementReference);
        }
        Ok(Self {
            tenant_storage_ref,
            run_id,
            instrument_id,
            atomic_units,
            operation_ref_hash,
            expected_result_ref_hash,
        })
    }

    pub fn tenant_storage_ref(&self) -> &TenantStorageRef {
        &self.tenant_storage_ref
    }

    pub const fn run_id(&self) -> Uuid {
        self.run_id
    }

    pub fn instrument_id(&self) -> &InstrumentId {
        &self.instrument_id
    }

    pub const fn atomic_units(&self) -> AtomicUnits {
        self.atomic_units
    }

    pub fn operation_ref_hash(&self) -> &str {
        &self.operation_ref_hash
    }

    pub fn expected_result_ref_hash(&self) -> &str {
        &self.expected_result_ref_hash
    }
}

/// What a successful `settle` returns.
///
/// `result_ref_hash` is the result reference that the runner computed before
/// the call; it confirms which operation the adapter answered.
/// `external_receipt_hash` is evidence of the effect itself: a lowercase
/// SHA-256 reference over the adapter's receipt from the external system
/// (for a NEP-141 transfer, its transaction), which exists only after the
/// call. It is `None` for an adapter whose effect is internal. Both are
/// hash-only; the raw transaction hash, account, or RPC URL never crosses
/// this seam. How an adapter derives `external_receipt_hash` from its
/// receipt is part of the adapter's identity and must be stable, so a
/// repeated operation returns the same value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementReceipt {
    result_ref_hash: String,
    external_receipt_hash: Option<String>,
}

impl SettlementReceipt {
    /// A receipt for an effect with no external record.
    pub fn internal(result_ref_hash: impl Into<String>) -> Result<Self, ContractError> {
        Self::checked(result_ref_hash.into(), None)
    }

    /// A receipt for an effect that an external system recorded.
    pub fn external(
        result_ref_hash: impl Into<String>,
        external_receipt_hash: impl Into<String>,
    ) -> Result<Self, ContractError> {
        Self::checked(result_ref_hash.into(), Some(external_receipt_hash.into()))
    }

    fn checked(
        result_ref_hash: String,
        external_receipt_hash: Option<String>,
    ) -> Result<Self, ContractError> {
        if !is_sha256(&result_ref_hash)
            || external_receipt_hash
                .as_deref()
                .is_some_and(|hash| !is_sha256(hash))
        {
            return Err(ContractError::InvalidSettlementReference);
        }
        Ok(Self {
            result_ref_hash,
            external_receipt_hash,
        })
    }

    pub fn result_ref_hash(&self) -> &str {
        &self.result_ref_hash
    }

    pub fn external_receipt_hash(&self) -> Option<&str> {
        self.external_receipt_hash.as_deref()
    }

    /// `true` when this receipt carries `request`'s expected result
    /// reference. The runner fails the leg closed, without a retry, on
    /// `false`.
    pub fn answers(&self, request: &SettlementRequest) -> bool {
        self.result_ref_hash == request.expected_result_ref_hash
    }
}

/// Why a settlement attempt did not produce a result.
#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum SettlementError {
    /// The adapter could not complete the effect now, or cannot say whether
    /// it completed. The effect may or may not have happened. Any later
    /// attempt for this operation MUST reuse the same request; the
    /// adapter's idempotency contract on [`SettlementAdapter::settle`] is
    /// what makes that safe. Whether the runner retries or forfeits the leg
    /// (for example after a withdrawal) is runtime policy.
    #[error("settlement adapter is unavailable")]
    Unavailable,
    /// The operation reference was used before with different request
    /// content. No effect happened, and the runner MUST NOT retry the
    /// operation.
    #[error("settlement request conflicts with a prior operation")]
    Conflict,
    /// The adapter refuses this request (for example, an instrument that it
    /// does not settle). No effect happened, and the runner MUST NOT retry
    /// the operation.
    #[error("settlement request was rejected")]
    Rejected,
}

impl SettlementError {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "settlement_adapter_unavailable",
            Self::Conflict => "settlement_request_conflict",
            Self::Rejected => "settlement_request_rejected",
        }
    }
}

/// Performs one instrument's external settlement effect. A proprietary
/// backend implements this; the runner holds it as a trait object.
#[async_trait]
pub trait SettlementAdapter: Send + Sync {
    /// The one instrument this adapter settles.
    fn instrument_id(&self) -> &InstrumentId;

    /// A safe label (`^[a-z0-9_]{1,64}$`) that names the implementation in
    /// hash-only reports.
    fn adapter_identity(&self) -> &str;

    /// `false` by default. Readiness fails closed on `false`.
    fn production_qualified(&self) -> bool {
        false
    }

    /// A safe label for the external payout rail, for example `near`, or
    /// `none` when there is no external payout.
    fn payout_rail(&self) -> &str;

    /// Performs the instrument's external effect for one settlement
    /// operation and returns its receipt.
    ///
    /// Idempotency contract: an adapter must return the same receipt,
    /// including the same `external_receipt_hash`, for a repeated
    /// `request.operation_ref_hash` and must not repeat its effect.
    /// Recovery depends on this. The pipeline calls `settle` before it
    /// records the leg durably, so a crash, a stale lease, or a rolled-back
    /// ledger transaction after the call makes the next attempt call
    /// `settle` again with the same request; that call must be answered
    /// from the first one's outcome. A repeated `operation_ref_hash` with
    /// different request content is an error, never a second effect.
    ///
    /// On success, the receipt answers the request
    /// ([`SettlementReceipt::answers`]): its `result_ref_hash` is
    /// `request.expected_result_ref_hash`. Any other value fails the leg
    /// closed, and the runner does not retry it. The runner records the
    /// receipt's `external_receipt_hash` on the leg.
    async fn settle(
        &self,
        request: &SettlementRequest,
    ) -> Result<SettlementReceipt, SettlementError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;
    use std::pin::{Pin, pin};
    use std::sync::Arc;
    use std::task::{Context, Poll, Waker};

    /// Resolves on its second poll, not its first: `Pending` (waking the
    /// waker so a real executor would re-poll), then `Ready(())`. Lets a
    /// test prove that a future can suspend, which a `settle` body with no
    /// await point cannot do.
    struct YieldOnce {
        yielded: bool,
    }

    impl YieldOnce {
        fn new() -> Self {
            Self { yielded: false }
        }
    }

    impl Future for YieldOnce {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.yielded {
                Poll::Ready(())
            } else {
                self.yielded = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    struct TestAdapter {
        instrument_id: InstrumentId,
        result: SettlementReceipt,
    }

    #[async_trait]
    impl SettlementAdapter for TestAdapter {
        fn instrument_id(&self) -> &InstrumentId {
            &self.instrument_id
        }

        fn adapter_identity(&self) -> &str {
            "test_adapter"
        }

        fn payout_rail(&self) -> &str {
            "test_rail"
        }

        async fn settle(
            &self,
            _request: &SettlementRequest,
        ) -> Result<SettlementReceipt, SettlementError> {
            YieldOnce::new().await;
            Ok(self.result.clone())
        }
    }

    fn request() -> SettlementRequest {
        request_with(InstrumentId::trace_credit(), 1, OPERATION, RESULT).unwrap()
    }

    #[test]
    fn settlement_adapter_is_object_safe_and_async() {
        let adapter: Arc<dyn SettlementAdapter> = Arc::new(TestAdapter {
            instrument_id: InstrumentId::trace_credit(),
            result: SettlementReceipt::external(RESULT, RECEIPT).unwrap(),
        });
        assert!(!adapter.production_qualified());

        let request = request();
        let future = adapter.settle(&request);
        let mut future = pin!(future);
        let mut cx = Context::from_waker(Waker::noop());

        assert!(
            matches!(future.as_mut().poll(&mut cx), Poll::Pending),
            "first poll must suspend at the yield point"
        );
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(result) => assert!(result.unwrap().answers(&request)),
            Poll::Pending => panic!("test adapter's future must resolve on the second poll"),
        }
    }

    #[test]
    fn settlement_error_labels_are_safe() {
        let labels = [
            SettlementError::Unavailable.label(),
            SettlementError::Conflict.label(),
            SettlementError::Rejected.label(),
        ];
        for label in labels {
            assert!(
                !label.is_empty()
                    && label.len() <= 64
                    && label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "label {label} is not safe"
            );
        }
        assert_ne!(labels[0], labels[1]);
        assert_ne!(labels[0], labels[2]);
        assert_ne!(labels[1], labels[2]);
    }

    const OPERATION: &str =
        "sha256:af1bf43fd32181119472618d89802ba083b6a025b41aef9f60e71593b0b9117e";
    const RESULT: &str = "sha256:3c101340d8a3b60c110d1bb23eab041c04ec9c9ab0ad3df00629fb443b67ed30";
    const RECEIPT: &str = "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

    fn tenant() -> TenantStorageRef {
        TenantStorageRef::new("tenant_sha256:00112233445566778899aabbccddeeff").unwrap()
    }

    fn request_with(
        instrument_id: InstrumentId,
        atomic_units: u128,
        operation_ref_hash: &str,
        expected_result_ref_hash: &str,
    ) -> Result<SettlementRequest, ContractError> {
        SettlementRequest::new(
            tenant(),
            Uuid::nil(),
            instrument_id,
            AtomicUnits::from_raw(atomic_units),
            operation_ref_hash,
            expected_result_ref_hash,
        )
    }

    #[test]
    fn settlement_request_refuses_malformed_references() {
        let uppercase = OPERATION.replace("af1b", "AF1B");
        for (operation, result) in [
            ("not-a-hash", RESULT),
            (OPERATION, "not-a-hash"),
            (uppercase.as_str(), RESULT),
            (OPERATION, "sha256:3c10"),
            ("", RESULT),
        ] {
            assert_eq!(
                request_with(InstrumentId::trace_credit(), 1, operation, result),
                Err(ContractError::InvalidSettlementReference),
                "{operation} / {result}"
            );
        }
        let request = request_with(InstrumentId::trace_credit(), 1, OPERATION, RESULT).unwrap();
        assert_eq!(request.operation_ref_hash(), OPERATION);
        assert_eq!(request.expected_result_ref_hash(), RESULT);
    }

    #[test]
    fn settlement_request_refuses_amounts_no_leg_can_hold() {
        assert_eq!(
            request_with(InstrumentId::trace_credit(), 0, OPERATION, RESULT),
            Err(ContractError::ZeroInstrumentAward)
        );
        assert_eq!(
            request_with(
                InstrumentId::trace_credit(),
                u128::from(i64::MAX as u64) + 1,
                OPERATION,
                RESULT
            ),
            Err(ContractError::TraceCreditOutOfRange)
        );
        assert!(
            request_with(
                InstrumentId::new("bat").unwrap(),
                u128::MAX,
                OPERATION,
                RESULT
            )
            .is_ok()
        );
    }

    #[test]
    fn settlement_receipt_refuses_malformed_references() {
        assert_eq!(
            SettlementReceipt::external("not-a-hash", RECEIPT),
            Err(ContractError::InvalidSettlementReference)
        );
        assert_eq!(
            SettlementReceipt::external(RESULT, "0xabc"),
            Err(ContractError::InvalidSettlementReference)
        );
        assert_eq!(
            SettlementReceipt::internal("not-a-hash"),
            Err(ContractError::InvalidSettlementReference)
        );

        let internal = SettlementReceipt::internal(RESULT).unwrap();
        assert_eq!(internal.result_ref_hash(), RESULT);
        assert_eq!(internal.external_receipt_hash(), None);

        let external = SettlementReceipt::external(RESULT, RECEIPT).unwrap();
        assert_eq!(external.result_ref_hash(), RESULT);
        assert_eq!(external.external_receipt_hash(), Some(RECEIPT));
    }

    #[test]
    fn settlement_receipt_matches_only_its_own_request() {
        let request = request_with(InstrumentId::trace_credit(), 1, OPERATION, RESULT).unwrap();
        let receipt = SettlementReceipt::external(RESULT, RECEIPT).unwrap();
        assert!(receipt.answers(&request));

        let other = request_with(InstrumentId::trace_credit(), 1, OPERATION, RECEIPT).unwrap();
        assert!(!receipt.answers(&other));
    }
}
