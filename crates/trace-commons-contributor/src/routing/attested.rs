//! Carrying one inference call's verbatim bodies into a trace.
//!
//! Everything else in `routing` is metadata: numbers a proxy reported, which
//! are worth having even when they are approximate. This module is the one
//! place that moves **bytes**, and it is held to a different standard because
//! of what those bytes are for.
//!
//! # Why bytes at all
//!
//! A NEAR AI inference receipt is an EIP-191 signature over
//! `<sha256 of the request body as sent>:<sha256 of the response body as
//! received>`. Verifying it needs the exact bytes and nothing else. So the
//! only useful thing to carry is a verbatim copy: anything that
//! pretty-prints, reorders keys, re-escapes a non-ASCII character or
//! round-trips a float turns a valid receipt into a hash mismatch, and a hash
//! mismatch reads as tampering rather than as the capture bug it is.
//!
//! IronWire already holds those bytes. `capture.bodies = true` writes each
//! exchange's request and response to `$IRONWIRE_HOME/bodies` (mode 0700) as
//! `&[u8]`, never a `String`, and records `request_sha256` /
//! `response_sha256` on the ledger row beside `body_ref`. This module reads
//! them back.
//!
//! # Where the bodies go, and where they must never go
//!
//! **Only in transit to a witness.** This module produces an event; the one
//! function that puts it into a contribution is
//! [`crate::witness::transport::witness_contribution`], and it appends it to
//! a copy that lives for the length of one request. The witness verifies the
//! receipt against these bytes inside its enclave, **strips them**, and
//! certifies the stripped artifact.
//!
//! So a captured body reaches exactly one remote process, is never written to
//! a queued trace, never passes through the local redaction path, and is
//! never in anything submitted to ingest. A contributor who has configured no
//! witness ships exactly what they shipped before this module existed --
//! `crate::envelope`'s
//! `a_locally_built_trace_never_carries_an_attested_body` is the pin.
//!
//! That shape is not a preference. Local redaction would not have made this
//! safe on its own: the deterministic passes catch *shaped* secrets, and the
//! only thing that removes a raw conversation prefix from a published
//! envelope is a wholesale field replacement keyed on a tool name. Depending
//! on a lookup table for that is a control one rename away from failing
//! silently. Not carrying the bytes at all has no such failure mode.
//!
//! # Only the final call, and only one
//!
//! One request/response pair per trace. A chat-completions request body
//! repeats the whole conversation prefix, so per-turn bodies are quadratic in
//! session length, and 7% of real sessions on this pilot already exceed the
//! 16 MB envelope cap at a 3.4:1 raw-to-envelope ratio.
//!
//! The *final* call, because the witness attests the last `HttpExchange`
//! event a trace declares and nothing else. If the final call cannot be
//! carried, this module refuses -- it does not fall back to an earlier one.
//! Falling back would put a body in the last position that is not the last
//! call, which is a quiet lie about which call was attested.
//!
//! And note what one pair does *not* establish: the transitive-coverage
//! argument (the final request contains the prefix) holds for an uncompacted
//! linear session and fails for one that summarised or truncated its context.
//! Nothing here or downstream may say the attested call covers the history.
//!
//! # Every refusal is fail-closed, and named
//!
//! [`Unattestable`] has one variant per reason, and every one of them means
//! *no bodies are carried*. There is no lossy path: this module never calls
//! `from_utf8_lossy`, never truncates, and never carries a body whose digest
//! disagrees with the one the proxy recorded. A body that cannot be carried
//! faithfully is not carried, and the trace is honestly unattestable.
//!
//! # Non-UTF-8 bodies are refused rather than converted
//!
//! `HttpExchangeRequest.body` and the envelope's `content` are `String`s, and
//! the witness hashes `str::as_bytes()`. `String::from_utf8` is exactly
//! lossless in both directions, so a body that *is* valid UTF-8 round-trips
//! byte for byte -- which is what makes the digest survive. A body that is
//! not valid UTF-8 has no faithful representation in that carrier at all, and
//! IronWire proves such a body can exist (it has a test for one). Converting
//! it lossily would change the bytes and produce a receipt failure that reads
//! as tampering. So it is refused as [`Unattestable::BodyNotUtf8`].
//!
//! This is the deliberate answer to "the carrier is a String but the bytes are
//! bytes", and it is chosen over widening the protocol. A base64 or
//! byte-array field would carry more, but only by adding a second encoding of
//! the same bytes that every consumer -- the witness, the redactor, ingest --
//! would have to learn, in exchange for a case that JSON and SSE bodies
//! cannot produce. If a route ever does carry a non-UTF-8 inference body,
//! this refusal is the signal to widen the protocol then, with the evidence
//! in hand.
//!
//! # Nothing here is logged
//!
//! Bodies, URLs, model names, session ids and the provider's own exchange
//! identifier are all caller data. [`Unattestable`] carries no payload beyond
//! its own name, and no `Debug` in this module prints a body.

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};
use trace_commons_protocol::trace_contribution::{
    RawTraceContributionEvent, TraceContributionEventType,
};
use uuid::Uuid;

