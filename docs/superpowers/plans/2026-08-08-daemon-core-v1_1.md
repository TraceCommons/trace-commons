# Daemon Core v1.1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `preview` tell the truth, add the methods a GUI needs, and expose the daemon through a C ABI so a native app can host it in-process.

**Architecture:** All work is in `crates/trace-commons-contributor` plus one new `crates/trace-commons-contributor-ffi` crate. Preview becomes a real dry run reading `RedactionReport` off the produced envelope. New IPC methods reuse the existing `handle_request` router. The FFI crate is a thin `catch_unwind` wrapper with no logic of its own.

**Tech Stack:** Rust 2024, tokio, serde, anyhow, uuid, chrono. Spec: `docs/superpowers/specs/2026-08-08-daemon-core-v1_1-design.md`.

## Global Constraints

- **No new dependencies on macOS or Linux.** `windows-sys` is approved but confined to `[target.'cfg(windows)'.dependencies]` and is NOT used by this plan. `git diff origin/main -- Cargo.lock` must stay empty except for the new workspace member.
- **Verify with CI flags:** `RUSTFLAGS='-D warnings' cargo check -p trace-commons-contributor --bins`, `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --no-run`, and `cargo fmt --all -- --check`. CI has 8 jobs including `cargo fmt --check`.
- **Clippy allow-list, not widened:** `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
- **Hash-only / label-only** across every boundary, FFI included. No path, token, URL, or trace content in any log, receipt, history record, or error string.
- **Fail-closed.** A configured-but-unavailable privacy filter refuses; never downgrade.
- **No emojis** anywhere. Commit subjects short and imperative, no `feat:`/`fix:` prefixes.
- **Baseline before starting:** record `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor` pass/fail counts. Current known baseline is **267 passed, 0 failed**.

## What already exists (do not rebuild)

- `RedactionReport` (`trace-commons-protocol/src/trace_contribution.rs:1399`) has `counts: BTreeMap<String,u32>`, `pii_labels_present`, `warnings`, `blocked_secret_detected`, `key_finding_detected` — and is already on the envelope at field `report` (`:355`). Preview reads it; it does not compute counts.
- `SubmitContext::submit_one` with `SubmitOptions.dry_run` already runs redaction without uploading.
- `envelope_size(&envelope) -> Result<usize>` (`envelope.rs:275`) gives the true redacted byte count.
- `consent::VALID_SCOPES` (`consent.rs:12`) and `scopes_to_allowed_uses` (`:47`) are the scope source of truth. `public_attribution` maps to `&[]`.
- `ipc::handle_request(shared, req, origin) -> Response` is the single router both socket and CLI use.

## File Structure

| File | Responsibility |
|---|---|
| `src/daemon/preview.rs` (create) | Run the dry run, build the summary from `RedactionReport` |
| `src/daemon/ipc.rs` (modify) | New methods, v1_1 version, drop `Origin` gating |
| `src/daemon/policy.rs` (modify) | Label disambiguation |
| `src/daemon/health.rs` (modify) | Precedence ordering |
| `src/daemon/audit.rs` (create) | Hash-only local audit log |
| `src/daemon/enroll.rs` (create) | `enroll`, `consent_options`, `set_consent_scopes` |
| `src/daemon/watcher.rs` (modify) | Retain eligibility reasons |
| `crates/trace-commons-contributor-ffi/` (create) | C ABI wrapper + generated header |
| `docs/contributor-daemon-ipc-v1.md` (modify) | Becomes the v1_1 contract |

---

### Task 1: Real preview

The defect: `ipc.rs` returns `entry.size_bytes`, the raw file size. Redaction shrinks the payload, so the number overstates what leaves the machine.

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/preview.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (the `"preview"` arm)
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs` (add `pub mod preview;`)

**Interfaces:**
- Consumes: `SubmitContext`, `SubmitOptions`, `QueueEntry`, `TraceSource`.
- Produces:
```rust
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct PreviewSummary {
    pub would_send_bytes: usize,
    pub raw_session_bytes: u64,
    pub event_count: usize,
    pub opening_prompt: String,
    pub redactions: std::collections::BTreeMap<String, u32>,
    pub pii_labels_present: Vec<String>,
    pub consent_scopes: Vec<String>,
    pub residual_risk: String,
}

/// Redact one session without uploading and describe exactly what would be
/// sent. Same redaction path the uploader uses.
pub async fn build_preview(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    near_ai: Option<NearAiSettings>,
    source: &dyn TraceSource,
    session_ref: &SessionRef,
) -> anyhow::Result<(PreviewSummary, String)>;   // (summary, redacted body)
```

