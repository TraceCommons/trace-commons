# Self-hosted Privacy Filter Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `self-hosted` privacy-filter backend that classifies PII against
a locally-served `openai/privacy-filter`, leaving the `near-ai` backend intact
and selectable.

**Architecture:** A new sibling module `privacy_filter_self_hosted.rs` in the
protocol crate holds an HTTP adapter for a loopback service that reproduces
NEAR AI's `POST {base}/privacy/classify` wire shape. The span-decoding logic
(`apply_spans`) is shared, not duplicated. A Python FastAPI shim wrapping the
official `opf` package serves that endpoint on `127.0.0.1`. Because the local
model has a real 128k context, the adapter issues one request per field with no
windowing, no tokenizer, and no window cache.

**Tech Stack:** Rust (reqwest, wiremock for tests), Python 3.11+
(FastAPI/uvicorn, `opf`), systemd, GCE.

**Spec:** `docs/superpowers/specs/2026-08-29-self-hosted-privacy-filter-design.md`

## Global Constraints

- **Licensing.** `trace-commons-protocol` is `MIT OR Apache-2.0` and ships
  inside proprietary harnesses. Do **not** add any dependency on
  `trace-commons-server`, `-gate-api`, or `-gate-enclave`. No AGPL headers on
  files in this crate. `crates/trace-commons-server/tests/license_boundary.rs`
  enforces this; never edit its expected sets.
- **Standalone permissive build.** CI runs `cargo check (permissive crates,
  standalone)` building this crate alone with `--no-default-features`. Every
  new item must be behind a feature gate and must not break that build.
- **No new third-party crate.** `reqwest` and `tracing` are already optional
  dependencies of the protocol crate. Adding anything else needs explicit
  approval per the dependency policy.
- **Hash-only logging.** No raw URLs, tokens, trace bodies, contributor
  identity, or classified text in any log line or stored row. Fingerprint with
  `classify_input_diagnostics`-style hashing.
- **Fail-closed.** A configured-but-broken backend refuses the path; it never
  silently degrades to deterministic-regex-only redaction.
- **No emojis** in commits, PRs, code, or comments. Short imperative commit
  subjects, no `feat:`/`fix:` prefixes.
- **Verification before any green claim:**
  `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`,
  `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`,
  clippy with the repo allow-list, and `cargo fmt --all`.

## File Structure

| File | Responsibility |
|---|---|
| `crates/trace-commons-protocol/src/privacy_filter_spans.rs` | **Create.** Shared span types and decoding (`ClassifySpan`, `apply_spans`), moved out of the NEAR module so both backends share one implementation. |
| `crates/trace-commons-protocol/src/privacy_filter_near_ai.rs` | **Modify.** Re-export the moved items; delete the local copies. No behaviour change. |
| `crates/trace-commons-protocol/src/privacy_filter_self_hosted.rs` | **Create.** `SelfHostedPrivacyFilterAdapter` + `build_from_env`. |
| `crates/trace-commons-protocol/src/lib.rs` | **Modify.** Declare the two new modules behind feature gates. |
| `crates/trace-commons-protocol/src/trace_contribution.rs` | **Modify.** `PrivacyFilterBackendTag::SelfHosted`, the `"self-hosted"` dispatch arm, `build_self_hosted_adapter`. |
| `crates/trace-commons-protocol/Cargo.toml` | **Modify.** New `self-hosted-privacy-filter` feature. |
| `crates/trace-commons-protocol/tests/privacy_filter_self_hosted_http.rs` | **Create.** wiremock tests for the adapter. |
| `deploy/pilot-gcp/privacy-filter/` | **Create.** Shim (`app.py`), pinned `requirements.txt`, systemd unit, weight-staging script. |
| `deploy/pilot-gcp/privacy-filter/test_app.py` | **Create.** Shim tests, including the offset-convention golden vectors. |
| `deploy/pilot-gcp/ingest.env.template` | **Modify.** Document the new env vars. |
| `docs/operator/env-reference.md`, `docs/operator/deployment.md` | **Modify.** Backend table + rollout. |

---

### Task 1: Extract shared span decoding

Pure refactor. No behaviour change. It exists so the security-critical decoder
is single-sourced before a second caller appears.

