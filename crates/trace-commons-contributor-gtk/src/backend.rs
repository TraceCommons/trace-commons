//! Connect to the daemon, or be it.
//!
//! On Linux the separate daemon under the systemd user unit is the primary
//! deployment and this application is an optional client over its socket --
//! the inverse of macOS, where the app hosts the loop in-process. Both have
//! to work here: a contributor who only ever runs the app should not have to
//! learn what a user unit is, and a contributor who runs the unit should not
//! have the app fail on the lock.
//!
//! The existing exclusive lock arbitrates, and the arbitration is the whole
//! of the logic below: try to take it, and if somebody else has it, connect
//! to them instead. There is no other coordination and no second lock.
//!
//! ## The one capability that differs between the two modes
//!
//! `preview` over the socket is a **summary** -- counts, sizes, and the
//! redacted opening prompt -- by deliberate design of the contract. The full
//! redacted body is available only in-process, through
//! `daemon::ipc::open_preview`. So the shell can show "Exactly what would be
//! sent" and search the transcript **only when it hosts the loop**. Attached
//! to a daemon it did not start, it cannot, and it says so rather than
//! pretending. See `docs/superpowers/plans/linux-shell-report.md`.

use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::daemon;
use trace_commons_contributor::daemon::ipc::DaemonShared;
use trace_commons_contributor::daemon::settings::SourceDeclaration;

/// How the shell reaches the daemon.
pub enum Backend {
    /// A daemon was already running -- almost always the systemd user unit.
    /// Every call is one request over the control socket.
    Attached { store: ConfigStore },
    /// Nothing held the lock, so this process took it and runs the loop
    /// itself. Calls are answered in-process.
    Hosting(Box<Hosting>),
}

pub struct Hosting {
    rt: tokio::runtime::Runtime,
    shared: Arc<DaemonShared>,
    /// Kept alive for the life of the process: dropping it would release
    /// the exclusive lock while the loop is still mutating the queue.
    _embedded: daemon::EmbeddedDaemon,
}

/// The label this shell reports when the contributor has not said which
/// session folders to watch.
///
/// The same string the C ABI uses, deliberately: macOS and Windows read it
/// off `tc_daemon_start`, this shell produces it directly, and one label
/// across the product means the three onboarding flows can route on the same
/// fact rather than each inventing a spelling.
pub const ERR_ROOTS_NOT_DECLARED: &str = "roots-not-declared";
/// A declaration named a source this build does not know. Fixed and
/// content-free like every other label here: the input that would identify
/// it is an adapter name, and the failure is refused rather than written.
pub const ERR_UNKNOWN_SOURCE: &str = "unknown-source";

/// Record the two session roots the contributor named, so the next
/// `Backend::open` can start.
///
/// Goes through `apply_settings_object` rather than assigning the fields,
/// for the reason `tc_daemon_start_with_settings` shares `set_settings`'
/// validator: one definition of a valid settings object. This shell links
/// the core directly, so this is that same definition rather than a second
/// implementation of it -- the objection that keeps Swift and C# from
/// writing `daemon-settings.json` themselves does not apply to a caller
/// that is already Rust.
///
/// Refuses an incomplete declaration here rather than writing one and
/// letting the next start refuse: a half-written settings file is a worse
/// thing to leave behind than an unanswered question.
pub fn declare_sources(dir: &std::path::Path, answers: &[(&str, SourceDeclaration)]) -> Result<()> {
    let store = ConfigStore::open(dir.to_path_buf())?;
    let mut settings = daemon::settings::DaemonSettings::load(&store)?;
    // Every answer the screen collected, keyed by adapter name rather than
    // by a list of fields kept here. A source the roots screen can now
    // discover but this function had never heard of would otherwise be
    // rendered, answered, and silently dropped.
    //
    // The `_source` spellings, not the `_root` ones: a path can say where to
    // watch but cannot say "off", and off is the answer this exists to make
    // expressible.
    let mut object = serde_json::Map::new();
    for (source, declaration) in answers {
        let key = daemon::settings::source_settings_key(source)
            .ok_or_else(|| anyhow!(ERR_UNKNOWN_SOURCE))?;
        object.insert(key.to_string(), serde_json::to_value(declaration)?);
    }
    daemon::settings::apply_settings_object(&mut settings, &serde_json::Value::Object(object))
        .map_err(|label| anyhow!(label))?;
    if !daemon::settings::roots_declared(&settings) {
        bail!(ERR_ROOTS_NOT_DECLARED);
    }
    settings.save(&store)?;
    Ok(())
}

