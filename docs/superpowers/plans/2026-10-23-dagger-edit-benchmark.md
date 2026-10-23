# Warm-cache Rust edit benchmark plan

**Goal:** Measure ordinary edit feedback using the same existing Dagger build lane on laptop, GCP, and standard GitHub Actions.

**Frozen scope:** Start from Dagger PR commit 208b68ab3. Run `dagger --progress=plain -v check '*:build'`; Rust 1.99.1, Node 24.22.1, RUSTFLAGS=-D warnings, CARGO_INCREMENTAL=0, four Cargo jobs, debug info disabled, unprivileged CI user. No production code or main-branch CI changes.

**Experiment:** Warm the build lane once; measure unchanged baseline; apply three unique Boolean reorderings in protocol UsdMicrosRange::is_valid, each logically equivalent because end_ok is pure; measure each; repeat the final input unchanged. Require protocol recompilation for each edit and cached whole-check results for unchanged repeats. Restore the Rust file on every exit. Keep exact source, module, lockfile, and harness hashes with raw logs and monotonic timings.

**Boundaries:** The laptop runs Linux/ARM64 in Colima; GCP and GitHub run Linux/AMD64. Record available CPU/RAM. Prime timings are not comparable cold-build benchmarks, because initial cache histories differ. This benchmarks the build lane, not the full test suite. GitHub's warm cache persists within this job; cross-job sharing is not established. Docker engine CPU counters are telemetry, not Dagger Cloud billing.

- [ ] Freeze and review harness and branch-only GitHub workflow.
- [ ] Run laptop, dedicated bounded GCP VM, and GitHub independently.
- [ ] Verify every stage passed, edit recompilations occurred, source restored, and all hashes match.
- [ ] Compare medians and costs; retain receipts; stop Colima and delete temporary GCP resources.