**Files:**
- Create: `crates/trace-commons-protocol/src/privacy_filter_spans.rs`
- Modify: `crates/trace-commons-protocol/src/privacy_filter_near_ai.rs`
- Modify: `crates/trace-commons-protocol/src/lib.rs`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub(crate) struct ClassifySpan { pub category: String, pub start: usize, pub end: usize, pub score: f64 }` — `Deserialize, Clone`. `start`/`end` are **codepoint** offsets into the classified text.
  - `pub(crate) fn apply_spans(text: &str, spans: Vec<ClassifySpan>) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError>`

- [ ] **Step 1: Confirm the existing tests pass before touching anything**

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter privacy_filter`
Expected: PASS. Record the count — it is the baseline this refactor must preserve.

- [ ] **Step 2: Create the new module by moving code verbatim**

Create `privacy_filter_spans.rs`. Move `ClassifySpan` (currently
`privacy_filter_near_ai.rs:186`-ish) and `apply_spans` (line 769) **unmodified**
— same body, same comments. Add the gate at the top of the file:

```rust
//! Span types and decoding shared by every classify-shaped privacy backend.
//!
//! Offsets from a classifier are CODEPOINT offsets, not byte offsets. The
//! conversion to byte indices happens here, once, so no backend can get it
//! wrong independently.

use crate::trace_contribution::{
    RedactionReport, SafePrivacyFilterRedaction, SafePrivacyFilterSummary,
    TraceContributionError, safe_privacy_filter_label,
};
```

Move the span-related tests too: `replaces_single_span`,
`maps_codepoint_offsets_over_multibyte_text`,
`collapses_overlapping_spans_keeps_highest_score`,
`redacts_multibyte_codepoint_span_without_splitting`,
`rejects_out_of_range_span`, `rejects_out_of_range_span_over_multibyte_text`,
`unknown_category_maps_to_unknown_with_warning`,
`known_categories_land_in_allowlist`, and the `span()` helper.

- [ ] **Step 3: Declare the module**

In `lib.rs`, beside the existing `privacy_filter_near_ai` declaration:

```rust
#[cfg(any(feature = "near-ai-privacy-filter", feature = "self-hosted-privacy-filter"))]
pub(crate) mod privacy_filter_spans;
```

- [ ] **Step 4: Re-point the NEAR module at the shared code**

In `privacy_filter_near_ai.rs`, delete the moved definitions and add:

```rust
use crate::privacy_filter_spans::{ClassifySpan, apply_spans};
```

`apply_windowed_spans` (line 751) stays in the NEAR module — window shifting is
specific to a backend that windows.

- [ ] **Step 5: Verify the refactor changed nothing**

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter privacy_filter`
Expected: PASS, with the same test count as Step 1. A changed count means
something was dropped in the move.

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-protocol --no-default-features`
Expected: PASS. This is the permissive-standalone configuration CI gates.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-protocol/src/privacy_filter_spans.rs \
        crates/trace-commons-protocol/src/privacy_filter_near_ai.rs \
        crates/trace-commons-protocol/src/lib.rs
git commit -m "Move classify span decoding to a shared module

A second classify-shaped backend is about to need it, and the codepoint
to byte conversion is the one part that must not exist twice."
```

---

### Task 2: The self-hosted adapter

**Files:**
- Create: `crates/trace-commons-protocol/src/privacy_filter_self_hosted.rs`
- Modify: `crates/trace-commons-protocol/Cargo.toml`
- Modify: `crates/trace-commons-protocol/src/lib.rs`

**Interfaces:**
- Consumes: `ClassifySpan`, `apply_spans` from Task 1.
- Produces:
  - `pub struct SelfHostedPrivacyFilterAdapter`
  - `pub fn SelfHostedPrivacyFilterAdapter::new(base_url: impl Into<String>, model: impl Into<String>, timeout: Duration, max_input_bytes: usize) -> Result<Self, PrivacyFilterConfigError>` — note: **no api_key parameter**.
  - `pub fn build_from_env() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError>`
  - `pub const DEFAULT_MODEL: &str = "openai/privacy-filter"`
  - `pub const DEFAULT_TIMEOUT_MS: u64 = 30_000`

- [ ] **Step 1: Add the cargo feature**

In `crates/trace-commons-protocol/Cargo.toml`:

```toml
self-hosted-privacy-filter = ["dep:reqwest", "dep:tracing"]
```

Deliberately **not** `dep:tiktoken-rs` or `dep:futures`: this backend does not
window, so it needs neither a tokenizer nor concurrent streams.

- [ ] **Step 2: Write the failing test**

Create `crates/trace-commons-protocol/tests/privacy_filter_self_hosted_http.rs`:

```rust
#![cfg(feature = "self-hosted-privacy-filter")]

use std::time::Duration;