/// Declare both sources as folders to watch.
///
/// Kept as the two-paths spelling for callers that have two paths; the
/// window goes through [`declare_sources`], because a folder chooser cannot
/// express "I don't use this agent".
pub fn declare_roots(
    dir: &std::path::Path,
    claude_root: &std::path::Path,
    codex_root: &std::path::Path,
) -> Result<()> {
    declare_sources(
        dir,
        &[
            (
                trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                SourceDeclaration::Watch {
                    path: claude_root.to_path_buf(),
                },
            ),
            (
                trace_commons_contributor::source::SOURCE_CODEX,
                SourceDeclaration::Watch {
                    path: codex_root.to_path_buf(),
                },
            ),
        ],
    )
}

impl Backend {
    /// Attach to a running daemon, or start one in-process.
    ///
    /// The running-daemon check comes first, and a lost race on the lock
    /// falls back to attaching rather than failing: between the check and
    /// the `try_lock`, the user unit may have come up.
    ///
    /// Starting one in-process fails closed on undeclared session roots, on
    /// the rule `daemon::settings::roots_declared` owns. Until this landed
    /// the Linux shell had no such check at all: it started, and an unset
    /// `claude_root` meant the daemon watched the contributor's real
    /// `~/.claude`.
    ///
    /// The ATTACH path is deliberately not gated. A daemon that is already
    /// running was started by `trace-commons-contributor daemon` -- somebody
    /// typing a command on purpose -- and the CLI keeps its own defaults by
    /// design. This is an application-shell posture, and refusing to attach
    /// to a daemon this shell did not start would be a different decision
    /// than the one made here.
    pub fn open(dir: std::path::PathBuf) -> Result<Self> {
        let store = ConfigStore::open(dir.clone())?;
        if daemon::client::is_running(&store) {
            return Ok(Backend::Attached { store });
        }
        let settings = daemon::settings::DaemonSettings::load(&store)?;
        if !daemon::settings::roots_declared(&settings) {
            bail!(ERR_ROOTS_NOT_DECLARED);
        }
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        match rt.block_on(daemon::start_embedded(store)) {
            Ok(embedded) => {
                let shared = Arc::clone(&embedded.shared);
                let loop_shared = Arc::clone(&shared);
                rt.spawn(async move {
                    let _ = daemon::run_supervisor(loop_shared, false).await;
                });
                Ok(Backend::Hosting(Box::new(Hosting {
                    rt,
                    shared,
                    _embedded: embedded,
                })))
            }
            // Somebody took the lock in between. That is not an error: it is
            // the other half of the arbitration.
            Err(_) => Ok(Backend::Attached {
                store: ConfigStore::open(dir)?,
            }),
        }
    }

    pub fn hosts_the_loop(&self) -> bool {
        matches!(self, Backend::Hosting(_))
    }

    /// One request, one result. Blocking: callers run this off the GTK
    /// thread (see `worker`), because a `preview` can run the whole
    /// redaction pipeline and, under an external privacy filter, a network
    /// round trip.
    pub fn call(&self, method: &str, params: serde_json::Value) -> Result<serde_json::Value> {
        let response = match self {
            Backend::Attached { store } => {
                match daemon::client::try_call(store, method, &params)? {
                    Some(r) => r,
                    // The daemon was there when we attached and is not now.
                    // A fixed label, and the window's own not-running state
                    // handles the rest.
                    None => bail!("daemon-not-running"),
                }
            }
            Backend::Hosting(h) => daemon::ipc::handle_local(&h.shared, method, params),
        };
        if let Some(err) = response.error {
            // `error.message` is a fixed label by contract, so forwarding it
            // cannot leak a path or a token.
            bail!("{}", err.message);
        }
        response
            .result
            .ok_or_else(|| anyhow!("daemon answered with neither a result nor an error"))
    }