use super::RoutedExchange;

/// How large either carried body may be, in bytes.
///
/// 8 MiB, matching the witness's own `DEFAULT_MAX_BODY_BYTES`. The request
/// body is the whole conversation prefix, so this has to clear a large
/// session rather than a single turn, while staying well inside the 16 MB
/// envelope cap -- a pair at this bound plus the transcript would not fit,
/// which is why [`envelope`](crate::envelope)'s size guard still runs after
/// this one and is still authoritative.
///
/// The witness applies its own bound independently. This one is not a
/// substitute for it: a contributor can patch this binary, and a bound only
/// the client enforces is not a bound.
pub const MAX_ATTESTED_BODY_BYTES: usize = 8 * 1024 * 1024;

/// The tool name an attested exchange event carries.
///
/// Defence in depth, and worth keeping even though nothing now depends on it.
/// The primary control is that this event never reaches the local redaction
/// path at all. But if one ever did -- a future path that persists a witnessed
/// input, a test fixture that leaks into production -- `"http"` selects the
/// redactor's browser profile, whose `body` rule replaces the field wholesale
/// rather than merely scrubbing it. Under any other tool name the same
/// accident would publish the conversation prefix.
/// `the_event_uses_the_tool_name_the_redactor_profiles` pins it.
pub const ATTESTED_EXCHANGE_TOOL_NAME: &str = "http";

/// The marker an exchange carries when its stream was restarted mid-flight.
///
/// Read by the witness at `structured_payload["response"][..]`. IronWire's
/// resilience guard restarts a stalled stream and records **no** response
/// digest for it, so a restarted call has no receipt and never will. This
/// module cannot see a restart directly; what it sees is the missing digest,
/// and it writes this marker so an operator gets "unattestable" rather than
/// "the contributor withheld a receipt".
///
/// See the module-level note in the report: reconciling this into one
/// contract needs an IronWire change this branch did not make.
pub const STREAM_RESTARTED_MARKER: &str = "stream_restarted";

