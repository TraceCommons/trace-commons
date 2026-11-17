# TraceCommons current-behavior Quint model suite

Approved by the owner on 2026-11-05: "okay go for it".
Product promises remain awaiting separate selection. Structural model validation
is recorded in `docs/flows/trace-commons/validation.md`.

## Intent

Model the entire implemented client experience and server processing lifecycle
using the flow-promises workflow from `bkase/plugins`, pinned to
`6f35f6ae6b3581c499bdfcae080932b4ffa7e281`.
The owner selected current behavior and a suite of connected models.

The initial repository inspection was at
`8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6` on `main`. The primary checkout
contains unrelated untracked work. Implementation will use the repository's
`.claude/bin/tc-worktree` workflow, passing the selected commit explicitly as
the base rather than accepting its refreshed `origin/main` default. Each source
mapping must record the actual worktree commit. Untracked files are not part of the baseline and must be
listed as excluded, rather than silently treated as shipping behavior.

## Approach

Use detailed domain models plus an end-to-end composition with explicit
shared boundary events and a coverage index. A single detailed Cartesian
product of every UI, daemon, and worker state would be expensive to explore
and hard to interpret. A single simplified happy-path model would omit the
races this work is meant to expose. Domain models preserve those details;
the composition checks interactions at their documented abstraction boundary.

The builder must not claim that separate domain checks prove the composition,
or that a small composition proves every detailed implementation transition.

A composition correspondence table must name each shared boundary event, the
detailed model and transition that owns it, and its projected state in the
end-to-end model. Include account/device authority and allowance, pinned
submission bytes/key, durable receipt and routing owner, processing/holds,
settlement and payout, withdrawal/eligibility, and client read-back. Matching
event names alone do not establish correspondence.

## Coverage

| Model area | Required implemented behavior |
| --- | --- |
| Shell and lifecycle | Launch, daemon startup/refusal/missing roots, navigation and deferred destinations, settings, window closure versus daemon lifetime, subscription lag/resync, quit, updates |
| First run and identity | Current six-screen first run, private-AI-only route, source permissions, enrollment, browser sign-in and callbacks, organization choice, account renewal/change/sign-out, device management |
| Discovery and consent | Source discovery, watch roots, per-project choices, Ask versus automatic, consent scopes, source revisions, preview and pinned envelope, approval invalidation, held review, dismiss/expiry |
| Upload and receipts | All eight queue states; enrollment recheck; privacy/witness outcomes; caps, rate limits and pacing; network/auth failure, backoff and retry; successful send versus missing receipt; restart and history reconciliation |
| Private AI and managed tools | Credential and organization changes, provider/routing selection, harness plan/confirmation, managed session prepare/launch/run/stop/history, unavailable backend and capacity, cancellation and late replies |
| Insights and learning | Local analysis and empty/loading/error/cached states, submitted views and credit projections, mission drafts, skill candidate/review/evaluation/publication/install/rollback, scoped account ownership |
| Community and account surfaces | Profile publication/withdrawal, invitations/referrals and related visible state, credit summaries, funding and payout-link controls |
| API and storage boundary | Auth-derived tenant/actor/role, upload claims/admission authority, idempotency/conflict, quotas, encrypted object write and receipt commit/orphan, current legacy/versioned routing |
| Versioned processing | Immutable bundle selection, Admission/Review/Score/Settle, rejection/quarantine, privacy rescrub, human review and Score holds, worker lease/expiry/reclaim, operational retry/failure, operator hold and resolution |
| Index and settlement | Score/index command persistence and application, index invalidation/rebuild fences, settlement selection and instrument progress, credit ledger/audit/outbox, pipeline NEAR credit issuance and the separate account-level NEAR AI/referral payout rails, payout submission and confirmation, external success followed by local failure |
| Withdrawal and access | Local withdrawal and server revocation, previous-account boundaries, expiry, retention/legal holds/purge, derived index cleanup, authorized export and downstream propagation |
| Operations and composition | Bundle activation and policy intervention, missing controls, configured feature routes, crash/restart, concurrent worker/account/device events, end-to-end history/credit/withdrawal feedback |
| Other service entry points | Legacy gate/export paths, replay/benchmark/ranker and active-learning workers, grants and utility/vector work, publication/community snapshots, rewards/missions, token bundles, witness inference/evidence, analytics and operator drills; explicit domain models or documented coarse interfaces for every route family |

Every discovered entry point, IPC command, visible screen family, and server
route/worker family gets an index row: owning model, source anchor, observable
state, transitions, and abstraction or exclusion rationale. An unmodeled row
is a visible coverage gap. Merely naming a subsystem does not establish coverage.

Native macOS is the client reference, as required by AGENTS.md. Shared daemon
behavior is modeled once. Windows, GTK and Tauri require an explicit difference
inventory and additional transitions or models where behavior differs; parity
must not be assumed. External Ironclaw implementation is outside this repo;
its protocol boundary is included.

## Model discipline

