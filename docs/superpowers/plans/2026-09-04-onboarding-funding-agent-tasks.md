# Onboarding funding and pilot agent tasks

Date: 2026-09-04. Status: executable decomposition; implementation not started.
Parent: [Native inference onboarding](2026-09-04-native-inference-onboarding.md), slices 2-5.
Worktree: `.worktrees/native-onboarding-admission`, branch `native-onboarding-admission`.

Read `AGENTS.md` and `CLAUDE.md` first. This assignment adds no dependencies,
changes no economic terms, and authorizes no deployment or live trace-content
submission. Read-only API discovery, local contract fixtures, and mocked tests
can proceed immediately. Record unavailable external capabilities precisely;
continue independent work without inventing an API or sponsor.

## Shared evidence and boundaries

- `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`:
  `AccountCreditSummary` and `account_credit_summary_handler` serve
  `GET /v1/account/credit-summary`. Fields are earned points, optional currency
  estimate, settlement/grading posture, period, and pending review. No provider
  spendable balance, credential provisioning, or redemption contract is exposed.
- `crates/trace-commons-server/src/credit_numbers.rs`: disabled settlement stays
  pending; dry-run settlement issues no funds; `graded=false` means estimates.
  An on-chain settlement acknowledgement alone does not prove inference access.
- `macos/Sources/TraceCommonsApp/Views/SettingsView.swift` and contributor
  `src/routing/ironwire.rs`: native discovery observes an existing local proxy.
  It does not install it, provision a provider key, or reconfigure an agent.
- Contributor `src/daemon/preview.rs`: enrolled witness review currently refuses
  with `witness_claim_unavailable`. Slice 1 consent is not a working native pilot.
- Contributor `src/routing/receipt.rs` fetches the receipt before witness
  processing. `src/daemon/settings.rs` keeps body consent separate from ledger
  observation. Privacy-scanner credentials are not inference credentials.
- Existing NEAR login resolves linked identities. A signature alone creates
  neither a new contributor account nor provider funding.

## Agent C: inference-funding

**Objective:** produce an implementable, reviewed contract connecting eligible
contribution credit or existing provider funds to usable inference, without
representing pending estimates as funds or assuming sponsorship.

**Ownership:** first create
`docs/superpowers/specs/2026-09-04-onboarding-funding-contract-design.md`.
After contract review, own a new server funding module and associated tests;
propose exact paths before implementing. The parent owns route integration,
migrations, and shared types in `trace-commons-protocol`; hand schema changes to
it. Permissive clients cannot depend on server or gate crates. Platform
secret adapters and native agent setup are explicit handoffs to native owners.

**Steps and required outputs:**

1. Trace account-scoped earned credit, settlement outbox, reversals, and auth to
   establish the existing source of truth. Inventory configured provider
   adapters without printing credentials or querying billable inference.
2. Inspect provider documentation and available read-only management surfaces.
   Record exact supported operations, authentication, units, freshness,
   idempotency, per-account key scopes, limits, revocation, and balance lookup.
   Gate implementation on these capabilities. Account creation, credit transfer,
   key minting, spend ceilings, refund, and usage reconciliation each need
   evidence; a generic inference API does not establish any of them.
3. Propose a versioned, authenticated contract with capability flags;
   estimated/pending/eligible credit; separate provider-available balance with
   currency or token unit and observation time; and provisioning state.
   Unknown, stale, unsupported, and zero are distinct. Native clients receive
   safe action/reason labels, never provider admin credentials or raw audit data.
4. Specify reservation transitions: requested -> reserved -> provisioning ->
   confirmed, with definite failure -> released, and ambiguous outcome ->
   reconciling. A timeout must not release funds that may already be spent.
   Document which transitions the actual provider supports before choosing
   retry or compensation behavior.
5. Bind each request to auth-derived tenant/account plus a durable idempotency
   key and canonical request fingerprint. Same-key retries return the same
   operation; changed payloads conflict. Reserve eligible value atomically;
   concurrent devices cannot overspend. Propagate a provider idempotency key
   when supported; otherwise require lookup/reconciliation before another call.
