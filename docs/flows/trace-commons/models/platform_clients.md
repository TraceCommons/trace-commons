# Windows, GTK and Tauri platform boundaries

Source baseline and inspected worktree HEAD:
`8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`.
Native macOS remains the reference in `client_shell`; this module models
additional current shell behavior. It does not assert parity or platform release
availability. Only the five `platform_clients` artifacts are owned by this slice.
Untracked implementation files are excluded from the source baseline.

## State, finite bounds and rendering

`platform` selects Windows, GTK or Tauri. `alive`, `visible`, `screen`, `page`,
`first`, `quit` and `changeOpen` describe presentation and process lifetime.
`owner` is none/hosted/attached; `external` describes a separate watcher and
survives UI-only exit. `roots`, `setup`, `enrolled`, `scopes`, `path`, `scrub`,
`witness`, `grant` and `savedRevision` are separate persisted/progress facts.
`req`, `subscription`, `auth`, `change`, `changeSent` and `update` describe
independent unfinished work. `answer` is an observable response category, not
untrusted message text. Local account-session authority (`session`) is distinct
from enrollment and device authority.

This is one shell process per trace, one selected platform, one enrollment,
one settings edit, one ordinary daemon request, one independent browser session,
one native login-change dialog and one external watcher. Platform selection and
persisted roots/setup, tray availability, mission visibility, second-scanner
availability and installation ownership are finite environment inputs. Account
identity and two-account authority races are delegated to `client_services` and
`server_identity`; no identity-separation result follows from this single-account
shell projection. Actual paths, tenants, tokens, endpoints, witnesses, selected
scope sets and project rows are coarse shared-boundary inputs.

Settings revisions and native Windows dialog generations each reuse two values.
`authGeneration` captures the native account-change request's generation;
`changeSent` retains its stage. There is at most one outstanding native change
reply, and beginning another dialog is disabled while that reply occupies `req`.
Closing the dialog toggles the generation without disabling close, quit or setup
at a numeric ceiling. Thus this finite projection distinguishes current versus
cancelled replies without an unbounded counter or eventual action exhaustion;
it does not cover overlapping replacement dialogs or integer wraparound. The
Tauri withdrawal callback deliberately has no invented generation guard.

Every frame has `kind: server`: these are internal cross-platform explanatory
panels, not proposed UI or exact native screenshots. `first` preserves all six
native stages and twelve Tauri stages while `platform_first_run` merges their
rendered panel. `page` preserves each route while `platform_workspace` merges
ordinary destinations. Updater phases share `platform_update`; `change` stages
share `platform_change`; connection checking/start share `platform_starting`.
The view prioritizes stopped/quit/hidden/dialog/browser state over the underlying
page. Navigation can still change the underlying `page` during a browser wait.
Controls name action groups and union the relevant platform affordances; action
guards select the currently applicable subset. They are not a literal copy audit.

## Executable source correspondence

All paths below are repository-relative. Source anchors resolve current behavior;
source comments describing desired parity do not establish parity.

