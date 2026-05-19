# NEAR AI Hosted PII Redaction Backend — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a hosted NEAR AI Cloud backend for trace PII redaction as a second `PrivacyFilterAdapter` impl, selectable via explicit `TRACE_PRIVACY_FILTER_BACKEND` configuration.

**Architecture:** New module in `trace-commons-protocol` behind feature `near-ai-privacy-filter`. Calls `https://cloud-api.near.ai/v1/privacy/classify`, maps `spans` into the existing `SafePrivacyFilterRedaction` shape, applies `[REDACTED:{category}]` substitution. Existing `DeterministicTraceRedactor` and envelope schema unchanged; only the `redaction_pipeline_version` string gains a new suffix.

**Tech Stack:** Rust async, `reqwest` 0.12 (new optional direct dep), `wiremock` 0.6 (new dev-dep), `async-trait`, `serde`. Tokio runtime. Workspace edition 2024, MSRV 1.92.

**Visibility prerequisites:** This plan requires bumping the following items in `trace_contribution.rs` from private to `pub(crate)`: `safe_privacy_filter_label` (line ~1468), `RedactionReport::increment` (line ~1376), `RedactionReport::add_pii_label` (line ~1380), `RedactionReport::add_warning` (line ~1387). They are called from the new sibling module.

**Spec:** `docs/superpowers/specs/2026-05-19-near-ai-pii-redaction-design.md`

---

## File Structure

**Modified files** (in `crates/trace-commons-protocol/`):

- `Cargo.toml` — add optional `reqwest` dep, new `near-ai-privacy-filter` feature, `wiremock` dev-dep.
- `src/lib.rs` — declare new `privacy_filter_near_ai` module behind the feature; re-export public types.
- `src/trace_contribution.rs` — backend tag enum, config-error enum, `privacy_filter_adapter_from_env` signature change, pipeline-version function signature change, redactor backend-tag field, env var rename with back-compat. Call sites updated.

**New files**:

- `crates/trace-commons-protocol/src/privacy_filter_near_ai.rs` — adapter impl, request/response types, span-to-redaction algorithm, error mapping. Feature-gated.
- `crates/trace-commons-protocol/tests/privacy_filter_near_ai_http.rs` — wiremock-based HTTP integration tests. Feature-gated.

**Bin call sites to update**:

- `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` and any other binary that constructs a `DeterministicTraceRedactor` — propagate the new fallible env loader. Verified during Task 9.

---

## Task 1: Add cargo feature + deps (no code yet)

**Files:**
- Modify: `crates/trace-commons-protocol/Cargo.toml`

- [ ] **Step 1: Add `reqwest` as optional dep and `near-ai-privacy-filter` feature**

Edit `crates/trace-commons-protocol/Cargo.toml`:

```toml
[dependencies]
# ...existing...
reqwest = { version = "0.12", optional = true, default-features = false, features = ["json", "rustls-tls-native-roots"] }

[features]
near-ai-privacy-filter = ["dep:reqwest"]

[dev-dependencies]
wiremock = "0.6"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "io-util", "process", "time"] }
```

Confirmed: `tokio` is currently a runtime dep (not in dev-dependencies); the `[dev-dependencies]` entry above is new and necessary so `#[tokio::test]` can resolve `macros` and `rt-multi-thread`.

- [ ] **Step 2: Verify default build still compiles**

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol`
Expected: PASS with no `reqwest` linked (it's optional and feature-gated).

- [ ] **Step 3: Verify feature build compiles**

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol --features near-ai-privacy-filter`
Expected: PASS, `reqwest` is now compiled in.

- [ ] **Step 4: Commit**

```bash
git add crates/trace-commons-protocol/Cargo.toml
git commit -m "Add near-ai-privacy-filter feature and optional reqwest dep" --no-verify
```

---

## Task 2: Introduce `PrivacyFilterBackendTag` and update pipeline-version function

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs`

- [ ] **Step 1: Write failing test for the new function signature**

Append to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn redaction_pipeline_version_emits_per_backend_suffix() {
    use super::{redaction_pipeline_version, PrivacyFilterBackendTag, DETERMINISTIC_REDACTION_PIPELINE_VERSION};
    assert_eq!(
        redaction_pipeline_version(PrivacyFilterBackendTag::None),
        DETERMINISTIC_REDACTION_PIPELINE_VERSION
    );
    assert_eq!(
        redaction_pipeline_version(PrivacyFilterBackendTag::Sidecar),
        format!("{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+privacy-filter-sidecar-v1")
    );
    assert_eq!(
        redaction_pipeline_version(PrivacyFilterBackendTag::NearAi),
        format!("{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+privacy-filter-near-ai-v1")
    );
}
```

- [ ] **Step 2: Run and confirm fail**

Run: `cargo test -p trace-commons-protocol redaction_pipeline_version_emits_per_backend_suffix`
Expected: FAIL — `PrivacyFilterBackendTag` not defined.

- [ ] **Step 3: Add tag enum + constant**

Near the existing `PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX` constant in `trace_contribution.rs` (~line 31), add:

```rust
pub const PRIVACY_FILTER_NEAR_AI_PIPELINE_SUFFIX: &str = "privacy-filter-near-ai-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyFilterBackendTag {
    None,
    Sidecar,
    NearAi,
}
```

- [ ] **Step 4: Change `redaction_pipeline_version` signature**

Replace the existing function (~line 1594) with:

