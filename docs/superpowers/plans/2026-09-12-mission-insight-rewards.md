# Mission and insight rewards

Date: 12 September 2026
Owner: Abhishek
Audience: Trace Commons maintainers and reward-program operators
Status: Operator-managed pilot design. Production activation requires the gates below.

## What earns a reward

A participant earns a fixed award for completing a published evidence task and obtaining an independent review against its frozen rubric. A mission may ask for a reproducible refactor, an evaluation, or an investigation. An Insights program may request a complete comparison bundle or a verified correction to an identified result. The offer states what evidence counts before the participant starts.

Completion concerns the work requested. A reproducible failure or a qualified inconclusive finding can earn the same award as a favorable finding when each satisfies that rubric. Missing attempts, abandoned work, and an unsupported assertion of success do not satisfy it. Paying only for favorable model results would bias the evidence the program collects.

| Activity | Reward condition |
| --- | --- |
| Mission completion | The submitted bundle meets the published task and evidence requirements. Retries and resumed sessions remain one work unit. |
| Requested Insights contribution | A reviewer accepts the requested comparison, correction, or evaluation with its supporting evidence and limitations. |
| Independent replication | A separately commissioned work unit produces its own evidence. Reusing another participant's evidence is insufficient. |
| Private analysis, draft creation, token volume, or a user acceptance label | These actions alone do not establish reward eligibility. |

The program purchases useful evidence. It does not infer effort from token spend, derive rewards from the model being evaluated, or convert an existing novelty score into a mission award. Correction tasks identify the result to check in advance, which limits incentives to manufacture an error and collect a repair reward.

## Existing system and integration boundary

