# Tested skill loop

**Status:** implementation contract for the first Trace Commons v0.12 skill family.

## Product outcome

An owner can open an accepted session, turn a decisive correction about generated files into an inspectable Agent Skill, compare that skill on different tasks, and install a passing version into Codex. Publication remains a separate, explicit action through the existing public-page flow.

The first family covers one recurring failure: changing a generated artifact instead of its source schema, template, generator, or dependency lock. This narrow family gives the comparison an exact scoring contract and rejects unrelated corrections.

## Owner flow

1. **Learn from session.** The daemon re-reads the selected session through the account-authenticated owner endpoint and offers a candidate only when the accepted correction contains a generated-artifact signal and a source-of-truth repair signal. The response cites the source submission, returns at most six 700-character excerpts from the permanently redacted envelope, and replaces the task text with a bounded hash and similarity fingerprint.
2. **Review skill.** The owner can edit the applicability and procedure before Rust validates the draft, renders the complete `SKILL.md`, and gives the app the exact bytes to show before any model call or filesystem write.
3. **Approve and test.** One explicit action binds the reviewed SHA-256 and starts baseline, one disclosed manual instruction, and the candidate skill on six source-excluded plan tasks plus two applicability probes. All 24 requests use one pinned NEAR AI model, a 900-output-token ceiling, a 90-second timeout, and at most two concurrent requests. The six plan tasks use a counterbalanced schedule that places each arm twice in each position; one comparison runs per daemon.
4. **Inspect results.** The result separates the six repository-plan scores from two applicability probes, then shows each model response, scoring failures, regressions against both controls, model ownership, and token ceilings. Installation stays unavailable until the candidate passes more repository plans than each control, passes both applicability probes, and avoids failing any task either control passed.
5. **Install skill.** A plan displays the complete `SKILL.md`, signed ownership marker, exact digest for each file, and symbolic `$CODEX_HOME/skills/<name>` locations. It refuses an occupied destination and requires a second action carrying both preview digests. Rollback remains available only while the signed package matches its recorded owner, device, lineage, and content.

## Evaluation contract

The fixed corpus contains seven distinct public repository incidents and two metadata-only applicability probes. Before prompting, a local text-free fingerprint excludes any incident whose task is an exact or lexical near-match for the source task. Six eligible incidents are selected for scoring; unused eligible incidents remain disclosed reserves, and the evaluation fails closed if six cannot remain. Each repository fixture records the immutable upstream commit, bounded file excerpts, generated outputs, regeneration commands, and independent verification commands. Evaluator-only answer keys never enter model prompts.

The scorer requires a complete structured repository plan with all of these properties:

- It edits every required source file and no generated output directly.
- It runs an exact allowlisted generator or source-derived build command.
- It runs an exact allowlisted independent drift or correctness check.
- It stays inside the bounded action schema and names no unrelated edit path.

The applicability probes give every arm the same neutral decision wrapper. The candidate arm receives only its reviewed name and description; it must recognize one relevant task and reject one unrelated handwritten-file task before its body can affect the six incident-level plan tasks.

The evaluator makes 24 Cloud completion requests after freezing the approved review digest: three arms across six plan tasks and two applicability probes. Applicability and plan scheduling each start at position zero; across the six plan tasks, every arm appears exactly twice in each position.

The report records selected, excluded, and reserve fixture ids so the held-out claim can be inspected. The corpus supports a controlled plan-quality gate; executable task completion, statistical significance, and sandbox execution remain unproven.

## Provider and privacy controls

