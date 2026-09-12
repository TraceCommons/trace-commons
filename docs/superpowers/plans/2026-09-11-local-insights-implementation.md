# Local Insights implementation plan

Date: 2026-09-11
Status: First CLI slice implemented and locally checked; desktop release and the remaining program packets are follow-on work.
Parent: [Trace insights program](2026-09-11-trace-insights-program.md).

## Initial implementation record

The isolated `insights-local-foundation` branch implements local protocol contracts,
the deterministic first-party provider, bounded Codex/trajectory analysis, dedicated
snapshot storage, and `insights analyze/list/explain/delete` commands. Analysis is
ephemeral unless `--save` is supplied. The contributor README describes usage and
the limitations of normalized counts and manually refreshed snapshots.

Review and focused verification covered six protocol tests, seven engine tests,
five end-to-end CLI tests, and four license-boundary tests. All passed. Workspace
formatting and contributor/protocol Clippy with warnings denied also passed. No
dependency or license-boundary changes were made.

The initial `RUSTFLAGS='-D warnings' cargo test --locked --workspace` run stopped
in the contributor library with 1,965 passed, six ignored, and three failures in
existing tests. The path-abbreviation test could not create its temporary directory
under home in the sandbox and passed outside it. The credential-forget and
wallet-listener tests failed timing assertions during the broad run and passed
individually. These reruns do not constitute a fully green workspace run; retain
that distinction in any PR or release handoff. The subsequent
`RUSTFLAGS='-D warnings' cargo test --locked --workspace --exclude trace-commons-contributor`
completed successfully outside the sandbox, covering the remaining crates and
their integration/doc tests. The standalone contributor binary check with
`--no-default-features` also passed. This slice does not qualify remote
evaluation, native desktop surfaces, or public analytics.

## Outcome and scope

Deliver an explicit-file, account-free local analyzer with descriptive observations. Analysis is ephemeral by default; `--save` explicitly persists a minimal derived snapshot. This is the first vertical slice of program packets 1–4, not completion of the private Insights release (packets 1–5). No deployment, remote inference, upload, mission publication, new dependency, or reward settlement is needed.

The CLI begins with Codex JSONL and trajectory files. These are a deliberately bounded integration choice, not the two richest telemetry adapters. Claude Code has the strongest current pricing support; Gemini and Cline preserve some usage. Claude's normal loader also expands a parent into delegated transcripts, so an explicit-file analysis path must resolve that scope before adding it. Codex plus trajectory exercise a native format and the common imported format without broad discovery. Actual user source prevalence has not been measured; do not claim this selection reflects a surveyed cohort.

Users receive Trace Commons results from a default local provider. Provider attribution and evidence are inspectable, but provider selection and provider-specific accounts are unnecessary. Common protocol contracts are extension points for later analytics, coaching, mission evaluators, and scouts; installing or registering a provider never grants access.

## Audit evidence and capability matrix

This audit reads repository adapter implementations and synthetic fixtures only. It does not inspect private transcripts or establish live adapter-version coverage. Paths below are relative to the repository root. `N` means a native recorded field is mapped, `P` means partial or conditional, `M` means missing from the normalized transcript, and `I` means inference that must be labeled. A field's availability is not a guarantee of completeness in every source file.

| Adapter | Model identity | Tokens/cache/reasoning usage | Time | Tool outcome | Task boundary, feedback, Git linkage |
| --- | --- | --- | --- | --- | --- |
| Claude Code | P: first session model; per-usage serving model only when usage is complete | P: input/output; complete cache split supports local price lookup; no distinct reasoning-token field | N: record timestamps; parent/subagents may interleave | P: explicit tool result verdict and call ID | M: no task acceptance or verified Git linkage; session grouping is I |
| Codex | P: first `turn_context.model`; later switches are not represented | M: normalized events have no counts or `served_by`; token-count records are opaque | N: record timestamps | P: explicit output `success` only; exit text is not interpreted | M: no task acceptance or verified Git linkage; session grouping is I |
| Gemini CLI | P: first model-bearing message | P: input/output on assistant text; cache/reasoning pricing data absent | N: message/turn timestamps | P: status becomes success/non-success, including cancellation | M: no task acceptance or verified Git linkage; session grouping is I |
| Cline | P: first message model or manifest fallback | P: checked input/output pair, attached to first assistant text block; no `served_by` | P: SDK messages may have timestamps; history exports may lack them | P: explicit tool-result verdict | M: no task acceptance or verified Git linkage; session grouping is I |
| OpenCode | N/P: provider/model in event structure; session model only if exactly one | M: normalized usage and serving metadata deliberately absent | N: validated message and part times | P: tool state verdict | M: no task acceptance or verified Git linkage; session grouping is I |
| Trajectory | P: declared `meta.model`, not verified serving identity | M: no normalized usage or serving metadata | N: parsed record timestamps | M: tool results carry no success verdict; pairing is validated | M: task outcomes absent; `meta.git_branch` deliberately discarded |

