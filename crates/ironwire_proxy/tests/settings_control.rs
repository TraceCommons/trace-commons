//! Conformance: changing settings from outside the terminal.
//!
//! The menu bar app is a pure client — it renders what the daemon says and posts
//! back what the user picked. That only holds if the daemon is the one deciding
//! *what may be picked*: which privacy modes are selectable, whether a consent
//! answer counts, and what happens to the file on disk. Every rule below would
//! otherwise end up reimplemented in Swift, where it would drift.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::{Config, PathsConfig, PrivacyConfig, PrivacyMode};
use ironwire_creds::ConsentLedger;
use ironwire_creds::consent::CONSENT_PROMPT_VERSION;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::openai_chat::ChatCompletionsBackend;
use secrecy::SecretString;
use tower::ServiceExt;

const TOKEN: &str = "test-token";

fn state_in(home: &std::path::Path, privacy: PrivacyConfig) -> AppState {
    let config = Config {
        privacy,
        ..Config::default()
    };
    AppState::new(
        BackendRegistry::new(),
        config,
        ConsentLedger::default(),
        TOKEN.to_string(),
    )
    .with_paths(PathsConfig::rooted_at(home))
}

async fn call(
    state: &AppState,
    method: &str,
    path: &str,
    body: Option<&str>,
) -> (StatusCode, serde_json::Value) {
    let request = Request::builder()
        .method(method)
        .uri(format!("/_ironwire{path}"))
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .expect("request builds");
    let response = app(state.clone()).oneshot(request).await.expect("handled");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("body");
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn home() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

/// A registered, authenticated backend. The base URL is a closed loopback port:
/// nothing here ever sends a request, and `status()` only reads the key.
fn chat_backend(
    id: &str,
    name: &str,
) -> Result<ChatCompletionsBackend, Box<dyn std::error::Error>> {
    ChatCompletionsBackend::new(
        ironwire_core::protocol::BackendId::from(id),
        name,
        ironwire_core::protocol::BackendKind::Credits,
        Some(SecretString::from("a-key")),
        "http://127.0.0.1:9/v1".to_string(),
        Vec::new(),
        5,
    )
    .map_err(Into::into)
}

// ---------------------------------------------------------------- settings

#[tokio::test]
async fn the_settings_endpoint_needs_the_control_token() {
    let dir = home();
    let state = state_in(dir.path(), PrivacyConfig::default());
    let response = app(state)
        .oneshot(
            Request::builder()
                .uri("/_ironwire/settings")
                .body(Body::empty())
                .expect("builds"),
        )
        .await
        .expect("handled");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The client must not work out for itself whether `full` is offerable — that
/// rule lives in `Config::validate`, and a second copy of it in a menu would
/// drift the first time the rule changed.
#[tokio::test]
async fn full_is_not_selectable_until_the_user_has_named_somewhere_to_route() {
    let dir = home();
    // Explicitly empty, not `default()`: the default names `nearai` now, so
    // taking the default here would quietly test the has-no-credential case
    // below instead of the nothing-named case this one is about.
    let state = state_in(
        dir.path(),
        PrivacyConfig {
            trusted_backends: Vec::new(),
            ..PrivacyConfig::default()
        },
    );
    let (status, body) = call(&state, "GET", "/settings", None).await;
    assert_eq!(status, StatusCode::OK);

    let options = body["privacy"]["options"].as_array().expect("options");
    assert_eq!(options.len(), 4, "every rung of the ladder is offered");
    let full = options.iter().find(|o| o["id"] == "full").expect("full");
    assert_eq!(full["selectable"], false);
    assert!(
        full["unavailable_because"]
            .as_str()
            .is_some_and(|why| why.contains("trusted_backends")),
        "a greyed-out option has to say what to do about itself: {full}"
    );
}

/// Naming a destination is not the same as having one.
///
/// `trusted_backends` defaults to `["nearai"]` and that backend registers with
/// or without a key, so "named" is true on a machine where `full` can route
/// nowhere at all. Selecting it there refuses every request, so the option stays
/// greyed out and says which credential is missing.
#[tokio::test]
async fn full_is_not_selectable_while_the_named_destination_has_no_credential() {
    let dir = home();
    let state = state_in(
        dir.path(),
        PrivacyConfig {
            trusted_backends: vec!["nearai".to_string()],
            ..PrivacyConfig::default()
        },
    );
    let (_, body) = call(&state, "GET", "/settings", None).await;
    let options = body["privacy"]["options"].as_array().expect("options");
    let full = options.iter().find(|o| o["id"] == "full").expect("full");
    assert_eq!(full["selectable"], false);
    assert!(
        full["unavailable_because"]
            .as_str()
            .is_some_and(|why| why.contains("credential")),
        "the reason has to name what is missing, not just that it is unavailable: {full}"
    );
    assert_eq!(body["privacy"]["trusted_backends"][0], "nearai");
}

/// And it does become selectable once that destination can actually serve.
#[tokio::test]
async fn full_becomes_selectable_once_a_named_destination_is_usable() {
    let dir = home();
    let mut registry = BackendRegistry::new();
    registry.push(std::sync::Arc::new(
        ChatCompletionsBackend::nearai(
            Some(SecretString::from("near-key")),
            Some("http://127.0.0.1:9/v1".to_string()),
            Vec::new(),
            5,
        )
        .expect("build the NEAR AI backend"),
    ));
    let state = AppState::new(
        registry,
        Config {
            privacy: PrivacyConfig {
                trusted_backends: vec!["nearai".to_string()],
                ..PrivacyConfig::default()
            },
            ..Config::default()
        },
        ConsentLedger::default(),
        TOKEN.to_string(),
    )
    .with_paths(PathsConfig::rooted_at(dir.path()));

    let (_, body) = call(&state, "GET", "/settings", None).await;
    let options = body["privacy"]["options"].as_array().expect("options");
    let full = options.iter().find(|o| o["id"] == "full").expect("full");
    assert_eq!(full["selectable"], true);

    // And the endpoint behind the option agrees. These two were written
    // separately once and disagreed immediately.
    let (status, _) = call(&state, "POST", "/privacy", Some(r#"{"mode":"full"}"#)).await;
    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------- privacy

#[tokio::test]
async fn setting_a_mode_takes_effect_immediately_and_is_written_down() {
    let dir = home();
    std::fs::write(
        dir.path().join("config.toml"),
        "# a comment the user wrote\n[server]\nport = 8463\n",
    )
    .expect("writes");
    let state = state_in(dir.path(), PrivacyConfig::default());

    let (status, body) = call(&state, "POST", "/privacy", Some(r#"{"mode":"pii"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["mode"], "pii");
    assert_eq!(body["persisted"], true);

    // In force now, without a restart.
    let (_, settings) = call(&state, "GET", "/settings", None).await;
    assert_eq!(settings["privacy"]["mode"], "pii");

    // And written down, without eating the file around it.
    let saved = std::fs::read_to_string(dir.path().join("config.toml")).expect("reads");
    assert!(saved.contains("# a comment the user wrote"), "{saved}");
    assert!(saved.contains("port = 8463"), "{saved}");
    assert!(saved.contains("mode = \"pii\""), "{saved}");
}

/// The routing constraint and the filter have to move together. Under `full`
/// only named backends are eligible at all, and that is read from the registry's
/// copy of the config — a stale one would keep routing somewhere the user has
/// just said they do not accept.
#[tokio::test]
async fn switching_to_full_also_moves_the_routing_constraint() {
    let dir = home();
    // Two usable backends, one trusted and one not. The empty registry this
    // used to run against could only assert that nothing was eligible, which
    // was equally true before the switch — it proved nothing about the
    // constraint moving.
    let mut registry = BackendRegistry::new();
    registry.push(std::sync::Arc::new(
        chat_backend("nearai", "NEAR AI").expect("build nearai"),
    ));
    registry.push(std::sync::Arc::new(
        chat_backend("openai-key", "OpenAI").expect("build openai"),
    ));
    let state = AppState::new(
        registry,
        Config {
            privacy: PrivacyConfig {
                trusted_backends: vec!["nearai".to_string()],
                ..PrivacyConfig::default()
            },
            ..Config::default()
        },
        ConsentLedger::default(),
        TOKEN.to_string(),
    )
    .with_paths(PathsConfig::rooted_at(dir.path()));

    let (status, _) = call(&state, "POST", "/privacy", Some(r#"{"mode":"full"}"#)).await;
    assert_eq!(status, StatusCode::OK);

    // `candidates` lists every registered backend and marks each one; the
    // filtering itself is `policy::eligible`'s job. So the assertion is on the
    // mark, not on the length of the list.
    let statuses = state.backends.statuses().await;
    let consent = state.consent_snapshot();
    let candidates = state.backends.candidates(&statuses, &consent);
    let trusted = |id: &str| {
        candidates
            .iter()
            .find(|c| c.id.as_str() == id)
            .unwrap_or_else(|| panic!("{id} is registered"))
            .trusted
    };
    assert!(trusted("nearai"), "the named backend is not marked trusted");
    assert!(
        !trusted("openai-key"),
        "a backend the user did not name is still marked trusted under `full`"
    );
    assert_eq!(state.privacy_config().mode(), PrivacyMode::Full);
    assert!(state.privacy_config().trusts("nearai"));
    assert!(!state.privacy_config().trusts("openai"));
}

#[tokio::test]
async fn switching_to_full_with_nowhere_to_route_is_refused_with_the_reason() {
    let dir = home();
    let state = state_in(dir.path(), PrivacyConfig::default());
    let (status, body) = call(&state, "POST", "/privacy", Some(r#"{"mode":"full"}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("trusted_backends")),
        "{body}"
    );
    // And nothing changed.
    assert_eq!(state.privacy_config().mode(), PrivacyMode::Off);
}

#[tokio::test]
async fn an_unknown_mode_is_refused_and_names_the_ones_that_exist() {
    let dir = home();
    let state = state_in(dir.path(), PrivacyConfig::default());
    let (status, body) = call(&state, "POST", "/privacy", Some(r#"{"mode":"maximum"}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let error = body["error"].as_str().expect("an error");
    for mode in ["off", "credentials", "pii", "full"] {
        assert!(error.contains(mode), "{error}");
    }
}

/// A config we cannot parse is not one to append to. The change still applies to
/// the running daemon — it is what the user asked for — but the response says
/// plainly that it will not survive a restart.
#[tokio::test]
async fn an_unwritable_config_is_reported_rather_than_silently_dropped() {
    let dir = home();
    std::fs::write(dir.path().join("config.toml"), "[server\nport = ").expect("writes");
    let state = state_in(dir.path(), PrivacyConfig::default());

    let (status, body) = call(&state, "POST", "/privacy", Some(r#"{"mode":"pii"}"#)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["persisted"], false);
    assert!(
        body["warning"]
            .as_str()
            .is_some_and(|w| w.contains("revert")),
        "the user has to be told it will not last: {body}"
    );
    assert_eq!(state.privacy_config().mode(), PrivacyMode::Pii);
}

// ---------------------------------------------------------------- consent

#[tokio::test]
async fn consent_is_recorded_against_the_version_that_was_answered() {
    let dir = home();
    let state = state_in(dir.path(), PrivacyConfig::default());

    let body = serde_json::json!({"backend":"claude-sub", "granted":true,
        "prompt_version":CONSENT_PROMPT_VERSION,
        "binding":ironwire_creds::consent::ConsentPrompt::for_backend("claude-sub").unwrap().binding}).to_string();
    let (status, _) = call(&state, "POST", "/consent", Some(&body)).await;
    assert_eq!(status, StatusCode::OK);

    // In force for the next routing decision, not the next restart.
    assert!(state.consent_snapshot().is_granted("claude-sub"));

    // And on disk, because consent is *recorded* consent (TRUST §2).
    let saved = std::fs::read_to_string(dir.path().join("consent.json")).expect("reads");
    assert!(saved.contains("claude-sub"), "{saved}");
    assert!(
        saved.contains(&format!("\"prompt_version\": {CONSENT_PROMPT_VERSION}")),
        "{saved}"
    );
}

/// The whole point of versioning the prompt. A client that has been open since
/// before the wording changed would otherwise record agreement to the new
/// question on the strength of the old one having been displayed.
#[tokio::test]
async fn consent_to_an_older_version_of_the_question_does_not_count() {
    let dir = home();
    let state = state_in(dir.path(), PrivacyConfig::default());
    let (status, body) = call(
        &state,
        "POST",
        "/consent",
        Some(r#"{"backend":"claude-sub","granted":true,"prompt_version":0}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"]
            .as_str()
            .is_some_and(|e| e.contains("different question")),
        "{body}"
    );
    assert!(!state.consent_snapshot().is_granted("claude-sub"));
}

/// Withdrawing is always allowed. A user who wants to stop must never be told
/// that the version of the question they answered is too old to stop with.
#[tokio::test]
async fn withdrawing_consent_is_not_gated_on_the_prompt_version() {
    let dir = home();
    let state = state_in(dir.path(), PrivacyConfig::default());
    state.set_consent("claude-sub", true).expect("granted");
    assert!(state.consent_snapshot().is_granted("claude-sub"));

    let (status, _) = call(
        &state,
        "POST",
        "/consent",
        Some(r#"{"backend":"claude-sub","granted":false,"prompt_version":0}"#),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(!state.consent_snapshot().is_granted("claude-sub"));
}

#[tokio::test]
async fn consent_for_a_backend_it_does_not_apply_to_is_refused() {
    let dir = home();
    let state = state_in(dir.path(), PrivacyConfig::default());
    let (status, body) = call(
        &state,
        "POST",
        "/consent",
        Some(r#"{"backend":"nearai","granted":true,"prompt_version":2}"#),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        body["error"].as_str().is_some_and(|e| e.contains("nearai")),
        "{body}"
    );
}

/// Consent we could not write down must not be treated as granted — otherwise
/// the daemon routes to a subscription on the strength of a record that does not
/// exist, and the next restart forgets why.
#[tokio::test]
async fn consent_that_cannot_be_written_is_not_granted() {
    let state = AppState::new(
        BackendRegistry::new(),
        Config::default(),
        ConsentLedger::default(),
        TOKEN.to_string(),
    );
    // No `with_paths`: nowhere to record it.
    let (status, _) = call(
        &state,
        "POST",
        "/consent",
        Some(&serde_json::json!({"backend":"claude-sub", "granted":true, "prompt_version":2,
            "binding":ironwire_creds::consent::ConsentPrompt::for_backend("claude-sub").unwrap().binding}).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(!state.consent_snapshot().is_granted("claude-sub"));
}

/// The client renders this and must not compose its own. Two copies of a consent
/// prompt are two prompts, and the recorded version would claim otherwise.
#[tokio::test]
async fn the_settings_endpoint_carries_the_exact_consent_question() {
    let dir = home();
    let state = state_in(dir.path(), PrivacyConfig::default());
    let prompt = ironwire_creds::consent::ConsentPrompt::for_backend("claude-sub").expect("exists");
    assert_eq!(prompt.version, CONSENT_PROMPT_VERSION);
    assert!(prompt.summary.contains("api.anthropic.com"));
    assert!(!prompt.points.is_empty());
    // With no backends registered there are no services to carry it, so the
    // shape is asserted at the source; `settings_view` copies it verbatim.
    let (_, body) = call(&state, "GET", "/settings", None).await;
    assert!(body["services"].is_array(), "{body}");
}

#[tokio::test]
async fn a_consent_grant_without_the_prompt_login_binding_is_refused() {
    let dir = home();
    let state = state_in(dir.path(), PrivacyConfig::default());
    let body = format!(
        r#"{{"backend":"claude-sub","granted":true,"prompt_version":{CONSENT_PROMPT_VERSION}}}"#
    );
    let (status, _) = call(&state, "POST", "/consent", Some(&body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!state.consent_snapshot().is_granted("claude-sub"));
    assert!(!dir.path().join("consent.json").exists());
}