    /// How many times `needle` appears in an entry's PRE-redaction session
    /// text. `None` on any failure.
    ///
    /// A COUNT, never content -- that is the whole bound of the daemon call
    /// behind this, and the reason it is allowed to read unredacted bytes at
    /// all. `None` rather than a `Result` because the one thing the caller
    /// must not do is round a failure off to a clean answer; see
    /// [`crate::original_search::Outcome::Unknown`].
    pub fn search_original(&self, entry_id: &str, needle: &str) -> Option<u32> {
        // One arm for both modes: `handle_local` runs the same async
        // dispatcher the socket does, so a hosting build routes this method
        // without a second implementation to keep in step.
        let value = self
            .call(
                "search_original",
                serde_json::json!({ "entry_id": entry_id, "needle": needle }),
            )
            .ok()?;
        value["matches"]
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
    }

    /// The redacted body and the summary together, for the preview sheet.
    ///
    /// `Ok(None)` for the body means "this deployment cannot serve it" --
    /// attached to a daemon we do not host -- not "there is none". The sheet
    /// renders that honestly instead of showing an empty transcript.
    pub fn preview(
        &self,
        entry_id: &str,
    ) -> Result<(crate::model::PreviewSummary, Option<String>)> {
        match self {
            Backend::Attached { .. } => {
                let value = self.call("preview", serde_json::json!({ "entry_id": entry_id }))?;
                Ok((serde_json::from_value(value)?, None))
            }
            Backend::Hosting(h) => {
                let id: uuid::Uuid = entry_id.parse().map_err(|_| anyhow!("entry-id-invalid"))?;
                let shared = Arc::clone(&h.shared);
                let (summary, body) =
                    h.rt.block_on(daemon::ipc::open_preview(&shared, id))
                        .map_err(|label| anyhow!("{label}"))?;
                let summary = serde_json::from_value(serde_json::to_value(&summary)?)?;
                Ok((summary, Some(body)))
            }
        }
    }

    /// A blocking iterator over daemon events, for the thread that keeps the
    /// window live. Returns event names only, with one narrow exception --
    /// see [`DaemonEvent`] -- and never a payload the shell would have to
    /// trust as current state; the shell re-reads state itself, exactly as
    /// `resync_required` requires it to be able to do anyway.
    pub fn events(&self) -> Result<Box<dyn EventStream>> {
        match self {
            Backend::Attached { store } => Ok(Box::new(SocketEvents::open(store)?)),
            Backend::Hosting(h) => Ok(Box::new(BroadcastEvents {
                rx: h.shared.events.subscribe(),
            })),
        }
    }
}

/// One daemon event, as this shell's event pump sees it.
///
/// `entry_id` is populated for exactly one event, `preview_ready`, and is
/// `None` for every other. This is a deliberate, narrow exception to "names
/// only", not a reopening of the rule: an entry id is an identifier, not
/// state. Reading it only tells the shell *which* card to re-ask the daemon
/// about -- `App::handle_preview_ready` still calls `preview_request` and
/// still trusts nothing but that fresh answer. The alternative -- forgetting
/// which entry resolved and re-checking every card still outstanding on
/// every `preview_ready` -- is what turned filling a 500-card queue into
/// tens of thousands of redundant requests; see
/// `docs/superpowers/specs/2026-08-20-preview-scheduler-design.md`.
pub struct DaemonEvent {
    pub name: String,
    pub entry_id: Option<String>,
    /// Populated for exactly one event, `digest_due`, and `None` for every
    /// other.
    ///
    /// This is the second narrow exception to "names only", added for the
    /// same shape of reason as `entry_id` and held to the same limit. A
    /// digest can now be about traces that were contributed without ever
    /// being queued, and an armed project queues nothing -- so the shell
    /// cannot recover these numbers by looking at its own entries, the way
    /// it recovers the waiting count. It is counts and labels: no path, no
    /// id, no state the shell then reasons from. Everything else on the
    /// payload stays unread.
    pub digest: Option<DigestFacts>,
}