| Source entry point | Model actions and fields | Preserved behavior / boundary |
| --- | --- | --- |
| `crates/trace-commons-contributor-gtk/src/main.rs:142-185`, `:231-317` | `YouOpenTheGtkContributor`, `TheStartNeedsSourceDeclarations`, `YouDeclareTheSources`; `screen`, `roots` | Ordinary GTK launch uses daemon-free local Insights, resolving contributor state only on the subsequent contributor request. The full daemon window starts on Queue. Explicit headless/developer start-page variants are outside this module. |
| `windows/src/TraceCommons.App/DaemonHost.cs:181-204`; `ViewModels/MainViewModel.cs:1412-1443`; `ViewModels/SessionRootsViewModel.cs:173-197` | `TheShellHostsTheWatcher`, `WindowsFindsAnotherOwner`, `TheStartFails`, source declaration actions; `owner`, `screen`, `roots` | Native Windows constructs the FFI daemon off the UI thread and subscribes after adoption. It has no attach fallback. Missing roots has a dedicated UI answer. A second owner leaves a navigable presentation surface with no daemon. |
| `crates/trace-commons-contributor-gtk/src/backend.rs:144-180`, `:188-200` | `TheExistingWatcherAttaches`, `TheShellHostsTheWatcher`, `TheExternalWatcherDisappears`; `owner`, `external`, `subscription` | GTK attempts attachment before enforcing the new-host roots gate. Lost-lock startup falls back to attached mode. A later missing watcher reports daemon-not-running; there is no invented automatic GTK reconnect supervisor. Generic startup failure exits rather than silently adopting macOS behavior. |
| `tauri-desktop/src-tauri/src/runtime.rs:17-64`, `:91-118`; `src-tauri/src/state.rs:222-280` | Startup actions, `TauriRecoversItsConnection`; `owner`, `roots`, `subscription` | Tauri starts embedded or attaches after already-running. Undeclared roots leave a usable first-run frontend, whose Roots stage can start the daemon later. External replacement closes the old attachment. Recovery failure/delay is coarse. |
| `windows/src/TraceCommons.App/MainWindow.xaml.cs:670-726`; `windows/src/TraceCommons.Interop/OnboardingState.cs:74-106`; `crates/trace-commons-contributor-gtk/src/ui/onboarding.rs:114-172` | `TheStatusRequiresSetup`, `TheStatusRestoresCompletedSetup`; `setup`, `enrolled` | Completion marker/shared CLI proof and daemon enrollment are distinct. Native markers are per tenant. A successful enroll by itself does not skip Consent/Watch. Two-tenant marker identity is outside this single-enrollment bound. |
| `windows/src/TraceCommons.App/ViewModels/OnboardingViewModel.cs:403-455`, `:515-568`, `:656-673`, `:727-735`; GTK `ui/onboarding.rs:49-67`, `:459-469`, `:867-890`, `:962-980` | `YouContinueFirstRun`, `YouEnrollFromConnect`, `EnrollmentAnswers`, `YouChooseAndSaveScopes`, `ScopeSavingAnswers`, `YouFinishNativeWatching`, `TheNativeCompletionReadAnswers`; `first`, `req`, `setup`, `scopes` | Native flow is Welcome, Connect, Consent, optional Scan, Watch, Done. Both native scope-save handlers advance even when the daemon reply is an error: Windows ignores the awaited response, GTK callback ignores `_result`. This current-behavior difference from success-gated Tauri is retained. Watch's status/marker step may fail to persist completion while still reaching Done. |
| `tauri-desktop/frontend/src/features/onboarding/flow1.ts:73-153`, `:165-226` | `YouContinueFirstRun`, `YouChooseTheTauriPath`, `YouDecideScopesLaterInTauri`, `YouGoBackInTauri`, `YouRequestTheTauriGrant`, `TheGrantAnswers`; `first`, `path`, `scrub`, `witness`, `grant` | Tauri has Welcome/Roots/Connect/Consent/Path/Privacy/Inference/Scrub disclosure/Witness disclosure/Grant/Projects/Done. Decide later bypasses saving scopes and uses Ask. Back clears disclosures; automatic grant requires enrollment, scopes, path and both disclosure stages. Scope-member selection and actual witness identity are shared-domain inputs. |
| `windows/src/TraceCommons.App/MainWindow.xaml:137-336`, `MainWindow.xaml.cs:1090-1125`; GTK `ui/mod.rs:81-137`, `:357-384`; Tauri `frontend/src/app/routes.ts:1-86`, `app-routes.tsx:40-139` | `YouNavigate`, `TheMissionSettingChanges`, `YouOpenTauriAutomaticConsent`; `page`, `screen`, `missions` | Windows/GTK expose Waiting/Queue, History, Private AI, Insights, Mission drafts and Settings. Tauri also exposes Compute/Profile, defaults to Insights, redirects a hidden mission route to Insights and has NotFound. GTK mission visibility is latched for the process; Windows/Tauri can receive a changed setting. Native Settings is a pane/stack page, Tauri Settings a route; none acquires a separate macOS Settings window. Tauri automatic setup reuses consent through grant without unenrollment. |
| GTK `ui/mod.rs:725-765`; Windows `DaemonHost.cs:210-233`; Tauri `src-tauri/src/ipc.rs` | `YouSaveSettings`, `SettingsSavingAnswers`; `req`, `revision`, `savedRevision`, `answer` | Daemon calls run outside the UI loop and can answer after navigation/close-setup. Saving is one coarse IPC request rather than a complete settings schema or per-widget request-cancellation model. |
| GTK `worker.rs:119-158`, `ui/mod.rs:584-637`; Tauri `src-tauri/src/app.rs:182-205`, `:211-285` | `TheSubscriptionNeedsResync`, `TheSnapshotArrives`, `TheExternalWatcherDisappears`, `TauriRecoversItsConnection`; `subscription` | GTK has separate request and event threads. Resync refreshes queue/state and sweeps outstanding previews; ordinary preview-ready uses a targeted refresh. Tauri forwards allowlisted invalidation names, retries subscription, and recovers closed attachments. The model preserves freshness categories, not raw queue snapshots or periodic/backoff timing. |
| Tauri `src-tauri/src/commands/history.rs:15-59`; `src-tauri/src/state.rs:144-166`; `src-tauri/src/commands/platform.rs:42-50`; `frontend/src/features/history/hooks/use-history-withdrawal.ts:40-87` | `YouSignInForTauriWithdrawal`, `TheBrowserUrlArrives`, `YouReopenTheAuthorizedBrowserUrl`, `TheAccountCallbackAnswers`, `TheAccountSessionIsReread`; `auth`, `session`, `fallback`, `fallbackUsed` | Separate account-auth callback can finish after route navigation. Rust authorizes one exact fallback URL for one use and clears it when sign-in finishes. A callback accepted by account_auth is followed by frontend session reread, which can fail or show inactive. A page unmount does not become an invented backend cancellation. |
| Windows `Interop/AccountLoginChange.cs:644-668`, `:698-714`, `:729-762`, `:792-832`; GTK `ui/account_change.rs:123-147`, `:163-171`, `:327-388` | `YouBeginNativeAccountChange`, `YouStartTheChangeLeg`, `TheChangeLegAnswers`, `YouCloseAccountChange`; `change`, `changeOpen`, `changeSent`, generations | Native F10 dialog adapters hand browser ceremony to shared daemon. Windows captures generation for leg/confirm/reread replies and closes locally before cancellation IPC. GTK reread replaces retained state without that guard, even if its window was closed. The coarse stages preserve that response-lifetime difference; full current-proof/wallet/held-login/retry page mapping stays in shared ceremony domain. |
| GTK `ui/settings.rs:1217-1243`; Tauri `frontend/src/features/settings/components/account-attention-status.tsx:15-54`; Windows `ViewModels/ContributorSettingsViewModel.cs:1264-1305` | `YouConfirmSetUpAgain`, `SetUpAgainAnswers`; `req`, `enrolled`, `setup`, `answer` | Confirmed set-up-again requests unenrollment, and only successful removal enters first run. Private-AI credential sign-out, remote revoke versus local clear failure, and any credential left behind are shared identity/service boundaries, not equated with the Tauri withdrawal account session. |
| Windows `AppUpdater.cs:57-85`, `:102-158`; `ViewModels/MainViewModel.cs:1621-1654` | `YouCheckPlatformUpdates`, `ThePlatformUpdateCheckAnswers`, `YouApplyThePlatformUpdate`, `WindowsQuiesceAnswers`, `TheInstallerAnswers`; `installed`, `update`, `owner`, `subscription` | Unpackaged Windows reports unknown/unmanaged. MSIX handoff happens after quiesce and host disposal; failed handoff leaves owner none. Successful handoff may terminate before await returns. No claim of deployment success follows from an accepted request. |
| GTK `update.rs:47-60`, `:116-145`; `ui/update.rs:183-232` | `TheGtkPortalReportsItsState`, `YouApplyThePlatformUpdate`, `TheInstallerAnswers`; `installed`, `update` | GTK starts a portal monitor independently of navigation; source builds are unmanaged and make no install call. Portal success means new bytes on disk while current process is old. The restart button invokes the ordinary close confirmation and does not relaunch. Portal progress percentages and confirmation presentation are coarse. |
| Tauri `src-tauri/src/commands/platform.rs:105-124` | `YouCheckPlatformUpdates`; `update=guidance` | Homebrew-managed or installer-managed instructions, both feed_configured=false. This has no Swift/Sparkle or native Windows/GTK update installation transition. Installer/Homebrew subcategories share one rendered/guidance state. |
| Windows `MainWindow.xaml.cs:183-204`, `:204-284`, `:1766-1779`, `DaemonHost.cs:378-404`; GTK `ui/mod.rs:642-693`; Tauri `src-tauri/src/app.rs:405-410`, `:465-466`, `commands/platform.rs:181-184`, `frontend/src/app/quit-confirmation.tsx:26-71` | `YouCloseTheWindow`, `YouRaiseTheTrayWindow`, `YouRequestQuit`, `YouCancelQuit`, `YouConfirmQuit`, `YouLaunchAgain`; `visible`, `alive`, `owner`, `external`, `quit` | Windows reachable tray hides without disposal; without tray close confirms quit. GTK local Insights closes normally; full GTK attached UI may leave external watcher or request shutdown too. Tauri prevents close and emits quit-requested to React; exit cleans runtime. Attached Tauri exit closes attachment rather than stopping external watcher. These are distinct from macOS independent windows and menu-bar lifetime. |