/// Why the final call's bodies could not be carried.
///
/// Every variant means the same thing operationally -- no bodies, no
/// attestation -- and they are separate because an operator does something
/// different about each. Label-only by construction: no variant carries a
/// path, a size, a digest or an identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Unattestable {
    /// The session joined no inference hops at all.
    #[error("the session declares no inference call")]
    NoCall,
    /// The proxy recorded no bodies for the final call: `capture.bodies` is
    /// off, or the body could not be held whole.
    #[error("the final inference call has no captured bodies")]
    CaptureOff,
    /// One of the two digests is absent. On the response side that is what a
    /// restarted, cancelled or truncated stream looks like.
    #[error("the final inference call recorded no digest")]
    DigestAbsent,
    /// The proxy recorded no provider identifier, so no receipt is reachable.
    #[error("the final inference call has no provider identifier")]
    UpstreamIdAbsent,
    /// The body reference is not one the store could have written.
    #[error("the body reference is malformed")]
    ReferenceMalformed,
    /// The bodies are named by the row but not readable.
    #[error("the captured bodies could not be read")]
    BodiesUnreadable,
    /// A body is not valid UTF-8 and therefore has no faithful
    /// representation in the carrier. See the module docs.
    #[error("a captured body is not valid UTF-8")]
    BodyNotUtf8,
    /// A body is larger than [`MAX_ATTESTED_BODY_BYTES`].
    #[error("a captured body is larger than the attested-body bound")]
    BodyTooLarge,
    /// The bytes on disk do not hash to the digest the row recorded.
    ///
    /// Refused rather than carried. The receipt is taken over the digest the
    /// proxy recorded, so bytes that disagree with it cannot verify, and
    /// carrying them would spend the witness's work to produce a mismatch
    /// that reads as tampering.
    #[error("the captured bodies do not match the recorded digest")]
    DigestMismatch,
}

/// The final call's verbatim bodies, ready to become an event.
///
/// Deliberately no `Debug`, `Serialize` or `Clone` derive that would print or
/// duplicate the bodies casually: the fields are private and the accessors
/// hand out borrows.
pub struct AttestedCall {
    request_body: String,
    response_body: String,
    upstream_id: String,
    served_model: Option<String>,
    status: u16,
    timestamp: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for AttestedCall {
    /// Sizes, never contents. A `Debug` that printed the bodies would be a
    /// hole straight through the hash-only rule, and this type exists to be
    /// held beside things that get logged.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttestedCall")
            .field("request_bytes", &self.request_body.len())
            .field("response_bytes", &self.response_body.len())
            .finish_non_exhaustive()
    }
}

impl AttestedCall {
    /// The request body exactly as it went upstream.
    #[must_use]
    pub fn request_body(&self) -> &str {
        &self.request_body
    }

    /// The response body exactly as it came back. For a streamed response
    /// this is the raw concatenated event stream, and it **must not be
    /// parsed**: reassembled content would never hash to the same digest.
    #[must_use]
    pub fn response_body(&self) -> &str {
        &self.response_body
    }

    /// The provider's own identifier for this exchange -- NEAR AI's
    /// `chat_id`, and the handle its receipt endpoint takes.
    #[must_use]
    pub fn upstream_id(&self) -> &str {
        &self.upstream_id
    }

    /// The model the provider reported serving, when it reported one.
    ///
    /// The receipt endpoint takes this as an **unsigned** query parameter, so
    /// it establishes nothing on its own; it is here because the endpoint
    /// requires it, not because it is evidence.
    #[must_use]
    pub fn served_model(&self) -> Option<&str> {
        self.served_model.as_deref()
    }
}

