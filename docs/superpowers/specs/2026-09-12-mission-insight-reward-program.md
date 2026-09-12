# Mission and insight reward program

Date: 12 September 2026
Owner: Abhishek
Audience: Trace Commons maintainers and reward-program operators
Status: Program design for review. The operator ledger is the first implementation; participant access and redemption are not implemented.

## Product outcome

A participant can inspect an offer before starting work, secure its promised allocation, submit the requested evidence, receive an explained independent decision, and inspect the resulting reward record. A dispute keeps the original evidence and decision available. Private Insights continues to work without enrollment or sharing.

[The canonical Insights program](https://github.com/TraceCommons/trace-commons/blob/0bbf08b4469a669092e225c9c3b9ae76ce38d7ca/docs/superpowers/plans/2026-09-11-trace-insights-program.md) assigns reward design and implementation to Abhishek. Mission publication, participation and evaluation are separate dependencies owned by that program. The reward work therefore needs its own complete delivery plan, while integration needs an agreed interface with the source owners.

The proposed production program rewards accepted evidence work. Its initial implementation records nonredeemable program units. A funded reward offer requires a further explicit decision on the asset, backing and conversion rules; no existing unit silently acquires a cash value when that decision is made.

## What earns and how the amount is set

Each offer commissions a defined work unit for a fixed amount. Mission and Insights work use the same accounting rules, with different evidence requirements.

| Offer | Accepted work | Ineligible shortcut |
| --- | --- | --- |
| Mission completion | Complete the published task using the pinned starting artifact, allowed execution policy and required evidence; obtain independent assessment against its rubric | Create a draft, report success, submit only the final successful attempt, or reuse another participant's result |
| Requested Insights evaluation | Deliver the commissioned comparison or evaluation with task linkage, coverage, exclusions and limitations sufficient for independent review | Open Insights, collect sessions, accept a local answer, or produce a favorable ranking |
| Commissioned replication | Independently repeat an identified task under a separate offer and produce independent evidence | Copy an existing evidence bundle under another identity |
| Requested correction | Reproduce and resolve an identified evidence or evaluation defect under a published correction offer | Manufacture an error and claim an unsolicited correction reward |

The award is the offer's fixed unit amount when its rubric is satisfied; otherwise no units are awarded. A qualified negative or inconclusive result earns that same amount when it completes the commissioned procedure.

An incomplete investigation described as inconclusive does not qualify. Review evaluates completeness and reproducibility rather than favorability to a model or sponsor.

Offer owners select the amount and maximum number of awards before recruitment, using the task requirements and available budget; pilot observations then inform future offers. The implementation supplies no calibrated wage, exchange rate or difficulty multiplier.

Any change creates a new offer; an existing reservation retains its original amount and rubric.

Tasks with materially different effort requirements receive separate offers with published amounts. Review staffing and compensation are explicit operating costs; a reviewer never accepts their own compensation claim.

The [existing credit-quality score](../../../crates/trace-commons-server/src/credit_quality.rs) describes corpus-gate signals and remains independent of mission completion; mission awards neither multiply nor overwrite it. Corpus contribution credit still requires a separately authorized and qualified submission.

## Offers, capacity and funding

An offer records its immutable identity, activity, task definition, evidence rules, rubric, evaluator policy, rights, challenge procedure and sponsor. Amount, participant cap, total capacity, closing time and reservation lifetime are required. Participants must receive the intelligible artifacts behind the digests; a digest-only ledger cannot publish an understandable offer.

For the operator pilot, capacity means program units. Consumed capacity is the sum of live unsubmitted reservations, submitted claims awaiting review and recorded awards. Expired or cancelled unsubmitted reservations release their allocation. Submission before the deadline preserves it throughout review. Acceptance converts the existing allocation into an award without charging capacity again.

Reservations precede guaranteed reward invitations; a full or closed program refuses them, while an accepted reservation shows its exact expiry and pinned amount. The retained challenge policy supplies a review target and escalation route, and missing that target must not silently expire a submitted claim.

Production funding is a separate obligation from the unit counter. Before offering redeemable rewards, the owner must select a denomination and funded capacity, identify the custodian or payer, and define an exact conversion from reward units to that asset's atomic units. Use integer arithmetic or proper decimal types, with specified rounding and overflow behavior. An offer cannot reserve more redeemable liability than its confirmed backing permits. Funding loss suspends new reservations and triggers the published treatment of existing commitments.

The current ledger has no balance-of-funds check, asset account, wallet or payout command, so it describes allocations as nonredeemable units. A later conversion requires a qualified funding path and an explicit migration policy for existing awards; their denomination is never inferred from a new default.

## Participant identity and consent

Production enrollment uses the Trace Commons account and permission surface. The reward service derives the participant from authenticated context; callers cannot name another account to receive credit. Account consolidation needs an audited reward-principal mapping so merging accounts neither resets caps nor duplicates awards. The existing corpus-credit account resolver is evidence of an account pattern, not an authorization to reinterpret pilot pseudonyms.

The pilot instead records an operator-asserted participant hash. Its mode remains explicit in stored program state and outputs. An operator verifies stable identity and affiliations outside the ledger. This offers no Sybil guarantee, and a matching digest cannot establish control of a future authenticated account. Conversion to account-owned records requires proof of ownership and an explicit mapping event, with disputes and unmapped awards retained for resolution.

The proposed mapping requires the enrollment issuer and an independent account administrator to verify the original participation records against the authenticated claimant. One pilot participant can have only one active reward principal; account consolidation preserves prior mappings and aggregates their cap usage.

Conflicting ownership claims suspend conversion until adjudication. An operator's assertion alone cannot authorize migration of disputed awards, and insufficient retained proof leaves them unmapped.

Reward consent specifies the selected evidence, recipient, review purpose and retention terms, separately from permissions for private analysis, corpus contribution, public attribution and community analytics. An opt-in action shows the projection before transmission and issues a revocable receipt bound to source versions and the evidence digest. Refreshing or rescrubbing a source requires fresh authorization.

The pilot's `consent_hash` records an operator-supplied artifact, with no live grant or subscription to the corpus withdrawal API; operators must invalidate affected pending evidence manually. A production adapter needs a source-specific lifecycle identifier and durable invalidation delivery, with lost authorization or uncertain freshness blocking award admission.

A pilot participant requests withdrawal through the published operator contact. The operator invalidates withdrawn evidence, releasing any pending hold and preventing its reuse; a submitted claim has no participant-facing cancellation command.

## Mission admission contract

The mission producer publishes and versions the task independently of rewards. A reward offer references that immutable definition and its permitted evidence. Draft validation, discovery popularity and a participant's outcome label cannot replace publication or completion authority.

The proposed admission record contains the following bindings. These are interface requirements for the next integration slice, not existing protocol type names.

| Binding | Required authority and validation |
| --- | --- |
| Program and terms digest | Reward service resolves an existing immutable offer and reservation |
| Participant and consent receipt | Authenticated account owns the participation and authorized the selected review projection |
| Mission definition and starting artifact | Published version matches the offer; execution used the reviewed artifact digest |
| Participation and complete attempt set | Stable work identity joins retries and resumed attempts; omitted attempts remain visible |
| Evidence bundle and source versions | Canonical bounded projection identifies the actual artifacts and their lifecycle state |
| Evaluation result | Qualified evaluator identity, implementation/rubric version, input digest, outcome and supporting references are verifiable |
| Independent reward decision | Reviewer satisfies the published conflict policy and records a separate accept or reject decision |

The adapter validates these bindings before calling the reward admission boundary. It never writes award rows directly. Execution budgets, sandbox authority and provider credentials remain with the mission runtime. Reward eligibility cannot authorize executing a starting artifact or exceeding a tool budget.

The current [mission draft contract](https://github.com/TraceCommons/trace-commons/blob/8d018dc19b618967e599ba159bca960b12a63a3d/crates/trace-commons-protocol/src/mission_draft.rs) explicitly grants no publication or reward authority. The first adapter therefore depends on a qualified mission loop, not an added reward flag on that draft type.

## Insights admission contract

An Insights reward claim begins when a user selects evidence for a requested contribution through a separate consented action. Saving or refreshing a private report creates no claim, account requirement or upload.

An admissible projection binds complete task identities, all attempts, source/profile versions, evaluation provenance, coverage and relevant exclusions. Mixed-model work, delegated branches, shared parent sessions and overlapping exports require explicit treatment. Unsupported attribution cannot be filled from a model label or inferred from the final successful session. An uncertain comparison may qualify as a completed investigation when its rubric asks for that investigation; it cannot be presented as evidence of model superiority.

The [current task-attribution implementation](https://github.com/TraceCommons/trace-commons/blob/8d018dc19b618967e599ba159bca960b12a63a3d/crates/trace-commons-contributor/src/insights/comparison_tasks.rs) supplies source bindings for private comparisons. Those bindings are useful inputs, but user confirmations and local outcomes remain distinct from independent reward acceptance. The reward adapter must preserve source evidence and exclusions while adding authenticated participation, purpose-specific consent and an independent review result.

Shared serializable admission contracts belong in the permissive protocol crate when an actual producer and consumer are implemented together; local projection and preview belong in the contributor service. Server authorization and accounting remain in the AGPL server, without introducing an AGPL dependency into a client.

## Decisions, invalidation and disputes

| Event | Pilot behavior | Production requirement |
| --- | --- | --- |
| Independent acceptance | Append decision and one fixed award; preserve terms and evidence digests | Same accounting invariant, plus qualified producer and account authority |
| Rejection | Retain the reason and release pending capacity | Participant-readable explanation and a published appeal route |
| Evidence invalidated before acceptance | Tombstone prevents award and later reuse | Source lifecycle delivery must reach the claim boundary before acceptance |
| Evidence invalidated after acceptance | Preserve award and flag history; no clawback or capacity refund | Apply the prepublished treatment of any unpaid liability; do not invent a retrospective penalty |
| Appeal | Human resolution outside the ledger; no command reopens the claim | Append an appeal and independent decision tied to the original work; an overturn cannot create a second award |
| Changed offer | Create a separate immutable program; lifetime work uniqueness still applies | Preserve existing reservations and prevent version changes from resetting duplicate or cap controls |

An appeal must not be simulated by changing a program identifier or resubmitting the same evidence under a new participant. Reversal and adjustment commands remain absent until the owner defines their authority, funding effect and participant notice.

Reviewers cannot be the participant, sponsor, issuer, program creator or claim submitter. Database login identity enforces these comparisons in the pilot. Production must additionally verify underlying account identity and affiliations, including evaluator conflicts; two different labels do not establish independence. A provider evaluating its own model cannot become the sole authority for an award-bearing recommendation.

Before enrollment, the operator designates a qualified fallback reviewer and publishes the review escalation contact. Multiple reviewer logins can be provisioned, but the pilot has no queue-assignment service; unavailability preserves submitted allocations and pauses new invitations when review capacity is exhausted.

## Abuse controls and accounting invariants

Reservation expiry and participant caps limit allocation hoarding, while enrollment and affiliation review remain manual in the pilot. Work identities span retries and offer versions, and exact evidence duplicates are refused across participants and programs within the tenant.

Pilot reviewers must inspect semantic copies and reuse across tenants manually. Canonicalization tooling and any shared fraud registry require separate production design; the ledger provides neither global uniqueness nor automatic semantic duplicate detection.

Qualified negative results require the complete commissioned procedure. Correction offers name their target before participation. Sponsors cannot obtain favorable-only results by changing a frozen rubric or selecting only successful attempts. Review disagreements and excluded work remain visible in pilot evaluation.

The ledger preserves these invariants under concurrent requests:

- Within a tenant, an accepted work unit has one award; exact retries produce no extra allocation or award.
- Each reservation contributes to capacity once, including when it becomes an award.
- Tenant and participant boundaries apply to reads as well as mutations; pagination exposes all authorized history without leaking another participant's cursor.
- Invalidation and review serialize; the recorded outcome is consistent with their order.
- Integer limits fail closed, immutable records remain immutable, and unlike program denominations have no combined balance.

## Redemption and settlement interface

The [existing Trace Credit ledger](../../../migrations/V1__trace_commons_schema.sql) requires corpus submission provenance, and [settlement eligibility](../../../crates/trace-commons-server/src/bin/trace-commons-ingest.rs) selects specific corpus utility events. A mission award cannot masquerade as one of those events or borrow a submission identifier to enter settlement.

A future reward settlement authorization must identify the source kind, immutable award, owning reward principal, frozen denomination/conversion policy, payable amount and one idempotency key. The payment adapter may reuse qualified transfer infrastructure after review, while preserving reward-specific liability and provenance. It must reconcile submission, confirmation, retry and failure against the external payment receipt before marking an award paid.

The release decision requires a funded offer, authenticated recipient and a real authorized payment round trip. Tests must prove duplicate callbacks and uncertain submissions cannot pay twice; a failed payment cannot erase the entitlement. Wallet changes, account consolidation, consent withdrawal and evidence disputes need explicit treatment before enabling the adapter. No such payment is authorized or implemented by the operator pilot.

## Delivery evidence and unresolved inputs

The [delivery plan](../plans/2026-09-12-mission-insight-rewards.md) maps each work packet to code, dependencies and exit evidence, including ledger behavior established by database tests with synthetic evidence digests. Evaluator competence, actual mission completion, useful comparisons, economic calibration and a human participant experience require their own qualification.

The owner must still select the first commissioned task, a qualified reviewer and the published amount/capacity. For redeemable production rewards, the asset, funding source, conversion and dispute obligations also require explicit selection. The implementation can validate supplied policy; it cannot infer these product decisions from the existing score or invent funded promises.
