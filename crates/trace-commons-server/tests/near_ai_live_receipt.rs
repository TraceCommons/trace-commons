// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The receipt verifier, against a real NEAR AI triple.
//!
//! Every other test of `near_attestation::receipt` synthesizes its own
//! signature with a fixed key, which proves the verifier is self-consistent
//! and nothing about what the live service actually signs. This one runs the
//! production verifier over bytes captured from the pilot's inference
//! endpoint: the request as sent, the response as received, and the receipt
//! the enclave returned for them.
//!
//! It settled two things that reading the reference verifier could not:
//!
//! 1. The second hash is over the **entire raw response body**, not over
//!    `choices[0].message.content`. The drill hashed the message content; it
//!    would have failed at `receipt_verified:response_hash_mismatch` on its
//!    first real call, and nothing in CI would have caught it.
//! 2. The three-part `text`'s leading part is the **model name**. NEAR AI's
//!    reference verifier discards it, so what it held was unknown; the
//!    capture shows it, and the verifier now checks it.
//!
//! The captured response has `message.content: null` with the text in
//! `reasoning_content` and `finish_reason: "length"` -- the endpoint serves a
//! thinking model. That is why (1) was invisible to reasoning and obvious to
//! data: against this body the old code could not even deserialize.
//!
//! The fixture's signer differs from the one in
//! `near_ai_attestation_report.json`: a different model host with a different
//! enclave key. Neither fixture is canonical, and nothing may assume a single
//! signing address across them.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest as _, Sha256};
use trace_commons_server::near_attestation::AttestationReport;
use trace_commons_server::near_attestation::drill::attested_signer_address;
use trace_commons_server::near_attestation::receipt::{
    ReceiptError, ReceiptPayload, verify_receipt,
};

const TRIPLE: &str = include_str!("fixtures/near_ai_live_triple.json");

/// Byte offset of `report_data` within a TDX v4 quote: a 48-byte header
/// followed by 520 bytes of TD report body before it. Read directly here
/// because verifying the quote's signature chain needs Intel collateral this
/// fixture deliberately does not carry -- production reads the same field out
/// of a *verified* quote, never like this.
const TDX_QUOTE_REPORT_DATA_OFFSET: usize = 568;

struct Triple {
    report: AttestationReport,
    nonce: String,
    quote: Vec<u8>,
    request_body: Vec<u8>,
    response_body: Vec<u8>,
    receipt: ReceiptPayload,
    expected_request_sha256: String,
    expected_response_sha256: String,
}

