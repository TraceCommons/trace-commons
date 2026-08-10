# macOS onboarding flow -- build report

Scope: chain the six existing onboarding screens into a working first-run
flow. No new screens were built -- all six already existed
(`OnboardingWelcomeView`, `OnboardingConnectView`, `ConsentScopesView`,
`OnboardingPrivacyScanView`, `OnboardingProjectsView`, `OnboardingDoneView`).

## What was built

### `Views/OnboardingCoordinatorView.swift` (new)

Owns screen sequencing and the one piece of state that must survive a
screen change: the consent scopes chosen on screen 3.

- **Call ordering.** The contract's `enroll` accepts an optional `scopes`
  array at enroll time, but this product's screen order puts Connect
  (screen 2, fires `enroll`) *before* Consent (screen 3, the actual
  decision) -- so `enroll` cannot carry the answer. `enroll` therefore runs
  with no `scopes` (the daemon applies the floor scope,
  `debugging_evaluation`, only), and Consent's `Continue` applies the real
  choice via `set_consent_scopes`, a separate, local-only,
  already-enrolled-only call built for exactly this ordering problem. This
  was read from `docs/contributor-daemon-ipc-v1_1.md` before writing any
  code, per instructions -- not guessed.
- **Back navigation.** Screens 4 and 5 get a "Back to permissions" bar
  (rendered by the coordinator, not added to the screens themselves) that
  returns to Consent. The coordinator holds `selectedScopes` outside
  `ConsentScopesContent`'s own `@State` and re-seeds it via a new
  `initialSelection` parameter added to `ConsentScopesView`/
  `ConsentScopesContent` (mirroring the `previewPhase`/`previewText`
  pattern already used by `OnboardingConnectContent`), so a round trip to
  screen 4/5 and back does not lose what was ticked.
- **Privacy scan skip.** Screen 4 is skipped straight to screen 5 when the
  operator has not configured the second scanner
  (`daemonSettings?.nearAIConfigured != true`), matching
  `OnboardingPrivacyScanView`'s own existing gate.

### First-run detection (`Views/MainWindowView.swift`)

Branches on `model.status.loggedIn` (the daemon's own `status`, per
instructions -- never a file probe) and a new `AppModel.isOnboardingComplete`.
Both "not enrolled" and "enrolled but onboarding unfinished" render through
**one** `if` branch instantiating **one** `OnboardingCoordinatorView`, not
two separate branches each constructing their own instance. This was a real
bug caught during verification (see below): `set_consent_scopes` succeeding
flips `status.logged_in` from stale-false to true on the same turn the
coordinator advances its own `step`; two separate branches would count as
two different SwiftUI view identities and the flip would tear down and
rebuild the coordinator from `startAt: .consent`, discarding whatever step
the contributor had just reached.

### Atomicity (`AppModel.swift`)

Each daemon call (`enroll`, `set_consent_scopes`, `acknowledge_near_ai_notice`,
`set_project_mode`) is individually atomic -- it lands or it visibly fails,
and the coordinator does not advance past a failed one. The six-screen
*sequence* is deliberately **not** atomic; there is no wire-level
transaction spanning it, and no contract change was in scope to add one.

What makes that safe: a new `AppModel.isOnboardingComplete` reads a local
`UserDefaults` marker keyed by `status.tenantID`
(`trace_commons.onboarding_complete.<tenant_id>`), set only when
`OnboardingDoneView`'s button fires (screen 6). `status.logged_in` alone
cannot distinguish "fully onboarded" from "enrolled but consent was never
confirmed," because `enroll` flips `logged_in` true on screen 2, before
consent is even chosen on screen 3, and a floor-only `consent_scopes` array
looks identical whether it came from `enroll`'s default or from a
contributor explicitly choosing nothing extra on screen 3. The two states
are told apart by this local marker instead:

- not `logged_in` -> onboarding from `.welcome`.
- `logged_in` but not `isOnboardingComplete` -> onboarding **resumed** at
  `.consent` (enroll already ran; nothing past it is trustworthy yet).
- both true -> main window.

This is what stops a crash or quit between `enroll` and Done from silently
landing a contributor in the main window with an unset consent choice --
verified for real below, not just argued.

### Supporting changes

- `DaemonClient.swift`: added `setConsentScopes(_:)`.
- `AppModel.swift`: added `setConsentScopes(_:) async -> SetScopesOutcome`
  (bypasses `perform`, like `enroll`, so the coordinator can await and only
  advance on confirmed success), `isOnboardingComplete`,
  `markOnboardingComplete()`.
- `SelfTest.swift`: added an onboarding self-test path (see Verification)
  and a resume-check path, both inert unless their env vars are set.

## Contract read

`docs/contributor-daemon-ipc-v1_1.md` -- `enroll` and `set_consent_scopes`
entries -- confirmed `enroll` takes optional `scopes` at enroll time but
`set_consent_scopes` is the separate, already-enrolled-only, local-only call
this screen order actually needs. No contract gap found; nothing in
`crates/` was touched or needed to be.

## Verification

### Build

```
$ cd macos && swift build
Build complete!
$ ./scripts/make-app-bundle.sh
Build complete!
built .../TraceCommons.app
$ RUSTFLAGS='-D warnings' cargo check --workspace --bins   # from repo root
Finished `dev` profile [unoptimized + debuginfo] target(s)
```

`git status` shows only files under `macos/` changed; nothing under
`crates/` was modified.

### Real end-to-end onboarding, against a real local issuer