- [ ] **Step 1: Write the failing tests**

In `preview.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A session with a planted secret, so redaction has something to do.
    fn fixture_session() -> (tempfile::TempDir, ClaudeCodeSource, SessionRef) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let project = root.join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project).unwrap();
        std::fs::write(
            project.join("11111111-1111-1111-1111-111111111111.jsonl"),
            "{\"type\":\"user\",\"message\":{\"role\":\"user\",\
             \"content\":\"deploy with key sk-fake-fixture-secret-1234\"},\
             \"cwd\":\"/Users/testuser/code/myproj\",\
             \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
             \"sessionId\":\"11111111-1111-1111-1111-111111111111\",\
             \"uuid\":\"a1\"}\n",
        )
        .unwrap();
        let src = ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        (dir, src, r)
    }

    #[tokio::test]
    async fn preview_reports_the_redacted_size_not_the_raw_size() {
        // The defect this task exists to fix: the old code returned the raw
        // file size, which overstates what actually leaves the machine.
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert!(summary.raw_session_bytes > 0);
        assert!(summary.would_send_bytes > 0);
        assert_ne!(
            summary.would_send_bytes as u64, summary.raw_session_bytes,
            "a redacted envelope is not the same size as the raw session file"
        );
    }

    #[tokio::test]
    async fn preview_reports_what_redaction_actually_removed() {
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        let total: u32 = summary.redactions.values().sum();
        assert!(total > 0, "planted secret should appear in the counts: {:?}", summary.redactions);
    }

    #[tokio::test]
    async fn preview_body_does_not_contain_the_planted_secret() {
        // The whole point of showing a body is that it is the redacted one.
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (_summary, body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert!(!body.contains("sk-fake-fixture-secret-1234"), "secret survived into the preview body");
    }

    #[tokio::test]
    async fn preview_carries_an_opening_prompt_and_an_event_count() {
        let (_d, src, r) = fixture_session();
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert_eq!(summary.event_count, 1);
        assert!(!summary.opening_prompt.is_empty());
        assert!(!summary.opening_prompt.contains("sk-fake-fixture-secret-1234"),
            "the opening prompt must be the redacted one");
    }

    #[tokio::test]
    async fn preview_opening_prompt_is_truncated() {
        // 200 chars, so a huge first message cannot dominate a queue row.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("projects");
        let project = root.join("-Users-testuser-code-myproj");
        std::fs::create_dir_all(&project).unwrap();
        let long = "x".repeat(500);
        std::fs::write(
            project.join("22222222-2222-2222-2222-222222222222.jsonl"),
            format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"{long}\"}},\
                 \"cwd\":\"/Users/testuser/code/myproj\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"22222222-2222-2222-2222-222222222222\",\"uuid\":\"a1\"}}\n"
            ),
        )
        .unwrap();
        let src = ClaudeCodeSource::new(root);
        let r = src.discover().unwrap().remove(0);
        let (_sd, store) = crate::config::tests_support::temp_store();
        let cfg = sample_cfg(&store);
        let (summary, _body) = build_preview(&store, &cfg, None, &src, &r).await.unwrap();
        assert!(summary.opening_prompt.chars().count() <= 200);
    }
}
```

`sample_cfg(&store)` generates a device key and returns a `ContributorConfig`
with `issuer_url`/`ingest_url` set to `http://issuer.invalid` /
`http://ingest.invalid` and `pii_filter: None` — preview never reaches the
network.

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::preview`
Expected: FAIL, `build_preview` not found.

- [ ] **Step 3: Implement `build_preview`**

Load the transcript, build the raw contribution, redact through
`build_redactor_with` + `redact_to_envelope` (the same calls `submit_one`
makes), then read the results off the envelope:

```rust
let redactor = build_redactor_with(cfg, transcript.cwd.as_deref(), near_ai)
    .map_err(|_| anyhow::anyhow!("pii-filter-unavailable"))?;
