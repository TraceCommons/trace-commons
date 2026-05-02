use serde_json::json;
use tracedao_server::near_credit::{NearCreditReceipt, NearCreditReceiptCall};
use uuid::Uuid;

#[test]
fn near_credit_receipt_call_is_hash_only_and_deterministic() {
    let batch_id = Uuid::from_u128(0x100);
    let receipt = NearCreditReceipt {
        settlement_batch_id: batch_id,
        credit_account_hash: "sha256:account".to_string(),
        policy_version: "trace-credit-policy-v1".to_string(),
        source_list_hash: "sha256:sources".to_string(),
        attestation_hash: "sha256:attestation".to_string(),
        amount_micros: 1_750_000,
        issuer_signature_hash: "sha256:issuer-signature".to_string(),
    };

    let call = NearCreditReceiptCall::settle("trace-credits.testnet", receipt.clone())
        .expect("settlement call builds");
    let retry =
        NearCreditReceiptCall::settle("trace-credits.testnet", receipt).expect("retry call builds");

    assert_eq!(call, retry);
    assert_eq!(call.contract_id, "trace-credits.testnet");
    assert_eq!(call.method_name, "settle_credit_receipt");
    assert_eq!(
        call.args,
        json!({
            "settlement_batch_id": batch_id,
            "credit_account_hash": "sha256:account",
            "policy_version": "trace-credit-policy-v1",
            "source_list_hash": "sha256:sources",
            "attestation_hash": "sha256:attestation",
            "amount_micros": 1_750_000,
            "issuer_signature_hash": "sha256:issuer-signature"
        })
    );
    assert!(call.idempotency_key.starts_with("sha256:"));
    let serialized = serde_json::to_string(&call).expect("call serializes");
    assert!(!serialized.contains("trace body"));
    assert!(!serialized.contains("raw contributor"));
}

#[test]
fn near_credit_receipt_call_rejects_transfer_methods() {
    let error = NearCreditReceiptCall::raw("trace-credits.testnet", "ft_transfer", json!({}))
        .expect_err("transfer calls are not part of the non-transferable scope");

    assert!(error.to_string().contains("non-transferable"));
}