## Shared boundary and reference-model correspondence

The shells do not reimplement discovery, approval, upload, witness, processing,
settlement, credentials or account authority here. Requests are modeled as
bounded shell invocations and coarse replies. The common IPC dispatcher in
`crates/trace-commons-contributor/src/daemon/ipc.rs:94`, `:4005`, `:7079`
connects socket and local callers to the same Rust behavior. That code sharing
does not make shell lifetime or navigation equivalent.

| `platform_clients` event/projection | `client_shell` reference / shared owner | Correspondence limit |
| --- | --- | --- |
| Start/attach; `owner`, `subscription` | `TheDaemonStarts`, `TheExistingDaemonAttaches`; `daemonOwner`, `subscription` | Windows excludes attach; GTK local entry and attach-before-roots differ; Tauri recovery is explicit here. |
| `EnrollmentAnswers`; `enrolled`, `setup` | `TheInviteEnrollsThisMac`, `TheAccountEnrollmentCompletes`; `enrollment`, `completion` | Enrollment is shared daemon authority. Native per-tenant completion/read failures and Tauri first run remain distinct shell facts. |
| `ScopeSavingAnswers`, `TheGrantAnswers`; `scopes`, `grant` | `TheConsentScopesAreSaved`, `TheAutomaticGrantIsSaved`; `client_contribution` | Native scope errors can still advance. Shared Rust still owns actual scopes/grants; no UI advance is promoted into a saved grant. |
| `SetUpAgainAnswers`; enrollment removed | `TheUnenrollmentCompletes`; `client_services` | The event is confirmed unenrollment, not simple Private AI sign-out. Credential and account-session revocation are not inferred. |
| Native login-change and Tauri withdrawal callback | `client_services` identity/ceremony boundary, `server_identity` account authority | This model owns shell adapter lifetime only; shared models own account/device/provider/organization identities and server validation. Native wallet adapters (`windows/src/TraceCommons.Interop/NearAccountConnection.cs:37-78`, GTK `ui/onboarding_wallet.rs:126-188`) call `native_wallet_flow`; their transport/poll/cancel plumbing is a coarse interface, not the Tauri account_auth callback. |
| `TheSubscriptionNeedsResync`, `TheSnapshotArrives` | `TheSubscriptionFallsBehind`, `TheExplicitResyncCompletes` | Categories agree on refresh-needed/refreshed, not an atomic snapshot across every service. |
| Close/quit/update transitions | `YouCloseTheWindow` and native lifecycle/update boundaries | No blanket window lifetime, settings-window, updater or graceful-shutdown parity. Shared quiesce is a boundary result, not duplicated queue/drain logic. |