let raw = build_raw_contribution(&transcript, cfg, Utc::now());
let envelope = redact_to_envelope(&redactor, raw).await?;
let would_send_bytes = envelope_size(&envelope)?;
let report = &envelope.value.report;   // counts already computed
```

`opening_prompt` is the first event whose kind is `User`, taken from the
**redacted** envelope, truncated to 200 characters on a char boundary.
`residual_risk` is serialized from the envelope's residual-risk field, or
`"pattern-based"` when absent. The body is `serde_json::to_string_pretty` of
the redacted envelope events.

- [ ] **Step 4: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::preview`
Expected: PASS, 5 tests.

- [ ] **Step 5: Wire the IPC `preview` arm to it**

Replace the `"preview"` arm's body. It must resolve the session through its
adapter (as `drain_approved` does via `find_session`), call `build_preview`,
and return the summary. The socket returns summary only, never the body.

Because `handle_request` is synchronous and `build_preview` is async, add a
`preview_summary_blocking` helper that uses
`tokio::runtime::Handle::current().block_in_place` — or, preferred, make the
`preview` arm return a marker the async caller fulfils. Choose one and note it
in the module doc; do not leave both.

- [ ] **Step 6: Update the contract test and document**

`tests/daemon_ipc_contract.rs`: assert `preview` returns `would_send_bytes`
strictly less than `raw_session_bytes` for the fixture, and that the response
contains `redactions`.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add -A
git commit -m "Make preview report the redacted envelope, not the raw file"
```

---

### Task 2: Health precedence

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/health.rs`

**Interfaces:**
- Produces: `pub fn precedence(label: &str) -> u8` (lower wins) and
  `HealthState::fail` respecting it.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_higher_priority_condition_replaces_a_lower_one() {
    // Not-logged-in outranks a transient network problem: the contributor can
    // act on the first and only wait out the second.
    let mut h = HealthState::default();
    h.fail(LABEL_INGEST_UNREACHABLE, at("2026-08-08T12:00:00Z"));
    h.fail(LABEL_NOT_LOGGED_IN, at("2026-08-08T12:01:00Z"));
    assert_eq!(h.last_error_label.as_deref(), Some(LABEL_NOT_LOGGED_IN));
}

#[test]
fn a_lower_priority_condition_does_not_displace_a_higher_one() {
    let mut h = HealthState::default();
    h.fail(LABEL_NOT_LOGGED_IN, at("2026-08-08T12:00:00Z"));
    h.fail(LABEL_INGEST_UNREACHABLE, at("2026-08-08T12:01:00Z"));
    assert_eq!(h.last_error_label.as_deref(), Some(LABEL_NOT_LOGGED_IN));
}

