# Trace Commons System Contracts

Date: 2026-09-02
Status: proposed acceptance contract for the redesign in
[`docs/versioned-classifier-processing-design.html`](../../versioned-classifier-processing-design.html)
Scope: external APIs, user workflows, and operational workflows
Sources: `README.md`, `docs/trace-commons.md`, `docs/trace-spec.md`,
`docs/trace-commons-storage.md`, `docs/contributor-daemon-ipc-v1_1.md`,
`docs/upload-claim-issuer.md`, the operator runbooks under `docs/operator/`,
the design specs under `docs/superpowers/specs/`, and a read of the route
table and handlers in `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
and `crates/trace-commons-server/src/trace_upload_claim_issuer.rs`.

## 1. Purpose

The redesign replaces the classifier, decision, and effect model. The current
test suite is bound to the current schema and will not survive. This document
states what the system promises to the people and systems outside it. The
question each contract answers is: does the new system fulfil the promise the
old system made?

The contracts are written from user needs, not from the current code. Where
the current behaviour looks like an artifact of the old architecture, the
contract says so and states the need underneath it.

### 1.1 How to read a contract

Each contract has:

- an identifier, for example `SUB-3`;
- a tag;
- **Need**: the user need it serves;
- **Contract**: what the system must do, stated so a test can check it;
- **Check**: how to verify it against the new system;
- **Today**: where the current system implements it, when useful;
- **Note**: design commentary, when the current shape is doubtful.

Tags:

| Tag | Meaning |
|---|---|
| `KEEP` | A user need. The new system must fulfil it in the same observable way. |
| `RESHAPE` | A user need. The new system must fulfil it, but the current mechanism is not the contract. The wire shape may change if the need is still met. |
| `DROP?` | Probably an artifact of the old architecture. Keep only if a real user depends on it. Decide before migration. |

### 1.2 Parties

| Party | Who | Credential |
|---|---|---|
| Contributor | A person who runs the CLI, the daemon, or a desktop shell; or an Ironclaw workload | Device key, upload claim, account session |
| Reviewer | An operator who decides quarantined traces | Reviewer role token |
| Operator | The deployment owner | Admin role token, host access |
| Central issuer | The allowlisted principal that settles credit | Admin token whose principal hash is allowlisted |
| Customer | A frontier lab, auditor, or evaluator that reads exported data | Export or benchmark worker token plus a tenant access grant |
| Worker | An internal or external batch process | Scoped worker token |
| Public viewer | Anyone who opens the community site | None |

### 1.3 Vocabulary

- **Envelope**: one `ironclaw.trace_contribution.v1` document. The only thing
  that crosses the wire.
- **Submission**: one envelope under one `submission_id` in one tenant.
- **Status**: the contributor-visible lifecycle state of a submission.
- **Trace revision**: one stored form of a submission. A remediation or a
  re-scrub creates a new revision. This term is from the new design. The
  current system has no explicit revision, but the contracts below use it.
- **Evaluation**: one complete classifier run over one trace revision. This
  term is from the new design.
- **Effect**: an external state change that follows a decision: register,
  quarantine, insert a vector, issue credit, schedule review.

## 2. Identity and credentials

### 2.1 Onboarding

**ID-1** `KEEP` Invite-based enrollment

- **Need**: A new contributor can join a deployment with one invite and one
  locally generated key, without an account, a password, or a wallet.
- **Contract**:
  - The client generates an Ed25519 device key. The private key never leaves
    the machine. The server learns only the public key and its id
    `sha256:<hex of sha256(raw public key)>`.
  - `POST {issuer}/v1/onboard` with `trace_commons.onboard_request.v1`
    (`invite_code`, `device_public_key` as base64 of the raw 32-byte key,
    `client_info`) returns `trace_commons.onboard_response.v1` with
    `tenant_id`, `ingest_url`, `issuer_url`, `audience`, `device_key_id`, and
    optional community URLs.
  - The server stores only the hash of the invite code.
  - The response is returned once. Each success spends one use of the
    invite. The invite has a bounded use count and an optional expiry.
  - Error codes are the exact wire names of `TraceOnboardErrorCode`. An
    unknown, expired, and revoked code all return `InviteNotValid`. A caller
    cannot learn whether a guessed code was ever real.
- **Check**: Onboard twice with the same code and key. The second call spends
  a second use. Onboard with an expired code and with a random code. Both
  return `InviteNotValid` with the same status.
- **Today**: `trace_upload_claim_issuer.rs`, `onboarding.rs`,
  `docs/operator/pilot-contributor-onboarding.md`.
- **Note**: Onboarding is not idempotent. The release runbook keeps
  `contributor.json` across uninstall for this reason. A re-registration of
  an already known key should not need an invite use. Decide whether the new
  system keeps this rule.

**ID-2** `KEEP` Instance-vouched enrollment

- **Need**: An operator of a hosting instance can enroll a user without an
  invite by signing an attestation that binds a device key to a user subject.
- **Contract**: `POST {issuer}/v1/enroll` accepts
  `trace_commons.instance_enroll_request.v1`. The tenant id is derived as
  `tenant-<hex(sha256(instance_id || 0x1F || user_subject))>`. The signing
  bytes are the length-prefixed canonical form in
  `instance_enroll_attestation_signing_bytes`. A bad signature, an expired
  attestation, or a cap overflow returns `EnrollNotAuthorized`,
  `EnrollMalformed`, `EnrollRateLimited`, or `EnrollCapExceeded`.
- **Check**: Enroll with a valid grant, an expired grant, and a grant signed
  by a wrong key.

**ID-3** `RESHAPE` Self-serve invite claims gated on a wallet holding

- **Need**: A member of a partner community can claim an invite by proof of
  a wallet holding, without an operator in the loop.
- **Contract**: A challenge, a wallet signature over the challenge, an
  on-chain ownership check, and one invite grant with a rank-dependent use
  count. Refusals are truthful about the reason but do not leak other
  holders. A status endpoint reports availability. Routes are absent, not
  disabled, when the cohort is not configured.
- **Check**: Claim with a holding, without a holding, and twice with the same
  wallet.
- **Today**: `near_legion_claim.rs`, `celestine_sloth_claim.rs`, mounted
  under `/v1/onboard/{near-legion,celestine-sloths}/*`.
- **Note**: Two parallel modules exist by decision. The contract is the
  behaviour, not the module layout.

### 2.2 Upload claims

**ID-4** `KEEP` Short-lived upload claim minted from a device key

- **Need**: A contributor authenticates each upload with a short-lived token
  that the server can verify offline, so a stolen token is worth little and
  the ingest service never holds a signing key.
- **Contract**:
  - `POST {issuer}/v1/trace-upload-claim` accepts
    `ironclaw.trace_upload_claim_request.v1` under one of three auth modes:
    device key headers `x-trace-device-key-id` plus
    `x-trace-device-signature` over the exact body bytes, a device JWT, or a
    workload EdDSA JWT.
  - The response carries `access_token`, `token_type: Bearer`, `expires_at`,
    `expires_in`, and the granted `consent_scopes` and `allowed_uses`.
  - The claim is an EdDSA JWT with `kid`, `iss`, `aud`, `sub`,
    `principal_ref`, `tenant_id`, `role`, `iat`, `exp`, `jti`,
    `allowed_consent_scopes`, `allowed_uses`. The TTL ceiling is a
    configured maximum, 300 seconds by default.
  - The issuer publishes its keyset at
    `/.well-known/trace-commons-ed25519-keyset.json`. Ingest verifies and
    never mints.
  - The issuer fails closed at startup when key material is missing.
- **Check**: Mint with each auth mode. Verify the JWT with the published
  keyset. Present the claim after `exp`. Ingest returns 403.

**ID-5** `KEEP` Consent scope ceiling and clamp

- **Need**: A contributor chooses what uses to allow. An operator sets a
  ceiling per tenant. The contributor can never exceed the ceiling and can
  always see what was granted.
- **Contract**:
  - `ConsentScope` is one of `debugging_evaluation`, `benchmark_only`,
    `ranking_training`, `model_training`, `public_attribution`.
  - Each scope maps to a fixed set of allowed uses
    (`default_allowed_uses_for_scope`). `public_attribution` grants no
    content use and cannot submit traces on its own.
  - The issued claim carries `intersect(requested, ceiling)`. An empty
    request grants the full ceiling. An empty intersection is refused with
    `403 consent scopes not permitted`. The system never issues an
    empty-scope claim.
  - The granted set rides in the envelope and is visible per submission in
    status read-back.
- **Check**: Request a scope above the ceiling. Request nothing. Request only
  `public_attribution` and submit.

**ID-6** `RESHAPE` Claim verification and revocation at ingest

- **Need**: Ingest rejects bad, expired, and revoked credentials before it
  touches tenant data. An operator can cut off a device.
- **Contract**:
  - A missing or malformed `Authorization: Bearer` header is 401. An
    unknown, expired, or wrong-algorithm token is 403. Under managed-EdDSA
    mode, a stale keyset is 403 and nothing falls back to a weaker verifier.
  - Every ingest request derives tenant and principal from the credential.
    Envelope fields are never authorization inputs.
  - The operator can list and revoke device keys and tenant access grants.
    Revocation takes effect at the next claim mint at the latest.
- **Check**: Revoke a device key. The next mint fails. An in-flight claim
  works until `exp`.
- **Today**: `authenticate` in the ingest binary; `/v1/admin/device-keys`,
  `/v1/admin/tenant-access-grants/{id}/revoke`.
- **Note**: Static tenant tokens and HS256 claims are documented as pilot
  bridges. The new system does not need them. See `LEG-1`.

### 2.3 Account sessions

**ID-7** `KEEP` Account identity separate from device identity

- **Need**: A contributor can read their own content, withdraw a trace, and
  manage payout even after the submitting device is lost. A stolen device
  key must not be able to delete contribution history.
- **Contract**:
  - An account is created or reused when a device principal mints a login
    link (`POST /v1/account/login-links`, device bearer). The link is
    single-use, hashed at rest, capped at five outstanding per principal,
    and expires in five minutes.
  - Sessions come from: browser link redeem (cookie, 7 days), native
    loopback PKCE (`tcn1_` bearer, 12 hours, `S256` only, exact loopback
    redirect), passkey login, or NEAR NEP-413 login. Every native failure is
    one uniform 400.
  - The session is presented as a cookie or a bearer, never both. Both at
    once is 400. A device upload claim on an account route is 401.
  - The rotated session token is returned on every authenticated response.
  - `POST /v1/account/logout` and `POST /v1/account/sessions/revoke-all`
    exist.
- **Check**: Mint a link, redeem it twice. Present cookie and bearer
  together. Present an upload claim to an account route.

**ID-8** `KEEP` Strong authenticator gate

- **Need**: Adding or removing a passkey or NEAR key, and designating a
  payout target, must not be possible from a weak session.
- **Contract**: A session minted by passkey or NEAR is strong. A web cookie or
  device-derived session is weak. A weak session may add the first strong
  authenticator only. Otherwise the change is refused with 403 and a
  hash-only audit row.
- **Check**: Attempt each gated route from a weak session with and without an
  existing strong authenticator.

**ID-9** `KEEP` Account merge

- **Need**: A contributor with two device principals can fold them into one
  account and keep all credit.
- **Contract**: `merge/start` consumes the other device's login-link code as
  proof of control. `merge/confirm` is strong-auth gated and irreversible.
  After merge, credit and history resolve through the principal set without a
  backfill. Rejections are uniform. There is no unmerge.
- **Check**: Merge, then read credit and trace lists from the surviving
  account.

### 2.4 Operator and worker credentials

**ID-10** `RESHAPE` Role-scoped operator and worker credentials

- **Need**: Each automated job holds the narrowest credential that lets it do
  its job. A reviewer credential cannot run a worker route. A worker
  credential cannot read the review queue.
- **Contract**:
  - Roles: contributor, reviewer, admin, and scoped workers for export,
    retention, revocation, vector, benchmark, utility, process evaluation,
    and competition read.
  - Worker roles do not inherit reviewer visibility.
  - Ranking label writes bind role to label source: utility worker writes
    `frontier_lab`, reviewer writes `reviewer`, benchmark worker writes
    `benchmark`, process-evaluation worker writes `system`. Admin may
    override.
  - Access grants `(tenant, principal, role)` can only narrow a token. They
    cannot raise a role.
  - Each 403 names the accepted roles in a stable message.
- **Check**: Call every worker route with a reviewer token and every review
  route with each worker token. All are 403.
- **Note**: The exact role list follows the current pipeline layout. The new
  design has jobs and effects, so the worker roles may change. The need is
  least privilege per job kind. The DB check constraint on grant roles is
  missing `competition_read_worker` today. Do not carry that drift forward.

**ID-11** `KEEP` Issuer keyset rotation without a client change

- **Need**: An operator rotates the issuer signing key. In-flight claims stay
  valid until their expiry. Clients need no update.
- **Contract**: The keyset endpoint serves the active `kid`. Consumers cache
  it through a guarded HTTPS refresh with a host allowlist, no redirects, and
  a size cap. Under managed mode, a refresh older than the configured
  staleness bound fails closed.
- **Check**: Rotate the key. Claims signed by the old `kid` verify until
  `exp` plus the refresh window.
- **Note**: Today the issuer is single-key and rotation needs a restart
  window. Multi-key serving is a reasonable improvement, not a contract.

## 3. Trace submission

**SUB-1** `KEEP` Envelope contract

- **Need**: A client knows exactly what to send. A submission that is valid
  today stays valid tomorrow unless the schema version changes.
- **Contract**:
  - `schema_version` must equal `ironclaw.trace_contribution.v1`. A mismatch
    is `400 unsupported trace contribution schema version`.
  - Required: `trace_id`, `submission_id`, `created_at`, `ironclaw`,
    `consent`, `contributor`, `privacy`, `events`, `outcome`, `replay`,
    `value`. Defaulted: `trace_card`, `value_card`. Optional:
    `embedding_analysis`, `hindsight`, `training_dynamics`,
    `process_evaluation`.
  - `consent.revocable` must be true. `contributor.pseudonymous_contributor_id`
    must be present. `training_dynamics` fields must be in `[0, 1]`.
  - Additive optional fields do not bump the schema version. Consumers
    ignore unknown fields.
  - The maximum envelope is 16,000,000 bytes. The ingest body cap is at least
    that plus framing headroom. Over-size is 413.
  - The crate `trace-commons-protocol` is the source of truth for the shape.
- **Check**: A golden set of valid envelopes and one invalid envelope per
  rule above.

**SUB-2** `KEEP` The upload is untrusted

- **Need**: A contributor's privacy does not depend on the client being
  correct. The server re-scrubs and re-derives every risk field.
- **Contract**:
  - The server re-runs deterministic redaction and appends its version
    suffix to `privacy.redaction_pipeline_version`.
  - The server recomputes `redaction_hash` and `residual_pii_risk`.
  - The server corrects under-reported `message_text_included` and
    `tool_payloads_included` upward. It never corrects downward. A bare
    `tool_name` is metadata and does not trigger correction.
  - The server normalizes `contributor.tenant_scope_ref` to the
    auth-derived tenant.
  - Tenant, principal, and partition come from the credential only.
- **Check**: Submit an envelope that declares no message text but carries
  it. Submit an envelope with a forged tenant reference. Read back the
  stored record.

**SUB-3** `KEEP` Submission response and status vocabulary

- **Need**: A contributor learns at once whether the trace was filed, held,
  or refused, and what credit is pending.
- **Contract**:
  - `POST /v1/traces` returns `TraceSubmissionReceipt`: `status`,
    optional `credit_points_pending`, optional `credit_points_final`,
    `explanation[]`.
  - Statuses: `accepted`, `quarantined`, `awaiting_pii_backstop`,
    `rejected`, `revoked`, `expired`, `purged`. The set may grow. A status
    may not change meaning.
  - A non-accepted status has pending credit zero and an explanation that
    says why.
- **Check**: Submit low, medium, and high residual-risk envelopes. Compare
  status and explanation with the golden receipt.
- **Note**: Today the submit-time status is a function of residual PII risk
  only. The gates run later and change credit, not status. `trace-spec.md`
  says the opposite. See `EVAL-2` for the contract the new system should
  meet.

**SUB-4** `KEEP` Idempotency and quarantine remediation on one id

- **Need**: A retry never creates a second record. A contributor can fix a
  quarantined submission without competing with their own earlier copy for
  novelty.
- **Contract**:
  - `submission_id` is the idempotency key. The client derives it from the
    session content so the same session maps to the same id.
  - Same id, different principal: `409 submission id already belongs to
    another principal`.
  - Same id, same owner, status in `accepted`, `rejected`, `revoked`: the
    stored receipt is returned unchanged. An `idempotent_submit` audit row is
    written.
  - Same id, same owner, status `quarantined`: the new envelope supersedes
    the stored one. The server re-scrubs, reclassifies under current rules,
    clears the review lease, keeps the original `received_at` and principal,
    and does not spend a new quota slot. The audit action is
    `quarantine_remediated`.
  - There is no idempotency header. Do not add one.
- **Check**: Submit, retry, remediate, and retry again. Count records and
  audit rows.
- **Note**: In the new design, a remediation is a new trace revision and a
  new evaluation. The old evaluation stays immutable. The contributor still
  sees one submission id.

**SUB-5** `KEEP` Re-ingest of revoked content is refused

- **Need**: Once a contributor withdraws content, the same content cannot be
  filed again under a new id, by anyone in the tenant.
- **Contract**: A tombstone keyed on `submission_id`, `redaction_hash`, and
  `canonical_summary_hash` blocks re-ingest with `409 trace content was
  previously revoked for this tenant`. Tombstones outlive content deletion.
- **Check**: Submit, revoke, submit the same content under a new id.

**SUB-6** `KEEP` Rate limits and quotas

- **Need**: One principal cannot flood a tenant. A retry of an existing id
  never counts against a quota.
- **Contract**:
  - Per-principal request limit and concurrency limit on submit. Both return
    `429 rate limited`.
  - Optional hourly quotas per tenant and per principal, counting live
    accepted and quarantined submissions. Revoked, expired, and purged rows
    stop counting. Idempotent retries never count. Messages are stable.
- **Check**: Submit past each limit. Retry an existing id at the limit.
- **Today**: 30 per minute and 2 concurrent per principal, single-instance
  in-process limiter.
- **Note**: The limiter is per process. Behind a load balancer the effective
  limit multiplies. The contract is the bound, not the mechanism.

**SUB-7** `KEEP` Error body shape

- **Need**: Clients can act on failures with fixed labels and never see
  internal detail.
- **Contract**:
  - Every error is `{"error": "<stable message>"}`.
  - 500 is always the opaque `trace commons operation failed`. The server
    logs a salted hash, never the cause.
  - 503 names a missing control with a stable label, for example
    `attestation_signing_key_unconfigured` or
    `revocation refused: missing control trace_artifact_store_unconfigured`.
  - Messages are stable across releases. A change is a breaking change.
- **Check**: Snapshot test of every documented message.
- **Note**: There is no machine-readable code field. Adding one is
  compatible. Removing the `error` string is not.

**SUB-8** `KEEP` Contributor client outcome labels

- **Need**: A collector or shell can automate on the CLI result without
  parsing prose.
- **Contract**: `trace_commons.submit_result.v1` per session with `outcome`
  in `submitted`, `already-submitted`, `refused`, `failed`, `skipped` and a
  fixed `reason` label. Exit code is nonzero when anything was refused or
  failed. `already-submitted` is success. Sessions are independent. One
  failure never aborts a batch except the once-per-batch privacy-filter
  canary. Transport errors retry three times. A 401 or 403 triggers exactly
  one claim re-mint.
- **Check**: Golden runs of the CLI against a mock server for each label.
- **Today**: `docs/collector-integration.md`, `submit.rs`.

**SUB-9** `KEEP` Local-first privacy invariants

- **Need**: Raw sessions never leave the machine. Redaction is fail-closed.
- **Contract**:
  - Contribution is off by default.
  - Only envelopes cross the wire.
  - A session with a residual key-shaped secret after redaction is refused,
    not uploaded.
  - An optional remote PII filter receives only already-redacted message
    text, never structured payloads. If requested and unavailable, the batch
    is refused.
  - A synthetic canary runs through the active redactor once per batch. If
    it survives, the batch aborts.
  - Full local paths never appear in an envelope.
- **Check**: The existing canary and redaction audit harnesses become golden
  tests in CI.
- **Note**: These are client contracts. They survive a server redesign
  unchanged.

## 4. Trace evaluation

This section states what users need from the novelty, substance, and
privacy classification. The current schema binds these to specific columns.
The contracts below are written against outcomes, so the new evaluation
model can satisfy them.

**EVAL-1** `KEEP` Every filed trace is evaluated

- **Need**: A contributor's accepted trace is evaluated for novelty and
  substance. A customer never receives an unevaluated trace.
- **Contract**:
  - Every submission that reaches `accepted` has at least one durable
    evaluation record.
  - An accepted submission with no evaluation is an operator alert.
  - Evaluation is asynchronous. The contributor sees a bounded pending state
    and then a result.
- **Check**: Submit, wait, and query the evaluation for the submission.

**EVAL-2** `RESHAPE` Privacy risk decides quarantine; gates decide credit

- **Need**: A privacy-risky trace is held for a human before anyone else can
  read it. A dull or duplicate trace is not paid, but is not treated as a
  privacy problem.
- **Contract**:
  - `residual_pii_risk` is derived by the server: an unredactable finding,
    a post-scrub residual hit, or an incomplete residual scan forces `high`.
    A successfully redacted secret, any other redaction finding, or included
    message text or tool payloads yields `medium`. Otherwise `low`.
  - `high` quarantines. `medium` quarantines unless the operator enables
    medium acceptance. `low` is filed.
  - With the PII backstop enabled, a low-risk trace that carries message
    text is held in `awaiting_pii_backstop` until the backstop succeeds. No
    timeout releases it.
  - A quarantined trace earns zero pending credit until a reviewer decides.
  - The novelty and substance decisions affect credit and derived use. They
    do not change the privacy status.
  - Both gates must pass for utility credit. A gate that cannot run fails
    closed. It never passes by default.
- **Check**: Submit each risk tier. Confirm status. Disable the scorer and
  confirm no gate passes.
- **Note**: The current split is an outcome of history. It is still the right
  split. The new design's policy should keep privacy disposition and
  commons disposition as separate outputs of one decision.

**EVAL-3** `KEEP` Fail closed on missing evidence

- **Need**: A missing measurement never reads as a favorable one.
- **Contract**:
  - A scorer error refuses the evaluation. The submission is not credited.
  - A zero floor still passes a zero score, so the service refuses to start
    when every gate floor is zero.
  - A missing observation is typed as missing. A policy that requires it
    returns a closed decision or a processing error.
- **Check**: Run the gate with the scorer unreachable. Start the service with
  all floors at zero.
- **Today**: Issue #206 shows a failed replay sample coerced to 0.0 and a
  passing determinism gate. The new design's invariant 13 states this rule.

**EVAL-4** `RESHAPE` Complete provenance for every decision

- **Need**: An operator, an auditor, or a contributor can ask why a trace was
  accepted, rejected, or credited, and get a complete answer that names the
  exact inputs and versions.
- **Contract**:
  - Every decision names the classifier configuration that produced it as
    one identifier that closes over the canonical text format, the model,
    the configuration, the calibration, and the decision rule.
  - Every decision records the measurements it used, not only the labels.
  - Every novelty decision records the index state it compared against.
  - Every credit event names the decision it came from.
  - Changing a floor, a model, or the canonical renderer changes the
    identifier.
  - Old decisions are never rewritten. A re-evaluation is a new record.
- **Check**: For a sample of credited traces, walk from the credit event to
  the decision, the measurements, the inputs, and the classifier identity.
- **Today**: `trace_gate_decisions` with `gate_version_hash`. The hash omits
  the renderer. Measurements are partial. See the report
  `2026-08-24-version-the-classifiers.md`.
- **Note**: This is the core need behind the redesign. The new design's
  provenance path is the intended implementation.

**EVAL-5** `KEEP` Determinism and reproducibility

- **Need**: A second run of the same classifier over the same input gives the
  same answer, so a dispute can be re-checked.
- **Contract**: Given the same trace revision, the same classifier identity,
  and the same reference index state, the measurements and decision are
  equal. Nondeterministic scorers are wrapped in a checkpoint that records
  the observation once.
- **Check**: Replay a sample of evaluations and diff.

**EVAL-6** `RESHAPE` Shadow evaluation and safe promotion

- **Need**: An operator can trial a new classifier on live traffic without
  paying credit from it, compare it with the active one, and promote or roll
  back with a record.
- **Contract**:
  - More than one classifier configuration can evaluate the same traffic.
    Only the active one creates effects.
  - Activation and rollback are explicit operator actions with audit rows.
    Timestamp order never selects production behaviour.
  - A divergence report between shadow and active is available before
    promotion.
- **Check**: Run a shadow. Confirm no credit or vector insert comes from it.
  Promote. Confirm the next evaluation uses the new identity.
- **Today**: Credit quality and dedup penalty are persisted shadow scores.
  There is no shadow classifier. Model swap is an env change plus restart.

**EVAL-7** `RESHAPE` Reprocessing under a new policy or classifier

- **Need**: After a rule change or a model swap, an operator can re-evaluate
  stored traces without a new upload and without losing the old record.
- **Contract**:
  - Reprocessing creates a new evaluation for an existing trace revision.
  - Reprocessing is bounded, resumable, and idempotent.
  - Reprocessing never changes settled credit.
  - A policy-only change can re-run over stored measurements without a
    scorer.
- **Check**: Reprocess a batch twice. Count evaluations. Confirm settled
  credit is unchanged.
- **Today**: `/v1/admin/rescore-perplexity`, `/score-credit-quality`,
  `/recluster-dedup`, `/recompute-contributor-caps`, and the reviewer
  re-scrub routes. Each updates columns in place.
- **Note**: The four column-specific admin routes are artifacts. The need is
  one reprocess operation with a selector and a target classifier. See
  `LEG-6`.

**EVAL-8** `KEEP` Explicit calibration evidence

- **Need**: The operator can show what evidence justified the active
  thresholds.
- **Contract**:
  - A calibration corpus is a versioned artifact with a digest.
  - A calibration report records per-trace scores and operating-point pass
    rates at the chosen threshold, not only an aggregate AUC.
  - A corpus is admissible only when no-model baselines fail to separate the
    classes.
  - The active classifier identity links to its calibration report.
- **Check**: For the active classifier, retrieve the corpus digest and the
  report.
- **Today**: `trace-commons-gate-calibrate`, frozen JSON under `docs/`.

**EVAL-9** `RESHAPE` Trust evidence for the scoring backend

- **Need**: Credit is only issued from a scoring backend the operator trusts,
  and the record shows the evidence.
- **Contract**: Each evaluation records the scoring boundary it ran under. A
  deployment can require verified evidence for credit and deny with a stable
  label when evidence is missing, expired, or not allowlisted.
- **Today**: `is_production_gate_service_kind` is a name match on a list that
  excludes the backend the pilot runs. The gate-trust-evidence spec of
  2026-07-29 is unimplemented.
- **Note**: The current behaviour is not the contract. The spec is.

## 5. Contributor observability

**OBS-1** `KEEP` Batch status by owned submission ids

- **Need**: A client refreshes the status and credit of what it submitted,
  with no enumeration of other people's traces.
- **Contract**:
  - `POST /v1/contributors/me/submission-status` with `submission_ids[]`,
    capped at 500 ids (413 above).
  - Returns `TraceSubmissionStatusUpdate` per visible id: `submission_id`,
    `trace_id`, `status`, `credit_points_pending`, `credit_points_final?`,
    `credit_points_ledger`, `credit_points_total?`, `explanation[]`,
    `delayed_credit_explanations[]`, `consent_scopes[]`.
  - Unknown, cross-tenant, and cross-principal ids are omitted, not errored.
  - `credit_points_total` is final credit plus the delayed ledger delta. A
    revoked, expired, or purged trace reports a zero delayed ledger and a
    safe explanation.
- **Check**: Query own ids, another principal's ids, and random ids in one
  call.

**OBS-2** `KEEP` Credit summary and event ledger

- **Need**: A contributor sees pending, final, ledger, settled, reversed, and
  held credit, and the events behind them.
- **Contract**: `GET /v1/contributors/me/credit` returns per-status counts
  and the credit fields listed above. `GET /v1/contributors/me/credit-events`
  returns the events. Both resolve through the account's principal set, so a
  merged account sees all history.
- **Check**: Compare the summary with the sum of events.

**OBS-3** `KEEP` Signed score attestation

- **Need**: A contributor or a collector can obtain a statement of their
  scores that a third party can verify offline.
- **Contract**:
  - `GET /v1/contributors/me/score-attestation` takes no body and no query
    parameter. The principal comes from the credential only.
  - The response is a compact JWS whose payload is
    `trace_commons.score_attestation.v1`: tenant, principal, up to 500
    submissions with `credit_quality_micros`, `perplexity_micros`,
    `novelty_score_micros`, `gate_passed`, plus `issued_at`, `expires_at`,
    and a nonce. TTL is configured, 24 hours by default.
  - The keyset is at `/.well-known/trace-commons-attestation-keyset.json`.
    Verifiers select by `kid`.
  - When signing is unconfigured, both routes return 503 with
    `attestation_signing_key_unconfigured`. The system never returns an
    unsigned attestation and never an empty keyset.
- **Check**: Fetch, verify with the keyset, fetch with signing unconfigured.
- **Note**: The attested fields are the current classifier's outputs. The new
  system may attest different measurements. The attestation must then carry
  a schema identifier and the classifier identity. Keep the signing and
  fail-closed rules.

**OBS-4** `KEEP` Account trace list, detail, and content read-back

- **Need**: A contributor can see every trace they filed and read back the
  stored redacted copy.
- **Contract**:
  - `GET /v1/account/traces` with keyset pagination: `limit` 1 to 200,
    default 50; opaque `cursor`; a bad cursor is 400, never a silent reset.
  - Items carry status, privacy risk, score, credit, consent scopes,
    redaction counts, `received_at`, event count, coverage tags, tool
    sequence, tool categories, duplicate and novelty scores.
  - `GET /v1/account/traces/{id}` and `/{id}/content` return
    `404 trace not found` for both unknown and unowned ids, byte-identical.
  - Content is the stored, permanently redacted envelope. There is no
    unscrubbed read-back. A decrypt or integrity failure is a fail-closed
    500 with no detail.
  - Content reads are rate-limited per account.
  - Empty results are still audited.
- **Check**: List, page, read own content, read another account's id.

**OBS-5** `KEEP` Contributor daemon status and queue semantics

- **Need**: A shell application shows a contributor what is pending, what
  was sent, and why something is stuck, with one clear health label.
- **Contract**: The IPC contract `trace_commons.daemon.v1_1`: queue states
  `pending`, `approved`, `uploading`, `uploaded`, `refused`, `failed`,
  `expired`, `superseded`; the 14-day expiry clock pauses while unhealthy;
  an approval covers content and terms, so any change re-offers the entry;
  one health label at a time with the documented precedence; error codes
  are fixed labels.
- **Check**: The daemon's own conformance tests.
- **Note**: This is a client-side contract. It depends on the server only
  through the routes in this document.

## 6. Credit

**CR-1** `KEEP` Credit is a record, not currency

- **Need**: A contributor is told the truth: acceptance is not payment.
- **Contract**:
  - Acceptance creates a pending estimate. It is not settled.
  - Delayed utility events append to an append-only ledger only through
    privileged, audited paths bound to a downstream artifact or a review
    decision.
  - Settlement is an explicit, governed step by an allowlisted central
    issuer.
  - No route transfers, sells, or withdraws credit.
- **Check**: Every credit write path requires a privileged role. Trace each
  positive delta to an artifact or a decision reference.

**CR-2** `RESHAPE` Deterministic and explainable online estimate

- **Need**: A contributor can see why a submission scored as it did.
- **Contract**: The estimate is a pure function of the envelope and the
  server's derived measurements, versioned, with a per-axis scorecard and a
  human-readable explanation. Privacy risk is charged once. A high-risk
  submission estimates zero.
- **Check**: Golden envelopes with pinned scorecards.
- **Today**: `compute_value_scorecard` in the protocol crate.
- **Note**: The formula is a policy constant. It belongs in the versioned
  policy bundle, not in the wire crate.

**CR-3** `KEEP` Delayed utility event types and authority

- **Need**: Only the party that produced the utility can claim it.
- **Contract**:
  - Event types: `benchmark_conversion`, `regression_catch`,
    `training_utility`, `ranking_utility`, `reviewer_bonus`,
    `abuse_penalty`, `novelty_utility`.
  - `novelty_utility` is emitted only by the gate. No operator route can
    mint it.
  - The utility worker route accepts only `regression_catch`,
    `training_utility`, `ranking_utility`. Reviewer routes own
    `reviewer_bonus` and `abuse_penalty`.
  - A `ranking_utility` event is credit-bearing only when it binds to one
    prediction of the active model.
  - A per-event delta cap exists.
  - Every event is idempotent on its external reference.
- **Check**: Attempt each type through each route.

**CR-4** `KEEP` Settlement governance

- **Need**: The operator cannot mint credit to arbitrary accounts in the dark.
- **Contract**:
  - A dry run yields a canonical `source_list_hash`.
  - A listed principal records an approval for that exact list, policy
    version, and evidence hash.
  - A live run selects only unused, positive, eligible delayed events on
    still-accepted submissions, excludes held accounts, applies the
    per-account cap, and requires the same limit as the approval.
  - Unlisted principals can dry-run but cannot approve, finalize, or write
    positive credit through any path.
  - Payout resolution is fail-closed: zero NEAR identities or an ambiguous
    set holds the credit; exactly one or one designated settles.
  - A held credit emits no outbox row. Release is idempotent.
- **Check**: The settlement drill plus a live run against a fixture.

**CR-5** `KEEP` Revocation does not claw back settled credit

- **Need**: Withdrawing a trace is a right, not an offence.
- **Contract**:
  - Withdrawal and revocation leave settled credit untouched.
  - The reversal action is operator-raised only and audited.
  - Delayed events not yet settled on a revoked source are excluded from
    settlement.
- **Check**: Settle, withdraw, read credit.
- **Note**: The propagation worker today appends a compensating negative
  ledger row for delayed events on a revoked source. That is bookkeeping for
  unsettled events. Do not extend it to settled ones.

**CR-6** `KEEP` NEAR receipts are a mirror

- **Need**: A public, hash-only receipt exists for settled credit. The server
  ledger stays authoritative.
- **Contract**: Outbox rows carry hashes only. Submit records a public
  transaction hash. Confirm binds to it. Freeze and unfreeze follow account
  holds. Nothing on chain transfers value.
- **Check**: Inspect outbox rows for raw identifiers. None appear.

**CR-7** `RESHAPE` Caps and quality multipliers

- **Need**: One contributor cannot dominate credit by volume. Near-duplicates
  earn less. Anomalous scores earn nothing.
- **Contract**: A quality term, a duplicate penalty, and a concave
  per-contributor cap per epoch multiply the raw utility before settlement.
  Each is versioned. Each is a pure function of stored measurements.
- **Today**: All three are shadow-only columns on the gate decision row.
- **Note**: Fold these into the policy bundle. They are decisions, not
  observations.

## 7. Consent, withdrawal, and revocation

**CON-1** `KEEP` Withdrawal by account with honest reach reporting

- **Need**: A contributor can take a trace back, learn exactly what that
  achieved, and keep their credit.
- **Contract**:
  - `POST /v1/account/traces/{id}/withdraw` on an account session, not a
    device key.
  - Ownership is checked before any mutation. Unknown and unowned collapse
    to `404 trace not found`.
  - The response carries `distribution_reach` in `not_distributed`,
    `commons_not_distributed`, `commons_distributed`, plus `prior_status`,
    `withdrawn_at`, `already_distributed`, and `credit_retained: true`.
  - Reach is computed on the server during the call from live export
    membership. An existing tombstone is authoritative on retry.
  - Idempotent. Tombstone and status first, bytes second. Any deletion or
    eviction failure is a fail-closed 500 and the call is not reported
    complete.
  - Before the response: vector entries invalidated in the index and in
    metadata, dedup cluster membership cleared, derived artifacts and export
    manifests invalidated, stored objects deleted under every possible key.
  - The tombstone is hash-only and has no foreign key to the submission, so
    it outlives a hard delete.
- **Check**: Withdraw at each tier. Retry. Query the vector index and export
  manifests.
- **Note**: The tier copy in the daemon IPC document is binding on clients.
  The server must keep the three wire names.

**CON-2** `RESHAPE` Revocation by device credential

- **Need**: A client without an account session can still stop use of a
  trace.
- **Contract**:
  - `POST /v1/traces/{id}/revoke` and the `DELETE` forms on a device
    credential. Owner or reviewer.
  - A privileged revocation requires a non-empty reason. A blank reason is
    400.
  - Status becomes `revoked`. A tombstone with the content hashes is
    written. Derived use stops. Propagation to derived artifacts is
    asynchronous and eventually consistent.
  - If an artifact store is configured as required but absent, the request
    is refused with a missing-control label rather than leaving orphaned
    payload.
- **Check**: Revoke, then export. The trace is absent.
- **Note**: Revoke does not delete bytes. Withdraw does. Two verbs with
  different guarantees confuse contributors. Consider one withdrawal
  operation with the same reach reporting under both credentials, with the
  device path limited to the submitting device's own traces.

**CON-3** `KEEP` Revocation propagation is observable and bounded

- **Need**: An operator knows when a revocation did not fully propagate.
- **Contract**: Each propagation target has a bounded attempt count. An
  exhausted target goes terminal and appears as a per-kind counter in the
  operational summary. A terminal count above zero for more than an hour is
  an incident.
- **Check**: Break the vector backend, revoke, read the summary.
- **Note**: Today a `legacy_deterministic` gate service marks a vector
  invalidation as done without doing anything. The new effect model must
  not report a receipt for a no-op.

**CON-4** `KEEP` Consent scope changes are forward-only

- **Need**: A contributor can change scopes for future submissions and see the
  scopes of each past submission.
- **Contract**: A scope change applies to future envelopes. Past submissions
  keep their granted set, visible in read-back. The only downgrade on a past
  submission is withdrawal.
- **Note**: Retroactive scope changes are a real user need and are deferred.
  The new design's trace revision is a natural place to record one.

## 8. Trace lifecycle management

**LC-1** `KEEP` Retention derives from consent, not from the envelope

- **Need**: A trace does not outlive the use it was contributed for. A client
  cannot extend its own retention.
- **Contract**:
  - The server derives the retention class from the strongest allowed use.
    Debugging and evaluation: 730 days, revocable. Benchmark and training:
    1095 days, revocable. Aggregate-only: no expiry, not revocable, no
    derived artifacts.
  - `expires_at` is stamped once at receipt and preserved across
    remediation.
  - Review, export, and conversion cross-check the stored policy id and
    `expires_at` against the derived class and fail closed on mismatch.
- **Check**: Submit under each scope. Read `expires_at`. Attempt to extend
  it through file metadata.

**LC-2** `KEEP` Expire and purge with legal hold

- **Need**: Expired traces stop being used at once and are physically
  removed later, unless held.
- **Contract**:
  - A maintenance run moves expired traces to `expired` and pins final
    credit. Derived records follow.
  - Purge requires an explicit `purge_expired_before` cutoff and a
    non-empty purpose. A dry run never deletes.
  - Purge deletes stored objects, marks `purged`, and records the deleted
    targets.
  - Legal hold exempts a retention policy class from expire and purge.
  - Retention jobs are resumable, idempotent, and re-verify tenant,
    consent, status, and hold immediately before each destructive action.
  - Job and item ledgers exist with per-item action, status, and reason.
- **Check**: The retention dry-run drill plus a purge against a fixture with
  one held policy.
- **Note**: Hold is per policy class only. Per-submission and per-tenant
  holds are missing. Hold does not cover the revocation path. Decide whether
  the new system adds them.

**LC-3** `KEEP` Tombstones prevent re-ingest and re-export

- **Need**: Deleted content does not come back through a side door.
- **Contract**: Revocation, expiry, purge, and withdrawal each leave a
  tombstone with content hashes and a retention window longer than any
  cache. Export and ingest consult it.
- **Check**: Purge, then submit the same content and export the range.

**LC-4** `RESHAPE` PII backstop hold

- **Need**: A deployment can require a second prose-PII pass before any
  message-text trace is filed.
- **Contract**: When enabled, a low-risk message-text trace is held in
  `awaiting_pii_backstop`, not reviewer-eligible and not exported. The
  driver re-redacts residuals, writes a new envelope revision, invalidates
  the old reference, and releases to `accepted` or `quarantined`. Failure
  holds the row. Boot refuses when the backstop is enabled without its
  dependencies. Readers must prefer the rescrubbed revision.
- **Note**: In the new design this is a classifier role plus an effect. The
  hold state and the read-preference rule are the contract.

## 9. Review and quarantine

**REV-1** `KEEP` Review queue, leases, and decisions

- **Need**: A reviewer works quarantined traces without two reviewers
  colliding, and every decision carries a reason.
- **Contract**:
  - `GET /v1/review/quarantine`, `/active-learning`, `/routing-summary`
    list only live quarantined traces in the reviewer's tenant.
  - Leases are tenant-scoped, bound to the principal, TTL default 1800
    seconds and max 86400, reclaimable by the same principal or after
    expiry, and cleared when a trace leaves quarantine. Batch claims are
    capped at 50.
  - A decision is `approve` or `reject` with a required reason. Batch
    decisions are capped at 50 ids.
  - Decisions apply only to live quarantined traces. Accepted, rejected,
    revoked, expired, and purged rows are refused before any body read.
  - Approval of an aggregate-only retention class is refused.
  - Approve sets `accepted` and pending credit. Reject sets `rejected` and
    zero credit. Approval does not change the recorded privacy risk.
  - Body reads go through object references and are audited per read.
- **Check**: Two reviewers claim the same trace. Decide a non-quarantined
  trace. Decide with an empty reason.

**REV-2** `KEEP` Re-scrub in place

- **Need**: After a rule change, an operator can reclassify held traces
  without asking contributors to re-upload.
- **Contract**: `POST /v1/review/{id}/rescrub` and the bounded batch
  `POST /v1/review/quarantine/rescrub` with dry run. The content-addressed
  identity is kept. The result is a new revision and a new classification.
- **Note**: In the new design this is a reprocess job over a new trace
  revision.

**REV-3** `KEEP` Reviewer credit adjustments

- **Contract**: `POST /v1/review/{id}/credit-events` appends
  `reviewer_bonus` or `abuse_penalty` with a reason and an external
  reference. Positive writes require the central-issuer allowlist when it
  is configured. Penalties stay available.

## 10. Data consumption

**DC-1** `KEEP` Customers receive a filtered projection under an
intersection of consents

- **Need**: A customer gets exactly the traces they are allowed to use and
  nothing they are not. A contributor's consent bounds every read.
- **Contract**:
  - A trace is exported only when: status is `accepted`; residual risk is
    `low`; the contributor's consent grants the requested allowed use; the
    customer's token and tenant access grant carry that use.
  - Failing traces are skipped silently. The export is fail-closed.
  - The effective permission is the intersection. A grant only narrows.
  - `aggregate_analytics` never permits a per-trace artifact, including a
    vector entry.
  - Replay export requires `evaluation`. Benchmark conversion requires
    `benchmark_generation`. Ranker export requires
    `ranking_model_training`.
  - Export guardrails, when required, demand explicit low-risk, accepted,
    consent-scoped filters and a caller purpose.
- **Check**: A fixture tenant with one trace per scope and risk tier. Export
  under each use and compare the id set.

**DC-2** `KEEP` Exports are bounded, granted, manifested, and audited

- **Need**: An auditor can see which traces fed which dataset without seeing
  content.
- **Contract**:
  - Each interactive export mints a one-shot grant with a dataset kind, a
    purpose, an item cap, and an expiry.
  - Export jobs are queued, claimable, retryable with bounded backoff, and
    recoverable when stale.
  - Every export writes a manifest with item snapshots and a deterministic
    hash of the source list, mirrored into the audit row.
  - A per-request item cap clamps the export size. A list `limit` outside
    the documented range is rejected, not clamped.
  - Revocation and expiry invalidate manifests and manifest items.
- **Check**: Export, read the manifest, revoke one source, re-read.

**DC-3** `KEEP` Derived artifacts stay revocable and provenance-bound

- **Need**: A benchmark or ranker dataset can be traced to its sources and
  pulled when a source is revoked.
- **Contract**: Conversion records consent scope, review state, redaction
  version, replay requirements, and a manifest id. It fails closed on
  revoked, expired, unapproved, or non-replayable sources. Publication
  requires a passed evaluator result. Registry publish and revoke go through
  a hash-only outbox.

**DC-4** `KEEP` Aggregate analytics with a privacy budget

- **Need**: An operator can publish counts without exposing small cells.
- **Contract**: Cells under the minimum count are suppressed. Broad release
  requires a configured noise key, an epsilon charge, and an unexhausted
  cap, else it fails closed with a named reason. The response carries a
  `privacy_budget` block.

## 11. Community surface

**COM-1** `KEEP` Public attribution is a separate opt-in

- **Need**: A contributor appears by name only when they asked to, and can
  leave at any time.
- **Contract**:
  - `PUT /v1/community/profile` needs an upload claim carrying
    `public_attribution`. Setting a handle replaces the whole profile.
  - Handles are 3 to 32 characters, URL-safe, unique, and not on the
    reserved list. Bio is at most 280 bytes.
  - `DELETE /v1/community/profile` removes the contributor from the roster.
  - Withdrawal of the profile evicts the handle from every served snapshot
    within the published freshness bound. Serving a withdrawn handle is a
    refusal, never a stale answer.
- **Check**: Opt in, read the leaderboard, opt out, read again.

**COM-2** `KEEP` Public reads are gated by publication controls

- **Need**: Nothing public leaks below the k-anonymity floor or before noise
  is real.
- **Contract**:
  - `GET /v1/community/leaderboard`, `/contributors/{handle}`,
    `/analytics/summary` are unauthenticated and CORS-limited.
  - The roster gate is the minimum cell count. The analytics gate adds an
    approved noise mechanism and a minimum tenant cohort.
  - Withheld aggregates are returned as null with the reason named. Missing
    controls are label-only names, never counts.
  - A snapshot with no privacy block is refused, never grandfathered.
  - When the surface is disabled, all routes are 404.
- **Note**: The approved noise list is empty by construction, so analytics
  is always withheld today. That is a product decision, not a bug.

## 12. Operator workflows

**OP-1** `KEEP` Fail-closed configuration

- **Need**: A misconfigured deployment refuses to start rather than run with
  a weaker control.
- **Contract**:
  - Each `TRACE_COMMONS_*_REQUIRE_*` control refuses startup or the
    affected route when its dependency is absent, and names the missing
    control with a stable label.
  - No control falls back to plaintext, to a less-restricted backend, or to
    the runtime pool.
  - Gate floors and policy version have no implicit zero defaults when a
    real gate service is configured.
  - Attestation signing is all-or-nothing.
- **Check**: A startup matrix: each required control with its dependency
  absent.

**OP-2** `KEEP` Health and safe status surfaces

- **Need**: An operator can ask what is deployed and whether it is ready
  without SSH, and alerting can bind to stable field paths.
- **Contract**:
  - `GET /health` is unauthenticated and returns `status`,
    `schema_version`, `build_commit`, `build_time`, `build_version`.
  - `GET /v1/admin/config-status` reports readiness booleans, counts, key
    aggregates, and label lists. Never key material, URLs, tokens, host
    lists, or raw reasons.
  - `GET /v1/admin/operational-summary` and `/operational-metrics` have a
    documented field contract. A rename is a breaking change.
  - The issuer `GET /health` returns 200 `ok` or 503 `degraded` with named
    checks.
- **Check**: Snapshot test of field paths.

**OP-3** `RESHAPE` Drills and rollout smoke

- **Need**: Before promotion, the operator proves each risky path works on
  this deployment, with hash-only evidence.
- **Contract**:
  - Each risky pipeline has an idempotent drill that returns pass or fail
    and writes an audit row with a `sha256:` evidence hash.
  - A preflight aggregates required checks with a maximum evidence age of
    24 hours and reports recorded, passed, failed, stale, and missing
    separately.
  - Promotion and live settlement can be made to require a ready preflight.
- **Check**: Run every drill on a fresh deployment. Run the preflight.
- **Today**: 15 drill routes and 23 required checks with partial overlap.
  Adding one needs four coordinated edits.
- **Note**: Several checks prove file-versus-database parity for a cutover
  that is complete. See `LEG-3`. The need is a registry of required
  proofs, one place to add one, and evidence with an age.

**OP-4** `KEEP` Audit chain and forensics

- **Need**: Every read or mutation of trace content, credit, review, export,
  or revocation state leaves a tamper-evident, hash-only row that an
  auditor can query without seeing content.
- **Contract**:
  - Audit rows are append-only and hash-chained. Corrections are new rows.
  - Rows carry actor hash, role, action, reason where privileged, a decision
    inputs hash, and a typed safe-metadata projection. Backends reject
    unallowlisted keys.
  - Zero-result reads are audited.
  - A chain drill reproduces the hash computation. Any unexplained drift is
    a P0.
  - The named forensic questions in `audit-trail-forensics.md` remain
    answerable: why credit was minted, which classifier evaluated, what an
    operator did in a window, which revocations are stuck, and how to
    rebuild the vector index from the audit trail.
- **Check**: Run each forensic query against the new schema.

**OP-5** `KEEP` Hash-only logging

- **Contract**: Error classes are stable labels. No log or stored row carries
  raw URLs, bearer tokens, account references, transaction hashes,
  contributor identity, or trace bodies. The class list in
  `hash-only-logging.md` is the baseline. New classes follow
  `<Action>Failed`.
- **Check**: The grep in the runbook plus a canary-secret log scan.

**OP-6** `KEEP` Tenant isolation in the database

- **Contract**: Every tenant table carries `tenant_id`, has forced row-level
  security, and uses the `trace_current_tenant_id()` predicate. The serving
  role cannot bypass RLS. Narrow cross-tenant reader roles exist only for
  named drivers with column-scoped grants and role-scoped policies. An
  unprovisioned reader role fails closed. Cross-tenant tests seed the same
  ids in two tenants and prove no path crosses.
- **Check**: The RLS drill plus the cross-tenant test set.

**OP-7** `KEEP` Key management and rotation

- **Contract**:
  - Per-object data keys are wrapped by a KEK behind a trait with a
    production trust boundary flag. A local wrapper is dev-only and refused
    in production.
  - Context binding is doubled so a DEK cannot be moved between objects.
  - KEK rotation is zero-downtime when the new version is staged while the
    old one can still decrypt. The key-rotation drill proves wrap and
    unwrap. The old version is disabled only after the claim TTL plus the
    refresh window.
  - Loss of the KEK is loss of the artifacts. The runbook says so.
- **Check**: The rotation drill after a staged rotation.

**OP-8** `KEEP` Backup and restore

- **Contract**: PostgreSQL has snapshots and point-in-time recovery. Object
  storage has versioning and soft delete. The audit-chain drill validates a
  restore. The vector index is rebuildable from the audit trail and stored
  envelopes, and the rebuild tool is read-only at the audit and credit
  layer.
- **Note**: The vector index has no remote backup today. In the new design,
  an index epoch with a manifest makes rebuild a first-class operation.

**OP-9** `RESHAPE` Model swap and calibration runbooks

- **Need**: An operator can change the scoring model or the floors with a
  defined procedure and no retroactive effect on settled credit.
- **Contract**: Staged weights with checksums. A configuration change yields
  a new classifier identity. Credit rows are stamped with the identity that
  approved them and stay valid. A dimension change requires a new index
  epoch. Calibration follows the phased procedure with a decision rule
  committed before the run.
- **Note**: In the new design this becomes: register a bundle, run it in
  shadow, promote by assignment. The runbook changes. The guarantees do not.

**OP-10** `KEEP` Workers, schedulers, and bounded work

- **Need**: Every background job is bounded, idempotent, resumable, and
  observable, and can be driven by an external scheduler or an in-process
  loop with the same credential.
- **Contract**:
  - Every worker route takes a limit with a documented default and maximum
    and rejects values outside the range.
  - Every worker route has a dry-run mode.
  - Leases have a TTL. Stale runs are recoverable by an admin route.
  - Retries use bounded backoff and a maximum attempt count.
  - Scheduler tokens, purposes, and reasons are never returned by status
    surfaces.
- **Check**: For each job kind: run with an out-of-range limit, run twice,
  kill mid-run and recover.
- **Note**: The new design's processing job and effect outbox implement this
  for evaluation. The same rules apply to export, retention, settlement, and
  outbox jobs.

## 13. Cross-cutting invariants

These hold on every route and every job.

| Id | Invariant |
|---|---|
| INV-1 | Authorization comes from the credential. Envelope fields are attribution only. |
| INV-2 | Every read and write is tenant-scoped through auth-derived context. |
| INV-3 | Stored rows and logs are hash-only or label-only. |
| INV-4 | A missing required dependency refuses the path with a named missing control. |
| INV-5 | Unknown and unowned resources are indistinguishable to the caller. |
| INV-6 | An external effect is recorded before it is applied and receipted after. |
| INV-7 | Old decisions and audit rows are never rewritten. Current state is a projection. |
| INV-8 | Every list read has a default and a maximum limit. Out-of-range is rejected. |
| INV-9 | Every error body is `{"error": "<stable label or message>"}`. |
| INV-10 | Every privileged mutation requires a reason and writes an audit row. |

## 14. Contracts that are artifacts of the old architecture

These are current behaviours that a redesign should not carry forward as
promises. Each names the need underneath, if any.

**LEG-1** `DROP?` Static tenant tokens and HS256 signed claims

- The docs call these pilot bridges. The need is a dev-mode credential for
  local runs. Keep one dev-only mode behind an explicit flag that refuses
  production.

**LEG-2** `DROP?` File-backed store and the dual-write cutover flags

- `TRACE_COMMONS_DATA_DIR`, `DB_DUAL_WRITE`, the per-surface `DB_*_READS`
  and `*_TENANT_IDS` canary flags, `OBJECT_PRIMARY_*`, and the reconciliation
  and rollback drills that prove file-versus-database parity. The cutover is
  complete. The need is a single storage contract: metadata in PostgreSQL,
  bodies in encrypted object storage, references between them verified by
  a reconciliation check.

**LEG-3** `DROP?` Rollout-smoke checks tied to the cutover

- `db_reconciliation_clean`, `rollback_flag_drill`, `object_primary_reads`,
  `object_store_migration`. Replace with checks tied to the new risky paths:
  evaluation commit, effect application, index epoch, reprocess.

**LEG-4** `DROP?` Classifier-specific columns on the decision row

- `perplexity_micros`, `tail_fraction_micros`, `novelty_score_micros`,
  `dedup_simhash`, `credit_quality_*`, `contributor_*`. The need is
  measurements with a schema identifier. Forensic queries that read these
  columns need an equivalent over the new documents.

**LEG-5** `DROP?` The gate-service kind string as the trust decision

- `is_production_gate_service_kind`. Replace with recorded trust evidence.
  See `EVAL-9`.

**LEG-6** `DROP?` Column-specific admin re-score routes

- `rescore-perplexity`, `score-credit-quality`, `recluster-dedup`,
  `recompute-contributor-caps`. Replace with one reprocess operation. See
  `EVAL-7`.

**LEG-7** `DROP?` Vector insertion inside the classifier

- Today the novelty path inserts before the decision is durable. The need is
  an insert effect after commit. See the design's classifier restriction.

**LEG-8** `DROP?` Three separate vector paths

- Gate novelty, general vector metadata worker, and shadow dedup index. The
  need is named namespaces with explicit roles and consistency rules.

**LEG-9** `DROP?` `gate_version_hash` as the version identity

- It omits the renderer and the evidence. Replace with the bundle identity.
  Credit rows stamped with the old hash must still resolve to a classifier
  identity after migration.

**LEG-10** `RESHAPE` Two verbs for stopping use of a trace

- Revoke on a device key and withdraw on an account session with different
  deletion guarantees. See `CON-2`.

**LEG-11** `RESHAPE` Worker role list shaped by the current pipelines

- See `ID-10`. Define roles per job kind in the new job model.

**LEG-12** `DROP?` Envelope fields the server fills but nobody reads

- `embedding_analysis.vector_ids`, `nearest_traces`, `clusters` on the
  envelope duplicate what the decision record must hold. Keep the envelope
  as input. Put derived results in the evaluation.

## 15. Known drift between documents and code

The redesign should not inherit these. Fix the documents or the code before
the contract tests are written.

- `docs/trace-spec.md` presents status as the output of the two gates and
  omits `awaiting_pii_backstop`. Code derives status from residual risk.
- `docs/trace-commons.md` lists `GET /v1/traces` as a contributor route.
  It requires the reviewer role. The same document omits the whole
  `/v1/account/*` surface and the attestation routes.
- `docs/upload-claim-issuer.md` omits `POST /v1/enroll` and `GET /onboard`.
- `docs/contributor-daemon-ipc-v1_1.md` says withdrawal always answers
  `account-session-required`. The daemon now holds an account token.
- `docs/operator/pilot-contributor-onboarding.md` says quarantine has no
  downgrade path. Remediation and re-scrub exist.
- The invites design lists `InviteExpired` and
  `InviteCredentialAlreadyBound`. Neither exists on the wire.
- `TRACE_COMMONS_COMMUNITY_LEADERBOARD_SNAPSHOT_INTERVAL_SECONDS` is
  documented twice with different defaults.
- The grant-role check constraint omits `competition_read_worker`.
- The gate-trust-evidence spec of 2026-07-29 has no implementation.
- `embedder_model_id` lives on the vector entry, not the decision, so the
  replay tool compares against `gate_policy_version`.

## 16. Verification plan

The contracts above split into three test families. Each family runs against
the new system as a black box.

**Conformance tests over HTTP.** One test per `KEEP` and `RESHAPE` contract
in sections 2, 3, 5, 6, 7, 9, 10, and 11. Fixtures: two tenants seeded with
the same ids, one contributor per consent scope, one trace per residual-risk
tier, one revoked trace, one expired trace, one held policy. Golden files
pin every response body and error message named in this document.

**Evaluation contracts.** Section 4. Golden traces with pinned canonical
text, measurements, and decisions per classifier identity. A registration
guard asserts the count of semantic tests so a deregistration is visible. A
replay test re-evaluates a sample and diffs. A shadow test proves no effect
from a shadow bundle. A reprocess test proves immutability of old records.

**Operational drills.** Section 12. The drill set, the preflight, the
forensic queries, the startup matrix, the cross-tenant RLS set, and the
rotation and restore procedures, each run against a fresh deployment of the
new system.

A contract is fulfilled when its check passes on the new system without a
change to the client, the operator runbook step, or the customer's export
consumer, except where the contract is tagged `RESHAPE` and the change is
recorded.

## 17. Route index

Routes named in this document, grouped by party. Paths are the current wire
paths. A `RESHAPE` contract may move a path if the client is updated in the
same release.

Contributor, device credential:
`POST /v1/traces`, `POST /v1/traces/{id}/revoke`, `DELETE /v1/traces[/{id}]`,
`POST /v1/contributors/me/submission-status`,
`GET /v1/contributors/me/credit`, `GET /v1/contributors/me/credit-events`,
`GET /v1/contributors/me/score-attestation`,
`POST /v1/account/login-links`, `PUT|DELETE /v1/community/profile`.

Contributor, issuer:
`POST /v1/onboard`, `POST /v1/enroll`, `POST /v1/trace-upload-claim`,
`GET /.well-known/trace-commons-ed25519-keyset.json`,
`/v1/onboard/{cohort}/{challenge,claim,status}`.

Contributor, account session:
`GET /v1/account/traces[/{id}[/content]]`,
`POST /v1/account/traces/{id}/withdraw`, `POST /v1/account/logout`,
`POST /v1/account/sessions/revoke-all`, `/v1/account/passkeys/*`,
`/v1/account/near/*`, `/v1/account/near-identities/*`,
`/v1/account/merge/{start,confirm}`, `/v1/account/native/{authorize,token}`,
`GET /account/login`, `POST /account/login/confirm`,
`/account/passkey/login/*`, `/account/near/login/*`.

Public:
`GET /health`, `GET /.well-known/trace-commons-attestation-keyset.json`,
`GET /v1/community/{leaderboard,contributors/{handle},analytics/summary}`.

Reviewer:
`GET /v1/traces`, `/v1/review/*`, `GET /v1/analytics/summary`,
`GET /v1/audit/events`.

Customer and export workers:
`GET /v1/datasets/replay[/manifests]`, `GET|POST /v1/workers/replay-export`,
`/v1/workers/export/jobs/*`, `/v1/ranker/*`, `/v1/workers/ranker/*`,
`/v1/benchmarks/*`, `/v1/workers/benchmark-*`.

Operator:
`/v1/admin/config-status`, `/v1/admin/operational-{summary,metrics}`,
`/v1/admin/rollout-smoke/*`, `/v1/admin/*-drill`, `/v1/admin/maintenance`,
`/v1/admin/retention/*`, `/v1/admin/export/*`, `/v1/admin/tenant-policy`,
`/v1/admin/tenant-access-grants*`, `/v1/admin/device-keys*`,
`/v1/admin/vector-entries`, `/v1/admin/credit-*`, `/v1/admin/ranking/*`,
`/v1/admin/community/snapshots/recompute`, issuer admin `/v1/admin/invites*`.

Workers:
`/v1/workers/retention-maintenance`, `/v1/workers/revocation-propagation`,
`/v1/workers/vector-index`, `/v1/workers/gate/evaluate`,
`/v1/workers/utility-credit`, `/v1/workers/utility-attestations`,
`/v1/workers/credit-settlements/run`, `/v1/workers/credit-cycle/*`,
`/v1/workers/near-credit-outbox/*`, `/v1/workers/ranking/*`,
`/v1/workers/process-evaluation[s/run]`.