use serde_json::json;
use trace_commons_protocol::privacy_filter_self_hosted::SelfHostedPrivacyFilterAdapter;
use trace_commons_protocol::trace_contribution::PrivacyFilterAdapter;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn adapter(base_url: String) -> SelfHostedPrivacyFilterAdapter {
    SelfHostedPrivacyFilterAdapter::new(
        base_url,
        "openai/privacy-filter",
        Duration::from_secs(5),
        1_000_000,
    )
    .expect("adapter builds")
}

/// A field far larger than NEAR's 2,000-token window budget must go up in ONE
/// request. This is the whole point of self-hosting: the local model has a
/// real 128k context, so the window-and-stitch path is gone.
#[tokio::test]
async fn a_large_field_is_classified_in_a_single_request() {
    let server = MockServer::start().await;
    let filler = "The quick brown fox jumps over the lazy dog. ".repeat(4_000);
    let text = format!("{filler} contact bob@example.com now");
    let start = text.chars().count() - "contact bob@example.com now".chars().count() + 8;

    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": [{"spans": [{
                "category": "private_email",
                "start": start,
                "end": start + "bob@example.com".chars().count(),
                "score": 0.99
            }]}]
        })))
        .expect(1) // exactly one request: no windowing
        .mount(&server)
        .await;

    let out = adapter(format!("{}/v1", server.uri()))
        .redact_text(&text)
        .await
        .expect("classification succeeds")
        .expect("a redaction was produced");

    assert!(out.redacted_text.contains("[REDACTED:private_email]"));
    assert!(!out.redacted_text.contains("bob@example.com"));
}

/// Loopback carries no bearer token. A self-hosted shim must not be sent one.
#[tokio::test]
async fn no_authorization_header_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .and(|req: &wiremock::Request| req.headers.get("authorization").is_none())
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"spans": []}]})))
        .expect(1)
        .mount(&server)
        .await;

    adapter(format!("{}/v1", server.uri()))
        .redact_text("nothing sensitive here")
        .await
        .expect("classification succeeds");
}

/// Fail closed: a 5xx must surface as an error, never as "no PII found".
#[tokio::test]
async fn a_server_error_fails_closed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/privacy/classify"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let err = adapter(format!("{}/v1", server.uri()))
        .redact_text("contact bob@example.com")
        .await
        .expect_err("a 5xx must not be reported as a clean field");
    assert!(format!("{err}").contains("self-hosted"));
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-protocol --features self-hosted-privacy-filter --test privacy_filter_self_hosted_http`
Expected: FAIL to compile — `privacy_filter_self_hosted` does not exist.

- [ ] **Step 4: Implement the adapter**

Create `privacy_filter_self_hosted.rs`. Mirror the NEAR module's structure but
strip everything that exists only to cope with a WAN upstream — no window
cache, no `chunk_token_ranges`, no `MAX_CLASSIFY_ATTEMPTS` retry loop, no
bearer auth.

```rust
//! Loopback privacy-classifier backend serving `openai/privacy-filter`.
//!
//! Wire-compatible with the NEAR AI hosted endpoint by design, so both
//! backends share `apply_spans` and a shadow comparison is a direct diff.
//! Unlike that backend this one talks to a local process with a real 128k
//! context, so a field is classified in a single request.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;

use crate::privacy_filter_spans::{ClassifySpan, apply_spans};
use crate::trace_contribution::{
    PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES, PrivacyFilterAdapter,
    PrivacyFilterConfigError, SafePrivacyFilterRedaction, TraceContributionError,
};

pub const DEFAULT_MODEL: &str = "openai/privacy-filter";
/// Larger than the NEAR default: one request now carries a whole field
/// rather than a 2,000-token window, and CPU inference is slower per call
/// while needing far fewer calls.
pub const DEFAULT_TIMEOUT_MS: u64 = 30_000;

pub struct SelfHostedPrivacyFilterAdapter {
    client: reqwest::Client,
    base_url: String,
    model: String,
    max_input_bytes: usize,
}

#[derive(Serialize)]
struct ClassifyRequest<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(serde::Deserialize)]
struct ClassifyResponse {
    data: Vec<ClassifyEntry>,
}

#[derive(serde::Deserialize)]
struct ClassifyEntry {
    #[serde(default)]
    spans: Vec<ClassifySpan>,
}

impl SelfHostedPrivacyFilterAdapter {
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        timeout: Duration,
        max_input_bytes: usize,
    ) -> Result<Self, PrivacyFilterConfigError> {
        let base_url = base_url.into();
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|err| PrivacyFilterConfigError::InvalidEnv {
                var: "<reqwest client>",
                reason: err.to_string(),
            })?;
        Ok(Self { client, base_url, model: model.into(), max_input_bytes })
    }
}

