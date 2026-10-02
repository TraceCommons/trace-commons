//! Read-only discovery of published skill-evaluation packages.
//!
//! Requests use the configured ingest origin and the existing anonymous,
//! bounded client. No account/device credentials, local activity or matching
//! inputs are read. Discovery never retrieves selected details or changes
//! consent, project policy, approvals, upload state or reward reservations.

use trace_commons_protocol::mission_catalog::MissionCatalogQuery;

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::mission_catalog::MissionCatalogClient;

/// A failed fetch is unavailable, never a successful empty catalogue.
const UNAVAILABLE: &str = "mission-catalogue-unavailable";
const INVALID: &str = "mission-catalog-request-invalid";

pub(super) async fn handle_catalogue(shared: &DaemonShared, req: &Request) -> Response {
    let Ok(query) = serde_json::from_value::<MissionCatalogQuery>(req.params.clone()) else {
        return Response::err(req.id, ERR_BAD_PARAMS, INVALID);
    };
    if query.validate().is_err() {
        return Response::err(req.id, ERR_BAD_PARAMS, INVALID);
    }
    let Ok(Some(config)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    };
    let Ok(client) = MissionCatalogClient::new(&config.ingest_url, config.allowed_hosts.as_deref())
    else {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    };
    match client.list(&query).await {
        Ok(catalogue) => Response::ok(
            req.id,
            serde_json::json!({
                "kind": "skill_evaluation",
                "catalogue": catalogue,
                "disclosure": crate::consent_copy::MISSION_CATALOGUE_DISCLOSURE,
            }),
        ),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE),
    }
}
