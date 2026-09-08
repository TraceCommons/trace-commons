# v0.12 admission and witness audit

Baseline: `7d518662`. Workstream C; implementation in progress, not release evidence.

## Admission matrix and reproduced gaps

| Identity / contribution | Required authority | Current verification |
| --- | --- | --- |
| Redeemed invite, ordinary local trace | Existing authenticated invite grants, ordinary privacy and approval | Real registry resolution and device-registration transaction (idempotency and use cap), then synthetic authenticated tenant + ordinary ingest succeeds for two source names |
| NEAR identity without invite, ordinary trace | Refuse | Real PostgreSQL/RLS ingest reproduced HTTP 200; fixed to 403 |
| NEAR identity, verified single-call witness artifact | Receipt, account challenge, exact artifact certificate, authenticated approved upload | Existing synthetic signed witness-to-ingest PostgreSQL fixture passes after narrowing |
| Valid receipt plus unrelated companion history | Refuse unsupported coverage | Reproduced witness success; fixed by single-exchange admission restriction |
| Completed exact retry | Read existing authenticated immutable result | Existing PostgreSQL regression retained |
| New submission using absent/expired/swapped evidence | Refuse; previous admission is not standing authority | Every missing evidence/certificate header and changed approved bytes refused; changed-ID/expired evidence refusal retained |

The legacy V59 trial window is a budget mechanism, not valid invite authority.
NEAR account bootstrap establishes identity only. The submission boundary now
requires evidence for every new contribution in that namespace. Existing normal
invited credentials do not acquire a global receipt requirement.

## Bindings and remaining work

- `admission_evidence::verify_admission_call` verifies exact UTF-8 body bytes,
  configured signer/model policy, and the account challenge embedded in the
  signed request. Binding cannot be injected after receipt issuance.
- `WitnessService::witness_admission_contribution` redacts and certifies the
  returned bytes; the detached admission signature binds that artifact digest
  to the account challenge and receipt identity.
- Ingest verifies the certificate, allowed policy, signature, account identity,
  expiry, exact body digest, and durable replay reservation before processing.
- Single-call coverage does not attest earlier history, tools, session
  completeness, or arbitrary source metadata. Source names select no authority.
- Existing structured redaction skipped contributed metadata. A synthetic
  name-removal classifier regression reproduced leaked names in conversation,
  source flags, replay notes, and value explanations. Shared redactor hardening
  is under test; it preserves S5 correction refusal rather than rewriting it.
- Stored-artifact inspection passes for metadata email sentinels and exact
  request/response body/header carrier locations. Provider-TEE and gateway
  signatures both pass the witness fixture under configured synthetic trust.
- The admission witness projects away imported outcome, replay, cost/tool
  success, value and companion metadata claims before certification. This
  first profile yields a conservative single-call artifact, not useful full
  session history; useful text projection is a separate supported-shape task.
- Still required: complete native HTTP invite enrollment qualification,
  deployed signer/quote verification, real client/provider sessions and agreed
  native coverage/approval presentation. Local preview is not eligibility.
- Metadata traversal takes one byte/node/depth budget per pass -- the trace
  metadata, each event's metadata, each event's payload -- and going over
  budget degrades to `coverage_incomplete`, which forces residual risk to
  High. It is not a refusal: this pass runs on every submission path, and one
  budget spanning a whole contribution is reached by an ordinary long session.
  Key collisions and invalid typed output are still refusals.
- Typed leaves are not classified: UUIDs, RFC3339 timestamps, enum variants
  and server-assigned identifiers. A verdict on one is never useful and a
  rewrite of one fails the round-trip back into the typed struct. The whole
  `contributor` subtree is excluded for a second reason -- identity does not
  leave the machine for a third-party classifier.
  `metadata_schema_fields_are_pinned` fails when the schema grows, so both the
  typed set and the dynamic-key set get revisited deliberately.
- A credential detected in a metadata leaf is refused, not masked, matching
  the correction path. A contributor whose API key landed in `model_name` is
  told to rotate it rather than having it masked and uploaded.
- Fixed schema keys are not sent to the classifier; contributed values and
  dynamic map keys are. Additional classifier work is bounded but its
  real-provider latency has not been measured.
- `AdmissionProviderTrust` holds one signer set per receipt kind and never
  merges them, mirroring `check_inference_attestation`. The two-part gateway
  form binds no model, so admitting it against the provider-TEE set would
  downgrade the model binding from provider-attested to body-asserted; an
  operator opts into the weaker form by setting
  `TRACE_COMMONS_ADMISSION_GATEWAY_SIGNERS`, and absent that no gateway
  receipt is admissible.

## Evidence limits

The admission route test uses real local PostgreSQL with a restricted runtime
role, real synthetic Ed25519 receipt signatures, and real EIP-191 witness
signatures. Invite registry resolution and the issuer's durable device-registration transaction
are real: first registration succeeds, same-device retry is idempotent, a second
device exceeds the invite allowance, and unknown/expired registry lookups refuse.
The subsequently authenticated token is a fixture rather than a complete native
HTTP device-enrollment ceremony. Enclave
measurement/quote and classifier are synthetic test seams.
It does not establish deployed-enclave attestation, live NEAR AI provider
qualification, or a real OpenCode model call. No provider requests or deployments
were performed. Native/adapter integration remains a subsequent checkpoint.

## Local checkpoint validation

- Reproduced baseline failures: uninvited ordinary ingest returned 200 instead
  of 403; a final-call receipt certified unrelated history; metadata retained a
  synthetic name removed from event content.
- After fixes: 121 witness tests, 244 standalone protocol tests, 1,553
  contributor tests, the admission gate's own unit tests and 4
  license-boundary tests pass. Clippy passes with the repository's existing allow-list.
- The matrix also proves an already accepted attestation does not permit a new
  ordinary upload; a byte-identical completed retry without short-lived headers
  remains a receipt read. That matrix needs an isolated PostgreSQL and is
  `#[ignore]`d, so CI does not run it: the refusal itself is decided by
  `admission::evidence_binding`, a pure function whose unit tests do run under
  `cargo test --workspace`, and `restrict_contribution` is now covered by a
  running witness test asserting that importer-supplied replay claims and cost
  figures are absent from the certified artifact.
- Final residual-secret scan refuses incomplete coverage or classifier-echoed
  secrets before certificate signing. A clean deterministic scan is not a
  guarantee that every possible PII category was removed.