#[test]
fn the_documented_order_is_total_and_has_no_ties() {
    let order = [
        LABEL_NOT_LOGGED_IN, LABEL_NEAR_AI_NOTICE_PENDING, LABEL_CANARY_FAILED,
        LABEL_PII_FILTER_UNAVAILABLE, LABEL_CLAIM_MINT_FAILED,
        LABEL_INGEST_UNREACHABLE, LABEL_QUEUE_FULL, LABEL_DAILY_CAP_REACHED,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for l in order {
        assert!(seen.insert(precedence(l)), "duplicate precedence for {l}");
    }
    for pair in order.windows(2) {
        assert!(precedence(pair[0]) < precedence(pair[1]), "{} must outrank {}", pair[0], pair[1]);
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::health`
Expected: FAIL, `precedence` not found.

- [ ] **Step 3: Implement**

```rust
pub fn precedence(label: &str) -> u8 {
    match label {
        LABEL_NOT_LOGGED_IN => 0,
        LABEL_NEAR_AI_NOTICE_PENDING => 1,
        LABEL_CANARY_FAILED => 2,
        LABEL_PII_FILTER_UNAVAILABLE => 3,
        LABEL_CLAIM_MINT_FAILED => 4,
        LABEL_INGEST_UNREACHABLE => 5,
        LABEL_QUEUE_FULL => 6,
        LABEL_DAILY_CAP_REACHED => 7,
        _ => 8,
    }
}
```
`fail` keeps the current label when the incoming one has a worse (higher)
precedence, and keeps `since` when the label is unchanged.

- [ ] **Step 4: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::health`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all && git add -A
git commit -m "Order health labels so actionable conditions outrank transient ones"
```

---

### Task 3: Project label disambiguation

Two repositories both called `api` are indistinguishable in the queue, and one
might be the client's. This can cause someone to approve the wrong repository.

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/policy.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/watcher.rs` (call site)

**Interfaces:**
- Produces:
```rust
/// A display label unique within `known_keys`. Adds a short stable hash
/// suffix only when the basename collides.
pub fn disambiguated_label(project_key: &str, known_keys: &[String]) -> String;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_unique_basename_is_left_alone() {
    let keys = vec!["/Users/z/code/alpha".to_string(), "/Users/z/code/beta".to_string()];
    assert_eq!(disambiguated_label("/Users/z/code/alpha", &keys), "alpha");
}

#[test]
fn colliding_basenames_get_distinct_stable_suffixes() {
    // The dangerous case: one of these is the client's repo.
    let keys = vec!["/Users/z/work/api".to_string(), "/Users/z/client/api".to_string()];
    let a = disambiguated_label("/Users/z/work/api", &keys);
    let b = disambiguated_label("/Users/z/client/api", &keys);
    assert_ne!(a, b, "colliding projects must be distinguishable");
    assert!(a.starts_with("api ("), "got {a}");
    assert_eq!(a, disambiguated_label("/Users/z/work/api", &keys), "must be stable");
}

#[test]
fn a_suffix_never_contains_a_path_segment() {
    // The suffix is a hash, not a directory name: paths never cross the wire.
    let keys = vec!["/Users/z/work/api".to_string(), "/Users/z/client/api".to_string()];
    let a = disambiguated_label("/Users/z/work/api", &keys);
    assert!(!a.contains("work") && !a.contains('/'), "got {a}");
}

#[test]
fn the_unknown_bucket_is_never_suffixed() {
    let keys = vec![UNKNOWN_PROJECT_KEY.to_string()];
    assert_eq!(disambiguated_label(UNKNOWN_PROJECT_KEY, &keys), UNKNOWN_PROJECT_KEY);
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::policy`
Expected: FAIL.

- [ ] **Step 3: Implement**

Basename via the existing `project_label_for`. If any other key in
`known_keys` yields the same basename, append `" (xxxx)"` where `xxxx` is the
first four hex characters of `sha256(project_key)`.

- [ ] **Step 4: Use it in the watcher**

`tick` builds the label from the policy's known keys plus the keys of entries
already in the queue, so a collision is detected even before both projects
have a queue entry.

- [ ] **Step 5: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && git add -A
git commit -m "Disambiguate colliding project labels so the wrong repo cannot be approved"
```

---

### Task 4: Audit log

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/audit.rs`
- Modify: `crates/trace-commons-contributor/src/config.rs` (add `DAEMON_AUDIT_FILE`, extend `wipe()`)
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs`

**Interfaces:**
- Produces:
```rust
pub const DAEMON_AUDIT_FILE: &str = "daemon-audit.jsonl";

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub at: chrono::DateTime<chrono::Utc>,
    pub action: String,        // fixed label
    pub project_label: Option<String>,
    pub detail: Option<String>, // fixed label only
}

pub fn append(store: &ConfigStore, entry: &AuditEntry) -> anyhow::Result<()>;
pub fn load(store: &ConfigStore) -> anyhow::Result<Vec<AuditEntry>>;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn entries_round_trip_in_order() {
    let (_d, store) = crate::config::tests_support::temp_store();
    append(&store, &entry("armed-auto-upload", Some("proj"))).unwrap();
    append(&store, &entry("bulk-approved", None)).unwrap();
    let all = load(&store).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].action, "armed-auto-upload");
}

#[test]
fn an_audit_entry_never_carries_a_path() {
    let (_d, store) = crate::config::tests_support::temp_store();
    append(&store, &entry("armed-auto-upload", Some("proj"))).unwrap();
    let raw = store.read_daemon_file(DAEMON_AUDIT_FILE).unwrap().unwrap();
    let text = String::from_utf8(raw).unwrap();
    assert!(!text.contains('/'), "audit must be label-only: {text}");
}