Integration evidence:

- `crates/trace-commons-contributor/src/source/mod.rs`: `TraceSource`, `SessionRef`, `SessionTranscript`, `SessionEvent`, `ServedBy`. `success: None` explicitly means no verdict. `cwd` must never be serialized. A session hash is a byte-version digest, not a stable episode identity.
- `source/claude_code.rs`: `map_assistant_record`, `served_by_of`, grouped loading. Existing input/output extraction defaults missing fields to zero and casts to `u32`; strict `served_by_of` rejects missing or excessive counts. Do not expose those permissive counts as complete usage without correcting or qualifying the extraction. Native usage is attached once per message, including tool-only turns.
- `source/codex.rs`: `load_session`, `map_response_item`. Malformed JSONL lines are skipped with a count-only warning. A new explicit import should reject malformed input or expose incomplete parsing; it must not silently label the import complete.
- `source/gemini_cli.rs`: `map_gemini_message`. Input/output casts can truncate large values; no-text assistant usage is dropped. Non-success status includes cancelled calls, so the aggregate must not call every false verdict a failure.
- `source/cline.rs`: `token_counts_of`, `map_message`. Checked integer conversions; assistant usage requires a text block.
- `source/opencode.rs`: validated export mapping and model markers; existing tests explicitly assert that usage remains absent.
- `source/trajectory.rs`: `parse_trajectory`, `TrajectorySource::new`, `TraceSource::load`; strict metadata and tool pairing, declared source separate from adapter routing key.
- `fixtures/codex/`, `fixtures/claude-code/`, `fixtures/gemini-cli/`, and `tests/fixtures/opencode/`: reusable synthetic input, not proof of real-world coverage.

Before enabling any adapter beyond the initial two, add missing/overflow usage fixtures, tool-only usage fixtures, model-switch fixtures, and explicit source scope tests. Do not repair all adapters as a prerequisite for the bounded first CLI slice.

## Metric dictionary, version 1

The denominator is the selected set of current imported source snapshots, with source and date filters explicitly reported when available. Session snapshots are saved observations of selected files, not stable task episodes or inferred independent tasks. A source address tracks replacement on manual reimport; a byte digest identifies a particular snapshot. Preserve metric version and provider version with outputs.

| Metric | Unit and denominator | Evidence and missing-data rule | Prohibited interpretation |
| --- | --- | --- | --- |
| Imported sessions | Count of distinct saved source snapshots after exact-copy deduplication | Replace a changed snapshot of the same source; unchanged reimport is idempotent | Number of completed tasks |
| Normalized events | Count over each selected session snapshot | Current slice reports total normalized events; per-kind mix is follow-on. Opaque events indicate partial semantic coverage; counts are not original message or API-call counts | Productivity or number of inference calls |
| Recorded tool calls | Count of recognized `ToolCall` observations per selected session | Source digest identifies the analyzed snapshot; opaque records may conceal unsupported calls, so present these as recognized counts with partial coverage rather than complete activity | Number of retries or complete tool activity |
| Reported tool non-success | Count of explicit false tool-result verdicts; show known-verdict and all-result denominators | Unknown verdict remains separate; only authoritative explicit values | Rejected code, task failure, or all execution errors |
| Observed span (follow-on) | Latest minus earliest recorded event timestamp per eligible snapshot, seconds | Not implemented in the initial count slice; needs at least two timestamps; includes idle time; do not sum overlapping sessions as human work time | Time saved or active human effort |
| Usage coverage | Sessions with available usage / selected sessions; initial per-snapshot coverage is 0/1 | Initial Codex/trajectory unavailable; this denominator measures session coverage, not eligible API calls or usage-bearing events. Unknown token value remains null, not zero | Complete token or bill coverage |
| Estimated cost | USD decimal subtotal only for independently priceable observations, with price version and coverage | Initially unavailable; later reuse strict `ServedBy` and `pricing::list_price_usd`; unknown model/modifier stays unpriced | Actual billed spend or cheapest model |
| Task category/outcome coverage | Explicitly annotated eligible episodes / all episodes | Initially unknown until manual annotations land; no transcript-sentiment inference | Accepted-task rate based on tool success |
| Model attribution coverage | Declared identity versus observed serving identity | Initial CLI omits model labels; later display must distinguish declarations from serving evidence. Model switches are unobservable through initial normalization | All work came from the declared model |

