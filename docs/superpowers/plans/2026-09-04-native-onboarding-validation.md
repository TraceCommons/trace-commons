# Native review and first-contribution validation

This slice adds explicit remote witness review to macOS, Windows, and GTK.
The action requires the daemon's `hello.methods` capability, a pinned witness,
and a separate disclosure confirmation. Initial native requests omit outcome
and correction; certified previews disable editing these values. A failed or
lost response never says that the session stayed on the device. Requests are
not retried automatically and do not approve a contribution.

A successful `witness_preview_request` returns `status: ready`. The shell then
opens the saved preview using the existing rendering/approval path. GTK attached
mode now assembles `preview_body` pages with stable envelope/body digests, exact
byte offsets, stable totals, and a 16 MiB bound; incomplete or changed pages
cannot produce a reviewable body. This also covers the systemd daemon path.

The first-contribution guide stays available while local history has no recorded
contribution. It distinguishes finishing an agent task, review, upload, server
acceptance, and credit. No local points or configured proxy prove provider funds.
Existing-account agent instructions do not create credentials or enable capture.
Browser signup and verified admission binding are coordinated separately.

## Reproducible local checks

Run in the worktree root. Set `TC_FFI_LIB_DIR` to the integrated contributor FFI
build, not a dylib predating the shared `review` and `onboarding` copy objects.

```sh
TC_FFI_LIB_DIR=<integrated-target>/debug CLANG_MODULE_CACHE_PATH=/tmp/native-flow-clang-cache \
  swift build --disable-sandbox --cache-path /tmp/native-flow-swift-cache --package-path macos
TC_FFI_LIB_DIR=<integrated-target>/debug CLANG_MODULE_CACHE_PATH=/tmp/native-flow-clang-cache \
  swift test --disable-sandbox --cache-path /tmp/native-flow-swift-cache --package-path macos \
  --filter 'NativeWitnessReviewTests|WitnessSurfaceTests|WitnessExportTests|WitnessBindingTests|SetSettingsTests'
TRACE_COMMONS_SCREENSHOT_DIR=/tmp/native-flow-render TC_FFI_LIB_DIR=<integrated-target>/debug \
  CLANG_MODULE_CACHE_PATH=/tmp/native-flow-clang-cache swift test --disable-sandbox \
  --cache-path /tmp/native-flow-swift-cache --package-path macos --filter NativeOnboardingRenderTests
DOTNET_ROLL_FORWARD=Major NUGET_HTTP_CACHE_PATH=/tmp/native-flow-nuget-cache \
  TC_FFI_LIB_DIR=<integrated-target>/debug dotnet test windows/tests/TraceCommons.Interop.Tests \
  --filter 'FullyQualifiedName~Witness|FullyQualifiedName~Onboarding'
RUSTFLAGS='-D warnings' cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml witness --locked
RUSTFLAGS='-D warnings' cargo clippy --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml \
  --all-targets --locked -- -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
```

Observed: full Swift build; 81 selected Swift tests; one synthetic render test;
28 Windows interop/onboarding tests; 26 GTK witness tests. The render test uses
an unstarted model and shared copy only, never real sessions or live services.
The consent and empty-history images were inspected for legibility and clipping.

Full Windows build was attempted using `EnableWindowsTargeting=true`, ARM64,
and `RuntimeIdentifier=win-arm64`. Its existing Windows XAML compiler cannot
execute on macOS (exit 126); WinUI compilation/rendering still requires Windows
CI. GTK compiled locally against GTK 4.22.4/libadwaita 1.9.3; its Linux window
rendering is not represented by the macOS SwiftUI snapshots. None of these local
checks establishes live wallet, witness deployment, provider funding, or admission.

## Windows and GTK wallet handoff and next-inference preparation

The wallet path requires all four daemon methods (`near_account_capabilities`,
`near_account_start`, `near_account_status`, `near_account_cancel`). Availability
is checked for the entered commons; signup is never inferred from an invite or
proxy declaration. The exact browser URL must use the same HTTPS host and port,
without user info. Wallet attempts exclude invite enrollment, cancellation also
covers a start response arriving after close, and only `complete` advances to
Consent. Unknown/lost status remains cancellable and never enrolls optimistically.

`prepare_admission_session` is shown only with a true
`admission_evidence_required` setting and the advertised method. Confirmation
names the selected backend and the next request; no retroactive binding is
claimed. Only `ready_for_next_inference` with a future Unix `expires_at` produces
ready wording. The action does not change routing, capture, credentials, or funds.

Checks on 2026-09-04:

- Windows: 18 focused wallet, preparation, and onboarding tests pass; lifecycle
  tests use injected daemon responses and no browser/network service.
- GTK: 10 onboarding tests and one preparation test pass; all-target clippy with
  the repository allow-list passes under `RUSTFLAGS='-D warnings'`.
- Updated Windows XAML parses as XML. Full WinUI compilation remains a Windows
  runner check: this macOS host cannot execute `XamlCompiler.exe` (exit 126).