#[test]
fn a_corrupt_line_is_skipped_rather_than_losing_the_log() {
    let (_d, store) = crate::config::tests_support::temp_store();
    append(&store, &entry("armed-auto-upload", Some("proj"))).unwrap();
    let mut raw = store.read_daemon_file(DAEMON_AUDIT_FILE).unwrap().unwrap();
    raw.extend_from_slice(b"not json\n");
    store.write_daemon_file(DAEMON_AUDIT_FILE, &raw).unwrap();
    assert_eq!(load(&store).unwrap().len(), 1);
}

#[test]
fn the_audit_log_is_removed_on_logout() {
    let (_d, store) = crate::config::tests_support::temp_store();
    append(&store, &entry("armed-auto-upload", Some("proj"))).unwrap();
    store.wipe().unwrap();
    assert!(store.read_daemon_file(DAEMON_AUDIT_FILE).unwrap().is_none());
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::audit`
Expected: FAIL.

- [ ] **Step 3: Implement, appending via the existing atomic writer**

Read-modify-write the whole file through `write_daemon_file`. Add
`DAEMON_AUDIT_FILE` to the `wipe()` name list and to `tmp_prefixes`.

- [ ] **Step 4: Record the two events that matter**

In `handle_request`: `set_project_mode` to `auto_upload` writes
`armed-auto-upload`; `approve` with `all: true` writes `bulk-approved` with
the count as `detail`.

- [ ] **Step 5: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib daemon::audit config::`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && git add -A
git commit -m "Record autonomy changes in a hash-only local audit log"
```

---

### Task 5: New IPC methods

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/enroll.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/queue.rs` (cancel support)
- Modify: `crates/trace-commons-contributor/src/daemon/state.rs` (`paused_until`)

**Interfaces:**
- Produces, in `enroll.rs`:
```rust
/// The scope list with human descriptions, sourced from consent::VALID_SCOPES
/// so three shells cannot each hardcode a copy that drifts.
pub fn consent_options() -> serde_json::Value;
```
- Adds methods `enroll`, `consent_options`, `set_consent_scopes`,
  `acknowledge_near_ai_notice`, `cancel`, `list_audit`, `eligibility_reasons`,
  and `pause {until}`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn consent_options_lists_every_valid_scope_with_a_description() {
    let v = consent_options();
    let scopes = v["scopes"].as_array().unwrap();
    assert_eq!(scopes.len(), crate::consent::VALID_SCOPES.len());
    for s in scopes {
        assert!(!s["description"].as_str().unwrap().is_empty());
    }
}

#[test]
fn consent_options_marks_the_floor_scope_as_always_on() {
    let v = consent_options();
    let floor = v["scopes"].as_array().unwrap().iter()
        .find(|s| s["name"] == "debugging_evaluation").unwrap();
    assert_eq!(floor["always_on"], true);
}

#[test]
fn consent_options_marks_public_attribution_as_granting_no_data_use() {
    // It maps to an empty allowed-use set, so presenting it beside four real
    // data-use scopes with equal weight misleads in both directions.
    let v = consent_options();
    let pa = v["scopes"].as_array().unwrap().iter()
        .find(|s| s["name"] == "public_attribution").unwrap();
    assert_eq!(pa["grants_data_use"], false);
}

#[test]
fn acknowledging_the_near_ai_notice_clears_the_blocking_health_label() {
    // Without this an app-only contributor is stuck forever.
    let s = shared();
    s.health.lock().unwrap().fail(LABEL_NEAR_AI_NOTICE_PENDING, Utc::now());
    let r = handle_request(&s, &req("acknowledge_near_ai_notice", json!({})), Origin::Socket);
    assert!(r.error.is_none(), "{:?}", r.error);
    assert!(s.store.near_ai_notice_shown());
    assert!(s.health.lock().unwrap().ok());
}

#[test]
fn cancel_returns_an_approved_entry_to_pending() {
    let s = shared();
    let id = seed_approved_entry(&s);
    let r = handle_request(&s, &req("cancel", json!({"entry_id": id.to_string()})), Origin::Socket);
    assert!(r.error.is_none(), "{:?}", r.error);
    assert_eq!(s.queue.lock().unwrap().get(id).unwrap().state, QueueState::Pending);
}

#[test]
fn cancel_refuses_once_the_upload_is_in_flight() {
    let s = shared();
    let id = seed_entry_in_state(&s, QueueState::Uploading);
    let r = handle_request(&s, &req("cancel", json!({"entry_id": id.to_string()})), Origin::Socket);
    assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
}

#[test]
fn a_timed_pause_is_persisted_so_it_survives_a_restart() {
    // An app-side timer would die with the app and silently un-pause.
    let s = shared();
    let until = "2030-01-01T00:00:00Z";
    handle_request(&s, &req("pause", json!({"until": until})), Origin::Socket);
    assert_eq!(
        s.state.lock().unwrap().paused_until.map(|t| t.to_rfc3339()),
        Some(until.parse::<chrono::DateTime<Utc>>().unwrap().to_rfc3339())
    );
}

#[test]
fn a_timed_pause_lapses_on_its_own() {
    let s = shared();
    handle_request(&s, &req("pause", json!({"until": "2020-01-01T00:00:00Z"})), Origin::Socket);
    assert_eq!(s.status_value()["paused"], false, "an elapsed pause is not a pause");
}

#[test]
fn arming_autonomy_over_the_socket_is_now_allowed() {
    // The terminal-only gate is removed; see the v1.1 spec for the reasoning.
    let s = shared();
    let r = handle_request(
        &s,
        &req("set_project_mode", json!({"project_key": "/tmp/p", "mode": "auto_upload"})),
        Origin::Socket,
    );
    assert!(r.error.is_none(), "{:?}", r.error);
}

#[test]
fn arming_autonomy_writes_an_audit_entry() {
    let s = shared();
    handle_request(
        &s,
        &req("set_project_mode", json!({"project_key": "/tmp/p", "mode": "auto_upload"})),
        Origin::Socket,
    );
    let log = crate::daemon::audit::load(&s.store).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].action, "armed-auto-upload");
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor --lib daemon::ipc daemon::enroll`
Expected: FAIL.

- [ ] **Step 3: Implement `consent_options`**

Descriptions come from this table, matching the shared shell spec's copy so
three shells render identical words:

```rust
const DESCRIPTIONS: [(&str, &str, bool); 5] = [
    ("debugging_evaluation",
     "Researchers read traces to find where coding agents fail, and score agents against each other.",
     true),
    ("benchmark_only",
     "Parts of your sessions may become benchmark problems that agents are scored against.",
     true),
    ("ranking_training",
     "Used to train models that rank or grade what an agent produced. Not models that write code.",
     true),
    ("model_training",
     "Your traces become training data for models that write code, potentially including commercial ones.",
     true),
    ("public_attribution",
     "Lists your handle publicly as a contributor. Does not change how any trace is used.",
     false),   // grants_data_use
];
```

- [ ] **Step 4: Implement the remaining methods**

`enroll` and `set_consent_scopes` call the existing `commands::login` path
factored into a non-interactive function. `acknowledge_near_ai_notice` calls
`store.ensure_near_ai_notice_shown()` and clears the health label if it is the
current one. `cancel` moves `Approved -> Pending`, refusing any other state.
`pause` accepts an optional `until`; `status_value` treats an elapsed
`paused_until` as not paused. `list_audit` and `eligibility_reasons` are reads.

- [ ] **Step 5: Remove the `Origin` gate**

Delete the two `Origin::Socket` refusals and the `Origin` enum, its parameter,
and `handle_local`'s distinction. Update the two contract tests that assert
`not_authorized` to assert success instead, and update their comments to state
why the restriction was dropped.

- [ ] **Step 6: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all && git add -A
git commit -m "Add the enrollment, consent, cancel, and timed-pause methods"
```

