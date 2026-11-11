# Private AI, managed tools and account services

Baseline `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`; native macOS reference plus shared daemon. No selected promises or checks beyond typechecking.

## Separate state and bounds

Account, browser-attempt identity, management session, inference key, credential generation, spending organization, replacement-key phase and old-key revocation remain separate. Three reusable browser-attempt names, two organizations and three reusable generation names expose cancellation/late-result boundaries without an unbounded identifier space. A management session can expire while inference key remains (`nearai_credential/mod.rs:300-304`).

Listener enablement, harness plan/file revision/commit, backend availability, balance read, managed selection generation and prepare request are independent. Two durable managed sessions have their own owner/generation/connection/ticket/terminal/redeemed/stop/collection/native-conversation/visibility fields. Only one prepare operation is in flight. Tickets and leases use ready/expired relations, not real seconds. Profile cached status and authoritative publication, contribution credit/referral read, rail link and device revocation are coarse independent state.

## Source-grounded transitions

- Browser session/key lifecycle, named cancel, forget/sign-out and organization endpoints: `crates/trace-commons-contributor/src/daemon/nearai_credential/mod.rs:107-169`, `:300-350`, `:446-518`, `:624-662`; commit ownership interface `nearai_credential/ceremony.rs:1-35`; organization key identity/propagation/revocation `nearai_credential/spend_org.rs:156-201`, `:234-249`, `:332-340`. Exact callback proof parsing, commit lock and retry internals are coarse validated external outcomes here.
- macOS credential poll notification epoch checks: `macos/Sources/TraceCommonsApp/AppModel.swift:877-925`. These are not generalized to every callback; profile callbacks below lack an equivalent guard.
- Harness exposure, plan, cancellation and confirmation: `macos/Sources/TraceCommonsApp/AppModel.swift:633-754`. Enabling listener may precede failed planning. The committed held plan can fail or expire and is not silently retried.
- Managed prepare waits for balance and rechecks sign-in key and selection generation: `crates/trace-commons-contributor/src/daemon/managed.rs:324-325`, `:1060-1109`. Terminal launch hold versus pre-open failure versus unknown outcome: `:155-247`.
- Actual session states Starting, Running, Exited, Failed, Unknown, ticket life and lease mapping: `crates/trace-commons-contributor/src/managed/sessions.rs:20-60`. Prepare and redemption `:903-979`, `:1206-1244`; lifecycle report/collection `:1248-1367`, `:1403-1435`; stop request versus actual exit `:650-673`; restart/resume `:676-738`; ticket/lease expiration and lost-sign-in closure `:823-881`. Registry open after daemon restart is not reduced to UI closure.
- Native profile publication and cache-write outcome: `macos/Sources/TraceCommonsApp/AppModel.swift:3509-3611`. The result applies the captured request owner without a fabricated account-generation guard. `profileOwner` can remain the earlier request owner after an account change; this is an observation in the source projection, not an asserted defect or selected guarantee.
- Contribution account/credit/referral refresh guards: `macos/Sources/TraceCommonsApp/AppModel.swift:291-319`. Amount/count projections are coarse and do not issue funds.
- Payout link owned-org listing revision, pending handle, confirm/unlink: `crates/trace-commons-contributor/src/daemon/payout_link.rs:660-894`; separate from pipeline settlement/NEAR issuance. Device operations are a coarse remote authority boundary (precise route anchors assigned by coverage inventory).

## Frame/copy provenance

These are reference projections with authored state descriptions, not full native layout/copy reproductions. Private AI/Sign in/Cancel/Connect/Disconnect/Apply/Spending organization/Billing labels come from `crates/trace-commons-contributor/src/private_inference_copy.rs:212`, `:957-981`, `:1352-1360`, `:1930-1982`. Managed Launch/Refresh/Stop/Start again/Resume/Dismiss are `crates/trace-commons-contributor/src/managed/copy.rs:254-269`, `:367-371`. Profile Update profile/Withdraw delegate through `macos/Sources/TraceCommonsApp/Views/PublicProfileCopy.swift:23-26` to shared shell words. Account/Private AI/Close/Open represent navigation/window events, while Select organization, Select session and Revoke device are explicitly abstract row selectors/authority inputs, not asserted literal button text. Link/Unlink/Confirm project the existing payout-link flow; complete text and all confirmations remain omitted.

