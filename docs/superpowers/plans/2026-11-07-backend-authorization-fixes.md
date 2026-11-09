# Backend authorization fixes implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development for independent tasks; root integrates and reviews the complete branch.

**Goal:** Repair the five locally reproduced backend authorization finding families in a separate PR.

**Architecture:** Retain authentication-derived tenant/actor and existing role boundaries. Compare effective scope semantics; require attributable ownership for contributor access; enforce retention deadlines in export selection/completion; serialize bootstrap admission with identity insertion; and reject unsafe applicable RLS policy/role compositions at readiness. Preserve scoped admin recovery and explicit narrow resolver access.

**Tech Stack:** Rust, PostgreSQL 16, Quint 0.33, existing Python verification tooling. No new dependency.

**Spec:** Local audit commit `3d2a7b8ab` on `backend-authorization-quint`, baseline `f13a91d388340356d4c744e78596db853e8d45ff`; reports at `/private/tmp/tc-backend-authorization-quint/docs/security/backend-authorization/`.

## Global constraints

- Preserve the primary checkout and audit branch; fixes use this separate worktree/PR from origin/main.
- No merge, deployment, credential access or production probing.
- No dependency changes; preserve split license and AGPL headers.
- Every cargo invocation uses RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0.
- Follow AGENTS/CLAUDE: write all regression tests, then run integrated validation once after all tasks; per-task tests are deferred.
- Do not weaken existing auth, role, tenant, consent or audit checks; /v1/source remains public.

## Review focus

- Empty-list semantics across both consent scopes and allowed uses, mixed restricted/unrestricted callers and tenant ceilings.
- Legacy sentinel versus method-bound historical alias, account principal expansion and privileged recovery.
- Deadline equality, revocation, export selection versus later publication, and already-delivered history.
- Concurrent weak bootstrap finishes and concurrent strong-authenticator enrollment/removal paths; all writers must share account serialization.
- Permissive/restrictive policy composition, nested inherited memberships, exact explicit resolver transitions and supported migrations.

### Task 1: Scope and ownership guards

- [x] Convert audit scope/legacy witnesses into desired-behavior regressions; retain modern owner/reviewer and cross-tenant controls.
- [x] Reject effective target widening, including empty-as-unrestricted targets under nonempty ceilings.
- [x] Remove principal_legacy as contributor ownership/credit authority and self-revocation authority; preserve exact canonical/per-credential historical aliases and explicit reviewer/admin permissions.
- [x] Document fail-closed legacy recovery rather than invent ownership from attribution.

Files: ingest main target/ownership/credit helpers; two focused internal test files copied from audit. Shared test registry owned by root.

### Task 2: Export expiry

- [x] Convert expired export reproduction to refusal; add before/equal/after deadline, status and publication-time checks.
- [x] Gate legacy replay and ranker exports on current expiry at use, including revalidation before publishing when asynchronous work can cross the deadline.

Files: ingest export/eligibility helpers and focused tests. Root owns this task.

### Task 3: Atomic authenticator bootstrap

- [x] Add concurrent backend regression using valid same-account weak requests/DB operations.
- [x] Perform weak-session first-authenticator admission and identity insert atomically under the same account serialization followed by other relevant strong-authenticator writers.
- [x] Preserve strong-session legitimate enrollment, first weak enrollment, uniqueness/refusal behavior and API error mapping.

Files: Database trait, PostgreSQL identity/account operations, ingest enrollment call, relevant PG tests. No new migration unless essential; raise any unavoidable schema decision to root with evidence.

### Task 4: Fail-closed RLS readiness

- [x] Reproduce inherited resolver/additional permissive-policy configurations against actual diagnostics on a disposable migrated PG fixture.
- [x] Reject applicable policy/role widening while preserving intentional narrow explicit SET LOCAL ROLE resolver behavior and supported restrictive policies.
- [x] Validate transitive inheritance, runtime ownership/bypass paths and readiness failure propagation.

Files: PostgreSQL RLS/catalog and referral resolver readiness; focused PG tests and operator docs. Disjoint hunks from Task 3; no shared formatting until root integration.

### Task 5: Models, integrated verification and PR

- [x] Add compact fixed-behavior Quint models/checks, retaining historical counterexamples separately from current guarantees; do not import the entire historical inventory/evidence into this fix PR.
- [x] Run fmt, server/ingest tests and required PG suites, clippy with repository allow-list, license boundary and relevant feature checks; no skipped DB checks presented as passes.
- [x] Independent whole-branch security review; fix findings and rerun affected checks.
Publication deliverable: commit and push the separate branch, then open its PR with exact verification/limitations. Publication status is recorded by GitHub and the final task reply. No merge/release.

Ruling: legal hold preserves physical storage and suppresses maintenance status changes, but does not grant continued export after the stored deadline. The old legal-hold export fixture is updated to assert retained storage plus zero exported items. This follows the user-approved expiry repair and avoids treating a preservation control as new data-use authority.

Validation uses a fresh PostgreSQL database per server executable, matching CI fixture isolation.
