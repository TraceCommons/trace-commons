//! Revocable, credential-bound Claude Desktop Code connection. This is not a
//! managed CLI session and creates no transcript collection grant.
use super::ipc::{DaemonShared, Request, Response};
use crate::{
    config::ConfigStore,
    managed::{models::ChatModel, route::ManagedRoute, secrets::ApiKey},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{path::Path, sync::atomic::Ordering};
use uuid::Uuid;

const ROLLOUT_FLAG: &str = "claude_desktop_private_ai";
const FILE: &str = crate::config::DAEMON_CLAUDE_DESKTOP_CONNECTION_FILE;

/// Only routing metadata and an opaque local capability. Never an inference key.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Binding {
    version: u8,
    pub(crate) id: Uuid,
    pub(crate) project: String,
    pub(crate) port: u16,
    pub(crate) model: String,
    pub(crate) background_model: String,
    key_sha256: String,
    pub(crate) compact_window: u64,
    pub(crate) output_tokens: u64,
}

pub(crate) struct Connection {
    binding: Option<Binding>,
    route: Option<ManagedRoute>,
    generation: u64,
    changed: tokio::sync::watch::Sender<u64>,
    starting: bool,
    pending_key_sha256: Option<String>,
    start_done: Vec<tokio::sync::watch::Receiver<bool>>,
    drains: Vec<tokio::task::JoinHandle<()>>,
    unavailable: bool,
    terminating: bool,
    #[cfg(test)]
    test_base_url: Option<String>,
    #[cfg(test)]
    test_publication: Option<std::sync::Arc<dyn Fn() + Send + Sync>>,
}
impl Connection {
    pub(crate) fn open(store: &ConfigStore) -> Self {
        let loaded = load(store);
        Self {
            unavailable: loaded.is_err(),
            binding: loaded.ok().flatten(),
            route: None,
            generation: 0,
            changed: tokio::sync::watch::channel(0).0,
            starting: false,
            pending_key_sha256: None,
            start_done: vec![],
            drains: vec![],
            terminating: false,
            #[cfg(test)]
            test_base_url: None,
            #[cfg(test)]
            test_publication: None,
        }
    }
    fn advance(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.changed.send_replace(self.generation);
        self.starting = false;
        self.pending_key_sha256 = None;
        self.generation
    }
}

fn fingerprint(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}
fn rollout_enabled(shared: &DaemonShared) -> bool {
    shared.feature_flags.snapshot().is_enabled(ROLLOUT_FLAG)
}
fn eligible_key(shared: &DaemonShared) -> Result<String, &'static str> {
    if !rollout_enabled(shared) {
        return Err("unavailable");
    }
    if shared.shutdown.load(Ordering::Acquire) {
        return Err("disabled");
    }
    policy_key(shared)
}
fn policy_key(shared: &DaemonShared) -> Result<String, &'static str> {
    let settings = shared.settings.lock().expect("settings lock");
    if shared.dev_dry_run || !settings.private_inference {
        return Err("disabled");
    }
    settings
        .near_ai_inference
        .as_ref()
        .filter(|c| !c.key.is_empty())
        .map(|c| c.key.clone())
        .ok_or("sign_in_required")
}
fn load(store: &ConfigStore) -> Result<Option<Binding>, ()> {
    match std::fs::symlink_metadata(store.daemon_path(FILE)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(m) if m.is_file() && !m.file_type().is_symlink() && m.len() <= 16384 => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if m.permissions().mode() & 0o077 != 0 {
                    return Err(());
                }
            }
        }
        _ => return Err(()),
    }
    let bytes = store.read_daemon_file(FILE).map_err(|_| ())?.ok_or(())?;
    let binding: Binding = serde_json::from_slice(&bytes).map_err(|_| ())?;
    if binding.version != 1
        || binding.port == 0
        || binding.id.is_nil()
        || !binding.project.starts_with("nearai-")
        || binding.project.len() != 39
        || !binding.project[7..].bytes().all(|b| b.is_ascii_hexdigit())
        || binding.key_sha256.len() != 64
        || !binding.key_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || !crate::managed::opencode::valid_model(&binding.model)
        || !crate::managed::opencode::valid_model(&binding.background_model)
        || !(100_000..=1_000_000).contains(&binding.compact_window)
        || !(1..=8192).contains(&binding.output_tokens)
    {
        return Err(());
    }
    Ok(Some(binding))
}
fn save(store: &ConfigStore, binding: &Binding, expected: Option<&Binding>) -> Result<(), ()> {
    // Refuse a corrupt, redirected or concurrently replaced metadata file.
    if load(store)?.as_ref() != expected {
        return Err(());
    }
    store
        .write_daemon_file(FILE, &serde_json::to_vec(binding).map_err(|_| ())?)
        .map_err(|_| ())
}