/// What a `digest_due` event says happened since the last digest.
///
/// Labels, not keys: these are the daemon's own display strings, already
/// reduced from paths, and they go straight into notification text that a
/// desktop environment may persist.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DigestFacts {
    pub contributed: usize,
    pub contributed_projects: Vec<String>,
    pub credit_pending: f32,
}

/// `digest_due`'s `data` is built in `daemon::tick`. Absent fields decode as
/// zero and an empty list, which degrades this to the waiting-only digest
/// that shipped before rather than to a wrong number -- the case that
/// matters when a new shell attaches to an older daemon.
fn digest_facts(name: &str, data: &serde_json::Value) -> Option<DigestFacts> {
    if name != trace_commons_contributor::daemon::ipc::EVENT_DIGEST_DUE {
        return None;
    }
    Some(DigestFacts {
        contributed: data
            .get("contributed")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        contributed_projects: data
            .get("contributed_projects")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        credit_pending: data
            .get("credit_pending")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0) as f32,
    })
}

/// `preview_ready`'s `data` is always `PreviewOutcome::to_value`, which
/// always carries `entry_id` -- see `daemon::preview_scheduler`. Every other
/// event's `data` is left untouched: this reads the one field this shell
/// ever trusts off an event payload, and only for the one event it is
/// documented to carry it.
fn preview_ready_entry_id(name: &str, data: &serde_json::Value) -> Option<String> {
    if name != trace_commons_contributor::daemon::ipc::EVENT_PREVIEW_READY {
        return None;
    }
    data.get("entry_id")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

pub trait EventStream: Send {
    /// Blocks until the next event, or returns `None` when the stream ends.
    fn next(&mut self) -> Option<DaemonEvent>;
}

struct BroadcastEvents {
    rx: tokio::sync::broadcast::Receiver<trace_commons_contributor::daemon::ipc::Event>,
}

impl EventStream for BroadcastEvents {
    fn next(&mut self) -> Option<DaemonEvent> {
        match self.rx.blocking_recv() {
            Ok(event) => Some(DaemonEvent {
                entry_id: preview_ready_entry_id(&event.event, &event.data),
                digest: digest_facts(&event.event, &event.data),
                name: event.event,
            }),
            // Lagged: the shell's answer to a missed event is the same as
            // its answer to `resync_required` -- re-read everything.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => Some(DaemonEvent {
                name: "resync_required".to_string(),
                entry_id: None,
                digest: None,
            }),
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    }
}

/// A persistent subscription over the control socket. One connection for the
/// life of the window, separate from the one-shot request connections, so a
/// slow `preview` cannot stall the event stream.
struct SocketEvents {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
}

impl SocketEvents {
    fn open(store: &ConfigStore) -> Result<Self> {
        use std::io::Write;
        use trace_commons_contributor::config::DAEMON_SOCK_FILE;

        let mut stream =
            std::os::unix::net::UnixStream::connect(store.daemon_path(DAEMON_SOCK_FILE))?;
        stream.write_all(b"{\"id\":1,\"method\":\"subscribe\",\"params\":{}}\n")?;
        stream.flush().ok();
        Ok(Self {
            reader: std::io::BufReader::new(stream),
        })
    }
}

impl EventStream for SocketEvents {
    fn next(&mut self) -> Option<DaemonEvent> {
        use std::io::BufRead;
        loop {
            let mut line = String::new();
            match self.reader.read_line(&mut line) {
                Ok(0) | Err(_) => return None,
                Ok(_) => {}
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                continue;
            };
            // Events carry no `id`; that is how the contract says to tell
            // them from the `subscribe` response sharing this connection.
            if let Some(name) = value.get("event").and_then(|v| v.as_str()) {
                let data = value.get("data").unwrap_or(&serde_json::Value::Null);
                let entry_id = preview_ready_entry_id(name, data);
                let digest = digest_facts(name, data);
                return Some(DaemonEvent {
                    name: name.to_string(),
                    entry_id,
                    digest,
                });
            }
        }
    }
}

#[cfg(test)]
mod roots_tests {
    use super::*;
    use trace_commons_contributor::daemon::settings::DaemonSettings;