#[async_trait]
impl PrivacyFilterAdapter for SelfHostedPrivacyFilterAdapter {
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
                    "self-hosted privacy classifier input exceeded limit: input_len={} max_input_bytes={}",
                    text.len(),
                    self.max_input_bytes
                ),
            });
        }

        let endpoint = format!("{}/privacy/classify", self.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(&endpoint)
            .json(&ClassifyRequest { model: &self.model, input: text })
            .send()
            .await
            .map_err(|err| TraceContributionError::TransientRedactionFailed {
                reason: format!("self-hosted privacy classifier transport error: {err}"),
            })?;

        let status = response.status();
        if !status.is_success() {
            let reason = format!("self-hosted privacy classifier returned {status}");
            return Err(if status.is_server_error() {
                TraceContributionError::TransientRedactionFailed { reason }
            } else {
                TraceContributionError::RedactionFailed { reason }
            });
        }

        let parsed: ClassifyResponse = response.json().await.map_err(|err| {
            TraceContributionError::RedactionFailed {
                reason: format!("self-hosted privacy classifier response parse error: {err}"),
            }
        })?;

        // Fail closed on a shape we do not understand rather than treating it
        // as "no PII found".
        let entry = parsed.data.into_iter().next().ok_or_else(|| {
            TraceContributionError::RedactionFailed {
                reason: "self-hosted privacy classifier returned an empty data array".to_string(),
            }
        })?;

        apply_spans(text, entry.spans)
    }
}

pub fn build_from_env() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    let base_url = std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .ok_or(PrivacyFilterConfigError::MissingEnv {
            backend: "self-hosted",
            var: "TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL",
        })?;

    let model = std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_MODEL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());

    let timeout_ms = match std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_TIMEOUT_MS") {
        Ok(value) => value.trim().parse::<u64>().map_err(|err| {
            PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_SELF_HOSTED_TIMEOUT_MS",
                reason: err.to_string(),
            }
        })?,
        Err(_) => DEFAULT_TIMEOUT_MS,
    };

    let max_input_bytes = match std::env::var("TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_INPUT_BYTES") {
        Ok(value) => value.trim().parse::<usize>().map_err(|err| {
            PrivacyFilterConfigError::InvalidEnv {
                var: "TRACE_PRIVACY_FILTER_SELF_HOSTED_MAX_INPUT_BYTES",
                reason: err.to_string(),
            }
        })?,
        Err(_) => PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES,
    };

    Ok(Arc::new(SelfHostedPrivacyFilterAdapter::new(
        base_url,
        model,
        Duration::from_millis(timeout_ms),
        max_input_bytes,
    )?))
}
```

Declare it in `lib.rs`:

```rust
#[cfg(feature = "self-hosted-privacy-filter")]
pub mod privacy_filter_self_hosted;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-protocol --features self-hosted-privacy-filter --test privacy_filter_self_hosted_http`
Expected: PASS, 3 tests.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-protocol/
git commit -m "Add a loopback privacy-classifier adapter

Wire-compatible with the hosted endpoint so span decoding is shared, but
without the window budget, cache, retry loop and bearer auth that exist
only to cope with a WAN upstream."
```

---

### Task 3: Wire the backend into selection

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (`PrivacyFilterBackendTag` at :39, `privacy_filter_adapter_from_env` at :2642)

**Interfaces:**
- Consumes: `privacy_filter_self_hosted::build_from_env` from Task 2.
- Produces: `PrivacyFilterBackendTag::SelfHosted`, whose `label()` is `"self_hosted"`.

- [ ] **Step 1: Write the failing tests**

Add to the existing backend-selection test module in `trace_contribution.rs`,
following the shape of the tests at :7180-7230. These mutate process env, so
they must use whatever serialisation guard the neighbouring tests already use —
copy it rather than inventing one.

```rust
#[test]
fn self_hosted_backend_resolves_without_an_api_key() {
    let _guard = env_lock();
    unsafe {
        std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "self-hosted");
        std::env::set_var(
            "TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL",
            "http://127.0.0.1:8471/v1",
        );
        std::env::remove_var("TRACE_NEAR_AI_PRIVACY_API_KEY");
    }
    let (_adapter, tag) = privacy_filter_adapter_from_env()
        .expect("self-hosted backend resolves")
        .expect("a backend is configured");
    assert_eq!(tag.label(), "self_hosted");
    unsafe {
        std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
        std::env::remove_var("TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL");
    }
}

#[test]
fn self_hosted_backend_without_a_base_url_is_refused() {
    let _guard = env_lock();
    unsafe {
        std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "self-hosted");
        std::env::remove_var("TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL");
    }
    let err = privacy_filter_adapter_from_env()
        .expect_err("a backend with no endpoint must refuse, not default");
    assert!(format!("{err}").contains("TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL"));
    unsafe {
        std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p trace-commons-protocol --features self-hosted-privacy-filter self_hosted_backend`