- GTK `wallet_widget_render` passed under Linux/Xvfb in the existing `tc-gtk`
  image. The final 520 × 550 PNG was inspected: disclosure, both inputs, and
  actions fit. This renders the same widget constructor with synthetic strings
  and constructs no daemon or wallet request. It does not validate a live wallet
  ceremony or the entire onboarding window. Set `TC_WALLET_RENDER_PATH` to save
  its PNG; the exported window includes its opaque background.

```sh
DOTNET_ROLL_FORWARD=Major NUGET_HTTP_CACHE_PATH=/tmp/native-flow-nuget-cache \
  TC_FFI_LIB_DIR=<integrated-target>/debug dotnet test windows/tests/TraceCommons.Interop.Tests \
  --filter 'FullyQualifiedName~NearAccountConnection|FullyQualifiedName~AdmissionPreparation|FullyQualifiedName~Onboarding'
RUSTFLAGS='-D warnings' cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml onboarding --locked
RUSTFLAGS='-D warnings' cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml admission_preparation --locked
TC_WALLET_RENDER_PATH=/tmp/wallet.png xvfb-run -a cargo test \
  --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml wallet_widget_render \
  --locked -- --ignored --test-threads=1
```

## Integrated signup and admission validation (2026-09-04)

The integrated journey now supports: capability check, explicit NEAR wallet
proof and device provisioning, purpose selection, separate body-export consent,
preparation of the selected session's next inference, explicit witness review,
immutable preview, and approved submission. Preparation requires an existing
funded backend and configured IronWire route; it never enables capture or
creates provider funds. Signup does not imply that a contribution was accepted.

Actual restricted-role PostgreSQL tests cover challenge → provider-signed
exchange → signed witness evidence → durable ingest, window exhaustion,
attested submission after exhaustion, and terminal retry after evidence expiry
without duplicate processing cost. The integrated handler test passed locally.
The atomic ledger suite also covers receipt replay, cross-account rejection,
account/global budgets, RLS, and processing locks with a one-slot connection pool.

The complete macOS suite passed 501 tests, including wallet origin restrictions
and the explicit session/backend preparation payload. The final complete contributor
suite passed 1,300 tests (one ignored), including the existing-history profile
and receipt-endpoint followups. Redirect
regressions ensure attestation, raw witness bodies, device claims and bearer
credentials never follow a service redirect. Session persistence failure leaves
wallet signup retryable; malformed browser handoffs cancel their local attempt.

The separate IronWire branch includes final-request metadata insertion before
capture/send and the capture-readiness capability. Its conformance suite passed:
175 core tests, 101 proxy unit tests, 8 passthrough, 9 verbatim, and 15 settings
tests, plus all-target/all-feature Clippy. Native Claude/Codex sessions require a
supported final OpenAI Chat target; supported protocol translation is allowed.

These are local and synthetic results. Live wallet verification, deployed witness
measurement verification and a funded provider inference have not been exercised
as one production flow. Provider credit redemption remains unavailable; no
production configuration, funds, or services were changed. Full WinUI compiler
and render checks require a Windows runner.

### Final integration gates

- Contributor library: 1,300 passed, one ignored, including the existing-history
  profile and receipt-endpoint followups; invited enroll/claim/submit regression
  passed. Both contributor and FFI all-target Clippy passed with warnings denied.
- Native FFI rebuilt; real-daemon body consent round-trip passed. After the
  final profile/endpoint integration, 50 focused Swift/native read-back tests
  passed against the fresh FFI; final preparation snapshot inspected.
- macOS: all 501 tests passed against the rebuilt FFI. AppKit snapshots show real
  controls for wallet signup and preparation; all four snapshots were inspected.
- Windows: 39 integrated witness/onboarding/wallet/preparation interop tests passed.
  This does not substitute for WinUI compilation on Windows.
- GTK: 11 wallet/preparation tests, all-target Clippy, and actual Linux/Xvfb
  wallet render passed. Shared disclosure strings remain identical across shells.
- Server: warnings-denied binary check, all test targets compiled, NEAR scorer
  feature binary check, license-boundary 4/4, and all-target Clippy passed.
- Integrated actual PostgreSQL challenge/witness/ingest/retry test passed.

The test harnesses require local socket access. Sandbox-denied binds were rerun
with that access and passed; they were not counted as product failures or passes.

### Live pilot handoff

Required inputs are an HTTPS test Commons deployment, its validated witness and
issuer configuration, explicit admission limits/runtime database grants, a
matching IronWire build, and an existing funded NEAR AI test configuration.
Use configuration paths or names; never paste credentials into the test record.

1. Verify public capability readiness and perform a real wallet/device ceremony.
   Confirm ordinary unknown-account login still refuses implicit provisioning.
2. Select a contribution purpose. Review existing synthetic history through the
   witness and submit it within the configured allowance; repeat beyond the
   allowance and verify refusal without a new qualifying attestation.
3. Explicitly configure the funded route, local capture, receipt endpoint, and
   separate body-export consent. Prepare one selected session, continue its task,
   then review and approve the exact witnessed artifact.