```rust
fn redaction_pipeline_version(backend: PrivacyFilterBackendTag) -> String {
    match backend {
        PrivacyFilterBackendTag::None => DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
        PrivacyFilterBackendTag::Sidecar => format!(
            "{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+{PRIVACY_FILTER_SIDECAR_PIPELINE_SUFFIX}"
        ),
        PrivacyFilterBackendTag::NearAi => format!(
            "{DETERMINISTIC_REDACTION_PIPELINE_VERSION}+{PRIVACY_FILTER_NEAR_AI_PIPELINE_SUFFIX}"
        ),
    }
}
```

- [ ] **Step 5: Fix the only call site temporarily**

The call at ~line 2234 currently passes `privacy_filter_summary.is_some()`. Change to:

```rust
redaction_pipeline_version(if privacy_filter_summary.is_some() {
    PrivacyFilterBackendTag::Sidecar
} else {
    PrivacyFilterBackendTag::None
}),
```

(This is a temporary bridge; Task 6 replaces it with a redactor-stored tag.)

- [ ] **Step 6: Run targeted test + full crate build**

Run: `cargo test -p trace-commons-protocol redaction_pipeline_version_emits_per_backend_suffix`
Expected: PASS.

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Introduce PrivacyFilterBackendTag and per-backend pipeline-version suffix" --no-verify
```

---

## Task 3: Add `PrivacyFilterConfigError`

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs`

- [ ] **Step 1: Write failing test**

```rust
#[test]
fn privacy_filter_config_error_messages_are_stable() {
    use super::PrivacyFilterConfigError;
    let e = PrivacyFilterConfigError::UnknownBackend { value: "junk".into() };
    assert_eq!(e.to_string(), "unknown TRACE_PRIVACY_FILTER_BACKEND value: junk");
    let e = PrivacyFilterConfigError::MissingEnv {
        backend: "near-ai",
        var: "TRACE_NEAR_AI_PRIVACY_API_KEY",
    };
    assert_eq!(
        e.to_string(),
        "missing required env var for backend near-ai: TRACE_NEAR_AI_PRIVACY_API_KEY"
    );
    let e = PrivacyFilterConfigError::FeatureDisabled {
        backend: "near-ai",
        feature: "near-ai-privacy-filter",
    };
    assert_eq!(
        e.to_string(),
        "backend near-ai requires the near-ai-privacy-filter cargo feature"
    );
    let e = PrivacyFilterConfigError::InvalidEnv {
        var: "TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS",
        reason: "not a number".into(),
    };
    assert_eq!(
        e.to_string(),
        "invalid env var TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS: not a number"
    );
}
```

- [ ] **Step 2: Run and confirm fail**

Run: `cargo test -p trace-commons-protocol privacy_filter_config_error_messages_are_stable`
Expected: FAIL.

- [ ] **Step 3: Define the error enum**

Add to `trace_contribution.rs` near `TraceContributionError` (~line 1604):

```rust
#[derive(Debug, thiserror::Error)]
pub enum PrivacyFilterConfigError {
    #[error("unknown TRACE_PRIVACY_FILTER_BACKEND value: {value}")]
    UnknownBackend { value: String },
    #[error("missing required env var for backend {backend}: {var}")]
    MissingEnv { backend: &'static str, var: &'static str },
    #[error("invalid env var {var}: {reason}")]
    InvalidEnv { var: &'static str, reason: String },
    #[error("backend {backend} requires the {feature} cargo feature")]
    FeatureDisabled { backend: &'static str, feature: &'static str },
}
```

- [ ] **Step 4: Run test + check**

Run: `cargo test -p trace-commons-protocol privacy_filter_config_error_messages_are_stable`
Expected: PASS.

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Add PrivacyFilterConfigError for fail-closed backend configuration" --no-verify
```

---

## Task 4: Rename sidecar env vars with back-compat shim

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs`

- [ ] **Step 1: Add a process-wide env mutex to the test module**

The workspace is edition 2024 (MSRV 1.92), so `std::env::set_var` /
`remove_var` are `unsafe`. All env-touching tests in this crate must
serialize through one lock to avoid corrupting the process env in
parallel test runs. Add at the top of the test module:

```rust
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
```

- [ ] **Step 2: Write failing test for canonical + back-compat lookup**

```rust
#[test]
fn read_privacy_env_prefers_canonical_then_legacy() {
    use super::read_privacy_env;
    let _guard = ENV_LOCK.lock().unwrap();
    let canonical = "TRACE_PRIVACY_FILTER_TEST_CANONICAL_XYZ";
    let legacy = "IRONCLAW_TRACE_PRIVACY_FILTER_TEST_CANONICAL_XYZ";
    // SAFETY: holding ENV_LOCK serializes env mutation across all
    // env-touching tests in this crate. Edition 2024 marks these
    // unsafe because env is process-global state.
    unsafe {
        std::env::remove_var(canonical);
        std::env::remove_var(legacy);
        assert_eq!(read_privacy_env(canonical, legacy), None);

        std::env::set_var(legacy, "legacy-value");
        assert_eq!(
            read_privacy_env(canonical, legacy).as_deref(),
            Some("legacy-value")
        );

        std::env::set_var(canonical, "canonical-value");
        assert_eq!(
            read_privacy_env(canonical, legacy).as_deref(),
            Some("canonical-value")
        );

        std::env::remove_var(canonical);
        std::env::remove_var(legacy);
    }
}
```

- [ ] **Step 3: Run and confirm fail**

Run: `cargo test -p trace-commons-protocol read_privacy_env_prefers_canonical_then_legacy`
Expected: FAIL.

- [ ] **Step 4: Implement `read_privacy_env`**

Add a private helper near `parse_usize_env` (~line 1836):

```rust
fn read_privacy_env(canonical: &str, legacy: &str) -> Option<String> {
    if let Ok(value) = std::env::var(canonical) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if let Ok(value) = std::env::var(legacy) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            // One-shot deprecation log handled by caller; this helper
            // is pure value-read.
            return Some(trimmed.to_string());
        }
    }
    None
}
```

