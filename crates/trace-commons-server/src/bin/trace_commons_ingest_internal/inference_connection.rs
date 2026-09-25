// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Explicit account-session selection of operator-configured connections.

use super::*;
use trace_commons_protocol::inference_connection::{
    ConnectionWitnessConfig, SelectInferenceConnection,
};
use trace_commons_server::db::postgres_inference_connection::{
    InferenceDisconnectOutcome, InferenceSelectionOutcome,
};
use trace_commons_server::inference_connection::OperatorInferenceConnection;

const CATALOG_ENV: &str = "TRACE_COMMONS_INFERENCE_CONNECTION_CATALOG_JSON";

async fn no_store_middleware(request: Request, next: Next) -> axum::response::Response {
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    response
}

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/v1/account/inference-connection/offers",
            get(offers_handler),
        )
        .route(
            "/v1/account/inference-connection",
            get(current_handler).post(select_handler),
        )
        .route(
            "/v1/account/inference-connection/{connection_id}",
            delete(disconnect_handler),
        )
        .route_layer(axum::middleware::from_fn(no_store_middleware))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCatalogEntry {
    offer_id: String,
    provider_id: String,
    disclosure_version: String,
    witness: ConnectionWitnessConfig,
    inference_receipt_endpoint: Option<String>,
}

pub(super) fn catalog_from_env() -> anyhow::Result<Vec<OperatorInferenceConnection>> {
    let raw = match std::env::var(CATALOG_ENV) {
        Ok(raw) => raw,
        Err(std::env::VarError::NotPresent) => return Ok(Vec::new()),
        Err(std::env::VarError::NotUnicode(_)) => {
            anyhow::bail!("inference_connection_catalog_invalid")
        }
    };
    parse_catalog(&raw)
}

fn parse_catalog(raw: &str) -> anyhow::Result<Vec<OperatorInferenceConnection>> {
    if raw.len() > 65_536 {
        anyhow::bail!("inference_connection_catalog_invalid");
    }
    let entries: Vec<RawCatalogEntry> = serde_json::from_str(raw)
        .map_err(|_| anyhow::anyhow!("inference_connection_catalog_invalid"))?;
    if entries.len() > 16 {
        anyhow::bail!("inference_connection_catalog_invalid");
    }
    let mut ids = BTreeSet::new();
    let mut catalog = Vec::with_capacity(entries.len());
    for entry in entries {
        if !ids.insert(entry.offer_id.clone()) {
            anyhow::bail!("inference_connection_catalog_invalid");
        }
        catalog.push(
            OperatorInferenceConnection::new(
                entry.offer_id,
                entry.provider_id,
                &entry.disclosure_version,
                entry.witness,
                entry.inference_receipt_endpoint,
            )
            .map_err(|_| anyhow::anyhow!("inference_connection_catalog_invalid"))?,
        );
    }
    Ok(catalog)
}

fn no_store() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store"),
    );
    headers
}

fn require_mutating_account_session(ctx: &AccountCtx, headers: &HeaderMap) -> ApiResult<()> {
    if matches!(ctx.auth_method, AccountAuthMethod::DeviceBearer) {
        return Err(api_error(StatusCode::FORBIDDEN, "account session required"));
    }
    if matches!(ctx.auth_method, AccountAuthMethod::SessionCookie)
        && !confirm_is_same_origin(headers)
    {
        return Err(api_error(
            StatusCode::FORBIDDEN,
            "cross-origin account mutation",
        ));
    }
    Ok(())
}

pub(super) async fn offers_handler(
    State(state): State<Arc<AppState>>,
    Extension(_ctx): Extension<AccountCtx>,
) -> ApiResult<(HeaderMap, Json<serde_json::Value>)> {
    let offers: Vec<_> = state
        .inference_connection_catalog
        .iter()
        .map(OperatorInferenceConnection::offer)
        .collect();
    Ok((
        no_store(),
        Json(serde_json::json!({
            "contract_version": "inference-connection-v1",
            "inference_connection_selection_required": true,
            "description": "Selecting a service supplies witness configuration to this device after explicit confirmation; it does not enable folder contribution or attest inference without a verified receipt.",
            "offers": offers,
        })),
    ))
}