6. Specify refund/reversal semantics from real provider capability: release an
   unconsumed reservation; refund a confirmed charge only after confirmed
   provider reversal. Record partial usage and partial refunds exactly once.
   Define reconciliation ownership, retry limits, durable checkpoints, provider
   outage behavior, and discrepancy handling. Monetary arithmetic uses explicit
   units and exact representation, never inferred conversion of display strings.
7. Define least-privileged, revocable inference credentials, account/device
   binding, rotation, and logout behavior. Native owners must use platform
   secret storage and handle locked/unavailable stores without plaintext
   fallback. IPC/UI carry status or opaque handles. Scope server secrets to the
   provider adapter; redact request/response errors and retain hash-only audits.
8. Hand native owners zero-history states: existing funded provider account;
   no funding available; sponsor-backed allowance only when an actual sponsor
   and approved budget exist. Amounts, pricing, eligibility, and subsidy terms
   remain explicit policy inputs owned by the product/operator decision maker.
   Preserve local discovery and later resumption when funding is unavailable.

**Contract review handoff:** provide proposed schemas and example payloads,
transition table, ownership map, provider capability evidence, open decisions,
and an adapter fake. Identity owner reviews account/device binding; admission
owner reviews budget boundaries; native owners review credential transfer and
unknown-state copy; Agent G reviews observable acceptance evidence.
Do not publish speculative routes as deployed capabilities.

**Tests:** duplicate requests, changed-payload retries, concurrent reservations,
insufficient eligible credit, crash after provider success before local commit,
timeout reconciliation, partial usage/refund, duplicate callbacks, reversed
contribution credit, expired/revoked credentials, stale/unknown balances,
cross-tenant/account isolation, hash-only errors, unavailable secret store,
and disabled/dry-run settlement never yielding a spendable inference claim.

## Agent G: integration-validation

**Objective:** prove the selected native app can complete useful funded inference,
review and contribute that session, then use earned credit for another call.
Mocks prove contracts; a separately authorized live pilot proves interoperability.

**Ownership:** create
`docs/operator/native-inference-onboarding-pilot.md` and a proposed offline
integration fixture suite. Coordinate shared test-harness changes; do not edit
funding, identity, admission, or native implementations owned by other agents.

**Start immediately:** inventory existing receipt/witness tests and the
[release design](../specs/2026-09-04-attested-inference-release-design.md).
Build a dependency matrix and mocked failure scenarios from existing contracts.
Prepare a repeatable runbook, synthetic session fixture, and hash-only evidence
schema. No trace bodies, provider identifiers, tokens, or wallet identifiers
belong in the evidence report. Record test build versions and capability states.

**Dependencies before live execution:** Agent C contract plus working adapter;
explicit actual funding; native claim-backed witness preview and immutable
reviewed-envelope upload; supported IronWire agent configuration and receipt
endpoint; deployed witness pins/policy; explicit capture/body consent. Self-
service bootstrap and account-bound admission are additional prerequisites only
for the later public-admission pilot, not the invited first cohort.

**Acceptance sequence:** discover/select one agent without uploading; connect
actual funding and securely provision credentials; verify provider routing with
one useful task; capture the final exchange; fetch and verify its receipt;
review the witness-redacted artifact; upload the exact reviewed bytes; observe
acceptance; follow eligible credit through actual redemption; observe confirmed
provider-available balance; complete a subsequent funded inference call.
Record local setup completion separately from each observed activation milestone.

**Failure matrix:** cancel consent; withdraw consent; restart between every
stage; key rotation; insufficient funds; proxy unavailable; receipt unavailable;
claim expiry; witness/policy mismatch; settings changed after review; upload
retry; provider timeout after charge; duplicate redemption; settlement disabled
or dry-run; no existing traces or funding. Preserve work and resumability, never
silently bypass the witness, infer balance, or mark mock outcomes live-verified.

**Exit report:** list passed local tests, unresolved dependencies, failed stages,
and exact reproducible commands. Later live evidence must distinguish observed
provider usage, accepted contribution, settled credit, and usable inference.
No public rollout until its separate identity/admission gates also pass.
