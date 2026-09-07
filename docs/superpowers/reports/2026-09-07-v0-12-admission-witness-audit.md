# v0.12 admission and witness audit

Baseline: `7d518662`. Workstream C; implementation in progress, not release evidence.

## Admission matrix and reproduced gaps

| Identity / contribution | Required authority | Current verification |
| --- | --- | --- |
| Redeemed invite, ordinary local trace | Existing authenticated invite grants, ordinary privacy and approval | Real registry resolution + synthetic authenticated tenant + ordinary ingest succeeds for two source names; full enrollment/use-count test pending |
| NEAR identity without invite, ordinary trace | Refuse | Real PostgreSQL/RLS ingest reproduced HTTP200; fixed to403 |
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
- Still required: complete native invite enrollment/use-count qualification,
  deployed signer/quote verification, real client/provider sessions and agreed
  native coverage/approval presentation. Local preview is not eligibility.
- Metadata traversal shares existing byte/node/depth budgets and refuses
  collisions or invalid typed output. Fixed schema keys are not sent to the
  classifier; contributed values and dynamic map keys are. Additional classifier
  work is bounded but its real-provider latency has not been measured.

## Evidence limits

The admission route test uses real local PostgreSQL with a restricted runtime
role, real synthetic Ed25519 receipt signatures, and real EIP-191 witness
signatures. Invite registry resolution is real, but the subsequently authenticated token is
a fixture rather than a complete native device-enrollment ceremony. Enclave
measurement/quote and classifier are synthetic test seams.
It does not establish deployed-enclave attestation, live NEAR AI provider
qualification, or a real OpenCode model call. No provider requests or deployments
were performed. Native/adapter integration remains a subsequent checkpoint.

## Local checkpoint validation

- Reproduced baseline failures: uninvited ordinary ingest returned200 instead
  of403; a final-call receipt certified unrelated history; metadata retained a
  synthetic name removed from event content.
- After fixes:122 witness tests,277 protocol privacy-filter tests,239 standalone
  protocol tests, the real PostgreSQL/RLS admission matrix, and4 license-boundary
  tests pass. Clippy passes with the repository's existing allow-list.
- The matrix also proves an already accepted attestation does not permit a new
  ordinary upload; a byte-identical completed retry without short-lived headers
  remains a receipt read.
- Final residual-secret scan refuses incomplete coverage or classifier-echoed
  secrets before certificate signing. A clean deterministic scan is not a
  guarantee that every possible PII category was removed.
