# Native onboarding agent handoffs

Date: 2026-09-04. Decomposition only; neither task is implemented by this document.
Base: `native-onboarding-admission` in `.worktrees/native-onboarding-admission`.
Read `AGENTS.md`, `CLAUDE.md`, the [execution plan](2026-09-04-native-inference-onboarding.md),
[native design](../specs/2026-09-04-native-inference-onboarding-design.md), and
[admission design](../specs/2026-09-04-admission-invite-or-attestation-design.md).
The three-shell captured-body consent implementation is the baseline to preserve.

These agents have disjoint production ownership and can work independently.
The root integrates shared protocol DTOs, `daemon/ipc.rs` dispatch, contract
decisions, and merge order. Submit exact dispatch patches/helpers to root.
Schedule within the team limit of three active workers; do not spawn more.
No task enables public admission, sponsorship, capture, or deployed services.

## A — witness-preview

**Objective:** make an invited native contributor explicitly request a witnessed
review, approve the resulting immutable artifact, and upload that same artifact
with its verified certificate and compatible server-granted scopes.

**Inputs and contracts:** existing `IssuerClient`, `SubmitContext` claim minting,
`GrantedConsent`, witness trust/transport checks, queued approval fingerprints,
and the preview IPC contract in `docs/contributor-daemon-ipc-v1_1.md`.
Current blockers are `witness_claim_unavailable` in `daemon/preview.rs` and the
`witness_certificate_missing` approved-envelope guard in `submit.rs`; deleting
these guards alone is forbidden. A redaction certificate remains a redaction
certificate; this task does not make it an admission credential.

**Owned production files** (paths below start at `crates/trace-commons-contributor/src/`):

- `daemon/preview.rs`, `daemon/approved_envelope.rs`, `daemon/queue.rs`.
- `daemon/uploader.rs`, `submit.rs`, `issuer_client.rs`.
- `witness/transport.rs`, only reusable verified-result handling needed by preview.
- Tests: inline tests in those files; `crates/trace-commons-contributor/tests/daemon_preview_cli.rs`
  and `crates/trace-commons-contributor/tests/daemon_preview_body_over_socket.rs`.
- Documentation: preview/approval sections of `docs/contributor-daemon-ipc-v1_1.md`.

**Exclusions:** native UI files, server routes/migrations, certificate encoders,
NEAR account bootstrap, admission budgets/replay ledger, redemption, IronWire
configuration, dependencies, and license-boundary expected sets. Root owns IPC
dispatch and FFI/protocol integration; hand it a precise patch/helper contract.

**Readiness:** local implementation can start now using existing issuer/witness
contracts. Missing deployment evidence blocks live acceptance, not local work.

**Dependencies and implementation:**

1. Write a concrete claim lifecycle proposal before changing the refusal:
   authenticated identity/device, requested versus granted scopes, claim lifetime,
   refresh rules, and whether claim issuance reserves anything. Review/preview
   must not accidentally consume an admission attempt or award credit.
2. Agree the explicit off-device review trigger with F (native-onboarding). Existing card
   summaries and background refresh must not begin witness requests. Existing
   source comments promise local preview; document the witnessed exception and
   require the relevant user consent before any raw content leaves the machine.
3. Reuse the direct-submission pinned-measurement verification and transport path.
   Build with granted scopes inside certified bytes; retain endpoint/measurement,
   policy, consent, source-content, and configuration bindings needed for rechecks.
4. Extend approved-artifact persistence to retain the verified certificate and
   submission metadata alongside the exact reviewed envelope. Reject partial,
   corrupt, legacy-unwitnessed, or mismatched records; preserve atomic save/recovery.
   Never persist raw attached inference bodies or bearer claims in that record.
5. Upload the pinned artifact and certificate without re-redaction or silent
   scope rewriting. An expired claim requires a compatible fresh authorization;
   changed grant, source, consent, witness trust, or artifact requires fresh review.
   Preserve residual-secret checks and local-only previews when no witness is set.
6. Expose only the agreed usable review capability and safe failure labels.
   Hand the schema, trigger semantics, and evidence fixtures to F (native-onboarding).
   Ask the integrator to revise shared unavailable copy only after integration.

**Acceptance:** fake issuer/witness tests exercise missing/expired/foreign claims,
scope narrowing, transport refusal, invalid pins/signatures, altered artifact,
source/settings changes, restart, missing certificate, and partial disk writes.
Assert no background/unauthorized witness traffic; neither witness failure nor
legacy approved bytes falls back to an unwitnessed upload. Assert exact reviewed
versus uploaded canonical bytes and no attached bodies in queue/stored artifact.

```sh
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::preview --locked
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::uploader --locked
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib witness --locked
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --test daemon_preview_cli --test daemon_preview_body_over_socket --locked
RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --test license_boundary --locked
```

Run approved-artifact/claim-specific tests added outside these filters, formatting,
and repository Clippy. **Deliverables:** code/tests, reviewed lifecycle contract,
fixture handoff, test record, and invited live-pilot procedure. Completion of the
live milestone additionally needs configured pins/receipt endpoint, an explicitly
funded session, and accepted artifact inspection; mocks alone do not establish it.
**Stop conditions:** report the precise missing claim/scope/trigger contract or
external pilot dependency. Keep its path refused; continue unrelated tests and
contract work. Do not mint a placeholder claim or invent funding to get green.

