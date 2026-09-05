//! Fetching the provider's receipt for one inference call.
//!
//! `GET {base}/signature/{chat_id}?model={model}&signing_algo=ed25519` returns
//! the enclave's EIP-191 signature over `<requestHash>:<responseHash>`. This
//! module is the only thing in the tree that calls it. Ed25519, because that
//! signer is the one bound into the gateway's attestation quote; the ECDSA
//! signer appears in no report.
//!
//! # Why the contributor fetches it
//!
//! Three facts decide this, and together they leave one place it can live.
//!
//! - **Only this machine has the identifier.** `chat_id` is
//!   [`RoutedExchange::upstream_id`](super::RoutedExchange::upstream_id),
//!   which exists in the local proxy's SQLite ledger and nowhere else. The
//!   server never sees it and could not ask for it without being told, at
//!   which point it is a caller-supplied identifier rather than a recorded
//!   one.
//! - **The receipt has to arrive with the submission.** The witness verifies
//!   it against the raw bodies *before* redaction, because redaction destroys
//!   the attested bytes. A receipt fetched later has nothing left to verify
//!   against.
//! - **The receipt is not a secret and not a credential.** It is a signature
//!   over two hashes. Fetching it on the contributor's machine leaks nothing
//!   the contributor does not already hold.
//!
//! # What the fetch itself discloses
//!
//! The receipt's *contents* leak nothing. The **request** does: a `GET` for a
//! `chat_id` tells the provider that this client is preparing to submit that
//! specific exchange somewhere, at this moment, from this address. That is a
//! disclosure the inference call alone did not make -- the provider already
//! knew the call happened, and now learns it is being contributed.
//!
//! Nothing here can avoid it while the receipt has to arrive with the
//! submission, and the disclosure goes to the party that already served the
//! call rather than to a new one. It is recorded because a contributor
//! choosing to enable attested submission should be told what the choice
//! costs, not because it is mitigated.
//!
//! The alternative -- the witness fetching it -- fails on the first point and
//! adds an egress dependency to an enclave whose whole design is that it
//! talks to as little as possible.
//!
//! # The model is a query parameter, and query parameters are not signed
//!
//! The endpoint requires `model`, and the receipts NEAR AI signs today are
//! the **two-part** form: `<requestHash>:<responseHash>`, with no model
//! prefix. So the model this module sends is chosen by whoever fetches, is
//! not covered by the signature, and establishes nothing. It is sent because
//! the endpoint demands it, and for no other reason. Nothing downstream may
//! treat the fetched receipt as binding a model.
//!
//! # Failure is absence
//!
//! Every error resolves to no receipt. A submission without one is honestly
//! unattested; a witness that requires attestation refuses it by name. There
//! is no partial success and nothing here can fail a submission.
//!
//! # Nothing here is logged
//!
//! The identifier, the model, the base URL and the receipt fields are all
//! caller data. [`ReceiptFetchError`] is label-only.

use std::time::Duration;

use trace_commons_attestation::receipt::{ReceiptAlgo, ReceiptPayload};
use trace_commons_operator_client::host_allowlist::HostAllowlist;

use super::attested::AttestedCall;

/// How long a fetch may take.
///
/// A remote call on the submission path. Short enough that a provider having
/// a bad minute costs an unattested submission rather than a stalled daemon.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// How much of a response body will be read.
///
/// A receipt is three short strings. Anything larger is not one, and reading
/// it would let a redirected or hostile endpoint spend this process's memory.
const MAX_RECEIPT_BYTES: usize = 16 * 1024;

/// How much of an attestation report response will be read.
///
/// A live report was measured at 284,003 bytes on 2026-09-05, so 1 MiB is a
/// bound on a much larger document than a receipt, not a guess.
const MAX_ATTESTATION_REPORT_BYTES: usize = 1 << 20;

