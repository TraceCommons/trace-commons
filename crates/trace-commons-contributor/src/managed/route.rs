//! One NEAR AI proxy and host-owned credential source per managed session.
use super::{ManagedError, ToolId, secrets::ApiKey};
use crate::config::ConfigStore;
use ironwire_proxy::embed::{self, EmbedOptions, HostSecret, StartupProbes, UpdateChecks};
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

/// The route pins the NEAR AI backend only, never a model: a request that
/// names a model NEAR AI lists is served as that model, so each tool's own
/// model picker works. A request naming an id NEAR AI does not list is
/// given a listed model of the same tier by IronWire's routing, so every
/// launcher maps its tool's model names onto [`ManagedRoute::models`].
pub struct ManagedRoute {
    port: u16,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    owner: tokio::task::JoinHandle<()>,
    listener_stopped: tokio::sync::watch::Receiver<bool>,
    runtime: tokio::runtime::Handle,
    home: PathBuf,
    /// NEAR AI's chat models, as read when the route started.
    models: Vec<super::models::ChatModel>,
    /// The model a tool starts on: the requested one, or the tool's
    /// default ([`super::models::select`]) when none was requested.
    default_model: String,
}

/// A test build's NEAR AI endpoint, so a daemon-level test can start a
/// route against a local fixture. Absent from every other build.
#[cfg(test)]
pub(crate) static TEST_BASE_URL: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// The NEAR AI endpoint routes and the model catalogue use.
pub(crate) fn near_ai_base_url() -> &'static str {
    #[cfg(test)]
    if let Some(base_url) = TEST_BASE_URL.get() {
        return base_url;
    }
    crate::channel::NEAR_AI_INFERENCE_BASE
}

/// The IronWire home of session `id`'s route: `<config dir>/managed-routes/
/// <id>`, whichever process hosts it. The launcher of a sign-in session,
/// whose route the daemon hosts, finds it here from the session id.
pub fn home_for(store: &ConfigStore, id: Uuid) -> PathBuf {
    store.dir().join("managed-routes").join(id.to_string())
}

/// How long a route waits for NEAR AI's catalogue and then for its proxy
/// to serve the selected model.
const READY_WITHIN: std::time::Duration = std::time::Duration::from_secs(15);

async fn stop_desktop_proxy(
    mut proxy: embed::EmbeddedProxy,
    completion: Option<Arc<crate::daemon::claude_desktop::StartCompletion>>,
) {
    let (closed, wait) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        proxy.stop_accepting().await;
        drop(completion);
        let _ = closed.send(());
        proxy.shutdown().await;
    });
    let _ = wait.await;
}

struct DesktopRoute<'a> {
    binding: &'a crate::daemon::claude_desktop::Binding,
    cancelled: tokio::sync::watch::Receiver<u64>,
    completion: Arc<crate::daemon::claude_desktop::StartCompletion>,
}

struct StartingProxy {
    proxy: Option<embed::EmbeddedProxy>,
    completion: Option<Arc<crate::daemon::claude_desktop::StartCompletion>>,
}
impl Drop for StartingProxy {
    fn drop(&mut self) {
        if let (Some(mut proxy), Some(completion)) = (self.proxy.take(), self.completion.take()) {
            // A cancelled caller cannot acknowledge cleanup before the socket
            // is released. The completion guard outlives the request future.
            tokio::spawn(async move {
                proxy.stop_accepting().await;
                drop(completion);
                proxy.shutdown().await;
            });
        }
    }
}

async fn cancelled(receiver: &mut Option<tokio::sync::watch::Receiver<u64>>) {
    match receiver {
        Some(receiver) => {
            let _ = receiver.changed().await;
        }
        None => std::future::pending::<()>().await,
    }
}

impl ManagedRoute {
    /// Start a proxy for one session of `tool`, pinned to the NEAR AI
    /// backend, whose default model is `requested`, or the tool's default
    /// when none was requested. A requested model NEAR AI does not list as
    /// a chat model starts nothing: [`ManagedError::ModelUnavailable`].
    pub async fn start(
        store: &ConfigStore,
        id: Uuid,
        key: ApiKey,
        tool: ToolId,
        requested: Option<&str>,
    ) -> Result<Self, ManagedError> {
        Self::start_at(store, id, key, near_ai_base_url(), tool, requested).await
    }

    /// [`ManagedRoute::start`] against `base_url` instead of NEAR AI, for
    /// integration tests, which run the built executable against a route
    /// on a local fixture. Test builds only (`test-credential-store`).
    #[cfg(feature = "test-credential-store")]
    #[doc(hidden)]
    pub async fn start_against(
        store: &ConfigStore,
        id: Uuid,
        key: ApiKey,
        base_url: &str,
        tool: ToolId,
        requested: Option<&str>,
    ) -> Result<Self, ManagedError> {
        Self::start_at(store, id, key, base_url, tool, requested).await
    }

    async fn start_at(
        store: &ConfigStore,
        id: Uuid,
        key: ApiKey,
        base_url: &str,
        tool: ToolId,
        requested: Option<&str>,
    ) -> Result<Self, ManagedError> {
        Self::start_with_binding(store, id, key, base_url, tool, requested, None).await
    }

    /// Dedicated Desktop route: only capability-bound Vertex inference is mounted.
    pub(crate) async fn start_desktop(
        store: &ConfigStore,
        key: ApiKey,
        base_url: &str,
        binding: &crate::daemon::claude_desktop::Binding,
        cancelled: tokio::sync::watch::Receiver<u64>,
        completion: Arc<crate::daemon::claude_desktop::StartCompletion>,
    ) -> Result<Self, ManagedError> {
        tokio::time::timeout(
            READY_WITHIN,
            Self::start_with_binding(
                store,
                binding.id,
                key,
                base_url,
                ToolId::Claude,
                Some(&binding.model),
                Some(DesktopRoute {
                    binding,
                    cancelled,
                    completion,
                }),
            ),
        )
        .await
        .map_err(|_| {
            #[cfg(test)]
            eprintln!("Desktop route startup exceeded its deadline");
            ManagedError::LaunchUnknown
        })?
    }

