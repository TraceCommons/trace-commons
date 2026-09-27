# Tauri desktop architecture

Tauri reuses the existing permissively licensed contributor core. Swift, WinUI,
and GTK shells remain unchanged; no legacy app is removed by this work.

## Frontend and Rust boundary

- `frontend/src/lib/tauri/core-api.ts` uses official Tauri 2 `invoke` and
  `listen` APIs. No `window.__TAURI__` bridge, browser HTTP fallback, or direct
  frontend daemon access.
- Feature adapters validate `unknown` command results. Rust validates inputs,
  owns native access and domain operations, and returns typed JSON/errors.
- `daemon_call` stays read-only and allowlisted; mutations use named commands.
  Rust emits approved event names; app-level handlers invalidate feature query
  keys, and queries refetch through their feature adapters.
- Handler registration, generated build permissions, and the main-window
  capability manifest must stay aligned. CSP permits local assets and Tauri
  IPC; development adds fixed Vite/HMR origins.

## React state and lifecycle

- Eager routes; feature-owned adapters, query keys, forms, workflow hooks, and
  public entries. TanStack Query owns daemon state; components own ephemeral UI.
- Effects synchronize external subscriptions and authoritative native/server
  state only. Derived display values stay in render.
- Desktop listeners subscribe before consuming a cold-start deep link. A
  credential callback refreshes Private AI status, then opens Private AI.
- Event bridge starts after daemon startup or source-root selection. Existing-
  daemon attachment supports Unix sockets on macOS/Linux and the daemon's
  existing named pipe and ACL on Windows. It reconnects after transport loss
  and carries status plus pushed events over one connection. Its callback
  thread owns the persistent transport; the client refuses daemon shutdown.
  No-root startup keeps account-free routes available. Quit confirmation and
  macOS Reopen are wired. The quit prompt comes from the shared `quit_copy`
  table and is chosen per role: hosting (quitting stops the watcher),
  attached (the other process keeps watching; no stop option, since the
  attached client refuses shutdown), or no watcher reachable.

## Remaining parity gates

- The macOS app and Tauri both register `tracecommons://`. Assign callback
  ownership before distributing both on macOS; verify cold-start and
  already-running callback delivery from packaged builds on every OS. A
  digest notification click does not depend on that: the notification
  delegate calls back into Rust, which shows the window and opens the review
  queue in-process instead of asking LaunchServices for the scheme handler.
- Certificate display stays within native parity: held-session list and shared
  copy only; no Tauri-only certificate detail surface.
- Tauri tray still lacks legacy health, budget, and armed-project summaries;
  pause-until-tomorrow-morning behavior also remains unmatched.
- Packaged-device acceptance remains open for notifications, login items,
  tray, keychain, file grants, callback routing, and clean-machine install and
  upgrade behavior on each OS. macOS also needs signed packaging and
  notarization acceptance; provider callbacks, wallet completion, and witness
  availability need live acceptance.
- Contributor state now resolves through the shared core directory, including
  `TRACE_COMMONS_CONTRIBUTOR_DIR`. Existing prototype-specific data remains
  untouched; decide any non-destructive migration before release. Bundle
  identity remains a release decision: the development config uses
  `ai.tracecommons.tauri.prototype`; the release config currently uses
  `ai.tracecommons.desktop`.

## Bundles and updates

- Tauri CI builds Linux AppImage and DEB packages on Ubuntu 24.04, plus Windows
  MSI and NSIS installers on Windows. CI uploads unsigned artifacts for seven
  days; these are build checks, not release downloads.
- The existing signed release workflow packages the Swift, WinUI, and GTK
  shells. Its signing identities, update feeds, and package identities do not
  cover Tauri. Tauri has no updater plugin, update endpoint, or update signing
  key configured; do not publish Tauri updates until those are selected and
  tested together with the final bundle identity.
- Tauri release promotion still needs platform signing, a separate update
  publication flow, clean-machine install/upgrade acceptance, and a decision
  about callback ownership where another shell claims the same URL scheme.

## Checks

From repository root:

```bash
pnpm --dir tauri-desktop/frontend build
pnpm --dir tauri-desktop/frontend build-storybook
cargo fmt --manifest-path tauri-desktop/src-tauri/Cargo.toml -- --check
cargo check --locked --manifest-path tauri-desktop/src-tauri/Cargo.toml
cargo test --locked --manifest-path tauri-desktop/src-tauri/Cargo.toml
```

`.github/workflows/tauri-desktop.yml` runs on every push and pull request. It
runs frontend and Storybook builds,
`pnpm test`, `pnpm audit`, Rust format/check/test across macOS, Linux, and
Windows, Linux Clippy, Linux and Windows unsigned bundles, all-feature Cargo
license/source/advisory audits, and the declared Tauri MSRV check. CI proves
compile, test, and bundle creation; it does not establish GUI, OS-permission,
packaged-device, signed-package, update, callback, or provider acceptance.