/// Use the smaller advertised context, reserving the answer cap emitted to
/// the client: the smaller advertised output limit, capped at 8,192.
/// Claude's absolute auto-compact setting clamps below 100k, so smaller or
/// unsized models cannot safely use this setup. Never invent a token count.
pub(crate) fn context_budget(
    main: &str,
    background: &str,
    models: &[ChatModel],
) -> Option<(u64, u64)> {
    let main = models.iter().find(|m| m.id == main)?;
    let background = models.iter().find(|m| m.id == background)?;
    let window = main.context_length?.min(background.context_length?);
    let output = main
        .max_output_length?
        .min(background.max_output_length?)
        .min(8192);
    let compact = window.checked_sub(output)?.min(1_000_000);
    (compact >= 100_000).then_some((compact, output))
}

fn copy() -> Value {
    json!({
        "title":"Claude Desktop", "setup":"Set up", "save":"Save", "copy":"Copy", "disconnect":"Disconnect",
        "main_model":"Main model", "background_model":"Background model",
        "status_disconnected":"Disconnected", "status_ready":"Ready", "status_observed":"Connected",
        "status_unavailable":"Unavailable", "status_sign_in_required":"Sign in required", "status_disabled":"Private AI is off",
        "instructions":"In Claude Desktop’s Code tab, create a Local saved environment, select a folder, and paste these variables. Start a new session with that environment. Only sessions using it route through Private AI. Do not set ANTHROPIC_AUTH_TOKEN.",
        "error":"Setup unavailable. Check Private AI, sign-in, and model availability, then try again."
    })
}
fn environment(binding: &Binding) -> String {
    // Keep provider model IDs out of Vertex's model path. Stable custom aliases
    // let the gateway choose the explicit main and background models.
    [
        ("CLAUDE_CODE_USE_VERTEX", "1".to_owned()),
        ("CLAUDE_CODE_SKIP_VERTEX_AUTH", "1".to_owned()),
        (
            "ANTHROPIC_VERTEX_BASE_URL",
            format!("http://127.0.0.1:{}/vertex", binding.port),
        ),
        ("ANTHROPIC_VERTEX_PROJECT_ID", binding.project.clone()),
        ("CLOUD_ML_REGION", "global".to_owned()),
        ("ANTHROPIC_MODEL", "trace-private-main".to_owned()),
        (
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "trace-private-main".to_owned(),
        ),
        (
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "trace-private-main".to_owned(),
        ),
        (
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "trace-private-haiku".to_owned(),
        ),
        (
            "ANTHROPIC_SMALL_FAST_MODEL",
            "trace-private-haiku".to_owned(),
        ),
        (
            "CLAUDE_CODE_BG_CLASSIFIER_MODEL",
            "trace-private-haiku".to_owned(),
        ),
        (
            "CLAUDE_CODE_AUTO_MODE_MODEL",
            "trace-private-haiku".to_owned(),
        ),
        ("CLAUDE_CODE_AUTO_MODE_SERVER", "0".to_owned()),
        (
            "CLAUDE_CODE_AUTO_COMPACT_WINDOW",
            binding.compact_window.to_string(),
        ),
        (
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS",
            binding.compact_window.to_string(),
        ),
        (
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            binding.output_tokens.to_string(),
        ),
    ]
    .into_iter()
    .map(|(k, v)| format!("{k}={v}"))
    .collect::<Vec<_>>()
    .join("\n")
}
fn regular(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() <= 512 * 1024 * 1024)
}
fn observed(store: &ConfigStore, binding: &Binding) -> bool {
    let home = crate::managed::route::home_for(store, binding.id);
    for dir in [home.parent().unwrap(), home.as_path()] {
        if !std::fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        {
            return false;
        }
    }
    let path = home.join("ledger.sqlite");
    if !regular(&path) {
        return false;
    }
    for suffix in ["ledger.sqlite-wal", "ledger.sqlite-shm"] {
        let file = home.join(suffix);
        if std::fs::symlink_metadata(&file).is_ok() && !regular(&file) {
            return false;
        }
    }
    let Ok(home) = std::fs::canonicalize(home) else {
        return false;
    };
    let Ok(conn) = rusqlite::Connection::open_with_flags(
        home.join("ledger.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX
            | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
    ) else {
        return false;
    };
    let _ = conn.busy_timeout(std::time::Duration::from_millis(50));
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(50);
    conn.progress_handler(1000, Some(move || std::time::Instant::now() >= deadline));
    conn.query_row("SELECT EXISTS(SELECT 1 FROM exchanges WHERE path='/v1/messages' AND backend='nearai' AND status BETWEEN 200 AND 299 AND failure IS NULL AND response_sha256 IS NOT NULL AND served_model IN (?1,?2))",
        [&binding.model,&binding.background_model],|row|row.get::<_,bool>(0)).unwrap_or(false)
}
fn value(shared: &DaemonShared, explicit_preset: bool) -> Value {
    let eligible = eligible_key(shared);
    let held = shared
        .claude_desktop
        .lock()
        .expect("desktop connection lock");
    let ready = eligible.is_ok()
        && !held.terminating
        && held
            .route
            .as_ref()
            .is_some_and(|route| !route.is_finished());
    let did_observe = ready
        && held
            .binding
            .as_ref()
            .is_some_and(|b| observed(&shared.store, b));
    let state = match eligible {
        Err(state) => state,
        Ok(_) if !held.starting && held.start_done.iter().any(|done| !*done.borrow()) => {
            "unavailable"
        }
        Ok(_) if held.terminating => "disabled",
        Ok(_) if held.unavailable => "unavailable",
        Ok(_) if did_observe => "observed",
        Ok(_) if ready => "ready",
        Ok(_) => "disconnected",
    };
    let mut value = json!({"state":state,"observed":did_observe,"copy":copy()});
    if let Some(binding) = &held.binding {
        value["model"] = json!(binding.model);
        value["background_model"] = json!(binding.background_model);
        if ready && explicit_preset && matches!(state, "ready" | "observed") {
            value["environment"] = json!(environment(binding));
        }
    }
    value
}

/// Stop accepting before acknowledging disconnect, switch-off or key withdrawal.
/// The ledger remains available to Insights. Shutdown preserves the preset only
/// for a normal restart or temporary rollout disable; account/policy changes
/// revoke it permanently.
pub(crate) async fn stop(shared: &DaemonShared, revoke: bool) {
    let (route, starts, completion) = {
        let mut held = shared
            .claude_desktop
            .lock()
            .expect("desktop connection lock");
        held.advance();
        if revoke {
            held.binding = None;
            held.unavailable = shared.store.remove_daemon_file(FILE).is_err();
        }
        let route = held.route.take();
        let starts = std::mem::take(&mut held.start_done);
        let completion = route.as_ref().map(|_| {
            let (done, receiver) = tokio::sync::watch::channel(false);
            held.start_done.push(receiver);
            std::sync::Arc::new(StartCompletion(done))
        });
        (route, starts, completion)
    };
    if let Some(route) = route {
        let (closed, drain) = route.stop_desktop(completion).await;
        let mut held = shared
            .claude_desktop
            .lock()
            .expect("desktop connection lock");
        held.drains.retain(|task| !task.is_finished());
        held.drains.push(drain);
        if !closed {
            held.unavailable = true;
        }
    }
    for mut done in starts {
        while !*done.borrow_and_update() {
            if done.changed().await.is_err() {
                break;
            }
        }
    }
}
/// Drop path cannot await. Invalidate every pending start and transfer listener
/// cleanup to its existing owner task by dropping the route.
pub(crate) fn request_stop(shared: &DaemonShared) {
    let mut held = shared
        .claude_desktop
        .lock()
        .expect("desktop connection lock");
    held.terminating = true;
    held.advance();
    if let Some(route) = held.route.as_mut() {
        route.request_shutdown();
    }
}

/// Shared with a partially started proxy: cancellation is complete only once
/// its listener has closed, including when the requesting future was dropped.
pub(crate) struct StartCompletion(tokio::sync::watch::Sender<bool>);
impl Drop for StartCompletion {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

struct StartGuard<'a> {
    shared: &'a DaemonShared,
    generation: u64,
    completed: bool,
    done: Option<std::sync::Arc<StartCompletion>>,
}
impl Drop for StartGuard<'_> {
    fn drop(&mut self) {
        self.done.take();
        if self.completed {
            return;
        }
        let mut held = self
            .shared
            .claude_desktop
            .lock()
            .expect("desktop connection lock");
        if held.generation == self.generation {
            held.advance();
            held.unavailable = true;
        }
    }
}