## Deliberate coarse detail and gaps

The row selector for the second session allows independent session reports and stops; exact terminal process ownership, OS permissions, late report acceptance, CLI-owned session stopping limits, collection plans and conversation-root replay are not fully modeled. Dismiss eligibility currently coarsens the detailed conversation/profile conditions. The `connection` type includes non-NEAR paths but initialization/launch actions currently create only NEAR AI sign-in sessions; Unknown lease behavior is encoded but not reachable without a non-NEAR launch refinement. Full API-key/subscription account add/rename/remove/reconnect and model favorite/default/background settings are inventory coarse interfaces, not hidden claims of coverage. Provider catalogue/routing policy alternatives are represented by backend/provider fields, not full model catalogue selection.

Org switch cancellation after mint begins is deliberately absent: this projection does not claim cancelling a visible window undoes an in-flight native mutation. Key propagation failure/retry and revocation sweeper retain only the externally visible outcome. Browser provider selection, organization membership changes, credential storage/OS-keychain errors, balance receipt freshness and spending history are coarse. Profile fields/bio validation/invitation URLs, referrals and funding are coarse read/mutation interfaces. Device lists/revoke-other versus revoke-current, authority renewal and current-device IPC consequences require additional authority detail and are declared gaps. Windows/GTK/Tauri affordances and backend capabilities are not assumed identical.

## Composition hooks

`TheBrowserLoginCommitsCurrentCredential` supplies inference key plus independent management session; `TheOrganizationSwitchCommits` changes key/org identity before `TheOldOrganizationKeyIsRevoked`; `TheManagedLaunchIsPrepared` creates a durable starting row after guarded precheck; `TheLauncherRedeemsItsTicket`, `TheToolReportsRunning`, `TheToolReportsNormalExit` and `TheManagedCollectionCompletes` separate process and trace collection. `TheLostLoginEndsManagedSignInSessions` is a subsequent lifecycle effect. Profile publication/cache result, credit read, payout-link confirmation and device revocation each export a distinct boundary event; none stands in for server settlement.

No universal wait/stutter action is enabled. Timing and fairness remain unselected; no deadlock or progress claim follows from this model.

Device boundary exact dispatch: `crates/trace-commons-contributor/src/daemon/ipc.rs:7092-7097`; implementation list/rename/revoke: `crates/trace-commons-contributor/src/daemon/device_codes.rs:228-318`. Transfer-code mint/status/redeem remain coarse inventory interfaces, not represented by revoke-other state.

The contribution-account credit/referral request captures `creditRequestOwner`; mismatched results are discarded in `TheOldAccountCreditReadIsDiscarded`, preserving `macos/Sources/TraceCommonsApp/AppModel.swift:308-311`. This guard is specific to that read, not assumed for profile callbacks.

## Source-review repair: reusable finite generations

Sign out remains enabled after a login and organization switch: there is no artificial generation-cap guard. Browser sign-in attempts and credential/managed-selection generations use fresh members of finite sets excluding current and captured request identities. This preserves equality/inequality for the explicitly retained outstanding snapshots without unbounded counters or ABA reuse against those snapshots. Three generation names suffice for current plus one captured organization/prepare request; three attempt names exclude current plus retained callback identity. Source actions do not stop at a small numeric cap (`macos/Sources/TraceCommonsApp/AppModel.swift:4763-4792`; daemon organization identity check `managed.rs:1094-1102`). Unretained historical requests remain outside the one-request abstraction; these finite names do not claim to model an unbounded request history.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees; owner promise selection and TLC checks remain pending.