/// Read the final inference call's verbatim bodies out of the proxy's body
/// store.
///
/// `rows` are the hops joined to this session, and the last of them by
/// `started_at` is the final call. Refuses rather than falling back to an
/// earlier hop -- see the module docs.
///
/// # Errors
///
/// [`Unattestable`] for every reason a faithful pair could not be produced.
/// A caller builds a trace without an exchange event and the session is
/// honestly unattestable; nothing here may fail a submission.
pub fn attested_final_call(
    rows: &[RoutedExchange],
    bodies_dir: &Path,
) -> Result<AttestedCall, Unattestable> {
    let final_call = rows
        .iter()
        // `max_by_key` on a tie keeps the last element, which is what we
        // want: the ledger's own insertion order breaks a timestamp tie, and
        // `exchanges_since` yields oldest first.
        .max_by_key(|row| (row.started_at, row.id))
        .ok_or(Unattestable::NoCall)?;

    let body_ref = final_call
        .body_ref
        .as_deref()
        .ok_or(Unattestable::CaptureOff)?;
    let (request_sha256, response_sha256) = match (
        final_call.request_sha256.as_deref(),
        final_call.response_sha256.as_deref(),
    ) {
        (Some(request), Some(response)) => (request, response),
        // A missing response digest is the restarted/truncated-stream case.
        // Named as an absent digest rather than as a restart, because that is
        // what was actually observed: the proxy records no marker.
        _ => return Err(Unattestable::DigestAbsent),
    };
    let upstream_id = final_call
        .upstream_id
        .as_deref()
        .ok_or(Unattestable::UpstreamIdAbsent)?;

    let (request_bytes, response_bytes) = read_bodies(bodies_dir, body_ref)?;

    if request_bytes.len() > MAX_ATTESTED_BODY_BYTES
        || response_bytes.len() > MAX_ATTESTED_BODY_BYTES
    {
        return Err(Unattestable::BodyTooLarge);
    }
    if sha256_hex(&request_bytes) != request_sha256.to_ascii_lowercase()
        || sha256_hex(&response_bytes) != response_sha256.to_ascii_lowercase()
    {
        return Err(Unattestable::DigestMismatch);
    }

    // Lossless in both directions. `as_bytes()` on the result is byte-for-byte
    // what was read, which is the whole reason a receipt can still verify.
    let request_body = String::from_utf8(request_bytes).map_err(|_| Unattestable::BodyNotUtf8)?;
    let response_body = String::from_utf8(response_bytes).map_err(|_| Unattestable::BodyNotUtf8)?;

    Ok(AttestedCall {
        request_body,
        response_body,
        upstream_id: upstream_id.to_string(),
        served_model: final_call.served_model.clone(),
        status: u16::try_from(final_call.status).unwrap_or(0),
        timestamp: final_call.started_at,
    })
}

/// The attested call as an envelope event.
///
/// The shape is fixed by what the witness reads:
/// `structured_payload["request"]["body"]` is the request as sent, and
/// `content` is the response as received. It is also fixed by what the
/// redactor does with it -- see [`ATTESTED_EXCHANGE_TOOL_NAME`].
///
/// The event is appended last, and the witness takes the last `HttpExchange`
/// event in trace order. A caller that appends anything after this one has
/// changed which call is attested.
#[must_use]
pub fn attested_exchange_event(call: &AttestedCall) -> RawTraceContributionEvent {
    RawTraceContributionEvent {
        event_id: Uuid::new_v4(),
        parent_event_id: None,
        event_type: TraceContributionEventType::HttpExchange,
        timestamp: call.timestamp,
        content: Some(call.response_body.clone()),
        structured_payload: serde_json::json!({
            // No `url` key. The proxy's row does not record one, and a
            // synthesised URL would be an assertion rather than an
            // observation -- an absent field beats a fabricated one.
            "request": {
                "method": "POST",
                "body": call.request_body,
            },
            "response": {
                "status": call.status,
                // Declared false rather than omitted. The witness reads an
                // absent marker as "not declared", never as "did not happen";
                // saying so explicitly is only honest because
                // `attested_final_call` refuses a call with no response
                // digest, which is the only restart this side can observe.
                STREAM_RESTARTED_MARKER: false,
            },
        }),
        tool_name: Some(ATTESTED_EXCHANGE_TOOL_NAME.to_string()),
        tool_call_id: None,
        latency_ms: None,
        token_counts: None,
        cost_usd: None,
        success: Some((200..400).contains(&call.status)),
        failure_modes: Vec::new(),
    }
}