No macOS GUI-automation tool exists in this repo (AXe is iOS-Simulator-only)
and this sandbox has no live window-server session (`screencapture` against
the running app returns a blank frame here), so the flow was proven the way
the task asked -- with a stub issuer -- rather than with a screenshot:

1. Ran the **real** `trace-commons-upload-claim-issuer` binary (unmodified,
   from `crates/`) against the local `trace_commons_test` PostgreSQL
   instance, with a freshly generated Ed25519 signing keypair and a
   file-based invite allowlist (`docs/operator/pilot-allowlist.md`'s
   documented flow) -- a real device-key registry, a real `/v1/onboard`
   endpoint, no mocks.
2. Launched the **actual app bundle** (`make-app-bundle.sh`'s output, not
   `swift build`'s raw executable) against a **clean temp state directory
   with no `contributor.json`/device key** -- confirmed absent before
   launch -- so onboarding is what a genuinely first-run contributor would
   see, per `status.logged_in == false`.
3. Drove the exact `AppModel` calls `OnboardingCoordinatorView` makes, in
   the same order, via a new self-test hook
   (`TRACE_COMMONS_ONBOARD_SELFTEST_OUT`/`_INVITE`): `enroll(invite:)` with
   no scopes, then `setConsentScopes([...])` with a chosen set, exactly as
   screen 2 -> screen 3 does on the wire.

Real output from that run:

```
trace-commons macOS onboarding self-test
before: status.logged_in=false consent_scopes=[]
enroll: enrolled=true tenant_id=tenant-onboard-verify consent_scopes=["debugging_evaluation"]
set_consent_scopes: consent_scopes=["debugging_evaluation", "benchmark_only", "public_attribution"]
near_ai not configured; screen 4 would not have shown
after: status.logged_in=true tenant_id=tenant-onboard-verify consent_scopes=["debugging_evaluation", "benchmark_only", "public_attribution"]
isOnboardingComplete after markOnboardingComplete: true
last action error: none
```

Then quit the app and read the **persisted on-disk config** back with a
completely separate process (the real `trace-commons-contributor` CLI,
`whoami --json`, against the same state directory):

```json
{
  "config_dir": "/tmp/tcapp-nwkCAO",
  "device_key_id": "sha256:54e43385eec019eace54e79d31fd20824a42c7fc238006518d13e6cc3407364e",
  "tenant_id": "tenant-onboard-verify",
  ...
}
```

and the raw `contributor.json` it read:

```json
{
  "consent_scopes": ["debugging_evaluation", "benchmark_only", "public_attribution"],
  "tenant_id": "tenant-onboard-verify",
  ...
}
```

The scopes chosen in the self-test (`public_attribution`, `benchmark_only`,
plus the always-on floor) are exactly what landed on disk -- not an
in-memory artifact of the same process.

### Resume-mid-onboarding, across a real relaunch

Repeated the same real-issuer flow against a second fresh state directory,
but with `TRACE_COMMONS_ONBOARD_SELFTEST_SKIP_COMPLETE=1` -- `enroll` and
`set_consent_scopes` both ran for real (`status.logged_in=true`,
`tenant_id=tenant-onboard-verify-2`, the three consent scopes persisted),
but `markOnboardingComplete()` was **not** called, simulating a quit or
crash between screen 3 and screen 6. The app was then killed (`kill -9`)
and **relaunched as a brand new process** against the same state directory,
running only a read-only resume check (`TRACE_COMMONS_RESUME_CHECK_OUT`)
against a fresh `AppModel`/fresh `UserDefaults` read:

```
trace-commons macOS resume check
status.logged_in=true tenant_id=tenant-onboard-verify-2 consent_scopes=["debugging_evaluation", "benchmark_only", "public_attribution"]
isOnboardingComplete=false
would show: onboarding (resumed at .consent, since logged_in is true)
```

This is the exact pair of predicates `MainWindowView` branches on. A fresh
process reading real persisted daemon state (`logged_in=true`, scopes
survived) combined with a real local marker never having been set
(`isOnboardingComplete=false`) proves the forbidden outcome -- silently
landing in the main window with an unconfirmed consent choice -- does not
happen; the app instead resumes onboarding at the consent screen.

## Bug found and fixed during verification (not a contract gap)

The first cut of `MainWindowView` had two separate `if`/`else if` branches,
each constructing its own `OnboardingCoordinatorView(startAt:)` for "not
logged in" vs. "logged in but not complete." Verification showed
`set_consent_scopes` succeeding flips `status.logged_in` on the same
SwiftUI turn the coordinator advances past the consent screen; the two
branches were different view identities to SwiftUI, so the flip tore down
the in-progress coordinator and rebuilt it at `startAt: .consent`,
regressing a contributor who had just reached screen 4/5 back to screen 3.
Fixed by merging both conditions into one `if` constructing one
`OnboardingCoordinatorView` instance for the whole "onboarding not finished"
span (`MainWindowView.swift`).

## Everything that did not survive reading the contract

Nothing. `enroll` and `set_consent_scopes` as documented in
`docs/contributor-daemon-ipc-v1_1.md` were sufficient for this screen order
with no contract change. The one real surprise was operational, not
contractual: this environment's scratchpad path is long enough that
`<state_dir>/daemon.sock` exceeds the 104-byte AF_UNIX limit the app itself
already refuses to exceed (`DaemonHost.swift`) -- state directories for this
verification had to live under `/tmp` (mirroring
`scripts/run-demo.sh`'s own existing "short path on purpose" comment), not
the task scratchpad. This is an environment constraint, not a code change.