    /// A scratch directory that removes itself.
    ///
    /// Hand-rolled rather than pulling in `tempfile`: this crate is its own
    /// workspace with its own lockfile, so a dev-dependency here is a real
    /// new package edge, and the repo's dependency policy asks for an inline
    /// utility when one fits in a few lines.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!("tc-gtk-{tag}-{unique}"));
            std::fs::create_dir_all(&path).unwrap();
            Scratch(path)
        }

        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn declare(dir: &std::path::Path, claude: Option<&str>, codex: Option<&str>) {
        let store = ConfigStore::open(dir.to_path_buf()).unwrap();
        let make = |name: &str| {
            let p = dir.join(name);
            std::fs::create_dir_all(&p).unwrap();
            p
        };
        use trace_commons_contributor::daemon::settings::SourceDeclaration;
        DaemonSettings {
            claude_source: claude
                .map(make)
                .map(|path| SourceDeclaration::Watch { path }),
            codex_source: codex
                .map(make)
                .map(|path| SourceDeclaration::Watch { path }),
            ..Default::default()
        }
        .save(&store)
        .unwrap();
    }

    #[test]
    fn open_refuses_to_host_a_daemon_when_no_roots_are_declared() {
        let dir = Scratch::new("no-roots");
        let message = match Backend::open(dir.path().to_path_buf()) {
            Ok(_) => panic!("undeclared roots must refuse to start"),
            Err(e) => e.to_string(),
        };
        assert_eq!(
            message, ERR_ROOTS_NOT_DECLARED,
            "the label has to be exactly the one the other two shells route on"
        );
    }

    #[test]
    fn open_refuses_when_only_one_root_is_declared() {
        // The fail-open an `||` would have allowed: an unset codex_root is
        // the real ~/.codex, not "no codex source".
        let dir = Scratch::new("half-roots");
        declare(dir.path(), Some("claude"), None);

        let message = match Backend::open(dir.path().to_path_buf()) {
            Ok(_) => panic!("half a declaration must refuse"),
            Err(e) => e.to_string(),
        };
        assert_eq!(message, ERR_ROOTS_NOT_DECLARED);
    }

    #[test]
    fn declare_roots_turns_a_refusal_into_a_start() {
        // The whole exit from the refusal, end to end: refused, the
        // contributor names two folders, and the next open hosts.
        let dir = Scratch::new("declare");
        assert!(Backend::open(dir.path().to_path_buf()).is_err());

        let claude = dir.path().join("claude");
        let codex = dir.path().join("codex");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        declare_roots(dir.path(), &claude, &codex).unwrap();

        let backend = match Backend::open(dir.path().to_path_buf()) {
            Ok(b) => b,
            Err(e) => panic!("after declaring roots the shell must start: {e}"),
        };
        assert!(backend.hosts_the_loop());
    }

    #[test]
    fn declare_roots_survives_a_folder_name_that_would_break_string_building() {
        // These come from a file chooser. A quote or a backslash in a folder
        // name must round-trip, which is why the settings object is built as
        // JSON rather than formatted.
        let dir = Scratch::new("awkward");
        let claude = dir.path().join(r#"He said "hi"\back"#);
        let codex = dir.path().join("codex");
        std::fs::create_dir_all(&claude).unwrap();
        std::fs::create_dir_all(&codex).unwrap();

        declare_roots(dir.path(), &claude, &codex).unwrap();

        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let settings = DaemonSettings::load(&store).unwrap();
        assert_eq!(
            settings.claude_source.as_ref().and_then(|d| d.path()),
            Some(claude.as_path())
        );
    }

    #[test]
    fn declaring_an_agent_off_is_a_real_answer_that_lets_the_daemon_start() {
        // "I don't use Codex" has to be an answer, not a silence. Before
        // `SourceDeclaration` the only way to express it was to leave the
        // root unset, and an unset root means the real ~/.codex -- so the
        // answer a privacy-conscious contributor is most likely to give was
        // the one that scanned their work.
        let dir = Scratch::new("codex-off");
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(&claude).unwrap();

        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Watch {
                        path: claude.clone(),
                    },
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
            ],
        )
        .unwrap();

        let backend = match Backend::open(dir.path().to_path_buf()) {
            Ok(b) => b,
            Err(e) => panic!("an off declaration is complete and must start: {e}"),
        };
        assert!(backend.hosts_the_loop());
    }

    #[test]
    fn an_agent_declared_off_never_reaches_the_conventional_location() {
        // Asserting on the declaration itself, not on a count: the failure
        // that matters is not "a source went missing" but "a source is
        // rooted at the contributor's home directory".
        let dir = Scratch::new("off-not-home");
        let claude = dir.path().join("claude");
        std::fs::create_dir_all(&claude).unwrap();

        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Watch { path: claude },
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
            ],
        )
        .unwrap();

        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let settings = DaemonSettings::load(&store).unwrap();
        assert_eq!(settings.codex_source, Some(SourceDeclaration::Off));
        assert_eq!(
            settings.codex_source.as_ref().and_then(|d| d.path()),
            None,
            "off must resolve to no directory at all, not to a fallback"
        );
    }

    /// The roots screen renders one row per discovered source, and every
    /// row it renders must be written. A source the screen can show but
    /// this function drops would ask the contributor a question and then
    /// ignore the answer.
    #[test]
    fn every_answered_source_is_written_not_just_the_two_that_gate_the_start() {
        let dir = Scratch::new("every-answer");
        let gemini = dir.path().join("gemini");
        std::fs::create_dir_all(&gemini).unwrap();

        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Off,
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
                (
                    trace_commons_contributor::source::SOURCE_GEMINI_CLI,
                    SourceDeclaration::Watch {
                        path: gemini.clone(),
                    },
                ),
            ],
        )
        .unwrap();

        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let settings = DaemonSettings::load(&store).unwrap();
        assert_eq!(
            settings.gemini_source,
            Some(SourceDeclaration::Watch { path: gemini }),
            "the contributor said to watch it; the answer must survive"
        );
    }

    /// A source this build has no settings key for is refused, not written
    /// under a guessed field name.
    #[test]
    fn an_unknown_source_name_is_refused() {
        let dir = Scratch::new("unknown-source");
        let err = declare_sources(dir.path(), &[("not-an-adapter", SourceDeclaration::Off)])
            .expect_err("an unknown source must refuse");
        assert_eq!(err.to_string(), ERR_UNKNOWN_SOURCE);
    }

    #[test]
    fn both_agents_off_is_a_complete_declaration() {
        // Somebody who uses neither agent still has to be able to leave the
        // screen. Refusing here would be the dead end this whole slice
        // exists to remove.
        let dir = Scratch::new("both-off");
        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Off,
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
            ],
        )
        .unwrap();

        let backend = match Backend::open(dir.path().to_path_buf()) {
            Ok(b) => b,
            Err(e) => panic!("two off declarations are still a declaration: {e}"),
        };
        assert!(backend.hosts_the_loop());
    }

    #[test]
    fn an_off_declaration_survives_a_reload_rather_than_reverting_to_unasked() {
        // The regression that would matter most quietly: if `off` failed to
        // round-trip through the settings file it would load back as None,
        // which is "never asked" -- and never-asked is the state that falls
        // back to the real location.
        let dir = Scratch::new("off-round-trip");
        declare_sources(
            dir.path(),
            &[
                (
                    trace_commons_contributor::source::SOURCE_CLAUDE_CODE,
                    SourceDeclaration::Off,
                ),
                (
                    trace_commons_contributor::source::SOURCE_CODEX,
                    SourceDeclaration::Off,
                ),
            ],
        )
        .unwrap();

        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let reloaded = DaemonSettings::load(&store).unwrap();
        assert_eq!(reloaded.claude_source, Some(SourceDeclaration::Off));
        assert_eq!(reloaded.codex_source, Some(SourceDeclaration::Off));
    }

    #[test]
    fn open_hosts_a_daemon_once_both_roots_are_declared() {
        let dir = Scratch::new("both-roots");
        declare(dir.path(), Some("claude"), Some("codex"));

        let backend = match Backend::open(dir.path().to_path_buf()) {
            Ok(b) => b,
            Err(e) => panic!("declared roots must start: {e}"),
        };
        assert!(
            backend.hosts_the_loop(),
            "nothing else held the lock, so this shell should be hosting"
        );
    }
}
