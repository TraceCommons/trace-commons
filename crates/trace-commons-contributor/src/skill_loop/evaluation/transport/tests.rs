//! INTEGRATION: exercises the bounded NEAR AI transport against synthetic
//! catalog and completion responses without making live or paid requests.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::extract::Query;
use axum::http::{HeaderMap, Response, StatusCode};
use axum::response::{IntoResponse, Response as AxumResponse};
use axum::{Json, Router, routing::get, routing::post};

use super::*;
use crate::skill_loop::evaluation::fixtures::FIXTURES;

const MODEL: &str = "deepseek-ai/DeepSeek-V4-Flash";

fn current_model(
    id: &str,
    owner: &str,
    is_ready: Option<bool>,
    features: &[&str],
    output_modalities: &[&str],
) -> serde_json::Value {
    serde_json::json!({
        "modelId": id,
        "inputCostPerToken": {"amount": 0, "scale": 9, "currency": "USD"},
        "outputCostPerToken": {"amount": 0, "scale": 9, "currency": "USD"},
        "costPerImage": {"amount": 0, "scale": 9, "currency": "USD"},
        "metadata": {
            "ownedBy": owner,
            "isReady": is_ready,
            "supportedFeatures": features,
            "architecture": {
                "inputModalities": ["text"],
                "outputModalities": output_modalities
            }
        }
    })
}

fn current_catalog(
    models: Vec<serde_json::Value>,
    offset: usize,
    total: usize,
) -> serde_json::Value {
    serde_json::json!({
        "models": models,
        "limit": CATALOG_PAGE_LIMIT,
        "offset": offset,
        "total": total
    })
}

fn eligible_current_catalog() -> serde_json::Value {
    current_catalog(
        vec![current_model(
            MODEL,
            "nearai",
            Some(true),
            &["json_mode"],
            &["text"],
        )],
        0,
        1,
    )
}

async fn spawn(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve test router");
    });
    (format!("http://{address}/v1"), server)
}

async fn authenticated_catalog(headers: HeaderMap) -> AxumResponse {
    if headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        != Some("Bearer catalog-key")
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(eligible_current_catalog()).into_response()
}

#[tokio::test]
async fn official_model_list_shape_requires_accepted_bearer_auth() {
    let (base_url, server) =
        spawn(Router::new().route("/v1/model/list", get(authenticated_catalog))).await;
    assert!(matches!(
        NearAiEvaluationClient::discover_at(base_url.clone(), "wrong-key".to_string(), false).await,
        Err(SkillEvaluationError::ProviderRejected)
    ));
    let client = NearAiEvaluationClient::discover_at(base_url, "catalog-key".to_string(), false)
        .await
        .expect("accepted bearer");
    assert_eq!(client.model(), MODEL);
    server.abort();
}

#[tokio::test]
async fn payment_required_is_distinct_for_catalog_and_completion_requests() {
    let catalog_app = Router::new().route(
        "/v1/model/list",
        get(|| async { StatusCode::PAYMENT_REQUIRED }),
    );
    let (catalog_base_url, catalog_server) = spawn(catalog_app).await;
    assert!(matches!(
        NearAiEvaluationClient::discover_at(catalog_base_url, "key".to_string(), false).await,
        Err(SkillEvaluationError::FundingRequired)
    ));
    catalog_server.abort();

    let completion_app = Router::new().route(
        "/v1/chat/completions",
        post(|| async { StatusCode::PAYMENT_REQUIRED }),
    );
    let (completion_base_url, completion_server) = spawn(completion_app).await;
    let client =
        NearAiEvaluationClient::at(completion_base_url, "key".to_string(), MODEL.to_string())
            .expect("client");
    assert!(matches!(
        client
            .complete(FIXTURES[0], EvaluationArm::Baseline, "", "")
            .await,
        Err(SkillEvaluationError::FundingRequired)
    ));
    completion_server.abort();
}

