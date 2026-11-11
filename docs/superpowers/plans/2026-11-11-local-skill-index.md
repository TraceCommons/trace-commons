# Local Skill Index Implementation Plan

> **For agentic workers:** Use the approved dynamic workflow: parallel scoped implementers, root integration, then independent whole-branch review. Track tasks below.

**Goal:** Build the approved local inventory and usage experience as a feature-flagged PR.
**Architecture:** Extend the existing Skills implementation from PR #2004, which depends on the environment feature flags PR #2003. Keep discovery and extraction in permissive Rust crates, daemon-owned policy and IPC, and native SwiftUI presentation.
**Tech Stack:** Rust, existing serde/chrono/filesystem utilities, SwiftUI, typed JSON IPC.
**Spec:** ../specs/2026-11-11-local-skill-index-design.md

## Global constraints

- Existing `insights_skills` flag remains default off. Flag enablement grants no filesystem or analysis permission.
- Use isolated `tc-wt-local-skill-index`, created with `.claude/bin/tc-worktree`; preserve the primary checkout.
- Stack PR on `insights-skills-v1`; do not modify or merge either prerequisite PR.
- No new dependencies. No AGPL dependencies in client crates. No raw path/definition logging.
- RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0 on every Cargo command.
- Per repository guidance, write tests with each task and run suites once after integration.
- Preserve current navigation identity; implement the inventory Index within the existing Skills surface unless code evidence establishes a newer Index route.

## Review focus

- Excluded canonical targets through allowed symlinks must not be scanned.
- Disabled/cached plugin files must not be labeled available without activation evidence.
- Name collisions and duplicate imported sessions must not inflate attributed usage.
- Disable/clear/exclude during background work must prevent stale data writes.
- Empty, unsupported, and partially analyzed history must remain distinct from observed zero use.

## Dynamic execution

Root owns integration, flag verification, validation, final review fixes, commit and PR.
Parallel agents own disjoint inventory, usage/daemon, and Swift/shared-copy areas.
Agents publish additive wire contracts before consumers implement; dependent edits wait
for those contracts. Reassign work when evidence changes the boundary. Do not launch
competing cargo builds. Use existing model for complex implementation and review;
no model switch is assumed or claimed.

### Task 1: Inventory and identity (inventory agent)

Files: contributor-insights/src/insights/skills.rs and skills/{inventory,policy,tests}.rs.
Interface: existing scan_inventory_with_policy; additive inventory details keyed by
installation ID, with aliases, availability and definition status. Preserve callers.
- [x] Add configured-root, plugin-state, malformed-definition and safe-alias fixtures.
- [x] Extend bounded provider discovery and conservative identity handling.
- [x] Expose partial coverage without leaking local paths to telemetry/storage.

### Task 2: Usage and daemon integration (usage agent)

Files: daemon/insights_skills.rs, its tests, insights_week integration and dedicated
usage extraction modules as required. Interface: existing Skills snapshot IPC,
additive usage window, sessions, invocations, loads, last use and evidence fields.
- [x] Add fixtures for invocation/load distinction, listing exclusion, duplicates,
  ambiguous names, window boundaries, partial coverage and permission changes.
- [x] Implement deterministic extraction and window aggregation, preserving flag,
  consent, bounded scheduling and cancellation behavior.
- [x] Send exact wire schema to UI owner and root before consumer edits.

### Task 3: Native Index experience (UI agent)

Files: InsightsSkillsModel.swift, Views/InsightsSkillsTab.swift,
TCShellCore/DataContract/SkillsModels.swift, relevant Swift tests and skills_copy.rs.
Consumes additive Task 1/2 snapshot fields; missing fields decode conservatively.
- [x] Add wire and UI-state tests for unavailable/pending/partial usage and flag off.
- [x] Implement searchable/filterable inventory, usage windows and evidence details.
- [x] Keep flag-hidden navigation and polling; preserve selection during refresh.

### Task 4: Integration and verification (root)

- [x] Reconcile schemas and inspect spec coverage; record material limitations.
- [x] Run cargo fmt checks and targeted insights/contributor suites with fixed flags.
- [x] Run license_boundary suite and appropriate lint checks.
- [x] Build FFI, then run affected Swift suites; inspect representative UI if supported.
- [x] Independent whole-branch review, fix substantive findings and rerun affected checks.
- [ ] Commit, push dedicated branch and open PR targeting insights-skills-v1, explaining
  prerequisite PRs, flag behavior, validation evidence and remaining limitations.
- [ ] Inspect CI and report PR, local checks and hosted checks separately.

## Execution ledger

- 2026-11-11: Primary checkout stale relative to main; fresh helper-created worktree.
  No finished donor available. Existing #2003/#2004 are OPEN, so stack on #2004
  instead of duplicating its implementation or taking its worktree.
- User authorized dynamic implementation through PR; continue without another plan
  approval. No product rollout or merge is authorized by this workflow.
- Navigation ruling: current prerequisite branch has Insights > Skills > Inventory;
  label that inventory subtab Index. Preserve persisted top-level identities and
  avoid introducing a second Skills experience. This is narrower than inventing
  a top-level Index tab that does not exist in the target branch.
- Fourth scoped integration agent owns safe on-demand definition/evidence detail,
  coordinating shared-file edits with daemon/UI owners.
- Usage attribution ruling: old retained evidence has names, not immutable
  installation provenance. Do not assign ambiguous observations to installations
  or imply a historical definition was used; expose observation uncertainty.
- Initial implementation coverage: Claude invocation/read and Codex successful-read
  evidence; OpenCode and delegated-session evidence remain unsupported/partial.
  Installed OpenCode skills remain discoverable. Do not advertise complete usage
  coverage. The existing evidence list is capped at 20 references per skill.
- Inventory and UI implementers completed their changes and regression fixtures;
  integrated Rust library suites are running. No green claim yet.
- User added Flow Promises statechart work and selected all five proposed promises.
  Added docs/flows/skills-insights: bounded design model, generated chart projections,
  review surface and mutation checks. TLC explored 1,338 states/18,795 transitions;
  all five promises hold in the model, and all five negative controls fail as
  intended. This is not implementation proof; README records replay obligations.
- Contributor library suite: 5,260 passed, 8 ignored. Affected contributor suite
  after integration: 141 passed, 3 ignored. Insights suite after copy/fixture fixes:
  725 passed, 3 ignored. Client clippy passed. Swift and license gates pending.
- Whole-branch review found and fixed: omitted sanitized inventory details on IPC;
  exclusion/clear/flag changes during response assembly; shared-root observations
  incorrectly requiring agent=shared. New regressions cover actual daemon response
  budgets, late scope changes, and separate per-consumer shared usage without
  manufactured summed trace totals. Final affected checks are rerunning.
- Native full run reached one source-guard failure caused by the callback name
  openSettings; renamed to reviewSettings. The subsequent 33 selected native
  tests passed, including that guard, Skills behavior, and wire decoding.
- Final review-fix validation: 20 daemon Skills tests passed (1 ignored), shared
  copy contract passed, client clippy passed, FFI rebuilt, cargo fmt check passed,
  34 selected native tests passed against the rebuilt FFI. License boundary
  remains 4 passed; no dependencies or license-boundary edges changed.
