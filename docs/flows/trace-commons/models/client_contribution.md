# Contribution lifecycle

Source baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`. Shared daemon behavior; native macOS labels. Only typechecked, no promises selected.

## State and bounds

Two identified traces (`entries[0]`, `entries[1]`) have separate queue state, source revision, offered revision, pinned-byte identity, artifact existence, approved scopes/inputs, manual/unattended origin, holds, undo deadline, retry deadline/budget, submission ID, proof receipt, account owner and history verdict. The eight actual queue states are Pending, Approved, Uploading, Uploaded, Refused, Failed, Expired, Superseded (`crates/trace-commons-contributor/src/daemon/queue.rs:33-66`); `absent` is only the pre-discovery model state. UI selection, preview entry and uploader entry stay distinct, allowing a response for one trace while another is selected.

Source/input/scope revisions are bounded to one change (0→1), two accounts (1→2), two envelope identities (one per trace), one uploader and preview, and a retry ladder 0→1→2. The numeric retry thresholds, queue/day caps and times are abstract relations, not actual constants. Witness saturation/rate-limit deferral consumes no modeled attempt.

## Source anchors

- Watch/discover/per-project offers/unattended decisions: `crates/trace-commons-contributor/src/daemon/watcher.rs:130-178`, `:953-1091`, `:1576-1778`, `:2025-2123`; project modes `crates/trace-commons-contributor/src/daemon/policy.rs:69-93`.
- Preview persistence versus approval: `crates/trace-commons-contributor/src/daemon/ipc.rs:7286-7440`, `:8342-8459`; pin and scrub consistency `crates/trace-commons-contributor/src/daemon/queue.rs:1860-1953`; manual/unattended approval `:1757-1825`. Manual approve can build without a preview; pin lifetime is separate.
- Keep/undo-keep/dismiss/expiry: `crates/trace-commons-contributor/src/daemon/queue.rs:1298-1381`, `:894`, `:2519-2590`. Expiry is a world event only while healthy; no automatic time fairness is asserted. A kept hold's exact original reason restoration is coarse.
- Undo and atomic claim: `crates/trace-commons-contributor/src/daemon/queue.rs:844-857`, `:1961-1991`, `:2519-2560`; recheck global/policy/current entry before claim `crates/trace-commons-contributor/src/daemon/mod.rs:1310-1385`.
- Enrollment and immutable source/input/artifact checks: `crates/trace-commons-contributor/src/daemon/uploader.rs:380-410`, `:475-652`. The caller reloads/re-hashes the source and reuses pinned bytes rather than rerunning a nondeterministic classifier. Source-byte change becomes Superseded; changed terms become Pending; privacy/admission refusal becomes Refused.
- Scrub/witness holds, missing certificate and capacity: `crates/trace-commons-contributor/src/daemon/uploader.rs:586-654`, `:724-778`; result persistence `crates/trace-commons-contributor/src/daemon/mod.rs:1557-1783`. Manual Scrub check `queue.rs:2018-2039`, `settings.rs:350-374`.
- Network retry, exhaustion, classifier retry and pass interruption: `crates/trace-commons-contributor/src/daemon/mod.rs:1400-1469`, `:1638-1783`; queue retry storage `crates/trace-commons-contributor/src/daemon/queue.rs:2225-2290`. Restart returns Uploading to Approved; no automatic successful send is assumed.
- Delivery versus proof receipt: `crates/trace-commons-contributor/src/daemon/mod.rs:1836-1880`. Missing inference/proof receipt does not undo successful delivery. The local submission receipt/history row remains a separate concept; `receipt=missing` in this model means `ReceiptShipped` proof outcome, not absence of all local receipts.
- History joins/status/current-account classification: `crates/trace-commons-contributor/src/daemon/history.rs:90-105`, `:312-349`, `:499-568`.

## Coarse behavior and omissions

Source paths, filesystem watchers, source formats/grouped subagents, actual privacy cryptography and classifier quality are abstract evidence. Queue cap is not counted across a large collection; two rows cannot reproduce eviction/ordering pressure. Day volume, pacing jitter and exact retry duration are external bounded booleans/relations. One project policy stands for both trace projects; separate grant identities, folder overrides, scoped grants, automatic wording revocation and source-root revision histories need larger models. Discovery permission errors are coarse accessible/inaccessible inputs. A fresh superseding record allocation and old Superseded record retention are not both materialized: Superseded is explicit, replacement allocation remains a documented gap. No `AlreadySubmitted` watcher action is added: `mod.rs:1469-1494` documents that arm currently unreachable through hash-based watcher IDs.

Source/scopes and manual-Scrub retractions are asynchronous projections; folder Ask/Never changes include their immediate queued-row retractions in the same modeled action. Their full lock/interrupt ordering with unenroll is omitted; `TheEnrollmentDisappears` is a pre-send authority input, not a claim that actual unenroll clears files during an active upload. One-shot send success is an atomic external result; the server receipt model owns remote object/receipt commit crashes. Preview races are bounded by one outstanding preview; preview cancellation does not itself delete durable pins. History verdict/credit/withdrawal alternatives are further refined by composition/server lifecycle. The previous-account action models a validated classification outcome, not every receipt age/grace predicate. No temporal assumption forces network, permissions, expiry, review or retries to advance. Folder-Never classification still takes precedence over a global override when a recovered unattended approval is rechecked.

## Composition correspondence hooks

`TheWatcherDiscoversATrace` exposes `(trace, sourceRev, account)`; `YouApproveThePinnedPreview` / `TheApprovalBuildPinsAndApproves` bind `(trace,pin,artifact,approvedScopes,approvedInputs)`; `TheUploaderClaimsTheApproval` sets durable Uploading; `TheUploadPassesItsPreconditions` projects valid authority/evidence; `TheSendSucceedsWithProofReceipt` / `TheSendSucceedsWithoutProofReceipt` separate sent and proof; `ThePrivacyOrWitnessReviewHoldsTheTrace` returns local human review; `TheDaemonRestartsDuringAnUpload` retains pin/approval; `TheHistoryReadFindsProcessing` / `TheHistoryReadFindsASettledVerdict` are client feedback, not server mutation.

All maps and controls are finite. No invariants, tests asserting product guarantees, temporal properties or TLC checks are included.

No universal wait/stutter action is enabled. Timing and fairness remain unselected; no deadlock or progress claim follows from this model.

## Copy and withdrawal audit

Trace tree literal controls are Review, Submit, Keep, Not this one and Undo: `crates/trace-commons-contributor/src/preview_copy.rs:542-565`, `crates/trace-commons-contributor/src/project_copy.rs:178-182`. The reference projection uses those labels. Choose other trace, Policy, History, Back to trace, Change scopes/inputs and source toggles are explicitly abstract navigation/input selectors, not literal shipping button text. Bodies are authored explanations.

`YouWithdrawTheContributedTrace` captures `withdrawalEntry`, then `TheWithdrawalRequestSucceeds` records the confirmed outcome without rewriting the visible history verdict. `TheHistoryReadReflectsWithdrawal` is the subsequent read-back. Refused/failed requests and missing account session remain separate. Source: `macos/Sources/TraceCommonsApp/AppModel.swift:3652-3690`. Distribution reach and token-deletion note remain coarse response fields; remote revocation/derived-index cleanup belong to server lifecycle, not this local event.

## Source-review repair: approval origin, global override and independent queue iteration

`TheUploaderClaimsTheApproval` checks folder mode only for an unattended approval. A manual approval can still send after its folder changes to Never; the separate `globalHold` input blocks both origins. This matches `crates/trace-commons-contributor/src/daemon/mod.rs:1091-1165`, `:1340-1385`, and `policy.rs:678-686`. This distinction is current behavior, not a selected product promise.

Folder Ask synchronously returns Approved unattended rows to Pending and clears their approval/pin terms; folder Never synchronously refuses Pending and Approved unattended rows while leaving manual Approved and Uploading rows alone. Source: `crates/trace-commons-contributor/src/daemon/ipc.rs:5910-5968`; `queue.rs:1480-1532`, `:1640-1681`. The two model rows share one project, so both are affected atomically. A change after claim does not retract the in-flight Uploading row: source says those bytes are already in flight. If the pass aborts/restarts, `release_in_flight` returns it to Approved and the origin-sensitive pre-send retraction runs on the next pass (`mod.rs:1091-1208`, `queue.rs:1981-1991`). Normal send outcomes remain possible after folder/global changes while already Uploading; the model does not invent an interrupt for a project preference change.

`YouHoldEverySend` / `YouResumeEverySend` model the global Never override separately. Holding retains manual approvals and pins, returns queued unattended approvals to waiting, blocks discovery/automatic approval and new manual approval calls, and does not rewrite the folder mode. Clearing releases retained manual approvals. Source: `crates/trace-commons-contributor/src/daemon/ipc.rs:6035-6057`, `:6072-6220`, `:6222-6270`, `:7359-7371`; `policy.rs:678-699`. Hold all sends / Resume all sends are explicit abstract input selectors for the global override, not claimed literal native labels. Global Ask/Automatic override precedence and arming-from-now exceptions remain outside this boolean override projection.

Watcher discovery, source change, preview-pin sweep, unattended approval, undo expiry, offer expiry, changed-scope/manual-Scrub retraction, artifact loss, uploader claim, retry expiry and recovered-approval policy rechecks now choose a daemon entry independently from the UI-selected row. They change the reference frame only when their target is currently selected. This models queue iteration rather than pausing work when the user selects another trace (`mod.rs:1077-1090`, `:1310-1385`; `watcher.rs:1576-1778`). The one uploader still captures `uploadEntry`, so navigation does not redirect an in-flight response. Preview and user actions continue to target explicit captured/current UI entry identities.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees; owner promise selection and TLC checks remain pending.
