# Reproduce the released writer fixtures

The source pin is `rust-v0.154.0`, commit `6b9826e3aa83b1a5947db50f4332cb9c65f1b340` in the upstream Codex repository. Use an isolated checkout and the toolchain specified by that checkout. This harness uses synthetic localhost responses and an innocuous local command; it needs no real provider credentials or private trace input.

1. In the upstream checkout, apply `workspace-version-lock-release-0.154.0.patch` with `git apply`. The release's workspace version is 0.154.0 while its lockfile still contains 0.0.0 for internal packages. The patch changes exactly 150 internal package versions and no external dependency records. Its SHA-256 is `f05ad0fb05be624c6adb2e41cbcd8df517a201eca498dc77f40bb6de7fd85101`.
2. Copy `generator-release-0.154.0.rs` into `codex-rs/core/tests/suite/insights_direct_profile_writer_fixture.rs`.
3. Add `pub mod insights_direct_profile_writer_fixture;` to `codex-rs/core/tests/suite/mod.rs`.
4. From `codex-rs`, run the command below. The generator uses the fixed temporary output directories shown in the mapping table; preserve any previous fixture run before running it again.

```sh
CARGO_HOME=/private/tmp/codex-fixture-cargo \
CARGO_TARGET_DIR=/private/tmp/codex-fixture-target \
CARGO_INCREMENTAL=0 RUST_MIN_STACK=8388608 \
cargo test --locked -p codex-core --test all insights_direct_profile_writer_fixture -- --nocapture
```

| Generated file | Repository fixture |
| --- | --- |
| `/private/tmp/trace-insights-codex-release-writer-fixtures/codex-alpha-direct.jsonl` | `codex-release-0.154.0-alpha-direct.jsonl` |
| `/private/tmp/trace-insights-codex-release-writer-fixtures/codex-beta-direct.jsonl` | `codex-release-0.154.0-beta-direct.jsonl` |
| `/private/tmp/trace-insights-codex-release-tool-fixtures/codex-tool-reasoning.jsonl` | `codex-release-0.154.0-tool-reasoning.jsonl` |
| `/private/tmp/trace-insights-codex-default-instruction-fixture/codex-default-instructions.jsonl` | `codex-release-0.154.0-default-instructions.jsonl` |

Compare SHA-256 values with `manifest-release-0.154.0.json`, including its generator digest. Run the harness a second time and verify identical fixture bytes. The manifest in this repository is the combined admission manifest; the generator's direct-fixture `manifest.json` is a separate harness artifact.

The writer flushes complete rollouts before sanitization. Sanitization replaces paths and identifiers consistently, preserves their equality relationships and record structure, and normalizes synthetic timestamps. The tool fixture also asserts a successful command result before sanitization. This verifies source structure and reproducibility, not production outcomes, wall-clock measurements, model quality, or live serving identity. The source-profile document states the complete admission boundary.
