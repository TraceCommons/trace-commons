# Attested inference in a witness request

**Status:** design. Nothing here is built.

A witness certificate today says a specific enclave redacted specific bytes and
reached a verdict over its own pass. It says nothing about where the transcript
came from, and nothing stops a contributor synthesising one. This spec covers
carrying NEAR AI's per-request receipts through the witness so a trace can also
say **the inference it records actually happened, in an attested enclave**.

## What already exists, and what does not

Built, and used by the attestation drill:

- `crates/trace-commons-attestation/src/receipt.rs` -- `verify_receipt`,
  `ReceiptPayload`, `ReceiptVerdict`, `ReceiptError`. It recovers the signer
  rather than trusting the claimed address, and binds
  `SHA256(request_body_as_sent)` and `SHA256(response_body_as_received)`.
- `crates/trace-commons-server/src/near_attestation/client.rs` --
  `fetch_receipt`, `GET {base}/signature/{chat_id}`.
- The crate is **permissive** (MIT OR Apache-2.0), so the contributor, the
  witness and ingest can all verify. Where verification happens is a design
  choice, not a licensing constraint.

Not built: anything that requires, carries, or reports a receipt on the
contribution path. The witness has never seen one.

## The obstacle that shapes everything else

A receipt binds the **HTTP bodies**, not the transcript.

The witness receives a rendering of a session. To verify a receipt it would
need `request_body_as_sent` and `response_body_as_received` byte-for-byte --
and `client.rs` already documents why an approximation fails: a
re-serialisation surfaces as `RequestHashMismatch`, "which reads as tampering
rather than as the caller bug it would be."

So requiring verification **inside** the witness means sending it the raw HTTP
bodies as well as the transcript. That enlarges the blast radius of the
component whose README already calls it the largest in the system: "the witness
holds every raw transcript that passes through it."

**This is the decision the rest of the design turns on**, and it is why the
recommendation below does not put verification in the enclave.

## Recommendation: carry and report, do not require

Three properties, in the order they matter:

1. **The contributor verifies, because the contributor already has the bytes.**
   The client made the inference call; it holds the request and response bodies
   without anyone sending them anywhere. `verify_receipt` is permissive and
   already usable there. Nothing new leaves the machine.
2. **The witness carries a summary, and says what it is.** The request gains an
   optional `inference_attestation` block: per inference, the receipt's
   `chat_id`, the recovered `signing_address`, and the two bound hashes -- not
   the bodies. The certificate gains a field stating how many of the trace's
   inferences carried a verified receipt, in the form `n_of_m`.
3. **Requiring it is a server policy, not a witness gate.** Ingest already has
   the shape for this: `witness_admits_trace` requires a verified certificate
   AND a `Low` verdict AND an allowlisted `redaction_policy_version`. An
   attested-inference threshold belongs beside those, where it can be tightened
   per deployment without a client release.

### Why not require it in the witness

- **It would exclude the corpus.** Claude Code, Codex, Gemini and Cline traces
  run on Anthropic, OpenAI and Google. A NEAR AI receipt exists only for
  inference that went through NEAR AI -- in practice, only via IronWire. A hard
  requirement makes witnessing conditional on the inference provider, which is
  a product decision with a large scope consequence and should be taken
  explicitly rather than inherited from a security control.
- **It moves raw bodies into the enclave**, per above.
- **It costs per-inference egress on a public unauthenticated route** already
  bounded at four concurrent requests and a 300s deadline.

### What the certificate may and may not say

The certificate's existing discipline applies unchanged: it attests mechanics
and a verdict, never cleanliness. An `inference_attested` count says *this many
receipts verified against these hashes*. It does **not** say the trace is
genuine, complete, or that unattested inferences did not occur -- a trace can
carry one verified receipt and nine fabricated turns. Anything written on an
operator surface must say `n_of_m`, never "attested".

## Open questions

- **Does IronWire's ledger already carry the `chat_id`?** The review of the
  routing integration found `upstream_id` -- the provider's own response id --
  fetched and then dropped by the contributor. If that is the `chat_id` the
  receipt endpoint takes, the plumbing is a field away rather than a capture
  change. Confirm before designing the capture path.
- **What binds a receipt to *this* trace?** The hashes bind it to bodies. If
  nothing ties those bodies to the transcript, a contributor can attach someone
  else's valid receipts. The transcript would need to carry the same hashes, or
  the join is decorative.
- **Two-part vs three-part receipts.** `ReceiptVerdict.model` is `None` for the
  two-part form, "which binds no model at all". A policy that cares which model
  served the inference has to refuse the two-part form.
- **Revocation and expiry.** Receipts have no freshness rule here; the witness
  certificate now does. Decide whether an old receipt is admissible.

## Not in this spec

- Requiring attested inference. That is the policy layer, and it should be
  designed once this is carried and measurable in real traces.
- Verifying the NEAR AI enclave's own quote per request. The drill does this
  for the endpoint; doing it per inference is a separate cost decision.