/// Why no receipt was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptFetchError {
    /// The base URL is not one this module will call.
    #[error("the receipt endpoint is not an https URL")]
    EndpointNotHttps,
    /// The identifier is not a shape that can go in a path segment.
    #[error("the exchange identifier is not a usable path segment")]
    IdentifierMalformed,
    /// The endpoint's host is outside the allowlist this client enforces.
    #[error("the receipt endpoint is not an allowed host")]
    EndpointNotAllowed,
    /// This deployment configured no receipt endpoint, or the proxy recorded
    /// no served model to name on the query the endpoint requires.
    #[error("no receipt endpoint is configured for this call")]
    NotConfigured,
    /// The provider was unreachable, slow, or answered with an error status.
    #[error("the receipt endpoint did not answer")]
    Unreachable,
    /// The answer was larger than a receipt can be.
    #[error("the receipt response is larger than a receipt")]
    ResponseTooLarge,
    /// The answer was not a receipt this verifier can read.
    #[error("the receipt response is not a receipt")]
    ResponseMalformed,
    /// The receipt verified, but its signer is not the gateway key a
    /// nonced attestation report bound. Carries nothing.
    #[error("receipt signer is not the attested gateway key")]
    SignerNotAttested,
}

/// Read a receipt out of the endpoint's JSON answer.
///
/// Split out from the fetch so the shape contract is testable without a
/// network. Accepts the receipt at the document root or nested under
/// `"receipt"`: the live capture in
/// `crates/trace-commons-server/tests/near_ai_live_receipt.rs` nests it, and
/// a provider that stops nesting it should not silently stop being
/// attestable.
///
/// No field is normalised. `verify_receipt` is the only thing entitled to
/// judge these strings, and a "helpful" rewrite here -- trimming, lowercasing
/// an address, stripping an `0x` -- would change what gets verified.
///
/// # Errors
///
/// [`ReceiptFetchError::ResponseMalformed`] when any of the three fields is
/// missing or is not a string.
pub fn parse_receipt_response(body: &str) -> Result<ReceiptPayload, ReceiptFetchError> {
    let document: serde_json::Value =
        serde_json::from_str(body).map_err(|_| ReceiptFetchError::ResponseMalformed)?;
    let receipt = document.get("receipt").unwrap_or(&document);
    let field = |name: &str| {
        receipt
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or(ReceiptFetchError::ResponseMalformed)
    };
    let signing_algo = match receipt.get("signing_algo") {
        None => ReceiptAlgo::Ecdsa,
        Some(serde_json::Value::String(s)) => {
            ReceiptAlgo::from_wire(s).ok_or(ReceiptFetchError::ResponseMalformed)?
        }
        // Present but not a string: the provider said something this
        // client cannot read. That is a malformed response, not an absent
        // field, and it must not be read as the ECDSA default.
        Some(_) => return Err(ReceiptFetchError::ResponseMalformed),
    };
    Ok(ReceiptPayload {
        text: field("text")?,
        signature: field("signature")?,
        signing_address: field("signing_address")?,
        signing_algo,
    })
}

/// The URL a fetch would call.
///
/// Built rather than formatted, so a `chat_id` off another process's database
/// cannot inject a path segment or a query parameter. `url`'s own
/// percent-encoding does that; the shape check below refuses the cases where
/// escaping would produce a valid-looking but wrong request.
///
/// # Errors
///
/// [`ReceiptFetchError`] when the base is not https or the identifier is not
/// a usable segment.
pub fn receipt_url(base: &str, chat_id: &str, model: &str) -> Result<url::Url, ReceiptFetchError> {
    let base = url::Url::parse(base).map_err(|_| ReceiptFetchError::EndpointNotHttps)?;
    if base.scheme() != "https" {
        return Err(ReceiptFetchError::EndpointNotHttps);
    }
    if chat_id.is_empty()
        || !chat_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(ReceiptFetchError::IdentifierMalformed);
    }

    let mut url = base;
    url.path_segments_mut()
        .map_err(|_| ReceiptFetchError::EndpointNotHttps)?
        .pop_if_empty()
        .push("signature")
        .push(chat_id);
    url.query_pairs_mut()
        .append_pair("model", model)
        // The signer bound into the gateway's TDX quote. Pinned rather than
        // configurable: a receipt in another algorithm is one this client
        // cannot check against the attestation, and asking for one would
        // produce a signature that fails verification for a reason nobody
        // could read.
        .append_pair("signing_algo", ReceiptAlgo::Ed25519.as_wire());
    Ok(url)
}

