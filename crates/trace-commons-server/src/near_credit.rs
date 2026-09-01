// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! NEAR non-transferable credit receipt payloads.
//!
//! The server keeps the off-chain settlement ledger authoritative. These types
//! build deterministic method-call payloads for a NEAR contract that mirrors
//! finalized settlement state without exposing raw trace content or enabling
//! contributor-to-contributor transfers.

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use trace_commons_protocol::canonical_json;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NearCreditReceipt {
    pub settlement_batch_id: Uuid,
    pub credit_account_hash: String,
    pub policy_version: String,
    pub source_list_hash: String,
    pub attestation_hash: String,
    pub amount_micros: i64,
    pub issuer_signature_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NearCreditReceiptCall {
    pub contract_id: String,
    pub method_name: String,
    pub args: Value,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NearCreditReceiptArgs {
    settlement_batch_id: Uuid,
    credit_account_hash: String,
    policy_version: String,
    source_list_hash: String,
    attestation_hash: String,
    amount_micros: i64,
    issuer_signature_hash: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NearCreditFreezeAccountArgs {
    credit_account_hash: String,
    reason_hash: String,
}

impl NearCreditReceiptCall {
    pub fn settle(
        contract_id: impl Into<String>,
        receipt: NearCreditReceipt,
    ) -> anyhow::Result<Self> {
        validate_receipt(&receipt)?;
        Self::raw(
            contract_id,
            "settle_credit_receipt",
            json!({
                "settlement_batch_id": receipt.settlement_batch_id,
                "credit_account_hash": receipt.credit_account_hash,
                "policy_version": receipt.policy_version,
                "source_list_hash": receipt.source_list_hash,
                "attestation_hash": receipt.attestation_hash,
                "amount_micros": receipt.amount_micros,
                "issuer_signature_hash": receipt.issuer_signature_hash,
            }),
        )
    }

    pub fn reverse(
        contract_id: impl Into<String>,
        receipt: NearCreditReceipt,
    ) -> anyhow::Result<Self> {
        validate_receipt(&receipt)?;
        Self::raw(
            contract_id,
            "reverse_credit_receipt",
            json!({
                "settlement_batch_id": receipt.settlement_batch_id,
                "credit_account_hash": receipt.credit_account_hash,
                "policy_version": receipt.policy_version,
                "source_list_hash": receipt.source_list_hash,
                "attestation_hash": receipt.attestation_hash,
                "amount_micros": receipt.amount_micros,
                "issuer_signature_hash": receipt.issuer_signature_hash,
            }),
        )
    }

    pub fn freeze_account(
        contract_id: impl Into<String>,
        credit_account_hash: impl Into<String>,
        reason_hash: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let credit_account_hash = credit_account_hash.into();
        let reason_hash = reason_hash.into();
        ensure_hash_like("credit_account_hash", &credit_account_hash)?;
        ensure_hash_like("reason_hash", &reason_hash)?;
        Self::raw(
            contract_id,
            "freeze_credit_account",
            json!({
                "credit_account_hash": credit_account_hash,
                "reason_hash": reason_hash,
            }),
        )
    }

    pub fn unfreeze_account(
        contract_id: impl Into<String>,
        credit_account_hash: impl Into<String>,
        reason_hash: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let credit_account_hash = credit_account_hash.into();
        let reason_hash = reason_hash.into();
        ensure_hash_like("credit_account_hash", &credit_account_hash)?;
        ensure_hash_like("reason_hash", &reason_hash)?;
        Self::raw(
            contract_id,
            "unfreeze_credit_account",
            json!({
                "credit_account_hash": credit_account_hash,
                "reason_hash": reason_hash,
            }),
        )
    }

    pub fn raw(
        contract_id: impl Into<String>,
        method_name: impl Into<String>,
        args: Value,
    ) -> anyhow::Result<Self> {
        let contract_id = validate_near_contract_id(contract_id.into())?;
        let method_name = method_name.into().trim().to_string();
        ensure_non_transferable_method(&method_name)?;
        validate_method_args(&method_name, &args)?;
        // The idempotency key is a hash of these bytes and backs a UNIQUE
        // constraint on the outbox, so the key order the args serialize in
        // decides whether a replay collapses or duplicates. `serde_json`
        // gives key order only while its map is a `BTreeMap`; store the args
        // already sorted so `validate` recomputes the same key from what it
        // reads back. A no-op under today's feature set.
        let mut args = args;
        canonical_json::canonicalize(&mut args);
        let canonical_args =
            serde_json::to_string(&args).context("failed to serialize NEAR call args")?;
        let idempotency_key =
            sha256_prefixed(format!("{contract_id}\n{method_name}\n{canonical_args}").as_bytes());
        Ok(Self {
            contract_id,
            method_name,
            args,
            idempotency_key,
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        let contract_id = validate_near_contract_id(self.contract_id.clone())?;
        anyhow::ensure!(
            contract_id == self.contract_id,
            "NEAR credit call contract id is not canonical"
        );
        let method_name = self.method_name.trim();
        ensure_non_transferable_method(method_name)?;
        anyhow::ensure!(
            method_name == self.method_name,
            "NEAR credit call method name is not canonical"
        );
        validate_method_args(method_name, &self.args)?;
        let canonical_args = canonical_json::to_canonical_string(&self.args)
            .context("failed to serialize NEAR call args for validation")?;
        let expected_idempotency_key = sha256_prefixed(
            format!(
                "{}\n{}\n{}",
                self.contract_id, self.method_name, canonical_args
            )
            .as_bytes(),
        );
        anyhow::ensure!(
            self.idempotency_key == expected_idempotency_key,
            "NEAR credit call idempotency key does not match canonical payload"
        );
        Ok(())
    }
}

fn validate_receipt(receipt: &NearCreditReceipt) -> anyhow::Result<()> {
    ensure_hash_like("credit_account_hash", &receipt.credit_account_hash)?;
    ensure_hash_like("source_list_hash", &receipt.source_list_hash)?;
    ensure_hash_like("attestation_hash", &receipt.attestation_hash)?;
    ensure_hash_like("issuer_signature_hash", &receipt.issuer_signature_hash)?;
    ensure_policy_version_like("policy_version", &receipt.policy_version)?;
    anyhow::ensure!(
        receipt.amount_micros > 0,
        "settled NEAR credit amount must be positive"
    );
    Ok(())
}

fn validate_method_args(method_name: &str, args: &Value) -> anyhow::Result<()> {
    match method_name {
        "settle_credit_receipt" | "reverse_credit_receipt" => {
            let args: NearCreditReceiptArgs = serde_json::from_value(args.clone())
                .context("invalid NEAR credit receipt method args")?;
            validate_receipt(&NearCreditReceipt {
                settlement_batch_id: args.settlement_batch_id,
                credit_account_hash: args.credit_account_hash,
                policy_version: args.policy_version,
                source_list_hash: args.source_list_hash,
                attestation_hash: args.attestation_hash,
                amount_micros: args.amount_micros,
                issuer_signature_hash: args.issuer_signature_hash,
            })?;
        }
        "freeze_credit_account" | "unfreeze_credit_account" => {
            let args: NearCreditFreezeAccountArgs = serde_json::from_value(args.clone())
                .context("invalid NEAR credit freeze method args")?;
            ensure_hash_like("credit_account_hash", &args.credit_account_hash)?;
            ensure_hash_like("reason_hash", &args.reason_hash)?;
        }
        _ => ensure_non_transferable_method(method_name)?,
    }
    Ok(())
}

fn validate_near_contract_id(contract_id: String) -> anyhow::Result<String> {
    anyhow::ensure!(
        !contract_id.trim().is_empty(),
        "NEAR contract id is required"
    );
    anyhow::ensure!(
        contract_id.trim() == contract_id,
        "NEAR contract id must not contain leading or trailing whitespace"
    );
    anyhow::ensure!(
        (2..=64).contains(&contract_id.len()),
        "NEAR contract id must be between 2 and 64 characters"
    );

    let mut previous_separator = false;
    for (index, byte) in contract_id.bytes().enumerate() {
        let is_account_char = byte.is_ascii_lowercase() || byte.is_ascii_digit();
        let is_separator = matches!(byte, b'.' | b'-' | b'_');
        anyhow::ensure!(
            is_account_char || is_separator,
            "NEAR contract id must contain only lowercase account-id characters"
        );
        anyhow::ensure!(
            !(index == 0 && is_separator),
            "NEAR contract id must not start with a separator"
        );
        anyhow::ensure!(
            !(previous_separator && is_separator),
            "NEAR contract id must not contain consecutive separators"
        );
        previous_separator = is_separator;
    }
    anyhow::ensure!(
        !previous_separator,
        "NEAR contract id must not end with a separator"
    );

    Ok(contract_id)
}

fn ensure_hash_like(label: &str, value: &str) -> anyhow::Result<()> {
    let Some(hex_digest) = value.strip_prefix("sha256:") else {
        anyhow::bail!("{label} must be a canonical sha256-prefixed hex digest");
    };
    anyhow::ensure!(
        hex_digest.len() == 64
            && hex_digest
                .chars()
                .all(|ch| ch.is_ascii_digit() || matches!(ch, 'a'..='f')),
        "{label} must be a canonical sha256-prefixed hex digest"
    );
    Ok(())
}

fn ensure_policy_version_like(label: &str, value: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!value.trim().is_empty(), "{label} is required");
    anyhow::ensure!(
        value.trim() == value,
        "{label} must not contain leading or trailing whitespace"
    );
    anyhow::ensure!(value.len() <= 256, "{label} is too long");
    anyhow::ensure!(
        value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':')),
        "{label} contains unsupported characters"
    );
    Ok(())
}

fn ensure_non_transferable_method(method_name: &str) -> anyhow::Result<()> {
    let normalized = method_name.trim();
    anyhow::ensure!(
        matches!(
            normalized,
            "settle_credit_receipt"
                | "reverse_credit_receipt"
                | "freeze_credit_account"
                | "unfreeze_credit_account"
        ),
        "NEAR credit receipts are non-transferable; unsupported credit methods are not allowed"
    );
    Ok(())
}

fn sha256_prefixed(input: &[u8]) -> String {
    let digest = Sha256::digest(input);
    format!("sha256:{}", hex::encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDIT_ACCOUNT_HASH: &str =
        "sha256:1111111111111111111111111111111111111111111111111111111111111111";
    const SOURCE_LIST_HASH: &str =
        "sha256:2222222222222222222222222222222222222222222222222222222222222222";
    const ATTESTATION_HASH: &str =
        "sha256:3333333333333333333333333333333333333333333333333333333333333333";
    const ISSUER_SIGNATURE_HASH: &str =
        "sha256:4444444444444444444444444444444444444444444444444444444444444444";

    fn fixture_receipt() -> NearCreditReceipt {
        NearCreditReceipt {
            settlement_batch_id: Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef),
            credit_account_hash: CREDIT_ACCOUNT_HASH.to_string(),
            policy_version: "credit-policy.v1".to_string(),
            source_list_hash: SOURCE_LIST_HASH.to_string(),
            attestation_hash: ATTESTATION_HASH.to_string(),
            amount_micros: 1_500_000,
            issuer_signature_hash: ISSUER_SIGNATURE_HASH.to_string(),
        }
    }

    /// The idempotency key backs `UNIQUE (tenant_id, idempotency_key)` on the
    /// settlement outbox, so a shift in it is a duplicate submission rather
    /// than a failed one -- there is no error to notice.
    ///
    /// Pinned to a literal, over args whose `json!` declaration order is
    /// deliberately not alphabetical.
    ///
    /// This crate depends on `dcap-qvl`, whose mandatory `std` feature turns
    /// on `serde_json/preserve_order`, so `serde_json::Map` here is an
    /// insertion-ordered `IndexMap` and this test is **falsifiable in the
    /// ordinary suite**: drop the canonicalization in `raw` and the args
    /// serialize in declaration order, the key moves off this literal, and
    /// this fails. It used to pass either way, under a `BTreeMap` that handed
    /// back sorted keys with or without `canonicalize`, and only the
    /// `serde_json preserve_order guard` CI job made it mean anything. That
    /// job now covers `trace-commons-protocol` alone, which is the graph
    /// still backed by a `BTreeMap`.
    #[test]
    fn the_settle_idempotency_key_is_pinned_over_key_ordered_args() {
        let call = NearCreditReceiptCall::settle("credit.tracecommons.near", fixture_receipt())
            .expect("settle call");
        assert_eq!(
            call.idempotency_key,
            "sha256:672567e8705fc1b61cff0a082b244b26c42563914744af32bd85e86f04702b23"
        );
    }

    /// The args are stored already sorted, not merely hashed sorted, so
    /// `validate` recomputes the same key from what it reads back out of the
    /// outbox row.
    #[test]
    fn the_stored_args_are_themselves_key_ordered() {
        let call = NearCreditReceiptCall::settle("credit.tracecommons.near", fixture_receipt())
            .expect("settle call");
        assert_eq!(
            serde_json::to_string(&call.args).expect("serialize"),
            canonical_json::to_canonical_string(&call.args).expect("serialize")
        );
        call.validate().expect("round trip");
    }

    #[test]
    fn validate_rejects_a_key_that_does_not_match_the_payload() {
        let mut call = NearCreditReceiptCall::settle("credit.tracecommons.near", fixture_receipt())
            .expect("settle call");
        call.idempotency_key = sha256_prefixed(b"not the payload");
        let error = call.validate().expect_err("mismatched key must be refused");
        assert!(
            error
                .to_string()
                .contains("idempotency key does not match canonical payload"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn the_freeze_and_settle_keys_do_not_collide() {
        let settle = NearCreditReceiptCall::settle("credit.tracecommons.near", fixture_receipt())
            .expect("settle call");
        let freeze = NearCreditReceiptCall::freeze_account(
            "credit.tracecommons.near",
            CREDIT_ACCOUNT_HASH,
            SOURCE_LIST_HASH,
        )
        .expect("freeze call");
        assert_ne!(settle.idempotency_key, freeze.idempotency_key);
        freeze.validate().expect("round trip");
    }
}
