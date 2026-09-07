//! Fetching the provider's receipt for one inference call.
//!
//! `GET {base}/signature/{chat_id}?model={model}&signing_algo=ed25519` returns
//! the enclave's EIP-191 signature over `<requestHash>:<responseHash>`. This
//! module is the only thing in the tree that calls it. Ed25519, because that
//! signer is the one an attestation report binds; the ECDSA signer appears in
//! no ed25519 attestation.
//!
//! **Which** ed25519 key depends on the protocol the inference call used, for
//! the same hosted model: a Chat Completions call yields a `provider_tee`
//! receipt signed by the model's own `model_attestations` key, a Responses
//! API call a `gateway` receipt signed by the report's `gateway_attestation`
//! key. Codex speaks only the Responses API, so the second is not an edge
//! case. The receipt's own `signature_kind` routes it; see
//! [`signer_matches_report`].
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
//! # The model is a query parameter here, and query parameters are not signed
//!
//! The endpoint requires `model`, and the value this module sends is chosen by
//! whoever fetches: it is not covered by any signature and establishes
//! nothing on its own. It is sent because the endpoint demands it.
//!
//! The **receipt** is a different matter. A hosted model answers with the
//! three-part form, `{model}:{requestHash}:{responseHash}`, whose leading part
//! *is* signed -- so the receipt names its own model, and that is the name the
//! attestation check below looks a signer set up by.
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

use trace_commons_attestation::receipt::{
    ReceiptAlgo, ReceiptPayload, ReceiptSignatureKind, ReceiptVerdict, verify_receipt,
};
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
    /// The provider could not be reached at all: no connection, a timeout, a
    /// body that stopped arriving mid-read, or this process failing to build
    /// the client or a nonce for the call. Nothing was answered, so this says
    /// nothing whatever about the call the receipt would have been for.
    ///
    /// Deliberately *not* the home of an error status any more. An endpoint
    /// that answered told us something; see [`Self::ReceiptNotFound`].
    #[error("the receipt endpoint did not answer")]
    Unreachable,
    /// The endpoint answered 404: it holds no receipt for this call.
    ///
    /// Three unrelated situations produce this, and none of them is a
    /// transport failure, which is why it may not share a value with one:
    ///
    /// - a **brokered** model, where receipts do not exist at all and 404 is
    ///   the permanent, expected answer -- the discrimination this client
    ///   already relies on;
    /// - a **hosted** call the provider never wrote a record for. Once
    ///   `nearai/cloud-api` #943 lands, Responses-API attestation is
    ///   best-effort and an interrupted stream creates no `resp_*` record and
    ///   no legacy disconnect fallback, so a call that should have carried a
    ///   receipt answers 404 like a brokered one;
    /// - an identifier the provider has expired or never had.
    ///
    /// This variant does not tell those three apart -- nothing on this
    /// machine can, from a 404 alone. It separates them from the endpoint
    /// being unreachable, which is the distinction that was being lost.
    #[error("the receipt endpoint has no receipt for this call")]
    ReceiptNotFound,
    /// The endpoint refused this client's credentials (401 or 403).
    ///
    /// Absence to the submission, like every other variant here, but the only
    /// one an operator fixes by fixing this machine's configuration rather
    /// than by looking at the provider or the call.
    #[error("the receipt endpoint refused this client's credentials")]
    ReceiptEndpointUnauthorized,
    /// The endpoint answered with some other non-success status: a 5xx, a
    /// rate limit, anything else. Reachable and not refusing us, but not
    /// serving a receipt on this attempt.
    #[error("the receipt endpoint answered with an unexpected status")]
    ReceiptEndpointUnexpectedStatus,
    /// The attestation-report endpoint answered 404 for this model. Only
    /// reachable with `check_attestation` on.
    ///
    /// Named apart from [`Self::ReceiptNotFound`] because the two send an
    /// operator to different places: a receipt exists and verified by the
    /// time this can happen, and what is missing is the report that would say
    /// whether its signer is attested.
    #[error("no attestation report was served for this model")]
    AttestationReportNotFound,
    /// The attestation-report endpoint refused this client's credentials.
    #[error("the attestation report endpoint refused this client's credentials")]
    AttestationReportUnauthorized,
    /// The attestation-report endpoint answered with some other non-success
    /// status.
    #[error("the attestation report endpoint answered with an unexpected status")]
    AttestationReportUnexpectedStatus,
    /// The answer was larger than a receipt can be.
    #[error("the receipt response is larger than a receipt")]
    ResponseTooLarge,
    /// The answer was not a receipt this verifier can read.
    #[error("the receipt response is not a receipt")]
    ResponseMalformed,
    /// The receipt verified, but its signer is not one of the keys a nonced
    /// attestation report bound for the model the receipt names. Carries
    /// nothing.
    #[error("receipt signer is not an attested key for this model")]
    SignerNotAttested,
    /// The receipt did not verify against the bytes of the call it is for:
    /// a bad signature, a hash over other bytes, or a bound model that is not
    /// the one that served.
    ///
    /// Distinct from [`Self::SignerNotAttested`] because the two send an
    /// operator somewhere different -- a receipt that does not verify at all
    /// points at the endpoint that served it, and one that verifies under an
    /// unattested key points at the attestation. Neither carries a payload;
    /// the underlying `ReceiptError` is caller data.
    #[error("the receipt does not verify against this call")]
    ReceiptUnverified,
}