/// Fetch the receipt for one exchange.
///
/// # Errors
///
/// [`ReceiptFetchError`] for every failure. A caller treats all of them as
/// "no receipt" and submits unattested.
pub async fn fetch_receipt(
    client: &reqwest::Client,
    allowlist: &HostAllowlist,
    base: &str,
    chat_id: &str,
    model: &str,
) -> Result<ReceiptPayload, ReceiptFetchError> {
    let url = receipt_url(base, chat_id, model)?;
    // Before the request, not after: the same gate the issuer, ingest and
    // witness URLs pass. An operator who narrowed this client's egress did
    // not thereby agree to a new third-party host, and the check belongs here
    // rather than at the call site so a second call site cannot omit it.
    allowlist
        .check(&url)
        .map_err(|_| ReceiptFetchError::EndpointNotAllowed)?;
    let response = client
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|_| ReceiptFetchError::Unreachable)?;
    if !response.status().is_success() {
        return Err(ReceiptFetchError::Unreachable);
    }
    // The declared length is a hint, not a bound -- a chunked response
    // declares none -- so the body is bounded again after reading.
    if response
        .content_length()
        .is_some_and(|declared| declared > MAX_RECEIPT_BYTES as u64)
    {
        return Err(ReceiptFetchError::ResponseTooLarge);
    }
    let body = response
        .text()
        .await
        .map_err(|_| ReceiptFetchError::Unreachable)?;
    if body.len() > MAX_RECEIPT_BYTES {
        return Err(ReceiptFetchError::ResponseTooLarge);
    }
    parse_receipt_response(&body)
}

/// GET the attestation report. Same gate and same bounds as
/// [`fetch_receipt`]: the allowlist is checked before the request so a
/// second call site cannot omit it, and the body is bounded after reading
/// because a chunked response declares no length. Returns the raw JSON;
/// parsing is `attestation_report::gateway_ed25519_key`'s job.
async fn fetch_attestation_report(
    client: &reqwest::Client,
    allowlist: &HostAllowlist,
    base: &str,
    model: &str,
    nonce: &str,
) -> Result<String, ReceiptFetchError> {
    let url = super::attestation_report::attestation_report_url(base, model, nonce)
        .map_err(|_| ReceiptFetchError::SignerNotAttested)?;
    allowlist
        .check(&url)
        .map_err(|_| ReceiptFetchError::EndpointNotAllowed)?;
    let response = client
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|_| ReceiptFetchError::Unreachable)?;
    if !response.status().is_success() {
        return Err(ReceiptFetchError::Unreachable);
    }
    if response
        .content_length()
        .is_some_and(|declared| declared > MAX_ATTESTATION_REPORT_BYTES as u64)
    {
        return Err(ReceiptFetchError::ResponseTooLarge);
    }
    let body = response
        .text()
        .await
        .map_err(|_| ReceiptFetchError::Unreachable)?;
    if body.len() > MAX_ATTESTATION_REPORT_BYTES {
        return Err(ReceiptFetchError::ResponseTooLarge);
    }
    Ok(body)
}

/// A fresh 32-byte nonce, lowercase hex, for one attestation-report fetch.
///
/// Freshly random per call so a report bound to it cannot be a replay of one
/// fetched for a previous submission.
fn fresh_nonce_hex() -> Result<String, ReceiptFetchError> {
    let mut bytes = [0u8; 32];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .map_err(|_| ReceiptFetchError::Unreachable)?;
    Ok(hex::encode(bytes))
}

