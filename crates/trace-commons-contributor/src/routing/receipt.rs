//! Fetching the provider's receipt for one inference call.
//!
//! `GET {base}/signature/{chat_id}?model={model}&signing_algo=ecdsa` returns
//! the enclave's EIP-191 signature over `<requestHash>:<responseHash>`. This
//! module is the only thing in the tree that calls it.
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

use trace_commons_attestation::receipt::ReceiptPayload;

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

/// Why no receipt was obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptFetchError {
    /// The base URL is not one this module will call.
    #[error("the receipt endpoint is not an https URL")]
    EndpointNotHttps,
    /// The identifier is not a shape that can go in a path segment.
    #[error("the exchange identifier is not a usable path segment")]
    IdentifierMalformed,
    /// The provider was unreachable, slow, or answered with an error status.
    #[error("the receipt endpoint did not answer")]
    Unreachable,
    /// The answer was larger than a receipt can be.
    #[error("the receipt response is larger than a receipt")]
    ResponseTooLarge,
    /// The answer was not a receipt this verifier can read.
    #[error("the receipt response is not a receipt")]
    ResponseMalformed,
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
    Ok(ReceiptPayload {
        text: field("text")?,
        signature: field("signature")?,
        signing_address: field("signing_address")?,
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
        // ECDSA over secp256k1, which is what `verify_receipt` recovers from.
        // Pinned rather than configurable: a receipt in another algorithm is
        // one this client cannot check, and asking for one would produce a
        // signature that fails verification for a reason nobody could read.
        .append_pair("signing_algo", "ecdsa");
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
    base: &str,
    chat_id: &str,
    model: &str,
) -> Result<ReceiptPayload, ReceiptFetchError> {
    let url = receipt_url(base, chat_id, model)?;
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

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://qwen3-6-27b.completions.near.ai/v1";

    #[test]
    fn the_url_is_the_endpoint_the_provider_documents() {
        let url = receipt_url(BASE, "chatcmpl-abc123", "Qwen/Qwen3.6-27B-FP8").expect("url");
        assert_eq!(
            url.as_str(),
            "https://qwen3-6-27b.completions.near.ai/v1/signature/chatcmpl-abc123\
             ?model=Qwen%2FQwen3.6-27B-FP8&signing_algo=ecdsa"
        );
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