Detailed models checking independently would not establish composition. This
module does not import the shared daemon's mutable state or claim that similarly
named shell actions prove an end-to-end guarantee. The suite owner must map
selected boundary fields to the composition before checking any promises.

## Explicit omissions and coarse primitives

This module is a finite shell-difference projection, not every platform widget's
loading/error/empty state. Source indexes distinguish components from separately
reachable screens. Native wallet callback parsing, browser/provider/organization
choice, invitation URL parsing and redirection arbitration, local sheets, legacy
move/funding/deposit, source-row project actions, notification/login-item/folder
portal UI, and individual per-screen stale-response guards require the owning
shared domain or further platform projections. Declaring those shared interfaces
does not certify that every shell renders each reply correctly.

Native F10 start is represented by opening the first available proof stage;
start RPC failure, held-login skipping, detailed wallet/current-provider proof,
multiple concurrent status polls and retry-page variants are coarse. Failure is
one terminal error category requiring reopening. Windows/GTK native-wallet
adapters' close/cancel/late-start handling is documented at the boundary above,
not exhaustively modeled. Native setup windows are represented separately from
the main destination by first-run state, but simultaneous main/setup/settings
windows and focus are not a Cartesian product. Tauri tenant-keyed session fallback
when localStorage writes fail is coarsened to completion; process restart does
not explore loss of that session-only fallback. Setup identity changes require
`client_services`; this module keeps a single enrollment/completion marker.