Expected: FAIL — `UnknownBackend { value: "self-hosted" }`.

- [ ] **Step 3: Add the tag variant**

In `trace_contribution.rs` at the enum (:39) and its `label()` (:48):

```rust
pub enum PrivacyFilterBackendTag {
    None,
    Sidecar,
    NearAi,
    SelfHosted,
}
```

```rust
PrivacyFilterBackendTag::SelfHosted => "self_hosted",
```

Adding a variant will break any exhaustive `match` on this enum. Compile and
fix each one; do not add a `_ =>` arm, since the compiler catching these is the
point.

- [ ] **Step 4: Add the builder and dispatch arm**

Beside `build_near_ai_adapter`:

```rust
#[cfg(feature = "self-hosted-privacy-filter")]
fn build_self_hosted_adapter() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    crate::privacy_filter_self_hosted::build_from_env()
}

#[cfg(not(feature = "self-hosted-privacy-filter"))]
fn build_self_hosted_adapter() -> Result<Arc<dyn PrivacyFilterAdapter>, PrivacyFilterConfigError> {
    Err(PrivacyFilterConfigError::FeatureDisabled {
        backend: "self-hosted",
        feature: "self-hosted-privacy-filter",
    })
}
```

And in `privacy_filter_adapter_from_env`:

```rust
"self-hosted" => build_self_hosted_adapter()
    .map(|adapter| Some((adapter, PrivacyFilterBackendTag::SelfHosted))),
```

- [ ] **Step 5: Run to verify they pass**

Run: `cargo test -p trace-commons-protocol --features self-hosted-privacy-filter self_hosted_backend`
Expected: PASS, 2 tests.

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter privacy_filter`
Expected: PASS. The near-ai path must be unchanged.

- [ ] **Step 6: Enable the feature for the server**

In `crates/trace-commons-server/Cargo.toml:54`:

```toml
trace-commons-protocol = { path = "../trace-commons-protocol", features = ["near-ai-privacy-filter", "self-hosted-privacy-filter"] }
```

Leave `crates/trace-commons-contributor/Cargo.toml:10` alone — the contributor
does not talk to a server-side loopback shim.

- [ ] **Step 7: Full verification**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-protocol --no-default-features
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test license_boundary
```
Expected: all PASS.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add crates/
git commit -m "Select the self-hosted privacy filter backend by env

TRACE_PRIVACY_FILTER_BACKEND=self-hosted resolves the loopback adapter and
reports self_hosted in boot logs, health and the canary, so the backend that
answered is never in doubt."
```

---

### Task 4: The serving shim

**Files:**
- Create: `deploy/pilot-gcp/privacy-filter/app.py`
- Create: `deploy/pilot-gcp/privacy-filter/requirements.txt`
- Create: `deploy/pilot-gcp/privacy-filter/test_app.py`

**Interfaces:**
- Produces: `POST /v1/privacy/classify` taking `{"model": str, "input": str}` and returning `{"data": [{"spans": [{"category": str, "start": int, "end": int, "score": float}]}]}` with **codepoint** offsets, plus `GET /healthz` returning `{"status": "ok", "model": str}` only once weights are loaded.

**This task carries the single highest-risk detail in the plan.** `apply_spans`
interprets `start`/`end` as codepoint offsets. If `opf` returns byte offsets or
token indices and the shim passes them through, redaction destroys the wrong
text and leaves the PII in place while reporting success. Step 1 establishes
the convention empirically; nothing else may be written until it is known.

- [ ] **Step 1: Determine the offset convention empirically**

Run the model once on deliberately multi-byte input and inspect raw output:

```bash
python -c "
from opf import PrivacyFilter
pf = PrivacyFilter.from_pretrained('openai/privacy-filter', device='cpu')
text = 'Ping 大三 about bob@example.com today'
print('bytes:', len(text.encode()), 'codepoints:', len(text))
print(pf.detect(text))
"
```

The email begins at codepoint 16 and byte 20. Whichever number appears in the
raw output identifies the convention. **Write the observed output into a
comment at the top of `app.py`** — the next person must not have to rediscover
it. If the API differs from `pf.detect`, consult the installed package rather
than guessing.

- [ ] **Step 2: Write the failing test**

Create `deploy/pilot-gcp/privacy-filter/test_app.py`. The offset test is
independent of the model: it asserts the shim's own contract with a stubbed
detector, so it stays fast and deterministic.

```python
"""Contract tests for the privacy-filter shim.

The offset test is the important one. `apply_spans` on the Rust side treats
start/end as CODEPOINT offsets. If this shim ever emits byte offsets, redaction
lands on the wrong characters: the PII survives and unrelated text is
destroyed, while the call reports success.
"""