4. Confirm durable accepted/processing state from the service, account/global
   budget accounting, and retry idempotency. Verify stored redacted artifacts
   contain neither raw request nor raw response bodies. Record hashes and status
   evidence only, alongside the tested build versions and witness measurement.

A live provider charge is confined to the authorized funded test configuration.
This record contains no such live execution yet; successful local fixtures are
not a substitute for it. Credit redemption needs its own verified provider
contract before any earned-credit-to-inference loop can be offered.

### Draft PR platform validation

Coordinated drafts: [Trace Commons #602](https://github.com/TraceCommons/trace-commons/pull/602)
and [IronWire #25](https://github.com/nearai/ironwire/pull/25). Current main was
merged before publishing; its deployment manifests were retained without changes.

IronWire CI passed on Linux and macOS, including all-feature tests, native macOS
app build/tests, packaging, command-line journey, and size checks. Trace Commons
CI exposed a missing GTK caption class, WinUI `IsEnabled` bindings on layout
panels, and migration registration that did not use the repository's explicit
versioned-block convention. The production code was corrected without relaxing
those guards. Windows consent dialogs also use the existing collision guard and
scrolling disclosure. The follow-up CI run is authoritative for these fixes.

The complete local ingest handler suite passed 1,074 tests with two ignored
against the merged main. The existing stylesheet contract suite passed 5/5.
Full Windows compilation and workspace-suite results should be read from the
PR's latest head; earlier cancelled/failed runs do not establish that head's state.

The next CI compiler stage exposed duplicate Windows loading-property names;
local constants were renamed while public binding names stayed unchanged. RLS
coverage now includes V58's creation migration and explicitly checks V58/V59
ENABLE/FORCE and canonical policies without changing historical migrations or
removing runtime registry entries. All 31 default RLS tests and the dedicated
real-PostgreSQL provisioning/admission tests passed.

The pre-review optional PostgreSQL RLS suite ran 30/31: the export fixture
`store_facade_preserves_export_grant_job_scope_and_updates` inserted a random
nonexistent grant and failed its foreign key. The review revision reproduced
that failure on main and repaired the missing prerequisite row without changing
assertions. The full revised suite now passes 32/32 against real PostgreSQL; see
[baseline evidence and repair](2026-09-05-export-fixture-validation.md).


## Review revision, 2026-09-05

The review revision incorporates main's Ed25519 receipt/report work and current
witness deployment pin. Admission evidence v2 signs the model and request byte
length and accepts only explicitly configured Ed25519 gateway keys, model names
and a positive minimum request size. These policy inputs are rechecked at ingest.
Client tests reject validly signed evidence for another account, challenge,
receipt, request, response, model, size or expiry.

New native profiles set both receipt-attestation checking and admission evidence
on. Explicit body-export consent is now required before witness review for these
profiles, including existing-history/window review; disabling consent refuses
before claim or witness network access. This conservative guard prevents a
previously bound session from appearing unbound when the source reader suppresses
its bodies. It does not enable capture or consent automatically.

The wallet lifecycle, origin validation, polling cadence and admission expiry
check are owned by contributor core. macOS, Windows and GTK render shared copy
and refusal states. Failed consent saves reread settings, and disabled approval
controls have the immutable-artifact explanation beside them.

Local verification during this revision:

- Contributor library: 1,328 passed, one ignored.
- Server library: 615 passed.
- Real HTTP NEAR window review through `witness_preview_request`, approval and
  uploader: passed; uploaded bytes and both certificate headers equal the witness
  response. Only DCAP cryptography uses a one-shot test fixture; nonce, measurement,
  claim signing, HTTP transport, builder, artifact pin and uploader remain real.
  This case does not claim a live wallet ceremony or funded provider receipt.
- Quote verification: 14 passed, including fixture consumption, nonce rejection
  and the single production construction-site guard.
- Swift app compiled; 49 focused tests passed. Windows adapters/copy/consent: 29
  passed. GTK all-target check and Clippy passed; two focused tests passed, with
  the display-backed wallet rendering test ignored in the local headless run.
- Restricted PostgreSQL retention harness passed: migrator memberships revoked,
  runtime grants remain possible, dry run and batch limit honored, foreign-tenant
  challenges and durable receipt/budget/submission records preserved.
- IronWire revision `cc1d826`: 936 Rust tests passed, two ignored; Clippy and
  optimized privacy-cost checks passed. All five remote CI jobs passed, including
  Ubuntu/macOS tests and the journey harness. A separate local journey run was
  affected by an existing default-port daemon and fixed startup timing; it is not
  cited as a passing run.

The final PostgreSQL checks passed 32 RLS tests, the restricted-role retention
test (including refusal of runtime membership in either guard), and the atomic
admission-ledger test. The main-baseline failure and minimal fixture repair are
recorded in [the reproduction note](2026-09-05-export-fixture-validation.md).
Current-head CI is recorded in PR #602. Earlier results above describe their stated revisions, not subsequent
heads. No live pilot, deployment or provider spending occurred in this revision.
