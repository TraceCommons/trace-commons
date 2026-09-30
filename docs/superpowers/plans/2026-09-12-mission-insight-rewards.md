# Mission and insight rewards

Date: 12 September 2026
Owner: Abhishek
Audience: Trace Commons maintainers and reward-program operators
Status: Reward-program delivery plan for review. The operator pilot is implemented; participant integration and redemption remain undelivered.

The [program design](../specs/2026-09-12-mission-insight-reward-program.md) specifies the full participant outcome, economic decisions, admission contracts, consent lifecycle and settlement boundary. This plan maps that design to code and concrete delivery packets; completing the pilot packet does not complete the program.


## Existing system and integration boundary

[PR #870](https://github.com/TraceCommons/trace-commons/pull/870) assigns mission and insight rewards to Abhishek. Its [current reviewed program revision](https://github.com/TraceCommons/trace-commons/blob/0bbf08b4469a669092e225c9c3b9ae76ce38d7ca/docs/superpowers/plans/2026-09-11-trace-insights-program.md) keeps rewards outside the Insights team's delivery obligations, including funding, redemption and payout work. Those responsibilities remain in this reward program.

The inspected Insights stack through [#941](https://github.com/TraceCommons/trace-commons/pull/941) contains [local mission drafts](https://github.com/TraceCommons/trace-commons/blob/8d018dc19b618967e599ba159bca960b12a63a3d/crates/trace-commons-protocol/src/mission_draft.rs), whose validation does not authorize publication or rewards. Its [task workflow](https://github.com/TraceCommons/trace-commons/blob/8d018dc19b618967e599ba159bca960b12a63a3d/crates/trace-commons-contributor/src/insights/comparison_tasks.rs) adds source and overlap bindings, while user confirmations remain distinct from independent evaluation. It supplies no authoritative mission-completion producer or reward-consent grant.

The [Trace Credit schema](https://github.com/TraceCommons/trace-commons/blob/60bfe5329aa166a5ec2cb5519d616f581d43fef3/migrations/V1__trace_commons_schema.sql) binds credits to corpus submissions, so mission and insight program awards use a separate ledger. Private Insights remains account-free under its [existing design](../specs/2026-08-07-private-contributor-insight-design.md); local analysis creates no reward claim and uploads no history.

## First implementation boundary

R0 implements a manually reviewed, nonredeemable program-unit ledger and an operator CLI. Database logins establish issuer/reviewer authority, while participant identity, intelligible publication, consent and evidence qualification remain operator assertions retained outside the ledger.

The [program design](../specs/2026-09-12-mission-insight-reward-program.md) is the authority for offer accounting, admission, identity and lifecycle policy. Later packets must supply authenticated participation and qualified source contracts before exposing end-user claims or redeemable promises.

## Design decisions and rejected alternatives

| Decision | Reason and implementation consequence |
| --- | --- |
| Award fixed units after independent evidence review | Eligibility-only records do not deliver an award; unqualified local outcomes cannot establish completion. The operator loop records actual unit awards without claiming automatic verification. |
| Reserve capacity once | Acceptance consumes the existing hold. Subtracting at both reservation and award would understate available capacity. |
| Separate reward provenance from Trace Credit | Corpus submission foreign keys and utility-event settlement rules do not describe mission work. Reusing those events would falsify evidence provenance. |
| Authenticate reviewers through database logins | Caller-chosen reviewer or authority labels cannot enforce independence. The CLI supplies no identity override. |
| Retain lifetime work and evidence uniqueness | A new program identifier or terms version cannot reset duplicate protection. Identical-evidence replication exceptions are excluded. |
| Keep original awards after invalidation | Reversals, clawbacks and discretionary adjustments require a published authority and funding policy. The pilot records the invalidation without inventing that policy. |
| Store the operator-asserted identity mode | Future account integration must distinguish pilot assertions from authenticated ownership. No automatic account conversion is implied. |
| Implement shared admission types with their adapters | Unused protocol structs do not establish a producer. The next contract requires a qualified source, consent lifecycle and consumer together. |

## Delivery packets

Abhishek owns the reward packets; dependencies on mission and Insights producers require agreement with their owners. Packets after R0 are planned, not implemented.

| Packet | Concrete work | Dependency | Exit evidence |
| --- | --- | --- | --- |
| R0: Operator ledger | Immutable terms and identity mode; reserve, submit, independent review, award, cancellation, invalidation and paginated history; least-privilege roles; operator runbook | Current main, no new dependencies | Full migration chain on a fresh database followed by real role and CLI workflows; denied authority, replay, cap races, clock expiry, pagination and invalidation tests; structure/security review; reviewable PR and CI |
| R1: Published offers and participant principals | Participant-readable terms; authenticated reward principal; enrollment and stable cap identity; explicit mapping of any pilot award; offer suspension and reservation status | Owner selects first commissioned task and whether production rewards are redeemable | Account spoofing and merge cannot duplicate awards or reset caps; unpublished/full/suspended offers cannot invite work; a participant can inspect and reserve an offer |
| R2: Consent and evidence admission | Shared bounded claim contract; explicit local projection/preview; purpose-specific consent receipt; source-version and invalidation bindings; independent evaluator provenance | Agreed source and rights contracts; authenticated reward principal | Saving private Insights sends nothing; unauthorized, stale, oversized, replayed and withdrawn evidence is refused; valid opt-in reaches the claim boundary through a live round trip |
| R3: First mission reward | Bind one published mission version, participation and complete attempt set to R2; record independent completion and reward acceptance separately | Qualified mission loop from #870, including starting-artifact controls and evaluator | An authorized participant completes the selected task and receives the pinned reward; incomplete/omitted attempts and self-review fail; retained evidence supports the decision |
| R4: First Insights reward | Bind one requested comparison, correction or evaluation to R2; preserve coverage, overlapping/delegated attempts and unsupported attribution | Qualified requested contribution and inspectable task evidence | One authorized contribution earns after independent review; a qualified negative result can earn, while an unsupported favorable claim cannot; private analysis remains account-free |
| R5: Participant status and disputes | Terms, allocation, review reason, complete history and appeal route; append-only appeal/adjudication; provisional rejection holds and recorded notice; enrollment abuse and conflict review | R1-R4 identity, evidence and decision records | User can contest a refusal; timely appeals preserve capacity, final rejection releases it and an overturn creates at most one award without oversubscription; invalidation cannot erase an earlier outcome |
| R6: Funding and redemption | Choose denomination, backing, exact conversion and liability rules; define migration treatment for pilot units; authorize a reward-specific settlement record | Explicit economic policy and funded capacity; authenticated recipient | Funding reconciles to outstanding promises; unfunded offers refuse reservations; loss of funding and disputed unpaid awards follow published rules |
| R7: Payment and wider release | Integrate qualified transfer infrastructure with reward-specific provenance, receipt reconciliation and idempotency; operational recovery and participant notice | R6 plus approved payment execution | Real authorized payment succeeds; uncertain submissions and duplicate callbacks cannot double-pay; failures preserve entitlement; release evidence is reviewed before enrollment expands |

## Current implementation and verification map

The server [adapter](../../../crates/trace-commons-server/src/mission_rewards.rs), [ledger migration](../../../migrations/V69__mission_insight_rewards.sql), [forward migration](../../../migrations/V70__reward_history_pagination.sql) and [operator CLI](../../../crates/trace-commons-server/src/bin/trace-commons-reward-operator.rs) implement R0 without consuming local mission or Insights objects. The [test fixture](../../../crates/trace-commons-server/tests/mission_rewards_pg/fixture.rs) runs the normal production migrator before creating issuer and reviewer logins.

| Requirement | Evidence to retain with the PR |
| --- | --- |
| Deployed schema and permissions | Migration record, guard ownership, runtime execute surface, forced RLS and repeat-migration preservation from deployment tests |
| Valid and invalid authority | Real issuer/reviewer connections; same-identity, missing-grant, wrong-role, cross-tenant and direct-write refusals |
| Award and capacity invariants | Concurrent reservations and exact review retries; participant caps; duplicate work/evidence; invalidation before and after award |
| Clock and read behavior | Natural database-clock expiry/closure; observed advisory-lock waiter before commit; consistent post-commit history |
| Complete operator history | Equal-timestamp cursor ordering, foreign-cursor refusal, full totals across pages and real CLI continuation |
| Compatibility | Server build and relevant tests, license/storage boundaries, formatting and Clippy; upstream main and source-contract revisions pinned in this plan |

These tests execute real PostgreSQL transactions with synthetic evidence inputs. They prove accounting and access behavior, not completion of a real mission, useful model comparisons, economic calibration or participant usability. CI status must be tied to the actual PR head; a local pass does not establish a remote platform result.

## Production activation gates

The owner must select an actual mission or requested Insights contribution and publish its intelligible terms, exact unit schedule, evidence requirements, and challenge procedure. A designated issuer and independent reviewer must complete the workflow using authorized evidence. That exercise records review burden, rejected and abandoned work, duplicate attempts, disputes, and whether accepted evidence answers the commissioned question. Test fixtures cannot establish evaluator competence or economic calibration.

The ledger requires a live PostgreSQL verification of permission denial and isolation, plus a complete CLI round trip. Regression checks cover the existing corpus-credit boundary and server build. Maintainability and correctness/security reviews precede a push. Deployment, participant invitations, and any payout remain separate operational actions.

The [operator runbook](../../operator/mission-insight-rewards.md) describes provisioning, commands, and the evidence retained for each decision.