async fn start(
    shared: &DaemonShared,
    mut binding: Binding,
    key: String,
    generation: u64,
    cancelled: tokio::sync::watch::Receiver<u64>,
    done: std::sync::Arc<StartCompletion>,
) {
    let expected = (binding.port != 0).then(|| binding.clone());
    let mut guard = StartGuard {
        shared,
        generation,
        completed: false,
        done: Some(done.clone()),
    };
    let base_url = crate::managed::route::near_ai_base_url().to_owned();
    #[cfg(test)]
    let base_url = shared
        .claude_desktop
        .lock()
        .unwrap()
        .test_base_url
        .clone()
        .unwrap_or(base_url);
    let candidate = match ApiKey::new(key) {
        Ok(key) => {
            ManagedRoute::start_desktop(&shared.store, key, &base_url, &binding, cancelled, done)
                .await
                .map_err(|error| {
                    #[cfg(test)]
                    eprintln!("desktop route start failed: {error:?}");
                    #[cfg(not(test))]
                    let _ = error;
                })
        }
        Err(_) => Err(()),
    };
    let Ok(route) = candidate else {
        let mut held = shared
            .claude_desktop
            .lock()
            .expect("desktop connection lock");
        if held.generation == generation {
            held.starting = false;
            held.unavailable = true;
        }
        guard.completed = true;
        return;
    };
    let budget = context_budget(&binding.model, &binding.background_model, route.models());
    let mut rejected = Some(route);
    #[cfg(test)]
    let publication = shared
        .claude_desktop
        .lock()
        .unwrap()
        .test_publication
        .clone();
    {
        // Settings -> feature policy -> Desktop ownership. The policy guard
        // spans save and publication, so accepting an off manifest cannot finish
        // between an enabled decision and making the route visible.
        let settings = shared.settings.lock().expect("settings lock");
        shared.feature_flags.with_snapshot(|snapshot| {
            let current = snapshot.is_enabled(ROLLOUT_FLAG)
                && !shared.shutdown.load(Ordering::Acquire)
                && !shared.dev_dry_run
                && settings.private_inference
                && settings
                    .near_ai_inference
                    .as_ref()
                    .is_some_and(|credential| fingerprint(&credential.key) == binding.key_sha256);
            #[cfg(test)]
            if let Some(publication) = publication {
                publication();
            }
            let mut held = shared
                .claude_desktop
                .lock()
                .expect("desktop connection lock");
            if held.generation == generation {
                held.starting = false;
            }
            if current && held.generation == generation && !held.terminating {
                if let Some((compact, output)) = budget {
                    if binding.port == 0 {
                        binding.compact_window = compact;
                        binding.output_tokens = output;
                    }
                    binding.port = rejected.as_ref().unwrap().port();
                    if save(&shared.store, &binding, expected.as_ref()).is_ok() {
                        held.binding = Some(binding);
                        held.route = rejected.take();
                        held.unavailable = false;
                    } else {
                        #[cfg(test)]
                        eprintln!("desktop metadata save failed");
                        held.unavailable = true;
                    }
                } else {
                    held.unavailable = true;
                }
            }
        });
    }
    guard.completed = true;
    if let Some(route) = rejected {
        let (_, drain) = route.stop_desktop(guard.done.clone()).await;
        shared
            .claude_desktop
            .lock()
            .expect("desktop connection lock")
            .drains
            .push(drain);
    }
}