impl ReceiptFetchError {
    /// The fixed label for this outcome.
    ///
    /// The only part of this type that reaches a log line. Every arm is a
    /// literal: no identifier, URL, status code or response body is
    /// interpolated, so the set of strings this can ever emit is the set
    /// written here and cannot grow with caller data.
    ///
    /// `SignerNotAttested` keeps the exact string `submit.rs` has always
    /// logged; a narrower enum is no reason to rename a label someone may
    /// already be grepping for.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::EndpointNotHttps => "receipt_endpoint_not_https",
            Self::IdentifierMalformed => "receipt_identifier_malformed",
            Self::EndpointNotAllowed => "receipt_endpoint_not_allowed",
            Self::NotConfigured => "receipt_not_configured",
            Self::Unreachable => "receipt_endpoint_unreachable",
            Self::ReceiptNotFound => "receipt_not_found",
            Self::ReceiptEndpointUnauthorized => "receipt_endpoint_unauthorized",
            Self::ReceiptEndpointUnexpectedStatus => "receipt_endpoint_unexpected_status",
            Self::AttestationReportNotFound => "attestation_report_not_found",
            Self::AttestationReportUnauthorized => "attestation_report_unauthorized",
            Self::AttestationReportUnexpectedStatus => "attestation_report_unexpected_status",
            Self::ResponseTooLarge => "receipt_response_too_large",
            Self::ResponseMalformed => "receipt_response_malformed",
            Self::SignerNotAttested => "receipt_signer_not_attested",
            Self::ReceiptUnverified => "receipt_unverified",
        }
    }
}

/// Which outcome a non-success status from the *receipt* endpoint is.
///
/// Split out from the fetch for the same reason `parse_receipt_response` is:
/// the receipt endpoint is https-only and allowlisted, so there is no way to
/// stand a test server in front of it, and the mapping would otherwise be
/// reachable only from a live provider. A pure function over a status is the
/// whole of the decision and is testable on its own.
///
/// Only the two classes worth acting on are named. Everything else is one
/// value on purpose: a client that cannot do anything different for a 500
/// than for a 429 should not pretend it can.
fn receipt_status_error(status: reqwest::StatusCode) -> ReceiptFetchError {
    match status {
        reqwest::StatusCode::NOT_FOUND => ReceiptFetchError::ReceiptNotFound,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            ReceiptFetchError::ReceiptEndpointUnauthorized
        }
        _ => ReceiptFetchError::ReceiptEndpointUnexpectedStatus,
    }
}