/// Read one exchange's bodies back out of the store.
///
/// The reference comes off a row this process read from another process's
/// database, so it is validated rather than trusted: a `..` in it would read
/// whatever this process can.
fn read_bodies(dir: &Path, reference: &str) -> Result<(Vec<u8>, Vec<u8>), Unattestable> {
    if !is_body_reference(reference) {
        return Err(Unattestable::ReferenceMalformed);
    }
    let request = std::fs::read(body_path(dir, reference, "req"))
        .map_err(|_| Unattestable::BodiesUnreadable)?;
    let response = std::fs::read(body_path(dir, reference, "res"))
        .map_err(|_| Unattestable::BodiesUnreadable)?;
    Ok((request, response))
}

fn body_path(dir: &Path, reference: &str, suffix: &str) -> PathBuf {
    dir.join(format!("{reference}.{suffix}"))
}

/// `<digits>-<digits>`, and nothing else: no separator, no dot, no `..`.
///
/// Mirrors IronWire's own `is_reference`. Duplicated rather than imported
/// because this crate takes no dependency on IronWire; if the two ever
/// disagree the cost is a refusal, not an escape, because this one is the
/// stricter direction to get wrong.
fn is_body_reference(reference: &str) -> bool {
    let Some((left, right)) = reference.split_once('-') else {
        return false;
    };
    !left.is_empty()
        && !right.is_empty()
        && left.bytes().all(|b| b.is_ascii_digit())
        && right.bytes().all(|b| b.is_ascii_digit())
}

