# Native inference onboarding execution plan

Date: 2026-09-04

Branch: `native-onboarding-admission`

Worktree: `.worktrees/native-onboarding-admission`, based on `dd26e016`.
The original checkout remains on `witness-production-config`.

Progress: slice 1 implemented and reviewed. Slices 2-5 remain planned;
public admission and inference funding are not enabled by this branch.

Design: [Native inference onboarding](../specs/2026-09-04-native-inference-onboarding-design.md).
Admission: [Invite or attested inference](../specs/2026-09-04-admission-invite-or-attestation-design.md).

## Delegated work and ownership

| Worker | Bounded task | Owned files |
|---|---|---|
| Primary | Integrate findings, write target design/plan, shared disclosure, verification | Companion docs, contributor `witness_copy.rs`, FFI tests |
| Native surface agent | Audit native flows; implement explicit consent and persistence checks | `macos/`, `windows/` |
| Funding/routing agent | Audit actual funding/routing; implement GTK consent | `crates/trace-commons-contributor-gtk/` |
| Admission agent | Clarify proposed server contract, blockers, tests; review consent security | Original admission spec |

No agent changes deployed services or enables admission, capture, or public
sponsorship. Shared copy is settled before native integration. Review checks
that no shell authors a different privacy claim.

## Slice 1: explicit native consent (this worktree)

1. Extend `WitnessCopy` with the full disclosure, capture/retention note,
   capability limits, consent actions, saved-state words, and save error.
   Reuse `tc_witness_copy`; GTK imports the Rust constants directly.
2. Decode `ironwire_attested_bodies` with legacy default false in all native
   settings models, separately tracking whether the field is supported.
3. Add a disclosure confirmation in each witness settings section. Confirmation
   alone writes true; withdrawal writes false without requiring a working
   proxy/witness. Serialize only this key.
4. Read actual returned settings and require the field to exist and match.
   Keep errors visible and do not treat a failed save as successful consent.
5. Preserve independent consent for privacy scanning and independent proxy
   configuration. Existing enrollment and approval invalidation remain intact.
6. Check old/missing settings, true/false persistence, ignored/malformed
   responses, cancellation, and independent patch scope. Render the native
   disclosure for visual inspection where platform tooling is available.

Exit: all three shells expose the same informed, default-off permission.
This exit does not assert that native witness submissions or funded inference
are operational.

## Slice 2: make invited native witness review work

Dependencies: deployed witness pins and supported policy; usable receipt
endpoint; funded agent routing through IronWire.

- Replace `witness_claim_unavailable` only with a real scoped-claim preview
  path. Reuse direct submission trust and transport checks; do not produce a
  locally redacted substitute when witness processing fails.
- Store the reviewed witnessed envelope as approved immutable bytes. Upload
  those bytes with the bound scopes, subject to current fingerprint checks.
- Add tests for claim failure/expiry, scope mismatch, changed settings,
  body stripping, witness refusal, and identical reviewed/uploaded bytes.
- Pilot one useful session in a native app and verify the stored artifact.
- Replace the desktop-review-unavailable disclosure only once the real
  native path is supported and tested.

Exit: recorded native end-to-end evidence, not just isolated mocks.

## Slice 3: funding and first-contribution progress

- Specify the missing redemption/funding contract before adding spendable
  balance UI. Separate estimates, pending credit, settled credit, and actual
  available inference.
- Provision least-privileged inference credentials and store them through
  platform secret storage. Configure one supported agent only after user
  selection; provide rollback and verify a real provider call.
- Add a persistent activation checklist using observed/server evidence.
  Keep local setup completion and activation separate across restarts.
- Provide existing-history and zero-history entry paths. A sponsored starter
  allowance must have an explicit sponsor, amount, budget, and reconciliation.

Exit: an accepted contribution funds a subsequent successful inference call,
with pending/failed/redemption states represented honestly.

## Slice 4: self-service identity and admission evidence

These can be delegated independently once the contracts are agreed:

- Identity worker: explicit NEAR signup/bootstrap, challenge expiry and
  origin/recipient binding, native browser return, device authorization, and
  account/key lifecycle. Preserve existing-login behavior.