pub(super) async fn select_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    Json(body): Json<SelectInferenceConnection>,
) -> ApiResult<(
    HeaderMap,
    Json<trace_commons_protocol::inference_connection::SelectedInferenceConnection>,
)> {
    require_mutating_account_session(&ctx, &headers)?;
    if body.validate().is_err() {
        return Err(api_error(
            StatusCode::BAD_REQUEST,
            "invalid inference selection",
        ));
    }
    let catalog = state
        .inference_connection_catalog
        .iter()
        .find(|entry| entry.offer().offer_id == body.offer_id)
        .ok_or_else(|| api_error(StatusCode::CONFLICT, "connection_reselection_required"))?;
    let outcome = account_db(state.as_ref())?
        .select_inference_connection(&ctx.tenant_id, ctx.account_id.as_uuid(), &body, catalog)
        .await
        .map_err(internal_error)?;
    let selected = match outcome {
        InferenceSelectionOutcome::Selected(selected) => selected,
        InferenceSelectionOutcome::AccountIneligible => {
            return Err(api_error(StatusCode::FORBIDDEN, "account is not eligible"));
        }
        InferenceSelectionOutcome::InvalidSelection => {
            return Err(api_error(
                StatusCode::BAD_REQUEST,
                "invalid inference selection",
            ));
        }
        InferenceSelectionOutcome::VersionConflict => {
            return Err(api_error(
                StatusCode::CONFLICT,
                "connection_version_conflict",
            ));
        }
        InferenceSelectionOutcome::IdempotencyConflict => {
            return Err(api_error(StatusCode::CONFLICT, "idempotency key reused"));
        }
        InferenceSelectionOutcome::ReselectionRequired => {
            return Err(api_error(
                StatusCode::CONFLICT,
                "connection_reselection_required",
            ));
        }
    };
    Ok((no_store(), Json(selected)))
}

pub(super) async fn current_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
) -> ApiResult<(HeaderMap, Json<serde_json::Value>)> {
    let selection = account_db(state.as_ref())?
        .current_inference_connection(
            &ctx.tenant_id,
            ctx.account_id.as_uuid(),
            state.inference_connection_catalog.as_slice(),
        )
        .await
        .map_err(internal_error)?;
    Ok((
        no_store(),
        Json(serde_json::json!({
            "contract_version": "inference-connection-v1",
            "selection": selection,
            "install_on_this_device": false,
        })),
    ))
}

pub(super) async fn disconnect_handler(
    State(state): State<Arc<AppState>>,
    Extension(ctx): Extension<AccountCtx>,
    headers: HeaderMap,
    AxumPath(connection_id): AxumPath<Uuid>,
) -> ApiResult<(HeaderMap, Json<serde_json::Value>)> {
    require_mutating_account_session(&ctx, &headers)?;
    let outcome = account_db(state.as_ref())?
        .disconnect_inference_connection(&ctx.tenant_id, ctx.account_id.as_uuid(), connection_id)
        .await
        .map_err(internal_error)?;
    match outcome {
        InferenceDisconnectOutcome::Revoked { state_version } => Ok((
            no_store(),
            Json(serde_json::json!({
                "connection_id": connection_id,
                "state_version": state_version,
                "status": "revoked",
            })),
        )),
        InferenceDisconnectOutcome::AccountIneligible => {
            Err(api_error(StatusCode::FORBIDDEN, "account is not eligible"))
        }
        InferenceDisconnectOutcome::NotFound => {
            Err(api_error(StatusCode::NOT_FOUND, "connection not found"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_catalog_refuses_unreviewed_or_duplicate_descriptors() {
        let witness = serde_json::json!({
            "url": "https://private-witness.example/v1",
            "signing_address": format!("0x{}", "ab".repeat(20)),
            "expected_measurements": [format!("mrtd={}", "ab".repeat(48))],
        });
        let descriptor = serde_json::json!({
            "offer_id": "pilot",
            "provider_id": "near-ai",
            "disclosure_version": trace_commons_protocol::inference_connection::DISCLOSURE_VERSION,
            "witness": witness,
            "inference_receipt_endpoint": null,
        });
        let valid = serde_json::json!([descriptor.clone()]).to_string();
        assert_eq!(parse_catalog(&valid).unwrap().len(), 1);
        assert!(
            parse_catalog(&serde_json::json!([descriptor.clone(), descriptor.clone()]).to_string())
                .is_err()
        );
        let mut arbitrary = descriptor.clone();
        arbitrary["url"] = serde_json::json!("https://attacker.example");
        assert!(parse_catalog(&serde_json::json!([arbitrary]).to_string()).is_err());
        let mut insecure = descriptor;
        insecure["witness"]["url"] = serde_json::json!("http://private-witness.example/v1");
        assert!(parse_catalog(&serde_json::json!([insecure]).to_string()).is_err());
    }
}
