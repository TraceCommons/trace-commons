//! K2 of #1173: the recorder behind macOS's `SampleDaemonData` sample sets,
//! and the drift test that holds them to it.
//!
//! C1 filled `empty`, `normalDay`, `busyQueue`, `heldSessions`, `armedFolder`
//! and `unknownCounts` with hand-typed JSON, shaped from a one-off read of
//! real replies. K2 replaces that with genuine replies: this file starts the
//! real daemon (the same `daemon::ipc::{bind, serve}` surface
//! `daemon_ipc_flow2_states.rs` and the Swift `DaemonDataKeyCoverageTests`
//! exercise) on a throwaway store built for each sample set, calls the
//! methods `SampleDaemonData` serves, and writes what comes back under
//! `macos/Sources/TCShellCore/DataContract/RecordedSamples/`.
//!
//! What each state is built from:
//!
//! - The queue, the project policy and the local history cache are built
//!   directly through `Queue::upsert`, `ProjectPolicy::set_mode` /
//!   `arm_from_now` and `HistoryCache::save` -- the same technique
//!   `daemon_ipc_flow2_states.rs` already uses for a single entry, scaled up
//!   to the several a sample set needs. This is the real queue, real policy
//!   and real history-rollup code; only the inputs are authored rather than
//!   grown by a watcher tick, which would need real wall-clock polling for
//!   every entry a busy queue has.
//! - Scrub counts (`marks`, `content_marks`, `unsure_spans`) are written
//!   straight onto the authored `QueueEntry` as a pinned `ScrubRecord`,
//!   rather than produced by actually running the redaction pipeline over a
//!   planted transcript. `entry_value` and `second_look::insert_fields` --
//!   the real serialization -- are what turn that into `scrub`, `marks` and
//!   `second_look` on the wire, so the SHAPE is exercised for real; the
//!   counts themselves are chosen to be representative, not measured.
//! - One entry (the one `approve` is recorded against, in `normal_day`) is
//!   backed by a real on-disk claude-code session and goes through the
//!   real `preview` → `approve` pipeline, so that recording is a genuine
//!   redaction count, not an authored one.
//! - `approve`, `keep`, `undo_keep` and `set_project_mode` answer the same
//!   shape no matter the sample set (`SampleDaemonData.reply` never reads
//!   `set` for them), so each is recorded once, from `normal_day`, into
//!   `RecordedSamples/shared/`.
//!
//! The network methods (C3, #1187: `inference_summary`,
//! `inference_call_proof`, `model_spend`, `private_ai`, `mission_catalogue`,
//! `invite_lookup`, `passkey_state`, `account_session_status`,
//! `activity_missions_catalogue`) are not recorded here: they answer from
//! the network, and this recorder's temp store has none behind it. They stay
//! hand-written in `SampleDaemonData.swift` in the shapes the daemon
//! serves, marked `"_sample":"hand-written"`; emitting them as fixtures from
//! #1187's own mock-server tests is a follow-up. The PROVISIONAL shapes the
//! Inference and Missions screens still read are hand-written there too,
//! marked `"_sample":"no source yet"`. `coreDown` stays hand-written as
//! well: it answers `nil` for every method by construction, nothing to
//! record.
//!
//! Some recordings cannot be the raw real capture either, because the
//! screen each feeds needs a state a temp store cannot exhibit (a live
//! IronWire, a day of uploads, an older daemon) --
//! `HAND_WRITTEN_OVERRIDES` and `apply_hand_written_overrides` below. Each is
//! marked `"_sample"` with the reason why. The drift test compares the
//! fields an override leaves alone (`override_touched_keys`), and skips only
//! the files an override replaces whole.
//!
//! # Re-recording
//!
//! `cargo test -p trace-commons-contributor --test k2_sample_recorder -- --ignored record_samples_to_disk`
//! rebuilds every sample set against the current daemon and overwrites
//! `RecordedSamples/` in place; re-run `swift test` in `macos/` afterwards
//! and commit both. `drift_sample_data_matches_the_real_daemon` (an
//! ordinary, non-ignored test) is what the daemon-shape check actually is:
//! it rebuilds the same recordings in memory on every run and fails,
//! naming the method, the sample set and the first differing field path,
//! the moment a committed file stops matching what the daemon now sends.

#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use uuid::Uuid;

use trace_commons_contributor::config::{
    CONTRIBUTOR_CONFIG_SCHEMA_VERSION, ConfigStore, ContributorConfig,
};
use trace_commons_contributor::daemon::history::{HistoryCache, HistoryRecord};
use trace_commons_contributor::daemon::ipc::{DaemonShared, bind, serve};
use trace_commons_contributor::daemon::policy::{ProjectMode, ProjectPolicy, project_id_for};
use trace_commons_contributor::daemon::preview_scheduler;
use trace_commons_contributor::daemon::queue::{
    Queue, QueueEntry, QueueState, SessionShape, entry_id_for,
};
use trace_commons_contributor::daemon::second_look::{ScrubCounts, ScrubRecord};
use trace_commons_contributor::daemon::settings::{DaemonSettings, ScrubCheck, SourceDeclaration};
use trace_commons_contributor::daemon::state::{CwdCacheEntry, DaemonState};
use trace_commons_contributor::identity::DeviceIdentity;
use trace_commons_contributor::source::TraceSource;
use trace_commons_contributor::source::claude_code::ClaudeCodeSource;

// ---------------------------------------------------------------------------
// A tiny JSON-RPC client over the daemon's unix socket (copied, as every
// other test in this suite does, rather than shared across files -- see
// `daemon_ipc_flow2_states.rs`, `daemon_ipc_contract.rs`,
// `daemon_preview_body_over_socket.rs`).
// ---------------------------------------------------------------------------

struct Client {
    reader: BufReader<tokio::net::unix::OwnedReadHalf>,
    writer: tokio::net::unix::OwnedWriteHalf,
}

impl Client {
    async fn connect(store_dir: &Path) -> Self {
        let stream = UnixStream::connect(store_dir.join("daemon.sock"))
            .await
            .unwrap();
        let (r, w) = stream.into_split();
        Client {
            reader: BufReader::new(r),
            writer: w,
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        let line = json!({ "id": 1, "method": method, "params": params });
        self.writer
            .write_all(format!("{line}\n").as_bytes())
            .await
            .unwrap();
        self.writer.flush().await.unwrap();
        let mut reply = String::new();
        self.reader.read_line(&mut reply).await.unwrap();
        serde_json::from_str(&reply).unwrap_or_else(|e| panic!("bad frame {reply:?}: {e}"))
    }

    /// The `result` of one call; panics on an error frame, exactly like the
    /// Swift harness's `result(_:_:_:)`.
    async fn result(&mut self, method: &str, params: Value) -> Value {
        let frame = self.call(method, params).await;
        assert!(frame["error"].is_null(), "{method}: {frame}");
        frame["result"].clone()
    }
}

fn dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .unwrap_or_else(|e| panic!("bad timestamp {s:?}: {e}"))
        .with_timezone(&Utc)
}