---

### Task 6: Bump the contract to v1_1

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs`
- Modify: `crates/trace-commons-contributor/tests/daemon_ipc_contract.rs`
- Modify: `docs/contributor-daemon-ipc-v1.md` → rename to `docs/contributor-daemon-ipc-v1_1.md`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn hello_reports_v1_1_and_still_claims_v1_compatibility() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"hello"}"#).await;
    let r = c.recv_json().await;
    assert_eq!(r["result"]["schema_version"], "trace_commons.daemon.v1_1");
    let supported: Vec<String> = r["result"]["supported_versions"].as_array().unwrap()
        .iter().map(|v| v.as_str().unwrap().to_string()).collect();
    assert!(supported.contains(&"trace_commons.daemon.v1".to_string()),
        "a v1 client must still be told it is supported");
}
```

- [ ] **Step 2: Run it, verify it fails**

Run: `cargo test -p trace-commons-contributor --test daemon_ipc_contract hello_reports`
Expected: FAIL.

- [ ] **Step 3: Implement and extend `METHODS`**

`METHODS` gains the eight new names, keeping the array sorted; its length
constant updates. The existing conformance test that compares `hello` against
`METHODS` then covers them automatically.

- [ ] **Step 4: Rewrite the contract document**

Rename to `-v1_1.md`, update `README.md`'s link, add the new methods with
params and results, replace the Authorization section's TTY carve-out with the
accepted-risk note from the spec, document `PreviewSummary`, and record the
health precedence order.

