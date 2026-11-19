//! Vertex is an input envelope around Messages, not a second dispatch pipeline.

use axum::Router;
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::IntoResponse;
use axum::response::Response;
use axum::routing::post;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{Map, Value};

use super::error::FacadeError;
use super::guard::{self, RowClaim};
use crate::state::AppState;

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/{*path}", post(predict))
        .layer(axum::extract::DefaultBodyLimit::max(
            super::MAX_REQUEST_BODY_BYTES,
        ))
}

/// Authenticate the opaque raw path segment before reading the body, parsing
/// an envelope or answering a count request. Percent-encoding is not an alias.
pub(crate) async fn authorize(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    if let Some(gateway) = &state.vertex_gateway {
        let project = request
            .uri()
            .path()
            .strip_prefix("/projects/")
            .and_then(|path| path.split_once('/').map(|(project, _)| project));
        // Accepted keep-alive connections can race graceful shutdown's
        // per-connection notification. Revoke future bound requests at the
        // shared stop flag while already-authorized responses keep draining.
        if state.shutdown.begun() || !gateway.accepts(project) {
            if let Some(claim) = request.extensions().get::<RowClaim>() {
                claim.failed(guard::INVALID_VERTEX_CAPABILITY);
            }
            return FacadeError::new(
                StatusCode::FORBIDDEN,
                "permission_error",
                "Invalid Vertex capability.",
            )
            .into_response();
        }
    }
    next.run(request).await
}

/// Even an upload rejected before the handler must not persist cloud routing
/// identifiers. Never derive a recorded path from client-controlled segments.
pub(crate) fn canonical_path(path: &str) -> &'static str {
    if path.ends_with("/count-tokens:rawPredict") {
        "/v1/messages/count_tokens"
    } else {
        "/v1/messages"
    }
}

async fn predict(
    State(state): State<AppState>,
    uri: Uri,
    mut headers: HeaderMap,
    claim: Option<Extension<RowClaim>>,
    super::FacadeBody(body): super::FacadeBody<false>,
) -> Result<Response, FacadeError> {
    let claim = RowClaim::from_extension(claim);
    let invalid = || {
        claim.failed(guard::INVALID_VERTEX_REQUEST);
        FacadeError::invalid_request(
            "Invalid Vertex envelope. Use the documented /vertex/projects/PROJECT/locations/REGION/publishers/anthropic/models/MODEL:rawPredict or :streamRawPredict route with a matching JSON object.",
        )
    };
    if uri.query().is_some() || headers.contains_key(crate::pipeline::ROUTE_OVERRIDE_HEADER) {
        return Err(invalid());
    }
    // Read the raw URI rather than a percent-decoded Path extractor: encoded
    // separators and malformed encodings must not create another valid route.
    let segments: Vec<&str> = uri.path().split('/').collect();
    let [
        "",
        "projects",
        project,
        "locations",
        region,
        "publishers",
        "anthropic",
        "models",
        operation,
    ] = segments.as_slice()
    else {
        return Err(invalid());
    };
    let Some((model, action)) = operation.split_once(':') else {
        return Err(invalid());
    };
    if !identifier(project, false) || !identifier(region, false) || !identifier(model, true) {
        return Err(invalid());
    }
    let streaming = match action {
        "rawPredict" => false,
        "streamRawPredict" if model != "count-tokens" => true,
        _ => return Err(invalid()),
    };
    let Object(mut object) = serde_json::from_slice(&body).map_err(|_| invalid())?;
    if model == "count-tokens" {
        claim.failed(guard::UNSUPPORTED_TOKEN_COUNT);
        return Err(FacadeError::invalid_request(
            "Vertex token counting is unsupported. IronWire cannot provide an authoritative count for the selected backend; no request was sent upstream.",
        ));
    }
    if object
        .get("model")
        .is_some_and(|value| value.as_str() != Some(model))
        || object
            .get("stream")
            .is_some_and(|value| value.as_bool() != Some(streaming))
        || object
            .get("anthropic_version")
            .is_some_and(|value| value.as_str() != Some("vertex-2023-10-16"))
    {
        return Err(invalid());
    }
    object.remove("anthropic_version");
    object.insert("model".into(), Value::String(model.to_owned()));
    object.insert("stream".into(), Value::Bool(streaming));
    let normalized = serde_json::to_vec(&object).map_err(|_| invalid())?;
    // Google-specific headers can name a project or carry a Google API key;
    // neither belongs beside the selected backend's credential. Ordinary
    // auth stripping still happens in the shared Messages pipeline.
    let google: Vec<_> = headers
        .keys()
        .filter(|name| name.as_str().starts_with("x-goog-"))
        .cloned()
        .collect();
    for name in google {
        headers.remove(name);
    }
    super::anthropic::forward(
        claim,
        state,
        headers,
        normalized.into(),
        "/v1/messages",
        None,
    )
    .await
}

fn identifier(value: &str, model: bool) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && value.bytes().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.') || (model && c == b'@')
        })
}

/// A JSON object without ambiguous duplicate routing keys. Errors are rendered
/// as a fixed envelope above, never as parser messages quoting client content.
struct Object(Map<String, Value>);

impl<'de> Deserialize<'de> for Object {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor;
        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = Object;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an object with unique keys")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Object, A::Error> {
                let mut object = Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if object.contains_key(&key) {
                        return Err(serde::de::Error::custom("duplicate field"));
                    }
                    object.insert(key, map.next_value()?);
                }
                Ok(Object(object))
            }
        }
        deserializer.deserialize_map(ObjectVisitor)
    }
}