from fastapi.testclient import TestClient

import app as shim

# 'Ping 大三 about bob@example.com today'
#  codepoint 16 == byte 20, because the two CJK characters are 3 bytes each.
TEXT = "Ping 大三 about bob@example.com today"
EMAIL_CODEPOINT_START = TEXT.index("bob@example.com")


def test_spans_are_codepoint_offsets_not_byte_offsets(monkeypatch):
    monkeypatch.setattr(
        shim,
        "detect_spans",
        lambda text: [("private_email", EMAIL_CODEPOINT_START,
                       EMAIL_CODEPOINT_START + len("bob@example.com"), 0.99)],
    )
    client = TestClient(shim.app)
    body = client.post(
        "/v1/privacy/classify",
        json={"model": "openai/privacy-filter", "input": TEXT},
    ).json()

    span = body["data"][0]["spans"][0]
    assert span["start"] == 16, "codepoint offset expected"
    assert span["start"] != 20, "byte offset leaked into the response"
    # The offsets must actually select the email when used as codepoint indices.
    assert TEXT[span["start"]:span["end"]] == "bob@example.com"


def test_empty_input_returns_an_empty_span_list_not_an_error():
    client = TestClient(shim.app)
    body = client.post(
        "/v1/privacy/classify", json={"model": "openai/privacy-filter", "input": ""}
    ).json()
    assert body == {"data": [{"spans": []}]}


def test_healthz_reports_the_loaded_model():
    client = TestClient(shim.app)
    body = client.get("/healthz").json()
    assert body["status"] == "ok"
```

- [ ] **Step 3: Run to verify it fails**

Run: `cd deploy/pilot-gcp/privacy-filter && python -m pytest test_app.py -v`
Expected: FAIL — no module named `app`.

- [ ] **Step 4: Write the shim**

Create `app.py`. Convert to codepoint offsets explicitly if and only if Step 1
showed the model returns bytes; if it already returns codepoints, pass through
and say so in the comment.

```python
"""Loopback privacy-classifier serving openai/privacy-filter.

Reproduces NEAR AI Cloud's POST /v1/privacy/classify wire shape so the Rust
side shares one span decoder across both backends.

OFFSET CONVENTION: spans are CODEPOINT offsets. See test_app.py for why that
matters. Raw model output observed in Task 4 Step 1:
    <paste the observed output here>
"""

import os

from fastapi import FastAPI
from pydantic import BaseModel

MODEL_ID = os.environ.get("PRIVACY_FILTER_MODEL", "openai/privacy-filter")
DEVICE = os.environ.get("PRIVACY_FILTER_DEVICE", "cpu")

app = FastAPI()
_model = None


def _load():
    global _model
    if _model is None:
        from opf import PrivacyFilter
        # local_files_only: weights are staged at deploy time. A boot that
        # silently reaches the network is exactly the dependency the
        # fail-closed convention forbids.
        _model = PrivacyFilter.from_pretrained(
            MODEL_ID, device=DEVICE, local_files_only=True
        )
    return _model


def detect_spans(text):
    """Return [(category, start_codepoint, end_codepoint, score)].

    Seam for tests: test_app.py monkeypatches this so the contract can be
    verified without loading 1.5B parameters.
    """
    return [
        (s.category, s.start, s.end, float(s.score)) for s in _load().detect(text)
    ]


class ClassifyRequest(BaseModel):
    model: str
    input: str


@app.post("/v1/privacy/classify")
def classify(req: ClassifyRequest):
    if not req.input.strip():
        return {"data": [{"spans": []}]}
    spans = [
        {"category": c, "start": s, "end": e, "score": sc}
        for (c, s, e, sc) in detect_spans(req.input)
    ]
    return {"data": [{"spans": spans}]}


@app.get("/healthz")
def healthz():
    return {"status": "ok", "model": MODEL_ID}
