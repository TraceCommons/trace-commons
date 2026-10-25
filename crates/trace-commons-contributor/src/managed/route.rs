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
    shutdown: tokio::sync::oneshot::Sender<()>,
    owner: tokio::task::JoinHandle<()>,
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
                .map(|models| super::models::offered(tool, models))
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
        let bodies = crate::daemon::settings::DaemonSettings::load(store)
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
responses_via_chat = {responses_via_chat}
"#,
                    base_url = toml::Value::String(base_url.into()),
                    bodies = bodies,
                    responses_via_chat = tool == ToolId::Codex
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
        let port = proxy.port();
        let (shutdown, mut requested) = tokio::sync::oneshot::channel();
        let consent_store = store.clone();
        let retained_home = home.clone();
        let owner = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
            loop {
                tokio::select! {
                    // Dropping a route also closes the sender and stops its
                    // listener. Reenabling consent never revives this route.
                    _ = &mut requested => break,
                    _ = interval.tick(), if bodies => {
                        if !crate::daemon::settings::DaemonSettings::load(&consent_store)
                            .is_ok_and(|settings| settings.ironwire_attested_bodies)
                        {
                            break;
                        }
                    }
                }
            }
            // Stop accepting calls before draining previously consented ones.
            // Proof workers may stop with pending rows; never invent settlement.
            proxy.shutdown().await;
            let _ = std::fs::remove_file(retained_home.join("control.token"));
        });
        Ok(Self {
            port,
            shutdown,
            owner,
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

    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.owner.await;
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
            assert!(config.contains(if route.port() == a.port() {
                "responses_via_chat = false"
            } else {
                "responses_via_chat = true"
            }));
            let listed: Vec<&str> = route.models().iter().map(|m| m.id.as_str()).collect();
            // Codex's routes leave out the model NEAR AI refuses Codex.
            let offered: Vec<&str> = chat_ids
                .iter()
                .copied()
                .filter(|id| route.port() == a.port() || *id != "openai/gpt-6.1-sol")
                .collect();
            assert_eq!(listed, offered);
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

    /// Codex replays an answer as output_text on the next turn. NEAR AI
    /// accepts that same assistant text only as input_text in request history.
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
                        content.iter().any(|part| part["type"] == "output_text")
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
            // Non-Codex routes retain the native Responses lane.
            ToolId::Claude,
            Some("deepseek/deepseek-v4.1-flash-1017"),
        )
        .await
        .unwrap();
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let url = format!("http://127.0.0.1:{}/openai/v1/responses", route.port());
        let first = json!({"type":"message","role":"user",
            "content":[{"type":"input_text","text":"Say OK."}]});
        let history = json!([
            first,
            {"type":"message","role":"assistant","status":"completed",
                "content":[{"type":"output_text","text":"OK","annotations":[]}]},
            {"type":"message","role":"user",
                "content":[{"type":"input_text","text":"Say OK again."}]}
        ]);
        let mut outcomes = vec![];
        for input in [json!([first]), history.clone()] {
            let result = client
                .post(&url)
                .json(&json!({"model":route.default_model(),"input":input,"stream":false}))
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
        let mut expected = history;
        expected[1]["content"][0]["type"] = json!("input_text");
        assert_eq!(seen[1]["input"], expected);
    }

    /// The hosted model's Responses stream can finish without the tool call
    /// its Chat stream carries. Codex must receive the call and be able to
    /// replay its result, rather than silently ending an otherwise successful turn.
    #[tokio::test]
    async fn codex_tools_execute_and_replay_results_through_near_chat() {
        const MODEL: &str = "abliteration/abliterated-model";
        type Seen = Arc<Mutex<Vec<Value>>>;
        fn sse(events: Vec<Value>) -> ([(&'static str, &'static str); 1], String) {
            let mut body = events
                .into_iter()
                .map(|event| format!("data: {event}\n\n"))
                .collect::<String>();
            body.push_str("data: [DONE]\n\n");
            ([("content-type", "text/event-stream")], body)
        }
        async fn blank_response() -> ([(&'static str, &'static str); 1], String) {
            sse(vec![json!({"type":"response.completed","response":{
                "id":"resp_empty","object":"response","status":"completed",
                "model":MODEL,"output":[]
            }})])
        }
        async fn chat(
            State(seen): State<Seen>,
            Json(body): Json<Value>,
        ) -> ([(&'static str, &'static str); 1], String) {
            let replay = body["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|message| message["role"] == "tool");
            seen.lock().unwrap().push(body);
            let chunk = |delta: Value, finish: Value| {
                json!({
                    "id":"chatcmpl_fixture","object":"chat.completion.chunk",
                    "created":1,"model":MODEL,
                    "choices":[{"index":0,"delta":delta,"finish_reason":finish}]
                })
            };
            if replay {
                sse(vec![
                    chunk(
                        json!({"role":"assistant","content":"Tool returned synthetic-tool-check."}),
                        Value::Null,
                    ),
                    chunk(json!({}), json!("stop")),
                ])
            } else {
                sse(vec![
                    chunk(
                        json!({"role":"assistant","tool_calls":[{
                            "index":0,"id":"call_probe","type":"function",
                            "function":{"name":"probe_echo","arguments":"{\"marker\":"}
                        }]}),
                        Value::Null,
                    ),
                    chunk(
                        json!({"tool_calls":[{
                            "index":0,"function":{"arguments":"\"synthetic-tool-check\"}"}
                        }]}),
                        Value::Null,
                    ),
                    chunk(json!({}), json!("tool_calls")),
                ])
            }
        }
        fn events(body: &str) -> Vec<Value> {
            body.lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .filter(|data| *data != "[DONE]")
                .map(|data| serde_json::from_str(data).unwrap())
                .collect()
        }
        let seen: Seen = Arc::new(Mutex::new(vec![]));
        let app = Router::new()
            .route(
                "/v1/models",
                get(|| async {
                    Json(json!({"data":[{"id":MODEL,
                    "input_modalities":["text"],"output_modalities":["text"]}]}))
                }),
            )
            .route("/v1/responses", post(blank_response))
            .route("/v1/chat/completions", post(chat))
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
        let tool = json!({"type":"function","name":"probe_echo",
            "description":"Return a marker",
            "parameters":{"type":"object","properties":{"marker":{"type":"string"}},
                "required":["marker"],"additionalProperties":false}});
        let user = json!({"role":"user","content":[{"type":"input_text",
            "text":"Call probe_echo with marker synthetic-tool-check."}]});
        let request = |input: Value, choice: &str| {
            json!({
                "model":MODEL,"stream":true,"input":input,"tools":[tool],"tool_choice":choice
            })
        };
        let first = client
            .post(&url)
            .header("session-id", "managed-tool-roundtrip")
            .json(&request(json!([user]), "required"))
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
            .map(|event| event["item"].clone());
        let Some(call) = call else {
            route.shutdown().await;
            server.abort();
            panic!("successful Codex turn lost its executable tool call: {first}");
        };
        assert_eq!(call["name"], "probe_echo");
        assert_eq!(call["call_id"], "call_probe");
        let arguments: Value = serde_json::from_str(call["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(arguments, json!({"marker":"synthetic-tool-check"}));
        // A harmless executor echoes the marker from the received arguments.
        let result = json!({"type":"function_call_output","call_id":call["call_id"],
            "output":arguments["marker"]});
        let second = client
            .post(&url)
            .header("session-id", "managed-tool-roundtrip")
            .json(&request(json!([user, call, result]), "auto"))
            .send()
            .await
            .unwrap();
        assert!(second.status().is_success());
        let second = second.text().await.unwrap();
        route.shutdown().await;
        server.abort();
        let second_events = events(&second);
        assert!(
            second_events
                .iter()
                .any(|event| event["type"] == "response.completed"),
            "{second}"
        );
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
        assert_eq!(message["content"][0]["text"], text);
        assert!(second_events.iter().any(|event| {
            event["type"] == "response.output_item.done" && event["item"] == *message
        }));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["model"], MODEL);
        assert_eq!(seen[0]["tool_choice"], "required");
        assert_eq!(seen[0]["tools"][0]["function"]["name"], "probe_echo");
        assert_eq!(
            seen[0]["tools"][0]["function"]["parameters"],
            tool["parameters"]
        );
        assert_eq!(seen[1]["messages"][0], seen[0]["messages"][0]);
        assert_eq!(seen[1]["messages"].as_array().unwrap().len(), 3);
        let replay_call = &seen[1]["messages"][1];
        assert_eq!(replay_call["role"], "assistant");
        assert_eq!(replay_call["tool_calls"][0]["id"], "call_probe");
        assert_eq!(
            replay_call["tool_calls"][0]["function"]["name"],
            "probe_echo"
        );
        let replay_arguments: Value = serde_json::from_str(
            replay_call["tool_calls"][0]["function"]["arguments"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(replay_arguments, arguments);
        assert_eq!(
            seen[1]["messages"][2],
            json!({
                "role":"tool","tool_call_id":"call_probe","content":"synthetic-tool-check"
            })
        );
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
        assert!(line.starts_with("ironwire"), "{line}");

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