- Account credentials and NEAR AI keys remain in the existing OS credential store. They never cross IPC, enter prompts, or reach logs. Every workflow call derives an owner-scope SHA-256 from the current authenticated account and device context; a changed scope clears held state, and evaluation checks the scope again after all model calls.
- The evaluator accepts a model only when the live `/v1/model/list` record reports `ownedBy: nearai`, `isReady: true`, structured JSON support, and text output. It uses the same-origin `/v1/models` compatibility route only when the current catalog route is unavailable. Selection follows a deterministic compatibility list of models already exercised against the fixture contract, then the lexically first eligible model. Updating that list requires rerunning the transport and scoring suites. One model id is pinned for the comparison, and a completion that reports another id aborts the result.
- Evaluation prompts contain public fixture context and the arm-specific control. The source session task, contributed correction, transcript excerpts, account identifier, wallet material, host rules, host skills, and local repository files stay local. The baseline receives no added instruction, the manual arm receives the disclosed one-line control, and the candidate arm receives the approved generic skill.
- Catalog responses are capped at 2 MiB, completion responses at 128 KiB, retained model output at 8 KiB, and the serialized report at 768 KiB.
- Replacing or forgetting the inference credential increments a local revision. The daemon cancels the active comparison and discards its result if that revision changes before completion.

## Installation controls

The daemon resolves one absolute Codex skills root internally, accepts no path over IPC, exposes only `$CODEX_HOME/skills/<name>` symbolic locations, and limits validated skill names to 64 lowercase ASCII letters, digits, or single internal hyphens. The installer assembles and syncs the complete two-file package in a private sibling directory, publishes it with an exclusive atomic directory rename, and refuses existing directories, symlinks, modified files, multiple lineage matches, or a root scan beyond 512 entries.

### Signed installation

The schema-v2 marker binds source and evaluation lineage to the current owner-scope SHA-256 and device key id. A domain-separated Ed25519 signature from the existing `DeviceIdentity` authenticates the canonical marker across daemon restarts; the private key stays inside the identity and never serializes or enters logs. The marker also carries a content digest and rejects unknown fields or non-canonical JSON.

Plan returns the exact skill and marker bytes with `skill_sha256` and `marker_file_sha256`. Commit accepts the plan id plus `as_previewed_sha256` and `as_previewed_marker_sha256`; it writes only when both files, the review, identity, owner scope, and evaluation still match. The platform-specific rename refuses to replace an existing target, and post-rename identity verification binds the result to the directory that was staged.

Status resolves an installation through its source submission identifier and verifies the exact signed owner and device-bound package. Rollback validates the two-file package before renaming the whole target to an ignored quarantine name. That rename makes the skill unavailable and constitutes the rollback commit; the quarantined `SKILL.md` and marker receive unrecognized retained names, so success returns `removed: true, retained_directory: true`. A validation or namespace failure before the rename leaves the loadable target intact. Plan, commit, status, and rollback hold one process-wide transaction lock across filesystem inspection and the matching daemon state update.

## Wire contract

| Method | Input | Result |
| --- | --- | --- |
| `skill_candidate` | `submission_id` | Candidate, correction, evidence, manual control, evaluation contract, and any recoverable review token |
| `skill_review` | `candidate_id`, `draft`, optional `replaces_review_id` | Complete `SKILL.md`, digest, and source lineage |
| `skill_evaluate` | `review_id`, `skill_sha256` | Bounded three-arm report and install verdict |
| `skill_install_plan` | `evaluation_id` | Exact skill and signed marker, separate digests, symbolic locations, occupancy, and install permission |
| `skill_install_commit` | `plan_id`, `as_previewed_sha256`, `as_previewed_marker_sha256` | Installed package identity, source lineage, digests, time, and symbolic location |
| `skill_install_status` | `source_submission_id` | Signed, owner-bound installation when present |
| `skill_install_rollback` | `install_id`, `source_submission_id` | `removed` and `retained_directory` |

Candidate, evaluation, plan, and installation identifiers are random correlators. Each consequential transition also checks the source lineage, content digest, current workflow phase, or filesystem state.

## Deferred work

The first release installs into Codex and evaluates structured plans for the generated-source-repair family. Additional task families, executable repository sandboxes, other coding tools, and installable public packages require separate evidence and review. Sharing can attach evaluation results and contributor lineage to the existing public-page flow after this gate demonstrates improvement.
