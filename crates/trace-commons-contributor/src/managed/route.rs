//! One NEAR AI proxy and host-owned credential source per managed session.
use super::{ManagedError, secrets::ApiKey};
use crate::config::ConfigStore;
use ironwire_proxy::embed::{
    self, EmbedOptions, EmbeddedProxy, HostSecret, StartupProbes, UpdateChecks,
};
use std::path::PathBuf;
use uuid::Uuid;

pub struct ManagedRoute {
    proxy: EmbeddedProxy,
    home: PathBuf,
}

impl ManagedRoute {
    pub async fn start(store: &ConfigStore, id: Uuid, key: ApiKey) -> Result<Self, ManagedError> {
        Self::start_at(store, id, key, "https://cloud-api.near.ai/v1").await
    }

    async fn start_at(
        store: &ConfigStore,
        id: Uuid,
        key: ApiKey,
        base_url: &str,
    ) -> Result<Self, ManagedError> {
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
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .map_err(|_| ManagedError::LaunchUnknown)?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        let selected_model = loop {
            let response = client
                .get(format!(
                    "http://127.0.0.1:{}/openai/v1/models",
                    proxy.port()
                ))
                .send()
                .await;
            if let Ok(response) = response {
                if let Ok(value) = response.json::<serde_json::Value>().await {
                    if let Some(model) = value["data"]
                        .as_array()
                        .and_then(|models| models.first())
                        .and_then(|model| model["id"].as_str())
                    {
                        break model.to_owned();
                    }
                }
            }
            if tokio::time::Instant::now() >= deadline {
                proxy.shutdown().await;
                let _ = std::fs::remove_dir_all(&home);
                return Err(ManagedError::SignInRequired);
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
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
        Ok(Self { proxy, home })
    }

    pub fn port(&self) -> u16 {
        self.proxy.port()
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
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async { Json(json!({"data":[{"id":"deepseek-ai/DeepSeek-V3.1"}]})) }),
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
        let a = ManagedRoute::start_at(
            &store,
            Uuid::new_v4(),
            ApiKey::new("fixture-a".into()).unwrap(),
            &endpoint,
        )
        .await
        .unwrap();
        let b = ManagedRoute::start_at(
            &store,
            Uuid::new_v4(),
            ApiKey::new("fixture-b".into()).unwrap(),
            &endpoint,
        )
        .await
        .unwrap();
        assert_ne!(a.port(), b.port());
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        for route in [&a, &b] {
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
                    "deepseek-ai/DeepSeek-V3.1".into()
                ),
                (
                    "Bearer fixture-b".into(),
                    "deepseek-ai/DeepSeek-V3.1".into()
                )
            ]
        );
        a.shutdown().await;
        b.shutdown().await;
        server.abort();
    }
}
