# Contributor Background Daemon Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn `trace-commons-contributor` from a one-shot CLI into a background daemon that watches local coding-agent sessions, tells the contributor which traces are uploadable, uploads on approval, auto-uploads opted-in projects, and serves a frozen IPC contract for three future native shells.

**Architecture:** A new `src/daemon/` module tree inside the existing contributor crate. A long-lived `SubmitContext` extracted from `submit_sessions` gives the daemon the exact CLI upload pipeline one session at a time. Pure-function cores (eligibility, policy, queue, rollup) are unit-tested without I/O; the watcher, uploader, history poller, and IPC server compose them in a tokio runtime.

**Tech Stack:** Rust 2024, tokio, serde/serde_json, chrono, uuid v5, anyhow. Spec: `docs/superpowers/specs/2026-08-08-contributor-background-daemon-design.md`.

## Global Constraints

- **Zero new crates.** `tokio` moves `net` from dev-deps to deps and adds `sync` + `signal` — feature additions only. Locking uses `std::fs::File::try_lock_exclusive` (stable 1.89; workspace `rust-version = "1.92"`, `Cargo.toml:13`). No `notify`, no `fs2`, no notifier crate, no `windows-sys`.
- **Verify with CI flags before claiming green:** `RUSTFLAGS='-D warnings' cargo check -p trace-commons-contributor --bins` and `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --no-run`. Plain `cargo check` does not apply `-D warnings`; CI does.
- **Clippy is CI-enforced:** `cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen the allow-list.
- **Hash-only / label-only.** Never put a raw filesystem path, trace content, token, or URL into `receipts.jsonl`, `daemon-history.jsonl`, any log string, or anything server-bound. `path` lives only in `daemon-queue.jsonl` and `daemon-state.json` and is never rendered by a shell — shells get `project_label`.
- **Fail-closed.** A configured-but-unavailable PII filter refuses; it never downgrades to deterministic-only or plaintext.
- **No emojis** in code, commits, or PR text. Commit subjects are short and imperative, no `feat:`/`fix:` prefixes.
- **All state writes atomic**, via the existing `write_atomic_0600` (`src/config.rs:274`). Do not write a second atomic writer.
- **Windows is specified, not implemented.** Unix socket only in v1; the named-pipe ACL needs `windows-sys` and is deferred.

## File Structure

| File | Responsibility |
|---|---|
| `src/submit.rs` (modify) | Add `SubmitContext` + `submit_one`; `submit_sessions` becomes a loop over it |
| `src/daemon/mod.rs` (create) | Module wiring, `DaemonHandle`, `run()` supervisor |
| `src/daemon/settings.rs` (create) | `DaemonSettings` load/save, defaults |
| `src/daemon/state.rs` (create) | `DaemonState`: cwd cache, path→last-upload index, daily counters, digest clock |
| `src/daemon/eligibility.rs` (create) | Pure quiescence + growth-threshold decision |
| `src/daemon/policy.rs` (create) | `ProjectMode`, `ProjectPolicy`, unknown-project bucket |
| `src/daemon/queue.rs` (create) | `QueueEntry`, `QueueState`, transitions, expiry, supersede |
| `src/daemon/health.rs` (create) | Daemon-level health state + `reason_label` taxonomy |
| `src/daemon/uploader.rs` (create) | Re-hash guard, volume caps, retry/backoff, receipts |
| `src/daemon/history.rs` (create) | Status poller, receipt join, cache, rollup |
| `src/daemon/watcher.rs` (create) | Poll loop, cwd caching, feeds eligibility→policy |
| `src/daemon/notify.rs` (create) | Digest batching decision + optional shell-out |
| `src/daemon/ipc.rs` (create) | Framing, request routing, subscribe broadcast, authz |
| `src/daemon/install.rs` (create) | systemd user unit emitter |
| `src/commands.rs` (modify) | `daemon` subcommand handlers |
| `src/bin/trace-commons-contributor.rs` (modify) | `daemon` clap subcommand tree |
| `src/config.rs` (modify) | `wipe()` covers daemon state; daemon file path helpers |
| `docs/contributor-daemon-ipc-v1.md` (create) | The frozen contract |
| `tests/daemon_ipc_contract.rs` (create) | IPC round-trip integration test |
| `tests/daemon_logout_revocation.rs` (create) | Logout-while-running integration test |

---

### Task 1: Extract `SubmitContext` from `submit_sessions`

Pure refactor, no behavior change. Everything downstream depends on it.

**Files:**
- Modify: `crates/trace-commons-contributor/src/submit.rs:159` (`submit_sessions`)
- Test: `crates/trace-commons-contributor/src/submit.rs` (inline `#[cfg(test)]`)

**Interfaces:**
- Consumes: existing `SubmitOptions`, `SubmitOutcome`, `ContributorConfig`, `ConfigStore`.
- Produces:
```rust
pub struct SubmitContext<'a> {
    store: &'a ConfigStore,
    cfg: &'a ContributorConfig,
    effective_cfg: ContributorConfig,
    opts: &'a SubmitOptions,
    device: Option<DeviceIdentity>,
    issuer: IssuerClient,
    claim: Option<ClaimToken>,
    canary_checked_at: Option<DateTime<Utc>>,
    near_ai: Option<NearAiSettings>,
    receipts: Vec<Receipt>,
}

impl<'a> SubmitContext<'a> {
    pub fn new(
        store: &'a ConfigStore,
        cfg: &'a ContributorConfig,
        opts: &'a SubmitOptions,
        near_ai: Option<NearAiSettings>,
    ) -> Result<Self>;

    pub async fn submit_one(
        &mut self,
        source: &dyn TraceSource,
        session_ref: &SessionRef,
    ) -> Result<SubmitOutcome>;

    /// Force the next `submit_one` to re-run the privacy-filter canary.
    pub fn invalidate_canary(&mut self);
    /// Drop the cached claim, e.g. after config changed.
    pub fn invalidate_claim(&mut self);
}
```

- [ ] **Step 1: Write the failing test**

Add to `submit.rs` tests:
```rust
#[tokio::test]
async fn submit_context_reuses_claim_and_canary_across_sessions() {
    // Two dry-run sessions through one context must run the canary once.
    let (_d, store) = test_store();
    let cfg = sample_config_no_filter();
    let opts = SubmitOptions {
        dry_run: true,
        pii_filter: None,
        no_reasoning: false,
        machine_readable: true,
        unenrolled_preview: false,
        remediate_quarantined: false,
    };
    let mut ctx = SubmitContext::new(&store, &cfg, &opts, None).unwrap();
    let (src, a, b) = two_fixture_sessions();
    let first = ctx.submit_one(src.as_ref(), &a).await.unwrap();
    let second = ctx.submit_one(src.as_ref(), &b).await.unwrap();
    assert!(matches!(first, SubmitOutcome::Submitted { .. }));
    assert!(matches!(second, SubmitOutcome::Submitted { .. }));
    assert_eq!(ctx.canary_runs(), 1, "canary must not re-run per session");
}
```
Add `#[cfg(test)] pub fn canary_runs(&self) -> u32` backed by a counter field.

- [ ] **Step 2: Run test, verify it fails**

Run: `cargo test -p trace-commons-contributor submit_context_reuses -- --nocapture`
Expected: FAIL, `SubmitContext` not found.

- [ ] **Step 3: Implement the extraction**

Move the body of the `for (source, session_ref) in sessions` loop (`submit.rs:~200-400`) into `submit_one`, replacing every `continue` with `return Ok(outcome)` and every `outcomes.push(x); continue;` with `return Ok(x)`. Hoisted state (`device`, `issuer`, `claim`, `canary_checked`, `receipts`, `effective_cfg`) becomes struct fields. Replace the hardcoded `near_ai_settings_from_env()` call at the `build_redactor_with` site with `self.near_ai.clone()`.

- [ ] **Step 4: Rewrite `submit_sessions` as a loop over the context**

```rust
pub async fn submit_sessions(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    sessions: Vec<(Box<dyn TraceSource>, SessionRef)>,
    opts: &SubmitOptions,
) -> Result<Vec<SubmitOutcome>> {
    if opts.unenrolled_preview && !opts.dry_run {
        anyhow::bail!("unenrolled preview requires dry-run");
    }
    let mut ctx = SubmitContext::new(store, cfg, opts, near_ai_settings_from_env())?;
    let mut outcomes = Vec::with_capacity(sessions.len());
    for (source, session_ref) in sessions {
        outcomes.push(ctx.submit_one(source.as_ref(), &session_ref).await?);
    }
    Ok(outcomes)
}
```
Note: the canary previously aborted the whole batch via `?`. Preserve that here — `submit_one` returns `Err` on canary failure and the `?` propagates, so CLI behavior is unchanged.

