# CI advisory remediation plan

> **For agentic workers:** Use disjoint scopes: the chrono policy/tests slice and the ZIP dependency investigation. The coordinator owns dependency updates, aggregate checks and PR creation.

**Goal:** Clear the six failing advisory jobs without disabling advisory enforcement or concealing reachable vulnerabilities.

**Architecture:** Continue to use the shared reviewed policy in deny.toml. Treat the no-fixed-release chrono maintenance warnings separately from ZIP security findings. Prefer a compatible upstream dependency update for ZIP; verify both the resolved graph and the local model integration before accepting it.

**Tech Stack:** Cargo, cargo-audit, cargo-deny, Python policy wrappers, existing optional mistralrs/Candle inference stack.

- [x] Reproduce current failures with a fresh isolated RustSec database and identify all affected lockfiles and dependency paths.
- [x] Add exact chrono/chrono-tz maintenance exceptions with migration/removal conditions; test preservation of denial flags and nonzero failures in the audit wrapper.
- [x] Upgrade pinned mistralrs to 0.9.5 and Candle to 0.11.1, resolving ZIP 9.4.1. Remove all four obsolete ZIP exceptions; no new ZIP exceptions were added.
- [x] Verify root, GTK and Tauri audits and all-feature deny checks; check licenses under the four required feature sets if dependencies change. Compile and test the affected local inference integration, plus required license boundary and formatting.
- [x] Independently review the final diff and prepare the PR. Hosted advisory CI will be checked after publication. No automatic merge, deployment or release.

Baseline: RustSec 233155b4025b854c68a4b199c0e32d7fd53291a2; root has ZIP0545/0546/0547/0548 plus chrono0543/chrono-tz0555; GTK and Tauri have only the two maintenance warnings. No changes to dependency manifests or lockfiles caused the findings in PR #2169.

User explicitly approved the pinned mistralrs 0.9.5 update and its added sandbox/code-execution dependencies. Scoring must keep those capabilities disabled. No GPU qualification or model download is authorized or claimed by this source/CI repair.

## Verification so far

- Fresh shared-policy cargo-audit: root, GTK and Tauri all passed.
- All-feature cargo-deny: advisories, licenses, sources and bans passed for all three workspaces. Root licenses also passed under default, near-ai-scorer and local-gpu-models feature sets.
- Nine Python policy tests passed; unknown warnings and cargo failure statuses still fail. Formatting and diff checks passed.
- Server/ingest non-CUDA local-model feature compile passed. Local scorer tests: 25 passed, one hardware integration test ignored; both new request contract tests passed.
- All resolved dependencies declare Rust versions no higher than the repository's 1.96 floor; the local build used rustc 1.100.0.
- Read-only source review found no enabled new execution capabilities. Upstream GPU attention/memory defaults changed; GPU loading, numerical parity and CUDA qualification remain unverified and required before deploying the local inference backend.

The extra local-gpu-models Clippy run reports `await_holding_lock` in unchanged scorer code. The same command against a clean archive of base43bf0bb68 reproduces the same diagnostic (old line401, updated line418) with rustc1.100.0. No lint suppression or unrelated concurrency change was added. The normal CI server/ingest Clippy scope is checked separately.

The normal CI server/ingest Clippy command passed with the existing allow-list. Required license boundary: four passed. No suppression was added for the separately reproduced, pre-existing feature-only lint.

Final independent review found no remaining material issues. Hosted CI results belong to the resulting PR and are separate from the local evidence above.
