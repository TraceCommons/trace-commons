//! One NEAR AI proxy and host-owned credential source per managed session.
use super::{ManagedError, ToolId, secrets::ApiKey};
use crate::config::ConfigStore;
use ironwire_proxy::embed::{
    self, EmbedOptions, EmbeddedProxy, HostSecret, StartupProbes, UpdateChecks,
};
use std::path::PathBuf;
use uuid::Uuid;

pub struct ManagedRoute {
    proxy: EmbeddedProxy,
    home: PathBuf,
    /// The NEAR AI model this route pinned: the requested one, or the
    /// tool's default ([`super::models::select`]) when none was requested.
    model: String,
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

/// How long a route waits for NEAR AI's catalogue and then for its proxy
/// to serve the selected model.
const READY_WITHIN: std::time::Duration = std::time::Duration::from_secs(15);

impl ManagedRoute {
    /// Start a proxy for one session of `tool` and pin it to `requested`,
    /// or to the tool's default model when none was requested. A requested
    /// model NEAR AI does not list as a chat model starts nothing:
    /// [`ManagedError::ModelUnavailable`].
    pub async fn start(
        store: &ConfigStore,
        id: Uuid,
        key: ApiKey,
        tool: ToolId,
        requested: Option<&str>,
    ) -> Result<Self, ManagedError> {
        Self::start_at(store, id, key, near_ai_base_url(), tool, requested).await
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
        // Choose first: a model that cannot be pinned starts no proxy. An
        // empty catalogue is read again, as an unreadable one is.
        let selected_model = loop {
            if let Some(models) = super::models::fetch(&client, base_url)
                .await
                .filter(|models| !models.is_empty())
            {
                break super::models::select(tool, requested, &models)?;
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(ManagedError::SignInRequired);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        let root = store.dir().join("managed-routes");
        super::profiles::private_directory(&root)?;
        let home = root.join(id.to_string());
        super::profiles::private_directory(&home)?;
        let config =
            ConfigStore::open(home.clone()).map_err(|_| ManagedError::StorageUnavailable)?;
        config
            .write_daemon_file(
                "config.toml",
                format!(
                    r#"[capture]
enabled = false
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
        // selected model.
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
            .json(&serde_json::json!({"backend":"nearai","model":selected_model}))
            .send()
            .await
            .map_err(|_| ManagedError::LaunchUnknown)?;
        if !pinned.status().is_success() {
            return Err(ManagedError::LaunchUnknown);
        }
        Ok(Self {
            proxy,
            home,
            model: selected_model,
        })
    }

    pub fn port(&self) -> u16 {
        self.proxy.port()
    }

    /// The model this route pinned, so a tool that must name one asks for
    /// the model the proxy already serves.
    pub fn model(&self) -> &str {
        &self.model
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
        // model first: neither may be pinned unless asked for.
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
        // The model each route pinned: the tool's default when none is
        // requested, never the first listed; the requested one otherwise.
        assert_eq!(a.model(), "anthropic/claude-sonnet-5-5");
        assert_eq!(b.model(), "selected-model");
        assert_eq!(c.model(), "openai/gpt-6.1-sol");
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
        for route in [&a, &b, &c] {
            let result = client.post(format!("http://127.0.0.1:{}/anthropic/v1/messages", route.port()))
                .header("anthropic-version", "2023-06-01")
                .json(&json!({"model":"claude-sonnet-4-6","max_tokens":16,"messages":[{"role":"user","content":"Hello"}]}))
                .send().await.unwrap();
            assert!(
                result.status().is_success(),
                "{}",
                result.text().await.unwrap()
            );
            let config = std::fs::read_to_string(route.home.join("config.toml")).unwrap();
            assert!(!config.contains("fixture-"));
        }
        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                (
                    "Bearer fixture-a".into(),
                    "anthropic/claude-sonnet-5-5".into()
                ),
                ("Bearer fixture-b".into(), "selected-model".into()),
                ("Bearer fixture-c".into(), "openai/gpt-6.1-sol".into()),
            ]
        );
        a.shutdown().await;
        b.shutdown().await;
        c.shutdown().await;
        server.abort();
    }
}