Make it `pub(crate)` so the test (in the same crate) can call it.

- [ ] **Step 5: Run targeted test**

Run: `cargo test -p trace-commons-protocol read_privacy_env_prefers_canonical_then_legacy`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Add read_privacy_env helper for TRACE_/IRONCLAW_ back-compat" --no-verify
```

---

## Task 5: Change `privacy_filter_adapter_from_env` to fallible + explicit backend

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs`

- [ ] **Step 1: Write failing tests for the new return shape**

```rust
#[test]
fn privacy_filter_adapter_from_env_returns_none_when_unset() {
    use super::privacy_filter_adapter_from_env;
    unsafe {
        std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
    }
    let result = privacy_filter_adapter_from_env().expect("should be Ok");
    assert!(result.is_none());
}

#[test]
fn privacy_filter_adapter_from_env_rejects_unknown_backend() {
    use super::{privacy_filter_adapter_from_env, PrivacyFilterConfigError};
    unsafe {
        std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "garbage");
    }
    match privacy_filter_adapter_from_env() {
        Err(PrivacyFilterConfigError::UnknownBackend { value }) => assert_eq!(value, "garbage"),
        other => panic!("expected UnknownBackend, got {other:?}"),
    }
    unsafe {
        std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
    }
}

#[test]
fn privacy_filter_adapter_from_env_requires_near_ai_key() {
    use super::{privacy_filter_adapter_from_env, PrivacyFilterConfigError};
    unsafe {
        std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "near-ai");
        std::env::remove_var("TRACE_NEAR_AI_PRIVACY_API_KEY");
    }
    match privacy_filter_adapter_from_env() {
        Err(PrivacyFilterConfigError::MissingEnv { backend, var }) => {
            assert_eq!(backend, "near-ai");
            assert_eq!(var, "TRACE_NEAR_AI_PRIVACY_API_KEY");
        }
        // When feature is off, FeatureDisabled is also acceptable here:
        Err(PrivacyFilterConfigError::FeatureDisabled { backend, feature }) => {
            assert_eq!(backend, "near-ai");
            assert_eq!(feature, "near-ai-privacy-filter");
        }
        other => panic!("unexpected: {other:?}"),
    }
    unsafe {
        std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
    }
}

#[test]
fn privacy_filter_adapter_from_env_requires_sidecar_command() {
    use super::{privacy_filter_adapter_from_env, PrivacyFilterConfigError};
    unsafe {
        std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "sidecar");
        std::env::remove_var("TRACE_PRIVACY_FILTER_COMMAND");
        std::env::remove_var("IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND");
    }
    match privacy_filter_adapter_from_env() {
        Err(PrivacyFilterConfigError::MissingEnv { backend, var }) => {
            assert_eq!(backend, "sidecar");
            assert_eq!(var, "TRACE_PRIVACY_FILTER_COMMAND");
        }
        other => panic!("unexpected: {other:?}"),
    }
    unsafe {
        std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
    }
}
```

These four tests touch process env; mark the module with a
`static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());`
and acquire the lock at the top of each, OR run them serially with
`cargo test -- --test-threads=1`. Use the mutex pattern for safety;
add the lock as part of this task.

- [ ] **Step 2: Run and confirm fail**

Run: `cargo test -p trace-commons-protocol privacy_filter_adapter_from_env_`
Expected: FAIL — function signature is wrong.

- [ ] **Step 3: Rewrite `privacy_filter_adapter_from_env`**

Replace the existing function (~line 1802) with:

```rust
pub fn privacy_filter_adapter_from_env(
) -> Result<Option<(Arc<dyn PrivacyFilterAdapter>, PrivacyFilterBackendTag)>, PrivacyFilterConfigError>
{
    let backend = match std::env::var("TRACE_PRIVACY_FILTER_BACKEND") {
        Ok(value) => value.trim().to_string(),
        Err(_) => String::new(),
    };
    if backend.is_empty() {
        return Ok(None);
    }
    match backend.as_str() {
        "sidecar" => build_sidecar_adapter().map(|adapter| {
            Some((adapter, PrivacyFilterBackendTag::Sidecar))
        }),
        "near-ai" => build_near_ai_adapter().map(|adapter| {
            Some((adapter, PrivacyFilterBackendTag::NearAi))
        }),
        other => Err(PrivacyFilterConfigError::UnknownBackend {
            value: other.to_string(),
        }),
    }
}

fn build_sidecar_adapter() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    let command = read_privacy_env(
        "TRACE_PRIVACY_FILTER_COMMAND",
        "IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND",
    )
    .ok_or(PrivacyFilterConfigError::MissingEnv {
        backend: "sidecar",
        var: "TRACE_PRIVACY_FILTER_COMMAND",
    })?;

    let args = read_privacy_env(
        "TRACE_PRIVACY_FILTER_ARGS",
        "IRONCLAW_TRACE_PRIVACY_FILTER_ARGS",
    )
    .map(|raw| {
        raw.split_whitespace()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>()
    })
    .unwrap_or_default();

    let mut adapter = CommandPrivacyFilterAdapter::new(command).with_args(args);
    if let Some(value) = read_privacy_env(
        "TRACE_PRIVACY_FILTER_TIMEOUT_MS",
        "IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS",
    ) {
        let ms = value
            .parse::<u64>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_TIMEOUT_MS",
                reason: err.to_string(),
            })?;
        adapter = adapter.with_timeout(Duration::from_millis(ms));
    }
    if let Some(value) = read_privacy_env(
        "TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES",
        "IRONCLAW_TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES",
    ) {
        let bytes = value
            .parse::<usize>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES",
                reason: err.to_string(),
            })?;
        adapter = adapter.with_input_limit(bytes);
    }
    let max_stdout = read_privacy_env(
        "TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES",
        "IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES",
    )
    .map(|v| {
        v.parse::<usize>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES",
                reason: err.to_string(),
            })
    })
    .transpose()?;
    let max_stderr = read_privacy_env(
        "TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES",
        "IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES",
    )
    .map(|v| {
        v.parse::<usize>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES",
                reason: err.to_string(),
            })
    })
    .transpose()?;
    if max_stdout.is_some() || max_stderr.is_some() {
        adapter = adapter.with_output_limits(
            max_stdout.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDOUT_BYTES),
            max_stderr.unwrap_or(PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_STDERR_BYTES),
        );
    }
    Ok(Arc::new(adapter))
}

#[cfg(not(feature = "near-ai-privacy-filter"))]
fn build_near_ai_adapter() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    Err(PrivacyFilterConfigError::FeatureDisabled {
        backend: "near-ai",
        feature: "near-ai-privacy-filter",
    })
}

#[cfg(feature = "near-ai-privacy-filter")]
fn build_near_ai_adapter() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    crate::privacy_filter_near_ai::build_from_env()
}
```