```

`requirements.txt`, pinned:

```
fastapi==0.115.6
uvicorn==0.34.0
pydantic==2.10.4
```

Add `opf` and its torch pin at the versions resolved during Step 1; record the
exact versions rather than a range.

- [ ] **Step 5: Run to verify it passes**

Run: `cd deploy/pilot-gcp/privacy-filter && python -m pytest test_app.py -v`
Expected: PASS, 3 tests.

- [ ] **Step 6: End-to-end check against the real model**

Start the shim with weights staged, then:

```bash
curl -s localhost:8471/v1/privacy/classify \
  -H 'content-type: application/json' \
  -d '{"model":"openai/privacy-filter","input":"Ping 大三 about bob@example.com today"}'
```
Expected: `start` is 16, not 20. If it is 20, the conversion in `detect_spans`
is wrong — fix it before going further, and do not proceed on the assumption
that the Rust side will compensate.

- [ ] **Step 7: Commit**

```bash
git add deploy/pilot-gcp/privacy-filter/
git commit -m "Add the loopback privacy-filter serving shim

Reproduces the hosted classify wire shape over locally-staged weights. The
offset convention is pinned by test, because a byte-for-codepoint slip
redacts the wrong characters and reports success."
```

---

### Task 5: Deployment surface

**Files:**
- Create: `deploy/pilot-gcp/privacy-filter/trace-commons-privacy-filter.service`
- Create: `deploy/pilot-gcp/privacy-filter/stage-weights.sh`
- Modify: `deploy/pilot-gcp/ingest.env.template`
- Modify: `docs/operator/env-reference.md`, `docs/operator/deployment.md`

- [ ] **Step 1: Write the systemd unit**

```ini
[Unit]
Description=Trace Commons privacy filter (openai/privacy-filter)
After=network.target

[Service]
Type=exec
User=tc-privacy-filter
Group=tc-privacy-filter
Environment=HF_HUB_OFFLINE=1
Environment=PRIVACY_FILTER_DEVICE=cpu
EnvironmentFile=/etc/tracecommons/privacy-filter.env
WorkingDirectory=/opt/tracecommons-privacy-filter
ExecStart=/opt/tracecommons-privacy-filter/venv/bin/uvicorn app:app \
    --host 127.0.0.1 --port 8471 --workers 1
Restart=on-failure
RestartSec=5

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadOnlyPaths=/opt/tracecommons-privacy-filter
RestrictAddressFamilies=AF_INET AF_UNIX

[Install]
WantedBy=multi-user.target
```

`HF_HUB_OFFLINE=1` and `local_files_only` together make a boot that would have
reached the network fail loudly instead.

- [ ] **Step 2: Write the weight-staging script**

`stage-weights.sh` downloads `openai/privacy-filter` into
`/opt/tracecommons-privacy-filter/models` at deploy time, verifies the
directory is non-empty, and exits non-zero otherwise. It is run by the operator
before first start, never by the service.

- [ ] **Step 3: Document the env vars**

In `ingest.env.template`, beside the existing privacy-filter block:

```bash
#TRACE_PRIVACY_FILTER_BACKEND=self-hosted
#TRACE_PRIVACY_FILTER_SELF_HOSTED_BASE_URL=http://127.0.0.1:8471/v1
#TRACE_PRIVACY_FILTER_SELF_HOSTED_MODEL=openai/privacy-filter
#TRACE_PRIVACY_FILTER_SELF_HOSTED_TIMEOUT_MS=30000
```

Add the same four rows to the backend table in `env-reference.md` (:448), and
note in `deployment.md` that `self_hosted` is the expected
`privacy_filter_backend` value in `/health` and the boot line once cut over.

- [ ] **Step 4: Commit**

```bash
git add deploy/ docs/
git commit -m "Add the privacy-filter service unit and operator config