/// The same three-way split for the *attestation report* endpoint, under its
/// own names. The two resources fail independently -- by the time a report is
/// fetched a receipt has already arrived and verified -- so sharing one set
/// of labels would report a missing receipt when what is missing is a report.
fn attestation_report_status_error(status: reqwest::StatusCode) -> ReceiptFetchError {
    match status {
        reqwest::StatusCode::NOT_FOUND => ReceiptFetchError::AttestationReportNotFound,
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            ReceiptFetchError::AttestationReportUnauthorized
        }
        _ => ReceiptFetchError::AttestationReportUnexpectedStatus,
    }
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
    // Which attested key the provider says signed it. Absent reads as
    // `Unrecognised` rather than as a guess: a receipt that names no kind
    // names no key source, and the routing below refuses it. Present but not
    // a string is malformed, for the same reason `signing_algo` is -- the
    // provider said something this client cannot read.
    //
    // An unrecognised *string* is not malformed, though. It is a kind this
    // client has not learned yet, and the honest outcome is an unattested
    // submission rather than a parse failure that reads as a broken endpoint.
    let signature_kind = match receipt.get("signature_kind") {
        None => ReceiptSignatureKind::Unrecognised,
        Some(serde_json::Value::String(s)) => ReceiptSignatureKind::from_wire(s),
        Some(_) => return Err(ReceiptFetchError::ResponseMalformed),
    };
    Ok(ReceiptPayload {
        text: field("text")?,
        signature: field("signature")?,
        signing_address: field("signing_address")?,
        signing_algo,
        signature_kind,
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
        // An answer, not a failure to get one. Which answer decides whether
        // this call could ever have had a receipt, whether this machine's
        // credentials are wrong, or whether the provider is simply having a
        // bad minute -- see `receipt_status_error`.
        return Err(receipt_status_error(response.status()));
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
/// parsing is `attestation_report::model_ed25519_keys`'s job.
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
        return Err(attestation_report_status_error(response.status()));
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
/// The parse-and-compare half of the attestation check, with no network in
/// it, so it can be tested without a stub. [`receipt_for_attested_call`]
/// does the fetch and hands the body here.
///
/// Verify a receipt against the bytes of the call it claims to be for.
///
/// The other network-free half of the attestation gate, split out for the same
/// reason [`signer_matches_report`] is: it can be tested without a stub.
///
/// This is what stops the gate from being a comparison of two strings. Without
/// it the check reads the `signing_address` the endpoint *claimed* and asks
/// whether that address is attested -- which a hostile or compromised receipt
/// endpoint satisfies by returning a genuinely attested address over a
/// signature that verifies against nothing. The witness would still refuse
/// such a submission, so the gap was fail-closed in direction, but a client
/// that reports "attested" on an unverifiable receipt is telling the
/// contributor something untrue.
///
/// Returns the verdict rather than `()` so the caller compares the **verified**
/// signer against the attested set instead of the claimed one.
///
/// # Errors
///
/// [`ReceiptFetchError::ReceiptUnverified`] when the signature does not
/// verify, either digest is over other bytes, or the receipt binds a model
/// that is not the one that served.
pub fn receipt_matches_call(
    payload: &ReceiptPayload,
    call: &AttestedCall,
    model: &str,
) -> Result<trace_commons_attestation::receipt::ReceiptVerdict, ReceiptFetchError> {
    verify_receipt(
        payload,
        call.request_body().as_bytes(),
        call.response_body().as_bytes(),
        model,
    )
    .map_err(|_| ReceiptFetchError::ReceiptUnverified)
}

/// The signer is checked against the key set the receipt's **own
/// `signature_kind`** names, and against no other.
///
/// NEAR AI signs with a different attested key depending on the protocol the
/// inference call used, for the same hosted model: a Chat Completions call
/// yields `provider_tee`, signed by that model's key in `model_attestations`;
/// a Responses API call yields `gateway`, signed by the key in
/// `gateway_attestation`. The Codex CLI speaks the Responses API exclusively,
/// so a client that only ever consults `model_attestations` reports every
/// Codex-driven receipt as unattested.
///
/// One report answers both -- the same fetch returns `gateway_attestation`
/// and `model_attestations` -- so routing costs no second request.
///
/// The kind selects **one** source. A `gateway` receipt is never tried
/// against a model key, nor the reverse, and a kind this client does not
/// recognise selects nothing and is refused; see
/// [`trace_commons_attestation::receipt::attested_keys_for_receipt`].
///
/// `requested_model` is a fallback for the `provider_tee` arm only, and it is
/// only ever reached by a two-part receipt: `verify_receipt` has already
/// established that a three-part receipt's bound model *is* the requested one,
/// so on every receipt that binds a model the two values are equal and the
/// receipt's own name is what is used.
///
/// # Errors
///
/// [`ReceiptFetchError::SignerNotAttested`] when the receipt names no
/// recognised kind, the report cannot be parsed, attests no ed25519 signer for
/// the selected source, is for a different nonce, or names a different signer.
pub fn signer_matches_report(
    verdict: &ReceiptVerdict,
    report_json: &str,
    nonce: &str,
    requested_model: &str,
) -> Result<(), ReceiptFetchError> {
    let model = verdict.model.as_deref().unwrap_or(requested_model);
    let attested = trace_commons_attestation::receipt::attested_keys_for_receipt(
        verdict.signature_kind,
        report_json,
        nonce,
        Some(model),
    )
    .map_err(|_| ReceiptFetchError::SignerNotAttested)?;
    if !super::attestation_report::signer_is_attested_for_model(&verdict.signing_address, &attested)
    {
        return Err(ReceiptFetchError::SignerNotAttested);
    }
    Ok(())
}

/// `check_attestation` additionally fetches a freshly-nonced, ed25519
/// attestation report for this model and refuses the receipt if its signer is
/// not one of the keys that report attests **for that model**. Off by default
/// at the config level -- see
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
    let client = receipt_http_client()?;
    // One attempt, no retry. A retry loop on the submission path multiplies a
    // provider outage into a stalled uploader; the cost of not retrying is one
    // unattested submission.
    let payload = fetch_receipt(&client, allowlist, endpoint, call.upstream_id(), model).await?;

    if check_attestation {
        // Verify the receipt before asking whether its signer is attested.
        //
        // Without this the gate checks only that the address the endpoint
        // *claimed* is attested for the model we asked for -- so a hostile or
        // compromised receipt endpoint could hand back a genuinely attested
        // address over a bogus signature and pass. The witness would still
        // catch it, so the old shape was fail-closed in direction, but a
        // client-side gate that reports "attested" on an unverifiable receipt
        // is telling the contributor something untrue.
        //
        // `verify_receipt` over the call's own bytes is what closes it: it
        // checks the signature, both body digests, and -- on the three-part
        // form a hosted model returns -- that the model the receipt *binds*
        // is the model that served. That last one matters here specifically,
        // because the model is the key the attested set is looked up by, and
        // it must be the receipt's own rather than the one we typed.
        let verdict = receipt_matches_call(&payload, call, model)?;

        let nonce = fresh_nonce_hex()?;
        let report = fetch_attestation_report(&client, allowlist, endpoint, model, &nonce).await?;
        // The whole verdict, not just the signer: the comparison needs the
        // *verified* signer (equal to the claimed one by the time this runs,
        // and taking it from the verdict is what keeps that true if the two
        // ever diverge), the model the receipt itself bound, and the kind
        // that decides which attested key set it is checked against.
        signer_matches_report(&verdict, &report, &nonce, model)?;
    }

    Ok(payload)
}

fn receipt_http_client() -> Result<reqwest::Client, ReceiptFetchError> {
    reqwest::Client::builder()
        // The configured provider cannot delegate this call's identifier.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|_| ReceiptFetchError::Unreachable)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://qwen3-6-27b.completions.near.ai/v1";

    /// A verdict as `verify_receipt` would have produced it, for the tests
    /// that exercise the attestation comparison on its own.
    ///
    /// The hashes are not read by `signer_matches_report` -- they were
    /// checked before it runs -- so they are placeholders here and the three
    /// fields that matter are real: the signer, the model the receipt bound,
    /// and the kind that selects which attested key set it is checked
    /// against.
    fn verdict(kind: ReceiptSignatureKind, signer: &str, model: Option<&str>) -> ReceiptVerdict {
        ReceiptVerdict {
            request_sha256: "0".repeat(64),
            response_sha256: "0".repeat(64),
            signing_address: signer.to_string(),
            model: model.map(str::to_string),
            signing_algo: ReceiptAlgo::Ed25519,
            signature_kind: kind,
        }
    }

    /// The common case in these tests: a chat-completions receipt, which
    /// binds its model and is checked against that model's attested keys.
    fn provider_verdict(signer: &str, model: &str) -> ReceiptVerdict {
        verdict(ReceiptSignatureKind::ProviderTee, signer, Some(model))
    }

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

    /// The fetch asks for the scheme whose signer is bound into a TDX quote
    /// -- the gateway's or the serving model's, per the receipt's
    /// `signature_kind`. The ECDSA signer has no such binding: `signing_algo`
    /// defaults to ECDSA on the report endpoint, but the ECDSA attestations
    /// it returns list the model enclave's address rather than the signer an
    /// ECDSA receipt recovers to.
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
    #[tokio::test]
    async fn receipt_transport_never_follows_provider_redirects() {
        use axum::{Router, routing::get};
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let target = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let target_url = format!("http://{}/redirected", target.local_addr().unwrap());
        let target_router = Router::new().route(
            "/redirected",
            get(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    "unexpected"
                }
            }),
        );
        let target_task = tokio::spawn(async move {
            axum::serve(target, target_router).await.unwrap();
        });
        let origin = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_url = format!("http://{}/signature/test", origin.local_addr().unwrap());
        let origin_router = Router::new().route(
            "/signature/test",
            get(move || {
                let target_url = target_url.clone();
                async move {
                    (
                        axum::http::StatusCode::FOUND,
                        [(axum::http::header::LOCATION, target_url)],
                    )
                }
            }),
        );
        let origin_task = tokio::spawn(async move {
            axum::serve(origin, origin_router).await.unwrap();
        });
        // Exercise the production client's redirect policy in isolation; the
        // higher-level receipt URL gate continues requiring HTTPS in production.
        let response = receipt_http_client()
            .unwrap()
            .get(origin_url)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
        assert_eq!(hits.load(Ordering::SeqCst), 0);
        origin_task.abort();
        target_task.abort();
    }

    /// Live values, captured 2026-09-06. `GATEWAY_KEY` is the key this check
    /// used to compare against; `MODEL_A_KEY` and `MODEL_B_KEY` are the
    /// `provider_tee` keys that actually sign those two models' receipts.
    /// They are all different, which is the defect in one line.
    const ATTESTATION_NONCE: &str =
        "482934fb749d13aa81b2e543a253cf4d8cc847dab55a8d49989effd5023ddb5d";
    const GATEWAY_KEY: &str = "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6";
    const MODEL_A: &str = "Qwen/Qwen3.6-35B-A3B-FP8";
    const MODEL_A_KEY: &str = "aba45f0b8f90869baab26db02e8b01354bb8f8730769c60650cb7a635da602d4";
    const MODEL_B: &str = "Qwen/Qwen3.8-27B";
    const MODEL_B_KEY: &str = "73cf225ab4f09154ad8b299d4ac89425c7f25468a42ba9a87d09fcd4e87b8bf5";

    /// The report shape `signing_algo=ed25519` returns, reduced to the fields
    /// read. `report_data` is `signing_address || request_nonce` in every
    /// attestation object, gateway and model alike.
    fn attestation_report(nonce: &str) -> String {
        let entry = |model: &str, key: &str| {
            format!(
                r#"{{"model_name":"{model}","signing_address":"{key}","signing_algo":"ed25519","request_nonce":"{nonce}","report_data":"{key}{nonce}"}}"#
            )
        };
        format!(
            r#"{{"gateway_attestation":{{"signing_address":"{GATEWAY_KEY}","signing_algo":"ed25519","request_nonce":"{nonce}","report_data":"{GATEWAY_KEY}{nonce}"}},"model_attestations":[{},{}]}}"#,
            entry(MODEL_A, MODEL_A_KEY),
            entry(MODEL_B, MODEL_B_KEY)
        )
    }

    /// An attested signer over a signature that verifies against nothing.
    ///
    /// This is what a hostile or compromised receipt endpoint returns to walk
    /// through a gate that only compares the *claimed* `signing_address`
    /// against the attested set. `receipt_matches_call` is what refuses it,
    /// and it refuses before any report is fetched.
    #[test]
    fn an_attested_address_over_a_bogus_signature_is_refused() {
        let model = "Qwen/Qwen3.6-27B-FP8";
        let (call, _dir) = attestable_call(Some(model));

        let forged = ReceiptPayload {
            text: format!(
                "{model}:{}:{}",
                hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
                    call.request_body().as_bytes()
                )),
                hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
                    call.response_body().as_bytes()
                )),
            ),
            // 64 bytes of hex that is a well-formed signature and signs
            // nothing.
            signature: "11".repeat(64),
            // A real, genuinely attested per-model key.
            signing_address: MODEL_A_KEY.to_string(),
            signing_algo: ReceiptAlgo::Ed25519,
            signature_kind: ReceiptSignatureKind::ProviderTee,
        };

        assert_eq!(
            receipt_matches_call(&forged, &call, model).unwrap_err(),
            ReceiptFetchError::ReceiptUnverified,
            "an attested address does not make an unverifiable receipt verified"
        );

        // And the weaker check it replaces would have passed it: the claimed
        // address really is attested for this model.
        let report = attestation_report(ATTESTATION_NONCE);
        assert!(
            signer_matches_report(
                &provider_verdict(&forged.signing_address, MODEL_A),
                &report,
                ATTESTATION_NONCE,
                MODEL_A
            )
            .is_ok(),
            "the address alone passes the attestation comparison, which is \
             exactly why the signature has to be verified too"
        );
    }

    /// The composed check `receipt_for_attested_call` runs when the
    /// attestation gate is on, tested without a network: the signer of a real
    /// hosted-model receipt against the model attestation for that model.
    #[test]
    fn a_matching_model_signer_over_a_good_report_is_accepted() {
        assert!(
            signer_matches_report(
                &provider_verdict(MODEL_A_KEY, MODEL_A),
                &attestation_report(ATTESTATION_NONCE),
                ATTESTATION_NONCE,
                MODEL_A,
            )
            .is_ok()
        );
    }

    /// The defect. The gateway key signs no receipt, so a check that compares
    /// a signer against it refuses every real one -- and, read the other way,
    /// a signer that *is* the gateway key is not an attested receipt signer
    /// for any model. Reverting `signer_matches_report` to the gateway
    /// comparison turns the test above red and this one red too.
    #[test]
    fn the_gateway_key_is_not_a_receipt_signer_for_any_model() {
        let report = attestation_report(ATTESTATION_NONCE);
        assert_eq!(
            signer_matches_report(
                &provider_verdict(GATEWAY_KEY, MODEL_A),
                &report,
                ATTESTATION_NONCE,
                MODEL_A
            )
            .unwrap_err(),
            ReceiptFetchError::SignerNotAttested
        );
    }

    /// Per-model selection: model B's key is not accepted for model A. One
    /// attested key for the whole provider cannot express this.
    #[test]
    fn another_models_signer_is_refused() {
        let report = attestation_report(ATTESTATION_NONCE);
        assert_eq!(
            signer_matches_report(
                &provider_verdict(MODEL_B_KEY, MODEL_A),
                &report,
                ATTESTATION_NONCE,
                MODEL_A
            )
            .unwrap_err(),
            ReceiptFetchError::SignerNotAttested
        );
        assert!(
            signer_matches_report(
                &provider_verdict(MODEL_B_KEY, MODEL_B),
                &report,
                ATTESTATION_NONCE,
                MODEL_B
            )
            .is_ok(),
            "the same key is attested for its own model"
        );
    }

    /// A model the report attests nothing for is refused rather than checked
    /// against whatever else the report carried.
    #[test]
    fn a_model_the_report_does_not_attest_is_refused() {
        assert_eq!(
            signer_matches_report(
                &provider_verdict(MODEL_A_KEY, "Qwen/Qwen3.9-Nonexistent"),
                &attestation_report(ATTESTATION_NONCE),
                ATTESTATION_NONCE,
                "Qwen/Qwen3.9-Nonexistent",
            )
            .unwrap_err(),
            ReceiptFetchError::SignerNotAttested
        );
    }

    #[test]
    fn a_different_signer_over_a_good_report_is_refused() {
        assert_eq!(
            signer_matches_report(
                &provider_verdict(
                    "0000000000000000000000000000000000000000000000000000000000000000",
                    MODEL_A,
                ),
                &attestation_report(ATTESTATION_NONCE),
                ATTESTATION_NONCE,
                MODEL_A,
            )
            .unwrap_err(),
            ReceiptFetchError::SignerNotAttested
        );
    }

    // ---- Both signature kinds -------------------------------------------
    //
    // NEAR AI signs with a different attested key depending on the protocol
    // the call used, for the same hosted model: Chat Completions yields
    // `provider_tee` (the model's key), the Responses API yields `gateway`
    // (the gateway's key). Codex speaks only the Responses API, so a client
    // consulting `model_attestations` alone reports every Codex-driven
    // receipt as unattested.
    // ---------------------------------------------------------------------

    /// A `gateway` receipt is attested by the gateway key. Route it as
    /// `provider_tee` -- the shipped behaviour -- and the same signer over the
    /// same report is refused, which is the defect this fixes.
    #[test]
    fn a_gateway_receipt_is_attested_by_the_gateway_key() {
        let report = attestation_report(ATTESTATION_NONCE);
        // A gateway receipt from the Responses API binds no model, so the
        // verdict carries none: the gateway arm does not need one.
        let gateway = verdict(ReceiptSignatureKind::Gateway, GATEWAY_KEY, None);
        assert!(
            signer_matches_report(&gateway, &report, ATTESTATION_NONCE, MODEL_A).is_ok(),
            "the gateway key attests a gateway-kind receipt"
        );
        assert_eq!(
            signer_matches_report(
                &provider_verdict(GATEWAY_KEY, MODEL_A),
                &report,
                ATTESTATION_NONCE,
                MODEL_A
            )
            .unwrap_err(),
            ReceiptFetchError::SignerNotAttested,
            "and the model arm refuses it -- routing is what makes the \
             difference, not the key"
        );
    }

    /// The mirror: a model key does not attest a gateway-kind receipt. Each
    /// kind has one source, and neither vouches for the other.
    #[test]
    fn a_model_key_does_not_attest_a_gateway_receipt() {
        let report = attestation_report(ATTESTATION_NONCE);
        assert_eq!(
            signer_matches_report(
                &verdict(ReceiptSignatureKind::Gateway, MODEL_A_KEY, Some(MODEL_A)),
                &report,
                ATTESTATION_NONCE,
                MODEL_A
            )
            .unwrap_err(),
            ReceiptFetchError::SignerNotAttested
        );
    }

    /// A kind this client does not recognise names no key source and is
    /// refused. Not checked against every key it holds.
    #[test]
    fn an_unrecognised_signature_kind_is_refused() {
        let report = attestation_report(ATTESTATION_NONCE);
        for signer in [MODEL_A_KEY, GATEWAY_KEY] {
            assert_eq!(
                signer_matches_report(
                    &verdict(ReceiptSignatureKind::Unrecognised, signer, Some(MODEL_A)),
                    &report,
                    ATTESTATION_NONCE,
                    MODEL_A
                )
                .unwrap_err(),
                ReceiptFetchError::SignerNotAttested,
                "an unrecognised kind is refused even for a genuinely \
                 attested signer"
            );
        }
    }

    /// The kind is read off the wire. Absent means unrecognised, not a
    /// default -- there is no safe default, since guessing one would check a
    /// signer against a set the receipt never claimed.
    #[test]
    fn the_wire_signature_kind_reaches_the_payload() {
        let body = |kind: &str| {
            format!(
                r#"{{"text":"a:b","signature":"cc","signing_address":"dd","signing_algo":"ed25519","signature_kind":"{kind}"}}"#
            )
        };
        assert_eq!(
            parse_receipt_response(&body("gateway"))
                .expect("a receipt")
                .signature_kind,
            ReceiptSignatureKind::Gateway
        );
        assert_eq!(
            parse_receipt_response(&body("provider_tee"))
                .expect("a receipt")
                .signature_kind,
            ReceiptSignatureKind::ProviderTee
        );
        assert_eq!(
            parse_receipt_response(&body("something_new"))
                .expect("a receipt")
                .signature_kind,
            ReceiptSignatureKind::Unrecognised
        );
        assert_eq!(
            parse_receipt_response(
                r#"{"text":"a:b","signature":"cc","signing_address":"dd","signing_algo":"ed25519"}"#
            )
            .expect("a receipt")
            .signature_kind,
            ReceiptSignatureKind::Unrecognised,
            "an absent kind is unrecognised, never a default"
        );
        // Present but not a string is malformed, matching `signing_algo`.
        assert_eq!(
            parse_receipt_response(
                r#"{"text":"a:b","signature":"cc","signing_address":"dd","signature_kind":7}"#
            )
            .unwrap_err(),
            ReceiptFetchError::ResponseMalformed
        );
    }

    /// A good signer over a report issued for a *different* nonce is still
    /// refused -- the report was not attested for this fetch.
    #[test]
    fn a_good_signer_over_a_report_for_a_different_nonce_is_refused() {
        let other_nonce = "0".repeat(64);
        assert_eq!(
            signer_matches_report(
                &provider_verdict(MODEL_A_KEY, MODEL_A),
                &attestation_report(&other_nonce),
                ATTESTATION_NONCE,
                MODEL_A,
            )
            .unwrap_err(),
            ReceiptFetchError::SignerNotAttested
        );
    }

    /// A 404 says the endpoint has no receipt for this call. That is an
    /// answer, not a failure to get one, and the three situations it used to
    /// be indistinguishable from -- a brokered model, an outage, and (once
    /// `nearai/cloud-api` #943 makes Responses-API attestation best-effort) a
    /// hosted call whose stream was interrupted before a record was written
    /// -- are what this fetch has to be able to tell apart.
    #[test]
    fn a_receipt_endpoint_404_is_absence_not_a_transport_failure() {
        assert_eq!(
            receipt_status_error(reqwest::StatusCode::NOT_FOUND),
            ReceiptFetchError::ReceiptNotFound
        );
        assert_ne!(
            receipt_status_error(reqwest::StatusCode::NOT_FOUND),
            ReceiptFetchError::Unreachable,
            "an endpoint that answered is not an endpoint that did not"
        );
    }

    /// A credential the endpoint rejects looks exactly like absence from the
    /// caller's side unless it is named: both leave the submission
    /// unattested, but only one is fixed by fixing this machine's config.
    #[test]
    fn a_receipt_endpoint_auth_refusal_is_its_own_case() {
        for status in [
            reqwest::StatusCode::UNAUTHORIZED,
            reqwest::StatusCode::FORBIDDEN,
        ] {
            assert_eq!(
                receipt_status_error(status),
                ReceiptFetchError::ReceiptEndpointUnauthorized
            );
        }
    }

    /// Everything else the endpoint can answer -- 500, 502, a rate limit --
    /// is a fourth thing again: the endpoint is there and did not refuse us,
    /// it just did not serve a receipt this time.
    #[test]
    fn any_other_receipt_endpoint_status_is_neither_absence_nor_refusal() {
        for status in [
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            reqwest::StatusCode::BAD_GATEWAY,
            reqwest::StatusCode::TOO_MANY_REQUESTS,
        ] {
            assert_eq!(
                receipt_status_error(status),
                ReceiptFetchError::ReceiptEndpointUnexpectedStatus
            );
        }
    }

    /// The point of the split, stated as one assertion: the four ways a
    /// receipt can fail to arrive from a reachable-or-not endpoint are four
    /// values, not one. Collapse any pair and this fails.
    #[test]
    fn the_four_receipt_fetch_outcomes_are_pairwise_distinct() {
        let outcomes = [
            receipt_status_error(reqwest::StatusCode::NOT_FOUND),
            receipt_status_error(reqwest::StatusCode::UNAUTHORIZED),
            receipt_status_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR),
            ReceiptFetchError::Unreachable,
        ];
        for (i, a) in outcomes.iter().enumerate() {
            for b in &outcomes[i + 1..] {
                assert_ne!(a, b, "two receipt-fetch outcomes share one value");
            }
        }
    }

    /// The report fetch gets the same three-way split, under its own names.
    /// Reusing the receipt's names would report "no receipt for this call"
    /// when what happened is that the *attestation report* was missing, which
    /// is a different thing to go look at.
    #[test]
    fn the_attestation_report_statuses_split_the_same_way_under_their_own_names() {
        assert_eq!(
            attestation_report_status_error(reqwest::StatusCode::NOT_FOUND),
            ReceiptFetchError::AttestationReportNotFound
        );
        assert_eq!(
            attestation_report_status_error(reqwest::StatusCode::FORBIDDEN),
            ReceiptFetchError::AttestationReportUnauthorized
        );
        assert_eq!(
            attestation_report_status_error(reqwest::StatusCode::BAD_GATEWAY),
            ReceiptFetchError::AttestationReportUnexpectedStatus
        );
        assert_ne!(
            attestation_report_status_error(reqwest::StatusCode::NOT_FOUND),
            receipt_status_error(reqwest::StatusCode::NOT_FOUND),
            "a missing report is not a missing receipt"
        );
    }

    /// Every variant carries a distinct fixed label, because the label is the
    /// only thing that reaches a log line and two variants sharing one would
    /// undo the split at exactly the surface that has to show it.
    #[test]
    fn every_fetch_error_has_its_own_fixed_label() {
        let all = [
            ReceiptFetchError::EndpointNotHttps,
            ReceiptFetchError::IdentifierMalformed,
            ReceiptFetchError::EndpointNotAllowed,
            ReceiptFetchError::NotConfigured,
            ReceiptFetchError::Unreachable,
            ReceiptFetchError::ReceiptNotFound,
            ReceiptFetchError::ReceiptEndpointUnauthorized,
            ReceiptFetchError::ReceiptEndpointUnexpectedStatus,
            ReceiptFetchError::AttestationReportNotFound,
            ReceiptFetchError::AttestationReportUnauthorized,
            ReceiptFetchError::AttestationReportUnexpectedStatus,
            ReceiptFetchError::ResponseTooLarge,
            ReceiptFetchError::ResponseMalformed,
            ReceiptFetchError::SignerNotAttested,
            ReceiptFetchError::ReceiptUnverified,
        ];
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                assert_ne!(a.label(), b.label(), "two variants share one label");
            }
        }
        // The label submit.rs has always logged for this one, unchanged: a
        // narrower enum should not rename a label an operator already greps.
        assert_eq!(
            ReceiptFetchError::SignerNotAttested.label(),
            "receipt_signer_not_attested"
        );
        // Labels are fixed strings, never caller data: nothing here is
        // interpolated, and this asserts the shape rather than trusting it.
        for err in all {
            assert!(
                err.label()
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b == b'_'),
                "a label is not a fixed lowercase token"
            );
        }
    }
}