- [ ] **Step 5: Run the full suite**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all && git add -A
git commit -m "Bump the daemon contract to v1_1"
```

---

### Task 7: The C ABI crate

**Files:**
- Create: `crates/trace-commons-contributor-ffi/Cargo.toml`, `src/lib.rs`, `include/trace_commons.h`
- Modify: root `Cargo.toml` (workspace member)
- Test: `crates/trace-commons-contributor-ffi/tests/abi.rs`

**Interfaces:** exactly the C surface in the spec.

- [ ] **Step 1: Create the crate**

```toml
[package]
name = "trace-commons-contributor-ffi"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[lib]
crate-type = ["cdylib", "staticlib", "rlib"]

[dependencies]
trace-commons-contributor = { path = "../trace-commons-contributor" }
anyhow = "1"
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread"] }
```
`rlib` is included so the integration test can link it without loading a
shared object.

- [ ] **Step 2: Write the failing tests**

```rust
#[test]
fn a_call_returns_json_the_caller_owns() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = call(h, "status", "{}");
    assert!(out.contains("\"logged_in\""), "{out}");
    stop(h);
}

#[test]
fn a_second_start_against_the_same_directory_fails_on_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let a = start(dir.path());
    let mut err: *mut c_char = std::ptr::null_mut();
    let b = unsafe { tc_daemon_start(cstr(dir.path()).as_ptr(), &mut err) };
    assert!(b.is_null(), "two daemons must not run against one directory");
    assert!(!err.is_null(), "a failure must set the error out-param");
    unsafe { tc_string_free(err) };
    stop(a);
}

#[test]
fn an_unknown_method_returns_an_error_frame_rather_than_crashing() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = call(h, "no_such_method", "{}");
    assert!(out.contains("unknown_method"), "{out}");
    stop(h);
}

#[test]
fn malformed_params_json_is_an_error_not_a_panic() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let out = call(h, "status", "{not json");
    assert!(out.contains("bad_params"), "{out}");
    stop(h);
}

#[test]
fn repeated_calls_do_not_leak_or_double_free() {
    // Exercises the ownership rule: every char* is freed exactly once.
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    for _ in 0..500 {
        let out = call(h, "status", "{}");
        assert!(!out.is_empty());
    }
    stop(h);
}

#[test]
fn preview_of_an_unknown_entry_sets_the_error_and_returns_null() {
    let dir = tempfile::tempdir().unwrap();
    let h = start(dir.path());
    let mut err: *mut c_char = std::ptr::null_mut();
    let p = unsafe { tc_preview_open(h, cstr_str("00000000-0000-0000-0000-000000000000").as_ptr(), &mut err) };
    assert!(p.is_null());
    assert!(!err.is_null());
    unsafe { tc_string_free(err) };
    stop(h);
}
```
`start`, `stop`, `call`, `cstr`, `cstr_str` are helpers in the test file that
wrap the unsafe calls and free every returned string.

- [ ] **Step 3: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor-ffi`
Expected: FAIL to compile.

- [ ] **Step 4: Implement**

Every exported function body is wrapped:

```rust
fn guard<T>(f: impl FnOnce() -> anyhow::Result<T> + std::panic::UnwindSafe) -> Result<T, String> {
    match std::panic::catch_unwind(f) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(format!("{e:#}")),
        // A Rust panic must never unwind into Swift, C#, or C.
        Err(_) => Err("panic".to_string()),
    }
}
```