pub(crate) async fn reconcile(shared: &DaemonShared) {
    let (result, wait) = tokio::sync::oneshot::channel();
    let (lifetime, ended) = tokio::sync::oneshot::channel::<()>();
    if shared
        .spawn_detached(move |owned| async move {
            tokio::select! {
                biased;
                _=ended => {},
                _=reconcile_inline(&owned) => {let _=result.send(());}
            }
        })
        .is_some()
    {
        // Keep this sender alive across the complete wait. Its drop means the
        // requesting future was cancelled, rather than successful publication.
        let _request_lifetime = lifetime;
        let _ = wait.await;
        return;
    }
    reconcile_inline(shared).await;
}

async fn reconcile_inline(shared: &DaemonShared) {
    let terminating = shared.shutdown.load(Ordering::Acquire)
        || shared
            .claude_desktop
            .lock()
            .expect("desktop connection lock")
            .terminating;
    if terminating {
        // Termination must not turn a status request into persistent revocation.
        // The next daemon rechecks both policy and key before resuming.
        stop(shared, false).await;
        return;
    }
    let key = policy_key(shared);
    let (must_revoke, corrupt, pending_key_mismatch) = {
        let held = shared
            .claude_desktop
            .lock()
            .expect("desktop connection lock");
        let mismatch = held.binding.as_ref().is_some_and(|binding| {
            key.as_ref().map(|key| fingerprint(key)).as_ref() != Ok(&binding.key_sha256)
        });
        let corrupt = held.binding.as_ref().is_some_and(|binding| {
            load(&shared.store).as_ref().ok().and_then(Option::as_ref) != Some(binding)
        });
        let pending_key_mismatch = held.starting
            && held
                .pending_key_sha256
                .as_ref()
                .is_some_and(|fingerprint_saved| {
                    key.as_ref().map(|key| fingerprint(key)).as_ref() != Ok(fingerprint_saved)
                });
        (mismatch, corrupt, pending_key_mismatch)
    };
    if must_revoke || key.is_err() || corrupt || pending_key_mismatch {
        stop(shared, true).await;
        if corrupt {
            shared
                .claude_desktop
                .lock()
                .expect("desktop connection lock")
                .unavailable = true;
        }
        return;
    }
    if !rollout_enabled(shared) {
        stop(shared, false).await;
        return;
    }
    let pending = {
        let mut held = shared
            .claude_desktop
            .lock()
            .expect("desktop connection lock");
        if held.terminating
            || held.starting
            || held
                .route
                .as_ref()
                .is_some_and(|route| !route.is_finished())
        {
            return;
        }
        // A failed route never silently changes its saved port or capability.
        if held.route.take().is_some() {
            held.unavailable = true;
        }
        let Some(binding) = held.binding.clone() else {
            return;
        };
        let generation = held.advance();
        held.starting = true;
        held.pending_key_sha256 = Some(binding.key_sha256.clone());
        let (done, receiver) = tokio::sync::watch::channel(false);
        held.start_done.retain(|done| !*done.borrow());
        held.start_done.push(receiver);
        (
            binding,
            generation,
            held.changed.subscribe(),
            std::sync::Arc::new(StartCompletion(done)),
        )
    };
    start(
        shared,
        pending.0,
        key.unwrap(),
        pending.1,
        pending.2,
        pending.3,
    )
    .await;
}