The initial report binds each metric to its source digest only. `explain` displays the saved report and source references; it does not recover source content, event indices, or a step-by-step calculation. Event-level drilldown and richer calculation explanations remain first-release follow-on work. Unsupported stronger conclusions return unavailable/insufficient evidence. Do not invent refactor/test/docs categories, retry counts, correction rounds, winner rankings, or time saved in this slice.

## Exact integration points and local storage

1. Add serializable insight/provider contracts to a new permissive protocol module exported by `crates/trace-commons-protocol/src/lib.rs`. Keep local filesystem locations out of shared contracts. Initial references carry source digest only; event indices remain follow-on work. Provider result validation checks input identity, schema/version, bounded values, evidence membership, and declared execution mode.
2. Add local analysis in `crates/trace-commons-contributor/src/insights.rs` (or a module directory if lifecycle code warrants it), exported by contributor `lib.rs`. It consumes `SessionTranscript` from the existing source adapters. Use a typed allowlist for Codex and trajectory explicit-file loading. No call to global source discovery, enrollment, routing receipt overlays, envelope construction, or submission.
3. Add the `insights` command family in `src/bin/trace-commons-contributor.rs` alongside existing Clap commands. Analyze takes an explicit source and file; list summarizes stored snapshots; explain uses a stored opaque ID; delete removes one saved report and its imported aliases. Bulk deletion is follow-on. Analysis persists only when `--save` is supplied. Final flag spellings must be pinned in CLI help and tests during integration.
4. Use a dedicated local Insights directory, selected by `--store-dir` or the platform local-data directory under `trace-commons/insights`. Reuse the atomic owner-restricted write helper in `config.rs`, but do not resolve or initialize `ConfigStore` on the Insights command path. No-save analysis creates neither enrollment nor Insights state. Existing daemon history and queue are contribution-specific and must not be reused as analytics outcome stores. Admission acceptance is not code acceptance.
5. Persist a minimal content-free projection: stable opaque source key, byte digest, adapter format and provider version, known numeric observations, a saved-snapshot timestamp and provider results. Current storage contains source-level counts; timestamp coverage and event-level projections are follow-on. Do not store raw prompts, code, tool arguments/output, source paths, working directories, project names, raw tool IDs, or credentials. Treat model strings and declared source labels as untrusted display input; bound and validate them before persistence/output.
6. Reuse `config::write_atomic_0600` atomic persistence, but add insight-specific concurrency control for the read/modify/write transaction. Atomic rename alone does not prevent lost updates. Fail closed on corrupt/unsupported stores and unsafe file types; sanitize error labels instead of forwarding raw path-bearing IO errors.
7. Reimporting a changed file at the same canonical address replaces that address's previous derived snapshot, never appends the whole history again. This is file snapshot replacement, not stable identity across renamed files or resumed sessions stored at different addresses. Stable address and content digest are different concepts. Exact copied imports need a declared duplicate policy and tests; no double counting identical snapshots. Content-free storage should use opaque aliases if distinct local paths refer to one snapshot.
8. Explicit delete removes controlled derivatives. Saved reports are dated snapshots: source freshness is not monitored. Changes or deletion of the original file do not automatically update saved results. Users must manually reimport changed files or delete saved reports; state this limitation in help and do not promise automatic source-deletion propagation. A later watcher/revalidation packet must close that gap before the full program release claim. No cached exports are created in the first slice.

Unix helpers set directory/file permissions to 0700/0600. They are not encryption at rest and do not establish equivalent Windows ACL enforcement. The first CLI slice needs portable correctness tests; platform release privacy qualification remains explicit. An inference proxy or attestation transport already existing elsewhere does not qualify remote Insights processing.

