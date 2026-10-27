# Private AI default tool connections

> **For agentic workers:** Use subagent-driven development with separate Rust backend and native Settings ownership; final review and validation cover the combined branch.

**Goal:** Settings → Private AI offers persistent default routing for Claude Code, Codex and OpenCode through the shared proxy, with explicit Disconnect and the existing proxy switch.

**Architecture:** Reuse harness list → preview → single-use commit. Claude/Codex keep their existing editors; OpenCode gets a version-aware editor integrated into the same guarded transaction. Keep controls in Settings, as explicitly requested, and the main Private AI screen's existing Settings link. Global configuration supplies defaults; project, environment and managed-session settings may override them.

**Tech stack:** Rust contributor daemon and existing FFI; native macOS SwiftUI; existing OpenCode V1/V2 config shapes.

## Decisions

- Reuse existing Settings panels rather than add another control surface. Do not change the FTUX toggle or managed-session routing.
- Show installed tools and retained configurations that can be disconnected. A stopped proxy or missing executable must not strand a saved connection.
- OpenCode requires a custom provider/default-model edit; the generic catalog cannot express this. Use real model IDs from a bounded read of the live proxy, never a fabricated default.
- Save only after a displayed preview is confirmed. Preserve unrelated user configuration, reject unsupported/malformed/ambiguous inputs, and refuse stale previews and occupied app-owned slots.
- Keep ownership state private and compare its preimage alongside the tool config. Disconnect restores only still-owned values, never an entire historical backup.
- Proxy Off stops the listener and retains explicit tool connections; Disconnect reverses a tool's routing. This avoids silently switching providers.
- Add no dependencies or upstream pin changes. Tests use isolated temporary homes; never configure the developer's real tools.

## Tasks

- [x] Extend OpenCode list/plan/commit/disconnect for V1/V2, with bounded installed/version/model discovery and ownership-aware restoration.
- [x] Preserve native Settings control placement and visible disconnects; describe saved default routing in shared copy. Add async plan dispatch and refusal bindings.
- [x] Run decisive Rust/FFI/native regression suites, review the complete diff, and prepare the PR. No merge/release is authorized by this request.

## Verification

OpenCode V1/V2 round trips; actual available default model; restore previous selection; preserve intervening user edits; reject occupied/malformed/version-unknown/stale/replayed plans; disconnect while proxy off or executable gone. Existing Claude/Codex harness regressions remain green. Native tests verify Settings placement, all three tool rows, stale-port/off disconnect visibility and old payload compatibility. Run formatting, contributor/FFI Clippy, contributor tests, refreshed FFI before Swift tests, and ABI/copy contracts. Verify Windows/GTK enum changes if needed; state untested platform gates accurately.