fn triple() -> Triple {
    let v: serde_json::Value = serde_json::from_str(TRIPLE).expect("fixture parses");
    let report = AttestationReport::from_json(&v["report"].to_string()).expect("report parses");
    let quote = report.quote_bytes().expect("quote decodes");
    let text = |path: &str| v[path].as_str().expect("string field").to_string();
    let b64 = |path: &str| {
        BASE64
            .decode(v[path].as_str().expect("string field"))
            .expect("base64 decodes")
    };
    Triple {
        nonce: text("_nonce"),
        quote,
        request_body: b64("request_body_b64"),
        response_body: b64("response_body_b64"),
        receipt: ReceiptPayload {
            text: v["receipt"]["text"].as_str().expect("text").to_string(),
            signature: v["receipt"]["signature"]
                .as_str()
                .expect("signature")
                .to_string(),
            signing_address: v["receipt"]["signing_address"]
                .as_str()
                .expect("signing_address")
                .to_string(),
        },
        expected_request_sha256: v["_checks"]["request_sha256"]
            .as_str()
            .expect("request_sha256")
            .to_string(),
        expected_response_sha256: v["_checks"]["response_sha256"]
            .as_str()
            .expect("response_sha256")
            .to_string(),
        report,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The signer the quote attests, read out of the raw quote's report data.
fn quote_attested_signer(quote: &[u8]) -> String {
    let start = TDX_QUOTE_REPORT_DATA_OFFSET;
    attested_signer_address(&quote[start..start + 64]).expect("report data is in signer mode")
}

#[test]
fn a_real_receipt_verifies_against_the_bytes_the_service_signed() {
    let t = triple();
    let verdict = verify_receipt(
        &t.receipt,
        &t.request_body,
        &t.response_body,
        &t.report.model_name,
    )
    .expect("a real receipt from a real endpoint must verify");

    assert_eq!(verdict.request_sha256, t.expected_request_sha256);
    assert_eq!(verdict.response_sha256, t.expected_response_sha256);
    assert_eq!(verdict.model.as_deref(), Some(t.report.model_name.as_str()));
    // The signature really did recover to the address the provider claimed;
    // `verify_receipt` refuses otherwise, and the verdict carries the
    // recovered one, not the claimed one.
    assert!(
        verdict
            .signing_address
            .eq_ignore_ascii_case(&t.receipt.signing_address)
    );
}

#[test]
fn the_hash_is_over_the_whole_body_and_not_the_message_content() {
    // The bug this fixture exists to have caught. `content` is `null` here,
    // so the closest thing the old code could have hashed is the empty
    // string; the digests are measured to differ rather than assumed to.
    let t = triple();
    assert_eq!(sha256_hex(&t.response_body), t.expected_response_sha256);

    let parsed: serde_json::Value = serde_json::from_slice(&t.response_body).expect("body parses");
    let content = &parsed["choices"][0]["message"]["content"];
    assert!(
        content.is_null(),
        "the captured endpoint is a thinking model; content must be null here"
    );
    assert!(
        parsed["choices"][0]["message"]["reasoning_content"].is_string(),
        "the text is in reasoning_content, which is why content is null"
    );
    assert_ne!(sha256_hex(b""), t.expected_response_sha256);

    // And the verifier refuses that substitution rather than passing it.
    assert_eq!(
        verify_receipt(&t.receipt, &t.request_body, b"", &t.report.model_name)
            .expect_err("must be refused"),
        ReceiptError::ResponseHashMismatch
    );
}

#[test]
fn the_receipt_binds_the_model_that_served_it() {
    // Real evidence that the leading part is the model name: it equals the
    // report's own `model_name`, and asking for anything else is refused.
    let t = triple();
    let parts: Vec<&str> = t.receipt.text.split(':').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], t.report.model_name);

    assert_eq!(
        verify_receipt(
            &t.receipt,
            &t.request_body,
            &t.response_body,
            "some/other-model",
        )
        .expect_err("must be refused"),
        ReceiptError::ModelMismatch
    );
}

#[test]
fn the_receipts_signer_is_the_key_the_quote_attests() {
    // The join the drill makes: without it the enclave proof and the receipt
    // are two facts about possibly different machines.
    let t = triple();
    let verdict = verify_receipt(
        &t.receipt,
        &t.request_body,
        &t.response_body,
        &t.report.model_name,
    )
    .expect("verifies");

    let attested = quote_attested_signer(&t.quote);
    assert!(
        verdict.signing_address.eq_ignore_ascii_case(&attested),
        "the receipt signer must be the key the quote attests"
    );
    // The report's unsigned JSON agrees too. Reporting only -- the quote is
    // what makes the claim, and this is here to show they do not disagree.
    assert!(t.report.signing_address.eq_ignore_ascii_case(&attested));
}

#[test]
fn the_captured_nonce_is_bound_inside_the_signed_quote() {
    // Otherwise the report is a replay and says nothing about this session.
    let t = triple();
    assert!(
        t.report
            .quote_binds_nonce(&t.nonce)
            .expect("nonce is well formed")
    );
}

#[test]
fn this_fixtures_signer_is_not_the_other_fixtures_signer() {
    // A different model host with a different enclave key. Pinned so that
    // nothing grows a constant assuming one signing address across fixtures.
    let t = triple();
    let other: serde_json::Value = serde_json::from_str(include_str!(
        "../../trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json"
    ))
    .expect("report fixture parses");
    let other_signer = other["signing_address"].as_str().expect("signing_address");
    assert!(!t.report.signing_address.eq_ignore_ascii_case(other_signer));
}