[PR #870](https://github.com/TraceCommons/trace-commons/pull/870) assigns mission and insight rewards to Abhishek. Its [program plan at the reviewed revision](https://github.com/TraceCommons/trace-commons/blob/15a24ee2846dbb8e792fd135e9da6008e6f14cbd/docs/superpowers/plans/2026-09-11-trace-insights-program.md) keeps rewards outside the Insights team's delivery obligations.

The inspected Insights stack contains [local mission drafts](https://github.com/TraceCommons/trace-commons/blob/3bfedab827f116b45f242e0508ecf79beacb7de9/crates/trace-commons-protocol/src/mission_draft.rs), whose validation does not authorize publication or rewards. Its [task workflow](https://github.com/TraceCommons/trace-commons/blob/3bfedab827f116b45f242e0508ecf79beacb7de9/crates/trace-commons-contributor/src/insights/comparison_tasks.rs) provides useful evidence-binding conventions, but user confirmations do not establish independent evaluation. Neither is an authoritative mission-completion producer.

The [Trace Credit schema](https://github.com/TraceCommons/trace-commons/blob/60bfe5329aa166a5ec2cb5519d616f581d43fef3/migrations/V1__trace_commons_schema.sql) binds credits to corpus submissions, so mission and insight program awards use a separate ledger. Private Insights remains account-free under its [existing design](../specs/2026-08-07-private-contributor-insight-design.md); local analysis creates no reward claim and uploads no history.

## Offer and accounting rules

An offer binds a program identifier to immutable terms: activity, task definition, rubric, evaluator policy, required evidence, rights, challenge procedure, and sponsor identity. It specifies positive integer award units, total program capacity, a participant cap, a closing time, and a bounded reservation lifetime.

Operators must make the externally retained terms available to participants because a digest alone does not communicate an offer.

The pilot records program units with no present redemption or exchange-rate promise, reporting each program's balance separately without conversion into Trace Credit. Operators set award sizes and capacity explicitly; the implementation supplies no invented market rate or calibrated weight, and execution-resource budgets remain separate from the reward offer.

Before commissioning work under a guaranteed award, the issuer reserves its exact amount against the published terms digest. Unsubmitted holds have a deadline: the earlier of their published lifetime or the program closing time. Timely submission preserves the hold throughout review. Reviewer delay does not forfeit it, while rejection or cancellation of an unsubmitted reservation releases capacity under the published rules.

Capacity consumption equals active unsubmitted holds, submitted claims awaiting review, and recorded awards. Acceptance converts the existing hold, counting the work once; invalidation after acceptance preserves both the original award and its capacity consumption.

The pilot provides no clawback or discretionary adjustment operation.

Terms cannot be amended in this version. Creating an existing program identifier with different terms fails. Lifetime work and evidence uniqueness spans programs within a tenant, so changing a program identifier cannot pay the same work again. The conservative pilot also retains duplicate protection after rejection or invalidation. Corrections and appeals require human resolution under the published challenge policy; the CLI does not silently reopen a final decision.

## Participant and reviewer authority

The first implementation is an operator-managed PostgreSQL workflow. An issuer records a stable participant pseudonym, obtains consent, and retains the supporting evidence in an approved external location. Participant identity is explicitly `operator_asserted`. It is not an authenticated contributor account or proof that two pseudonyms represent different people. Operators must apply caps to a stable identity across devices and accounts.

Issuer and reviewer authority derives from separate authenticated database logins. A DBA provisions each login's tenant grant and comparable identity digest. The runtime can execute bounded reward functions; it cannot edit grants or ledger tables directly. The CLI exposes no actor or reviewer override. Exact identity matches between the reviewer and the participant, sponsor, program creator, reservation creator, or submitter block acceptance. Affiliations require human verification because distinct digests cannot establish independence.

Every operation checks its login's tenant grant, including reads, and forced row-level security applies to the reward tables. Writes serialize within a tenant so reservation capacity, duplicate checks, and evidence invalidation form one atomic decision; the pilot favors inspectable behavior over parallel write throughput.

## Evidence lifecycle

The reservation binds a complete-work identity and consent record. Submission adds the evidence-bundle and evaluation digests. The reviewer checks completeness, rights, rubric compliance, evaluator provenance, and conflicts against the retained materials. A hash proves neither truth nor evaluation quality.

Exact retries return the original logical result; reusing an identifier with changed content fails. A different account or program cannot submit an already claimed evidence digest, and the pilot has no exception that turns copied evidence into independent replication.

Invalidation records a durable tombstone even if evidence has not yet been submitted. That tombstone blocks subsequent claims. Invalidation before acceptance releases the pending hold and prevents an award. After acceptance, history shows both the award and the invalidation; the original decision and units remain intact. Revoking access to externally retained evidence follows that storage system's controls and cannot erase plaintext already delivered to a reviewer.

## Delivery sequence

| Stage | Deliverable and exit evidence |
| --- | --- |
| Operator pilot | PostgreSQL migration, permissions, immutable terms, reservation and review functions, CLI, bounded history, and runbook. Real database and CLI tests prove authorized issuance and review, denied authority, replay, capacity races, duplicate protection, and invalidation. |
| Mission integration | Agree an interface with the #870 owners after an authoritative mission attempt and evaluator result exist. Bind the frozen mission version, complete attempt evidence, independent evaluator, consent, and lifecycle events to the existing reward admission boundary. |
| Insights integration | Offer a separate opt-in action for requested evidence contributions. Qualify complete task linkage and evidence projections, including mixed/delegated work and omitted attempts. Prove that private analysis alone creates no claim or network upload. |
| Contributor access and wider campaigns | Use authenticated account identity, define account-merge and cross-tenant campaign behavior, deliver terms/status/history, and qualify appeals and abuse handling before opening enrollment. |
| Redemption decision | Define the funding source, unit conversion if any, obligations, and settlement interface through a separate approval. Qualify its accounting and operational controls before making a value-bearing promise. |

## Production activation gates

The owner must select an actual mission or requested Insights contribution and publish its intelligible terms, exact unit schedule, evidence requirements, and challenge procedure. A designated issuer and independent reviewer must complete the workflow using authorized evidence. That exercise records review burden, rejected and abandoned work, duplicate attempts, disputes, and whether accepted evidence answers the commissioned question. Test fixtures cannot establish evaluator competence or economic calibration.

The ledger requires a live PostgreSQL verification of permission denial and isolation, plus a complete CLI round trip. Regression checks cover the existing corpus-credit boundary and server build. Maintainability and correctness/security reviews precede a push. Deployment, participant invitations, and any payout remain separate operational actions.

The [operator runbook](../../operator/mission-insight-rewards.md) describes provisioning, commands, and the evidence retained for each decision.