## F — native-onboarding

**Objective:** add resumable first-contribution guidance after setup across macOS,
Windows, and GTK, then expose new onboarding routes only from deployed capabilities.
Setup completion, useful inference observation, processing, acceptance, and
spendable funding must remain distinct facts with distinct authorities.

**Inputs and contracts:** existing local project/session discovery, routing
observations, receipt coverage, server-backed history, and tenant-keyed setup
markers. Consume A's agreed explicit-review contract. Self-service
account/device bootstrap, capability freshness/versioning, admission entitlement,
and actual inference funding require upstream contracts from their assigned owners.
Absent contracts are not permission to create placeholder endpoint calls or enums.

**Owned production files:**

- macOS `Sources/TraceCommonsApp/{AppModel.swift,DaemonClient.swift,Models.swift}`;
  `Views/Onboarding*.swift`, `Views/MainWindowView.swift`, `Views/QueueView.swift`,
  and `Views/PreviewSheet.swift`; relevant `Sources/TCShellCore/` presentation models.
- Windows `src/TraceCommons.App/{OnboardingWindow.xaml,OnboardingWindow.xaml.cs}`;
  `ViewModels/{OnboardingViewModel.cs,MainViewModel.cs}`, `MainWindow.xaml` and code-behind;
  `Controls/PreviewSheet.xaml` and code-behind;
  `src/TraceCommons.Interop/OnboardingState.cs` and native capability/progress DTOs.
- GTK `crates/trace-commons-contributor-gtk/src/{model.rs,copy.rs}` and
  `ui/{onboarding.rs,preview.rs,queue.rs,mod.rs}`.
- New first-contribution views/models beside those files, with live consumers.
  Platform secret-storage adapters and selected-agent configuration/rollback
  belong here after C supplies its scoped-credential contract; coordinate any
  shared Rust adapter with root before implementation.
  Tests: corresponding macOS app/core tests, Windows Interop tests, GTK inline tests.

**Exclusions:** contributor daemon/submit/witness internals owned above; server,
protocol, FFI, wallet signing implementation, credential provisioning/redemption,
and shared Rust privacy copy without an agreed integrator handoff. No modification
to an agent's configuration or proxy capture merely from entering onboarding.

**Dependencies and implementation:**

1. Inventory existing observations and their failure/staleness semantics. Build the
   invited-path checklist from evidence already returned; unknown is not complete.
   Do not infer NEAR funding or successful inference from an IronWire declaration.
2. Keep current invite enrollment and tenant-keyed setup/resume semantics. Add
   existing-history and no-history guidance with actual next actions; consent
   cancellation, empty discovery, or pending processing must not trap the wizard.
3. Integrate explicit witnessed review only after A's contract lands.
   Preserve independent privacy-scan/body-export consent, refusal presentation,
   user-visible review, and approval invalidation; never auto-submit for activation.
4. When deployed bootstrap/admission/funding contracts exist, decode supported
   versions with freshness checks. Hide unsupported actions; render failed reads
   as unknown. Refresh after browser return, account/device change, and restart.
5. Persist only local progress/navigation facts, keyed to account/tenant. Re-query
   entitlement, accepted contributions, and actual funds; never persist optimistic
   balance/activation as authority. Pending credit remains visibly non-spendable.
6. Apply the same state/next-action fixtures to all shells and inspect actual UI.
   Preserve invited users' non-NEAR choices and access after window exhaustion.
7. Consume C's provider-issued scoped credentials through platform secret
   storage. Configure only the selected supported agent after explicit user
   action, preserve its previous configuration for rollback, and verify an
   actual provider call. Do not implement provider-side funding/provisioning.

**Acceptance:** restart/account-switch isolation; empty and existing-history paths;
missing/unknown/stale capabilities; login return replay and failure via supplied
contract fixtures; consent cancel/save failure; witness refusal; pending/rejected/
accepted contribution transitions; exhausted entitlement; unfunded/funded inference;
pending credit never unlocking funding. A settings save cannot complete activation.

```sh
RUSTFLAGS='-D warnings' cargo build -p trace-commons-contributor-ffi --locked
swift test --package-path macos
DOTNET_ROLL_FORWARD=Major dotnet test windows/tests/TraceCommons.Interop.Tests/TraceCommons.Interop.Tests.csproj
RUSTFLAGS='-D warnings' cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --locked --lib
```

Set `TC_FFI_LIB_DIR` to the freshly built FFI directory; use the execution plan's
cache flags when needed. Run WinUI build/tests on Windows and GTK visual checks
on a supported display. **Deliverables:** three-shell code/tests, state fixture
matrix, screenshots, and supported/deferred capability list. **Stop conditions:**
missing upstream schema, native browser return contract, funding authority, or
witness trigger blocks only its dependent action. Deliver the existing invited
path; retain honest unavailable copy instead of simulating the unbuilt path.