## Implementation packets and review evidence

| Packet | Change | Acceptance and focused tests |
| --- | --- | --- |
| A: protocol contracts | Versioned observations, coverage, provider request/result, local execution policy | Two identities; different metric/rubric versions remain distinct; invalid evidence references rejected; unsupported execution mode and schema fail closed; denial/unavailable states serialize without raw content |
| B: local projection | Explicit-file Codex/trajectory load; deterministic facts; first-party local result | Synthetic inputs only; no account required; no upload/envelope path; malformed/truncated/oversized input rejected; unknown cost/outcome preserved; explain returns saved metrics and source digest, with event-level calculations explicitly deferred |
| C: persistence lifecycle | Atomic, bounded content-free store; source aliases and snapshot digests; delete | Identical import unchanged; same-address changed source replaces; copied source deduplicated; corrupt/version-mismatched file fails closed; concurrent mutation cannot lose a source; delete removes source and dependent result; permission and secret-marker serialization assertions |
| D: CLI integration | Analyze/list/explain/delete with text and JSON views | CLI parsing and subprocess fixture test using temporary HOME/config; no enrollment or storage in no-save mode; explicit --save required; error output contains safe labels only; invalid source/path handled; no provider selection; JSON missingness matches human text |
| E: coverage expansion | Claude/Gemini or Cline usage extraction; model segments; manual categories/outcomes | Choose source order from explicit user usage evidence; overflow/incomplete cache/modifier tests; tool-only and mixed-model fixtures; no missing-as-zero; annotation provenance and lifecycle |
| F: desktop | Shared backend through daemon/FFI and native clients | See platform scope below; workspace tests and rebuilt FFI precede native suites |

A–D are the bounded implementation pass being started while further product input arrives. Their actual delivery is a count-based CLI snapshot, with source-level evidence only and explicit incomplete-coverage states. Saved snapshots do not establish current source availability or semantic task boundaries. They do not complete packets E–F or the parent program's first release. A contract existing in protocol does not prove every provider failure mode is implemented by the local engine: record unavailable conformance cases as follow-on scope, not passing tests.

Verification commands for implementation owners:

```sh
cargo fmt --all -- --check
RUSTFLAGS='-D warnings' cargo test -p trace-commons-protocol
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor insights
RUSTFLAGS='-D warnings' cargo check -p trace-commons-contributor --no-default-features
cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test license_boundary
```

Run explicit CLI integration test targets in addition to the module filter. Run `RUSTFLAGS='-D warnings' cargo test --workspace` for shared lifecycle or FFI changes; record resource/environment blockers honestly. No dependencies are planned, so the dependency-change license matrix is not triggered. Never alter expected licensing sets to accommodate a new dependency direction. Documentation-only checks do not establish implementation or runtime qualification.

## Desktop scope remaining after the CLI slice

All desktop surfaces remain follow-on work: macOS SwiftUI, GTK, and Windows. Add a common daemon API and FFI projection for summary/explain/delete and later annotations; desktop code must not implement separate metric arithmetic. Preserve one Insights vocabulary, attribution, missingness, and evidence drilldown across shells.

For each platform, qualify empty state, incomplete evidence, mixed models when supported, source replacement, deletion, keyboard navigation, long text, localized number/date rendering, and owner-only local persistence. Rebuild contributor FFI before macOS/Windows native tests; run the Rust workspace suite for lifecycle/FFI changes and GTK checks in its separate workspace. A CLI pass does not imply a desktop release. Document platform availability explicitly before shipping.

### Desktop implementation handoff: startup isolation

Status: follow-on design and implementation; none of the desktop work below is implemented by the CLI slice.

The existing startup paths prevent simply placing Insights behind ordinary daemon calls. `crates/trace-commons-contributor-ffi/src/lib.rs` checks `roots_refusal` in `tc_daemon_start*`; GTK `src/backend.rs` also refuses undeclared source roots. macOS `Views/MainWindowView.swift` gates `traceContent` on daemon startup and contribution onboarding. A private analyzer must remain usable before both enrollment and source-root declaration. Do not silently declare sources off to make daemon startup succeed or weaken the existing watcher startup gate.

