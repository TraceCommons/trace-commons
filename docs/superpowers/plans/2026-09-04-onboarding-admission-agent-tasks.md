# Onboarding admission: three agent handoffs

Date: 2026-09-04. Status: decomposition only; no server admission work enabled.
Worktree: `.worktrees/native-onboarding-admission`.

Read [admission design](../specs/2026-09-04-admission-invite-or-attestation-design.md),
[native design](../specs/2026-09-04-native-inference-onboarding-design.md), and
[execution plan](2026-09-04-native-inference-onboarding.md), plus AGENTS.md and
CLAUDE.md before starting. Existing native consent is a separate completed slice.
These handoffs describe implementation work after contract review; proposed
module/API names below are ownership assignments, not existing capabilities.

## Coordination and dependency order

1. Contract discovery and negative-test design can run in parallel as slots
   permit under the parent plan's dispatch order. Each returns an interface
   proposal, unresolved decisions, and an
   exact edit list before implementing interfaces consumed by another agent.
2. Root reconciles the stable account anchor, evidence profile, challenge
   lifetime/consumption, receipt key, reservation semantics, and RLS boundaries.
   Numeric subsidies, redemption rates, lifecycle retention, and sponsorship
   are still open product decisions. Do not invent production defaults.
3. Identity builds the authenticated anchor/provisioning module. Binding builds
   evidence verification against the agreed opaque anchor/challenge contract.
   Ledger builds accounting against those contracts using test fixtures.
   Neither fixtures nor client assertions become production authorization.
4. Root integrates identity first, evidence second, and ledger third. Activate
   none of the routes until their required controls exist. External capture
   and live native witness validation remain release dependencies.

Root alone edits shared wiring: server `src/lib.rs`, `src/db/{mod,postgres}.rs`,
`src/bin/trace-commons-ingest.rs`, its `trace_commons_ingest_internal/tests.rs`,
server/protocol/witness module registries, protocol exports and shared wire
contracts, dependency manifests, migration registries, and deployment config.
Agents return wiring patches as handoff notes; they do not concurrently edit
these files. Root allocates each migration number after inspecting current
migrations, then applies each reviewed migration serially. No prechosen V number.
Owned paths are disjoint. Here `server` means `crates/trace-commons-server`;
`contributor` means `crates/trace-commons-contributor`. Docs paths are root-relative.

## Agent B: near-account-bootstrap

**Objective:** establish a verified, stable account anchor and explicit device
provisioning without changing unknown-key login into account creation.

**Inputs and integration points:**

- Server `src/account_near.rs`: `verify_nep413`, `NearAccessKeyChecker`, and
  current FullAccess-key ownership verification.
- Server `src/account_native_auth.rs` and contributor `src/account_auth.rs`:
  existing PKCE, exact loopback redirect, single-use code, and session boundary.
- Ingest `account_near_enroll_start_handler`, `account_near_enroll_finish_handler`,
  `account_near_login_finish_inner`; DB `insert_near_identity`,
  `resolve_near_public_key_tenant`, and `issue_near_session`.
- `migrations/V33__near_identities.sql` and account consolidation V34, read-only.

**Exclusive ownership:** new server `src/account_onboarding.rs`, new server
`tests/account_onboarding_contract.rs`, and
`docs/superpowers/plans/2026-09-04-onboarding-identity-handoff.md`.
Root places the proposed module export and HTTP/DB adapters after review.
**Steps:**

1. Specify an explicit provisioning ceremony distinct from login and enrollment:
   verified ownership, purpose-bound nonce, recipient/origin, expiry, replay,
   account-switch checks, account-to-tenant assignment, and device authorization.
2. Propose stable anchor behavior for multiple keys/devices, unlink/relink,
   deletion, and consolidation. Escalate retention/product ambiguities in the
   handoff; never grant a fresh window merely because credentials change.
3. Implement ceremony validation and typed outcomes in the owned module using
   existing verification/session helpers. Authentication must precede writes.
4. Return transactional persistence and route wiring requirements to root,
   including a migration draft in the handoff document if needed. Reuse native
   PKCE flow; do not expose browser cookies as native credentials.
5. Provide the ledger a verified anchor and device/account authorization result.
   Do not issue inference funds, provider keys, or submission entitlement.

**Tests and commands:** invalid signature, wrong purpose/recipient, replay,
expiry, ownership-check outage, unknown login without provisioning, intercepted
native code without verifier, account switch, reinstall/key rotation preserving
anchor. Root adds handler/DB tests to shared files; include real PostgreSQL
rollback/RLS tests once its adapter exists.
Run `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --test account_onboarding_contract`.
**Deliverables:** module and negative tests, reviewed ceremony/schema contract,
root wiring instructions, lifecycle decisions still open, and actual results.

## Agent D: receipt-binding

**Objective:** make verified inference evidence cryptographically distinguishable
from redaction-only evidence and bind it to the submitting account challenge.

