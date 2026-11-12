# TraceCommons Quint Modeling Implementation Plan

> **For agentic workers:** Execute the approved design with independent model families in parallel, integration and a fresh whole-suite review. This dynamic workflow follows the repository's delegation preference.

**Goal:** Build a source-grounded current-behavior model suite and review surface before selecting product promises.

**Architecture:** Detailed finite Quint models expose separate durable, visible and in-flight state. A composed model and correspondence table connect subsystem boundaries. A coverage manifest makes every coarse or absent behavior explicit.

**Tech Stack:** Quint 0.33.0, Python standard library, HTML/CSS, pinned bkase flow-promises tooling; later TLC and Pele.

**Spec:** `docs/superpowers/specs/2026-11-05-quint-client-server-design.md` (owner approved).

## Global Constraints

- Source baseline `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`; plugin baseline `6f35f6ae6b3581c499bdfcae080932b4ffa7e281`.
- Current behavior; no production modifications or application dependencies.
- No product invariants, temporal properties or TLC checks before owner promise selection.
- MacOS leads; record platform differences and coarse abstractions explicitly.
- No hand-drawn state charts; initial charts may be absent pending the first selected-promise check.
- No promise proof inferred from typechecking or simulation.

## Review Focus

- Late results after account change or UI closure must retain actual request guards.
- Approval, byte identity, successful send and confirmed receipt must stay distinct.
- Durable phase leases and human-review claims must stay distinct.
- Physical index writes, logical eligibility and invalidation must stay distinct.
- Internal credit, pipeline NEAR issuance and account NEAR AI payouts must stay distinct.

## Interfaces

Each model family owns `models/NAME.qnt`, `models/NAME.cfg`, `models/NAME.view.mts`, `models/NAME.frames.json`, and `models/NAME.md`.
The module name equals NAME; exports are `init`, `step`, `vars`, `controls(frame: str): Set[str]`.
Use one record variable `s` or separately named variables; action names are sentences such as `YouApproveTrace` and `TheReceiptFails`.
Each config initially contains only `INIT q_init` and `NEXT q_step`.
The view exports `title`, `state(vars)` returning the visible frame label, and `step(action, from, to)`.
Frame JSON is an array of `{ "label": str, "title": str, "body": str, "controls": [str], "kind": "client"|"server" }`.
Frame labels must equal those returned by the view; user actions guard on the controls they represent.
Domain notes record exact source anchors, variables/bounds, transitions, omissions and composition boundary events. Keep promises out of Quint/config files until selected.

### Task 1: Current client and daemon models

- [x] Build `client_shell`, `client_contribution`, `client_services` with source notes, views and frames. Preserve independent state and all approved client domains.
- [x] Record any coarse behaviors, platform-specific differences and omitted subflows.
- [x] Refine grant identity/revocation in `client_consent`, provider account/non-NEAR managed behavior in `client_providers`, and platform lifecycle differences in `platform_clients`; these are approved-scope gaps found during coverage review.
- [x] Typecheck on integration; add no property checks yet.

### Task 2: Receipt and processing models

- [x] Build `server_identity`, `server_receipt` and `server_pipeline` with source notes, views and frames.
- [x] Separate routing/idempotency/staged storage from leased phase processing and human holds.
- [x] Typecheck on integration; add no property checks yet.

### Task 3: Settlement, access and other service interfaces

- [x] Build `server_settlement`, `server_lifecycle`, `server_interfaces` with source notes, views and frames.
- [x] Preserve the two payout systems, index uncertainty, withdrawal/export races and other route families as explicit abstractions.
- [x] Typecheck on integration; add no property checks yet.

### Task 4: Composition, inventory and review surface

- [x] Build `end_to_end` and correspondence mapping to detailed actions/fields.
- [x] Generate source route/IPC/screen inventory and assign each to a detailed model, coarse interface or explicit coverage gap.
- [x] Build an HTML surface from frame JSON and current design tokens/copy, with a chart placeholder pending the first TLC check.
- [x] Provide repeatable typecheck/simulation, inventory and surface commands; no application dependency changes.

### Task 5: Verification, review and promise selection handoff

- [x] Typecheck every model and run finite simulation without product invariant checks; record exact evidence and simulation limitations.
- [x] Validate frame/view/control correspondence and coverage inventory; validate generated HTML/JS and local HTTP.
- [ ] Inspect the rendered surface visually: unavailable because the CUA browser access host is unavailable.
- [x] Fresh reviewer checks source fidelity, abstraction honesty and composition. Fix material defects and repeat affected checks.
- [x] Show the surface, then propose product-language promises for owner selection. Record pending approval explicitly.
- [ ] Following selection in a later step: encode only selected promises, run pinned TLC/statechart/Pele checks, replay failures, and publish exact verdicts/generated charts.

## Progress

- Setup: isolated worktree reused and moved to `/private/tmp/tc-quint-client-server`; primary checkout untouched.
- Design approval received: "okay go for it". Plan execution is authorized; no redundant plan-approval gate.
- Delegated independent model families to scoped agents; integration remains in the primary agent. All writes are confined to this worktree.
- Built structural verifier (Quint typecheck, evaluated control projections, sampled ITF/view consistency), generated review surface and selected-promise check wrapper. No product promises encoded.
- Ruling: end-to-end frames are internal composition panels rather than literal client screens, because multi-step account/device boundary abstractions must not be presented as real native buttons. Detailed client frames carry client-facing labels.
- Browser preview attempt: CUA browser access host unavailable. Local HTML server is available; visual browser verification remains unavailable through that tool. Structural and model validation continue.
- Review found manual-vs-unattended project policy and stale external dispatch mismatches; authors are correcting them against source. Added server identity detail within the approved identity scope instead of leaving all ceremonies external to receipt acceptance.
- Initial 10-model structural run passed: 1,280 sampled traces / 129,280 sampled states, with unobserved actions/frames reported. Evidence predates review fixes and is not final validation; relevant models will be rechecked.
- Coverage-driven refinements add consent, provider and non-macOS platform families. This follows the approved requirements for grant identity, complete account flows and explicit platform behavior rather than assuming parity.

- Final structural validation:14 models,155 frame/control projections,1,792 sampled traces /180,992 sampled states; all model/view/frame hashes match the recorded evidence. Source reviews resolved material findings within inspected scopes.
- Pending: owner choice of P1-P13; no promise definitions/TLC checks or generated checked charts. Browser visual QA remains unavailable. Explicit coverage gaps remain documented.