Decision for packet F: introduce a shared local Insights service that has no daemon handle or enrollment dependency. Expose a bounded handle-free FFI entry point for macOS/Windows and call the same Rust service directly from GTK. Optional daemon methods may delegate to it when the daemon is already running. Keep source selection explicit and saving opt-in. Handle file IO off the UI thread, with defined cancellation and window-close behavior. The service must use the same dedicated store and validation as the CLI.

| Layer | Existing integration files | Follow-on change |
| --- | --- | --- |
| Shared Rust service | `crates/trace-commons-contributor/src/insights.rs` | Shared request/result and storage selection; explicit-file operations remain independent of discovery, enrollment, and upload |
| Optional daemon transport | `crates/trace-commons-contributor/src/daemon/ipc.rs` | Add delegated methods to `METHODS` and applicable request dispatcher; preserve advertised-method parity and safe errors |
| C bridge | `crates/trace-commons-contributor-ffi/src/lib.rs`, `include/trace_commons.h` | Handle-free local entry point with bounded input, panic containment, safe errors, and explicit response ownership/free contract; no daemon startup |
| macOS | `macos/Sources/TraceCommonsApp/Views/MainWindowView.swift`, `MainWindowNavigation.swift`, `macos/Sources/TCBridge/TCDaemon.swift` | Add Insights navigation outside `traceContent` gate; new view/model and a local bridge wrapper beside the daemon wrapper; do not route refresh through account/history polling |
| Windows | `windows/src/TraceCommons.Interop/NativeMethods.cs`, `windows/src/TraceCommons.App/ViewModels/MainViewModel.cs`, `MainWindow.xaml`, `MainWindow.xaml.cs` | New local protocol/service and Insights view/model; add `MainPane` destination accessible before onboarding and watcher startup |
| GTK | `crates/trace-commons-contributor-gtk/src/backend.rs`, `src/ui/mod.rs` | Independent local-service access, new `ui/insights.rs` page; prevent contribution onboarding from blocking this destination |

Use common rendered vocabulary and metric arithmetic from Rust. The shell should own layout, selection, and interaction, not compute a different meaning of coverage or task success. Show source-digest-only evidence honestly until event-level drilldown is implemented.

Desktop acceptance:

- On a fresh installation, before login and source-root declaration, open Insights and analyze one explicitly selected synthetic Codex/trajectory file without creating enrollment state, invoking discovery, or starting network work.
- No-save analysis creates no Insights state. Explicit save, list, explain, and delete interoperate with the CLI's store; a changed snapshot updates only after manual reimport. Deletion preserves the original file.
- All shells display unknown token/cost/outcome values, partial recognition when opaque events exist, provider attribution, and dated-snapshot/manual-refresh limitations consistently.
- Test null pointers, invalid UTF-8, malformed/oversized requests, safe error labels, response ownership/free behavior, and fresh C-header/export/native declaration correspondence.
- Test closing the window or cancelling while bounded IO is in flight; no UI-thread blocking, dangling callbacks, or post-close state mutation.
- Keep current daemon roots and contribution-onboarding tests passing. Add first-run Insights navigation tests that explicitly demonstrate the new destination does not weaken those gates.

Verification requires `RUSTFLAGS='-D warnings' cargo test --workspace`, a fresh contributor-FFI build, then macOS `swift test`, Windows Interop tests against the freshly built library, and GTK's separate-workspace checks/tests plus its weston UI smoke. Relevant existing fixtures include macOS `DaemonStartupTests.swift` and `DaemonFieldDecodingTests.swift`, Windows `DaemonFieldDecodingTests.cs` and onboarding tests, and GTK backend roots tests. A native suite linked to a stale library is not evidence for the new bridge. Platform availability and persistence protections remain release gates, not conclusions from this map.

## Later architecture preserved

Git/test/review evidence and manual outcomes precede model comparisons. Semantic coaching needs a calibrated evaluator and independently qualified private remote path. Hosted team storage requires its own consent, retention, and tenant design. The public provider catalog, provider billing, and automated rewards remain deferred.

Mission definitions and scout proposals will use the same evidence/provenance vocabulary and unified product presentation. Scouts first create bounded sourced drafts, with curator review and a reviewed proposal digest before publication. A paper URL or source claim is not an outcome, executable permission, publication approval, or reward authorization. Live source adapters, controlled mission execution, and public aggregate release remain separate qualification packets in the parent plan.
