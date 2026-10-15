# Saved accounts and managed sessions: validation

Implementation is in the isolated `design/saved-model-accounts` worktree. The
original dirty checkout was preserved. The user approved `keyring = 3.6.3` and
direct `toml = 0.9`; client code remains permissively licensed. After the
rebase onto `main`, saved API keys use the `keyring-core` OS stores `main`
already carries (approved 2026-09-09), so `keyring` 3.6.3 is not added.

## Implemented

- Shared NEAR AI / compatibility CLI, saved account metadata, OS-backed keys,
  isolated native subscription profiles, explicit managed defaults.
- Daemon account/session service with versioned selections, one-use launch
  tickets, authenticated lifecycle reports, durable terminal/failure receipts,
  expiry, and dismissal tombstones that preserve launch idempotence.
- Existing-terminal CLI launches, external-terminal UI handoff, per-session
  NEAR proxy with host-only credentials and no standard/global configuration
  writes. Native profile mutations are refused while the account is held.
- macOS, Windows, and GTK account/session controls using the same service and
  shared Rust wording; event/poll refresh makes CLI sessions visible. Existing
  global-settings preview/commit controls are labelled separately.
- Packaged helper/signing allowlist updates, installer compatibility alias,
  GTK lockfile and checksum-derived Flatpak source updates, user documentation.

## Verified locally

All Rust checks used `RUSTFLAGS=-D warnings`.

- Managed/CLI/evidence integration suites: 31 tests passed after the final
  additions. Coverage includes two distinct managed accounts plus an ordinary
  native fixture running simultaneously; native exit-code preservation; daemon
  downtime receipts; hangup both during setup and after native startup; expired
  tickets; stale selections; profile/config isolation; TOML escaped-key overrides;
  unsupported logout refusing before credentials change; and duplicate requests
  remaining inert after session dismissal.
- Managed unit tests: 2 passed, including an actual translated Anthropic request
  through each of two isolated NEAR proxies to a loopback fixture backend, which
  verifies distinct credentials and a model from the discovered catalogue.
- Existing daemon IPC tests: 169 passed. License-boundary tests: 4 passed.
- Contributor standalone `--no-default-features` check and contributor/GTK
  all-target clippy checks passed with the repository's existing allowlist.
- All four required `cargo deny ... check licenses` invocations passed.
- Swift managed decoding/render tests: 3 passed. The account, session-list, and
  launch-sheet screenshots were rendered from synthetic data and inspected.
- Windows interop tests: 2 passed on macOS using the installed .NET 10 runtime
  with `DOTNET_ROLL_FORWARD=Major` for the net8 test assembly.
- Shell script syntax checks and `git diff --check` passed. GTK source archives
  were checked against every registry checksum in its current lockfile.
- A separate reviewer found lifecycle, compatibility, implicit-selection, and
  model-discovery issues; fixes were verified with regression fixtures. Its final
  focused recheck reported no remaining findings.

## Verification limits

- The full Swift suite reaches one pre-existing wording-ratchet failure: the
  untouched QueueView and WithdrawalCopy files have fewer sentences than their
  untouched baseline permits. New managed wording was moved into Rust and does
  not add a wording violation.
- No real subscription login, API-key write, or paid provider call was made.
  Fixture testing does not replace release smoke tests with supported native
  tools, actual OS credential stores, and external terminal emulators.
- WinUI compilation/runtime, Windows console-window-close behavior, Linux
  runtime, Flatpak build/runtime, signing, notarization, and release deployment
  were not verified on this macOS host. GTK compiled against locally installed
  GTK/libadwaita; that is source validation, not a Linux runtime claim.
- Flatpak host-terminal launch and terminal focusing are explicitly unavailable.
  Forced helper termination can leave Unknown ownership requiring local recovery.
- Managed NEAR currently pins its isolated proxy to the first discovered model;
  a model-picker UI is not part of this change. No IronWire source change or
  dependency revision change was needed for the implemented isolation contract.
