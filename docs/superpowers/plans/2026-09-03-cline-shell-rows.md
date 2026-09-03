# Cline Shell Rows Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every contributor shell (GTK, macOS, Windows) offers Cline the way it offers Gemini CLI: a row on the session-roots screen, a row on the settings screen driven by `cline_source_mode`, and a row on the IronWire tools surface.

**Architecture:** Gemini CLI was the last source added to the shells and every touch point is still visible by grepping `gemini` (case-insensitive) in each shell. Cline mirrors it exactly, one file at a time, with one deliberate difference stated below. Task 1 lands the shared Rust contract the shells consume (tool word, `SourceTool` key, FFI docs and header). Tasks 2, 3 and 4 are one shell each and are independent of one another.

**Tech Stack:** Rust (contributor + ffi + gtk crates), Swift (macOS), C# (Windows). No new dependencies anywhere.

**Spec:** `docs/superpowers/specs/2026-09-03-ironwire-agent-alignment.md` section 4.2 and 4.7; the daemon side is already on this branch (commits `788df843`..`4851f288`).

## Global Constraints

- Work only in this worktree with relative paths. `CARGO_TARGET_DIR=/Users/zakimanian/code/trace-commons-server/target` on every root-workspace cargo command. The GTK crate is its own workspace: build it with `--manifest-path crates/trace-commons-contributor-gtk/Cargo.toml` and its own target dir (`crates/trace-commons-contributor-gtk/target`); do not point it at the shared one.
- No new dependencies. No emojis. No paths, ids or content in log strings.
- **Ids, fixed.** Discovery source `cline`. Settings key `cline_source`. Settings-view mode field `cline_source_mode`. `SourceTool::from_key("cline")`. IronWire tool id `cline` (there is no upstream row for it, exactly as with Gemini: the tools surface renders it as unknown, never as a verdict). Tool word `TOOL_CLINE = "Cline"`. Roots title on GTK `ROOTS_CLINE = "Cline sessions"`.
- **The one deliberate difference from Gemini:** none in behaviour. Cline is optional on the roots screen the way Gemini is: left blank, nothing is read (`Undeclared::Nothing`). It must NOT be added to the daemon start gate (`roots_declared` stays two-conjunct: claude and codex). Every shell has a comment explaining why Gemini is excluded from that gate; Cline joins the same sentence.
- **Copy comes from one place.** Shells never spell "Cline" themselves. GTK uses `trace_commons_contributor::routing_copy::TOOL_CLINE`; macOS and Windows read `tool_cline` from the routing-copy JSON the FFI exports. The roots-screen label for the `cline` source id is the one exception, mirroring how each shell already maps `gemini-cli` to a display name.
- **The C header is duplicated and CI requires the two copies byte-identical:** `crates/trace-commons-contributor-ffi/include/trace_commons.h` and `macos/Sources/CTraceCommons/include/trace_commons.h`. Edit one, copy to the other, `cmp` them.
- The rustfmt hook can rewrite whole files. After every commit run `git show --stat HEAD`; if an unintended file appears, restore it and amend.
- Another agent may be committing in this worktree at the same time. If `git commit` fails on `index.lock`, wait a few seconds and retry; never delete the lock file. Only ever `git add` your own named files.
- Commit per task, short imperative subject, no prefix.

---

### Task 1: The shared Rust contract

**Files:**
- Modify: `crates/trace-commons-contributor/src/routing_copy.rs` (`TOOL_CLINE`, `RoutingCopy.tool_cline`, `routing_copy()`, the existing test that asserts each `tool_*` field)
- Modify: `crates/trace-commons-contributor/src/source_copy.rs` (`SourceTool::Cline`, `from_key("cline")`, `name()`, extend every test that enumerates the three tools)
- Modify: `crates/trace-commons-contributor-ffi/src/lib.rs:2042` (doc comment: `claude`, `codex`, `gemini` or `cline`)
- Modify: both copies of `trace_commons.h` (the `tool_*` field list near line 278 and the `tool is ...` comment near line 452)
- Modify: `crates/trace-commons-contributor-ffi/tests/abi.rs` (wherever the tool list or `tool_gemini` is enumerated; add `cline` / `tool_cline` beside it)
- Modify: `crates/trace-commons-contributor-gtk/src/model.rs:172` (`"cline" => "Cline"` beside `"gemini-cli" => "Gemini CLI"`; this is the GTK crate but it is a one-line mapping the GTK task would otherwise trip over)

**Interfaces produced:** `pub const TOOL_CLINE: &str = "Cline"`, `RoutingCopy.tool_cline`, JSON field `tool_cline`, `SourceTool::Cline`, `tc_source_check_line("cline", mode)` returning `"Cline sessions folder set"` / `"Cline sessions read from the usual place"` / `"Cline marked not used, so nothing is opened for it"`.