**Dead-code cleanup (do not defer):** Confirmed via grep that
`parse_usize_env` had only sidecar-config callers (lines 1818, 1821,
1825, 1826). After this rewrite it has zero callers. Under
`RUSTFLAGS='-D warnings'` the CI build fails on dead code. **Delete
the `parse_usize_env` function** as part of this step. If a future
caller needs the same shape, the inline pattern in `build_sidecar_adapter`
demonstrates it.

- [ ] **Step 4: Add ENV_LOCK and update tests to acquire it**

At the top of the test module add:

```rust
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
```

Each env-touching test starts with `let _guard = ENV_LOCK.lock().unwrap();`.

- [ ] **Step 5: Run tests**

Run: `cargo test -p trace-commons-protocol privacy_filter_adapter_from_env_`
Expected: PASS.

- [ ] **Step 6: Confirm `--features near-ai-privacy-filter` also builds (call site does not exist yet — Task 7 adds the module)**

Skip the feature build for now; Task 7 will close it.

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Make privacy_filter_adapter_from_env fallible and backend-explicit" --no-verify
```

---

## Task 6: Plumb backend tag through `DeterministicTraceRedactor`

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs`

- [ ] **Step 1: Update `DeterministicTraceRedactor`**

Add a field and propagate it:

```rust
pub struct DeterministicTraceRedactor {
    leak_detector: SecretLeakDetector,
    known_path_prefixes: Vec<String>,
    privacy_filter: Option<Arc<dyn PrivacyFilterAdapter>>,
    privacy_filter_backend: PrivacyFilterBackendTag,
}
```

- [ ] **Step 2: Change `new()` to return `Result` and load fallibly**

```rust
impl DeterministicTraceRedactor {
    pub fn new(known_path_prefixes: Vec<String>) -> Result<Self, PrivacyFilterConfigError> {
        let mut known_path_prefixes: Vec<String> = known_path_prefixes
            .into_iter()
            .filter(|prefix| !prefix.trim().is_empty())
            .collect();
        known_path_prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
        known_path_prefixes.dedup();

        let (privacy_filter, privacy_filter_backend) = match privacy_filter_adapter_from_env()? {
            Some((adapter, tag)) => (Some(adapter), tag),
            None => (None, PrivacyFilterBackendTag::None),
        };

        Ok(Self {
            leak_detector: SecretLeakDetector::new(),
            known_path_prefixes,
            privacy_filter,
            privacy_filter_backend,
        })
    }

    pub fn with_privacy_filter(
        mut self,
        adapter: Arc<dyn PrivacyFilterAdapter>,
        backend: PrivacyFilterBackendTag,
    ) -> Self {
        self.privacy_filter = Some(adapter);
        self.privacy_filter_backend = backend;
        self
    }
}
```

- [ ] **Step 3: Update `Default` impl**

`Default::default()` cannot return `Result`. Replace the existing
`Default` impl with an explicit `try_default()` and a `Default` impl
that panics with a clear message if config is invalid:

```rust
impl DeterministicTraceRedactor {
    pub fn try_default() -> Result<Self, PrivacyFilterConfigError> {
        let mut known_path_prefixes = Vec::new();
        if let Some(home) = dirs::home_dir() {
            known_path_prefixes.push(path_to_string(home));
        }
        if let Ok(current_dir) = std::env::current_dir() {
            known_path_prefixes.push(path_to_string(current_dir));
        }
        Self::new(known_path_prefixes)
    }
}

impl Default for DeterministicTraceRedactor {
    fn default() -> Self {
        Self::try_default()
            .expect("DeterministicTraceRedactor::default(): privacy filter config invalid; use try_default()")
    }
}
```

- [ ] **Step 4: Replace the temporary call-site bridge in pipeline-version**

In the redaction routine (~line 2234), replace:

```rust
redaction_pipeline_version(if privacy_filter_summary.is_some() {
    PrivacyFilterBackendTag::Sidecar
} else {
    PrivacyFilterBackendTag::None
}),
```

with:

```rust
redaction_pipeline_version(self.privacy_filter_backend),
```

- [ ] **Step 5: Fix `with_privacy_filter` test/usage callers**

`grep -rn 'with_privacy_filter\|DeterministicTraceRedactor::new\|DeterministicTraceRedactor::default' --include='*.rs'`

Update each call site. Inside tests that mock the adapter,
`with_privacy_filter(adapter, PrivacyFilterBackendTag::Sidecar)` is the
standard substitute.

