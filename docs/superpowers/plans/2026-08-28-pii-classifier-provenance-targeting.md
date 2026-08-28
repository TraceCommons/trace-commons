# Provenance-Targeted PII Classification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Send only prose-bearing events to the NEAR AI privacy classifier, cutting per-trace round trips by roughly 10x.

**Architecture:** `TraceContributionEventType` already distinguishes contributor/model prose from tool traffic, so selection is a pure predicate over an existing envelope field -- no protocol change. A `PiiClassifyPolicy` value threads from the ingest binary into `rescrub_envelope_prose_pii_with`, which gates both its prose loop and its structured-payload loop. Policy and examined/skipped counts are recorded additively so existing serialized envelopes are byte-identical.

**Tech Stack:** Rust, tokio, serde, PostgreSQL. Crates: `trace-commons-protocol` (the classifier path), `trace-commons-server` (the ingest binary).

## Global Constraints

- Branch: `pii-provenance-targeting`, worktree `.claude/worktrees/pii-provenance-targeting`. Work only there; the main checkout is shared with another session.
- Verify with `RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins` and `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --no-run`. Plain `cargo check` does not apply `-D warnings`; CI does.
- Clippy is CI-enforced: `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen the allow-list.
- Run `cargo fmt --all` before every commit. After committing, run `git show --stat HEAD` and confirm only intended files changed -- the repo is not rustfmt-clean, so the editor hook can turn a one-line edit into a whole-file diff.
- `PiiClassifyPolicy` MUST be defined **ungated** (no `#[cfg(feature = ...)]`). PR #481 broke the default-features build and the macOS CI job by having ungated code reference a feature-gated type; `cargo check -p trace-commons-protocol` with default features is the check that catches it, because workspace builds unify features and hide it.
- Envelope schema must not change observably. New serialized fields use `#[serde(default, skip_serializing_if = ...)]` so envelopes that do not set them serialize byte-identically. A golden envelope digest is pinned in the contributor crate; verify with a workspace test run, not a single-crate one.
- No emojis in code, commits, or PRs. Commit subjects are short and imperative, with no `feat:`/`fix:` prefix.
- Audit and log surfaces are hash-only or label-only: never event content, contributor identity, or raw text.
- Default is fail-closed: `TRACE_COMMONS_PII_CLASSIFY_POLICY` defaults to `all-events` (today's behaviour).

---

### Task 1: `PiiClassifyPolicy` type and env parsing

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (add near the other policy/config types; `privacy_filter_backend_from_env` at ~2655 is the shape to copy)

**Interfaces:**
- Produces: `pub enum PiiClassifyPolicy { AllEvents, ProseOnly }`, `impl PiiClassifyPolicy { pub fn as_label(&self) -> &'static str; pub fn from_env() -> Self; }`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `trace_contribution.rs`:

```rust
#[test]
fn pii_classify_policy_parses_known_labels() {
    assert_eq!(
        PiiClassifyPolicy::from_label("prose-only"),
        Some(PiiClassifyPolicy::ProseOnly)
    );
    assert_eq!(
        PiiClassifyPolicy::from_label("all-events"),
        Some(PiiClassifyPolicy::AllEvents)
    );
    // Unknown values do not silently become the fast policy.
    assert_eq!(PiiClassifyPolicy::from_label("nonsense"), None);
}

#[test]
fn pii_classify_policy_label_round_trips() {
    for policy in [PiiClassifyPolicy::AllEvents, PiiClassifyPolicy::ProseOnly] {
        assert_eq!(PiiClassifyPolicy::from_label(policy.as_label()), Some(policy));
    }
}

#[test]
fn pii_classify_policy_defaults_to_all_events() {
    assert_eq!(PiiClassifyPolicy::default(), PiiClassifyPolicy::AllEvents);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-protocol pii_classify_policy -- --nocapture`
Expected: FAIL to compile, `cannot find type PiiClassifyPolicy in this scope`.

- [ ] **Step 3: Write the implementation**

Add to `trace_contribution.rs`, **not** inside any `#[cfg(feature = ...)]` block:

```rust
/// Which events the NEAR AI privacy classifier is asked to examine.
///
/// Throughput is `windows x round-trip` and the round trip is ~4.5 s, so the
/// only lever that moves it is issuing fewer windows. Contributor and model
/// prose are ~10% of trace volume; tool traffic is the other ~90%.
///
/// Defaults to `AllEvents`: an operator who has not made this decision keeps
/// today's behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PiiClassifyPolicy {
    /// Examine every event. Today's behaviour.
    #[default]
    AllEvents,
    /// Examine only prose-bearing events; tool traffic relies on the
    /// deterministic detectors, which still run over everything.
    ProseOnly,
}

impl PiiClassifyPolicy {
    /// The stable label used for both configuration and the recorded value,
    /// so the configured and recorded policy cannot drift apart.
    pub fn as_label(&self) -> &'static str {
        match self {
            Self::AllEvents => "all-events",
            Self::ProseOnly => "prose-only",
        }
    }

    pub fn from_label(label: &str) -> Option<Self> {
        match label.trim() {
            "all-events" => Some(Self::AllEvents),
            "prose-only" => Some(Self::ProseOnly),
            _ => None,
        }
    }

    /// Reads `TRACE_COMMONS_PII_CLASSIFY_POLICY`. An unset or unparseable
    /// value yields `AllEvents`: a typo must not silently narrow what the
    /// classifier examines.
    pub fn from_env() -> Self {
        std::env::var("TRACE_COMMONS_PII_CLASSIFY_POLICY")
            .ok()
            .and_then(|raw| Self::from_label(&raw))
            .unwrap_or_default()
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-protocol pii_classify_policy`
Expected: 3 passed.

- [ ] **Step 5: Verify the ungated build**

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol`
Expected: clean. This is the check that #481 failed.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Add PiiClassifyPolicy and its env parsing"
git show --stat HEAD
```

---

### Task 2: Event-type selection predicate

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs`

**Interfaces:**
- Consumes: `PiiClassifyPolicy` from Task 1.
- Produces: `pub fn policy_examines_event(policy: PiiClassifyPolicy, event_type: TraceContributionEventType) -> bool`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn all_events_policy_examines_every_event_type() {
    for event_type in [
        TraceContributionEventType::UserMessage,
        TraceContributionEventType::AssistantMessage,
        TraceContributionEventType::Reasoning,
        TraceContributionEventType::Feedback,
        TraceContributionEventType::ToolCall,
        TraceContributionEventType::ToolResult,
        TraceContributionEventType::RoutingDecision,
        TraceContributionEventType::HttpExchange,
    ] {
        assert!(
            policy_examines_event(PiiClassifyPolicy::AllEvents, event_type),
            "AllEvents must examine {event_type:?}"
        );
    }
}

#[test]
fn prose_only_policy_examines_authored_prose() {
    for event_type in [
        TraceContributionEventType::UserMessage,
        TraceContributionEventType::AssistantMessage,
        TraceContributionEventType::Reasoning,
        TraceContributionEventType::Feedback,
    ] {
        assert!(
            policy_examines_event(PiiClassifyPolicy::ProseOnly, event_type),
            "ProseOnly must examine {event_type:?}"
        );
    }
}

#[test]
fn prose_only_policy_skips_tool_traffic() {
    for event_type in [
        TraceContributionEventType::ToolCall,
        TraceContributionEventType::ToolResult,
        TraceContributionEventType::RoutingDecision,
        TraceContributionEventType::HttpExchange,
    ] {
        assert!(
            !policy_examines_event(PiiClassifyPolicy::ProseOnly, event_type),
            "ProseOnly must skip {event_type:?}"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-protocol policy_examines -- --nocapture`
Expected: FAIL to compile, `cannot find function policy_examines_event`.

- [ ] **Step 3: Write the implementation**

```rust
/// Whether `policy` submits this event's text to the classifier.
///
/// The match is exhaustive on purpose: a newly added event type must not
/// default into either bucket. Adding a variant will fail this to compile,
/// which is the intended prompt to decide whether it carries authored prose.
pub fn policy_examines_event(
    policy: PiiClassifyPolicy,
    event_type: TraceContributionEventType,
) -> bool {
    match policy {
        PiiClassifyPolicy::AllEvents => true,
        PiiClassifyPolicy::ProseOnly => match event_type {
            // Authored by a human or the model: where unpatterned PII such as
            // names and addresses actually originates.
            TraceContributionEventType::UserMessage
            | TraceContributionEventType::AssistantMessage
            | TraceContributionEventType::Reasoning
            | TraceContributionEventType::Feedback => true,
            // Tool traffic: ~90% of volume. Patterned secrets here are still
            // caught by the deterministic detectors, which are unaffected by
            // this policy. Unpatterned PII arriving through tool output is the
            // accepted, documented gap.
            TraceContributionEventType::ToolCall
            | TraceContributionEventType::ToolResult
            | TraceContributionEventType::RoutingDecision
            | TraceContributionEventType::HttpExchange => false,
        },
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-protocol policy_examines`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Add the prose-only event selection predicate"
git show --stat HEAD
```

---

### Task 3: Gate both classifier loops on the policy

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` -- `rescrub_envelope_prose_pii_with` (signature at ~4238; prose loop at ~4252; structured-payload loop at ~4273)
- Modify: all 13 call sites (`grep -rn "rescrub_envelope_prose_pii_with(" --include='*.rs' crates/`)

**Interfaces:**
- Consumes: `PiiClassifyPolicy`, `policy_examines_event` from Tasks 1-2.
- Produces: `pub async fn rescrub_envelope_prose_pii_with(adapter: &dyn PrivacyFilterAdapter, envelope: &mut TraceContributionEnvelope, policy: PiiClassifyPolicy) -> Result<Vec<ResidualRiskCondition>, TraceContributionError>`

- [ ] **Step 1: Write the failing test**

This test encodes the accepted gap so it cannot regress into a silent bug. Use the existing recording adapter pattern in this file's test module; if none records submitted text, add one:

```rust
/// Records every string handed to the classifier so a test can assert what
/// was and was not submitted.
struct RecordingAdapter {
    seen: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl PrivacyFilterAdapter for RecordingAdapter {
    async fn redact_text(
        &self,
        text: &str,
    ) -> Result<Option<SafePrivacyFilterRedaction>, TraceContributionError> {
        self.seen.lock().unwrap().push(text.to_string());
        Ok(None)
    }
}

#[tokio::test]
async fn prose_only_policy_does_not_submit_tool_result_text() {
    let adapter = RecordingAdapter { seen: Default::default() };
    let mut envelope = envelope_with_events(vec![
        (TraceContributionEventType::UserMessage, "my name is Dana Ruiz"),
        (TraceContributionEventType::ToolResult, "file says Dana Ruiz, 12 Oak Street"),
    ]);

    rescrub_envelope_prose_pii_with(&adapter, &mut envelope, PiiClassifyPolicy::ProseOnly)
        .await
        .expect("rescrub succeeds");

    let seen = adapter.seen.lock().unwrap().clone();
    assert!(
        seen.iter().any(|t| t.contains("my name is")),
        "prose event must be submitted"
    );
    // The accepted gap, asserted deliberately: unpatterned PII reaching the
    // trace through tool output is NOT model-examined under this policy.
    assert!(
        !seen.iter().any(|t| t.contains("12 Oak Street")),
        "tool result must not be submitted under prose-only"
    );
}

#[tokio::test]
async fn all_events_policy_still_submits_tool_result_text() {
    let adapter = RecordingAdapter { seen: Default::default() };
    let mut envelope = envelope_with_events(vec![
        (TraceContributionEventType::ToolResult, "file says Dana Ruiz, 12 Oak Street"),
    ]);

    rescrub_envelope_prose_pii_with(&adapter, &mut envelope, PiiClassifyPolicy::AllEvents)
        .await
        .expect("rescrub succeeds");

    assert!(
        adapter.seen.lock().unwrap().iter().any(|t| t.contains("12 Oak Street")),
        "all-events must preserve today's behaviour exactly"
    );
}
```

Add this helper next to the tests. It builds on
`sample_envelope_with_event_content` (defined at ~7900 in this file's test
module), which is the only envelope fixture here -- there is no
`sample_envelope()` or `sample_event()`:

```rust
fn envelope_with_events(
    events: Vec<(TraceContributionEventType, &str)>,
) -> super::TraceContributionEnvelope {
    use super::*;
    let mut envelope = sample_envelope_with_event_content("seed");
    // The fixture ships exactly one UserMessage event; reuse it as a template
    // so every required field stays populated.
    let template = envelope.events[0].clone();
    envelope.events = events
        .into_iter()
        .map(|(event_type, text)| {
            let mut event = template.clone();
            event.event_id = Uuid::new_v4();
            event.event_type = event_type;
            event.redacted_content = Some(text.to_string());
            event.structured_payload = Value::Null;
            event
        })
        .collect();
    envelope
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter prose_only_policy_does_not_submit -- --nocapture`
Expected: FAIL to compile -- `rescrub_envelope_prose_pii_with` takes 2 arguments, 3 supplied.

- [ ] **Step 3: Add the parameter and gate the prose loop**

Change the signature to take `policy: PiiClassifyPolicy`, then gate the loop at ~4252:

```rust
    for (index, event) in envelope.events.iter().enumerate() {
        if !policy_examines_event(policy, event.event_type) {
            skipped_events += 1;
            continue;
        }
        let Some(content) = event.redacted_content.as_deref() else {
            continue;
        };
        examined_events += 1;
        if let Some(redaction) = adapter.redact_text(content).await? {
            merge_privacy_filter_summary(&mut summary, &redaction.summary);
            report.merge(redaction.report);
            event_updates.push((index, redaction.redacted_text));
        }
    }
```

Declare the counters beside the other accumulators near the top of the function:

```rust
    let mut examined_events: u32 = 0;
    let mut skipped_events: u32 = 0;
```

- [ ] **Step 4: Gate the structured-payload loop**

At ~4273, skip payloads of non-examined events. `structured_complete` MUST NOT be set false here: a policy skip is a deliberate scope decision, not a budget exhaustion, and conflating them would suppress downgrades for every trace.

```rust
        for (index, event) in envelope.events.iter().enumerate() {
            if !policy_examines_event(policy, event.event_type) {
                continue;
            }
            if event.structured_payload.is_null() {
                continue;
            }
```

- [ ] **Step 5: Update all call sites**

Every existing call site passes `PiiClassifyPolicy::AllEvents`, so the 12 existing protocol tests continue to assert today's behaviour unchanged:

```bash
grep -rn "rescrub_envelope_prose_pii_with(" --include='*.rs' crates/
```

For the production caller at `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:40425`, pass `PiiClassifyPolicy::AllEvents` for now; Task 5 replaces it with the configured value.

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter`
Expected: the two new tests pass and all 12 pre-existing call-site tests still pass.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-protocol/src/trace_contribution.rs crates/trace-commons-server/src/bin/trace-commons-ingest.rs
git commit -m "Gate the classifier passes on the classify policy"
git show --stat HEAD
```

---

### Task 4: Record the policy and the examined/skipped counts

**Files:**
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` -- `SafePrivacyFilterSummary` (~424) and the summary write in `rescrub_envelope_prose_pii_with` (~4310)

**Interfaces:**
- Consumes: `examined_events` / `skipped_events` from Task 3.
- Produces: `SafePrivacyFilterSummary { classify_policy: Option<String>, events_examined: u32, events_skipped_by_policy: u32, .. }`

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn summary_without_policy_serializes_unchanged() {
    // The envelope digest is pinned in the contributor crate. A summary that
    // does not set the new fields must serialize byte-identically to before.
    let summary = SafePrivacyFilterSummary {
        schema_version: 1,
        output_mode: "spans".to_string(),
        span_count: 0,
        by_label: Default::default(),
        decoded_mismatch: false,
        classify_policy: None,
        events_examined: 0,
        events_skipped_by_policy: 0,
    };
    let json = serde_json::to_string(&summary).expect("serializes");
    assert!(!json.contains("classify_policy"), "absent policy must not serialize");
    assert!(!json.contains("events_examined"), "zero counts must not serialize");
}

#[tokio::test]
async fn prose_only_records_policy_and_counts() {
    let adapter = RecordingAdapter { seen: Default::default() };
    let mut envelope = envelope_with_events(vec![
        (TraceContributionEventType::UserMessage, "my name is Dana Ruiz"),
        (TraceContributionEventType::ToolResult, "file says Dana Ruiz, 12 Oak Street"),
        (TraceContributionEventType::ToolCall, "grep -rn Dana"),
    ]);

    rescrub_envelope_prose_pii_with(&adapter, &mut envelope, PiiClassifyPolicy::ProseOnly)
        .await
        .expect("rescrub succeeds");

    let summary = envelope
        .privacy
        .privacy_filter_summary
        .as_ref()
        .expect("summary recorded");
    assert_eq!(summary.classify_policy.as_deref(), Some("prose-only"));
    assert_eq!(summary.events_examined, 1);
    assert_eq!(summary.events_skipped_by_policy, 2);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter records_policy -- --nocapture`
Expected: FAIL to compile, `struct SafePrivacyFilterSummary has no field named classify_policy`.

- [ ] **Step 3: Add the fields additively**

```rust
pub struct SafePrivacyFilterSummary {
    pub schema_version: u16,
    pub output_mode: String,
    pub span_count: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub by_label: BTreeMap<String, u32>,
    pub decoded_mismatch: bool,
    /// Which classify policy produced this result, so decisions made under
    /// different policies stay distinguishable after the fact. Label only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classify_policy: Option<String>,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub events_examined: u32,
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub events_skipped_by_policy: u32,
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}
```

Fix every other construction site of this struct that the compiler now rejects:

```bash
RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol --features near-ai-privacy-filter 2>&1 | grep -n "missing field" | head -20
```

- [ ] **Step 4: Populate them in the rescrub summary write**

Where the summary is merged into the envelope (~4310), set the recorded values before the merge:

```rust
    if let Some(summary) = &mut summary {
        summary.classify_policy = Some(policy.as_label().to_string());
        summary.events_examined = examined_events;
        summary.events_skipped_by_policy = skipped_events;
    }
    if let Some(summary) = &summary {
        merge_privacy_filter_summary(&mut envelope.privacy.privacy_filter_summary, summary);
    }
```

Then confirm `merge_privacy_filter_summary` carries the new fields through rather than dropping them; if it builds its output field-by-field, add them there too.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-protocol --features near-ai-privacy-filter`
Expected: all pass.

- [ ] **Step 6: Verify the pinned envelope digest is unmoved**

Run: `RUSTFLAGS='-D warnings' cargo test --workspace --no-run` then `cargo test --workspace 2>&1 | tail -30`
Expected: the contributor crate's golden-digest test passes. Test the workspace, not the crate -- the pin lives outside `trace-commons-protocol`.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-protocol/src/trace_contribution.rs
git commit -m "Record the classify policy and per-policy event counts"
git show --stat HEAD
```

---

### Task 5: Wire the policy through the ingest binary

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (import at ~38; call site at ~40425)
- Modify: `deploy/pilot-gcp/ingest.env.template`

**Interfaces:**
- Consumes: `PiiClassifyPolicy::from_env()` from Task 1.

- [ ] **Step 1: Write the failing test**

Add to `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`:

```rust
#[test]
fn classify_policy_env_defaults_closed() {
    const VAR: &str = "TRACE_COMMONS_PII_CLASSIFY_POLICY";
    let prior = std::env::var(VAR).ok();
    // SAFETY: this test runs single-threaded with respect to env mutation; no
    // other thread reads these vars while we overwrite them. Rust 2024 marks
    // set_var/remove_var unsafe to flag the global-state hazard. This mirrors
    // the existing env-var tests in this file (see ~70361).
    unsafe { std::env::remove_var(VAR) };
    assert_eq!(PiiClassifyPolicy::from_env(), PiiClassifyPolicy::AllEvents);

    unsafe { std::env::set_var(VAR, "prose-only") };
    assert_eq!(PiiClassifyPolicy::from_env(), PiiClassifyPolicy::ProseOnly);

    // A typo must not silently narrow what the classifier examines.
    unsafe { std::env::set_var(VAR, "typo") };
    assert_eq!(PiiClassifyPolicy::from_env(), PiiClassifyPolicy::AllEvents);

    unsafe {
        match prior {
            Some(value) => std::env::set_var(VAR, value),
            None => std::env::remove_var(VAR),
        }
    }
}
```

Do NOT add a test-env crate for this. `temp_env` is not a dependency of this
workspace, and adding any dependency requires explicit approval.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-server classify_policy_env_defaults_closed -- --nocapture`
Expected: FAIL -- unresolved import or assertion failure.

- [ ] **Step 3: Read the policy once and pass it at the call site**

Add `PiiClassifyPolicy` to the existing `trace_commons_protocol` import block at line ~38. At the call site (~40425), replace the placeholder from Task 3:

```rust
    let classify_policy = PiiClassifyPolicy::from_env();
    let residual_risk_basis =
        rescrub_envelope_prose_pii_with(adapter, &mut envelope, classify_policy).await?;
```

- [ ] **Step 4: Log the active policy once at startup**

Find the existing startup configuration log and add the label. Label only -- no content:

```rust
    tracing::info!(
        policy = PiiClassifyPolicy::from_env().as_label(),
        "Trace Commons PII classify policy"
    );
```

- [ ] **Step 5: Document the variable in the env template**

Append to `deploy/pilot-gcp/ingest.env.template`:

```bash
# Which events the NEAR AI privacy classifier examines.
#   all-events  every event (default; today's behaviour)
#   prose-only  contributor/model prose only -- roughly 10x fewer round trips.
#               Tool output relies on the deterministic detectors, which run
#               over everything regardless. Unpatterned PII arriving through
#               tool output is not model-examined under this policy.
# Unset or unrecognised values fall back to all-events.
TRACE_COMMONS_PII_CLASSIFY_POLICY=all-events
```

- [ ] **Step 6: Run the full verification**

```bash
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins
RUSTFLAGS='-D warnings' cargo check -p trace-commons-protocol
RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
```

Expected: all clean.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-server/src/bin/trace-commons-ingest.rs crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs deploy/pilot-gcp/ingest.env.template
git commit -m "Wire the classify policy through the ingest binary"
git show --stat HEAD
```

---

### Task 6: Operator documentation

**Files:**
- Create: `docs/operator/pii-classify-policy.md`
- Modify: `docs/operator/README.md` (runbook index)

- [ ] **Step 1: Write the runbook**

Create `docs/operator/pii-classify-policy.md` covering: what the two policies do; the measured ~10x round-trip reduction and the 10.3% prose share it rests on; the accepted tool-output gap in plain language; that rollback is a config change plus a service restart, not a redeploy; and how to confirm the active policy from the startup log line.

- [ ] **Step 2: Add it to the runbook index**

Add a line to `docs/operator/README.md` matching the existing entry format.

- [ ] **Step 3: Commit**

```bash
git add docs/operator/pii-classify-policy.md docs/operator/README.md
git commit -m "Document the PII classify policy for operators"
git show --stat HEAD
```

---

### Task 7: Open the pull request

- [ ] **Step 1: Push and open the PR**

```bash
git push -u origin pii-provenance-targeting
gh pr create --repo zmanian/trace-commons-server \
  --title "Target the PII classifier at prose-bearing events" \
  --body "$(cat <<'BODY'
Throughput on the PII backstop is `windows x round-trip` at ~4.5 s per window,
which accounts for the 29-90 minute driver ticks in #475. Concurrency and
batching are both unavailable, so the only lever is issuing fewer windows.

Measured over 60 local sessions and 13.2M characters, contributor and model
prose are 10.3% of trace volume; tool traffic is ~90%. This narrows the
classifier to prose-bearing events, roughly a 10x reduction, stacking with the
window cache from #477.

`TRACE_COMMONS_PII_CLASSIFY_POLICY` defaults to `all-events`, so behaviour is
unchanged unless an operator opts in. The deterministic detectors still run
over every event; only the model pass narrows. Unpatterned PII arriving
through tool output is no longer model-examined -- an accepted, documented
gap, asserted by a test so it cannot regress silently.

Design: docs/superpowers/specs/2026-08-28-pii-classifier-provenance-targeting-design.md
BODY
)"
```

- [ ] **Step 2: Confirm CI is green**

```bash
gh pr checks --repo zmanian/trace-commons-server --watch
```

All sixteen-plus jobs gate the PR. `cargo check (default features)` and `macOS app tests` are the two that catch the feature-unification mistake from #481.
