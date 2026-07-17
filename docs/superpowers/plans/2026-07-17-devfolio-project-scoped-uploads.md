# Devfolio Project-Scoped Uploads Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a devfolio hackathon participant upload only the traces from one project directory and stamp every uploaded envelope with a self-asserted devfolio submission id.

**Architecture:** Two independent changes in the `trace-commons-contributor` crate. (1) Sharpen the existing `--project` filter so it matches the session's *true decoded working directory* (hyphen-safe) instead of the unreliable cwd basename. (2) Add a self-asserted `--devfolio-submission <id>` that rides through `ContributorConfig` into `build_raw_contribution`, where it is written as a `feature_flags["devfolio_submission_id"]` entry on the envelope. The server stores the envelope opaquely — no protocol schema change, no migration, no route.

**Tech Stack:** Rust, clap, serde, anyhow. Crate: `crates/trace-commons-contributor`.

## Global Constraints

- PostgreSQL-only repo; a single `cargo check -p trace-commons-server` is sufficient. Do NOT add feature-flag/dual-backend verification.
- CI applies `RUSTFLAGS=-D warnings` to check + test. Always verify with the `RUSTFLAGS` form; plain `cargo check` hides warnings CI fails on.
- Clippy is CI-enforced with this allow-list (do not widen): `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`.
- No new dependencies. Reuse existing clap/serde/anyhow surface.
- Hash-only audit: the devfolio submission id is devfolio-issued, non-secret attribution metadata carried in the envelope (same class as the existing `feature_flags["project"]`). It is never an authorization input and never gates a read/write path.
- No emojis in commits/code. Short imperative commit subjects, no `feat:`/`fix:` prefixes.
- **Naming:** the devfolio field is `devfolio_submission_id` everywhere. Do NOT reuse the bare name `submission_id` — that already exists as the internal per-session id (`source::submission_id_for`, `SubmitOutcome::Submitted { submission_id }`, `envelope.submission_id`). The user-facing flag is `--devfolio-submission` (chosen over the spec's illustrative `--submission` precisely to avoid this collision).

---

### Task 1: Sharpen `--project` to match the true session cwd

**Files:**
- Modify: `crates/trace-commons-contributor/src/commands.rs` (`discover_filtered`, ~lines 189-222; add a pure helper + a `#[cfg(test)]` module)

**Interfaces:**
- Produces: `fn cwd_matches_project(cwd: Option<&str>, legacy_project: Option<&str>, path: &Path, project: &Path) -> bool` — pure project-filter predicate used by `discover_filtered`.
- Consumes: `crate::source::{SessionRef, TraceSource}`, existing `source_for(name) -> Option<Box<dyn TraceSource>>`, `SessionTranscript.cwd` (the true decoded working dir, `Option<String>`).

**Background:** Today `discover_filtered` matches `--project` on `SessionRef.project`, which is the cwd *basename* decoded from the encoded directory name — unreliable for hyphenated project names (a project named `my-hack` decodes to only `hack`; see `source/claude_code.rs:58-72`). The true working directory is available as `SessionTranscript.cwd` after `source.load()`. We prefer that literal path (hyphen-safe) and fall back to the legacy heuristic when it is unavailable.

- [ ] **Step 1: Write the failing test**

Append (or add to an existing `#[cfg(test)] mod tests`) at the end of `crates/trace-commons-contributor/src/commands.rs`:

```rust
#[cfg(test)]
mod project_filter_tests {
    use super::cwd_matches_project;
    use std::path::Path;

    #[test]
    fn true_cwd_prefix_matches_including_hyphenated_name() {
        // Project literally named "my-hack" — the legacy basename would decode
        // to "hack" and miss it; the true cwd matches exactly.
        let cwd = Some("/Users/dev/code/my-hack");
        assert!(cwd_matches_project(
            cwd,
            Some("hack"),
            Path::new("/Users/dev/.claude/projects/-Users-dev-code-my-hack/s.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
    }

    #[test]
    fn true_cwd_excludes_sibling_and_prefix_collision() {
        // Sibling dir and a "my-hack-2" name must NOT match "my-hack".
        assert!(!cwd_matches_project(
            Some("/Users/dev/code/other"),
            None,
            Path::new("/x.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
        assert!(!cwd_matches_project(
            Some("/Users/dev/code/my-hack-2"),
            None,
            Path::new("/x.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
    }

    #[test]
    fn falls_back_to_basename_or_path_prefix_when_cwd_unknown() {
        // No true cwd available -> legacy heuristic: basename match ...
        assert!(cwd_matches_project(
            None,
            Some("my-hack"),
            Path::new("/somewhere/s.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
        // ... or session-file path prefix.
        assert!(cwd_matches_project(
            None,
            None,
            Path::new("/Users/dev/code/my-hack/s.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
        // Neither matches -> false.
        assert!(!cwd_matches_project(
            None,
            Some("other"),
            Path::new("/elsewhere/s.jsonl"),
            Path::new("/Users/dev/code/my-hack"),
        ));
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor project_filter_tests 2>&1 | tail -20`
Expected: FAIL — `cannot find function cwd_matches_project in this scope`.

- [ ] **Step 3: Add the pure helper**

In `crates/trace-commons-contributor/src/commands.rs`, immediately above `fn discover_filtered` (around line 189), add:

```rust
/// Pure predicate for the `--project` filter. Prefers the session's true
/// decoded working directory (`cwd`) for a hyphen-safe, component-wise
/// path-prefix match; falls back to the legacy basename-or-path heuristic
/// only when the true cwd is unavailable.
fn cwd_matches_project(
    cwd: Option<&str>,
    legacy_project: Option<&str>,
    path: &Path,
    project: &Path,
) -> bool {
    if let Some(cwd) = cwd {
        return Path::new(cwd).starts_with(project);
    }
    let basename = project
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    legacy_project == Some(basename) || path.starts_with(project)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p trace-commons-contributor project_filter_tests 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 5: Wire the helper into `discover_filtered`**

In `crates/trace-commons-contributor/src/commands.rs`, replace the `project_ok` block inside the `refs.retain(...)` closure (currently lines ~206-214):

```rust
        let project_ok = match project_filter {
            None => true,
            Some(p) => {
                let basename = p.file_name().and_then(|n| n.to_str()).unwrap_or_default();
                let basename_match = r.project.as_deref() == Some(basename);
                let prefix_match = r.path.starts_with(p);
                basename_match || prefix_match
            }
        };
```

with:

```rust
        let project_ok = match project_filter {
            None => true,
            Some(p) => {
                // Prefer the true decoded working directory read from the
                // session file (hyphen-safe); only pay the load cost when a
                // project filter is actually set.
                let cwd = source_for(r.source)
                    .and_then(|s| s.load(r).ok())
                    .and_then(|t| t.cwd);
                cwd_matches_project(cwd.as_deref(), r.project.as_deref(), &r.path, p)
            }
        };
```

Also update the doc comment on `discover_filtered` (lines ~184-188) to state that `project` now matches the session's true working directory when available, falling back to the basename/path heuristic.

- [ ] **Step 6: Verify the crate builds under CI flags**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run 2>&1 | tail -20`
Expected: builds with no warnings/errors.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor/src/commands.rs
git commit -m "Match --project against the true session cwd"
```

---

### Task 2: Persist the devfolio submission id on the envelope

**Files:**
- Modify: `crates/trace-commons-contributor/src/config.rs` (add field to `ContributorConfig` struct ~line 26; update `sample_config` test literal ~line 314)
- Modify: `crates/trace-commons-contributor/src/envelope.rs` (`build_raw_contribution` ~lines 259-277; `test_config` literal ~line 442; add a test)
- Modify: `crates/trace-commons-contributor/src/submit.rs` (`ContributorConfig` test literal ~line 568)
- Modify: `crates/trace-commons-contributor/src/identity.rs` (`ContributorConfig` test literal ~line 313)
- Modify: `crates/trace-commons-contributor/src/commands.rs` (`ContributorConfig` production literal in `login` ~line 66)

**Interfaces:**
- Produces: `ContributorConfig.devfolio_submission_id: Option<String>` — self-asserted devfolio submission id; when `Some`, `build_raw_contribution` writes `feature_flags["devfolio_submission_id"]`.
- Consumes: existing `build_raw_contribution(t, cfg, now)` (signature unchanged — reads the new cfg field).

**Background:** Store-opaquely means the id lives inside the envelope with no dedicated column. `feature_flags` is a `BTreeMap<String,String>` already used for `project`/`agent`/`cwd_hash`, so it is the zero-schema-change home. `#[serde(default)]` keeps old config files loadable; `skip_serializing_if` keeps new files tidy.

- [ ] **Step 1: Write the failing test**

In `crates/trace-commons-contributor/src/envelope.rs`, inside the existing `#[cfg(test)] mod tests`, add (the `test_config()` and `fixture_transcript()` helpers already exist in that module):

```rust
    #[test]
    fn devfolio_submission_id_written_to_feature_flags_when_set() {
        let mut cfg = test_config();
        cfg.devfolio_submission_id = Some("devfolio-sub-123".to_string());
        let raw = build_raw_contribution(&fixture_transcript(), &cfg, chrono::Utc::now());
        assert_eq!(
            raw.ironclaw.feature_flags.get("devfolio_submission_id"),
            Some(&"devfolio-sub-123".to_string())
        );
    }

    #[test]
    fn devfolio_submission_id_absent_when_unset() {
        let cfg = test_config(); // devfolio_submission_id defaults to None
        let raw = build_raw_contribution(&fixture_transcript(), &cfg, chrono::Utc::now());
        assert!(!raw
            .ironclaw
            .feature_flags
            .contains_key("devfolio_submission_id"));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor devfolio_submission_id 2>&1 | tail -20`
Expected: FAIL — `no field devfolio_submission_id on type ContributorConfig`.

- [ ] **Step 3: Add the config field**

In `crates/trace-commons-contributor/src/config.rs`, add to the `ContributorConfig` struct (after `allowed_hosts`, line ~37):

```rust
    /// Self-asserted devfolio submission id, stamped onto every uploaded
    /// envelope's `feature_flags`. Attribution only; never an authorization
    /// input. Set once here or overridden per-run by `--devfolio-submission`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devfolio_submission_id: Option<String>,
```

- [ ] **Step 4: Fix every `ContributorConfig {}` literal**

Add `devfolio_submission_id: None,` to each of these struct literals:
- `crates/trace-commons-contributor/src/commands.rs:66` (production `login` writer)
- `crates/trace-commons-contributor/src/config.rs:314` (`sample_config`, test)
- `crates/trace-commons-contributor/src/submit.rs:568` (test)
- `crates/trace-commons-contributor/src/identity.rs:313` (test)
- `crates/trace-commons-contributor/src/envelope.rs:442` (`test_config`, test)

- [ ] **Step 5: Write the feature-flags entry**

In `crates/trace-commons-contributor/src/envelope.rs`, in `build_raw_contribution`, immediately after the `feature_flags.insert("project", ...)` block (line ~270), add:

```rust
    if let Some(id) = cfg.devfolio_submission_id.as_deref() {
        feature_flags.insert("devfolio_submission_id".to_string(), id.to_string());
    }
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor devfolio_submission_id 2>&1 | tail -20`
Expected: PASS (2 tests).

- [ ] **Step 7: Verify the crate builds under CI flags**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run 2>&1 | tail -20`
Expected: builds clean.

- [ ] **Step 8: Commit**

```bash
git add crates/trace-commons-contributor/src/config.rs crates/trace-commons-contributor/src/envelope.rs crates/trace-commons-contributor/src/submit.rs crates/trace-commons-contributor/src/identity.rs crates/trace-commons-contributor/src/commands.rs
git commit -m "Stamp self-asserted devfolio submission id on envelope"
```

---

### Task 3: Expose `--devfolio-submission` and thread it to the config

**Files:**
- Modify: `crates/trace-commons-contributor/src/submit.rs` (`SubmitOptions` ~lines 44-47; `effective_config` ~lines 308-314; add a test in the existing test module)
- Modify: `crates/trace-commons-contributor/src/commands.rs` (`SubmitSelection` ~lines 325-333; `submit()` `SubmitOptions` construction ~lines 390-393)
- Modify: `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs` (`Submit` clap variant ~lines 39-60; match arm ~lines 106-124)

**Interfaces:**
- Consumes: `ContributorConfig.devfolio_submission_id` (Task 2), existing `SubmitOptions`/`effective_config` override pattern (mirrors `pii_filter`), existing `SubmitSelection` flow.
- Produces: CLI flag `--devfolio-submission <id>` → `SubmitSelection.devfolio_submission: Option<&str>` → `SubmitOptions.devfolio_submission_id: Option<String>` → `effective_config` sets `ContributorConfig.devfolio_submission_id`.

- [ ] **Step 1: Write the failing test**

In `crates/trace-commons-contributor/src/submit.rs`, inside the existing `#[cfg(test)] mod tests` (near the `ContributorConfig` test builder at line ~567), add:

```rust
    #[test]
    fn effective_config_applies_devfolio_submission_override() {
        let mut cfg = sample_submit_config();
        cfg.devfolio_submission_id = Some("from-config".to_string());
        let opts = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            devfolio_submission_id: Some("from-flag".to_string()),
        };
        let eff = effective_config(&cfg, &opts);
        assert_eq!(eff.devfolio_submission_id.as_deref(), Some("from-flag"));

        // When the flag is absent, the config value survives.
        let opts_none = SubmitOptions {
            dry_run: false,
            pii_filter: None,
            devfolio_submission_id: None,
        };
        let eff2 = effective_config(&cfg, &opts_none);
        assert_eq!(eff2.devfolio_submission_id.as_deref(), Some("from-config"));
    }
```

NOTE: name the config builder to match whatever the existing test module uses (the literal at `submit.rs:567-568` — likely a helper like `sample_submit_config()` or an inline builder). If it is an inline literal rather than a named helper, construct the `ContributorConfig` the same way inside this test.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor effective_config_applies_devfolio 2>&1 | tail -20`
Expected: FAIL — `SubmitOptions` has no field `devfolio_submission_id`.

- [ ] **Step 3: Add the `SubmitOptions` field**

In `crates/trace-commons-contributor/src/submit.rs`, extend `SubmitOptions` (lines 44-47):

```rust
pub struct SubmitOptions {
    pub dry_run: bool,
    pub pii_filter: Option<String>,
    pub devfolio_submission_id: Option<String>,
}
```

- [ ] **Step 4: Apply the override in `effective_config`**

In `crates/trace-commons-contributor/src/submit.rs`, extend `effective_config` (lines 308-314), mirroring the `pii_filter` override:

```rust
fn effective_config(cfg: &ContributorConfig, opts: &SubmitOptions) -> ContributorConfig {
    let mut c = cfg.clone();
    if opts.pii_filter.is_some() {
        c.pii_filter = opts.pii_filter.clone();
    }
    if opts.devfolio_submission_id.is_some() {
        c.devfolio_submission_id = opts.devfolio_submission_id.clone();
    }
    c
}
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p trace-commons-contributor effective_config_applies_devfolio 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: Thread through `SubmitSelection` and `submit()`**

In `crates/trace-commons-contributor/src/commands.rs`, add a field to `SubmitSelection` (lines 325-333):

```rust
pub struct SubmitSelection<'a> {
    pub all: bool,
    pub since: Option<&'a str>,
    pub project: Option<&'a Path>,
    pub source: Option<&'a str>,
    pub yes: bool,
    pub dry_run: bool,
    pub pii_filter: Option<&'a str>,
    pub devfolio_submission: Option<&'a str>,
}
```

Then in `submit()`, extend the `SubmitOptions` construction (lines ~390-393):

```rust
    let opts = SubmitOptions {
        dry_run: sel.dry_run,
        pii_filter: sel.pii_filter.map(str::to_string),
        devfolio_submission_id: sel.devfolio_submission.map(str::to_string),
    };
```

- [ ] **Step 7: Add the CLI flag and thread it in `main`**

In `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs`, add to the `Submit` variant (after `pii_filter`, line ~59):

```rust
        /// Devfolio submission id to stamp on every uploaded envelope
        /// (self-asserted attribution; overrides the config value)
        #[arg(long = "devfolio-submission")]
        devfolio_submission: Option<String>,
```

Extend the `Command::Submit { .. }` destructuring (lines 106-113) to include `devfolio_submission,` and the `SubmitSelection { .. }` construction (lines 115-123):

```rust
            let sel = commands::SubmitSelection {
                all,
                since: since.as_deref(),
                project: project.as_deref(),
                source: source.as_deref(),
                yes,
                dry_run,
                pii_filter: pii_filter.as_deref(),
                devfolio_submission: devfolio_submission.as_deref(),
            };
```

- [ ] **Step 8: Verify the crate builds under CI flags**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run 2>&1 | tail -20`
Expected: builds clean (this catches every other `SubmitSelection`/`SubmitOptions` construction site the compiler now requires the new field on).

- [ ] **Step 9: Commit**

```bash
git add crates/trace-commons-contributor/src/submit.rs crates/trace-commons-contributor/src/commands.rs crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs
git commit -m "Add --devfolio-submission flag threading to config"
```

---

### Task 4: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Tests build under CI flags**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run 2>&1 | tail -20`
Expected: clean build.

- [ ] **Step 2: Tests pass**

Run: `cargo test -p trace-commons-contributor 2>&1 | tail -20`
Expected: all pass (including the 5 new tests).

- [ ] **Step 3: Clippy clean under the repo allow-list**

Run:
```bash
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching 2>&1 | tail -20
```
Expected: no warnings.

- [ ] **Step 4: Manual smoke of the flag surface**

Run: `cargo run -p trace-commons-contributor --bin trace-commons-contributor -- submit --help 2>&1 | tail -20`
Expected: help lists `--devfolio-submission <DEVFOLIO_SUBMISSION>` and `--project <PROJECT>`.

- [ ] **Step 5: Confirm the whole workspace still builds under CI flags**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins 2>&1 | tail -20`
Expected: clean (no cross-crate breakage from the protocol/contributor edges).

## Self-Review notes

- **Spec coverage:** scope control → Task 1 (`--project` sharpening) + interactive picker unchanged; envelope→submission link → Task 2 (`feature_flags["devfolio_submission_id"]`); user-controlled submission id → Task 3 (`--devfolio-submission` + config default). Store-opaquely / no-migration / no-route honored (no server or protocol files touched). Self-asserted / attribution-only honored (no verification path).
- **Naming collision:** guarded in Global Constraints — devfolio id is always `devfolio_submission_id`; the flag is `--devfolio-submission` (deliberate deviation from the spec's illustrative `--submission`).
- **Out of scope (unchanged):** no devfolio-signed attestation, no typed protocol field, no server column/index/route, no id verification.
