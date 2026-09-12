# Fresh independent refactor sessions

Parent: [private pilot runbook](2026-09-12-private-refactor-pilot.md); canonical program: [PR #870](https://github.com/TraceCommons/trace-commons/pull/870).

Status: four clean worktrees prepared at the pinned base; Sol task selection and Astra review complete. No fresh implementation sessions have run and no outcomes have been assigned.

The user chose four new independent refactor sessions instead of continuing historical discovery. Preserve the original #616/#606/#632 evidence and accepted assessments as a separate retrospective audit. Those shared-root tasks do not become independent observations.

## Prepared task allocation

[Task briefs and verification commands](2026-09-12-fresh-refactor-task-briefs.md) define four scopes. Allocation is fixed before execution: task 1 Cline to cohort A, task 2 Gemini to cohort B, task 3 OpenCode to cohort B, task 4 trajectory to cohort A. Cohort A/B are preparation slots, not model declarations; bind exact model selectors before launch and retain observed labels independently. All worktrees were verified clean at the same base. Local manifests contain unique allocated session UUIDs and explicitly mark each session as not started. Four complete prompts and a common instruction template are prepared with SHA-256 digests; exact selectors, effective tool policy and execution limits remain unfilled launch fields. Astra reviewed the pinned source and cleared the corrected briefs for preparation freeze.

## Baseline validation

At the pinned commit, the existing contributor library suites passed with `--locked` and `RUSTFLAGS=-D warnings`: Cline 11/11, Gemini CLI 11/11, OpenCode 11/11, trajectory 26/26 (59 total; all commands exited 0). The checkout stayed clean. The four raw logs and their SHA-256 manifest remain local. This establishes the pre-refactor test baseline; it is not implementation, model-performance or outcome evidence.

## Proposed launch defaults

No model or spending decision had been made when preparation was selected. Proposed defaults are cohort A `claude-opus-5` and cohort B `claude-sonnet-5`, high effort for both, identical tools, a 30-minute wall-time limit and a $10 API budget per session ($40 across four sessions). [Anthropic documents both exact selectors](https://support.claude.com/en/articles/11940350-claude-code-model-configuration). These are proposals, not approved spending or verified account access. Confirm the execution limits and effective tool policy, and verify how the harness enforces the budget and timeout before launch. The local manifest keeps proposed values separate from actual execution fields.

## Design and capture rules

Use four distinct useful work items from Trace Commons commit `60bfe5329aa166a5ec2cb5519d616f581d43fef3`, each in its own clean worktree and fresh root session. Keep scopes disjoint; do not replay one problem four times, reuse prior solutions, resume another task's session, or pass one implementation to another participant. Use two matched task pairs, assigning one task in each pair to each of two declared-model cohorts before execution. Freeze task briefs and assignment before outcomes. This balances selected task types but cannot establish causal model superiority or equal difficulty.

Each task has one common instruction template plus its task-specific scope and validation commands. Record the base commit, brief/template hashes, actual writer version, requested selector and observed model declaration, language/project identity, effective harness settings, tools, effort, resource limits, start/end times, root/branch identifiers, attempt membership, test results and final patch digest. Keep private paths, raw traces, credential-bearing settings and detailed session manifests local. Record safe configuration fields rather than copying secrets. Model labels remain declaration evidence, not verified provider identity. Use the common instruction-template hash for the configuration prompt-template digest; keep per-task brief and complete rendered-prompt hashes as separate provenance so task-specific prompts do not create four different comparison strata.

Keep model selection separate from the common non-model configuration fingerprint. Both cohorts must have the same reviewed project/language/harness/configuration context. Resolve exact selectors, common effort/tool policy and equal execution limits before launch; do not silently use changing aliases, automatic fallback models, inherited plugins/hooks/MCP tools or unequal settings. A changed configuration or mixed-model attempt is retained and assessed under the normal eligibility rules, not relabelled.

Use normal permission enforcement, local repository edits and bounded test commands. No permission bypass, remote writes, dependency additions, external task discovery or access to private history. The common prompt explicitly supplies repository instructions. No peer-task access or delegation inside a work item. Distinct root UUIDs and worktrees support provenance; they do not substitute for human review of complete attempt membership and independence.

All retries, failures, abandoned attempts and human interventions remain attached to the original work item. A failed attempt does not justify replacing the work item or silently rerandomizing assignment. Record substantial reviewer or human rework separately. Sol prepares the work; Astra reviews task design and implementation evidence independently. Review outcomes are not user acceptance.

## Shared instruction template

> Perform only the refactor described in this task brief. Read and follow AGENTS.md and CLAUDE.md supplied from the pinned checkout. Preserve public behavior, serialization, error text, ordering, platform gates and licensing boundaries. Do not add dependencies or broaden scope. Work only in the listed files; stop and report if the task requires changes elsewhere. Do not inspect other pilot tasks, prior implementations, private traces or other worktrees. Do not delegate. Run the listed existing checks with --locked and RUSTFLAGS=-D warnings, and report their actual completion and failures. Leave a reviewable diff and a concise account of changes, tests and unresolved limitations. Do not commit, push, merge or publish.

The final rendered prompt replaces the task-brief placeholder with the reviewed scope and commands. Hash that complete prompt before starting each session.

## Source qualification and execution checkpoint

Installed Claude Code is `2.1.269`; the current comparison attribution contract admits observed `2.1.260` agent branches only. Fresh direct-root sessions are also a different shape from that branch contract. Capture the real writer and root shape, then qualify their identity, record chains, tool completion, model coverage and task boundaries with reviewed fixtures and malformed-input tests. Do not rewrite versions, fabricate child-agent envelopes, or broaden an allowlist from metadata alone. Generic import may work while comparison attribution remains unavailable.

Preparation produces reviewed briefs, four isolated worktrees and local capture manifests. Before launching, fill in concrete model selectors, equal time/cost limits and the effective harness configuration. Do not describe an unexecuted command template or an allocated session UUID as a captured session. No new paid runs have been made by this preparation step.

## Review and completion

After execution, root inspects the authorized local captures. Human review supplies task context, complete attempt membership, independence and accepted/partial/rejected/unknown outcomes for these new tasks; earlier accepted assessments do not carry over. Preserve pending and excluded work in coverage. Import into an isolated store, inspect the real macOS workflow, save the comparison with its actual chronology, and check comprehension of coverage and uncertainty.

Two assessed tasks per cohort meet only the exact estimator's minimum support rule. Four tasks are a workflow pilot and will generally provide broad intervals or suppressed precision; they are not evidence for a model ranking, cost advantage or time saved. Saved comparison specifications remain retrospective unless the product's prospective evidence contract is separately implemented and verified. This operational preparation document is not such a specification.

Rewards for missions and insights remain outside scope and owned by Abhishek.
