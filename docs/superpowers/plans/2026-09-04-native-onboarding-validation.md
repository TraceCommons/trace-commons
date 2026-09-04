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