- [ ] **Step 5: Run the full existing suite to prove no regression**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor`
Expected: PASS, same count as the pre-change baseline. Capture the baseline first with `git stash` if not already recorded.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/submit.rs
git commit -m "Extract SubmitContext so one pipeline serves CLI and daemon"
```

---

### Task 2: Daemon settings and state stores

**Files:**
- Create: `src/daemon/mod.rs`, `src/daemon/settings.rs`, `src/daemon/state.rs`
- Modify: `src/lib.rs` (add `pub mod daemon;`), `src/config.rs` (path helpers, extend `wipe()`)
- Modify: `crates/trace-commons-contributor/Cargo.toml` (tokio features)

**Interfaces:**
- Produces:
```rust
// settings.rs
pub const DAEMON_SETTINGS_SCHEMA: &str = "trace_commons.daemon_settings.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSettings {
    pub schema_version: String,
    pub poll_interval_secs: u64,        // 60
    pub quiescence_secs: u64,           // 1800
    pub digest_interval_secs: u64,      // 14400
    pub queue_ttl_days: i64,            // 14
    pub growth_factor: f64,             // 2.0
    pub growth_min_new_bytes: u64,      // 65536
    pub max_reuploads: u32,             // 3
    pub max_uploads_per_day: u32,       // 50
    pub max_bytes_per_day: u64,         // 209_715_200
    pub max_queue_entries: usize,       // 500
    pub history_poll_secs: u64,         // 1800
    pub canary_interval_secs: u64,      // 3600
    pub local_notifications: bool,      // false
    pub near_ai: Option<NearAiSettings>,
}
impl Default for DaemonSettings { /* the values above */ }
impl DaemonSettings {
    pub fn load(store: &ConfigStore) -> Result<Self>;   // default when absent
    pub fn save(&self, store: &ConfigStore) -> Result<()>;
}

// state.rs
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PriorUpload { pub hash: String, pub size_bytes: u64, pub upload_count: u32 }

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CwdCacheEntry { pub size_bytes: u64, pub modified_at: DateTime<Utc>, pub cwd: Option<String> }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub schema_version: String,          // "trace_commons.daemon_state.v1"
    pub cwd_cache: BTreeMap<String, CwdCacheEntry>,     // key: path string
    pub prior_uploads: BTreeMap<String, PriorUpload>,   // key: path string
    pub last_observation: BTreeMap<String, u64>,        // key: path, value: size at previous poll
    pub last_digest_at: Option<DateTime<Utc>>,
    pub day_bucket: Option<String>,      // "YYYY-MM-DD" UTC
    pub uploads_today: u32,
    pub bytes_today: u64,
}
impl DaemonState {
    pub fn load(store: &ConfigStore) -> Result<Self>;
    pub fn save(&self, store: &ConfigStore) -> Result<()>;
    /// Reset counters when the UTC day rolls over. Call before every cap check.
    pub fn roll_day(&mut self, now: DateTime<Utc>);
    pub fn record_upload(&mut self, path: &Path, hash: &str, size_bytes: u64, now: DateTime<Utc>);
}
```

- [ ] **Step 1: Add tokio features**

In `crates/trace-commons-contributor/Cargo.toml`, change line 24 to:
```toml
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time", "fs", "io-util", "net", "sync", "signal"] }
```
Leave the dev-dependency `tokio` entry as-is.

- [ ] **Step 2: Write the failing tests**

In `src/daemon/state.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_day_resets_counters_on_utc_day_change() {
        let mut s = DaemonState::new();
        s.day_bucket = Some("2026-08-07".to_string());
        s.uploads_today = 9;
        s.bytes_today = 1234;
        s.roll_day("2026-08-08T00:00:01Z".parse().unwrap());
        assert_eq!(s.uploads_today, 0);
        assert_eq!(s.bytes_today, 0);
        assert_eq!(s.day_bucket.as_deref(), Some("2026-08-08"));
    }

    #[test]
    fn roll_day_preserves_counters_within_the_same_day() {
        let mut s = DaemonState::new();
        s.roll_day("2026-08-08T01:00:00Z".parse().unwrap());
        s.uploads_today = 3;
        s.roll_day("2026-08-08T23:59:00Z".parse().unwrap());
        assert_eq!(s.uploads_today, 3);
    }

    #[test]
    fn record_upload_increments_count_for_the_same_path() {
        let mut s = DaemonState::new();
        let now = "2026-08-08T01:00:00Z".parse().unwrap();
        s.record_upload(Path::new("/tmp/a.jsonl"), "sha256:aa", 10, now);
        s.record_upload(Path::new("/tmp/a.jsonl"), "sha256:bb", 30, now);
        let prior = s.prior_uploads.get("/tmp/a.jsonl").unwrap();
        assert_eq!(prior.upload_count, 2);
        assert_eq!(prior.hash, "sha256:bb");
        assert_eq!(prior.size_bytes, 30);
    }
}
```
In `src/daemon/settings.rs`:
```rust
#[test]
fn settings_round_trip_through_the_store() {
    let (_d, store) = crate::config::tests_support::temp_store();
    let mut s = DaemonSettings::default();
    s.quiescence_secs = 60;
    s.save(&store).unwrap();
    assert_eq!(DaemonSettings::load(&store).unwrap().quiescence_secs, 60);
}

#[test]
fn settings_default_when_file_absent() {
    let (_d, store) = crate::config::tests_support::temp_store();
    assert_eq!(DaemonSettings::load(&store).unwrap().quiescence_secs, 1800);
}
```
Add `pub(crate) mod tests_support` in `config.rs` exposing `temp_store() -> (TempDir, ConfigStore)` if no equivalent helper is already public to sibling modules.

- [ ] **Step 3: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor daemon::`
Expected: FAIL, module not found.

- [ ] **Step 4: Implement settings.rs and state.rs**

Both use the existing atomic writer. In `config.rs`, add:
```rust
pub const DAEMON_SETTINGS_FILE: &str = "daemon-settings.json";
pub const DAEMON_STATE_FILE: &str = "daemon-state.json";
pub const DAEMON_PROJECTS_FILE: &str = "daemon-projects.json";
pub const DAEMON_QUEUE_FILE: &str = "daemon-queue.jsonl";
pub const DAEMON_HISTORY_FILE: &str = "daemon-history.jsonl";
pub const DAEMON_SOCK_FILE: &str = "daemon.sock";
pub const DAEMON_LOCK_FILE: &str = "daemon.lock";

impl ConfigStore {
    pub fn daemon_path(&self, name: &str) -> PathBuf { self.dir.join(name) }
    /// Atomic 0600 write of an arbitrary daemon state file.
    pub fn write_daemon_file(&self, name: &str, body: &[u8]) -> Result<()>;
    pub fn read_daemon_file(&self, name: &str) -> Result<Option<Vec<u8>>>;
}
```

- [ ] **Step 5: Extend `wipe()` to cover daemon state**

In `config.rs:231`, add all five daemon data files to the deletion list (settings, state, projects, queue, history) and add their temp prefixes to `tmp_prefixes` at `config.rs:244`. The socket and lock are handled by Task 11.

Add the regression test:
```rust
#[test]
fn wipe_removes_daemon_state() {
    let (_d, store) = store();
    for name in [DAEMON_SETTINGS_FILE, DAEMON_STATE_FILE, DAEMON_PROJECTS_FILE,
                 DAEMON_QUEUE_FILE, DAEMON_HISTORY_FILE] {
        store.write_daemon_file(name, b"{}").unwrap();
    }
    store.wipe().unwrap();
    for name in [DAEMON_SETTINGS_FILE, DAEMON_STATE_FILE, DAEMON_PROJECTS_FILE,
                 DAEMON_QUEUE_FILE, DAEMON_HISTORY_FILE] {
        assert!(store.read_daemon_file(name).unwrap().is_none(), "{name} survived logout");
    }
}
```

- [ ] **Step 6: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor daemon:: config::`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor/Cargo.toml crates/trace-commons-contributor/src/
git commit -m "Add daemon settings and state stores, wiped on logout"
```

---

### Task 3: Eligibility

Pure functions, no I/O.

**Files:**
- Create: `src/daemon/eligibility.rs`

**Interfaces:**
- Consumes: `DaemonSettings` (Task 2), `PriorUpload` (Task 2).
- Produces:
```rust
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub modified_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eligibility {
    Eligible,
    NotQuiescent,
    Unstable,             // size changed since the previous poll
    AlreadyUploaded,      // same size as last upload, no growth
    GrowthBelowThreshold,
    ReuploadCapReached,
}

