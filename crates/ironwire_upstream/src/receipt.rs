//! Fetching a provider's receipt for one exchange.
//!
//! NEAR AI serves `GET {base}/signature/{id}?model=..&signing_algo=ed25519`: a
//! short `text` carrying the SHA-256 of the request and response bodies, and a
//! signature over it. This module only *fetches* and *parses* that document.
//! Whether it proves anything is decided elsewhere (`ironwire_proxy::proof`),
//! and nothing here claims it does.
//!
//! Ed25519 is asked for because that is the signer an attestation report
//! binds; the ECDSA signer appears in no ed25519 attestation.
//!
//! Nothing here is logged: the id, the model and every receipt field are the
//! caller's data. [`ReceiptFetch`] carries no payload beyond the receipt itself.

use serde::Deserialize;

/// How much of a response will be read. A receipt is four short strings;
/// anything larger is not one, and reading it would let a misbehaving endpoint
/// spend this process's memory.
pub const MAX_RECEIPT_BYTES: usize = 16 * 1024;

/// How long one fetch may take. Off the response path, so this bounds how long
/// a background check holds a concurrency slot, not anybody's request.
pub const FETCH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// A receipt as the provider returned it. Unverified.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct RawReceipt {
    /// The signed text: `<model>:<request sha256>:<response sha256>`, or the
    /// two-part form without a model.
    pub text: String,
    /// The signature over `text`, hex.
    pub signature: String,
    /// For ed25519, the 32-byte public key, hex.
    pub signing_address: String,
    /// `ed25519` or `ecdsa`.
    pub signing_algo: String,
    /// `provider_tee` or `gateway`. Absent on older responses, which is read
    /// as neither.
    #[serde(default)]
    pub signature_kind: Option<String>,
}

/// What a receipt fetch came back with.
///
/// Deliberately exhaustive: a new outcome must make every consumer decide
/// what it means for proof, not fall into a wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiptFetch {
    /// This backend issues no receipts at all.
    NotOffered,
    /// The endpoint answered 404: it holds no receipt for this id. The
    /// permanent answer for a model NEAR AI brokers rather than hosts.
    NotFound,
    /// Nothing usable this time -- no connection, a timeout, a 5xx, a rate
    /// limit, a refused credential. Worth retrying.
    Unavailable,
    /// The endpoint answered with something that is not a receipt.
    Malformed,
    /// A receipt. Not yet verified.
    Receipt(RawReceipt),
}

/// Whether `id` can go into a URL path segment unescaped.
///
/// Provider ids are hex, or a short prefix and hex. Anything else is refused
/// rather than escaped: an id containing `/` or `..` would address a different
/// endpoint on the same host with the user's credential attached.
#[must_use]
pub fn usable_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Classify a response and parse its body.
pub(crate) async fn read(response: reqwest::Response) -> ReceiptFetch {
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return ReceiptFetch::NotFound;
    }
    if !status.is_success() {
        return ReceiptFetch::Unavailable;
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_RECEIPT_BYTES as u64)
    {
        return ReceiptFetch::Malformed;
    }
    let mut body = Vec::new();
    let mut response = response;
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > MAX_RECEIPT_BYTES {
                    return ReceiptFetch::Malformed;
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => return ReceiptFetch::Unavailable,
        }
    }
    parse(&body)
}

/// Parse a receipt document.
#[must_use]
pub fn parse(body: &[u8]) -> ReceiptFetch {
    match serde_json::from_slice::<RawReceipt>(body) {
        Ok(receipt) => ReceiptFetch::Receipt(receipt),
        Err(_) => ReceiptFetch::Malformed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_escape_is_not_a_usable_id() {
        for id in [
            "",
            "../attestation/report",
            "a/b",
            "a?b",
            "a%2fb",
            &"a".repeat(129),
        ] {
            assert!(!usable_id(id), "{id:?}");
        }
        for id in [
            "c54961ab1d594cf591e5566caa21196b",
            "chatcmpl-abc123",
            "resp_32464c3bb3064e1ba888d5e5f7073fb3",
        ] {
            assert!(usable_id(id), "{id:?}");
        }
    }

    #[test]
    fn a_receipt_parses_and_anything_else_is_malformed() {
        let body = br#"{"text":"m:aa:bb","signature":"00","signing_address":"11",
                        "signing_algo":"ed25519","signature_kind":"provider_tee"}"#;
        match parse(body) {
            ReceiptFetch::Receipt(r) => {
                assert_eq!(r.signature_kind.as_deref(), Some("provider_tee"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(parse(b"{}"), ReceiptFetch::Malformed);
        assert_eq!(parse(b"<html>"), ReceiptFetch::Malformed);
    }
}