#[tokio::test]
async fn malformed_catalog_fails_closed() {
    let app = Router::new().route(
        "/v1/model/list",
        get(|| async { Response::new(Body::from("not-json")) }),
    );
    let (base_url, server) = spawn(app).await;
    assert!(matches!(
        NearAiEvaluationClient::discover_at(base_url, "key".to_string(), false).await,
        Err(SkillEvaluationError::CatalogUnavailable)
    ));
    server.abort();
}

#[tokio::test]
async fn oversized_catalog_returns_response_too_large() {
    let app = Router::new().route(
        "/v1/model/list",
        get(|| async { Response::new(Body::from(vec![b'a'; MAX_CATALOG_BYTES + 1])) }),
    );
    let (base_url, server) = spawn(app).await;
    assert!(matches!(
        NearAiEvaluationClient::discover_at(base_url, "key".to_string(), false).await,
        Err(SkillEvaluationError::ResponseTooLarge)
    ));
    server.abort();
}

#[tokio::test]
async fn official_catalog_missing_owned_by_fails_closed() {
    let app = Router::new().route(
        "/v1/model/list",
        get(|| async {
            Json(serde_json::json!({
                "models": [{
                    "modelId": MODEL,
                    "metadata": {
                        "isReady": true,
                        "supportedFeatures": ["json_mode"],
                        "architecture": {"outputModalities": ["text"]}
                    }
                }],
                "limit": CATALOG_PAGE_LIMIT,
                "offset": 0,
                "total": 1
            }))
        }),
    );
    let (base_url, server) = spawn(app).await;
    assert!(matches!(
        NearAiEvaluationClient::discover_at(base_url, "key".to_string(), false).await,
        Err(SkillEvaluationError::CatalogUnavailable)
    ));
    server.abort();
}

#[tokio::test]
async fn official_catalog_allows_missing_optional_capabilities_but_rejects_model() {
    let app = Router::new().route(
        "/v1/model/list",
        get(|| async {
            Json(serde_json::json!({
                "models": [{
                    "modelId": MODEL,
                    "metadata": {"ownedBy": "nearai"}
                }],
                "limit": CATALOG_PAGE_LIMIT,
                "offset": 0,
                "total": 1
            }))
        }),
    );
    let (base_url, server) = spawn(app).await;
    assert!(matches!(
        NearAiEvaluationClient::discover_at(base_url, "key".to_string(), false).await,
        Err(SkillEvaluationError::NoPrivateModel)
    ));
    server.abort();
}

#[tokio::test]
async fn legacy_data_shape_uses_architecture_when_output_modalities_is_null() {
    let app = Router::new().route(
        "/v1/models",
        get(|| async {
            Json(serde_json::json!({
                "object": "list",
                "data": [{
                    "id": MODEL,
                    "owned_by": "nearai",
                    "is_ready": true,
                    "supported_features": ["structured_outputs"],
                    "output_modalities": null,
                    "architecture": {
                        "inputModalities": ["text"],
                        "outputModalities": ["text"]
                    }
                }]
            }))
        }),
    );
    let (base_url, server) = spawn(app).await;
    let client = NearAiEvaluationClient::discover_at(base_url, "key".to_string(), false)
        .await
        .expect("legacy catalog");
    assert_eq!(client.model(), MODEL);
    server.abort();
}

#[tokio::test]
async fn legacy_catalog_requires_supported_features() {
    let app = Router::new().route(
        "/v1/models",
        get(|| async {
            Json(serde_json::json!({
                "data": [{
                    "id": MODEL,
                    "owned_by": "nearai",
                    "is_ready": true,
                    "output_modalities": ["text"]
                }]
            }))
        }),
    );
    let (base_url, server) = spawn(app).await;
    assert!(matches!(
        NearAiEvaluationClient::discover_at(base_url, "key".to_string(), false).await,
        Err(SkillEvaluationError::CatalogUnavailable)
    ));
    server.abort();
}