/// The receipt for one attested call, or none.
///
/// The single entry point the submission path uses. Every failure -- no
/// configured endpoint, a proxy that recorded no served model, a host outside
/// the allowlist, a provider that timed out, an answer that is not a receipt
/// -- resolves to `Err`, and the caller submits unattested. Nothing here can
/// fail a submission, which is the rule the rest of `routing` runs under.
///
/// `endpoint` is a configured base URL rather than something read off the
/// ledger row: the proxy records no upstream URL, so a base derived from a
/// row would be one this client invented. A deployment routing through more
/// than one provider slug therefore has one base configured and the others
/// unattestable, which is a limit of the ledger's shape rather than a choice
/// made here.
///
/// `check_attestation` additionally fetches a freshly-nonced attestation
/// report and refuses the receipt if its signer is not the gateway key that
/// report binds. Off by default at the config level -- see
/// `ContributorConfig::inference_receipt_check_attestation` -- because it
/// costs a second network call and this module does not verify the report's
/// quote, only its internal consistency.
///
/// # Errors
///
/// [`ReceiptFetchError`] for every reason no receipt was obtained.
pub async fn receipt_for_attested_call(
    endpoint: Option<&str>,
    allowlist: &HostAllowlist,
    call: &AttestedCall,
    check_attestation: bool,
) -> Result<ReceiptPayload, ReceiptFetchError> {
    let endpoint = endpoint.ok_or(ReceiptFetchError::NotConfigured)?;
    // The endpoint requires a model and the proxy does not always record one.
    // Refused rather than guessed: a model this client chose is a parameter
    // the provider looks the receipt up by, and a wrong one produces "no such
    // receipt" rather than anything an operator could read.
    let model = call
        .served_model()
        .ok_or(ReceiptFetchError::NotConfigured)?;
    let client = reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|_| ReceiptFetchError::Unreachable)?;
    // One attempt, no retry. A retry loop on the submission path multiplies a
    // provider outage into a stalled uploader; the cost of not retrying is one
    // unattested submission.
    let payload = fetch_receipt(&client, allowlist, endpoint, call.upstream_id(), model).await?;

    if check_attestation {
        let nonce = fresh_nonce_hex()?;
        let report = fetch_attestation_report(&client, allowlist, endpoint, model, &nonce).await?;
        let attested = super::attestation_report::gateway_ed25519_key(&report, &nonce)
            .map_err(|_| ReceiptFetchError::SignerNotAttested)?;
        if !super::attestation_report::signer_is_attested(&payload.signing_address, &attested) {
            return Err(ReceiptFetchError::SignerNotAttested);
        }
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://qwen3-6-27b.completions.near.ai/v1";

    /// An attestable call, built through the real reader so its identifier
    /// and model are the ones a fetch would actually use.
    fn attestable_call(served_model: Option<&str>) -> (AttestedCall, tempfile::TempDir) {
        use sha2::Digest as _;

        const REQUEST: &str = "{\"model\":\"Qwen/Qwen3.6-27B-FP8\"}";
        const RESPONSE: &str = "data: [DONE]\n\n";

        let dir = tempfile::tempdir().expect("a temporary body store");
        let reference = "00000000000000000009-000000";
        std::fs::write(dir.path().join(format!("{reference}.req")), REQUEST).expect("req");
        std::fs::write(dir.path().join(format!("{reference}.res")), RESPONSE).expect("res");

        let row = crate::routing::RoutedExchange {
            id: Some(9),
            started_at: chrono::Utc::now(),
            client_session_id: Some("session".to_string()),
            total_ms: Some(10),
            facade: "openai".to_string(),
            backend: "nearai".to_string(),
            requested_model: Some("Qwen/Qwen3.6-27B-FP8".to_string()),
            served_model: served_model.map(str::to_string),
            upstream_id: Some("chatcmpl-abc123".to_string()),
            request_sha256: Some(hex::encode(sha2::Sha256::digest(REQUEST.as_bytes()))),
            response_sha256: Some(hex::encode(sha2::Sha256::digest(RESPONSE.as_bytes()))),
            body_ref: Some(reference.to_string()),
            rung: "full".to_string(),
            attempts: 1,
            input_tokens: Some(1),
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: Some(1),
            cost_usd: Some(0.0),
            status: 200,
        };
        let call = super::super::attested::attested_final_call(&[row], dir.path())
            .expect("the fixture must be attestable, or these tests prove nothing");
        (call, dir)
    }

    /// No configured endpoint is "no receipt", and it must not reach the
    /// network to find that out.
    #[tokio::test]
    async fn an_unconfigured_endpoint_fetches_nothing() {
        let (call, _dir) = attestable_call(Some("Qwen/Qwen3.6-27B-FP8"));
        assert_eq!(
            receipt_for_attested_call(None, &HostAllowlist::permissive(), &call, false)
                .await
                .unwrap_err(),
            ReceiptFetchError::NotConfigured
        );
    }

    /// A row with no served model cannot name the query parameter the
    /// endpoint requires, and a model this client invented would be looked up
    /// against and produce a receipt for nothing.
    #[tokio::test]
    async fn a_call_with_no_served_model_is_not_fetched_for() {
        let (call, _dir) = attestable_call(None);
        assert_eq!(
            receipt_for_attested_call(Some(BASE), &HostAllowlist::permissive(), &call, false)
                .await
                .unwrap_err(),
            ReceiptFetchError::NotConfigured
        );
    }

    /// The allowlist an operator set applies to this host too.
    ///
    /// The receipt endpoint is a third party, and an operator who narrowed
    /// this client's egress to their own issuer and ingest did not thereby
    /// admit a new one.
    #[tokio::test]
    async fn an_endpoint_outside_the_allowlist_is_refused() {
        let (call, _dir) = attestable_call(Some("Qwen/Qwen3.6-27B-FP8"));
        let allowlist = HostAllowlist::from_csv("issuer.example,ingest.example");
        assert_eq!(
            receipt_for_attested_call(Some(BASE), &allowlist, &call, false)
                .await
                .unwrap_err(),
            ReceiptFetchError::EndpointNotAllowed
        );
    }

    /// A plaintext endpoint is refused before the allowlist is consulted, so
    /// an operator who allowlisted a host has not thereby allowed http to it.
    #[tokio::test]
    async fn a_plaintext_endpoint_is_refused_even_when_allowlisted() {
        let (call, _dir) = attestable_call(Some("Qwen/Qwen3.6-27B-FP8"));
        assert_eq!(
            receipt_for_attested_call(
                Some("http://qwen3-6-27b.completions.near.ai/v1"),
                &HostAllowlist::permissive(),
                &call,
                false,
            )
            .await
            .unwrap_err(),
            ReceiptFetchError::EndpointNotHttps
        );
    }

    #[test]
    fn the_url_is_the_endpoint_the_provider_documents() {
        let url = receipt_url(BASE, "chatcmpl-abc123", "Qwen/Qwen3.6-27B-FP8").expect("url");
        assert_eq!(
            url.as_str(),
            "https://qwen3-6-27b.completions.near.ai/v1/signature/chatcmpl-abc123\
             ?model=Qwen%2FQwen3.6-27B-FP8&signing_algo=ed25519"
        );
    }

    /// The fetch asks for the scheme whose signer is bound into the gateway's
    /// TDX quote. The ECDSA signer appears in no attestation report.
    #[test]
    fn the_receipt_url_asks_for_ed25519() {
        let url = receipt_url(
            "https://cloud-api.near.ai/v1",
            "ee64b242d74f4c7eb59b05b046f33f7b",
            "Qwen/Qwen3.6-35B-A3B-FP8",
        )
        .unwrap();
        assert!(
            url.query().unwrap().contains("signing_algo=ed25519"),
            "{url}"
        );
        assert!(!url.query().unwrap().contains("ecdsa"), "{url}");
    }

    /// The response's own `signing_algo` is what the payload records, not
    /// what was asked for. A provider answering a different scheme than
    /// requested is a fact to carry, not to overwrite.
    #[test]
    fn the_parsed_receipt_carries_the_scheme_the_provider_answered() {
        let body = r#"{"text":"81e9887990592366b55ef758cad3b3a097e890871bedc023a51b2828ed237cc3:6f7091a0fbe5917a631c70805833760fe63ceea3493466e3230bd830816a3f2e","signature":"838765bd299514ec80084d50b7cef9357172ce2923dd35aa837beed0c6af04e684673e61db6c0d3ae8d69476b680d94c8e1e36e05277a1b103c27a12f563eb0c","signing_address":"cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6","signing_algo":"ed25519","signature_kind":"gateway"}"#;
        let payload = parse_receipt_response(body).unwrap();
        assert_eq!(payload.signing_algo, ReceiptAlgo::Ed25519);
        assert_eq!(
            payload.signing_address,
            "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6"
        );
    }

    /// A response with no `signing_algo` is ECDSA -- the pre-field form.
    #[test]
    fn a_response_without_signing_algo_is_ecdsa() {
        let body = r#"{"text":"a:b","signature":"0xcc","signing_address":"0xdd"}"#;
        assert_eq!(
            parse_receipt_response(body).unwrap().signing_algo,
            ReceiptAlgo::Ecdsa
        );
    }

    /// An unrecognised `signing_algo` is a malformed response, not a guess.
    #[test]
    fn an_unknown_signing_algo_in_the_response_is_malformed() {
        let body =
            r#"{"text":"a:b","signature":"0xcc","signing_address":"0xdd","signing_algo":"rsa"}"#;
        assert_eq!(
            parse_receipt_response(body).unwrap_err(),
            ReceiptFetchError::ResponseMalformed
        );
    }

    /// A present-but-unreadable `signing_algo` is malformed, not absent. The
    /// absent arm is for the pre-field response shape only; a provider that
    /// sends the field as a number, null, or a list has said something this
    /// client cannot read, and reading it as ECDSA would be a guess.
    #[test]
    fn a_non_string_signing_algo_is_malformed_not_absent() {
        for body in [
            r#"{"text":"a:b","signature":"0xcc","signing_address":"0xdd","signing_algo":7}"#,
            r#"{"text":"a:b","signature":"0xcc","signing_address":"0xdd","signing_algo":null}"#,
            r#"{"text":"a:b","signature":"0xcc","signing_address":"0xdd","signing_algo":["ed25519"]}"#,
        ] {
            assert_eq!(
                parse_receipt_response(body).unwrap_err(),
                ReceiptFetchError::ResponseMalformed,
                "{body}"
            );
        }
    }

    /// The identifier comes off another process's database. A `..` or a `?`
    /// in it must not become a different request.
    #[test]
    fn an_identifier_that_is_not_a_segment_is_refused() {
        for hostile in ["../../admin", "abc?model=other", "abc/def", "abc#frag", ""] {
            assert_eq!(
                receipt_url(BASE, hostile, "m").unwrap_err(),
                ReceiptFetchError::IdentifierMalformed,
                "{hostile} must not reach the endpoint"
            );
        }
    }

    #[test]
    fn a_plaintext_endpoint_is_refused() {
        assert_eq!(
            receipt_url("http://near.ai/v1", "abc", "m").unwrap_err(),
            ReceiptFetchError::EndpointNotHttps
        );
    }

    /// The three fields come back exactly as the provider wrote them.
    /// Normalising any of them would change what `verify_receipt` checks.
    #[test]
    fn a_receipt_is_read_verbatim() {
        let body = r#"{"receipt":{"text":"AbCd0123:EfGh4567","signature":"0xDEADbeef","signing_address":"0xAbCdEf0123456789aBcDeF0123456789AbCdEf01"}}"#;
        let receipt = parse_receipt_response(body).expect("a receipt");
        assert_eq!(receipt.text, "AbCd0123:EfGh4567");
        assert_eq!(receipt.signature, "0xDEADbeef");
        assert_eq!(
            receipt.signing_address,
            "0xAbCdEf0123456789aBcDeF0123456789AbCdEf01"
        );
    }

    #[test]
    fn an_unnested_receipt_reads_the_same() {
        let body = r#"{"text":"a:b","signature":"0x01","signing_address":"0x02"}"#;
        assert_eq!(parse_receipt_response(body).expect("a receipt").text, "a:b");
    }

    /// A partial answer is not a receipt. Accepting one would hand
    /// `verify_receipt` an empty signature and turn a provider outage into an
    /// unverifiable-receipt refusal, which reads as tampering.
    #[test]
    fn a_partial_answer_is_not_a_receipt() {
        for body in [
            r#"{"receipt":{"text":"a:b","signature":"0x01"}}"#,
            r#"{"receipt":{"text":"a:b","signature":"0x01","signing_address":null}}"#,
            r#"{"receipt":{"text":1,"signature":"0x01","signing_address":"0x02"}}"#,
            "not json",
        ] {
            assert_eq!(
                parse_receipt_response(body).unwrap_err(),
                ReceiptFetchError::ResponseMalformed,
                "{body} must not parse as a receipt"
            );
        }
    }
}