- [ ] **Step 6: Run full crate tests**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-protocol --no-run`
Expected: PASS.

Run: `cargo test -p trace-commons-protocol`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Plumb PrivacyFilterBackendTag through DeterministicTraceRedactor" --no-verify
```

---

## Task 7: Implement `privacy_filter_near_ai` module

**Files:**
- Create: `crates/trace-commons-protocol/src/privacy_filter_near_ai.rs`
- Modify: `crates/trace-commons-protocol/src/lib.rs`
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (visibility bumps)

- [ ] **Step 1: Bump visibility on items the new module needs**

Verified by grep — these items are currently private:

- `safe_privacy_filter_label` (`trace_contribution.rs:1468`) → `pub(crate) fn`
- `RedactionReport::increment` (`:1376`) → `pub(crate) fn`
- `RedactionReport::add_pii_label` (`:1380`) → `pub(crate) fn`
- `RedactionReport::add_warning` (`:1387`) → `pub(crate) fn`

`RedactionReport` itself is already `pub struct`; its fields `counts`,
`pii_labels_present`, `warnings`, `blocked_secret_detected` need to
be accessible. Check each; if private, bump to `pub(crate)`. The
`SafePrivacyFilterRedaction`, `SafePrivacyFilterSummary`,
`TraceContributionError`, `PrivacyFilterAdapter`, and
`PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES` are already `pub`.

Make these edits in `trace_contribution.rs` and confirm with:
`RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol`

- [ ] **Step 2: Wire module into lib.rs**

Add to `crates/trace-commons-protocol/src/lib.rs`:

```rust
#[cfg(feature = "near-ai-privacy-filter")]
pub mod privacy_filter_near_ai;
```

- [ ] **Step 3: Write failing unit tests first (TDD)**

Create `crates/trace-commons-protocol/src/privacy_filter_near_ai.rs` with:

```rust
//! NEAR AI Cloud hosted privacy-classifier backend for trace redaction.
//!
//! See docs/superpowers/specs/2026-05-19-near-ai-pii-redaction-design.md
//! for the contract this module implements.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::trace_contribution::{
    PrivacyFilterAdapter, PrivacyFilterConfigError, RedactionReport, SafePrivacyFilterRedaction,
    SafePrivacyFilterSummary, TraceContributionError, safe_privacy_filter_label,
    PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES,
};

pub const DEFAULT_BASE_URL: &str = "https://cloud-api.near.ai/v1";
pub const DEFAULT_MODEL: &str = "openai/privacy-filter";
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

#[derive(Clone)]
struct SecretApiKey(String);

impl std::fmt::Debug for SecretApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecretApiKey(***)")
    }
}

pub struct NearAiPrivacyFilterAdapter {
    client: reqwest::Client,
    base_url: String,
    model: String,
    api_key: SecretApiKey,
    max_input_bytes: usize,
}

impl NearAiPrivacyFilterAdapter {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: impl Into<String>,
        timeout: Duration,
        max_input_bytes: usize,
    ) -> Result<Self, PrivacyFilterConfigError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "<reqwest client>",
                reason: err.to_string(),
            })?;
        Ok(Self {
            client,
            base_url: base_url.into(),
            model: model.into(),
            api_key: SecretApiKey(api_key.into()),
            max_input_bytes,
        })
    }
}

pub fn build_from_env() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    let api_key = std::env::var("TRACE_NEAR_AI_PRIVACY_API_KEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or(PrivacyFilterConfigError::MissingEnv {
            backend: "near-ai",
            var: "TRACE_NEAR_AI_PRIVACY_API_KEY",
        })?;

    let base_url = std::env::var("TRACE_NEAR_AI_PRIVACY_BASE_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());

    let model = std::env::var("TRACE_NEAR_AI_PRIVACY_MODEL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    let timeout_ms = match std::env::var("TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS") {
        Ok(value) => value
            .trim()
            .parse::<u64>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS",
                reason: err.to_string(),
            })?,
        Err(_) => DEFAULT_TIMEOUT_MS,
    };

    let max_input_bytes = match std::env::var("TRACE_NEAR_AI_PRIVACY_MAX_INPUT_BYTES") {
        Ok(value) => value
            .trim()
            .parse::<usize>()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_NEAR_AI_PRIVACY_MAX_INPUT_BYTES",
                reason: err.to_string(),
            })?,
        Err(_) => PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES,
    };

    let adapter = NearAiPrivacyFilterAdapter::new(
        base_url,
        model,
        api_key,
        Duration::from_millis(timeout_ms),
        max_input_bytes,
    )?;
    Ok(Arc::new(adapter))
}

#[derive(Serialize)]
struct ClassifyRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct ClassifyResponse {
    data: Vec<ClassifyEntry>,
}

#[derive(Deserialize)]
struct ClassifyEntry {
    #[serde(default)]
    spans: Vec<ClassifySpan>,
}

#[derive(Deserialize, Clone)]
struct ClassifySpan {
    category: String,
    start: usize,
    end: usize,
    #[serde(default)]
    score: f64,
}

#[async_trait]
impl PrivacyFilterAdapter for NearAiPrivacyFilterAdapter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        if text.trim().is_empty() {
            return Ok(None);
        }
        if text.len() > self.max_input_bytes {
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "near-ai privacy classifier input exceeded limit: input_len={} max_input_bytes={}",
                    text.len(),
                    self.max_input_bytes
                ),
            });
        }

        let endpoint = format!("{}/privacy/classify", self.base_url.trim_end_matches('/'));
        let request_body = ClassifyRequest {
            model: &self.model,
            input: text,
        };
        let response = self
            .client
            .post(&endpoint)
            .bearer_auth(&self.api_key.0)
            .json(&request_body)
            .send()
            .await
            .map_err(|err| TraceContributionError::RedactionFailed {
                reason: format!("near-ai privacy classifier transport error: {}", err),
            })?;

        let status = response.status();
        if !status.is_success() {
            // Hash the body for audit; do not include it verbatim.
            let body_bytes = response.bytes().await.unwrap_or_default();
            let body_hash = format!(
                "sha256:{}",
                hex::encode(<sha2::Sha256 as sha2::Digest>::digest(&body_bytes))
            );
            return Err(TraceContributionError::RedactionFailed {
                reason: format!(
                    "near-ai privacy classifier returned non-2xx: status={} body_hash={} body_len={}",
                    status.as_u16(),
                    body_hash,
                    body_bytes.len()
                ),
            });
        }

        let parsed: ClassifyResponse =
            response
                .json()
                .await
                .map_err(|err| TraceContributionError::RedactionFailed {
                    reason: format!("near-ai privacy classifier response parse error: {}", err),
                })?;
        let entry = parsed
            .data
            .into_iter()
            .next()
            .ok_or(TraceContributionError::RedactionFailed {
                reason: "near-ai privacy classifier returned empty data array".to_string(),
            })?;

        apply_spans(text, &entry.spans)
    }
}

fn apply_spans(
    text: &str,
    spans: &[ClassifySpan],
) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
    let mut report = RedactionReport::default();
    let mut by_label = std::collections::BTreeMap::new();
    let span_count = spans.len() as u32;

    // Validate offsets and labels; populate summary book-keeping per
    // raw span (matches sidecar accounting).
    for span in spans {
        if span.start > span.end || span.end > text.len() {
            return Err(TraceContributionError::RedactionFailed {
                reason: "near-ai privacy classifier returned out-of-range span".to_string(),
            });
        }
        if !text.is_char_boundary(span.start) || !text.is_char_boundary(span.end) {
            return Err(TraceContributionError::RedactionFailed {
                reason: "near-ai privacy classifier returned non-utf8 span boundary".to_string(),
            });
        }
        let label = safe_privacy_filter_label(Some(&span.category), &mut report);
        *by_label.entry(label.clone()).or_insert(0u32) += 1;
        report.increment(format!("privacy_filter:{label}"));
        if label.eq_ignore_ascii_case("secret") {
            report.blocked_secret_detected = true;
        }
        if !report.pii_labels_present.contains(&label) {
            report.pii_labels_present.push(label);
        }
    }

    // Build redacted text. Collapse overlapping spans: sort by start,
    // pick widest end; on overlap pick the highest-score category.
    let mut sorted: Vec<ClassifySpan> = spans.to_vec();
    sorted.sort_by_key(|s| (s.start, std::cmp::Reverse(s.end)));

    let mut collapsed: Vec<ClassifySpan> = Vec::new();
    for span in sorted {
        match collapsed.last_mut() {
            Some(prev) if span.start < prev.end => {
                if span.end > prev.end {
                    prev.end = span.end;
                }
                if span.score > prev.score {
                    prev.category = span.category;
                    prev.score = span.score;
                }
            }
            _ => collapsed.push(span),
        }
    }

    let mut redacted_text = String::with_capacity(text.len());
    let mut cursor = 0usize;
    let mut dummy_report = RedactionReport::default();
    for span in &collapsed {
        redacted_text.push_str(&text[cursor..span.start]);
        let label = safe_privacy_filter_label(Some(&span.category), &mut dummy_report);
        redacted_text.push_str(&format!("[REDACTED:{label}]"));
        cursor = span.end;
    }
    redacted_text.push_str(&text[cursor..]);

    Ok(Some(SafePrivacyFilterRedaction {
        redacted_text,
        summary: SafePrivacyFilterSummary {
            schema_version: 1,
            output_mode: "redacted_text_only".to_string(),
            span_count,
            by_label,
            decoded_mismatch: false,
        },
        report,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(category: &str, start: usize, end: usize, score: f64) -> ClassifySpan {
        ClassifySpan {
            category: category.into(),
            start,
            end,
            score,
        }
    }

    #[test]
    fn empty_input_short_circuits() {
        // Cannot call redact_text without a client; test apply_spans
        // covers the inner behavior. Empty-text short-circuit is in
        // redact_text proper, exercised by integration tests in Task 8.
    }

    #[test]
    fn replaces_single_span() {
        let text = "email me at alice@example.com please";
        let spans = vec![span("private_email", 12, 29, 0.99)];
        let result = apply_spans(text, &spans).unwrap().unwrap();
        assert_eq!(result.redacted_text, "email me at [REDACTED:private_email] please");
        assert_eq!(result.summary.span_count, 1);
        assert_eq!(result.summary.by_label.get("private_email"), Some(&1));
    }

    #[test]
    fn collapses_overlapping_spans_keeps_highest_score() {
        let text = "abcdefghij";
        let spans = vec![
            span("private_email", 1, 5, 0.4),
            span("private_phone", 3, 7, 0.9),
        ];
        let result = apply_spans(text, &spans).unwrap().unwrap();
        assert_eq!(result.redacted_text, "a[REDACTED:private_phone]hij");
        // span_count is raw, even though only one collapsed redaction.
        assert_eq!(result.summary.span_count, 2);
    }

    #[test]
    fn rejects_non_char_boundary() {
        let text = "héllo";
        // 'é' starts at byte 1, ends at byte 3 (UTF-8 two bytes).
        // Splitting at byte 2 is mid-codepoint.
        let spans = vec![span("private_name", 1, 2, 0.9)];
        let err = apply_spans(text, &spans).unwrap_err();
        assert!(err.to_string().contains("non-utf8 span boundary"));
    }

    #[test]
    fn rejects_out_of_range_span() {
        let text = "short";
        let spans = vec![span("private_name", 0, 9999, 0.9)];
        let err = apply_spans(text, &spans).unwrap_err();
        assert!(err.to_string().contains("out-of-range"));
    }

    #[test]
    fn unknown_category_maps_to_unknown_with_warning() {
        let text = "secret-text";
        let spans = vec![span("brand_new_category", 0, 6, 0.5)];
        let result = apply_spans(text, &spans).unwrap().unwrap();
        assert_eq!(result.redacted_text, "[REDACTED:unknown]-text");
        assert!(result
            .report
            .warnings
            .iter()
            .any(|w| w.to_lowercase().contains("unsupported")));
    }

    #[test]
    fn known_categories_land_in_allowlist() {
        let table = [
            "private_email",
            "private_phone",
            "account_number",
            "private_address",
            "private_name",
            "secret",
        ];
        for raw in table {
            let mut r = RedactionReport::default();
            let label = safe_privacy_filter_label(Some(raw), &mut r);
            assert_eq!(label, raw, "{raw} should pass through allow-list");
        }
    }

    #[test]
    fn debug_does_not_leak_api_key() {
        let secret = SecretApiKey("super-secret-token".into());
        let debug = format!("{secret:?}");
        assert!(!debug.contains("super-secret-token"));
        assert!(debug.contains("***"));
    }
}
```

