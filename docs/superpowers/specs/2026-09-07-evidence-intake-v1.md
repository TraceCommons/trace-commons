# Application-independent local intake v1

Status: reviewed with workstream C before wire implementation.
Base: 7d518662. This document does not enable a provider or change admission.

## Admission and authority

An import is untrusted data, never an invitation, identity, consent grant or
attestation verdict. Existing authenticated invite redemption authorizes ordinary
contributions under normal privacy and contributor-approval policy. Without that
invite authority, each contribution needs its own verified inference evidence,
witness-redacted/certified artifact, and approval bound to those exact bytes.
A prior successful attestation does not authorize a later ordinary import.

## Proposed local document

A UTF-8 JSON object with `schema_version: 1`, `trace` (the existing permissive
`RawTraceContribution`), and optional `inference`. Unknown top-level/inference
fields are refused. The document is at most 32 MiB before deserialization;
trace is at most the existing 16 MiB envelope/raw limit. Event IDs must be
unique and parent references must name an earlier event. Source provenance uses existing
`ironclaw.feature_flags.agent`; its spelling never selects admission policy.
Imported tenant/contributor/consent claims are not trusted: authenticated local
configuration and the existing granted-consent path remain authoritative.

`inference` is a single explicitly identified call:

- `exchange_id`: a bounded opaque identifier, not a path or URL.
- `coverage`: exactly `final_call_only`; no history/tool/completeness claim.
- `request_body`, `response_body`: exact UTF-8 strings (8 MiB each maximum).
  JSON escaping is only the container; decoded UTF-8 bytes are signed bytes.
  No reserialization of embedded JSON or SSE reconstruction is permitted.
- `request_sha256`, `response_sha256`: expected lowercase hex SHA-256, checked
  over the decoded body bytes. These are consistency checks, not trust.
- `upstream_id`, optional `served_model`, `status`, `timestamp`: observed call
  metadata, bounded; model metadata is not covered by a two-part receipt.
- `receipt`: the existing witness receipt shape (`text`, `signature`,
  `signing_address`, `signing_algo`, `signature_kind`). Parse discriminators
  through the existing attestation crate and reuse its verifier. No new verifier.

V1 carries no local filenames, arbitrary retrieval URL, authentication header,
or caller-supplied trusted-measurement override. Provider signer/quote/collateral
verification uses the witness's existing configured NEAR AI retrieval and
measurement policy, not bundle-selected hosts. Missing nonce-bound signer
verification or unavailable witness produces a named refusal on the no-invite
path. Raw bodies may inherently contain sensitive model-call content; no client
credential container is added to the schema.

A no-invite import preserves the existing nonce/account AdmissionBinding in
its signed request; it cannot be injected after signing. Existing companion
HttpExchange events are refused for an evidence import, so only the isolated
call appended last is selected. A final-call receipt never admits unrelated
history: the witness must reject unsupported companion content or derive only
a verified-exchange projection under its explicit policy. Invited ordinary
traces remain subject to normal policy without this attestation requirement.

## Transport and lifecycle

The proposed local type lives in `trace-commons-protocol`; existing receipt
crypto stays in `trace-commons-attestation`. Local contributor intake validates
structure/bounds and exact digests, then converts the isolated call into the
existing `AttestedCall`/`AttestedInference` and witness request. No changes to
`POST /v1/witness` serialization are required for the first checkpoint.

Ordinary imports omit `inference` and use the existing local redaction path;
they are not refused for missing receipts. Evidence imports are never silently
converted to ordinary submissions. Parsing and local preview perform no network
requests, credential discovery, source auto-discovery, or capture enablement.
Remote witness processing requires existing explicit consent and a verified
witness before any raw data leaves the process. Final submission uses the exact
approved certified bytes and existing auth/claim/deduplication machinery.

Raw inference carriers exist only in memory for the explicit witness request;
never add them to an ordinary queued envelope or staging file. A local imported
file is explicitly user-selected; the importer does not persist a second copy.
The first executable checkpoint is bounded import/preview plus transport reuse
and tests; authoritative no-invite admission enforcement is workstream C's gate.
Until C verifies the complete conjunction, no local preview says eligible.

## Required tests

Two source names (OpenCode and a second client) produce identical policy.
Ordinary import needs no receipt. Evidence import refuses body swaps, unknown
receipt kind/algo, invalid signatures, missing evidence, invalid coverage,
duplicate IDs, oversized/invalid JSON, binary bodies, and URL/path fields.
Receipt verification is cryptographic but does not replace signer attestation.
Replay/account binding and unavailable-witness behavior require the shared
witness/admission tests; parsing alone cannot establish replay protection.
Witness-returned raw carriers or changed approved artifacts remain refused.

## First executable checkpoint

With a development build containing this change:

```sh
trace-commons-contributor import-preview --file ./contribution.import.json --cwd /original/project
```

The required `--cwd` supplies the original machine's working-directory prefix for
redaction; it is never opened or resolved. Imported identity, consent, correction
and other metadata claims are rebuilt as conservative unenrolled-preview defaults.
`preview_sha256` identifies only these local preview bytes, never a submit-ready or
certified artifact. A later witness artifact requires a separate exact-byte review.

This reads only the explicitly selected local file and prints the local redacted
preview as JSON. It does not open/create enrollment state, queue a contribution,
contact a witness/provider, or upload. `preview_only: true` and
`admission_verified: false` are always present. Evidence imports additionally
say `signature-consistent-witness-required` and `final_call_only`; a valid local
signature alone never establishes trusted signer attestation or replay safety.
No authenticated evidence-submission command is introduced by this checkpoint.

Existing invited users can continue to submit ordinary normalized exports using
`trace-commons-contributor submit --trajectory ./session.trajectory.json` under
the existing approval/privacy policy. The new document is a separate explicit
raw-contribution interchange format; it is not mislabeled as a trajectory file.

The reusable `PreparedImport::witness_input(config)` constructs conservative
metadata from local configuration and omits all imported history, outcome,
replay and value claims. Together with `attested_inference()` it feeds the
existing witness transport, which appends exactly the isolated final exchange.
It does not itself perform remote processing. Caller consent, verified witness,
server admission/profile checks and exact certified-artifact approval remain
required. This first projection promises neither full-session content nor full
session attestation; deriving useful response content is a separate qualified
witness change.