`tc_handle` owns a `tokio::runtime::Runtime` plus the `Arc<DaemonShared>` and
the join handle for the loop. `tc_daemon_stop` signals shutdown, joins, and
drops the runtime. Strings out are `CString::into_raw`; `tc_string_free` is
`CString::from_raw` and drop.

- [ ] **Step 5: Write the header by hand and test it compiles**

`include/trace_commons.h` with the declarations from the spec, wrapped in
`#ifdef __cplusplus extern "C" {`. Add a test that runs `cc -fsyntax-only` on
it when a compiler is available, skipping otherwise.

- [ ] **Step 6: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor-ffi`
Expected: PASS, 6 tests.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all && git add -A
git commit -m "Add the C ABI so a native app can host the daemon in-process"
```

---

### Task 8: Full verification

- [ ] **Step 1: Format gate**

Run: `cargo fmt --all -- --check`
Expected: clean.

- [ ] **Step 2: Check with CI flags**

Run: `RUSTFLAGS='-D warnings' cargo check --workspace --bins`
Expected: clean.

- [ ] **Step 3: Build all tests**

Run: `RUSTFLAGS='-D warnings' cargo test --workspace --no-run`
Expected: clean.

- [ ] **Step 4: Clippy**

Run:
```bash
cargo clippy -p trace-commons-contributor -p trace-commons-contributor-ffi --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```
Expected: clean.

- [ ] **Step 5: Full suite against baseline**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor -p trace-commons-contributor-ffi`
Expected: PASS, no regression against the 267-test baseline.

- [ ] **Step 6: Prove no new dependencies**

Run: `git diff origin/main -- Cargo.lock | grep '^+name' | sort -u`
Expected: only `trace-commons-contributor-ffi`, the new workspace member. Any
other package is a violation of the global constraint.

- [ ] **Step 7: Smoke the built binary**

```bash
D=/tmp/tcv11 && rm -rf $D && mkdir -p $D && chmod 700 $D
cargo run -p trace-commons-contributor -- daemon run --dry-run &
# in another shell, against the same TRACE_COMMONS_CONTRIBUTOR_DIR:
printf '{"id":1,"method":"hello"}\n' | nc -U $D/daemon.sock
```
Expected: `schema_version` is `trace_commons.daemon.v1_1` and `methods` has 25
entries. Keep the state directory path short — the socket path has a
104-byte kernel limit.

- [ ] **Step 8: Commit and open the PR**

```bash
git add -A && git commit -m "Verify daemon core v1.1 against CI flags"
gh pr create --repo zmanian/trace-commons-server \
  --title "Contributor background daemon and core v1.1" --body "..."
```

---

## Self-Review

**Spec coverage:** process model → Task 7 (`tc_daemon_start`); C ABI → Task 7;
preview correction → Task 1; preview-is-local → Task 1 Step 5 (socket returns
summary, body is FFI-only); new methods → Task 5; version bump → Task 6;
dropping the terminal gate → Task 5 Step 5; audit → Task 4; label
disambiguation → Task 3; eligibility reasons → Task 5 Step 4; health
precedence → Task 2; testing → every task; dependency constraint → Task 8
Step 6.

**Known deviation from the spec, deliberate:** the spec's preview sketch lists
`residual_risk` as a free string. `RedactionReport` already models risk, so
Task 1 serializes the existing value rather than inventing a parallel one, and
falls back to `"pattern-based"` only when absent.

**Placeholder scan:** no TBD/TODO; every code step carries real code; test
helpers (`sample_cfg`, `shared`, `seed_approved_entry`, `seed_entry_in_state`,
`start`/`stop`/`call`) are each described where introduced.

**Type consistency:** `PreviewSummary` field names are identical in Tasks 1
and 6. `precedence` (Task 2) is used by `HealthState::fail` in the same task.
`disambiguated_label` (Task 3) is called from `watcher::tick` in the same task.
`AuditEntry.action` values `armed-auto-upload` / `bulk-approved` match between
Tasks 4 and 5. `DAEMON_AUDIT_FILE` is defined once in Task 4 and referenced in
Task 5.

**Not covered here, by design:** trace withdrawal (its own spec and plan), the
three shells, and the Windows named pipe.
