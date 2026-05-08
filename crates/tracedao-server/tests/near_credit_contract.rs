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

#[test]
fn near_credit_receipt_call_rejects_unknown_credit_methods() {
    let receipt = sample_receipt(Uuid::from_u128(0x150));
    NearCreditReceiptCall::settle("trace-credits.testnet", receipt.clone())
        .expect("settlement method is allowed");
    NearCreditReceiptCall::reverse("trace-credits.testnet", receipt)
        .expect("reversal method is allowed");
    NearCreditReceiptCall::freeze_account(
        "trace-credits.testnet",
        "sha256:account",
        "sha256:freeze-reason",
    )
    .expect("freeze method is allowed");
    NearCreditReceiptCall::unfreeze_account(
        "trace-credits.testnet",
        "sha256:account",
        "sha256:unfreeze-reason",
    )
    .expect("unfreeze method is allowed");

    let error =
        NearCreditReceiptCall::raw("trace-credits.testnet", "mint_credit_receipt", json!({}))
            .expect_err("unknown methods are outside the non-transferable credit API");

    assert!(error.to_string().contains("non-transferable"));
}

#[test]
fn near_credit_receipt_call_rejects_malformed_allowed_method_args() {
    let missing_args = NearCreditReceiptCall::raw(
        "trace-credits.testnet",
        "settle_credit_receipt",
        json!({
            "credit_account_hash": "sha256:account"
        }),
    )
    .expect_err("settlement method requires the full receipt shape");
    assert!(
        missing_args
            .to_string()
            .contains("invalid NEAR credit receipt")
    );

    let extra_args = NearCreditReceiptCall::raw(
        "trace-credits.testnet",
        "freeze_credit_account",
        json!({
            "credit_account_hash": "sha256:account",
            "reason_hash": "sha256:freeze-reason",
            "raw_reason": "do not store this"
        }),
    )
    .expect_err("freeze method rejects unexpected raw fields");
    assert!(
        extra_args
            .to_string()
            .contains("invalid NEAR credit freeze")
    );
}

#[test]
fn near_credit_receipt_call_validate_rejects_tampered_idempotency_key() {
    let mut call = NearCreditReceiptCall::settle(
        "trace-credits.testnet",
        sample_receipt(Uuid::from_u128(0x175)),
    )
    .expect("settlement call builds");
    call.idempotency_key = "sha256:tampered".to_string();

    let error = call
        .validate()
        .expect_err("tampered NEAR outbox calls are rejected before relayer submit");

    assert!(error.to_string().contains("idempotency key"));
}

#[test]
fn near_credit_receipt_call_rejects_malformed_contract_ids() {
    let receipt = sample_receipt(Uuid::from_u128(0x200));

    for contract_id in [
        " Trace-Credits.testnet",
        "Trace-Credits.testnet",
        "trace credits.testnet",
        "trace-credits..testnet",
        "-trace-credits.testnet",
        "trace-credits.testnet-",
    ] {
        let error = NearCreditReceiptCall::settle(contract_id, receipt.clone())
            .expect_err("malformed NEAR account ids are rejected");
        assert!(
            error.to_string().contains("NEAR contract id"),
            "unexpected error for {contract_id}: {error}"
        );
    }
}

fn sample_receipt(settlement_batch_id: Uuid) -> NearCreditReceipt {
    NearCreditReceipt {
        settlement_batch_id,
        credit_account_hash: "sha256:account".to_string(),
        policy_version: "trace-credit-policy-v1".to_string(),
        source_list_hash: "sha256:sources".to_string(),
        attestation_hash: "sha256:attestation".to_string(),
        amount_micros: 1_750_000,
        issuer_signature_hash: "sha256:issuer-signature".to_string(),
    }
}