// ---------------------------------------------------------------------------
// Projects shared across sample sets.
// ---------------------------------------------------------------------------

struct Project {
    key: &'static str,
    label: &'static str,
}

const API: Project = Project {
    key: "/Users/sample/src/api",
    label: "api",
};
const WEB: Project = Project {
    key: "/Users/sample/src/web",
    label: "web",
};
const INFRA: Project = Project {
    key: "/Users/sample/src/infra",
    label: "infra",
};

// ---------------------------------------------------------------------------
// Entry authoring. One function builds a `QueueEntry` from the same kind of
// plain description `SampleDaemonData.entry` took in Swift -- a project, a
// state, a reason, an optional scrub record, a shape -- and the real
// `entry_value` + `second_look::insert_fields` turn it into the wire shape.
// ---------------------------------------------------------------------------

struct EntryPlan {
    n: u32,
    project: &'static Project,
    source: &'static str,
    reason_label: Option<&'static str>,
    scrub: Option<(u32, u32, u32)>,
    subagent_count: u32,
    subagents_dropped: u32,
    shape: Option<(&'static str, &'static str, u32)>,
    discovered_at: &'static str,
    size_bytes: u64,
    path: Option<PathBuf>,
}

impl EntryPlan {
    fn new(n: u32, project: &'static Project) -> Self {
        EntryPlan {
            n,
            project,
            source: "claude-code",
            reason_label: None,
            scrub: None,
            subagent_count: 0,
            subagents_dropped: 0,
            shape: None,
            discovered_at: "2026-09-30T09:12:00Z",
            size_bytes: 48_213,
            path: None,
        }
    }

    fn build(self) -> QueueEntry {
        let session_hash = format!("k2-sample-{}", self.n);
        let entry_id = entry_id_for(&session_hash);
        let scrub = self
            .scrub
            .map(|(marks, content_marks, unsure_spans)| ScrubRecord {
                envelope_digest: "sha256:k2-sample-pinned".into(),
                counts: ScrubCounts {
                    marks,
                    content_marks,
                    unsure_spans,
                    unsure_unreadable: false,
                },
            });
        let previewed_envelope_digest = scrub.as_ref().map(|r| r.envelope_digest.clone());
        let shape = self.shape.map(|(started, ended, user_turns)| SessionShape {
            started_at: Some(dt(started)),
            ended_at: Some(dt(ended)),
            user_turns,
        });
        QueueEntry {
            entry_id,
            session_hash,
            source: self.source.into(),
            project_key: self.project.key.into(),
            project_path: Some(self.project.key.into()),
            project_label: self.project.label.into(),
            path: self
                .path
                .unwrap_or_else(|| PathBuf::from(format!("/sample/sessions/k2-{}.jsonl", self.n))),
            size_bytes: self.size_bytes,
            discovered_at: dt(self.discovered_at),
            state: QueueState::Pending,
            reason_label: self.reason_label.map(str::to_string),
            subagent_count: self.subagent_count,
            subagents_dropped: self.subagents_dropped,
            shape,
            previewed_envelope_digest,
            scrub,
            ..Default::default()
        }
    }
}

/// The instant the committed history rows are written relative to. Every
/// history timestamp in `RecordedSamples/` reads as an offset from this.
const HISTORY_RECORDED_AT: &str = "2026-09-30T09:00:00Z";

/// The real instant the history rows are actually authored relative to,
/// captured once per process and truncated to whole seconds.
///
/// Unlike every other timestamp this file authors, the history rows cannot
/// sit at a fixed date: `history_rollup` buckets them into `week` and
/// `month` against the daemon's own `Utc::now()` (there is no clock seam to
/// pin), so a fixed date ages out of those windows and the drift test fails
/// on a calendar date rather than on a daemon change. Authoring them
/// relative to the real clock keeps every row's age -- and so every bucket
/// -- constant; `rebase_history_timestamps` then shifts the timestamps the
/// rows themselves carry back onto `HISTORY_RECORDED_AT`, so the recordings
/// stay byte-for-byte deterministic.
fn history_anchor() -> DateTime<Utc> {
    static ANCHOR: OnceLock<DateTime<Utc>> = OnceLock::new();
    *ANCHOR.get_or_init(|| {
        DateTime::from_timestamp(Utc::now().timestamp(), 0).expect("current time is representable")
    })
}

/// The keys a `HistoryRecord` serializes a timestamp under.
const HISTORY_TIMESTAMP_KEYS: &[&str] = &[
    "submitted_at",
    "last_refreshed_at",
    "withdrawn_at",
    "revoked_at",
];

/// Moves one history timestamp from `history_anchor()`'s frame onto
/// `HISTORY_RECORDED_AT`'s. A null (never withdrawn, never revoked) stays
/// null; anything that is not a timestamp is a daemon change the drift
/// test should see, so it is left exactly as the daemon sent it.
fn rebase_history_timestamp(value: &mut Value) {
    let Value::String(s) = value else { return };
    let Ok(t) = DateTime::parse_from_rfc3339(s) else {
        return;
    };
    let shift = history_anchor() - dt(HISTORY_RECORDED_AT);
    *s = (t.with_timezone(&Utc) - shift).to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
}

/// Applies `rebase_history_timestamp` to the history-derived timestamps of
/// the two replies built from the history cache: each row of
/// `list_history.history`, and `history_rollup.last_refreshed_at`. Confined
/// to those paths rather than matched by key name anywhere, so a same-named
/// field elsewhere is never moved.
fn rebase_history_timestamps(method: &str, reply: &mut Value) {
    match method {
        "list_history" => {
            if let Some(rows) = reply.get_mut("history").and_then(Value::as_array_mut) {
                for row in rows {
                    for key in HISTORY_TIMESTAMP_KEYS {
                        if let Some(v) = row.get_mut(*key) {
                            rebase_history_timestamp(v);
                        }
                    }
                }
            }
        }
        "history_rollup" => {
            if let Some(v) = reply.get_mut("last_refreshed_at") {
                rebase_history_timestamp(v);
            }
        }
        _ => {}
    }
}