- [x] Write the failing tests first: extend `source_copy.rs` tests (`each_mode_gets_its_own_sentence` gains the three Cline lines; `the_three_modes_never_share_a_sentence` iterates four tools; `from_key` test adds `"cline"`; `name()` test adds `Cline`), `routing_copy.rs`'s field test gains `tool_cline`, and `abi.rs` gains a `tc_source_check_line("cline", ...)` case matching the Rust function.
- [x] Run `cargo test -p trace-commons-contributor --lib source_copy routing_copy` and `cargo test -p trace-commons-contributor-ffi --test abi`; expect compile failures.
- [x] Implement, header included, `cmp` the two headers.
- [x] Run the same tests, expect green. Then `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins`, `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-ffi --no-run`, clippy with the repo allow-list, `cargo fmt --all`.
- [x] Commit: `Give Cline a tool word and a source key`.

---

### Task 2: GTK

**Files** (from `grep -rni gemini crates/trace-commons-contributor-gtk/src`):
- `src/copy.rs` (`ROOTS_CLINE`; the `TOOL_CLINE` re-export beside `TOOL_GEMINI` at line ~1826)
- `src/ui/roots.rs` (`source_title`: `SOURCE_CLINE => copy::ROOTS_CLINE`; the two tests around lines 461-474)
- `src/ui/settings.rs` (`modes.cline`, `IRONWIRE_TOOL_CLINE = "cline"`, the tool table at ~1029, the modes builder at ~1066, and the tests at ~2964-3098 that pin Gemini's unknown wiring; add the Cline twin)
- `src/model.rs` (`cline_source_mode: String` beside `gemini_source_mode`, `#[serde(default)]` like it)
- `src/backend.rs` (the settings round-trip test at ~620-648 that declares a Gemini root; add a Cline root to it)

- [ ] Read each Gemini site, add the Cline twin, tests first where a test exists.
- [ ] Verify: `cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml` and clippy with the repo allow-list. If GTK cannot link on this Mac, `cargo check` at minimum and say so.
- [ ] Commit: `Offer Cline on the GTK roots and settings screens`.

---

### Task 3: macOS

**Files** (from `grep -rni gemini macos/Sources macos/Tests`):
- `TCShellCore/RoutingCopy.swift` (`toolCline`, coding key `tool_cline`)
- `TCShellCore/RoutingSurface.swift` (`RoutingSourceModes.cline`, `.unset`, `ToolID.cline = "cline"`, the row table at ~443, the comment at ~322 that says Gemini has no upstream row)
- `TCShellCore/SessionRoots.swift` (`cline: SourceChoice`, the subscript cases, `cline_source` in the payload; the comment at ~104-109 on the start gate gains Cline; the gate itself stays two-conjunct)
- `TCShellCore/SourceCandidate.swift` (`case cline = "cline"`, display name `Cline`)
- `TraceCommonsApp/Models.swift` (display name for `"cline"`; `clineSourceMode: String?` with coding key `cline_source_mode`, defaulting to `unset`)
- `TraceCommonsApp/Views/OnboardingRootsView.swift` (a Cline row with the same optional-row treatment as Gemini; read the long comment at ~104-113 first, it explains exactly the failure this row must not repeat)
- Tests: `RoutingSurfaceTests`, `SessionRootsTests`, `OnboardingRootsRowsTests`, `RoutingCallTests`, `RoutingSurfaceExportTests` -- each has Gemini cases; add the Cline twin of each.

- [ ] `cargo build -p trace-commons-contributor-ffi` with the shared `CARGO_TARGET_DIR` first (the Swift package links the dylib); check `macos/README.md` or `Package.swift` for where it expects the dylib and satisfy that.
- [ ] Tests first, then the sources.
- [ ] Verify: `swift test` in `macos/`. Paste the summary line.
- [ ] Commit: `Offer Cline on the macOS roots and settings screens`.

---

### Task 4: Windows

**Files** (from `grep -rni gemini windows/src windows/tests`):
- `TraceCommons.Interop/SessionRoots.cs` (`SourceDiscovery.Cline = "cline"`, `SessionRoots.Cline` decision, `cline_source` in the payload; the gate comment at ~229-235 gains Cline, the gate stays two-conjunct)
- `TraceCommons.Interop/SessionRootsCopy.cs` (`SourceDiscovery.Cline => "Cline"`)
- `TraceCommons.Interop/SettingsProtocol.cs` (`ClineSourceMode`, `cline_source_mode`)
- `TraceCommons.Interop/RoutingCopy.cs` (`ToolCline`, `tool_cline`)
- `TraceCommons.Interop/RoutingTools.cs` (`Cline` mode property, `ClineId = "cline"`, the row at ~396, the comment at ~328)
- `TraceCommons.Interop/NativeMethods.cs:326` (doc comment)
- `TraceCommons.App/ViewModels/SessionRootsViewModel.cs` (`Cline` row, added to `Rows`, `PropertyChanged`, and the decision copy at ~204)
- `TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs:1001` (`Cline = settings?.ClineSourceMode`)
- `windows/tests/TraceCommons.Interop.Tests/RoutingSurfaceTests.cs` and any other test that enumerates the three sources.

- [ ] Tests first, then the sources.
- [ ] Verify: `dotnet test windows/tests/TraceCommons.Interop.Tests` (check `windows/README.md` for the exact invocation; the App project may be Windows-only and unbuildable here, in which case build and test what builds on macOS and say precisely what did not).
- [ ] Commit: `Offer Cline on the Windows roots and settings screens`.