Visibility bumps were already made in Step 1.

- [ ] **Step 4: Run feature-on build + unit tests**

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol --features near-ai-privacy-filter`
Expected: PASS.

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter privacy_filter_near_ai::`
Expected: all tests PASS.

- [ ] **Step 5: Run default-feature build**

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol`
Expected: PASS (module is feature-gated, dispatch returns `FeatureDisabled`).

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-protocol/src/privacy_filter_near_ai.rs crates/trace-commons-protocol/src/lib.rs crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Implement NearAiPrivacyFilterAdapter (feature: near-ai-privacy-filter)" --no-verify
```

---

## Task 8: HTTP integration tests with wiremock

**Files:**
- Create: `crates/trace-commons-protocol/tests/privacy_filter_near_ai_http.rs`

- [ ] **Step 1: Write integration test file**

```rust
#![cfg(feature = "near-ai-privacy-filter")]

use std::time::Duration;

use serde_json::json;
use trace_commons_protocol::privacy_filter_near_ai::NearAiPrivacyFilterAdapter;
use trace_commons_protocol::trace_contribution::{run_privacy_filter_canary, PrivacyFilterAdapter};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn adapter(base_url: String) -> NearAiPrivacyFilterAdapter {
    NearAiPrivacyFilterAdapter::new(
        base_url,
        "openai/privacy-filter",
        "test-api-key-do-not-leak",
        Duration::from_secs(5),
        1_000_000,
    )
    .expect("adapter builds")
}

#[tokio::test]
async fn classifies_and_redacts_single_span() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .and(header("authorization", "Bearer test-api-key-do-not-leak"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{
                "spans": [
                    {"category": "private_email", "start": 12, "end": 29, "score": 0.99, "text": "alice@example.com"}
                ]
            }]
        })))
        .mount(&server)
        .await;

    let adapter = adapter(server.uri());
    let result = adapter
        .redact_text("email me at alice@example.com please")
        .await
        .expect("call succeeds")
        .expect("non-empty redaction");
    assert_eq!(result.redacted_text, "email me at [REDACTED:private_email] please");
    assert_eq!(result.summary.span_count, 1);
}

#[tokio::test]
async fn surfaces_http_5xx_as_redaction_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(500).set_body_string("oh no"))
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("hello world")
        .await
        .expect_err("5xx must error");
    let msg = err.to_string();
    assert!(msg.contains("status=500"));
    assert!(msg.contains("body_hash=sha256:"));
    assert!(!msg.contains("oh no"));
}

#[tokio::test]
async fn timeout_surfaces_as_redaction_failed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;

    let adapter = NearAiPrivacyFilterAdapter::new(
        server.uri(),
        "openai/privacy-filter",
        "test-api-key-do-not-leak",
        Duration::from_millis(200),
        1_000_000,
    )
    .expect("adapter builds");

    let err = adapter
        .redact_text("hello world")
        .await
        .expect_err("timeout must error");
    let msg = err.to_string();
    assert!(msg.to_lowercase().contains("transport"));
    assert!(!msg.contains("test-api-key-do-not-leak"));
}

#[tokio::test]
async fn error_strings_do_not_leak_api_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let err = adapter(server.uri())
        .redact_text("x")
        .await
        .expect_err("401 errors");
    assert!(!err.to_string().contains("test-api-key-do-not-leak"));
}

#[tokio::test]
async fn canary_run_against_mock_returns_healthy() {
    let server = MockServer::start().await;
    // Canary text is three synthetic values joined by spaces; mock
    // returns three spans covering each.
    // Canary text is the three synthetic values joined by single
    // spaces: "<a> <b> <c>". Verified byte offsets:
    //   a = "trace-canary.person@example.invalid"  (35 bytes) → 0..35
    //   space at 35..36
    //   b = "tc_canary_secret_0123456789abcdef"     (33 bytes) → 36..69
    //   space at 69..70
    //   c = "/tmp/trace_canary_private/path.txt"   (34 bytes) → 70..104
    // Total text length: 104 bytes.
    Mock::given(method("POST"))
        .and(path("/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{
                "spans": [
                    {"category": "private_email",   "start":  0, "end":  35, "score": 0.99, "text": "trace-canary.person@example.invalid"},
                    {"category": "secret",          "start": 36, "end":  69, "score": 0.99, "text": "tc_canary_secret_0123456789abcdef"},
                    {"category": "private_address", "start": 70, "end": 104, "score": 0.99, "text": "/tmp/trace_canary_private/path.txt"}
                ]
            }]
        })))
        .mount(&server)
        .await;

    let report = run_privacy_filter_canary(&adapter(server.uri()))
        .await
        .expect("canary runs");
    assert!(report.healthy, "canary should be healthy: {:?}", report.failures);
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter --test privacy_filter_near_ai_http`
Expected: all 5 tests PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/trace-commons-protocol/tests/privacy_filter_near_ai_http.rs
git commit -m "Add wiremock integration tests for NEAR AI privacy filter" --no-verify
```

---

## Task 9: Update production call sites for fallible redactor construction

**Files (verified by grep, non-test only):**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:47016`
- Modify: `crates/trace-commons-server/src/bin/pilot_bootstrap/submitter.rs:213`
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs:2276` (the
  internal helper around the `build_envelope` flow — read the
  surrounding 40 lines first; if it's only reachable from tests,
  leaving `default()` is fine, otherwise migrate.)

The spec requires fail-closed: production binaries that misconfigure
`TRACE_PRIVACY_FILTER_BACKEND` MUST refuse to start, not panic and
not silently disable redaction.

- [ ] **Step 1: Migrate `trace-commons-ingest.rs:47016`**

Read the surrounding function to find the active `Result` type or
`anyhow::Error`. Replace `DeterministicTraceRedactor::default()` with:

```rust
DeterministicTraceRedactor::try_default()
    .map_err(|err| anyhow::anyhow!("privacy filter config invalid: {err}"))?