#[tokio::test]
async fn discovery_reads_later_pages_before_selecting_a_model() {
    let request_count = Arc::new(AtomicUsize::new(0));
    let handler_count = Arc::clone(&request_count);
    let app = Router::new().route(
        "/v1/model/list",
        get(move |Query(query): Query<HashMap<String, String>>| {
            let handler_count = Arc::clone(&handler_count);
            async move {
                handler_count.fetch_add(1, Ordering::SeqCst);
                assert_eq!(query.get("limit").map(String::as_str), Some("100"));
                let offset = query
                    .get("offset")
                    .and_then(|value| value.parse::<usize>().ok())
                    .expect("offset");
                let models = if offset == 0 {
                    (0..CATALOG_PAGE_LIMIT)
                        .map(|index| {
                            current_model(
                                &format!("external/model-{index}"),
                                "external",
                                Some(true),
                                &["json_mode"],
                                &["text"],
                            )
                        })
                        .collect()
                } else {
                    assert_eq!(offset, CATALOG_PAGE_LIMIT);
                    vec![current_model(
                        MODEL,
                        "nearai",
                        Some(true),
                        &["structured_outputs"],
                        &["text"],
                    )]
                };
                Json(current_catalog(models, offset, CATALOG_PAGE_LIMIT + 1))
            }
        }),
    );
    let (base_url, server) = spawn(app).await;
    let client = NearAiEvaluationClient::discover_at(base_url, "key".to_string(), false)
        .await
        .expect("second page model");
    assert_eq!(client.model(), MODEL);
    assert_eq!(request_count.load(Ordering::SeqCst), 2);
    server.abort();
}

#[tokio::test]
async fn catalog_total_over_model_bound_is_rejected_before_another_page() {
    let request_count = Arc::new(AtomicUsize::new(0));
    let handler_count = Arc::clone(&request_count);
    let app = Router::new().route(
        "/v1/model/list",
        get(move || {
            let handler_count = Arc::clone(&handler_count);
            async move {
                handler_count.fetch_add(1, Ordering::SeqCst);
                Json(current_catalog(Vec::new(), 0, MAX_CATALOG_MODELS + 1))
            }
        }),
    );
    let (base_url, server) = spawn(app).await;
    assert!(matches!(
        NearAiEvaluationClient::discover_at(base_url, "key".to_string(), false).await,
        Err(SkillEvaluationError::ResponseTooLarge)
    ));
    assert_eq!(request_count.load(Ordering::SeqCst), 1);
    server.abort();
}

#[test]
fn model_selection_requires_ready_owned_text_structured_metadata() {
    let candidate =
        |owner: &str, is_ready: Option<bool>, features: &[&str], modalities: &[&str]| ModelRecord {
            id: MODEL.to_string(),
            owned_by: owner.to_string(),
            is_ready,
            supported_features: features.iter().map(|value| (*value).to_string()).collect(),
            output_modalities: Some(
                modalities
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            ),
        };
    for rejected in [
        candidate("openai", Some(true), &["json_mode"], &["text"]),
        candidate("nearai", None, &["json_mode"], &["text"]),
        candidate("nearai", Some(false), &["json_mode"], &["text"]),
        candidate("nearai", Some(true), &[], &["text"]),
        candidate("nearai", Some(true), &["tools"], &["text"]),
        candidate("nearai", Some(true), &["json_mode"], &[]),
        candidate("nearai", Some(true), &["json_mode"], &["image"]),
    ] {
        assert_eq!(choose_private_model(&[rejected]), None);
    }
    let missing_output = ModelRecord {
        id: MODEL.to_string(),
        owned_by: "nearai".to_string(),
        is_ready: Some(true),
        supported_features: vec!["json_mode".to_string()],
        output_modalities: None,
    };
    assert_eq!(choose_private_model(&[missing_output]), None);
    let accepted = candidate("nearai", Some(true), &["structured_outputs"], &["text"]);
    assert_eq!(choose_private_model(&[accepted]), Some(MODEL));
}