- Capture/witness worker: account-bound challenge insertion into exact
  provider-hashed bytes, transformation compatibility, distinct signed
  certificate profile binding receipt evidence to contributor and artifact.
- Ingest worker: atomic admission/replay/idempotency ledger, attempt and
  processing-cost reservations, cross-device account budget, global ceiling,
  RLS and hash-only audit. No public source-offer gating changes.

Exit: the admission spec's negative/concurrency tests pass. A redaction-only
certificate cannot authorize a single submission. Receipt replay cannot buy
a new admission or a second reward.

## Slice 5: bounded self-service native onboarding

- Publish authenticated/versioned capability and admission state.
- Add the Connect-with-NEAR path only where bootstrap is deployed.
- Guide existing-history users through the window and available credit;
  guide zero-history users through real funding or explain the unavailable
  option. Preserve invited contributors' existing provider choices.
- After exhaustion, require qualifying attested evidence on each submission
  or a valid invite. Preserve local access, review status, and existing funds.
- Roll out to a bounded cohort, measure time to first useful call, first
  accepted contribution, funding latency, rejection reasons, and cost per
  activated account. Expand only within an explicit global budget.

## Validation record

Rust checks use `RUSTFLAGS='-D warnings'`. Commands ran from this worktree;
the primary Rust commands reused the original repository's `target` cache
through `CARGO_TARGET_DIR`. Swift and Windows used that freshly built FFI
library via `TC_FFI_LIB_DIR`. GTK used its own workspace target directory.

| Check | Result |
|---|---|
| Contributor FFI build | Passed |
| Contributor `--lib witness_copy` | 11 passed |
| FFI real-daemon consent round trip | 1 passed; persisted true/false and unrelated settings unchanged |
| FFI whole witness-copy payload | 1 passed |
| Server `license_boundary` | 4 passed; expected sets unchanged |
| Contributor and FFI Clippy, all targets, warnings denied | Passed with repository lint allow-list |
| Root/GTK Rust formatting and diff whitespace | Passed |
| macOS app compilation and focused settings/witness suites | 64 passed |
| Windows Interop consent/settings/witness suites | 29 passed |
| GTK check and final witness suite | Check passed; 24 final tests passed |
| macOS visual review | Actual settings view rendered; disclosure wraps and actions fit |

Representative commands (set `TC_FFI_LIB_DIR` to the FFI build directory):

```sh
RUSTFLAGS='-D warnings' cargo build -p trace-commons-contributor-ffi --locked
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib witness_copy --locked
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor-ffi --test abi --locked inference_
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor-ffi --test abi --locked the_witness_copy_call_carries_the_whole_card
RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --test license_boundary --locked
RUSTFLAGS='-D warnings' cargo clippy -p trace-commons-contributor -p trace-commons-contributor-ffi --all-targets --locked -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
RUSTFLAGS='-D warnings' cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --locked --offline --lib ui::settings::witness_tests
CLANG_MODULE_CACHE_PATH=/tmp/native-swift-module-cache swift test --package-path macos --disable-sandbox --disable-keychain --cache-path /tmp/native-swift-cache --filter 'SetSettingsTests|WitnessSurfaceTests|WitnessBindingTests'
DOTNET_ROLL_FORWARD=Major NUGET_HTTP_CACHE_PATH=/tmp/native-nuget-http-cache dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj --filter 'FullyQualifiedName~InferenceEvidenceConsentTests|FullyQualifiedName~WitnessSurfaceTests|FullyQualifiedName~SettingsProtocolTests'
```

The Swift cache/sandbox options avoid nested sandbox and cache-write failures
on this host. .NET major roll-forward was needed because the installed
runtime is .NET 10 while the test project targets .NET 8.

Native WinUI compilation requires Windows CI and was not run here. GTK's
dialog and the macOS/Windows native confirmation dialogs were not visually
driven. The macOS settings render used an unstarted model and made no live
enrollment, inference, or submission calls. A temporary rendering test was
removed after inspection. No dependency manifests, license-boundary
expected sets, or deployed configuration changed.