/// A feature-specific owner, run beside remote refresh/expiry. An off transition
/// cancels catalogue startup immediately, while stop keeps its socket-release
/// barrier before a later enable may resume the persisted capability.
pub(crate) async fn run_rollout(shared: &DaemonShared) {
    let mut changes = shared.feature_flags.subscribe();
    loop {
        let enabled = rollout_enabled(shared);
        if enabled {
            tokio::select! {
                biased;
                _ = wait_rollout_change(shared, &mut changes, enabled) => continue,
                _ = reconcile_inline(shared) => {},
            }
        } else {
            // Do not cancel the listener-close acknowledgement on re-enable.
            reconcile_inline(shared).await;
        }
        if rollout_enabled(shared) != enabled {
            continue;
        }
        wait_rollout_change(shared, &mut changes, enabled).await;
    }
}
async fn wait_rollout_change(
    shared: &DaemonShared,
    changes: &mut tokio::sync::watch::Receiver<u64>,
    enabled: bool,
) {
    loop {
        if rollout_enabled(shared) != enabled {
            return;
        }
        if changes.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

pub(crate) async fn handle(shared: &DaemonShared, request: &Request) -> Response {
    let request_owned = request.clone();
    let (result, wait) = tokio::sync::oneshot::channel();
    let (lifetime, ended) = tokio::sync::oneshot::channel::<()>();
    if shared
        .spawn_detached(move |owned| async move {
            tokio::select! {
                biased;
                _=ended => {},
                response=handle_inline(&owned,&request_owned) => {let _=result.send(response);}
            }
        })
        .is_some()
    {
        let _request_lifetime = lifetime;
        return wait.await.unwrap_or_else(|_| {
            Response::err(request.id, "unavailable", "claude-desktop-unavailable")
        });
    }
    handle_inline(shared, request).await
}

async fn handle_inline(shared: &DaemonShared, request: &Request) -> Response {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Configure {
        model: String,
        #[serde(default)]
        background_model: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    if request.method != "claude_desktop_configure"
        && serde_json::from_value::<Empty>(request.params.clone()).is_err()
    {
        return Response::err(
            request.id,
            "invalid_params",
            "claude-desktop-invalid-params",
        );
    }
    let explicit = matches!(
        request.method.as_str(),
        "claude_desktop_configure" | "claude_desktop_preset"
    );
    match request.method.as_str() {
        "claude_desktop_disconnect" => stop(shared, true).await,
        "claude_desktop_configure" => {
            let Ok(params) = serde_json::from_value::<Configure>(request.params.clone()) else {
                return Response::err(
                    request.id,
                    "invalid_params",
                    "claude-desktop-invalid-params",
                );
            };
            let Ok(key) = eligible_key(shared) else {
                reconcile_inline(shared).await;
                return Response::ok(request.id, value(shared, false));
            };
            if load(&shared.store).is_err() {
                stop(shared, true).await;
                shared
                    .claude_desktop
                    .lock()
                    .expect("desktop connection lock")
                    .unavailable = true;
                return Response::ok(
                    request.id,
                    json!({"state":"unavailable","observed":false,"copy":copy()}),
                );
            }
            let pending = {
                let settings = shared.settings.lock().expect("settings lock");
                let enabled = rollout_enabled(shared);
                let mut held = shared
                    .claude_desktop
                    .lock()
                    .expect("desktop connection lock");
                if held.terminating
                    || !enabled
                    || !settings.private_inference
                    || shared.shutdown.load(Ordering::Acquire)
                    || settings
                        .near_ai_inference
                        .as_ref()
                        .is_none_or(|credential| credential.key != key)
                {
                    drop(held);
                    drop(settings);
                    return Response::ok(request.id, value(shared, false));
                }
                let generation = held.advance();
                if shared.store.remove_daemon_file(FILE).is_err() {
                    held.unavailable = true;
                    drop(held);
                    drop(settings);
                    return Response::ok(request.id, value(shared, false));
                }
                held.binding = None;
                held.starting = true;
                let binding = Binding {
                    version: 1,
                    id: Uuid::new_v4(),
                    project: format!("nearai-{}", Uuid::new_v4().simple()),
                    port: 0,
                    background_model: params
                        .background_model
                        .unwrap_or_else(|| params.model.clone()),
                    model: params.model,
                    key_sha256: fingerprint(&key),
                    compact_window: 100_000,
                    output_tokens: 1,
                };
                held.pending_key_sha256 = Some(binding.key_sha256.clone());
                let (done, receiver) = tokio::sync::watch::channel(false);
                let prior_starts = std::mem::take(&mut held.start_done);
                held.start_done.push(receiver);
                (
                    binding,
                    generation,
                    held.changed.subscribe(),
                    held.route.take(),
                    std::sync::Arc::new(StartCompletion(done)),
                    prior_starts,
                )
            };
            let mut guard = StartGuard {
                shared,
                generation: pending.1,
                completed: false,
                done: Some(pending.4.clone()),
            };
            if let Some(route) = pending.3 {
                let (_, drain) = route.stop_desktop(Some(pending.4.clone())).await;
                shared
                    .claude_desktop
                    .lock()
                    .expect("desktop connection lock")
                    .drains
                    .push(drain);
            }
            for mut done in pending.5 {
                while !*done.borrow_and_update() {
                    if done.changed().await.is_err() {
                        break;
                    }
                }
            }
            start(shared, pending.0, key, pending.1, pending.2, pending.4).await;
            guard.completed = true;
        }
        _ => reconcile_inline(shared).await,
    }
    Response::ok(request.id, value(shared, explicit))
}

#[cfg(test)]
#[path = "claude_desktop_tests.rs"]
pub(super) mod tests;