/// The five history rows every non-empty sample set shares: one of each
/// provenance (approved, unattended, not recorded) and one withdrawn, so
/// `history_rollup`'s `taken_back` is never zero. Written relative to
/// `history_anchor()` -- the real clock -- so `history_rollup`'s `week` and
/// `month` windows see the same ages on every run; see `history_anchor` for
/// why, and for how the recordings stay deterministic anyway.
///
/// The ages are 1, 2, 4, 6 and 20 days, against cutoffs of 7 and 30 days
/// from the daemon's `Utc::now()` (which trails the anchor by the length of
/// one test run): the tightest margin is the 6-day row, a full day inside
/// the week.
fn sample_history_records() -> Vec<HistoryRecord> {
    let now = history_anchor();
    let row = |n: u32,
               project: &Project,
               status: &str,
               unattended: Option<bool>,
               verdict: Option<&str>,
               pending: f32,
               final_: Option<f32>,
               withdrawn: bool,
               age_days: i64| HistoryRecord {
        submission_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, format!("k2-history-{n}").as_bytes()),
        submitted_at: now - chrono::Duration::days(age_days),
        project_id: project_id_for(project.key),
        project_label: project.label.into(),
        source: "claude-code".into(),
        session_hash: format!("sha256:k2-history-{n}"),
        status: status.into(),
        consent_scopes: vec!["debugging_evaluation".into()],
        credit_points_pending: pending,
        credit_points_final: final_,
        explanations: Vec::new(),
        last_refreshed_at: Some(now - chrono::Duration::hours(1)),
        withdrawn_at: withdrawn.then(|| now - chrono::Duration::days(age_days - 1)),
        approved_unattended: unattended,
        approved_verdict: verdict.map(str::to_string),
        // K10: a plausible measured size per row, so the History graph's
        // byte weighting has something to draw.
        uploaded_bytes: Some(12_000 + (n as u64) * 2_048),
        // K12: no row here was withdrawn on the web.
        revoked_at: None,
    };
    vec![
        row(
            1,
            &API,
            "submitted",
            Some(false),
            Some("worked"),
            3.5,
            None,
            false,
            1,
        ),
        row(
            2,
            &INFRA,
            "accepted",
            Some(true),
            None,
            0.0,
            Some(6.0),
            false,
            2,
        ),
        row(
            3,
            &WEB,
            "quarantined",
            Some(false),
            Some("partly"),
            2.0,
            None,
            false,
            4,
        ),
        row(
            4,
            &API,
            "withdrawn",
            Some(false),
            Some("failed"),
            0.0,
            None,
            true,
            6,
        ),
        // Predates the provenance fields: `approved_unattended` is `None`,
        // which decodes as "not recorded" rather than as either answer.
        row(5, &WEB, "accepted", None, None, 0.0, Some(4.0), false, 20),
    ]
}

/// A real claude-code session on disk, discovered the way the daemon itself
/// would, so `size_bytes` and the on-disk content agree -- needed only for
/// the one entry `approve` actually runs its redaction pipeline over.
/// Returns the session reference and the root `find_session` must be told
/// to watch (via `claude_source`) to find it again by path at approve time.
fn real_claude_code_session(
    root: &Path,
    content: &str,
) -> (trace_commons_contributor::source::SessionRef, PathBuf) {
    let sessions_root = root.join("sessions/projects");
    let project = sessions_root.join("-Users-testuser-code-api");
    std::fs::create_dir_all(&project).unwrap();
    let session = "88888888-8888-4888-8888-888888888888";
    let user = json!({
        "type": "user",
        "message": {"role": "user", "content": content},
        "cwd": "/Users/testuser/code/api",
        "timestamp": "2026-09-30T09:00:00Z",
        "version": "2.0.1",
        "sessionId": session,
        "uuid": "a1",
    });
    std::fs::write(
        project.join(format!("{session}.jsonl")),
        format!("{user}\n"),
    )
    .unwrap();
    let src = ClaudeCodeSource::new(sessions_root.clone());
    let session_ref = TraceSource::discover(&src).unwrap().remove(0);
    (session_ref, sessions_root)
}

/// Seeds the watcher's working-directory cache with `count` sessions per
/// project, so `list_projects`' `session_count` and `last_session_at` --
/// which come from this cache, not from the queue -- read as a folder the
/// watcher has actually been through rather than a freshly-added one.
fn seed_sessions_seen(store: &ConfigStore, projects: &[(&Project, u32)]) {
    let mut state = DaemonState::new();
    let mut n = 0u32;
    for (project, count) in projects {
        for _ in 0..*count {
            n += 1;
            state.cwd_cache.insert(
                format!("/sample/sessions/seen-{n}.jsonl"),
                CwdCacheEntry {
                    size_bytes: 4_096,
                    modified_at: dt("2026-09-30T09:00:00Z"),
                    cwd: Some(project.key.to_string()),
                    project_key: Some(project.key.to_string()),
                    // Claude Code, so `list_projects.tools` (K11) reads as a
                    // folder the watcher saw that tool's sessions in.
                    tool: Some("claude-code".to_string()),
                    adapter: Some("claude-code".to_string()),
                },
            );
        }
    }
    // A daemon that has been through a working day has also read its
    // history back recently, so `status.nudge` can speak to verdict news
    // (upsell S4) rather than reading `unknown` for a poll that never ran.
    // Stamped at recording time, because freshness is judged against the
    // clock `status` reads.
    state.last_history_poll_at = Some(Utc::now());
    state.save(store).unwrap();
}

/// Common store setup: a device key and a saved config, so `status.logged_in`
/// is true and `approve` never answers `not-enrolled`. `empty` is the one
/// sample set that skips this.
fn enroll(store: &ConfigStore) {
    let device = DeviceIdentity::load_or_generate(store).unwrap();
    let cfg = ContributorConfig {
        inference_receipt_endpoint: None,
        consent_scopes_chosen: Some(true),
        witness_origin: None,
        inference_receipt_check_attestation: false,
        schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
        issuer_url: "http://issuer.invalid".into(),
        ingest_url: "http://ingest.invalid".into(),
        audience: "trace-commons-upload".into(),
        tenant_id: "tenant-sample".into(),
        instance_id: "instance-1".into(),
        user_subject: "sample".into(),
        device_key_id: device.device_key_id.clone(),
        consent_scopes: vec!["debugging_evaluation".into()],
        pii_filter: None,
        allowed_hosts: None,
        display_handle: None,
        public_bio: None,
        public_since: None,
        witness: None,
    };
    store.save_config(&cfg).unwrap();
}

/// The home directory every recording sees, in place of the real one.
///
/// The harness, IronWire and source probes all resolve the home through the
/// environment (`dirs::home_dir`, `$HOME`, `$IRONWIRE_HOME`), not through
/// the store, so a temp store alone records the machine it runs on: its
/// username in `config_path`, and whichever tools it has installed and
/// credentialed. This points all of them at one fake home, seeded with an
/// installed Claude Code and Codex and no IronWire, before any daemon
/// starts. `normalize` rewrites its path to [`RECORDED_HOME`].
///
/// Process-wide on purpose: the environment is, and both tests in this
/// binary go through `record_all`, which initializes it before either
/// starts a daemon.
struct SampleHome {
    fake: PathBuf,
    /// `$HOME` as the process started, so a recording can be checked for it.
    real: Option<String>,
}

const RECORDED_HOME: &str = "/Users/sample";