- Separate visible screen state, durable storage, in-flight requests, actor
  authority, and external side effects. Include request generations and
  identities wherever the implementation uses them.
- Give user and world events separate sentence-named actions. Allow world
  events during navigation, logout, closure and retry wherever code allows.
- Preserve current behavior even when it conflicts with an intended contract.
  Record the mismatch; do not repair production code or idealize the model.
- Use small explicit domains for tenants, devices, sessions, bundles, workers,
  attempts, clock relations and amounts. Include at least two actors/objects
  wherever identity confusion, races or duplicate work matter. Document bounds.
- Model cryptography as validated/invalid evidence and immutable byte identity;
  do not pretend to prove cryptographic primitives, Rust/Swift memory safety,
  PostgreSQL internals, model quality or deployed infrastructure configuration.
- Never reduce behavior to meet a chart budget. Merge labels only for rendering
  and disclose each merge. Distinguish exhaustive finite exploration, random
  simulation, temporal checks, and unchecked or resource-limited results.

## Design surface and promises

Create source-grounded HTML screen frames using current design tokens and copy,
including relevant loading, offline, error, saving and empty variants. Frame
labels and controls must match the model view. Server-only states appear as
pipeline panels alongside client frames. These panels describe internal state
and are not proposed application UI.

Show the surface before proposing guarantees. Then present product-language
promise candidates with the behavior each rules out. The owner selects the
promises before any invariant/property is written or checked, as required by
flow-promises section 4. Existing contracts and regression tests inform these
candidates; they do not count as owner selection.

Fairness assumptions belong only to progress promises that explicitly name
them. A permanently offline service, missing authority or a user who does not
act cannot be assumed away silently.

## Validation and handoff

1. Typecheck all Quint modules and validate screen/control correspondence.
2. After promise selection, use the pinned skill's check script to compile to
   TLA+, run TLC and render generated Mermaid/SVG through its statechart tools.
   Quint 0.33.0 is installed; Java availability must be resolved before TLC.
3. Record bounds, states explored, exact command/tool/source versions and each
   promise result. Never report a timeout or partial check as a proof.
4. Replay every counterexample against source and, where feasible, a focused
   real regression. Treat a non-reproducing path as a suspected model defect.
5. Let the owner choose any product change or promise revision. Recheck all
   selected promises after changing a flow/model.
6. Deliver `docs/flows/trace-commons/` with README/coverage, `.qnt`, `.cfg`,
   `.mts` views, generated `.mmd`/`.svg`, HTML frames, reproducible commands,
   source mappings, counterexample paths, and explicit omissions. Keep bulky
   transient TLC output outside the committed seed.

No new application dependencies, production behavior changes, deployments or
external messages are part of this modeling task. Tool downloads needed for
Quint/TLC/rendering are isolated from application dependency manifests.

## Initial source anchors

- Client lifecycle/navigation: `macos/Sources/TraceCommonsApp/AppModel.swift`,
  `MainWindowNavigation.swift`, `MonitorNavigation.swift`, `DaemonStartup.swift`.
- First run: `macos/Sources/TCShellCore/FirstRun/` and
  `macos/Sources/TraceCommonsApp/FirstRunRunner.swift`.
- Daemon: `crates/trace-commons-contributor/src/daemon/queue.rs`, `uploader.rs`,
  `settings.rs`, `policy.rs`, `approved_envelope.rs`, `automatic_gate.rs`,
  `account_onboarding.rs`, `account_session_renewal.rs`, `nearai_credential/`,
  `managed.rs`, `skill_loop.rs`, and the `insights_*.rs` modules.
- Current pipeline: `crates/trace-commons-server/src/versioned_pipeline.rs`
  and sibling `versioned_pipeline_*.rs` modules. Its run states are Pending,
  Leased, Retry, AwaitingReview, AwaitingOperator, Complete and Failed; phase
  state is separate. Review-start privacy processing and Score holds are
  separate from basic Admission/Review/Score/Settle ordering.
- Admission executes synchronously with receipt acceptance; worker leases
  apply to later phases. Receipt routing ownership survives activation and
  rollback. Physical index presence is distinct from logical eligibility.
  Confirming an already-submitted payout differs from dispatching a new one
  under a suspended policy. Do not collapse these transitions.
- Production runtime assembly is in
  `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/production_assembly.rs`.
  Treat runtime configuration and deployment enablement as inputs rather than
  inferring that a compiled feature is enabled on a live service.
- Boundary references: `docs/contributor-daemon-ipc-v1_1.md`,
  `docs/trace-commons.md`, `docs/trace-commons-storage.md`, and
  `docs/superpowers/specs/2026-09-11-versioned-pipeline-behavioral-contracts.md`.
  These guide discovery; implementation resolves current-behavior questions.
  The older deployment architecture document is not a reliable complete
  description of today's native client or production runtime.
- Public endpoints such as `/v1/source` require explicit public-route states;
  do not impose a blanket account-authentication rule on the route inventory.
