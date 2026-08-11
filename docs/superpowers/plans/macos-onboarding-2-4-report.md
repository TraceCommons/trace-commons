# macOS onboarding screens 2 and 4 -- build report

Scope: build only onboarding screen 2 ("Connect") and screen 4 ("Extra
privacy scan"). No chaining across the six-screen flow was attempted, per
instructions.

## What was built

### Screen 2 -- `Views/OnboardingConnectView.swift`

- Invite paste field (`TextField` + "Look up" button, `onSubmit` wired too)
  plus `tracecommons://enroll?invite=<encoded-invite-url>` deep-link
  handling via `.onOpenURL`.
- "Resolve and show the instance before committing" is done **entirely
  client-side**: an invite link is `https://issuer.example/onboard#CODE`
  (mirrors `parse_invite` in
  `crates/trace-commons-contributor/src/commands.rs`), so the issuer host
  is visible from the URL alone. No network call happens until the person
  taps "Join <host>". Only that tap drives the real `enroll` IPC call.
- `enroll` is called with `invite` only -- never `allowed_hosts` (the
  contract says the daemon does not accept one from a socket caller at
  all).
- Any enroll failure (and any locally-unparseable invite string) renders
  exactly: *"This invite link is no longer valid. Ask whoever sent it for a
  new one."* This is deliberate and matches the contract: `enroll` only
  ever reports the generic `unavailable` / `enroll-failed` over the socket,
  by design, because the underlying issuer response can carry a URL or
  response body that must never reach a UI. `AppModel.enroll` bypasses the
  app's usual `lastActionError` label path for this reason -- there is
  nothing more specific to show even if it tried.

### Screen 4 -- `Views/OnboardingPrivacyScanView.swift`

- Gated on `model.daemonSettings?.nearAIConfigured` (from `get_settings`) in
  the `ScrollView`-wrapping `OnboardingPrivacyScanView`; the un-gated
  `OnboardingPrivacyScanContent` is what's rendered directly by
  `DebugScreenshot`, same pattern as `ConsentScopesContent` /
  `OnboardingProjectsContent`.
- Copy is verbatim from the spec's "### 4. Extra privacy scan" block,
  including both required halves of the disclosure: message text is
  transmitted to a named third party (NEAR AI) before Trace Commons ever
  sees it, and if that scanner is unreachable nothing is sent at all
  (traces wait). Neither half was cut.
- The screen's own heading ("Extra scrub before sending? (optional)") and
  the picker option labels never say "PII filter" or "NEAR AI" as a
  headline; "NEAR AI" appears only inside the disclosure paragraph and the
  second radio option, naming who the data goes to, exactly as the spec's
  copy block itself does.
- Choosing "Local scrubbing + NEAR AI scan" and tapping Continue calls
  `AppModel.acknowledgeNearAINotice()` (`acknowledge_near_ai_notice` on the
  wire) before invoking `onContinue()`. Without that call the daemon leaves
  `near-ai-notice-not-acknowledged` set and refuses the filter permanently
  for a GUI-only contributor -- this is the only way this app clears it.

### Supporting changes (outside `crates/`, as required)

- `DaemonClient.swift`: added `enroll(invite:scopes:)` and
  `acknowledgeNearAINotice()` wrappers over the existing IPC plumbing.
- `Models.swift`: added `EnrollResult` (`enrolled`, `tenant_id`,
  `device_key_id`, `consent_scopes`), decoded from `enroll`'s success shape.
- `AppModel.swift`: added `enroll(invite:scopes:) async -> EnrollOutcome`
  (deliberately not the generic `perform` helper, since that helper surfaces
  `failure.message` and this call's message must never reach a screen) and
  `acknowledgeNearAINotice()` (uses the generic `perform` helper, refreshes
  settings + status on success).
- `DebugScreenshot.swift`: added renders for both screens, in three states:
  Connect resolved, Connect dead-invite, and Privacy-scan default.

## Contract read

`docs/contributor-daemon-ipc-v1_1.md`, `enroll` and
`acknowledge_near_ai_notice` entries, read before writing any code. Nothing
in the contract needed to change; no gap found for these two screens (the
"resolve before committing" requirement turned out not to need a new
method, since the issuer host is already visible in the invite URL
client-side).

## Verification

- `swift build`: **passed**, clean except three pre-existing warnings in
  `AppModel.swift`/`TCBridge` (Sendable-closure-capture warnings on
  `TCDaemon`/`TCSubscription`, unrelated to this change and present before
  it).
- `crates/` untouched -- no Rust re-verification needed; `git status` shows
  only files under `macos/` and `docs/` changed.
- `scripts/make-app-bundle.sh` run explicitly (twice, after a mid-review
  tweak) before every capture, per instructions -- `run-demo.sh` only
  rebuilds the bundle when it is missing, and skipping this step would have
  captured stale code.
- Screenshots rendered via the existing `TRACE_COMMONS_SCREENSHOT_DIR` /
  `DebugScreenshot` hook (real `ImageRenderer` output against the shipping
  view hierarchy, not a mock-up), through `scripts/run-demo.sh`.

## Screenshots

- `docs/images/macos-shell-onboarding-connect.png` -- Connect, resolved
  state showing the issuer host before commit.
- `docs/images/macos-shell-onboarding-connect-dead-invite.png` -- Connect,
  the fixed dead-invite sentence.
- `docs/images/macos-shell-onboarding-privacy-scan.png` -- Privacy scan,
  default state (local-only selected).

All three are non-blank; the earlier `ScrollView`-renders-blank problem
documented on `ConsentScopesView` was avoided the same way (a `Content`
type split out of the `ScrollView` wrapper, rendered directly by
`DebugScreenshot`).

## Known issue found, not a contract gap

The invite `TextField` renders as a solid yellow bar with a red "no entry"
glyph in the `ImageRenderer` capture, in both the Connect screenshots. This
is an `ImageRenderer`-only artifact: it appears to rasterize the text
field's insertion-point caret against no live window-server session,
producing the "no entry" cursor glyph baked into the bitmap instead of a
blinking caret. It reproduces identically under `.textFieldStyle(.plain)`
and `.textFieldStyle(.roundedBorder)`, ruling out the field style as the
cause. It does not appear in the running app -- only in this offline
capture path -- and every field around it (the resolved-instance text, the
button, the dead-invite sentence) renders correctly. Documented in a code
comment in `OnboardingConnectView.swift` rather than worked around further,
since this is the first `TextField` in the app and no prior view in this
codebase established a pattern for avoiding it.