pub fn evaluate(
    obs: &Observation,
    previous_size: Option<u64>,
    prior: Option<&PriorUpload>,
    now: DateTime<Utc>,
    settings: &DaemonSettings,
) -> Eligibility;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> DateTime<Utc> { s.parse().unwrap() }
    fn obs(size: u64, modified: &str) -> Observation {
        Observation { path: PathBuf::from("/tmp/s.jsonl"), size_bytes: size, modified_at: at(modified) }
    }

    #[test]
    fn not_quiescent_until_the_window_elapses() {
        let s = DaemonSettings::default(); // 1800s
        let o = obs(100, "2026-08-08T12:00:00Z");
        assert_eq!(evaluate(&o, Some(100), None, at("2026-08-08T12:20:00Z"), &s), Eligibility::NotQuiescent);
    }

    #[test]
    fn eligible_after_window_with_stable_size() {
        let s = DaemonSettings::default();
        let o = obs(100, "2026-08-08T12:00:00Z");
        assert_eq!(evaluate(&o, Some(100), None, at("2026-08-08T12:31:00Z"), &s), Eligibility::Eligible);
    }

    #[test]
    fn unstable_when_size_changed_since_previous_poll() {
        // mtime can be stale on some filesystems; size stability is the second gate.
        let s = DaemonSettings::default();
        let o = obs(200, "2026-08-08T12:00:00Z");
        assert_eq!(evaluate(&o, Some(100), None, at("2026-08-08T12:31:00Z"), &s), Eligibility::Unstable);
    }

    #[test]
    fn first_sighting_is_never_eligible() {
        // No previous observation means we cannot assert size stability yet.
        let s = DaemonSettings::default();
        let o = obs(100, "2026-08-08T12:00:00Z");
        assert_eq!(evaluate(&o, None, None, at("2026-08-08T12:31:00Z"), &s), Eligibility::Unstable);
    }

    #[test]
    fn already_uploaded_at_the_same_size_is_not_requeued() {
        let s = DaemonSettings::default();
        let o = obs(100, "2026-08-08T12:00:00Z");
        let prior = PriorUpload { hash: "sha256:aa".into(), size_bytes: 100, upload_count: 1 };
        assert_eq!(evaluate(&o, Some(100), Some(&prior), at("2026-08-08T12:31:00Z"), &s), Eligibility::AlreadyUploaded);
    }

    #[test]
    fn growth_below_both_thresholds_is_rejected() {
        // 100 -> 150 is neither 2x nor +64KiB.
        let s = DaemonSettings::default();
        let o = obs(150, "2026-08-08T12:00:00Z");
        let prior = PriorUpload { hash: "sha256:aa".into(), size_bytes: 100, upload_count: 1 };
        assert_eq!(evaluate(&o, Some(150), Some(&prior), at("2026-08-08T12:31:00Z"), &s), Eligibility::GrowthBelowThreshold);
    }

    #[test]
    fn doubling_in_size_requeues() {
        let s = DaemonSettings::default();
        let o = obs(200, "2026-08-08T12:00:00Z");
        let prior = PriorUpload { hash: "sha256:aa".into(), size_bytes: 100, upload_count: 1 };
        assert_eq!(evaluate(&o, Some(200), Some(&prior), at("2026-08-08T12:31:00Z"), &s), Eligibility::Eligible);
    }

    #[test]
    fn large_absolute_growth_requeues_without_doubling() {
        let s = DaemonSettings::default();
        let o = obs(1_000_000 + 65_536, "2026-08-08T12:00:00Z");
        let prior = PriorUpload { hash: "sha256:aa".into(), size_bytes: 1_000_000, upload_count: 1 };
        assert_eq!(evaluate(&o, Some(1_065_536), Some(&prior), at("2026-08-08T12:31:00Z"), &s), Eligibility::Eligible);
    }

    #[test]
    fn reupload_cap_stops_a_long_running_session() {
        let s = DaemonSettings::default(); // max_reuploads = 3
        let o = obs(1000, "2026-08-08T12:00:00Z");
        let prior = PriorUpload { hash: "sha256:aa".into(), size_bytes: 100, upload_count: 3 };
        assert_eq!(evaluate(&o, Some(1000), Some(&prior), at("2026-08-08T12:31:00Z"), &s), Eligibility::ReuploadCapReached);
    }

    #[test]
    fn a_shrinking_file_is_never_eligible() {
        // Truncation or rotation: treat as not-uploadable rather than as growth.
        let s = DaemonSettings::default();
        let o = obs(50, "2026-08-08T12:00:00Z");
        let prior = PriorUpload { hash: "sha256:aa".into(), size_bytes: 100, upload_count: 1 };
        assert_eq!(evaluate(&o, Some(50), Some(&prior), at("2026-08-08T12:31:00Z"), &s), Eligibility::GrowthBelowThreshold);
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor eligibility`
Expected: FAIL, `evaluate` not found.

- [ ] **Step 3: Implement `evaluate`**

Order of checks matters and is asserted by the tests: quiescence → size stability → prior-upload growth → cap.
```rust
pub fn evaluate(
    obs: &Observation,
    previous_size: Option<u64>,
    prior: Option<&PriorUpload>,
    now: DateTime<Utc>,
    settings: &DaemonSettings,
) -> Eligibility {
    let quiet_for = now.signed_duration_since(obs.modified_at);
    if quiet_for < Duration::seconds(settings.quiescence_secs as i64) {
        return Eligibility::NotQuiescent;
    }
    if previous_size != Some(obs.size_bytes) {
        return Eligibility::Unstable;
    }
    let Some(prior) = prior else { return Eligibility::Eligible };
    if obs.size_bytes == prior.size_bytes {
        return Eligibility::AlreadyUploaded;
    }
    let doubled = obs.size_bytes >= prior.size_bytes.saturating_mul(2);
    let grew_a_lot = obs.size_bytes.saturating_sub(prior.size_bytes) >= settings.growth_min_new_bytes;
    if !(doubled || grew_a_lot) {
        return Eligibility::GrowthBelowThreshold;
    }
    if prior.upload_count >= settings.max_reuploads {
        return Eligibility::ReuploadCapReached;
    }
    Eligibility::Eligible
}
```
Note `growth_factor` is stored in settings for future tuning but the default path uses the integer doubling check; if `growth_factor != 2.0`, compute `(prior.size_bytes as f64 * settings.growth_factor) as u64` instead. Implement that branch so the setting is not dead.

- [ ] **Step 4: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor eligibility`
Expected: PASS, 10 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/eligibility.rs crates/trace-commons-contributor/src/daemon/mod.rs
git commit -m "Add quiescence and bounded-growth eligibility"
```

---

### Task 4: Project policy

**Files:**
- Create: `src/daemon/policy.rs`

**Interfaces:**
- Produces:
```rust
pub const UNKNOWN_PROJECT_KEY: &str = "unknown-project";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMode { AutoUpload, NotifyOnly, Ignore }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry { pub mode: ProjectMode, pub added_at: DateTime<Utc>, pub label: String }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPolicy {
    pub schema_version: String, // "trace_commons.daemon_projects.v1"
    pub projects: BTreeMap<String, ProjectEntry>,
}

impl ProjectPolicy {
    pub fn load(store: &ConfigStore) -> Result<Self>;
    pub fn save(&self, store: &ConfigStore) -> Result<()>;
    /// Unknown projects default to NotifyOnly; the unknown-cwd bucket is always NotifyOnly.
    pub fn resolve(&self, project_key: &str) -> ProjectMode;
    /// Errors when setting AutoUpload on the unknown-cwd bucket.
    pub fn set_mode(&mut self, project_key: &str, label: &str, mode: ProjectMode, now: DateTime<Utc>) -> Result<()>;
}

/// The policy key for a session: its true cwd, or the unknown bucket.
/// Never falls back to the basename heuristic.
pub fn project_key_for(cwd: Option<&str>) -> String;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn now() -> DateTime<Utc> { "2026-08-08T12:00:00Z".parse().unwrap() }

    #[test]
    fn unknown_project_defaults_to_notify_only() {
        let p = ProjectPolicy::new();
        assert_eq!(p.resolve("/Users/z/code/never-seen"), ProjectMode::NotifyOnly);
    }

    #[test]
    fn sessions_without_cwd_land_in_the_unknown_bucket() {
        assert_eq!(project_key_for(None), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("/Users/z/code/proj")), "/Users/z/code/proj");
    }

    #[test]
    fn unknown_bucket_cannot_be_set_to_auto_upload() {
        let mut p = ProjectPolicy::new();
        let err = p.set_mode(UNKNOWN_PROJECT_KEY, "unknown", ProjectMode::AutoUpload, now()).unwrap_err();
        assert!(err.to_string().contains("unknown-project"));
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn unknown_bucket_stays_notify_only_even_if_a_hand_edited_file_says_auto() {
        // A tampered or hand-edited projects file must not grant autonomy.
        let mut p = ProjectPolicy::new();
        p.projects.insert(UNKNOWN_PROJECT_KEY.to_string(), ProjectEntry {
            mode: ProjectMode::AutoUpload, added_at: now(), label: "unknown".into(),
        });
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn set_and_resolve_round_trip_for_a_real_project() {
        let mut p = ProjectPolicy::new();
        p.set_mode("/Users/z/code/proj", "proj", ProjectMode::AutoUpload, now()).unwrap();
        assert_eq!(p.resolve("/Users/z/code/proj"), ProjectMode::AutoUpload);
        p.set_mode("/Users/z/code/proj", "proj", ProjectMode::Ignore, now()).unwrap();
        assert_eq!(p.resolve("/Users/z/code/proj"), ProjectMode::Ignore);
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor policy`
Expected: FAIL.

- [ ] **Step 3: Implement**

`resolve` checks `project_key == UNKNOWN_PROJECT_KEY` **before** consulting the map, so a hand-edited file cannot grant autonomy. `set_mode` rejects the same combination with `anyhow!("unknown-project sessions cannot be set to auto_upload")`.

- [ ] **Step 4: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor policy`
Expected: PASS, 5 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/policy.rs
git commit -m "Add per-project opt-in policy with a locked unknown-cwd bucket"
```

---

### Task 5: Queue

**Files:**
- Create: `src/daemon/queue.rs`

**Interfaces:**
- Consumes: `DaemonSettings`, `health::HealthState` (Task 6 defines the type; for this task accept a `bool blocked_on_health` parameter and Task 6 wires the real one).
- Produces:
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueState { Pending, Approved, Uploading, Uploaded, Refused, Failed, Expired, Superseded }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueEntry {
    pub entry_id: Uuid,
    pub session_hash: String,
    pub source: String,
    pub project_key: String,
    pub project_label: String,
    pub path: PathBuf,            // local-only, never leaves this file
    pub size_bytes: u64,
    pub discovered_at: DateTime<Utc>,
    pub state: QueueState,
    pub reason_label: Option<String>,
    pub attempts: u32,
    pub retry_after: Option<DateTime<Utc>>,
    pub submission_id: Option<Uuid>,
}

pub fn entry_id_for(session_hash: &str) -> Uuid; // v5, NAMESPACE_OID, stable across restarts

pub struct Queue { entries: Vec<QueueEntry> }

impl Queue {
    pub fn load(store: &ConfigStore) -> Result<Self>;
    pub fn save(&self, store: &ConfigStore) -> Result<()>;
    pub fn pending(&self) -> Vec<&QueueEntry>;
    pub fn get(&self, entry_id: Uuid) -> Option<&QueueEntry>;
    /// Idempotent: re-adding an existing session_hash does not duplicate.
    pub fn upsert(&mut self, entry: QueueEntry, max_entries: usize) -> Result<()>;
    pub fn set_state(&mut self, entry_id: Uuid, state: QueueState, reason_label: Option<String>);
    /// Mark superseded and return a fresh pending entry at the new hash.
    pub fn supersede(&mut self, entry_id: Uuid, new_hash: &str, new_size: u64, now: DateTime<Utc>) -> Option<QueueEntry>;
    /// Expire pending entries past TTL. No-op while blocked_on_health is true.
    pub fn expire(&mut self, now: DateTime<Utc>, ttl_days: i64, blocked_on_health: bool) -> usize;
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    fn at(s: &str) -> DateTime<Utc> { s.parse().unwrap() }
    fn entry(hash: &str, discovered: &str) -> QueueEntry {
        QueueEntry {
            entry_id: entry_id_for(hash),
            session_hash: hash.into(),
            source: "claude-code".into(),
            project_key: "/Users/z/code/proj".into(),
            project_label: "proj".into(),
            path: PathBuf::from("/Users/z/.claude/projects/x/s.jsonl"),
            size_bytes: 100,
            discovered_at: at(discovered),
            state: QueueState::Pending,
            reason_label: None,
            attempts: 0,
            retry_after: None,
            submission_id: None,
        }
    }

    #[test]
    fn entry_id_is_stable_for_a_session_hash() {
        assert_eq!(entry_id_for("sha256:aa"), entry_id_for("sha256:aa"));
        assert_ne!(entry_id_for("sha256:aa"), entry_id_for("sha256:bb"));
    }

    #[test]
    fn upsert_is_idempotent_on_session_hash() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500).unwrap();
        q.upsert(entry("sha256:aa", "2026-08-08T13:00:00Z"), 500).unwrap();
        assert_eq!(q.pending().len(), 1);
    }

    #[test]
    fn upsert_refuses_past_the_queue_cap() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 1).unwrap();
        assert!(q.upsert(entry("sha256:bb", "2026-08-08T12:00:00Z"), 1).is_err());
    }

    #[test]
    fn pending_entries_expire_after_the_ttl() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-07-01T12:00:00Z"), 500).unwrap();
        assert_eq!(q.expire(at("2026-08-08T12:00:00Z"), 14, false), 1);
        assert_eq!(q.get(entry_id_for("sha256:aa")).unwrap().state, QueueState::Expired);
    }

    #[test]
    fn expiry_is_suspended_while_blocked_on_health() {
        // A PII-filter outage must not silently drop two weeks of traces.
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-07-01T12:00:00Z"), 500).unwrap();
        assert_eq!(q.expire(at("2026-08-08T12:00:00Z"), 14, true), 0);
        assert_eq!(q.get(entry_id_for("sha256:aa")).unwrap().state, QueueState::Pending);
    }

    #[test]
    fn uploaded_entries_are_never_expired() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-07-01T12:00:00Z"), 500).unwrap();
        q.set_state(entry_id_for("sha256:aa"), QueueState::Uploaded, None);
        assert_eq!(q.expire(at("2026-08-08T12:00:00Z"), 14, false), 0);
    }

    #[test]
    fn supersede_marks_the_old_entry_and_returns_a_fresh_pending_one() {
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500).unwrap();
        let fresh = q.supersede(entry_id_for("sha256:aa"), "sha256:bb", 900, at("2026-08-08T16:00:00Z")).unwrap();
        assert_eq!(q.get(entry_id_for("sha256:aa")).unwrap().state, QueueState::Superseded);
        assert_eq!(fresh.session_hash, "sha256:bb");
        assert_eq!(fresh.size_bytes, 900);
        assert_eq!(fresh.state, QueueState::Pending);
        assert_eq!(fresh.entry_id, entry_id_for("sha256:bb"));
    }

    #[test]
    fn queue_round_trips_through_the_store() {
        let (_d, store) = crate::config::tests_support::temp_store();
        let mut q = Queue::new();
        q.upsert(entry("sha256:aa", "2026-08-08T12:00:00Z"), 500).unwrap();
        q.save(&store).unwrap();
        assert_eq!(Queue::load(&store).unwrap().pending().len(), 1);
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor queue`
Expected: FAIL.

- [ ] **Step 3: Implement**

`expire` only touches `QueueState::Pending`. Serialization is one JSON object per line (`daemon-queue.jsonl`), written whole via the atomic writer on each `save`.

- [ ] **Step 4: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor queue`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/queue.rs
git commit -m "Add the durable pending queue with health-suspended expiry"
```

---

### Task 6: Health state and the uploader

**Files:**
- Create: `src/daemon/health.rs`, `src/daemon/uploader.rs`

**Interfaces:**
- Consumes: `SubmitContext` (Task 1), `Queue` (Task 5), `DaemonState`/`DaemonSettings` (Task 2).
- Produces:
```rust
// health.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthState {
    pub last_error_label: Option<String>,   // label only, never a message body
    pub since: Option<DateTime<Utc>>,
}
impl HealthState {
    pub fn ok(&self) -> bool;
    pub fn fail(&mut self, label: &str, now: DateTime<Utc>);
    pub fn clear(&mut self);
    /// Labels that suspend queue expiry: the contributor cannot act on these.
    pub fn blocks_expiry(&self) -> bool;
}
pub const LABEL_NOT_LOGGED_IN: &str = "not-logged-in";
pub const LABEL_PII_FILTER_UNAVAILABLE: &str = "pii-filter-unavailable";
pub const LABEL_CLAIM_MINT_FAILED: &str = "claim-mint-failed";
pub const LABEL_INGEST_UNREACHABLE: &str = "ingest-unreachable";
pub const LABEL_DAILY_CAP_REACHED: &str = "daily-cap-reached";
pub const LABEL_NEAR_AI_NOTICE_PENDING: &str = "near-ai-notice-not-acknowledged";

// uploader.rs
#[derive(Debug, PartialEq)]
pub enum UploadDecision {
    Uploaded { submission_id: Uuid },
    Superseded { new_hash: String },
    Refused { reason_label: String },
    Failed { reason_label: String },
    CapReached,
}

pub struct Uploader<'a> { /* ctx, settings, state, health */ }

impl<'a> Uploader<'a> {
    /// Re-hash before upload; refuse to ship content the user did not approve.
    pub async fn upload_entry(
        &mut self,
        entry: &QueueEntry,
        now: DateTime<Utc>,
    ) -> Result<UploadDecision>;
}

/// Pure cap check, unit-tested without I/O.
pub fn cap_check(state: &DaemonState, size_bytes: u64, settings: &DaemonSettings) -> bool;
```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_check_rejects_past_the_daily_upload_count() {
        let mut st = DaemonState::new();
        st.uploads_today = 50;
        assert!(!cap_check(&st, 10, &DaemonSettings::default()));
    }

    #[test]
    fn cap_check_rejects_past_the_daily_byte_budget() {
        let mut st = DaemonState::new();
        st.bytes_today = 209_715_200;
        assert!(!cap_check(&st, 1, &DaemonSettings::default()));
    }

    #[test]
    fn cap_check_allows_a_normal_upload() {
        assert!(cap_check(&DaemonState::new(), 1024, &DaemonSettings::default()));
    }

    #[test]
    fn health_labels_that_the_user_cannot_act_on_suspend_expiry() {
        let mut h = HealthState::default();
        h.fail(LABEL_PII_FILTER_UNAVAILABLE, "2026-08-08T12:00:00Z".parse().unwrap());
        assert!(h.blocks_expiry());
        h.clear();
        assert!(!h.blocks_expiry());
    }

    #[tokio::test]
    async fn upload_refuses_when_the_session_grew_after_approval() {
        // The central consent property: approve 42 KB, never ship 900 KB.
        let fixture = GrowingSessionFixture::new();
        let entry = fixture.entry_at_original_hash();
        fixture.append_more_events();
        let mut up = fixture.uploader();
        let decision = up.upload_entry(&entry, fixture.now()).await.unwrap();
        match decision {
            UploadDecision::Superseded { new_hash } => assert_ne!(new_hash, entry.session_hash),
            other => panic!("expected Superseded, got {other:?}"),
        }
        assert_eq!(fixture.uploads_attempted(), 0, "nothing may be uploaded on mismatch");
    }

    #[tokio::test]
    async fn upload_proceeds_when_the_hash_still_matches() {
        let fixture = GrowingSessionFixture::new();
        let entry = fixture.entry_at_original_hash();
        let mut up = fixture.uploader();
        assert!(matches!(
            up.upload_entry(&entry, fixture.now()).await.unwrap(),
            UploadDecision::Uploaded { .. }
        ));
    }
}
```
`GrowingSessionFixture` writes a claude-code JSONL session into a tempdir, builds a `SubmitContext` in `dry_run` mode against a temp `ConfigStore`, and exposes `append_more_events()` which appends a valid event line so the file hash changes. Put it in the same test module.

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor uploader`
Expected: FAIL.

- [ ] **Step 3: Implement `health.rs`**

`blocks_expiry` returns true for `LABEL_PII_FILTER_UNAVAILABLE`, `LABEL_NOT_LOGGED_IN`, `LABEL_CLAIM_MINT_FAILED`, `LABEL_INGEST_UNREACHABLE`, `LABEL_DAILY_CAP_REACHED`, `LABEL_NEAR_AI_NOTICE_PENDING`.

- [ ] **Step 4: Implement `upload_entry`**

Order: cap check → reload the session via the source adapter → compare `transcript.session_hash` against `entry.session_hash` → on mismatch return `Superseded` **without uploading** → otherwise `ctx.submit_one(...)` → map `SubmitOutcome` onto `UploadDecision` → on success `state.record_upload(...)` and bump daily counters.

- [ ] **Step 5: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor uploader health`
Expected: PASS, 6 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/health.rs crates/trace-commons-contributor/src/daemon/uploader.rs
git commit -m "Add the uploader with a re-hash consent guard and volume caps"
```

---

### Task 7: History cache and rollup

**Files:**
- Create: `src/daemon/history.rs`

**Interfaces:**
- Consumes: `submit::status` (`src/submit.rs:404`), `ConfigStore::load_receipts`.
- Produces:
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryRecord {
    pub submission_id: Uuid,
    pub submitted_at: DateTime<Utc>,
    pub project_label: String,
    pub source: String,
    pub session_hash: String,
    pub status: String,
    pub consent_scopes: Vec<String>,
    pub credit_points_pending: f64,
    pub credit_points_final: Option<f64>,
    pub explanations: Vec<String>,
    pub last_refreshed_at: Option<DateTime<Utc>>,
}
// NOTE: deliberately no `path` field. History is the surface most likely to be shared.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HistoryRollup {
    pub week: HistoryCounts,
    pub month: HistoryCounts,
    pub all_time: HistoryCounts,
    pub credit_pending: f64,
    pub credit_final: f64,
    pub quarantined: u32,
    pub last_refreshed_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HistoryCounts { pub submitted: u32, pub accepted: u32, pub quarantined: u32, pub other: u32 }

/// Pure join: local receipts + server updates -> history records.
pub fn join(
    receipts: &[Receipt],
    updates: &[TraceSubmissionStatusUpdate],
    labels: &BTreeMap<Uuid, String>,   // submission_id -> project_label, from the queue
    refreshed_at: DateTime<Utc>,
) -> Vec<HistoryRecord>;

pub fn rollup(records: &[HistoryRecord], now: DateTime<Utc>) -> HistoryRollup;

pub struct HistoryCache;
impl HistoryCache {
    pub fn load(store: &ConfigStore) -> Result<Vec<HistoryRecord>>;
    pub fn save(store: &ConfigStore, records: &[HistoryRecord]) -> Result<()>;
}
```

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_carries_no_local_path_and_prefers_server_status() {
        let receipts = vec![receipt("sha256:aa", "submitted")];
        let updates = vec![update(receipts[0].submission_id, "accepted", 1.5, Some(2.0))];
        let recs = join(&receipts, &updates, &labels(), at("2026-08-08T12:00:00Z"));
        assert_eq!(recs[0].status, "accepted");
        assert_eq!(recs[0].credit_points_final, Some(2.0));
        let json = serde_json::to_string(&recs[0]).unwrap();
        assert!(!json.contains("path"), "history must never carry a local path");
    }

    #[test]
    fn join_falls_back_to_the_receipt_when_the_server_has_no_update() {
        let receipts = vec![receipt("sha256:aa", "submitted")];
        let recs = join(&receipts, &[], &labels(), at("2026-08-08T12:00:00Z"));
        assert_eq!(recs[0].status, "submitted");
        assert_eq!(recs[0].credit_points_final, None);
    }

    #[test]
    fn rollup_counts_quarantined_separately_from_failures() {
        // Quarantine means held for operator privacy review, not rejected.
        let recs = vec![
            record("accepted", "2026-08-08T10:00:00Z"),
            record("quarantined", "2026-08-08T10:00:00Z"),
            record("quarantined", "2026-08-08T10:00:00Z"),
        ];
        let r = rollup(&recs, at("2026-08-08T12:00:00Z"));
        assert_eq!(r.quarantined, 2);
        assert_eq!(r.all_time.accepted, 1);
        assert_eq!(r.all_time.quarantined, 2);
    }

    #[test]
    fn rollup_windows_split_week_month_and_all_time() {
        let recs = vec![
            record("accepted", "2026-08-07T10:00:00Z"),  // this week
            record("accepted", "2026-07-20T10:00:00Z"),  // this month, not this week
            record("accepted", "2026-01-01T10:00:00Z"),  // all time only
        ];
        let r = rollup(&recs, at("2026-08-08T12:00:00Z"));
        assert_eq!(r.week.accepted, 1);
        assert_eq!(r.month.accepted, 2);
        assert_eq!(r.all_time.accepted, 3);
    }

    #[test]
    fn rollup_sums_pending_and_final_credit_separately() {
        let mut a = record("accepted", "2026-08-08T10:00:00Z");
        a.credit_points_pending = 1.5;
        a.credit_points_final = Some(2.0);
        let mut b = record("submitted", "2026-08-08T10:00:00Z");
        b.credit_points_pending = 0.5;
        b.credit_points_final = None;
        let r = rollup(&[a, b], at("2026-08-08T12:00:00Z"));
        assert_eq!(r.credit_pending, 2.0);
        assert_eq!(r.credit_final, 2.0);
    }

    #[test]
    fn cache_round_trips_and_reports_staleness() {
        let (_d, store) = crate::config::tests_support::temp_store();
        let recs = vec![record("accepted", "2026-08-08T10:00:00Z")];
        HistoryCache::save(&store, &recs).unwrap();
        let loaded = HistoryCache::load(&store).unwrap();
        assert_eq!(loaded[0].last_refreshed_at, recs[0].last_refreshed_at);
    }
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor history`
Expected: FAIL.

- [ ] **Step 3: Implement `join`, `rollup`, `HistoryCache`**

`join` indexes updates by `submission_id` and overlays them on receipts; a receipt with no update keeps its local status and `None` final credit. `rollup` uses `now - 7d` and `now - 30d` windows.

- [ ] **Step 4: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor history`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/history.rs
git commit -m "Add contribution history join, rollup, and offline cache"
```

---

### Task 8: IPC server

**Files:**
- Create: `src/daemon/ipc.rs`
- Test: `tests/daemon_ipc_contract.rs`

**Interfaces:**
- Produces:
```rust
pub const IPC_SCHEMA: &str = "trace_commons.daemon.v1";
pub const MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Deserialize)]
pub struct Request { pub id: u64, pub method: String, #[serde(default)] pub params: serde_json::Value }

#[derive(Debug, Serialize)]
pub struct Response { pub id: u64, #[serde(skip_serializing_if="Option::is_none")] pub result: Option<serde_json::Value>, #[serde(skip_serializing_if="Option::is_none")] pub error: Option<IpcError> }

#[derive(Debug, Serialize)]
pub struct IpcError { pub code: String, pub message: String }  // code from the taxonomy; message is a label

#[derive(Debug, Serialize)]
pub struct Event { pub event: String, pub data: serde_json::Value }

pub const ERR_UNKNOWN_METHOD: &str = "unknown_method";
pub const ERR_BAD_PARAMS: &str = "bad_params";
pub const ERR_NOT_AUTHORIZED: &str = "not_authorized";
pub const ERR_BUSY: &str = "busy";
pub const ERR_UNAVAILABLE: &str = "unavailable";

/// Refuses to bind unless the parent dir is 0700 and owned by the euid.
pub async fn bind(store: &ConfigStore) -> Result<UnixListener>;
pub async fn serve(listener: UnixListener, shared: Arc<DaemonShared>) -> Result<()>;
```

- [ ] **Step 1: Write the failing integration test**

Create `tests/daemon_ipc_contract.rs`:
```rust
#[tokio::test]
async fn responses_echo_request_ids_and_events_carry_none() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":7,"method":"hello"}"#).await;
    let resp: serde_json::Value = c.recv_json().await;
    assert_eq!(resp["id"], 7);
    assert_eq!(resp["result"]["schema_version"], "trace_commons.daemon.v1");
}

#[tokio::test]
async fn unknown_method_returns_the_taxonomy_code() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"no_such_method"}"#).await;
    let resp: serde_json::Value = c.recv_json().await;
    assert_eq!(resp["error"]["code"], "unknown_method");
}

#[tokio::test]
async fn subscribe_sends_a_full_snapshot_before_any_delta() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":2,"method":"subscribe"}"#).await;
    let first: serde_json::Value = c.recv_json().await;
    assert_eq!(first["event"], "snapshot");
    assert!(first["data"]["pending"].is_array());
    assert!(first["id"].is_null(), "push frames must not carry an id");
}

#[tokio::test]
async fn setting_auto_upload_over_the_socket_is_refused() {
    // Same-user code execution must not be able to arm autonomous exfiltration.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":3,"method":"set_project_mode","params":{"project_key":"/tmp/p","label":"p","mode":"auto_upload"}}"#).await;
    let resp: serde_json::Value = c.recv_json().await;
    assert_eq!(resp["error"]["code"], "not_authorized");
}

#[tokio::test]
async fn setting_notify_only_over_the_socket_is_allowed() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":4,"method":"set_project_mode","params":{"project_key":"/tmp/p","label":"p","mode":"notify_only"}}"#).await;
    let resp: serde_json::Value = c.recv_json().await;
    assert!(resp["error"].is_null());
}

#[tokio::test]
async fn approve_all_over_the_socket_is_refused() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":5,"method":"approve","params":{"all":true}}"#).await;
    let resp: serde_json::Value = c.recv_json().await;
    assert_eq!(resp["error"]["code"], "not_authorized");
}

#[tokio::test]
async fn oversize_lines_are_rejected_and_close_the_connection() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send_raw(&format!(r#"{{"id":6,"method":"hello","params":"{}"}}"#, "x".repeat(2 * 1024 * 1024))).await;
    let resp: serde_json::Value = c.recv_json().await;
    assert_eq!(resp["error"]["code"], "bad_params");
    assert!(c.is_closed().await);
}

#[tokio::test]
async fn status_exposes_every_state_a_tray_needs() {
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":8,"method":"status"}"#).await;
    let r: serde_json::Value = c.recv_json().await;
    for key in ["logged_in", "paused", "queue_depth", "next_digest_at", "health"] {
        assert!(!r["result"][key].is_null() || key == "next_digest_at", "status missing {key}");
    }
}

#[tokio::test]
async fn daemon_refuses_to_bind_when_the_config_dir_is_world_readable() {
    // The 0700 dir is the enforcing control; UnixListener::bind does not set socket mode.
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(bind(&store).await.is_err());
}
```
`TestDaemon` starts `serve` on a tempdir socket with an in-memory `DaemonShared`, and `connect` returns a line-framed client helper.

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor --test daemon_ipc_contract`
Expected: FAIL, module not found.

- [ ] **Step 3: Implement framing and routing**

`DaemonShared` holds `Mutex<Queue>`, `Mutex<ProjectPolicy>`, `Mutex<DaemonState>`, `Mutex<DaemonSettings>`, `Mutex<HealthState>`, `paused: AtomicBool`, and a `tokio::sync::broadcast::Sender<Event>`. Each connection reads lines with a `BufReader` capped at `MAX_LINE_BYTES`; an oversize or unparseable line gets one `bad_params` response then the connection closes. `subscribe` immediately writes a `snapshot` event, then forwards broadcast frames; on `RecvError::Lagged` it emits `resync_required`.

- [ ] **Step 4: Implement the authorization carve-out**

`set_project_mode` with `mode == auto_upload` and `approve` with `all == true` return `ERR_NOT_AUTHORIZED` with message `"tty-required"`. Route the CLI's own calls through the in-process handler, not the socket, so the CLI is unaffected.

- [ ] **Step 5: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --test daemon_ipc_contract`
Expected: PASS, 9 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/ipc.rs crates/trace-commons-contributor/tests/daemon_ipc_contract.rs
git commit -m "Add the daemon IPC server with a TTY-only autonomy carve-out"
```

---

### Task 9: Watcher, notifier, and the `run` supervisor

**Files:**
- Create: `src/daemon/watcher.rs`, `src/daemon/notify.rs`
- Modify: `src/daemon/mod.rs`

**Interfaces:**
- Produces:
```rust
// notify.rs
/// Pure decision: is a digest due, and what does it say?
pub fn digest_due(last_digest_at: Option<DateTime<Utc>>, now: DateTime<Utc>, interval_secs: u64, pending: usize) -> bool;
pub fn digest_text(pending: &[&QueueEntry]) -> String;   // "3 sessions ready from proj, other"
/// Best-effort OS notification. Never returns Err; a missing notifier is a logged label.
pub fn emit_local(text: &str);

// watcher.rs
pub async fn tick(shared: &DaemonShared, now: DateTime<Utc>) -> Result<TickReport>;
#[derive(Debug, Default, PartialEq)]
pub struct TickReport { pub observed: usize, pub queued: usize, pub auto_uploaded: usize, pub ignored: usize }

// mod.rs
pub async fn run(store: ConfigStore, dry_run: bool) -> Result<()>;
```

- [ ] **Step 1: Write the failing tests**

```rust
// notify.rs
#[test]
fn digest_is_not_due_before_the_interval_elapses() {
    assert!(!digest_due(Some(at("2026-08-08T12:00:00Z")), at("2026-08-08T14:00:00Z"), 14400, 3));
}
#[test]
fn digest_is_due_after_the_interval_with_pending_work() {
    assert!(digest_due(Some(at("2026-08-08T12:00:00Z")), at("2026-08-08T16:01:00Z"), 14400, 3));
}
#[test]
fn digest_is_never_due_with_an_empty_queue() {
    assert!(!digest_due(Some(at("2026-08-08T12:00:00Z")), at("2026-08-09T12:00:00Z"), 14400, 0));
}
#[test]
fn first_digest_is_due_immediately_when_work_exists() {
    assert!(digest_due(None, at("2026-08-08T12:00:00Z"), 14400, 1));
}
#[test]
fn digest_text_names_distinct_projects_without_paths() {
    let text = digest_text(&[&e("proj"), &e("proj"), &e("other")]);
    assert!(text.contains("3 sessions"));
    assert!(text.contains("proj") && text.contains("other"));
    assert!(!text.contains('/'), "digest text must not contain a path");
}

// watcher.rs
#[tokio::test]
async fn tick_queues_a_quiesced_session_for_a_notify_only_project() {
    let f = WatcherFixture::new().with_quiesced_session("proj", 1000);
    let report = tick(&f.shared, f.now()).await.unwrap();
    assert_eq!(report.queued, 1);
    assert_eq!(f.queue_len(), 1);
}

#[tokio::test]
async fn tick_skips_a_session_in_an_ignored_project() {
    let f = WatcherFixture::new()
        .with_quiesced_session("proj", 1000)
        .with_project_mode("proj", ProjectMode::Ignore);
    let report = tick(&f.shared, f.now()).await.unwrap();
    assert_eq!(report.queued, 0);
    assert_eq!(report.ignored, 1);
    assert_eq!(f.queue_len(), 0);
}

#[tokio::test]
async fn tick_never_queues_a_session_that_is_still_being_written() {
    let f = WatcherFixture::new().with_active_session("proj", 1000);
    assert_eq!(tick(&f.shared, f.now()).await.unwrap().queued, 0);
}

#[tokio::test]
async fn tick_is_idempotent_across_repeated_polls() {
    let f = WatcherFixture::new().with_quiesced_session("proj", 1000);
    tick(&f.shared, f.now()).await.unwrap();
    tick(&f.shared, f.now()).await.unwrap();
    assert_eq!(f.queue_len(), 1);
}

#[tokio::test]
async fn tick_pauses_all_work_when_paused() {
    let f = WatcherFixture::new().with_quiesced_session("proj", 1000).paused();
    assert_eq!(tick(&f.shared, f.now()).await.unwrap(), TickReport::default());
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor watcher notify`
Expected: FAIL.

- [ ] **Step 3: Implement `notify.rs`**

`emit_local` shells out only when `settings.local_notifications` is true: `osascript -e 'display notification "..." with title "Trace Commons"'` on macOS, `notify-send "Trace Commons" "..."` on Linux. Both are `std::process::Command` with the text passed as a single argument (never interpolated into a shell string). A non-zero exit or missing binary logs `notifier-unavailable` and returns.

- [ ] **Step 4: Implement `tick`**

Per poll: if paused, return `TickReport::default()`. Otherwise enumerate sources via `all_sources(None, None, None)`, `stat` each `SessionRef.path`, resolve cwd through the cache, evaluate eligibility, resolve policy, then either upsert into the queue (`notify_only`), upload immediately (`auto_upload`), or count as ignored. Persist state and queue once at the end of the tick. Emit `queue_changed` on the broadcast channel when anything changed.

- [ ] **Step 5: Implement `run`**

Acquire `daemon.lock` with `try_lock_exclusive`; exit with a clear error if another daemon holds it. Spawn: the watcher loop (`poll_interval_secs`), the history poller (`history_poll_secs`), the expiry+digest loop (hourly), and the IPC server. `tokio::signal` handles SIGTERM/SIGINT for a clean shutdown that releases the lock and removes the socket.

- [ ] **Step 6: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor watcher notify`
Expected: PASS, 10 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/
git commit -m "Add the watcher tick, digest batching, and the run supervisor"
```

---

### Task 10: CLI surface

**Files:**
- Modify: `src/bin/trace-commons-contributor.rs`, `src/commands.rs`

**Interfaces:**
- Consumes: everything above, via an in-process handler (not the socket) so TTY-gated methods work.
- Produces: `commands::daemon_*` handlers.

- [ ] **Step 1: Write the failing tests**

In `commands.rs` tests:
```rust
#[test]
fn daemon_project_mode_parses_every_documented_value() {
    assert_eq!(parse_project_mode("auto").unwrap(), ProjectMode::AutoUpload);
    assert_eq!(parse_project_mode("notify").unwrap(), ProjectMode::NotifyOnly);
    assert_eq!(parse_project_mode("ignore").unwrap(), ProjectMode::Ignore);
    assert!(parse_project_mode("yolo").is_err());
}

#[test]
fn daemon_project_auto_is_refused_for_the_unknown_bucket_from_the_cli_too() {
    let (_d, store) = tests_support::temp_store();
    let err = daemon_set_project(&store, "unknown-project", ProjectMode::AutoUpload).unwrap_err();
    assert!(err.to_string().contains("unknown-project"));
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor daemon_project`
Expected: FAIL.

- [ ] **Step 3: Add the clap subcommand tree**

In `bin/trace-commons-contributor.rs`, add to `enum Command`:
```rust
    /// Run and control the background upload daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
```
and:
```rust
#[derive(Subcommand)]
enum DaemonAction {
    /// Run the daemon in the foreground (the service manager backgrounds it)
    Run {
        /// Watch, queue, and report, but upload nothing
        #[arg(long)]
        dry_run: bool,
    },
    Status,
    Pending,
    Preview { entry_id: String },
    Approve {
        entry_id: Option<String>,
        #[arg(long)]
        all: bool,
    },
    Dismiss { entry_id: String },
    Pause,
    Resume,
    Projects,
    Project {
        path: PathBuf,
        /// auto | notify | ignore
        #[arg(long)]
        mode: String,
    },
    History {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        #[arg(long)]
        refresh: bool,
    },
    Settings {
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
    },
    Install,
    Uninstall,
}
```

- [ ] **Step 4: Implement handlers**

Each handler loads the store, operates on the same types the IPC layer uses, and renders with the existing `print_table` helper. `--json` produces the same shapes as the IPC results so shells and scripts see one schema. `daemon history` prints submission, status, scopes, pending, final — mirroring `status` at `commands.rs:643` — plus a rollup line and a staleness note when `last_refreshed_at` is old.

- [ ] **Step 5: Run tests and the whole suite**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/
git commit -m "Add the daemon CLI control surface"
```

---

### Task 11: Logout revocation

**Files:**
- Modify: `src/commands.rs` (`logout`), `src/daemon/mod.rs`
- Test: `tests/daemon_logout_revocation.rs`

- [ ] **Step 1: Write the failing integration test**

```rust
#[tokio::test]
async fn logout_stops_a_running_daemon_and_removes_its_state() {
    let h = TestDaemon::start_with_enrollment().await;
    assert!(h.socket_path().exists());
    commands::logout(h.store()).unwrap();
    h.wait_for_exit(Duration::from_secs(5)).await.expect("daemon must exit on logout");
    assert!(!h.socket_path().exists());
    for name in [DAEMON_PROJECTS_FILE, DAEMON_QUEUE_FILE, DAEMON_HISTORY_FILE,
                 DAEMON_STATE_FILE, DAEMON_SETTINGS_FILE] {
        assert!(h.store().read_daemon_file(name).unwrap().is_none(), "{name} survived logout");
    }
}

#[tokio::test]
async fn the_uploader_refuses_once_the_device_key_is_gone() {
    // The cached claim stays valid for minutes; absence of enrollment must stop uploads now.
    let f = UploaderFixture::enrolled();
    f.remove_device_key();
    let decision = f.uploader().upload_entry(&f.entry(), f.now()).await.unwrap();
    assert!(matches!(decision, UploadDecision::Refused { ref reason_label } if reason_label == "not-logged-in"));
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor --test daemon_logout_revocation`
Expected: FAIL.

- [ ] **Step 3: Implement pre-upload revocation checks**

In `Uploader::upload_entry`, before anything else: reload `ContributorConfig`; if absent or `device_key_path()` is missing, set health to `LABEL_NOT_LOGGED_IN` and return `Refused { reason_label: "not-logged-in" }`. If `contributor.json` mtime changed since the context was built, call `ctx.invalidate_claim()` and rebuild the effective config.

- [ ] **Step 4: Implement logout shutdown**

`commands::logout` connects to `daemon.sock` and sends `{"id":0,"method":"shutdown"}`, waits up to 5s for the lock to release, then calls `store.wipe()` and removes the socket and lock files. If the socket is absent, wipe proceeds immediately.

- [ ] **Step 5: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --test daemon_logout_revocation`
Expected: PASS, 2 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/ crates/trace-commons-contributor/tests/daemon_logout_revocation.rs
git commit -m "Stop the daemon and drop its state on logout"
```

---

### Task 12: NEAR AI notice gate and install

**Files:**
- Create: `src/daemon/install.rs`
- Modify: `src/daemon/uploader.rs`

- [ ] **Step 1: Write the failing tests**

```rust
// uploader.rs
#[tokio::test]
async fn daemon_refuses_near_ai_until_the_notice_was_delivered_interactively() {
    // Under a service manager the notice println goes to a log nobody reads.
    let f = UploaderFixture::enrolled_with_near_ai_filter();
    f.clear_near_ai_notice_marker();
    let decision = f.uploader().upload_entry(&f.entry(), f.now()).await.unwrap();
    assert!(matches!(decision, UploadDecision::Refused { ref reason_label }
        if reason_label == "near-ai-notice-not-acknowledged"));
}

#[tokio::test]
async fn daemon_proceeds_once_the_notice_marker_exists() {
    let f = UploaderFixture::enrolled_with_near_ai_filter();
    f.set_near_ai_notice_marker();
    assert!(!matches!(f.uploader().upload_entry(&f.entry(), f.now()).await.unwrap(),
        UploadDecision::Refused { .. }));
}

// install.rs
#[test]
fn systemd_unit_names_the_run_subcommand_and_the_config_dir() {
    let unit = systemd_unit_text("/usr/local/bin/trace-commons-contributor", "/home/z/.config/trace-commons");
    assert!(unit.contains("ExecStart=/usr/local/bin/trace-commons-contributor daemon run"));
    assert!(unit.contains("TRACE_COMMONS_CONTRIBUTOR_DIR=/home/z/.config/trace-commons"));
    assert!(unit.contains("Restart=on-failure"));
}

#[test]
fn install_refuses_near_ai_without_persisted_settings() {
    // Otherwise every entry fails pii-filter-unavailable under systemd.
    let (_d, store) = tests_support::temp_store();
    let mut cfg = sample_config();
    cfg.pii_filter = Some("near-ai".into());
    store.save_config(&cfg).unwrap();
    let err = install(&store).unwrap_err();
    assert!(err.to_string().contains("near_ai"));
}
```

- [ ] **Step 2: Run tests, verify they fail**

Run: `cargo test -p trace-commons-contributor install near_ai`
Expected: FAIL.

- [ ] **Step 3: Implement the notice gate**

In `upload_entry`, when the effective `pii_filter == Some("near-ai")`, check the marker file exists **without creating it** (add `ConfigStore::near_ai_notice_shown() -> bool` alongside the existing `ensure_near_ai_notice_shown`). If absent, set health `LABEL_NEAR_AI_NOTICE_PENDING` and refuse.

- [ ] **Step 4: Implement `install.rs`**

`systemd_unit_text` returns the unit; `install` writes it to `~/.config/systemd/user/trace-commons-contributor.service`, refusing when `pii_filter == Some("near-ai")` and `settings.near_ai.is_none()`. On macOS and Windows, `install` prints that autostart is owned by the platform app and exits non-zero with `autostart-not-supported-on-this-platform`. `uninstall` removes the unit.

- [ ] **Step 5: Run tests, verify they pass**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor install near_ai`
Expected: PASS, 4 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/
git commit -m "Gate the NEAR AI filter on an acknowledged notice and add systemd install"
```

---

### Task 13: Freeze and document the IPC contract

**Files:**
- Create: `docs/contributor-daemon-ipc-v1.md`
- Modify: `README.md` (link it)

- [ ] **Step 1: Write the contract document**

Document, for every method: name, params schema, result schema, error codes, and whether it is TTY-gated. Document the framing rules (id echo, push frames without id, 1 MiB cap, out-of-order responses permitted), the `subscribe` event list (`snapshot`, `queue_changed`, `status_changed`, `digest_due`, `resync_required`), the queue-state enum, the `reason_label` taxonomy from `health.rs`, and the authorization model including the TTY carve-out and the 0700-dir requirement. State explicitly that `path` is local-only and shells must render `project_label`.

- [ ] **Step 2: Add a contract conformance test**

In `tests/daemon_ipc_contract.rs`:
```rust
#[tokio::test]
async fn hello_advertises_exactly_the_documented_method_set() {
    // The doc is the contract; drift between them is the bug this catches.
    let h = TestDaemon::start().await;
    let mut c = h.connect().await;
    c.send(r#"{"id":1,"method":"hello"}"#).await;
    let r: serde_json::Value = c.recv_json().await;
    let mut methods: Vec<String> = r["result"]["methods"].as_array().unwrap()
        .iter().map(|m| m.as_str().unwrap().to_string()).collect();
    methods.sort();
    let mut expected = vec!["approve","dismiss","get_settings","hello","history_rollup",
        "list_history","list_pending","list_projects","pause","preview","refresh_history",
        "resume","set_project_mode","set_settings","shutdown","status","subscribe"];
    expected.sort();
    assert_eq!(methods, expected);
}
```

- [ ] **Step 3: Run the test**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --test daemon_ipc_contract`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add docs/contributor-daemon-ipc-v1.md README.md crates/trace-commons-contributor/tests/daemon_ipc_contract.rs
git commit -m "Freeze and document the daemon IPC v1 contract"
```

---

### Task 14: Full verification

- [ ] **Step 1: Check with CI flags**

Run: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-contributor --bins`
Expected: clean.

- [ ] **Step 2: Build all tests with CI flags**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --no-run`
Expected: clean.

- [ ] **Step 3: Clippy with the repo allow-list**

Run:
```bash
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```
Expected: clean.

- [ ] **Step 4: Full suite, compared against the recorded baseline**

Run: `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor`
Expected: PASS, no regression against the Task 1 baseline.

- [ ] **Step 5: Confirm no new dependencies entered the tree**

Run: `git diff origin/main -- crates/trace-commons-contributor/Cargo.toml Cargo.lock`
Expected: only tokio feature additions; no new `[[package]]` entries in `Cargo.lock`.

- [ ] **Step 6: Manual smoke**

```bash
TRACE_COMMONS_CONTRIBUTOR_DIR=/tmp/tc-daemon-smoke cargo run -p trace-commons-contributor -- daemon run --dry-run
# in another shell:
TRACE_COMMONS_CONTRIBUTOR_DIR=/tmp/tc-daemon-smoke cargo run -p trace-commons-contributor -- daemon status
TRACE_COMMONS_CONTRIBUTOR_DIR=/tmp/tc-daemon-smoke cargo run -p trace-commons-contributor -- daemon pending
```
Expected: status reports `logged_in: false` with health `not-logged-in`; pending lists locally discovered quiesced sessions; nothing uploads.

- [ ] **Step 7: Commit any fixes and open the PR**

```bash
git add -A && git commit -m "Verify the daemon against CI flags"
gh pr create --repo zmanian/trace-commons-server --title "Add the contributor background daemon" --body "..."
```

---

## Self-Review

**Spec coverage:** placement/module layout → File Structure; submit seam → Task 1; watcher/eligibility/growth → Tasks 3, 9; policy + unknown bucket → Task 4; queue/expiry/suspension/supersede → Tasks 5, 6; history + rollup → Task 7; notifier/digest → Task 9; state files → Task 2; logout/revocation/config reload → Task 11; NEAR AI notice → Task 12; PII settings from persisted config → Tasks 1 (parameter) + 12 (install guard); IPC framing/methods/authz → Tasks 8, 13; CLI surface → Task 10; install → Task 12; dependencies → Global Constraints + Task 14 Step 5; error handling → Tasks 6, 11; testing → every task.

**Known gap, deliberately deferred:** the daemon-vs-CLI concurrent upload race noted under "Known concurrency limits" in the spec. Task 14 Step 6 does not exercise it. Verify `submission_id_for` determinism against the ingest dedupe path during PR review; if it does not hold, add a task making the CLI take `daemon.lock`.

**Placeholder scan:** no TBD/TODO; every code step carries real code; test fixtures (`GrowingSessionFixture`, `WatcherFixture`, `UploaderFixture`, `TestDaemon`) are each described where introduced.

**Type consistency:** `PriorUpload` (Task 2) is consumed by `evaluate` (Task 3) with matching fields. `QueueEntry`/`QueueState` (Task 5) are used unchanged in Tasks 6, 8, 9. `HealthState` labels (Task 6) are the `reason_label` taxonomy in Tasks 11-13. `DaemonSettings` field names are identical across Tasks 2, 3, 6, 9. `ProjectMode` variants match between Tasks 4, 8, 9, 10. `growth_min_new_bytes` replaced the spec's event-count threshold, and the spec was corrected to match.