fn sample_home() -> &'static SampleHome {
    static HOME: OnceLock<SampleHome> = OnceLock::new();
    HOME.get_or_init(|| {
        let real = std::env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty() && h != "/");
        let fake = std::env::temp_dir().join(format!("k2-sample-home-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&fake);
        std::fs::create_dir_all(fake.join(".claude")).unwrap();
        std::fs::write(fake.join(".claude/settings.json"), "{}\n").unwrap();
        std::fs::create_dir_all(fake.join(".codex")).unwrap();
        std::fs::write(fake.join(".codex/config.toml"), "").unwrap();
        // SAFETY: set once, before any daemon or other thread in this test
        // binary reads the environment (see the doc above).
        unsafe {
            std::env::set_var("HOME", &fake);
            std::env::set_var("IRONWIRE_HOME", fake.join(".ironwire"));
        }
        SampleHome { fake, real }
    })
}

/// Starts the real daemon over a real unix socket on a fresh temp store, and
/// returns a connected client. `build` runs with the store open but before
/// the daemon starts, which is where the queue, the policy and the history
/// cache are written.
async fn start_daemon(
    dir: &Path,
    settings: DaemonSettings,
    build: impl FnOnce(&ConfigStore),
) -> (PathBuf, Client) {
    let store_dir = dir.join("state");
    let store = ConfigStore::open(store_dir.clone()).unwrap();
    build(&store);
    settings.save(&store).unwrap();
    let shared = std::sync::Arc::new(
        DaemonShared::load(ConfigStore::open(store_dir.clone()).unwrap()).unwrap(),
    );
    let runner: std::sync::Arc<dyn preview_scheduler::PreviewJobRunner> = std::sync::Arc::new(
        preview_scheduler::DaemonPreviewRunner::new(std::sync::Arc::clone(&shared)),
    );
    preview_scheduler::spawn_workers(std::sync::Arc::clone(&shared.previews), runner);
    let listener = bind(&ConfigStore::open(store_dir.clone()).unwrap())
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = serve(listener, shared).await;
    });
    let client = Client::connect(&store_dir).await;
    (store_dir, client)
}

fn default_settings() -> DaemonSettings {
    DaemonSettings {
        claude_source: Some(SourceDeclaration::Watch {
            path: PathBuf::from("/sample/claude"),
        }),
        codex_source: Some(SourceDeclaration::Watch {
            path: PathBuf::from("/sample/codex"),
        }),
        quiescence_secs: 0,
        ..DaemonSettings::default()
    }
}

// Every live method `SampleDaemonData.reply` serves, with the params the
// Swift harness (`DaemonDataKeyCoverageTests`) sends them. Called before
// anything that mutates the queue (a keep or an approve), exactly as that
// harness orders it ("approve last").

/// Keys whose value is a timestamp computed from the real wall clock at call
/// time (`Utc::now()` inside the handler) rather than from anything this
/// file authored, and so differs on every run. Apart from the opaque account
/// scope (normalized separately), everything else either comes
/// from a literal this file chose or is derived from one by a pure function
/// (`entry_id_for`, `project_id_for`), and is therefore already stable --
/// except the history rows, which are authored against the real clock and
/// shifted back by `rebase_history_timestamps` (see `history_anchor`).
const VOLATILE_KEYS: &[&str] = &[
    "next_digest_at",
    "resets_at",
    "since",
    "observed_at",
    "last_refresh_at",
    "next_retry_at",
    "hold_until",
];

/// A fixed, recognizably-fake instant that still decodes as a real one.
/// `<next_digest_at>` would say "this value varies" just as plainly, but
/// every one of `VOLATILE_KEYS` is typed as a date on the Swift side, and a
/// sample file the decoder cannot parse defeats the entire point of
/// recording real replies.
const VOLATILE_TIMESTAMP_PLACEHOLDER: &str = "1970-01-01T00:00:00Z";

fn normalize(value: &mut Value, key: Option<&str>) {
    match value {
        Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                normalize(v, Some(k.as_str()));
            }
        }
        Value::Array(items) => {
            for v in items.iter_mut() {
                normalize(v, key);
            }
        }
        // The account scope includes the freshly generated credential epoch and
        // device identity. Preserve its wire type and validate its hash shape,
        // but do not pin one throwaway store's opaque identity in every sample.
        Value::String(s) if key == Some("account_scope") => {
            let digest = s.strip_prefix("sha256:").expect("account scope is SHA-256");
            assert!(
                digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
                "account scope carries a complete SHA-256 digest"
            );
            *s = format!("sha256:{}", "0".repeat(64));
        }
        Value::String(s) if key.is_some_and(|k| VOLATILE_KEYS.contains(&k)) && !s.is_empty() => {
            *s = VOLATILE_TIMESTAMP_PLACEHOLDER.to_string();
        }
        Value::String(s) => {
            let fake = sample_home().fake.to_string_lossy();
            if s.contains(fake.as_ref()) {
                *s = s.replace(fake.as_ref(), RECORDED_HOME);
            }
        }
        _ => {}
    }
}

/// Every recording this crate can make for macOS's `SampleDaemonData`:
/// `method/set` for the eleven methods whose reply depends on the sample
/// set, and `shared/method` for the four that do not (`SampleDaemonData`
/// never reads `set` for them either).
async fn record_all() -> BTreeMap<String, Value> {
    sample_home();
    let mut out = BTreeMap::new();

    for (state, value) in [
        ("empty", record_empty().await),
        ("normalDay", record_normal_day().await),
        ("busyQueue", record_busy_queue().await),
        ("heldSessions", record_held_sessions().await),
        ("armedFolder", record_armed_folder().await),
        ("unknownCounts", record_unknown_counts().await),
    ] {
        for (method, mut reply) in value.per_method {
            rebase_history_timestamps(method, &mut reply);
            normalize(&mut reply, None);
            out.insert(format!("{state}/{method}"), reply);
        }
        for (method, mut reply) in value.shared {
            rebase_history_timestamps(method, &mut reply);
            normalize(&mut reply, None);
            out.entry(format!("shared/{method}")).or_insert(reply);
        }
    }
    for (key, value) in &out {
        assert_no_machine_home(key, value);
    }
    out
}

/// Fails if a recording names the machine it was made on: its real home
/// (and so its username), or the fake home `normalize` should have
/// rewritten.
fn assert_no_machine_home(key: &str, value: &Value) {
    let home = sample_home();
    let text = value.to_string();
    let fake = home.fake.to_string_lossy();
    assert!(
        !text.contains(fake.as_ref()),
        "{key}: the fake home leaked unrewritten"
    );
    if let Some(real) = &home.real {
        assert!(
            !text.contains(real.as_str()),
            "{key}: records this machine's home directory"
        );
    }
}