```

If the enclosing function does not return `Result`, walk up the call
chain until it does (this is `main` or a setup helper called from
`main`) and propagate via `?`. Never collapse the error to
`.unwrap()`.

- [ ] **Step 2: Migrate `pilot_bootstrap/submitter.rs:213`**

Same shape. The pilot-bootstrap binary's `main` returns `Result`;
propagate via `?` and log via the binary's existing observability
hook.

- [ ] **Step 3: Audit and migrate `trace_contribution.rs:2276`**

Read 60 lines around it. If it's inside a `pub` function used by
production, migrate. If it's a `#[cfg(test)]`-only helper or only
reachable from tests, leave it alone — the `Default` impl's panic
contract is acceptable for tests.

- [ ] **Step 4: Re-grep to confirm no production `::default()` remains**

Run:
```bash
grep -rn 'DeterministicTraceRedactor::default' --include='*.rs' | grep -v 'test'
```
Expected: only the `impl Default` line itself (or empty).

- [ ] **Step 5: Run full workspace check and tests**

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins`
Expected: PASS.

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --no-run`
Expected: PASS.

Run: `cargo test -p trace-commons-server`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/
git commit -m "Propagate fallible privacy-filter config in trace-commons-server binaries" --no-verify
```

---

## Task 10: Clippy and CI parity check

**Files:**
- (verification only)

- [ ] **Step 1: Run the project clippy invocation**

Run:

```bash
cargo clippy -p trace-commons-protocol --all-targets --features near-ai-privacy-filter -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

Expected: no warnings.

Run:

```bash
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

Expected: no warnings.

- [ ] **Step 2: Run CI-equivalent build and test**

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol --features near-ai-privacy-filter`
Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins`
Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-protocol --features near-ai-privacy-filter --no-run`
Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --no-run`
Expected: all PASS.

- [ ] **Step 3: Verify pilot-bootstrap smoke still passes**

Run: `bash scripts/operator/pilot-bootstrap-smoke.sh`
Expected: PASS.

- [ ] **Step 4: Commit any incidental clippy fixes**

```bash
git status
# if changes exist:
git add -A
git commit -m "Apply clippy lints surfaced by near-ai-privacy-filter build" --no-verify
```

---

## Task 11: Operator runbook update

**Files:**
- Modify: `docs/operator/pilot-gcp-deployment.md`

- [ ] **Step 1: Document the new env vars**

Add a section "Privacy filter backend" with:

- `TRACE_PRIVACY_FILTER_BACKEND` — `sidecar` | `near-ai` | unset.
- `near-ai` vars: `TRACE_NEAR_AI_PRIVACY_API_KEY` (required),
  `TRACE_NEAR_AI_PRIVACY_BASE_URL`, `TRACE_NEAR_AI_PRIVACY_MODEL`,
  `TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS`, `TRACE_NEAR_AI_PRIVACY_MAX_INPUT_BYTES`.
- Sidecar vars renamed: `TRACE_PRIVACY_FILTER_COMMAND` (was
  `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND`) — note back-compat shim
  warns at startup.
- Pilot runs with `--features near-ai-privacy-filter` and
  `TRACE_PRIVACY_FILTER_BACKEND=near-ai`.
- First-traffic canary must pass before admitting real traces.

- [ ] **Step 2: Commit**

```bash
git add docs/operator/pilot-gcp-deployment.md
git commit -m "Document NEAR AI privacy filter backend env vars in pilot runbook" --no-verify
```

---

## Task 12: Final verification + PR prep

- [ ] **Step 1: Push branch**

```bash
git push -u origin racer-palo-verde
```

- [ ] **Step 2: Surface dep additions in PR description**

When opening the PR, explicitly call out:
- New direct dep on `trace-commons-protocol`: `reqwest 0.12` (optional, feature `near-ai-privacy-filter`).
- New dev-dep: `wiremock 0.6`.

Both pre-approved by the user during spec review (2026-05-19).
