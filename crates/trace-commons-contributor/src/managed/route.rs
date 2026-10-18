//! One NEAR AI proxy and host-owned credential source per managed session.
use super::{ManagedError, ToolId, secrets::ApiKey};
use crate::config::ConfigStore;
use ironwire_proxy::embed::{
    self, EmbedOptions, EmbeddedProxy, HostSecret, StartupProbes, UpdateChecks,
};
use std::path::PathBuf;
use uuid::Uuid;

/// The route pins the NEAR AI backend only, never a model: a request that
/// names a model NEAR AI lists is served as that model, so each tool's own
/// model picker works. A request naming an id NEAR AI does not list is
/// given a listed model of the same tier by IronWire's routing, so every
/// launcher maps its tool's model names onto [`ManagedRoute::models`].
pub struct ManagedRoute {
    proxy: EmbeddedProxy,
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
    "https://cloud-api.near.ai/v1"
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
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|_| ManagedError::LaunchUnknown)?;
        let deadline = tokio::time::Instant::now() + READY_WITHIN;
        // Choose first: a default that is not listed starts no proxy. An
        // empty catalogue is read again, as an unreadable one is.
        let (models, selected_model) = loop {
            if let Some(models) = super::models::fetch(&client, base_url)
                .await
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
        let home = home_for(store, id);
        if let Some(root) = home.parent() {
            super::profiles::private_directory(root)?;
        }
        super::profiles::private_directory(&home)?;
        let config =
            ConfigStore::open(home.clone()).map_err(|_| ManagedError::StorageUnavailable)?;
        config
            .write_daemon_file(
                "config.toml",
                // The call log is on, as the person's record of the session;
                // the status line does not read it. Bodies are off, so a
                // completed call is recorded as what it was (models, tokens,
                // timings, status), never what it said. A refused call's
                // error column holds IronWire's text for the error, which
                // includes up to 600 characters of the provider's error
                // message, and a provider can quote the request there. The
                // log lives in this home and goes with it at shutdown.
                format!(
                    r#"[capture]
enabled = true
bodies = false
[privacy]
mode = "full"
trusted_backends = ["nearai"]
[[backends]]
id = "nearai"
kind = "nearai"
base_url = {base_url}
"#,
                    base_url = toml::Value::String(base_url.into())
                )
                .as_bytes(),
            )
            .map_err(|_| ManagedError::StorageUnavailable)?;
        let options = EmbedOptions::default()
            .with_update_checks(UpdateChecks::Off)
            .with_startup_probes(StartupProbes::Configured)
            .with_credentials(move |name| {
                (name == "NEARAI_API_KEY").then(|| HostSecret::from(key.expose().to_owned()))
            });
        let proxy = embed::start_with_options(&home, Some(0), options, |_, _| {})
            .await
            .map_err(|_| ManagedError::LaunchUnknown)?;
        // The proxy is ready, and its key accepted, once it serves the
        // default model.
        let deadline = tokio::time::Instant::now() + READY_WITHIN;
        loop {
            let response = client
                .get(format!(
                    "http://127.0.0.1:{}/openai/v1/models",
                    proxy.port()
                ))
                .send()
                .await;
            if let Ok(response) = response {
                if let Ok(value) = response.json::<serde_json::Value>().await {
                    if value["data"].as_array().is_some_and(|models| {
                        models
                            .iter()
                            .any(|model| model["id"].as_str() == Some(selected_model.as_str()))
                    }) {
                        break;
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                proxy.shutdown().await;
                let _ = std::fs::remove_dir_all(&home);
                return Err(ManagedError::SignInRequired);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let token = std::fs::read_to_string(home.join("control.token"))
            .map_err(|_| ManagedError::StorageUnavailable)?;
        let pinned = client
            .post(format!("http://127.0.0.1:{}/_ironwire/pin", proxy.port()))
            .bearer_auth(token.trim())
            // The backend only: a model here would replace every model a
            // tool asks for with this one.
            .json(&serde_json::json!({"backend":"nearai"}))
            .send()
            .await
            .map_err(|_| ManagedError::LaunchUnknown)?;
        if !pinned.status().is_success() {
            return Err(ManagedError::LaunchUnknown);
        }
        Ok(Self {
            proxy,
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
        self.proxy.port()
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

    pub async fn shutdown(self) {
        self.proxy.shutdown().await;
        let _ = std::fs::remove_dir_all(self.home);
    }
}

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
        assert_eq!(c.default_model(), "openai/gpt-6.1-sol");
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
            let listed: Vec<&str> = route.models().iter().map(|m| m.id.as_str()).collect();
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
                "openai/gpt-6.1-sol",
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

    /// A real route serves the `statusline` subcommand's line from its
    /// home, which is where the launcher computes it, and its call log
    /// records the exchange but never a request body.
    #[tokio::test]
    async fn a_route_serves_the_status_line_and_logs_calls_without_bodies() {
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
        // The launcher of a sign-in session computes the daemon's route
        // home from the session id alone.
        assert_eq!(route.home(), home_for(&store, id));
        let line = crate::managed::statusline::line(&home_for(&store, id), None)
            .await
            .expect("a line from a running route");
        assert!(line.starts_with("ironwire"), "{line}");

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = client
            .post(format!(
                "http://127.0.0.1:{}/anthropic/v1/messages",
                route.port()
            ))
            .header("anthropic-version", "2023-06-01")
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
        // No file anywhere under the home holds the request body.
        let mut pending = vec![route.home().to_path_buf()];
        let mut files = 0;
        while let Some(path) = pending.pop() {
            if path.is_dir() {
                pending.extend(std::fs::read_dir(&path).unwrap().map(|e| e.unwrap().path()));
            } else if let Ok(bytes) = std::fs::read(&path) {
                files += 1;
                assert!(
                    !bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()),
                    "{}",
                    path.display()
                );
            }
        }
        assert!(files > 0);
        assert!(!route.home().join("bodies").exists());
        let home = route.home().to_path_buf();
        route.shutdown().await;
        assert!(!home.exists());
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
