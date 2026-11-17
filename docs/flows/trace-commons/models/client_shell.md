# Client shell and first run

Source baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`. Native macOS reference. This is a finite projection of current behavior, not a complete implementation or a promise set.

## State and bounds

`s.screen`, `window`, `destination`, `pendingDestination` are presentation; `startup`, `daemonOwner` and `subscription` are runtime; enrollment, roots, sharing, scopes/rules/past-session progress and completion marker are independent durable facts. The six ordered stages are represented individually. Private AI only clears source/rule/scope choices and skips Uses and Traces. Two enrollment states (none/account 1), one settings edit (0/1), abstract live/expired/missing session, and one in-flight daemon call are bounded. Account 2 and credential organization races live in `client_services`; they are not silently explored here.

## Source mapping and transition groups

- Startup/adoption/refusal/attachment/late disposal: `macos/Sources/TraceCommonsApp/AppModel.swift:1696-1805`, `macos/Sources/TraceCommonsApp/DaemonStartup.swift:32-96`, `:122-163`.
- Window/deferred navigation and onboarding gates: `macos/Sources/TraceCommonsApp/MonitorNavigation.swift:89-158`, `:225-310`; `macos/Sources/TraceCommonsApp/MainWindowNavigation.swift:24-55`.
- Stage order, Back retaining facts and clearing grant readiness, Private AI only: `macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift:12-64`, `:79-125`, `:141-191`.
- Roots, invite, browser and account enrollment calls: `macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:139-178`; ordered call execution stops on first failure: `macos/Sources/TraceCommonsApp/FirstRunRunner.swift:182-211`, `:238-307`.
- Completion order: `macos/Sources/TCShellCore/FirstRun/FirstRunPlan.swift:228-286`. Early non-Ask rules and selections precede the grant; Ask rules/selections go after it. Grant refusal can still finish on Ask; marker failure keeps setup unfinished (`macos/Sources/TraceCommonsApp/FirstRunRunner.swift:88-93`, `:171-180`).
- Lag/resync: `macos/Sources/TraceCommonsApp/AppModel.swift:1892-1895`; only managed sessions, queue and status are refreshed here, not an invented full atomic snapshot.
- Account reread/renew/sign-out: `macos/Sources/TraceCommonsApp/AppModel.swift:3743-3855`, `:4755-4792`; `crates/trace-commons-contributor/src/daemon/account_session_renewal.rs:330-446`. Sign-out keeps enrollment; remote revocation and local clear failures are distinct outcomes.
- Settings: `macos/Sources/TraceCommonsApp/AppModel.swift:2120-2264` coarse schema; updater ownership and Sparkle boundary: `macos/Sources/TraceCommonsApp/UpdateController.swift:31-81`.
- Quit refuses calls and ends events before draining handle: `macos/Sources/TraceCommonsApp/AppModel.swift:1924-1948`. Intentional leak versus successful drain is outside this projection.

## Explicit abstractions and gaps

Tool/root paths, individual permissions and bookmark bytes, actual invite validation, required-scope content, disclosure wording and witness address are coarse validated inputs. `TheSharingDisclosuresAreAccepted` represents the existing disclosure result; it is not an automatically supplied promise or user consent. A failed first-run retry resumes the failed call rather than re-emitting the exact idempotent full Start plan. Two folders' before/after-grant ordering is retained as stages but individual folder identities are not. Settings values and updater installation/network phases are external interfaces. The private-AI first-run switch is deliberately absent because current UI no longer sets it (`FirstRunState.swift:118-121`). Old persisted `privateAI=true` callers are omitted.

Closing the window does not cancel daemon work. Presentation assignments by late calls are coarse and can change `screen` while `window=false`; the renderer uses `screen` as a projection, not an actual NSWindow claim. Navigation aliases, menu-only versus foreground/background launch policy details and quit handle-leak diagnostics need implementation-level follow-up for exhaustive coverage. Device management is owned as a coarse authority boundary in `client_services`, not detailed device lists here. Windows/GTK/Tauri do not inherit these macOS first-run stages; see coverage inventory for platform differences.

## Composition events

`TheAccountEnrollmentCompletes` / `TheInviteEnrollsThisMac`: local device enrollment and account authority; `TheConsentScopesAreSaved`, `TheAutomaticGrantIsSaved`, `TheExplicitAskRulesAreSavedAfterGrant`: contribution policy boundary; `TheCompletionMarkerIsWritten`: UI onboarding completion, independent of actual receipt; `TheLocalSignOutCompletes`, `TheUnenrollmentCompletes`: authority removal; `TheSubscriptionFallsBehind` / `TheExplicitResyncCompletes`: read-back freshness. Composition must map these fields and outcomes, not infer transactionality from similarly named actions.

Only typechecking is authorized before promise selection. No invariants, temporal properties, TLC runs or guarantees are encoded here.

No universal wait/stutter action is enabled. Timing and fairness remain unselected; no deadlock or progress claim follows from this model.

## Copy and independent window audit

Executable registration is authoritative: `macos/Sources/TraceCommonsApp/TraceCommonsAppMain.swift:120-153` registers separate First run and Settings windows; `:159-168` hosts SettingsModal inside its own window. `monitorWindow`, `settingsWindow`, `firstRunWindow` retain independent lifetime while `screen` selects the reference frame being inspected. Closing Settings can leave Monitor/First run visible and closing Monitor can leave Settings visible. Exact focus/activation scheduling and independent simultaneous first-run/settings asynchronous presentation remain coarse. Older modal comments are not used as executable behavior.

Actual first-run control copy: `crates/trace-commons-contributor/src/first_run_copy.rs:489-495` Continue/Customize/Back; `:518` Sign in; `:532` Watch; `:565` Use Private AI only; `:605` Ask each time; `:622` Start sharing. Current Join has no Skip (`:144`, `:180`, `:1076`), despite legacy watch-only state/plan support: the invented Watch only control/action was removed. Watch-only completion is retained as an unreachable legacy/API branch, not a reachable native user choice. Use invite, Save settings, Open/Close/Launch and Check for updates represent input/navigation groups and have not all been reproduced as literal native button labels. Frame bodies are authored state explanations and titles are simplified projections, not screenshots or complete live copy.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees; owner promise selection and TLC checks remain pending.