**Inputs and integration points:** server `src/witness_service/inference.rs`,
`src/redaction_witness/{certificate,verification,request}.rs`, and
`tests/witness_certificate_cross_implementation.rs`; contributor
`src/routing/{ironwire,receipt}.rs` and `src/witness/{transport,verify}.rs`.
Read these existing paths; root controls their cross-module wiring for this wave.

**Exclusive ownership:** new server `src/redaction_witness/admission_evidence.rs`,
new server `tests/admission_evidence_contract.rs`, and
`docs/superpowers/plans/2026-09-04-onboarding-evidence-handoff.md`.
Root owns any new permissive wire-profile file and all its exports; the agent
supplies canonical fixtures/specification before root adds those contracts.
**Steps:**

1. Specify a distinct signed profile/domain, unambiguous canonical encoding,
   verified provider identity, request/response digests, extracted account
   challenge, and redacted artifact digest. Decide concrete field widths,
   challenge representation, and lifetime with identity/ledger agents and root.
2. Define verification output that only a successful verifier can construct.
   V1 certificates and an image measurement alone cannot produce that output.
3. Implement verification/profile tests and canonical replay-key derivation.
   Do not rely on raw signature spelling or client-claimed receipt identity.
4. Return witness certificate issuance and contributor transport changes as
   integration requirements. Keep raw bodies confined to existing capture and
   witness boundaries; store only redacted artifacts and necessary hashes.
5. Prepare an external **nearai/ironwire** handoff: insert the challenge in final
   provider-hashed request bytes, retain it across privacy/model/cross-family
   transforms, capture exact request/response bytes, and test streaming/restart
   behavior. This repository has no IronWire dependency or editable checkout;
   actual capture implementation needs separately established repo access.

**Tests and commands:** v1 rejected for admission; tampered digest/challenge;
wrong trust pin/domain; empty/malformed challenge; equivalent receipt encoding
same replay key; raw SSE bytes versus parsed output; no raw-body persistence.
Capture-side transformation tests run in IronWire, not against invented local
capture. Cross-implementation signing fixtures must agree before integration.
Run `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --test admission_evidence_contract`.
**Deliverables:** evidence module/tests, canonical fixtures, root integration
patch requirements, external capture contract, and explicit live-validation gaps.

## Agent E: admission-ledger

**Objective:** enforce per-submission entitlement, receipt replay protection,
and bounded account/global processing exposure with atomic recovery.

**Inputs and integration points:** verified identity from agent B; typed verified
evidence and canonical receipt key from agent D; ingest `submit_trace_handler`,
`verified_witness_for_submission`, existing tenant authorization and quarantine
remediation; server `src/db/{mod,postgres}.rs` and storage contract tests.
Inspect when current submission persistence and processing occur before choosing
transaction boundaries. Existing filesystem writes are not a PostgreSQL transaction.

**Exclusive ownership:** new server `src/admission.rs`, new server
`tests/admission_ledger_contract.rs`, and
`docs/superpowers/plans/2026-09-04-onboarding-ledger-handoff.md`.
Root owns DB trait/adapter wiring and migration installation.
**Steps:**

1. Propose invited/window/per-submission-attestation decisions and configuration
   with no implicit subsidy. Missing required controls refuse the new path.
2. Define idempotency key plus content commitment, challenge/receipt consumption,
   account attempt/cost and global cost reservations, and terminal settlement.
   Resolve remediation explicitly; do not break existing quarantine correction.
3. Design atomic persistence with forced RLS and a narrow hash-only cross-tenant
   uniqueness mechanism. Provide serial migration drafts and SQL operations to
   root. Do not introduce an unrestricted cross-tenant application query.
4. Implement reservation/recovery semantics: pre-processing failure releases;
   quality rejection burns attempt and incurred cost; interrupted processing
   retains incurred cost; identical retries resume without double charge.
5. Coordinate transaction/outbox or equivalent crash recovery with root so an
   accepted receipt cannot be consumed without durable admission, or admitted
   twice after process failure. Expired leases cannot mint free attempts.

**Tests and commands:** PostgreSQL races at final attempt/account/global budget;
cross-tenant replay; unchanged retry versus changed body; worker crash/restart;
pre/post-processing failure; quality rejection; remediation; account lifecycle;
missing controls; unchanged invited gates and public `/v1/source`.
Run `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --test admission_ledger_contract`;
record required PostgreSQL setup and never report skipped DB tests as passing.
**Deliverables:** policy/recovery module, DB concurrency tests, migration and
adapter handoff, explicit cost-unit/limit decisions, and rollback evidence.

## Root integration acceptance

Run repository-required warnings-denied server check/test compilation, relevant
feature checks, formatting, Clippy with the existing allow-list, and
`cargo test -p trace-commons-server --test license_boundary`. New server Rust
files require AGPL headers; permissive code must never depend on AGPL crates.
No new dependencies without explicit approval. No agent changes source-offer
auth, enables public admission, deploys, funds accounts, or sends external
messages. Module tests are not sufficient: root must run wired HTTP, PostgreSQL,
cross-implementation, and invited native end-to-end tests before enablement.