struct StateRecording {
    per_method: Vec<(&'static str, Value)>,
    /// Only populated by `normal_day`: the invariant methods, captured once.
    shared: Vec<(&'static str, Value)>,
}

async fn capture_state_varying(client: &mut Client) -> Vec<(&'static str, Value)> {
    vec![
        ("status", client.result("status", json!({})).await),
        (
            "list_pending",
            client.result("list_pending", json!({})).await,
        ),
        ("list_kept", client.result("list_kept", json!({})).await),
        (
            "list_projects",
            client.result("list_projects", json!({})).await,
        ),
        (
            "harness_list",
            client.result("harness_list", json!({})).await,
        ),
        (
            "get_settings",
            client.result("get_settings", json!({})).await,
        ),
        (
            "list_history",
            client.result("list_history", json!({ "limit": 50 })).await,
        ),
        (
            "history_rollup",
            client.result("history_rollup", json!({})).await,
        ),
        (
            "commons_credit_summary",
            client.result("commons_credit_summary", json!({})).await,
        ),
        (
            "tool_destinations",
            client.result("tool_destinations", json!({})).await,
        ),
        (
            "inference_calls",
            client
                .result("inference_calls", json!({ "limit": 25 }))
                .await,
        ),
    ]
}

async fn record_empty() -> StateRecording {
    let dir = tempfile::tempdir().unwrap();
    let (_store_dir, mut client) = start_daemon(dir.path(), default_settings(), |_store| {
        // Nothing: no config, no device key, no queue, no projects. A
        // fresh install -- `status.logged_in` is false and `tenant_id` is
        // null.
    })
    .await;
    StateRecording {
        per_method: capture_state_varying(&mut client).await,
        shared: Vec::new(),
    }
}

async fn record_normal_day() -> StateRecording {
    let dir = tempfile::tempdir().unwrap();
    let approve_root = dir.path().join("approve-session");
    std::fs::create_dir_all(&approve_root).unwrap();
    let (approve_session, approve_sessions_root) = real_claude_code_session(
        &approve_root,
        "Please email alice@example.com and bob@example.com about \
         /Users/testuser/code/api/src/main.rs and /Users/testuser/code/api/src/lib.rs",
    );
    // Both derived the same way `EntryPlan::build` derives every entry's
    // id: a stable v5 UUID of the session hash below, so the ids used after
    // the closure to keep and approve them match what was actually queued.
    let kept_entry_id = entry_id_for("k2-sample-90");
    let approve_entry_id = entry_id_for("k2-sample-91");

    // `claude_source` must watch the real session's root so `approve`'s
    // `find_session` can locate it again by path, rather than the sample
    // paths `default_settings` points at for every other entry.
    let settings = DaemonSettings {
        claude_source: Some(SourceDeclaration::Watch {
            path: approve_sessions_root,
        }),
        ..default_settings()
    };

    let (_store_dir, mut client) = start_daemon(dir.path(), settings, |store| {
        enroll(store);

        let mut policy = ProjectPolicy::new();
        policy
            .set_mode(API.key, ProjectMode::NotifyOnly, dt("2026-09-02T10:00:00Z"))
            .unwrap();
        policy
            .set_mode(WEB.key, ProjectMode::NotifyOnly, dt("2026-09-02T10:00:00Z"))
            .unwrap();
        policy.save(store).unwrap();

        let mut queue = Queue::new();
        let mut e1 = EntryPlan::new(1, &API);
        e1.scrub = Some((7, 4, 0));
        e1.subagent_count = 2;
        e1.shape = Some(("2026-09-30T09:05:00Z", "2026-09-30T09:47:00Z", 9));
        queue.upsert(e1.build(), 500).unwrap();

        let mut e2 = EntryPlan::new(2, &WEB);
        e2.discovered_at = "2026-09-30T10:12:00Z";
        queue.upsert(e2.build(), 500).unwrap();

        let mut e3 = EntryPlan::new(3, &API);
        e3.reason_label = Some("returned-from-keep");
        e3.source = "codex";
        e3.discovered_at = "2026-09-30T11:12:00Z";
        queue.upsert(e3.build(), 500).unwrap();

        let mut kept = EntryPlan::new(90, &WEB);
        kept.discovered_at = "2026-09-30T08:25:00Z";
        queue.upsert(kept.build(), 500).unwrap();

        let mut approve_target = EntryPlan::new(91, &API);
        approve_target.path = Some(approve_session.path.clone());
        approve_target.size_bytes = approve_session.size_bytes;
        queue.upsert(approve_target.build(), 500).unwrap();

        queue.save(store).unwrap();
        HistoryCache::save(store, &sample_history_records()).unwrap();
        seed_sessions_seen(store, &[(&API, 22), (&WEB, 9)]);
    })
    .await;

    // Pin the two settings `testProjectsAndToolsCarryK1AndK2Fields` reads a
    // literal value for: local notifications on, and the evening digest at
    // 18:00.
    let _ = client
        .result("set_settings", json!({ "local_notifications": true }))
        .await;
    let _ = client
        .result(
            "set_settings",
            json!({ "digest_schedule": { "mode": "evening", "hour": 18 } }),
        )
        .await;

    // Keep one entry and approve another before capturing anything, so the
    // normalDay narrative's three waiting entries are exactly the three
    // still `Pending` afterwards -- the kept and approved ones have already
    // left the queue, just as they would have in a real session. This
    // doubles as the real `keep` and `approve` recordings for `shared/`.
    let keep_reply = client
        .result("keep", json!({ "entry_id": kept_entry_id }))
        .await;
    let approve_reply = client
        .result("approve", json!({ "entry_id": approve_entry_id }))
        .await;

    let per_method = capture_state_varying(&mut client).await;

    // Captured after the per-state methods above, so these mutations never
    // change what normalDay's own methods answered.
    let set_project_mode_reply = client
        .result(
            "set_project_mode",
            json!({ "project_id": project_id_for(WEB.key), "mode": "notify_only" }),
        )
        .await;
    let undo_keep_reply = client
        .result("undo_keep", json!({ "entry_id": kept_entry_id }))
        .await;

    StateRecording {
        per_method,
        shared: vec![
            ("approve", approve_reply),
            ("keep", keep_reply),
            ("undo_keep", undo_keep_reply),
            ("set_project_mode", set_project_mode_reply),
        ],
    }
}

async fn record_busy_queue() -> StateRecording {
    let dir = tempfile::tempdir().unwrap();
    let (_store_dir, mut client) = start_daemon(dir.path(), default_settings(), |store| {
        enroll(store);

        let mut policy = ProjectPolicy::new();
        for p in [&API, &WEB, &INFRA] {
            policy
                .set_mode(p.key, ProjectMode::NotifyOnly, dt("2026-09-02T10:00:00Z"))
                .unwrap();
        }
        policy.save(store).unwrap();

        let mut queue = Queue::new();
        for n in 1..=14u32 {
            let project = [&API, &WEB, &INFRA][(n as usize) % 3];
            let mut e = EntryPlan::new(n, project);
            e.source = if n % 5 == 0 { "codex" } else { "claude-code" };
            e.subagent_count = n % 6;
            e.size_bytes = 9_000 + u64::from(n) * 3_100;
            e.discovered_at = "2026-09-30T09:12:00Z";
            if n % 4 == 0 {
                e.scrub = Some((n, n / 2, n % 3));
            }
            if n % 7 == 0 {
                e.shape = Some(("2026-09-30T09:05:00Z", "2026-09-30T09:47:00Z", 3 + n % 11));
            }
            queue.upsert(e.build(), 500).unwrap();
        }
        let kept = EntryPlan::new(90, &WEB);
        queue.upsert(kept.build(), 500).unwrap();
        queue.save(store).unwrap();
        HistoryCache::save(store, &sample_history_records()).unwrap();
        seed_sessions_seen(store, &[(&API, 61), (&WEB, 38), (&INFRA, 17)]);
    })
    .await;

    let kept_entry_id = entry_id_for("k2-sample-90");
    let _ = client
        .result("keep", json!({ "entry_id": kept_entry_id }))
        .await;
    StateRecording {
        per_method: capture_state_varying(&mut client).await,
        shared: Vec::new(),
    }
}

async fn record_held_sessions() -> StateRecording {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = default_settings();
    settings.scrub_check = ScrubCheck::Manual;
    let (_store_dir, mut client) = start_daemon(dir.path(), settings, |store| {
        enroll(store);

        let mut policy = ProjectPolicy::new();
        policy
            .set_mode(API.key, ProjectMode::AutoUpload, dt("2026-09-02T10:00:00Z"))
            .unwrap();
        policy
            .set_mode(WEB.key, ProjectMode::AutoUpload, dt("2026-09-02T10:00:00Z"))
            .unwrap();
        policy.save(store).unwrap();

        let mut queue = Queue::new();
        let mut e1 = EntryPlan::new(1, &API);
        e1.reason_label = Some("second-look-review-required");
        e1.scrub = Some((3, 0, 2));
        e1.subagent_count = 2;
        e1.subagents_dropped = 1;
        e1.shape = Some(("2026-09-30T09:05:00Z", "2026-09-30T09:47:00Z", 9));
        queue.upsert(e1.build(), 500).unwrap();

        let mut e2 = EntryPlan::new(2, &WEB);
        e2.reason_label = Some("second-look-review-required");
        e2.scrub = Some((5, 2, 1));
        e2.discovered_at = "2026-09-30T10:12:00Z";
        queue.upsert(e2.build(), 500).unwrap();

        let mut e3 = EntryPlan::new(3, &WEB);
        e3.reason_label = Some("scrub-check-manual");
        e3.discovered_at = "2026-09-30T11:30:00Z";
        queue.upsert(e3.build(), 500).unwrap();

        let mut e4 = EntryPlan::new(4, &API);
        e4.reason_label = Some("scrub-check-manual");
        e4.discovered_at = "2026-09-30T12:00:00Z";
        e4.shape = Some(("2026-09-30T12:05:00Z", "2026-09-30T12:11:00Z", 6));
        queue.upsert(e4.build(), 500).unwrap();

        let kept = EntryPlan::new(90, &WEB);
        queue.upsert(kept.build(), 500).unwrap();

        queue.save(store).unwrap();
        HistoryCache::save(store, &sample_history_records()).unwrap();
        seed_sessions_seen(store, &[(&API, 22), (&WEB, 9)]);
    })
    .await;

    let kept_entry_id = entry_id_for("k2-sample-90");
    let _ = client
        .result("keep", json!({ "entry_id": kept_entry_id }))
        .await;
    StateRecording {
        per_method: capture_state_varying(&mut client).await,
        shared: Vec::new(),
    }
}

async fn record_armed_folder() -> StateRecording {
    let dir = tempfile::tempdir().unwrap();
    let backlog_path = PathBuf::from("/sample/sessions/k2-armed-backlog.jsonl");
    let (_store_dir, mut client) = start_daemon(dir.path(), default_settings(), |store| {
        enroll(store);

        let armed_at = dt("2026-09-30T08:00:00Z");
        let mut policy = ProjectPolicy::new();
        policy
            .set_mode(INFRA.key, ProjectMode::AutoUpload, armed_at)
            .unwrap();
        policy.arm_from_now(INFRA.key, armed_at);
        // A source already recorded for this arming, with every path it saw
        // on disk at that moment: only `backlog_path` is in it, so the two
        // entries queued at paths outside it are new-since-arming and go on
        // their own, while `backlog_path` is held for a person. See
        // `ProjectPolicy::waits_for_a_person_at_send`.
        policy
            .armed_from_now
            .get_mut(INFRA.key)
            .expect("arm_from_now just inserted this key")
            .recorded_sources
            .insert("claude-code".into(), 0);
        policy
            .sessions_on_disk_at_arming
            .insert(backlog_path.to_string_lossy().into_owned(), 0);
        policy.save(store).unwrap();

        let mut queue = Queue::new();
        let mut e1 = EntryPlan::new(1, &INFRA);
        e1.scrub = Some((2, 1, 0));
        e1.discovered_at = "2026-09-30T13:12:00Z";
        queue.upsert(e1.build(), 500).unwrap();

        let mut e2 = EntryPlan::new(2, &INFRA);
        e2.discovered_at = "2026-09-30T14:08:00Z";
        queue.upsert(e2.build(), 500).unwrap();

        let mut e3 = EntryPlan::new(3, &INFRA);
        e3.path = Some(backlog_path.clone());
        e3.discovered_at = "2026-09-30T09:50:00Z";
        queue.upsert(e3.build(), 500).unwrap();

        queue.save(store).unwrap();
        HistoryCache::save(store, &sample_history_records()).unwrap();
        seed_sessions_seen(store, &[(&INFRA, 17)]);
    })
    .await;

    StateRecording {
        per_method: capture_state_varying(&mut client).await,
        shared: Vec::new(),
    }
}

async fn record_unknown_counts() -> StateRecording {
    let dir = tempfile::tempdir().unwrap();
    let (_store_dir, mut client) = start_daemon(dir.path(), default_settings(), |store| {
        // Enrolled, but neither folder has been added to the project
        // policy yet -- `list_projects` shows them `configured: false`,
        // "discovered" rather than set up, distinct from both `empty`
        // (nothing found) and `normalDay` (fully configured).
        enroll(store);

        let mut queue = Queue::new();
        let mut e1 = EntryPlan::new(1, &API);
        e1.scrub = Some((7, 4, 0));
        queue.upsert(e1.build(), 500).unwrap();
        let mut e2 = EntryPlan::new(2, &WEB);
        e2.discovered_at = "2026-09-30T10:12:00Z";
        queue.upsert(e2.build(), 500).unwrap();
        let mut e3 = EntryPlan::new(3, &API);
        e3.discovered_at = "2026-09-30T11:12:00Z";
        queue.upsert(e3.build(), 500).unwrap();
        queue.save(store).unwrap();
        HistoryCache::save(store, &sample_history_records()).unwrap();
    })
    .await;

    StateRecording {
        per_method: capture_state_varying(&mut client).await,
        shared: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Where the files live, and the two tests: write them, and prove they still
// match.
// ---------------------------------------------------------------------------

fn samples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../macos/Sources/TCShellCore/DataContract/RecordedSamples")
}

fn file_for(key: &str) -> PathBuf {
    // `key` is `"<set>/<method>"` or `"shared/<method>"`.
    samples_dir().join(format!("{key}.json"))
}

/// Recordings a temp store cannot produce for real, each marked `_sample`
/// (ignored by every decoder, like Zaki's provisional methods). The drift
/// test compares what an override leaves alone; see `override_touched_keys`.
const HAND_WRITTEN_OVERRIDES: &[&str] = &[
    "unknownCounts/status",
    "normalDay/inference_calls",
    "busyQueue/inference_calls",
    "normalDay/status",
    "busyQueue/status",
    "normalDay/harness_list",
    "busyQueue/harness_list",
    "empty/harness_list",
];

/// For an override that edits a real recording rather than replacing it,
/// the keys it touches. The drift test removes these from both sides and
/// compares the rest, so the untouched fields are still held to the daemon.
fn override_touched_keys(key: &str) -> Option<&'static [&'static str]> {
    match key {
        "unknownCounts/status" => Some(&[
            "decisions_owed",
            "unpurposed_traces",
            "nudge",
            "idle_sessions",
            "_sample",
        ]),
        "normalDay/status" | "busyQueue/status" => Some(&[
            "private_inference_state",
            "routing",
            "daily_budget",
            "_sample",
        ]),
        "normalDay/harness_list" | "busyQueue/harness_list" => Some(&[
            "harnesses",
            "activity",
            "spend",
            "destination_port",
            "destination_credentialed",
            "_sample",
        ]),
        "empty/harness_list" => Some(&["spend", "_sample"]),
        _ => None,
    }
}

/// Applied to a fresh `record_all()` map before it is written to disk, never
/// before it is compared in the drift test -- so the drift test's own
/// "fresh" values stay the honest real capture, and the overridden fields
/// are the ones that can never match it (by design, not by drift).
fn apply_hand_written_overrides(all: &mut BTreeMap<String, Value>) {
    // Ron's badge draws "—" for `decisions_owed` when the daemon is too old
    // to send it or unreachable -- never a fact a temp store's real daemon
    // can exhibit, since `status_value` (`daemon::ipc`) always computes a
    // concrete count. Take the real recording for every other field and
    // remove this one by hand. `unpurposed_traces` goes with it: a daemon
    // too old to send the badge count is too old to send the upsell count,
    // and an absent one must mean "no card", never 0. `nudge` goes for the
    // same reason: absent is "no suggestion", never a lead of its own, and
    // so does `idle_sessions`: absent is "no idle card", never 0.
    if let Some(Value::Object(status)) = all.get_mut("unknownCounts/status") {
        status.remove("decisions_owed");
        status.remove("unpurposed_traces");
        status.remove("nudge");
        status.remove("idle_sessions");
        status.insert(
            "_sample".into(),
            Value::String("absent on purpose: older daemon".into()),
        );
    }

    // `readable: true` needs a live IronWire ledger answering
    // `shared.routing_ledger()`, refreshed by the daemon's main loop
    // (`refresh_routing`, `pub(crate)`) polling a real proxy over HTTP --
    // nothing an IPC-only temp store can be. Shaped exactly like the real
    // reply `calls_page` builds (`daemon/inference_map.rs` and its own
    // tests): `id`, `at`, `tool`, `family`, `model`, `route`,
    // `cost.{known,priced_micros}`, `proof`.
    for key in ["normalDay/inference_calls", "busyQueue/inference_calls"] {
        all.insert(key.to_string(), hand_written_inference_calls());
    }

    // The screens for a connected tool, Private AI running, routed rows
    // and a day's uploads. Each needs a live IronWire on a real port and a
    // day of real uploads, so a temp store's real daemon always reports
    // them off, empty and unknown. C1's hand-written sets had them; these
    // keep the Swift previews of those screens. Every other field of these
    // files is still the real capture, held to the daemon by the drift
    // test (`override_touched_keys`).
    for set in ["normalDay", "busyQueue"] {
        if let Some(Value::Object(status)) = all.get_mut(&format!("{set}/status")) {
            status.insert(
                "private_inference_state".into(),
                json!({"state": "running", "port": 8463}),
            );
            status.insert(
                "routing".into(),
                json!({"state": "rows_seen", "derived": true,
                       "last_refresh_at": VOLATILE_TIMESTAMP_PLACEHOLDER, "unreadable_rows": 0}),
            );
            if let Some(Value::Object(budget)) = status.get_mut("daily_budget") {
                let max_bytes = budget
                    .get("max_bytes_per_day")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let max_uploads = budget
                    .get("max_uploads_per_day")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                budget.insert("bytes_today".into(), json!(1_048_576));
                budget.insert(
                    "bytes_remaining".into(),
                    json!(max_bytes.saturating_sub(1_048_576)),
                );
                budget.insert("uploads_today".into(), json!(4));
                budget.insert(
                    "uploads_remaining".into(),
                    json!(max_uploads.saturating_sub(4)),
                );
            }
            status.insert(
                "_sample".into(),
                json!("private AI, routing and budget need a live IronWire"),
            );
        }
        if let Some(Value::Object(list)) = all.get_mut(&format!("{set}/harness_list")) {
            if let Some(Value::Array(rows)) = list.get_mut("harnesses") {
                for row in rows.iter_mut().filter(|r| r["id"] == "claude") {
                    row["connected"] = json!(true);
                    row["state"] = json!("answering");
                    row["last_call_at"] = json!("2026-09-30T09:04:11+00:00");
                    row["can_connect"] = json!(false);
                    row["can_disconnect"] = json!(true);
                }
            }
            list.insert(
                "activity".into(),
                json!({"readable": true, "window_hours": 24, "last_call_at": "2026-09-30T09:04:11+00:00",
                       "families": [{"family": "anthropic", "last_call_at": "2026-09-30T09:04:11+00:00", "calls": 12}]}),
            );
            list.insert("spend".into(), json!({"known": true, "micros": 1_230_000}));
            list.insert("destination_port".into(), json!(8463));
            list.insert("destination_credentialed".into(), json!(true));
            list.insert(
                "_sample".into(),
                json!("a connected tool needs a live IronWire"),
            );
        }
    }
    // Known zero, distinct from the unknown every other set reports.
    if let Some(Value::Object(list)) = all.get_mut("empty/harness_list") {
        list.insert("spend".into(), json!({"known": true, "micros": 0}));
        list.insert(
            "_sample".into(),
            json!("known zero spend needs a live IronWire"),
        );
    }
}

fn hand_written_inference_calls() -> Value {
    json!({
        "_sample": "no live IronWire in a temp store",
        "readable": true,
        "window_hours": 24,
        "calls": [
            {"id": 414, "at": "2026-09-30T09:04:11+00:00", "tool": "claude-code", "family": "anthropic",
             "model": "zai-org/GLM-4.6", "route": "routed", "cost": {"known": true, "priced_micros": 8400},
             "proof": "verified"},
            {"id": 413, "at": "2026-09-30T08:58:40+00:00", "tool": "codex", "family": "openai",
             "model": "Qwen/Qwen3.6-27B-FP8", "route": "routed", "cost": {"known": true, "priced_micros": 12300},
             "proof": "gateway_only"},
            {"id": 412, "at": "2026-09-30T08:51:02+00:00", "tool": "unknown", "family": "unknown",
             "model": "unknown", "route": "outside", "cost": {"known": false, "priced_micros": null},
             "proof": "outside"},
            {"id": 411, "at": "2026-09-30T08:40:19+00:00", "tool": "claude-code", "family": "anthropic",
             "model": "zai-org/GLM-4.6", "route": "routed", "cost": {"known": true, "priced_micros": 5100},
             "proof": "pending"},
            {"id": 410, "at": "2026-09-30T08:31:55+00:00", "tool": "claude-code", "family": "anthropic",
             "model": "zai-org/GLM-4.6", "route": "routed", "cost": {"known": true, "priced_micros": 4900},
             "proof": "failed"}
        ],
        "next_cursor": null
    })
}

/// Re-records every sample set and overwrites the committed files. Run this,
/// then `swift test` in `macos/`, and commit both when re-recording for a
/// daemon change. Not run by a plain `cargo test`.
#[tokio::test]
#[ignore = "writes into the repo; run explicitly to re-record"]
async fn record_samples_to_disk() {
    let mut all = record_all().await;
    apply_hand_written_overrides(&mut all);
    for (key, value) in all {
        let path = file_for(&key);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut text = serde_json::to_string_pretty(&value).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
    }
}

/// Finds the first path at which two JSON values disagree, depth-first, so a
/// failure names one field rather than dumping both trees.
fn first_diff(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for key in e
                .keys()
                .chain(a.keys())
                .collect::<std::collections::BTreeSet<_>>()
            {
                let next = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                // Absent is not null: a field added or removed is drift even
                // when its value is null.
                let d = match (e.get(key), a.get(key)) {
                    (Some(ev), Some(av)) => first_diff(ev, av, &next),
                    (Some(_), None) => Some(format!("{next}: no longer sent by the daemon")),
                    (None, Some(_)) => Some(format!(
                        "{next}: sent by the daemon, absent from the committed file"
                    )),
                    (None, None) => None,
                };
                if d.is_some() {
                    return d;
                }
            }
            None
        }
        (Value::Array(e), Value::Array(a)) if e.len() == a.len() => {
            for (i, (ev, av)) in e.iter().zip(a.iter()).enumerate() {
                if let Some(d) = first_diff(ev, av, &format!("{path}[{i}]")) {
                    return Some(d);
                }
            }
            None
        }
        _ if expected == actual => None,
        _ => Some(format!("{path}: expected {expected}, got {actual}")),
    }
}

