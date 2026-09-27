// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The settlement adapter seam for the versioned pipeline (#971).
//!
//! [`SettlementAdapter`] is the trait object a proprietary backend
//! implements to perform one instrument's external effect for a settlement
//! operation. A settle policy holds it as a trait object, never a concrete
//! type, so a substituted backend can slot in at this seam.

use async_trait::async_trait;
use uuid::Uuid;

use crate::pipeline::{AtomicUnits, InstrumentId, TenantStorageRef};

/// One settlement operation: enough for an adapter to perform the
/// instrument's external effect and to prove, on retry, that a repeated
/// request carries the same content.
///
/// `tenant_storage_ref` is the tenant's derived storage reference, not the
/// raw tenant identifier — a gate-api seam never carries the raw tenant id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementRequest {
    pub tenant_storage_ref: TenantStorageRef,
    pub run_id: Uuid,
    pub instrument_id: InstrumentId,
    pub atomic_units: AtomicUnits,
    pub operation_ref_hash: String,
    pub expected_result_ref_hash: String,
}

/// Why a settlement attempt did not produce a result.
#[derive(Debug, Clone, Copy, thiserror::Error, PartialEq, Eq)]
pub enum SettlementError {
    /// The adapter could not complete the effect now, or cannot say whether
    /// it completed. The runner MUST retry with the same request; the
    /// adapter's idempotency contract on [`SettlementAdapter::settle`] is
    /// what makes that safe.
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
/// backend implements this; the pipeline runner holds it as a trait object.
#[async_trait]
pub trait SettlementAdapter: Send + Sync {
    fn instrument_id(&self) -> &InstrumentId;
    fn adapter_identity(&self) -> &str;
    fn production_qualified(&self) -> bool {
        false
    }
    fn payout_rail(&self) -> &str;

    /// Performs the instrument's external effect for one settlement
    /// operation and returns its result reference.
    ///
    /// Idempotency contract: an adapter must return the same result for a
    /// repeated `request.operation_ref_hash` and must not repeat its effect.
    /// Recovery depends on this. The pipeline calls `settle` before it
    /// records the leg durably, so a crash, a stale lease, or a rolled-back
    /// ledger transaction after the call makes the next attempt call
    /// `settle` again with the same request; that call must be answered
    /// from the first one's outcome. A repeated `operation_ref_hash` with
    /// different request content is an error, never a second effect.
    ///
    /// The returned string is the result reference. The runner compares it
    /// with `request.expected_result_ref_hash`.
    async fn settle(&self, request: &SettlementRequest) -> Result<String, SettlementError>;
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
        result: String,
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

        async fn settle(&self, _request: &SettlementRequest) -> Result<String, SettlementError> {
            YieldOnce::new().await;
            Ok(self.result.clone())
        }
    }

    fn request() -> SettlementRequest {
        SettlementRequest {
            tenant_storage_ref: TenantStorageRef::new(
                "tenant_sha256:00112233445566778899aabbccddeeff",
            )
            .unwrap(),
            run_id: Uuid::nil(),
            instrument_id: InstrumentId::trace_credit(),
            atomic_units: AtomicUnits::from_raw(1),
            operation_ref_hash: "sha256:abc".to_string(),
            expected_result_ref_hash: "sha256:def".to_string(),
        }
    }

    #[test]
    fn settlement_adapter_is_object_safe_and_async() {
        let adapter: Arc<dyn SettlementAdapter> = Arc::new(TestAdapter {
            instrument_id: InstrumentId::trace_credit(),
            result: "sha256:result".to_string(),
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
            Poll::Ready(result) => assert_eq!(result.unwrap(), "sha256:result"),
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
}