    async fn start_with_binding(
        store: &ConfigStore,
        id: Uuid,
        key: ApiKey,
        base_url: &str,
        tool: ToolId,
        requested: Option<&str>,
        desktop: Option<DesktopRoute<'_>>,
    ) -> Result<Self, ManagedError> {
        let (desktop, mut cancellation, completion) = match desktop {
            Some(options) => (
                Some(options.binding),
                Some(options.cancelled),
                Some(options.completion),
            ),
            None => (None, None, None),
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|_| ManagedError::LaunchUnknown)?;
        let deadline = tokio::time::Instant::now() + READY_WITHIN;
        #[cfg(test)]
        if desktop.is_some() {
            eprintln!("Desktop route startup: fetching catalogue");
        }
        // Choose first: a default that is not listed starts no proxy. An
        // empty catalogue is read again, as an unreadable one is.
        let (models, selected_model) = loop {
            let fetched = tokio::select! {
                biased;
                _ = cancelled(&mut cancellation) => return Err(ManagedError::LaunchUnknown),
                fetched = super::models::fetch(&client, base_url) => fetched,
            };
            if let Some(models) = fetched
                .filter(|models| !models.is_empty())
            {
                let selected = super::models::select(tool, requested, &models)?;
                break (models, selected);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ManagedError::SignInRequired);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        if let Some(binding) = desktop {
            let background = super::models::background_for(
                tool,
                &selected_model,
                Some(&binding.background_model),
                &models,
            )
            .ok_or(ManagedError::ModelUnavailable)?;
            let budget = crate::daemon::claude_desktop::context_budget(
                &selected_model,
                &binding.background_model,
                &models,
            );
            if background.id != binding.background_model
                || budget.is_none()
                || (binding.port != 0
                    && budget.is_some_and(|(compact, output)| {
                        compact < binding.compact_window || output < binding.output_tokens
                    }))
            {
                return Err(ManagedError::ModelUnavailable);
            }
        }
        #[cfg(test)]
        if desktop.is_some() {
            eprintln!("Desktop route startup: preparing private storage");
        }
        let home = home_for(store, id);
        if let Some(root) = home.parent() {
            super::profiles::private_directory(root)?;
        }
        super::profiles::private_directory(&home)?;
        if desktop.is_some() {
            for entry in std::fs::read_dir(&home).map_err(|_| ManagedError::StorageUnavailable)? {
                let entry = entry.map_err(|_| ManagedError::StorageUnavailable)?;
                if std::fs::symlink_metadata(entry.path())
                    .map_err(|_| ManagedError::StorageUnavailable)?
                    .file_type()
                    .is_symlink()
                {
                    return Err(ManagedError::StorageUnavailable);
                }
            }
        }
        let bodies = desktop.is_none()
            && crate::daemon::settings::DaemonSettings::load(store)
                .map_err(|_| ManagedError::StorageUnavailable)?
                .ironwire_attested_bodies;
        let config =
            ConfigStore::open(home.clone()).map_err(|_| ManagedError::StorageUnavailable)?;
        config
            .write_daemon_file(
                "config.toml",
                // The private ledger retains provider errors locally, including
                // possible quoted request text. Readers expose only safe fields.
                // Receipt checking never grants body or token consent.
                format!(
                    r#"[capture]
enabled = true
bodies = {bodies}
receipts = true
[privacy]
mode = "full"
trusted_backends = ["nearai"]
[[backends]]
id = "nearai"
kind = "nearai"
base_url = {base_url}
"#,
                    base_url = toml::Value::String(base_url.into()),
                    bodies = bodies,
                )
                .as_bytes(),
            )
            .map_err(|_| ManagedError::StorageUnavailable)?;
        let attestor = crate::daemon::ipc::signer_attestor_for(
            store,
            Some(key.expose().to_owned()),
            Arc::new(crate::routing::proof_attestor::PinProvider::for_daemon(
                if cfg!(test) {
                    None
                } else {
                    Some(store.clone())
                },
                &|name| std::env::var(name).ok(),
            )),
        );
        let mut options = EmbedOptions::default()
            .with_update_checks(UpdateChecks::Off)
            .with_startup_probes(StartupProbes::Configured)
            .with_credentials(move |name| {
                (name == "NEARAI_API_KEY").then(|| HostSecret::from(key.expose().to_owned()))
            });
        options.token_capture_enabled = Some(false);
        if let Some(attestor) = attestor {
            options = options.with_signer_attestor(attestor);
        }
        if let Some(binding) = desktop {
            options = options.with_vertex_gateway(
                embed::VertexGateway::new(
                    binding.project.clone(),
                    "nearai".into(),
                    binding.model.clone(),
                    Some(binding.background_model.clone()),
                )
                .map_err(|_| ManagedError::InvalidRequest)?,
            );
        }
        #[cfg(test)]
        if desktop.is_some() {
            eprintln!("Desktop route startup: starting embedded proxy");
        }
        let proxy = tokio::select! {
            biased;
            _ = cancelled(&mut cancellation) => return Err(ManagedError::LaunchUnknown),
            proxy = embed::start_with_options(&home, Some(desktop.map_or(0, |b| b.port)), options, |_, _| {}) => {
                proxy.map_err(|_error| {
                    #[cfg(test)]
                    if desktop.is_some() {
                        let label = match _error {
                            embed::EmbedError::Paths => "paths",
                            embed::EmbedError::Config => "config",
                            embed::EmbedError::Lock { .. } => "home-locked",
                            embed::EmbedError::PortInUse { .. } => "port-in-use",
                            embed::EmbedError::Bind => "bind",
                            embed::EmbedError::Registry { .. } => "registry",
                        };
                        eprintln!("Desktop embedded proxy startup failed: {label}");
                    }
                    ManagedError::LaunchUnknown
                })?
            }
        };
        let mut starting = StartingProxy {
            proxy: Some(proxy),
            completion,
        };
        let proxy = starting.proxy.as_ref().expect("starting proxy");
        #[cfg(test)]
        if desktop.is_some() {
            eprintln!("Desktop route startup: waiting for proxy readiness");
        }
        let ready: Result<(), ManagedError> = async {
            let token = std::fs::read_to_string(home.join("control.token"))
                .map_err(|_| ManagedError::StorageUnavailable)?;
            let deadline = tokio::time::Instant::now() + READY_WITHIN;
            loop {
                let response = tokio::select! {
                    biased;
                    _ = cancelled(&mut cancellation) => return Err(ManagedError::LaunchUnknown),
                    response = client.get(format!("http://127.0.0.1:{}/{}", proxy.port(),
                        if desktop.is_some() { "_ironwire/status" } else { "openai/v1/models" }))
                        .bearer_auth(token.trim()).send() => response,
                };
                if let Ok(response) = response {
                    if let Ok(value) = response.json::<serde_json::Value>().await {
                        let bound_ready = desktop.is_some()
                            && value["backends"].as_array().is_some_and(|backends| {
                                backends.iter().any(|backend| {
                                    backend["id"] == "nearai"
                                        && backend["models"].as_array().is_some_and(|models| {
                                            models.iter().any(|model| {
                                                model.as_str() == Some(selected_model.as_str())
                                            })
                                        })
                                })
                            });
                        if bound_ready
                            || value["data"].as_array().is_some_and(|models| {
                                models.iter().any(|model| {
                                    model["id"].as_str() == Some(selected_model.as_str())
                                })
                            })
                        {
                            break;
                        }
                    }
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(ManagedError::SignInRequired);
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            // Ordinary CLI routes pin the backend only. Desktop's scoped
            // gateway independently enforces both selected model bindings.
            #[cfg(test)]
            if desktop.is_some() {
                eprintln!("Desktop route startup: pinning backend");
            }
            let pin = client
                .post(format!("http://127.0.0.1:{}/_ironwire/pin", proxy.port()))
                .bearer_auth(token.trim())
                .json(&serde_json::json!({"backend":"nearai"}))
                .send();
            let pinned = tokio::select! {
                biased;
                _ = cancelled(&mut cancellation) => return Err(ManagedError::LaunchUnknown),
                pinned = pin => pinned.map_err(|_| ManagedError::LaunchUnknown)?,
            };
            if pinned.status().is_success() {
                Ok(())
            } else {
                Err(ManagedError::LaunchUnknown)
            }
        }
        .await;
        if let Err(error) = ready {
            #[cfg(test)]
            if desktop.is_some() {
                eprintln!("Desktop proxy readiness failed: {error:?}");
            }
            let proxy = starting.proxy.take().expect("starting proxy");
            if desktop.is_some() {
                stop_desktop_proxy(proxy, starting.completion.take()).await;
            } else {
                proxy.shutdown().await;
                let _ = std::fs::remove_dir_all(&home);
            }
            return Err(error);
        }
        let proxy = starting.proxy.take().expect("starting proxy");
        let port = proxy.port();
        let (listener_closed, listener_stopped) = tokio::sync::watch::channel(false);
        let (shutdown, mut requested) = tokio::sync::oneshot::channel();
        let consent_store = store.clone();
        let retained_home = home.clone();
        let owner = tokio::spawn(async move {
            let mut proxy = proxy;
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
            loop {
                tokio::select! {
                    // Dropping a route also closes the sender and stops its
                    // listener. Reenabling consent never revives this route.
                    _ = &mut requested => break,
                    _ = proxy.wait() => break,
                    _ = interval.tick(), if bodies => {
                        if !crate::daemon::settings::DaemonSettings::load(&consent_store)
                            .is_ok_and(|settings| settings.ironwire_attested_bodies)
                        {
                            break;
                        }
                    }
                }
            }
            proxy.stop_accepting().await;
            listener_closed.send_replace(true);
            // Stop accepting calls before draining previously consented ones.
            // Proof workers may stop with pending rows; never invent settlement.
            proxy.shutdown().await;
            let _ = std::fs::remove_file(retained_home.join("control.token"));
        });
        Ok(Self {
            port,
            shutdown: Some(shutdown),
            owner,
            listener_stopped,
            runtime: tokio::runtime::Handle::current(),
            home,
            models,
            default_model: selected_model,
        })
    }

    /// This route's IronWire home ([`home_for`]).
    pub fn home(&self) -> &std::path::Path {
        &self.home
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The model a tool starts on. Not a pin: the tool may ask for any of
    /// [`Self::models`].
    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// NEAR AI's chat models, in its order, as read when the route started:
    /// the ids a tool may name and have served as named.
    pub fn models(&self) -> &[super::models::ChatModel] {
        &self.models
    }

    /// Stop accepting Desktop calls while retaining the graceful drain task.
    /// CLI routes continue to use `shutdown`, which waits for complete drain.
    pub(crate) async fn stop_desktop(
        mut self,
        completion: Option<Arc<crate::daemon::claude_desktop::StartCompletion>>,
    ) -> (bool, tokio::task::JoinHandle<()>) {
        self.request_shutdown();
        let (closed, wait) = tokio::sync::oneshot::channel();
        // Own the listener barrier even if the IPC future waiting on it is
        // cancelled. Keep draining accepted requests afterward.
        let runtime = self.runtime.clone();
        let draining = runtime.spawn(async move {
            let stopped = loop {
                if *self.listener_stopped.borrow_and_update() {
                    break true;
                }
                if self.listener_stopped.changed().await.is_err() {
                    break false;
                }
            };
            drop(completion);
            let _ = closed.send(stopped);
            self.shutdown().await;
        });
        (wait.await.unwrap_or(false), draining)
    }

    pub(crate) fn is_finished(&self) -> bool {
        self.owner.is_finished()
    }

    #[cfg(test)]
    pub(crate) fn listener_stopped_for_test(&self) -> tokio::sync::watch::Receiver<bool> {
        self.listener_stopped.clone()
    }

    pub(crate) fn request_shutdown(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
    }

    pub async fn shutdown(mut self) {
        self.request_shutdown();
        let _ = self.owner.await;
    }
}

#[cfg(test)]
#[path = "desktop_route_tests.rs"]
mod desktop_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        extract::State,
        http::HeaderMap,
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn routes_discover_and_translate_models_with_only_their_own_key() {
        type Seen = Arc<Mutex<Vec<(String, String)>>>;
        async fn completion(
            State(seen): State<Seen>,
            headers: HeaderMap,
            Json(body): Json<Value>,
        ) -> Json<Value> {
            seen.lock().unwrap().push((
                headers["authorization"].to_str().unwrap().into(),
                body["model"].as_str().unwrap().into(),
            ));
            Json(
                json!({"id":"fixture","object":"chat.completion","created":1,"model":"deepseek-ai/DeepSeek-V3.1","choices":[{"index":0,"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}),
            )
        }
        let seen: Seen = Arc::new(Mutex::new(vec![]));
        let chat =
            |id: &str| json!({"id":id,"input_modalities":["text"],"output_modalities":["text"]});
        // NEAR AI's order puts a non-chat model and an off-family chat
        // model first: neither is a default unless asked for.
        let models = json!({"data":[
            {"id":"black-forest-labs/FLUX.2-klein-4B","input_modalities":["text"],"output_modalities":["image"]},
            chat("deepseek-ai/DeepSeek-V3.1"),
            chat("anthropic/claude-sonnet-5"),
            chat("anthropic/claude-sonnet-5-5"),
            chat("openai/gpt-6.1-sol"),
            chat("openai/gpt-6-sol"),
            chat("selected-model"),
        ]});
        let app = Router::new()
            .route(
                "/v1/models",
                get(move || {
                    let models = models.clone();
                    async move { Json(models) }
                }),
            )
            .route("/v1/chat/completions", post(completion))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().into()).unwrap();
        let start = |key: &str, tool, model: Option<&'static str>| {
            let store = &store;
            let endpoint = &endpoint;
            let key = ApiKey::new(key.into()).unwrap();
            async move {
                ManagedRoute::start_at(store, Uuid::new_v4(), key, endpoint, tool, model).await
            }
        };
        let a = start("fixture-a", ToolId::Claude, None).await.unwrap();
        let b = start("fixture-b", ToolId::Codex, Some("selected-model"))
            .await
            .unwrap();
        let c = start("fixture-c", ToolId::Codex, None).await.unwrap();
        assert_ne!(a.port(), b.port());
        // Each route's default model: the tool's default when none is
        // requested, never the first listed; the requested one otherwise.
        assert_eq!(a.default_model(), "anthropic/claude-sonnet-5-5");
        assert_eq!(b.default_model(), "selected-model");
        assert_eq!(c.default_model(), "openai/gpt-6-sol");
        // A model NEAR AI does not list as a chat model starts nothing,
        // and says so at once rather than waiting out the sign-in check.
        for refused in ["black-forest-labs/FLUX.2-klein-4B", "no-such-model"] {
            let begun = tokio::time::Instant::now();
            assert_eq!(
                start("fixture-d", ToolId::Claude, Some(refused))
                    .await
                    .err(),
                Some(ManagedError::ModelUnavailable),
                "{refused}"
            );
            assert!(begun.elapsed() < std::time::Duration::from_secs(10));
        }
        let routes = store.dir().join("managed-routes");
        assert_eq!(std::fs::read_dir(&routes).unwrap().count(), 3);
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        // Each route sends three requests: its default model, another
        // listed chat model, and an id NEAR AI does not list.
        let chat_ids = [
            "deepseek-ai/DeepSeek-V3.1",
            "anthropic/claude-sonnet-5",
            "anthropic/claude-sonnet-5-5",
            "openai/gpt-6.1-sol",
            "openai/gpt-6-sol",
            "selected-model",
        ];
        for (route, other) in [
            (&a, "openai/gpt-6.1-sol"),
            (&b, "anthropic/claude-sonnet-5"),
            (&c, "deepseek-ai/DeepSeek-V3.1"),
        ] {
            for model in [route.default_model(), other, "claude-sonnet-4-6"] {
                let result = client.post(format!("http://127.0.0.1:{}/anthropic/v1/messages", route.port()))
                    .header("anthropic-version", "2023-06-01")
                    .json(&json!({"model":model,"max_tokens":16,"messages":[{"role":"user","content":"Hello"}]}))
                    .send().await.unwrap();
                assert!(
                    result.status().is_success(),
                    "{}",
                    result.text().await.unwrap()
                );
            }
            let config = std::fs::read_to_string(route.home.join("config.toml")).unwrap();
            assert!(!config.contains("fixture-"));
            assert!(!config.contains("responses_via_chat"));
            let listed: Vec<&str> = route.models().iter().map(|m| m.id.as_str()).collect();
            // Every tool sees the catalogue's chat models, including models
            // previously excluded for old Chat Completions incompatibilities.
            assert_eq!(listed, chat_ids);
        }
        let seen = seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 9, "{seen:?}");
        let keys: Vec<&str> = seen.iter().map(|(key, _)| key.as_str()).collect();
        assert_eq!(
            keys,
            [
                ["Bearer fixture-a"; 3],
                ["Bearer fixture-b"; 3],
                ["Bearer fixture-c"; 3]
            ]
            .concat()
        );
        let reached: Vec<&str> = seen.iter().map(|(_, model)| model.as_str()).collect();
        // A listed model, the default or another, reaches NEAR AI as named:
        // the pin names the backend only, so each tool's picker works.
        assert_eq!(
            [
                reached[0], reached[1], reached[3], reached[4], reached[6], reached[7]
            ],
            [
                "anthropic/claude-sonnet-5-5",
                "openai/gpt-6.1-sol",
                "selected-model",
                "anthropic/claude-sonnet-5",
                "openai/gpt-6-sol",
                "deepseek-ai/DeepSeek-V3.1",
            ]
        );
        // An unlisted id is not forwarded as is: IronWire picks a listed
        // model of the request's tier in its place (against this fixture,
        // `anthropic/claude-sonnet-5` on all three routes). Which one is
        // IronWire's choice, so only "some listed model" is asserted.
        for unlisted in [reached[2], reached[5], reached[8]] {
            assert!(chat_ids.contains(&unlisted), "{unlisted}");
        }
        a.shutdown().await;
        b.shutdown().await;
        c.shutdown().await;
        server.abort();
    }

    /// Codex owns its history, tool definitions and tool execution. NEAR AI's
    /// native Responses contract accepts those items without reshaping them.
    #[tokio::test]
    async fn native_near_responses_replays_assistant_text_without_losing_history() {
        type Seen = Arc<Mutex<Vec<Value>>>;
        async fn response(
            State(seen): State<Seen>,
            Json(body): Json<Value>,
        ) -> (axum::http::StatusCode, Json<Value>) {
            seen.lock().unwrap().push(body.clone());
            let rejected = body["input"].as_array().unwrap().iter().any(|item| {
                item["role"] == "assistant"
                    && item["content"].as_array().is_some_and(|content| {
                        content.iter().any(|part| part["type"] == "input_text")
                    })
            });
            if rejected {
                return (
                    axum::http::StatusCode::BAD_REQUEST,
                    Json(json!({"error":{"message":"input did not match ResponseInput"}})),
                );
            }
            (
                axum::http::StatusCode::OK,
                Json(json!({
                    "id":"resp_fixture","object":"response","status":"completed",
                    "model":"deepseek/deepseek-v4.1-flash-1017",
                    "output":[{"type":"message","role":"assistant","status":"completed",
                        "content":[{"type":"output_text","text":"OK","annotations":[]}]}],
                    "usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}
                })),
            )
        }
        let seen: Seen = Arc::new(Mutex::new(vec![]));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async {
                    Json(json!({"data":[{
                        "id":"deepseek/deepseek-v4.1-flash-1017",
                        "input_modalities":["text"],"output_modalities":["text"]
                    }]}))
                }),
            )
            .route("/v1/responses", post(response))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().into()).unwrap();
        let route = ManagedRoute::start_at(
            &store,
            Uuid::new_v4(),
            ApiKey::new("fixture-codex".into()).unwrap(),
            &endpoint,
            ToolId::Codex,
            Some("deepseek/deepseek-v4.1-flash-1017"),
        )
        .await
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://127.0.0.1:{}/openai/v1/responses", route.port());
        let first = json!({"type":"message","role":"user",
            "content":[{"type":"input_text","text":"Say OK."}]});
        let function = json!({"type":"function","name":"probe_echo", "parameters":{"type":"object","properties":{},"additionalProperties":false}});
        let custom = json!({"type":"custom","name":"apply_patch","format":{"type":"grammar","syntax":"lark","definition":"start: \"marker\""}});
        let namespace = json!({"type":"namespace","name":"functions","tools":[function.clone()]});
        let tools = json!([custom, namespace]);
        let additional_tools = json!({"type":"additional_tools","tools":[function]});
        let history = json!([
            first,
            additional_tools,
            {"type":"custom_tool_call","id":"ct_fixture","call_id":"call_patch","name":"apply_patch","input":"marker"},
            {"type":"custom_tool_call_output","call_id":"call_patch","output":"marker written"},
            {"type":"function_call","id":"fc_fixture","call_id":"call_echo","namespace":"functions","name":"probe_echo","arguments":"{}"},
            {"type":"function_call_output","call_id":"call_echo","output":"marker"},
            {"type":"message","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":"OK","annotations":[]}]},
            {"type":"message","role":"user",
                "content":[{"type":"input_text","text":"Say OK again."}]}
        ]);
        let mut outcomes = vec![];
        for input in [json!([first]), history.clone()] {
            let result = client
                .post(&url)
                .json(&json!({"model":route.default_model(),"input":input,"stream":false,"store":false,"tools":tools}))
                .send()
                .await
                .unwrap();
            outcomes.push((result.status(), result.text().await.unwrap()));
        }
        route.shutdown().await;
        server.abort();
        assert!(outcomes[0].0.is_success(), "first turn: {:?}", outcomes[0]);
        assert!(outcomes[1].0.is_success(), "follow-up: {:?}", outcomes[1]);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["input"], json!([first]));
        assert_eq!(seen[1]["input"], history);
        for request in seen.iter() {
            assert_eq!(request["tools"], tools);
            assert_eq!(request["store"], false);
        }
    }

    /// Codex uses NEAR AI's stateless Responses endpoint directly. Its
    /// streamed tool call and client-executed result survive the next turn.
    #[tokio::test]
    async fn codex_tools_execute_and_replay_results_through_near_responses() {
        const MODEL: &str = "abliteration/abliterated-model";
        #[derive(Default)]
        struct Seen {
            responses: Mutex<Vec<Value>>,
            chat: std::sync::atomic::AtomicUsize,
        }
        fn sse(events: Vec<Value>) -> ([(&'static str, &'static str); 1], String) {
            let body = events
                .into_iter()
                .map(|event| {
                    format!(
                        "event: {}\ndata: {event}\n\n",
                        event["type"].as_str().unwrap()
                    )
                })
                .collect();
            ([("content-type", "text/event-stream")], body)
        }
        async fn response(
            State(seen): State<Arc<Seen>>,
            Json(body): Json<Value>,
        ) -> ([(&'static str, &'static str); 1], String) {
            let replay = body["input"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["type"] == "function_call_output");
            seen.responses.lock().unwrap().push(body);
            let output = if replay {
                json!({"id":"msg_fixture","type":"message","role":"assistant","status":"completed",
                    "content":[{"type":"output_text","text":"Tool returned synthetic-tool-check.","annotations":[]}]})
            } else {
                json!({"id":"fc_fixture","type":"function_call","status":"completed",
                    "call_id":"call_probe","name":"probe_echo",
                    "arguments":"{\"marker\":\"synthetic-tool-check\"}"})
            };
            let mut events = vec![
                json!({"type":"response.created","response":{"id":"resp_fixture","object":"response","status":"in_progress","output":[]}}),
                json!({"type":"response.output_item.added","output_index":0,"item":output}),
            ];
            if replay {
                events.push(json!({"type":"response.output_text.delta","item_id":"msg_fixture","output_index":0,"content_index":0,"delta":"Tool returned synthetic-tool-check."}));
            } else {
                events.push(json!({"type":"response.function_call_arguments.delta","item_id":"fc_fixture","output_index":0,"delta":output["arguments"]}));
                events.push(json!({"type":"response.function_call_arguments.done","item_id":"fc_fixture","output_index":0,"arguments":output["arguments"]}));
            }
            events.extend([
                json!({"type":"response.output_item.done","output_index":0,"item":output}),
                json!({"type":"response.completed","response":{"id":"resp_fixture","object":"response","status":"completed","model":MODEL,"output":[output],"usage":{"input_tokens":1,"output_tokens":1,"total_tokens":2}}}),
            ]);
            sse(events)
        }
        async fn forbidden_chat(State(seen): State<Arc<Seen>>) -> axum::http::StatusCode {
            seen.chat.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            axum::http::StatusCode::BAD_GATEWAY
        }
        fn events(body: &str) -> Vec<Value> {
            body.lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter(|data| *data != "[DONE]")
                .map(|data| serde_json::from_str(data).unwrap())
                .collect()
        }
        let seen = Arc::new(Seen::default());
        let app = Router::new()
            .route("/v1/models", get(|| async {
                Json(json!({"data":[{"id":MODEL,"input_modalities":["text"],"output_modalities":["text"]}]}))
            }))
            .route("/v1/responses", post(response))
            .route("/v1/chat/completions", post(forbidden_chat))
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().into()).unwrap();
        let route = ManagedRoute::start_at(
            &store,
            Uuid::new_v4(),
            ApiKey::new("fixture-tool".into()).unwrap(),
            &endpoint,
            ToolId::Codex,
            Some(MODEL),
        )
        .await
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://127.0.0.1:{}/openai/v1/responses", route.port());
        let tool = json!({"type":"function","name":"probe_echo","description":"Return a marker",
            "parameters":{"type":"object","properties":{"marker":{"type":"string"}},
                "required":["marker"],"additionalProperties":false}});
        let user = json!({"role":"user","content":[{"type":"input_text","text":"Call probe_echo with marker synthetic-tool-check."}]});
        let request = |input: Value, choice: Value| {
            json!({
                "model":MODEL,"stream":true,"store":false,"input":input,"tools":[tool],
                "tool_choice":choice,"reasoning":{"effort":"low"}
            })
        };
        let first = client
            .post(&url)
            .header("session-id", "managed-tool-roundtrip")
            .json(&request(
                json!([user]),
                json!({"type":"function","name":"probe_echo"}),
            ))
            .send()
            .await
            .unwrap();
        assert!(first.status().is_success());
        let first = first.text().await.unwrap();
        let first_events = events(&first);
        let call = first_events
            .iter()
            .find(|event| {
                event["type"] == "response.output_item.done"
                    && event["item"]["type"] == "function_call"
            })
            .map(|event| event["item"].clone())
            .unwrap_or_else(|| {
                panic!("successful Codex turn lost its executable tool call: {first}")
            });
        assert!(
            first_events
                .iter()
                .any(|event| event["type"] == "response.completed")
        );
        assert_eq!(call["name"], "probe_echo");
        assert_eq!(call["call_id"], "call_probe");
        let arguments: Value = serde_json::from_str(call["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(arguments, json!({"marker":"synthetic-tool-check"}));
        let result = json!({"type":"function_call_output","call_id":call["call_id"],"output":arguments["marker"]});
        let history = json!([user, call, result]);
        let second = client
            .post(&url)
            .header("session-id", "managed-tool-roundtrip")
            .json(&request(history.clone(), json!("auto")))
            .send()
            .await
            .unwrap();
        assert!(second.status().is_success());
        let second = second.text().await.unwrap();
        route.shutdown().await;
        server.abort();
        let second_events = events(&second);
        let text: String = second_events
            .iter()
            .filter(|event| event["type"] == "response.output_text.delta")
            .filter_map(|event| event["delta"].as_str())
            .collect();
        assert_eq!(text, "Tool returned synthetic-tool-check.");
        let completed = second_events
            .iter()
            .find(|event| event["type"] == "response.completed")
            .unwrap();
        let message = &completed["response"]["output"][0];
        assert_eq!(message["type"], "message");
        assert_eq!(message["role"], "assistant");
        assert_eq!(
            message["content"][0]["text"],
            "Tool returned synthetic-tool-check."
        );
        assert!(
            second_events
                .iter()
                .any(|event| event["type"] == "response.output_item.done"
                    && event["item"] == *message)
        );
        assert_eq!(seen.chat.load(std::sync::atomic::Ordering::Relaxed), 0);
        let seen = seen.responses.lock().unwrap();
        assert_eq!(seen.len(), 2);
        for body in seen.iter() {
            assert_eq!(body["model"], MODEL);
            assert_eq!(body["store"], false);
            assert_eq!(body["tools"], json!([tool]));
            assert_eq!(body["reasoning"], json!({"effort":"low"}));
            assert!(body.get("previous_response_id").is_none());
            assert!(body.get("conversation").is_none());
        }
        assert_eq!(
            seen[0]["tool_choice"],
            json!({"type":"function","name":"probe_echo"})
        );
        assert_eq!(seen[1]["tool_choice"], "auto");
        assert_eq!(seen[1]["input"], history);
    }

    /// A real route serves the `statusline` subcommand's line from its
    /// home, which is where the launcher computes it, and its call log
    /// records the exchange but never a request body.
    #[tokio::test]
    async fn a_route_serves_the_status_line_and_logs_calls_without_bodies() {
        retained_route_capture(false, None).await;
    }

    #[tokio::test]
    async fn a_route_retains_bodies_only_with_attested_body_consent() {
        retained_route_capture(true, None).await;
    }

    #[tokio::test]
    async fn revoking_body_consent_stops_the_route_and_retains_history() {
        retained_route_capture(true, Some(false)).await;
    }

    #[tokio::test]
    async fn unreadable_body_consent_stops_the_route_and_retains_history() {
        retained_route_capture(true, Some(true)).await;
    }

    #[tokio::test]
    async fn metadata_only_route_stays_live_when_body_consent_is_unreadable() {
        retained_route_capture(false, Some(true)).await;
    }

    async fn retained_route_capture(bodies: bool, unreadable_consent: Option<bool>) {
        const MARKER: &str = "statusline-body-marker-5f2c9e";
        async fn completion(Json(body): Json<Value>) -> Json<Value> {
            assert!(body.to_string().contains(MARKER));
            Json(
                json!({"id":"fixture","object":"chat.completion","created":1,"model":"anthropic/claude-sonnet-5-5","choices":[{"index":0,"message":{"role":"assistant","content":"Hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}),
            )
        }
        let models = json!({"data":[
            {"id":"anthropic/claude-sonnet-5-5","input_modalities":["text"],"output_modalities":["text"]},
        ]});
        let app = Router::new()
            .route(
                "/v1/models",
                get(move || {
                    let models = models.clone();
                    async move { Json(models) }
                }),
            )
            .route("/v1/chat/completions", post(completion));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().into()).unwrap();
        let mut settings = crate::daemon::settings::DaemonSettings {
            ironwire_attested_bodies: bodies,
            ..Default::default()
        };
        settings.save(&store).unwrap();
        let id = Uuid::new_v4();
        let route = ManagedRoute::start_at(
            &store,
            id,
            ApiKey::new("fixture-key".into()).unwrap(),
            &endpoint,
            ToolId::Claude,
            None,
        )
        .await
        .unwrap();
        let exact_session = format!("ses_{}", id.simple());
        // The launcher of a sign-in session computes the daemon's route
        // home from the session id alone.
        assert_eq!(route.home(), home_for(&store, id));
        let line = crate::managed::statusline::line(&home_for(&store, id), None)
            .await
            .expect("a line from a running route");
        assert!(line.starts_with("claude -> near.ai"), "{line}");

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = client
            .post(format!(
                "http://127.0.0.1:{}/anthropic/v1/messages",
                route.port()
            ))
            .header("anthropic-version", "2023-06-01")
            .header("x-ironwire-session-id", &exact_session)
            .json(&json!({"model":"anthropic/claude-sonnet-5-5","max_tokens":16,"messages":[{"role":"user","content":MARKER}]}))
            .send()
            .await
            .unwrap();
        assert!(result.status().is_success());
        let _ = result.bytes().await.unwrap();
        // The exchange is logged once the response is done; wait for it.
        let ledger = route.home().join("ledger.sqlite");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let logged = |path: &std::path::Path| {
            ["", "-wal"].iter().any(|suffix| {
                let mut file = path.as_os_str().to_owned();
                file.push(suffix);
                std::fs::read(file).is_ok_and(|bytes| {
                    bytes
                        .windows(b"anthropic/claude-sonnet-5-5".len())
                        .any(|w| w == b"anthropic/claude-sonnet-5-5")
                })
            })
        };
        while !logged(&ledger) {
            assert!(std::time::Instant::now() < deadline, "no call logged");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let config: toml::Value =
            toml::from_str(&std::fs::read_to_string(route.home().join("config.toml")).unwrap())
                .unwrap();
        assert_eq!(config["capture"]["bodies"].as_bool(), Some(bodies));
        assert_eq!(config["capture"]["receipts"].as_bool(), Some(true));
        assert!(config["capture"].get("token_capture").is_none());
        let mut marker_seen = false;
        let mut pending = vec![route.home().to_path_buf()];
        let mut files = 0;
        while let Some(path) = pending.pop() {
            if path.is_dir() {
                pending.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else if let Ok(bytes) = std::fs::read(&path) {
                files += 1;
                marker_seen |= bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes());
            }
        }
        assert!(files > 0);
        assert_eq!(marker_seen, bodies);
        assert_eq!(route.home().join("bodies").exists(), bodies);
        let home = route.home().to_path_buf();
        if let Some(unreadable) = unreadable_consent {
            let known_good_settings = store
                .read_daemon_file("daemon-settings.json")
                .unwrap()
                .unwrap();
            if unreadable {
                store
                    .write_daemon_file("daemon-settings.json", b"{")
                    .unwrap();
            } else {
                settings.ironwire_attested_bodies = false;
                settings.save(&store).unwrap();
            }
            if bodies {
                let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
                while home.join("endpoint.json").exists() {
                    assert!(
                        tokio::time::Instant::now() < deadline,
                        "revoked route remained live"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
                assert!(
                    client
                        .get(format!(
                            "http://127.0.0.1:{}/openai/v1/models",
                            route.port()
                        ))
                        .send()
                        .await
                        .is_err()
                );
                if unreadable {
                    // Recover the deliberately corrupted fixture explicitly;
                    // production preference saves correctly reject corruption.
                    store
                        .write_daemon_file("daemon-settings.json", &known_good_settings)
                        .unwrap();
                }
                settings.ironwire_attested_bodies = true;
                settings.save(&store).unwrap();
                assert!(
                    client
                        .get(format!(
                            "http://127.0.0.1:{}/openai/v1/models",
                            route.port()
                        ))
                        .send()
                        .await
                        .is_err(),
                    "reenabling needs a fresh route"
                );
            } else {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                assert!(
                    client
                        .get(format!(
                            "http://127.0.0.1:{}/openai/v1/models",
                            route.port()
                        ))
                        .send()
                        .await
                        .unwrap()
                        .status()
                        .is_success()
                );
                assert!(home.join("endpoint.json").exists());
            }
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), route.shutdown())
            .await
            .expect("explicit shutdown after revocation must finish");
        assert!(home.join("ledger.sqlite").exists());
        assert!(!home.join("control.token").exists());
        assert!(!home.join("endpoint.json").exists());
        assert!(logged(&home.join("ledger.sqlite")));
        let db = rusqlite::Connection::open_with_flags(
            home.join("ledger.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let (session, proof): (Option<String>, Option<String>) = db
            .query_row(
                "SELECT client_session_id, proof FROM exchanges",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(session.as_deref(), Some(exact_session.as_str()));
        assert_ne!(
            proof.as_deref(),
            Some("verified"),
            "an unsigned fixture is never verified"
        );
        assert_eq!(home.join("bodies").exists(), bodies);
        server.abort();
    }

    /// The call log's error column is not body-free. A provider that
    /// rejects a request and quotes it back in its error message has up to
    /// 600 characters of that message (IronWire's `reason`) written to the
    /// log, prompt text included; nothing past them, and no `bodies`
    /// store. Pinned so that a change on either side is seen, and the docs
    /// that state it are updated with it.
    #[tokio::test]
    async fn a_refused_call_logs_up_to_600_characters_of_the_providers_error() {
        const HEAD: &str = "refusal-echo-head-8d1e4a";
        const TAIL: &str = "refusal-echo-tail-3b7f02";
        async fn refuse(Json(body): Json<Value>) -> (axum::http::StatusCode, Json<Value>) {
            // The provider quotes the prompt back in its error message.
            let prompt = body["messages"]
                .as_array()
                .and_then(|m| m.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or_default()
                .to_owned();
            (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({"error":{"message":format!("invalid prompt: {prompt}")}})),
            )
        }
        let models = json!({"data":[
            {"id":"anthropic/claude-sonnet-5-5","input_modalities":["text"],"output_modalities":["text"]},
        ]});
        let app = Router::new()
            .route(
                "/v1/models",
                get(move || {
                    let models = models.clone();
                    async move { Json(models) }
                }),
            )
            .route("/v1/chat/completions", post(refuse));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().into()).unwrap();
        let route = ManagedRoute::start_at(
            &store,
            Uuid::new_v4(),
            ApiKey::new("fixture-key".into()).unwrap(),
            &endpoint,
            ToolId::Claude,
            None,
        )
        .await
        .unwrap();
        let prompt = format!("{HEAD}{}{TAIL}", "x".repeat(700));
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = client
            .post(format!(
                "http://127.0.0.1:{}/anthropic/v1/messages",
                route.port()
            ))
            .header("anthropic-version", "2023-06-01")
            .json(&json!({"model":"anthropic/claude-sonnet-5-5","max_tokens":16,"messages":[{"role":"user","content":prompt}]}))
            .send()
            .await
            .unwrap();
        assert!(result.status().is_client_error(), "{}", result.status());
        let _ = result.bytes().await.unwrap();
        let ledger = route.home().join("ledger.sqlite");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let errors = loop {
            let errors: Vec<(i64, Option<String>)> = rusqlite::Connection::open_with_flags(
                &ledger,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )
            .and_then(|db| {
                db.prepare("SELECT status, error FROM exchanges")?
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .unwrap_or_default();
            if !errors.is_empty() {
                break errors;
            }
            assert!(std::time::Instant::now() < deadline, "no call logged");
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert_eq!(errors.len(), 1, "{errors:?}");
        let (status, error) = &errors[0];
        assert_eq!(*status, 400);
        let error = error.as_deref().expect("an error column");
        assert!(error.contains(HEAD), "{error}");
        assert!(!error.contains(TAIL), "{error}");
        assert!(error.chars().count() <= 700, "{}", error.chars().count());
        assert!(!route.home().join("bodies").exists());
        route.shutdown().await;
        server.abort();
    }
}