/// K2 of #1173: re-records every sample set in memory and compares it
/// against the committed files, field by field. A mismatch fails naming the
/// method, the sample set and the field path, rather than a diff of two
/// JSON blobs.
#[tokio::test]
async fn drift_sample_data_matches_the_real_daemon() {
    let fresh = record_all().await;
    let mut failures = Vec::new();
    for (key, actual) in &fresh {
        // `HAND_WRITTEN_OVERRIDES` are not the raw real capture by design
        // (see `apply_hand_written_overrides`). An override that edits a
        // recording is compared without the keys it touched; one that
        // replaces the whole file is skipped.
        let touched = override_touched_keys(key);
        if HAND_WRITTEN_OVERRIDES.contains(&key.as_str()) && touched.is_none() {
            continue;
        }
        let path = file_for(key);
        let committed_text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!("{key}: no committed recording at {path:?} ({e}); run record_samples_to_disk")
        });
        let mut committed: Value =
            serde_json::from_str(&committed_text).unwrap_or_else(|e| panic!("{key}: {e}"));
        let mut actual = actual.clone();
        for k in touched.unwrap_or(&[]) {
            for side in [&mut committed, &mut actual] {
                if let Value::Object(map) = side {
                    map.remove(*k);
                }
            }
        }
        if let Some(diff) = first_diff(&committed, &actual, "") {
            failures.push(format!("{key}: {diff}"));
        }
    }
    // A method the committed set no longer has (the recorder was narrowed)
    // is as much a drift as a changed field.
    for entry in std::fs::read_dir(samples_dir())
        .into_iter()
        .flatten()
        .flatten()
    {
        if entry.path().is_dir() {
            for file in std::fs::read_dir(entry.path())
                .into_iter()
                .flatten()
                .flatten()
            {
                let set = entry.file_name().to_string_lossy().into_owned();
                let method = file
                    .path()
                    .file_stem()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let key = format!("{set}/{method}");
                if !fresh.contains_key(&key) {
                    failures.push(format!(
                        "{key}: committed file has no matching recording any more"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "the real daemon no longer matches the committed samples:\n{}",
        failures.join("\n")
    );
}

#[test]
fn first_diff_reports_a_field_added_or_removed_even_when_null() {
    let committed = json!({"a": 1, "gone": null});
    let fresh = json!({"a": 1, "new": null});
    let diff = first_diff(&committed, &fresh, "").expect("a key set change is drift");
    assert!(diff.starts_with("gone:"), "{diff}");
    assert_eq!(
        first_diff(&json!({"a": 1}), &json!({"a": 1, "new": null}), ""),
        Some("new: sent by the daemon, absent from the committed file".into())
    );
    assert_eq!(
        first_diff(&json!({"a": null}), &json!({"a": null}), ""),
        None
    );
}

#[test]
fn account_scope_normalization_preserves_presence_and_type() {
    let mut scope = serde_json::json!({"account_scope": format!("sha256:{}", "a".repeat(64))});
    normalize(&mut scope, None);
    assert_eq!(scope["account_scope"], format!("sha256:{}", "0".repeat(64)));
    for original in [
        serde_json::json!({}),
        serde_json::json!({"account_scope": null}),
        serde_json::json!({"account_scope": 7}),
    ] {
        let mut value = original.clone();
        normalize(&mut value, None);
        assert_eq!(value, original);
        assert_ne!(value, scope);
    }
}

#[test]
#[should_panic(expected = "account scope carries a complete SHA-256 digest")]
fn account_scope_normalization_refuses_a_malformed_digest() {
    let mut scope = serde_json::json!({"account_scope": "sha256:truncated"});
    normalize(&mut scope, None);
}