Weights are staged at deploy time and the unit runs with HF_HUB_OFFLINE, so
a boot that would reach the network fails loudly instead of hanging."
```

---

### Task 6: Shadow comparison

Gates the drain. Runs both backends over the same text and diffs spans,
hash-only. Do not skip this on the grounds that both serve the same weights:
NEAR wraps a 512-context model in an internal splitter, so their output is not
a priori identical to a true single-pass classification.

**Files:**
- Create: `scripts/operator/privacy-filter-shadow-compare.sh` (or a
  `trace-commons-gate-calibrate` subcommand, if that binary is the better home
  — check its existing subcommand shape first and follow it)

- [ ] **Step 1: Decide the home**

Read `crates/trace-commons-server/src/bin/trace-commons-gate-calibrate.rs` and
follow whichever pattern its subcommands already use. Do not invent a new tool
shape if one fits.

- [ ] **Step 2: Implement the comparison**

For each sampled field: classify via both adapters, then report per field the
count of spans from each, the count of exactly-matching `(category, start,
end)` triples, and a SHA-256 of the redacted output from each. **Report hashes
and counts only — never the text, never the spans themselves.**

- [ ] **Step 3: Define the acceptance bar before running it**

Write down what agreement is good enough to proceed, and commit that to the
runbook, so the bar is not chosen after seeing the result. A defensible
starting bar: no field where the self-hosted backend finds strictly fewer
categories than NEAR. Disagreement in the other direction is expected and fine
— a true single-pass classifier should find more, not less.

- [ ] **Step 4: Run it and record the evidence**

Run over a sample drawn from held submissions. Record the output in the
operator runbook alongside the date and the sample size.

- [ ] **Step 5: Commit**

```bash
git add scripts/ docs/
git commit -m "Add a hash-only shadow comparison for the two privacy backends

Same weights does not imply same output: the hosted endpoint wraps a
512-context model in an internal splitter."
```

---

### Task 7: Backlog measurement and attempt reset

**Do not begin this task until Task 6's evidence meets the recorded bar.**

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (new admin route)

- [ ] **Step 1: Count the actual backlog**

Query the pilot for submissions on `awaiting_pii_backstop`, grouped by whether
they have exhausted `TRACE_COMMONS_PII_BACKSTOP_MAX_ATTEMPTS`. **Record the
real number.** Every figure in earlier documents is stale; this step exists
because the spec explicitly refuses to carry one forward.

- [ ] **Step 2: Write the failing test for the admin route**

Find an existing `/v1/admin/*` handler and copy its auth shape — the pilot
requires an EdDSA-signed admin JWT and refuses static tokens. Test that the
route requires admin auth, resets only rows on `awaiting_pii_backstop`, and
returns counts only.

- [ ] **Step 3: Run to verify it fails**

Run: `cargo test -p trace-commons-server --no-run` then the specific test.
Expected: FAIL — route not found.

- [ ] **Step 4: Implement the route**

`POST /v1/admin/pii-backstop-reset-attempts`. Scope strictly to
`awaiting_pii_backstop`; return `{"reset": <count>}` and nothing else. Emit a
hash-only audit row.

- [ ] **Step 5: Verify**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/
git commit -m "Add an admin route to reset exhausted PII backstop attempts

Submissions that burned their attempts against a failing backend stay
invisible to enumeration once it is healthy, because the filter that hides
them does not know why they failed."
```

---

## Rollout (not a code task)

In order, and only after Tasks 1-7 are merged:

1. Schedule the downtime window. Resize `tc-pilot-host` `e2-standard-2` ->
   `e2-standard-4`. **This requires stopping the instance.** Price the resize
   first; the spec deliberately quotes no figure.
2. Stage weights, start the service, confirm `/healthz`, confirm the shim is
   reachable **only** on loopback.
3. Deploy the new ingest binary with the backend still `near-ai`. Confirm no
   behaviour change.
4. Flip to `self-hosted` with `TRACE_COMMONS_REQUIRE_PRIVACY_FILTER=1` still
   set. Confirm the boot line and `/health` both report `self_hosted`.
   Per `project_pilot_host_checkout_is_not_deployed_code`, verify the deployed
   binary by string marker, not by `git log` on the host.
5. Bounded canary batch. Inspect hash-only evidence.
6. Reset attempt counters, then drain, monitored.

## Self-Review

**Spec coverage.** Sibling adapter -> Tasks 1-3. Shim -> Task 4. Topology and
systemd -> Task 5. Offsets -> Task 4 Steps 1/2/6, plus the shared decoder in
Task 1. Cutover -> Rollout. Shadow comparison -> Task 6. Attempt reset ->
Task 7. Testing -> per-task steps. The spec's licensing constraint, absent from
the first draft, is in Global Constraints and verified in Task 3 Step 7.

**Type consistency.** `SelfHostedPrivacyFilterAdapter::new` takes four
arguments and no API key in both Task 2's test and its implementation.
`PrivacyFilterBackendTag::SelfHosted` labels `"self_hosted"` (underscore) while
the env value is `"self-hosted"` (hyphen) — deliberate, matching `NearAi` ->
`"near_ai"` against `"near-ai"`. `detect_spans` has the same signature in
`app.py` and in the test that monkeypatches it.

**Known gap.** Task 4 Step 1 is a genuine unknown: the `opf` package's offset
convention has not been verified, only flagged. It is a measurement step, not a
placeholder, and the task cannot proceed past it without an answer.
