# Current-behavior coverage inventory

This is a source-discovery index, not a claim that the entire application is modeled. Owner assignment only identifies the model that should reconcile an entry. A row stays a visible gap until its concrete model actions and observable fields are audited. `coarse-interface` identifies a deliberately abstract interaction; it does not cover handler internals.

Source tree: `a2e0aa56608f8c403deff679f8b51e76ac2f5ca7`. Original discovery worktree: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`. Only files in the pinned Git tree are scanned. The primary checkout and its untracked additions are excluded.

Run from the repository root:

```sh
python3 docs/flows/trace-commons/scripts/inventory.py --self-test
python3 docs/flows/trace-commons/scripts/inventory.py
python3 docs/flows/trace-commons/scripts/inventory.py --check
```

The writer retains audited annotations in coverage.json only while the row ID and source-file fingerprint match. Covered rows require concrete model action names and modeled observables; the command checks that named actions and referenced state fields exist. Existence of an action does not establish semantic correspondence: human source/model review remains necessary.

## Extracted entry points

| Kind | Entries | Detailed | Coarse interface | Gaps |
| --- | ---: | ---: | ---: | ---: |
| boundary | 12 | 0 | 0 | 12 |
| client_worker | 16 | 0 | 0 | 16 |
| ipc | 190 | 0 | 0 | 190 |
| native_entry | 15 | 5 | 5 | 5 |
| navigation | 86 | 48 | 1 | 37 |
| route | 329 | 2 | 9 | 318 |
| screen | 340 | 3 | 14 | 323 |
| server_worker | 25 | 1 | 0 | 24 |

| Candidate owner | Detailed | Coarse interface | Gaps |
| --- | ---: | ---: | ---: |
| client_consent | 1 | 1 | 0 |
| client_contribution | 0 | 1 | 99 |
| client_learning | 0 | 2 | 57 |
| client_providers | 0 | 6 | 0 |
| client_services | 0 | 0 | 159 |
| client_shell | 6 | 1 | 264 |
| end_to_end | 0 | 0 | 0 |
| platform_clients | 49 | 9 | 4 |
| server_identity | 2 | 2 | 0 |
| server_interfaces | 0 | 7 | 241 |
| server_lifecycle | 0 | 0 | 29 |
| server_pipeline | 1 | 0 | 18 |
| server_receipt | 0 | 0 | 7 |
| server_settlement | 0 | 0 | 47 |

The complete per-entry list is [coverage.json](coverage.json), with exact source path/line, identifier, handler or dispatcher, owner, classification, rationale, actions and observables. HTTP methods are explicit registrations; Axum's implicit HEAD behavior on GET is not counted as a separate registration.

## Meaning of the classifications

Detailed rows map a specific source flow projection to named model actions and concrete state fields. Their rationale records omitted transport, rendering, parameter or unbounded-domain detail. A detailed row is not a claim that every branch of its containing file or endpoint has been modeled. Coarse interface rows preserve only the stated boundary and outcomes. Gaps are discovered entries with no reviewed correspondence.

Gap totals do not count missing independent flows. Routes and IPC methods are actual registered/dispatch entry points; screen rows are intentionally over-inclusive presentation declarations, including decorative components, and require reachability review. Each row's inventory_role records that distinction. Navigation declarations and worker spawners can also share behavior with another row. An unresolved component declaration is different from an omitted account or queue mutation.

Historical seed mappings included the six native macOS first-run screen families, core trace preview/approval/policy/history commands, selected credential/harness/managed-session/profile commands, phase-worker state and explicit auxiliary server interface families. A source refresh invalidates mappings whose stable ID or source-file fingerprint changed; only the currently retained annotations count in the tables above. The submit route is coarse at the HTTP entry: its versioned receipt branch is modeled in detail, while legacy processing and parsing are omitted. Cross-platform first-run and native widgets are not covered by those macOS mappings.

Substantial remaining gaps include unassigned UI component/sheet declarations and platform-specific navigation, per-card Insights/drilldown behavior and complete learning concurrency, account/device and enrollment route branches, legacy review/gate scheduling, exact administrative reports/drills, all generic-task spawn sites and standalone operator CLI entry points. The manifest is deliberately visible about these gaps; the model suite cannot be described as complete application coverage.

## Scanner boundaries

- Pinned Git-tree lexical extraction, not a compiler or reachability proof. Runtime feature flags, deployment configuration, middleware ordering, tenant scope and handler behavior require separate source review.
- HTTP scan includes production .rs files in ingest, server and contributor, excludes tests/ and *_test.rs / *_tests.rs, and resolves cfg(test)-only external module declarations in standard Rust layouts, and removes inline cfg(test) modules (including visibility and positive all(test, ...) forms). Axum .route literal registrations, protocol/local path string constants and AccountRoutes::new() helper chains are supported, including delegated builder files and post_with_body_limit. Qualified protocol aliases and unique imported protocol constant names are resolved; the one AccountRoutes forwarding .route(path, method_router) helper is explicitly recognized.
- Unknown .nest, .nest_service, .route_service, .fallback_service, method-router any/on, nonliteral .route paths, indirect handler expressions and unknown AccountRoutes builder methods fail extraction. Ordinary Router.merge calls are composition sites: registrations in all scanned files are listed, but their runtime reachability is not inferred. A new routing API or builder type requires scanner review; a custom router with no recognized construct can remain invisible.
- IPC scans the top-level literal arms in handle_request and handle_request_async, checks equality with advertised METHODS, and records managed_ asynchronous pre-dispatch delegation. Handler internals, parameter variants, pushed-event payloads and nested dispatchers are not separately enumerated. Future nonliteral/or-pattern method dispatch requires scanner review.
- macOS navigation cases include HomePage, TracesPage, MonitorDestination, SettingsSection and six FirstRunStep cases. SwiftUI View declarations are over-inclusive candidates (many are components), not proof of screen reachability. Rendering closures without a named View and dynamic sheets require manual UI review.
- Windows inventory includes Window/UserControl/ContentDialog declarations, MainWindow OnShow rail actions and OnboardingStep. GTK includes public *View/*Dialog/*Window declarations and the named screen tables; Tauri includes routePaths/flowPaths and rendered *Page sites. Audited native_entry rows additionally name startup, local Insights entry, close/quit and attached-event entry points, with exact executable selectors checked. These structural scans do not enumerate every local modal, control, conditional branch or platform entry point.
- Manual HTTP boundary rows explicitly enumerate native-account, wallet-onboarding, Cloud credential and browser chooser callback dispatch, wallet asset dispatch and the external IronWire listener start. Their exact executable selectors are checked. A new hand-written HTTP parser/listener is not automatically discovered; external IronWire implementation and HTTP routing are outside this source tree.
- Worker rows enumerate production spawn_* function declarations in ingest, server and contributor, plus manually audited named daemon supervisor/watch/upload/history/community/estimate/lease/token-cleanup pass entry points; worker HTTP routes are also ordinary route rows. Generic tokio::spawn sites without a spawn_* declaration, other inline tasks, standalone operator binaries and external scheduling/deployment topology are not exhaustively indexed. Worker counts are declaration candidates, not running-process counts.
- This extractor does not independently enumerate every CLI subcommand, exported C ABI function, Tauri command adapter, GTK D-Bus activation or native service API. Many delegate to the indexed IPC/domain behavior, but equivalence is not inferred. Native local-service flows (including Insights and mission files) have source notes in their domain models; complete adapter/ABI inventories require additional scanning.
- Owner assignments are heuristic triage. Newly discovered entries default to gap. No row becomes detailed or coarse merely because its subsystem has a model file. Concrete action/observable mappings must be reviewed, and selected-promise model-check results remain separate evidence.

## Source boundary and exclusions

The historical seed inventory excluded unrelated work in the primary checkout. The following paths were untracked there when the inventory was prepared; their contents were not inspected or modeled. This list is a historical exclusion snapshot, not a live scan and not a shipping-feature inventory.

```text
.build/.buildSystem_debug
.build/CACHEDIR.TAG
.cloudflare/cache/cloudflare-account.json
:memory:.ses
crates/trace-commons-contributor/src/daemon/remembered_passkeys.rs
crates/trace-commons-contributor/src/daemon/upload_allowance.rs
crates/trace-commons-contributor/src/daemon/upload_allowance_tests.rs
crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/native_passkey.rs
crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/step_up_page.js
crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/step_up_page.rs
crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/tests/account_binding_gate_tests.rs
crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/tests/native_passkey_tests.rs
crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/tests/near_ai_bind_tests.rs
crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/tests/step_up_page_tests.rs
crates/trace-commons-server/src/account_binding.rs
crates/trace-commons-server/src/account_native_passkey.rs
crates/trace-commons-server/src/account_passkey.rs
crates/trace-commons-server/src/account_reaper.rs
crates/trace-commons-server/src/db/postgres_account_binding.rs
crates/trace-commons-server/tests/unbound_account_reaper_pg.rs
docs/announcements/2026-09-08-v0-12-0.md
docs/announcements/2026-09-08-v0-12-0.md.bak-210141
docs/announcements/STYLE_GUIDE.md
docs/operator/native-passkey-release-qualification.md
docs/operator/native-step-up-page.md
docs/operator/pilot-deploy-2026-11-03-commands.md
docs/operator/pilot-deploy-2026-11-03-plan.md
docs/operator/pilot-deploy-2026-11-04-commands.md
docs/operator/pilot-deploy-2026-11-05-commands.md
docs/operator/pilot-payouts-enable-2026-11-03-commands.md
docs/operator/pilot-stage2-2026-11-03-commands.md
docs/operator/staging-deploy-2026-11-03-commands.md
docs/operator/staging-deploy-2026-11-04-commands.md
docs/operator/staging-deploy-2026-11-05-commands.md
docs/operator/synthetic-e2e-2026-11-05.md
docs/operator/unbound-account-reaper.md
docs/operator/witness-redeploy-2026-11-05-commands.md
docs/operator/witness-redeploy-2026-11-05-e2e.py
docs/operator/witness-redeploy-2026-11-05-liveness-body.json
docs/operator/witness-redeploy-2026-11-05-pins.py
docs/operator/witness-redeploy-2026-11-05-synthetic-body.json
docs/superpowers/plans/2026-11-05-environment-feature-flags.md
docs/superpowers/specs/2026-11-05-environment-feature-flags-design.md
list-key-v1.pem
macos/Sources/TCDesign/Components/CardShimmer.swift
macos/Sources/TCShellCore/DailyBudgetCopy.swift
macos/Sources/TCShellCore/NativePasskey.swift
macos/Sources/TraceCommonsApp/PlatformPasskeyProvider.swift
macos/Sources/TraceCommonsApp/Views/FirstRun/PasskeySheets.swift
macos/Sources/TraceCommonsApp/Views/Monitor/CreditDailyTotals.swift
macos/Sources/TraceCommonsApp/Views/Settings/ConnectionSection.swift
macos/Tests/TCDesignTests/CardShimmerTests.swift
macos/Tests/TCShellCoreTests/DailyBudgetCopyTests.swift
macos/Tests/TCShellCoreTests/NativePasskeyTests.swift
macos/Tests/TraceCommonsAppTests/ConnectionSourceStatusTests.swift
macos/Tests/TraceCommonsAppTests/NativePasskeyClientTests.swift
macos/Tests/TraceCommonsAppTests/PasskeySheetsTests.swift
macos/Tests/TraceCommonsAppTests/PlatformPasskeyProviderTests.swift
macos/Tests/TraceCommonsAppTests/SourceCheckBindingTests.swift
scripts/check-aasa.sh
scripts/ci/native-passkey-entitlements.py
scripts/ci/test-check-aasa.mjs
scripts/ci/test-step-up-page.cjs
scripts/ci/tests/test_native_passkey_entitlements.py
windows/tests/TraceCommons.Interop.Tests/DailyBudgetTests.cs
```

See [platforms.md](platforms.md) for platform-specific differences and their model limits.