The single `req` bounds ordinary concurrency. A native setup callback can still
arrive after close-setup/navigation, and browser/subscription/update events have
independent actions. Crash recovery, request cancellation at the OS/FFI boundary,
Windows disposed-handle drain/leak outcomes, exact channel lag, daemon scheduling,
GTK sequential worker job order and Tauri recovery retry delays are not proved.
Failed source declarations are a shared settings interface; the model only
enters startup after a successful declaration. Tauri startup retry may remain
offline indefinitely. No fairness, guaranteed callback, guaranteed installer
answer, universal progress or unconditional stutter is supplied.

OS APIs, browser launch, redirect socket parsing, credential-store/keychain
secrecy, named-pipe ACLs and Unix socket/filesystem permission checks, D-Bus portal
identity, MSIX signatures/installer policy, Homebrew detection correctness,
service managers, Rust/FFI memory safety and actual process cleanup remain
external primitives. In particular,
`crates/trace-commons-contributor/src/daemon/win_pipe.rs:166` and
`daemon/ipc.rs:9505` are runtime access-control boundaries, not results proved by
Quint typechecking. Legacy native callback response-before-validation and lack
of HTTP-method checking (`account_auth.rs:290`, `:349`) remain transport facts;
`TheAccountCallbackAnswers` means account_auth returned accepted/failed evidence,
not that a browser success page by itself authenticated anyone.

## Validation status

On 2026-11-05, Quint `0.33.0` successfully typechecked:

```sh
quint typecheck docs/flows/trace-commons/models/platform_clients.qnt
```

The config contains only `INIT q_init` and `NEXT q_step`. No invariant,
property, temporal formula, product-guarantee test, simulation, TLA+ compilation
or TLC run has been added or executed for this module before owner selection.
Typechecking establishes syntax/types, not behavioral correctness or a promise.
The 15 panels and their controls are declared artifacts; suite-level rendering
and control-correspondence validation remain the integration owner's checks.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees; owner promise selection and TLC checks remain pending.