/// Lowercase hex SHA-256 of exactly these bytes.
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    /// A request body a re-serialiser would demonstrably change: keys are not
    /// in alphabetical order, there is inconsistent whitespace, a non-ASCII
    /// character sits inside a string, and the float has more precision than
    /// a round-trip through `f64` formatting reliably preserves.
    ///
    /// `{"a":1}` would pass through any re-serialiser unchanged and would
    /// prove nothing.
    const AWKWARD_REQUEST: &str = "{\"model\":\"Qwen/Qwen3.6-27B-FP8\", \"temperature\":0.30000000000000004,\n  \"messages\":[{\"role\":\"user\",\"content\":\"café — naïve\"}],\"stream\":true}";

    const AWKWARD_RESPONSE: &str =
        "data: {\"choices\":[{\"delta\":{\"content\":\"café\"}}]}\n\ndata: [DONE]\n\n";

    fn row() -> RoutedExchange {
        RoutedExchange {
            id: Some(7),
            started_at: Utc.with_ymd_and_hms(2026, 9, 3, 12, 0, 0).unwrap(),
            client_session_id: Some("session".to_string()),
            total_ms: Some(1200),
            facade: "openai".to_string(),
            backend: "nearai".to_string(),
            requested_model: Some("Qwen/Qwen3.6-27B-FP8".to_string()),
            served_model: Some("Qwen/Qwen3.6-27B-FP8".to_string()),
            upstream_id: Some("chatcmpl-abc123".to_string()),
            request_sha256: Some(sha256_hex(AWKWARD_REQUEST.as_bytes())),
            response_sha256: Some(sha256_hex(AWKWARD_RESPONSE.as_bytes())),
            body_ref: Some("00000000000000000001-000000".to_string()),
            rung: "full".to_string(),
            attempts: 1,
            input_tokens: Some(10),
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: Some(4),
            cost_usd: Some(0.01),
            status: 200,
        }
    }

    fn store_with(request: &[u8], response: &[u8], reference: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(format!("{reference}.req")), request).expect("req");
        std::fs::write(dir.path().join(format!("{reference}.res")), response).expect("res");
        dir
    }

    /// The point of the whole module: what comes back out hashes to what went
    /// in, over a body that a re-serialiser would not reproduce.
    #[test]
    fn a_carried_body_is_byte_identical_to_what_was_captured() {
        let row = row();
        let dir = store_with(
            AWKWARD_REQUEST.as_bytes(),
            AWKWARD_RESPONSE.as_bytes(),
            row.body_ref.as_deref().unwrap(),
        );

        let call = attested_final_call(std::slice::from_ref(&row), dir.path()).expect("attestable");

        assert_eq!(
            sha256_hex(call.request_body().as_bytes()),
            row.request_sha256.as_deref().unwrap(),
            "the carried request body must hash to the digest the proxy recorded"
        );
        assert_eq!(
            sha256_hex(call.response_body().as_bytes()),
            row.response_sha256.as_deref().unwrap(),
            "the carried response body must hash to the digest the proxy recorded"
        );
        assert_eq!(call.request_body(), AWKWARD_REQUEST);
        assert_eq!(call.upstream_id(), "chatcmpl-abc123");
    }

    /// And the digest survives the trip through the event and back out of
    /// serialized JSON -- which is where a re-serialisation would actually
    /// happen in production.
    #[test]
    fn the_digest_survives_serialization_of_the_event() {
        let row = row();
        let dir = store_with(
            AWKWARD_REQUEST.as_bytes(),
            AWKWARD_RESPONSE.as_bytes(),
            row.body_ref.as_deref().unwrap(),
        );
        let call = attested_final_call(std::slice::from_ref(&row), dir.path()).expect("attestable");

        let event = attested_exchange_event(&call);
        let round_tripped: RawTraceContributionEvent =
            serde_json::from_str(&serde_json::to_string(&event).expect("serializes"))
                .expect("deserializes");

        let carried_request = round_tripped.structured_payload["request"]["body"]
            .as_str()
            .expect("the request body is a string the witness can read");
        let carried_response = round_tripped
            .content
            .as_deref()
            .expect("the response body is the event content");

        assert_eq!(
            sha256_hex(carried_request.as_bytes()),
            row.request_sha256.as_deref().unwrap(),
            "a JSON round trip must not change the attested request bytes"
        );
        assert_eq!(
            sha256_hex(carried_response.as_bytes()),
            row.response_sha256.as_deref().unwrap(),
            "a JSON round trip must not change the attested response bytes"
        );
    }

    /// A non-UTF-8 body is refused, not converted. IronWire has a test
    /// proving it can store one; this is the other half of that contract.
    #[test]
    fn a_non_utf8_body_is_refused_rather_than_converted() {
        let bytes = vec![0x7b, 0x22, 0x61, 0x22, 0x3a, 0xff, 0xfe, 0x7d];
        assert!(
            String::from_utf8(bytes.clone()).is_err(),
            "the fixture must actually be invalid UTF-8"
        );
        let mut row = row();
        row.request_sha256 = Some(sha256_hex(&bytes));
        let dir = store_with(
            &bytes,
            AWKWARD_RESPONSE.as_bytes(),
            row.body_ref.as_deref().unwrap(),
        );

        assert_eq!(
            attested_final_call(&[row], dir.path()).unwrap_err(),
            Unattestable::BodyNotUtf8
        );
    }

    /// Over the bound, nothing is carried. Fail closed and unattestable, not
    /// truncated: a digest of part of a body is a wrong answer.
    #[test]
    fn a_body_over_the_bound_is_refused_whole() {
        let oversized = vec![b'x'; MAX_ATTESTED_BODY_BYTES + 1];
        let mut row = row();
        row.request_sha256 = Some(sha256_hex(&oversized));
        let dir = store_with(
            &oversized,
            AWKWARD_RESPONSE.as_bytes(),
            row.body_ref.as_deref().unwrap(),
        );

        assert_eq!(
            attested_final_call(&[row], dir.path()).unwrap_err(),
            Unattestable::BodyTooLarge
        );
    }

    /// A body exactly at the bound is carried. Without this the previous test
    /// passes against an off-by-one that refuses everything.
    #[test]
    fn a_body_exactly_at_the_bound_is_carried() {
        let at_bound = vec![b'x'; MAX_ATTESTED_BODY_BYTES];
        let mut row = row();
        row.request_sha256 = Some(sha256_hex(&at_bound));
        let dir = store_with(
            &at_bound,
            AWKWARD_RESPONSE.as_bytes(),
            row.body_ref.as_deref().unwrap(),
        );

        let call = attested_final_call(&[row], dir.path()).expect("a body at the bound is carried");
        assert_eq!(call.request_body().len(), MAX_ATTESTED_BODY_BYTES);
    }

    /// A response with no digest is what a restarted, cancelled or truncated
    /// stream looks like from here. Refused, and no fallback to an earlier
    /// call that does have one.
    #[test]
    fn a_final_call_with_no_response_digest_is_unattestable() {
        let dir = store_with(
            AWKWARD_REQUEST.as_bytes(),
            AWKWARD_RESPONSE.as_bytes(),
            "00000000000000000001-000000",
        );
        let mut earlier = row();
        earlier.id = Some(1);
        earlier.started_at = Utc.with_ymd_and_hms(2026, 9, 3, 11, 0, 0).unwrap();

        let mut restarted = row();
        restarted.id = Some(2);
        restarted.response_sha256 = None;

        assert_eq!(
            attested_final_call(&[earlier, restarted], dir.path()).unwrap_err(),
            Unattestable::DigestAbsent,
            "an earlier attestable call must not stand in for the final one"
        );
    }

    /// Bodies that disagree with the recorded digest are refused rather than
    /// carried into a mismatch the witness would read as tampering.
    #[test]
    fn bodies_that_do_not_match_the_recorded_digest_are_refused() {
        let row = row();
        let dir = store_with(
            b"{\"model\":\"something-else\"}",
            AWKWARD_RESPONSE.as_bytes(),
            row.body_ref.as_deref().unwrap(),
        );

        assert_eq!(
            attested_final_call(&[row], dir.path()).unwrap_err(),
            Unattestable::DigestMismatch
        );
    }

    /// A reference off another process's database is validated, not trusted.
    #[test]
    fn a_traversing_body_reference_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut row = row();
        row.body_ref = Some("../../../etc/passwd-0".to_string());

        assert_eq!(
            attested_final_call(&[row], dir.path()).unwrap_err(),
            Unattestable::ReferenceMalformed
        );
    }

    /// Capture off is the deployed default, and it is a refusal by name
    /// rather than an empty body.
    #[test]
    fn capture_off_carries_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut row = row();
        row.body_ref = None;

        assert_eq!(
            attested_final_call(&[row], dir.path()).unwrap_err(),
            Unattestable::CaptureOff
        );
    }

    /// Without a provider identifier no receipt is reachable, so carrying the
    /// bodies would achieve nothing and would publish them for nothing.
    #[test]
    fn no_provider_identifier_carries_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut row = row();
        row.upstream_id = None;

        assert_eq!(
            attested_final_call(&[row], dir.path()).unwrap_err(),
            Unattestable::UpstreamIdAbsent
        );
    }

    #[test]
    fn a_session_with_no_hops_is_not_an_error_about_bodies() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(
            attested_final_call(&[], dir.path()).unwrap_err(),
            Unattestable::NoCall
        );
    }

    /// The tool name selects the redactor profile that would remove the
    /// request body if this event ever did reach the local redaction path.
    /// It is a backstop rather than the primary control -- see the constant's
    /// docs -- but changing it is still a privacy regression, not a rename.
    #[test]
    fn the_event_uses_the_tool_name_the_redactor_profiles() {
        let row = row();
        let dir = store_with(
            AWKWARD_REQUEST.as_bytes(),
            AWKWARD_RESPONSE.as_bytes(),
            row.body_ref.as_deref().unwrap(),
        );
        let call = attested_final_call(&[row], dir.path()).expect("attestable");
        let event = attested_exchange_event(&call);

        assert_eq!(event.tool_name.as_deref(), Some("http"));
        assert_eq!(event.event_type, TraceContributionEventType::HttpExchange);
    }
}