#[tokio::test]
async fn completion_parser_handles_the_live_response_shape() {
    let app = Router::new().route(
        "/v1/chat/completions",
        post(|| async {
            Json(serde_json::json!({
                "id": "chat-1",
                "model": MODEL,
                "choices": [{
                    "message": {"role":"assistant", "content":"{\"diagnosis\":\"generated\",\"edit_paths\":[],\"commands\":[],\"verification\":[]}"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 8, "total_tokens": 18}
            }))
        }),
    );
    let (base_url, server) = spawn(app).await;
    let client =
        NearAiEvaluationClient::at(base_url, "synthetic-key".to_string(), MODEL.to_string())
            .expect("client");
    let answer = client
        .complete(FIXTURES[0], EvaluationArm::Baseline, "", "")
        .await
        .expect("completion");
    assert_eq!(answer.served_model, MODEL);
    assert_eq!(answer.usage.total_tokens, Some(18));
    assert_eq!(answer.finish_reason, "stop");
    server.abort();
}

#[tokio::test]
async fn completion_failures_fail_closed_with_stable_errors() {
    for (status, expected) in [
        (
            StatusCode::UNAUTHORIZED,
            SkillEvaluationError::ProviderRejected,
        ),
        (
            StatusCode::FORBIDDEN,
            SkillEvaluationError::ProviderRejected,
        ),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillEvaluationError::RequestUnavailable,
        ),
    ] {
        let app = Router::new().route("/v1/chat/completions", post(move || async move { status }));
        let (base_url, server) = spawn(app).await;
        let client = NearAiEvaluationClient::at(base_url, "key".to_string(), MODEL.to_string())
            .expect("client");
        let result = client
            .complete(FIXTURES[0], EvaluationArm::Baseline, "", "")
            .await;
        assert!(matches!(result, Err(error) if error == expected));
        server.abort();
    }

    for body in ["not-json", r#"{"model":"model","choices":[]}"#] {
        let body = body.to_string();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(move || {
                let body = body.clone();
                async move { Response::new(Body::from(body)) }
            }),
        );
        let (base_url, server) = spawn(app).await;
        let client = NearAiEvaluationClient::at(base_url, "key".to_string(), MODEL.to_string())
            .expect("client");
        let result = client
            .complete(FIXTURES[0], EvaluationArm::Baseline, "", "")
            .await;
        assert!(matches!(
            result,
            Err(SkillEvaluationError::RequestUnavailable)
        ));
        server.abort();
    }
}

#[tokio::test]
async fn oversized_retained_output_returns_response_too_large() {
    let content = "a".repeat(MAX_RETAINED_RAW_OUTPUT_BYTES + 1);
    let app = Router::new().route(
        "/v1/chat/completions",
        post(move || {
            let content = content.clone();
            async move {
                Json(serde_json::json!({
                    "model": MODEL,
                    "choices": [{
                        "message": {"content": content},
                        "finish_reason": "stop"
                    }]
                }))
            }
        }),
    );
    let (base_url, server) = spawn(app).await;
    let client =
        NearAiEvaluationClient::at(base_url, "synthetic-key".to_string(), MODEL.to_string())
            .expect("client");
    assert!(matches!(
        client
            .complete(FIXTURES[0], EvaluationArm::Baseline, "", "")
            .await,
        Err(SkillEvaluationError::ResponseTooLarge)
    ));
    server.abort();
}

/// Runs only when the developer deliberately exports the existing
/// NEARAI_API_KEY credential. The value never enters assertion output.
#[tokio::test]
#[ignore = "requires a NEAR AI inference key and makes one live fixture request"]
async fn live_near_ai_catalog_and_one_fixture_request_match_the_contract() {
    let api_key = std::env::var("NEARAI_API_KEY")
        .expect("NEARAI_API_KEY must be set when the ignored live contract test is requested");
    assert!(
        !api_key.trim().is_empty(),
        "NEARAI_API_KEY must be nonempty when the ignored live contract test is requested"
    );
    let client = NearAiEvaluationClient::discover(api_key)
        .await
        .expect("live model catalog contract");
    let expected_model = client.model().to_string();
    let response = client
        .complete(FIXTURES[0], EvaluationArm::Baseline, "", "")
        .await
        .expect("live one-fixture completion contract");
    assert_eq!(response.served_model, expected_model);
    assert!(response.raw_output.len() <= MAX_RETAINED_RAW_OUTPUT_BYTES);
    assert!(!response.finish_reason.is_empty());
}
