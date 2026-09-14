//! Bounded, unauthenticated reads of published mission packages.
//!
//! INTEGRATION: Root must add `pub mod mission_catalog;` to `lib.rs`.
//!
//! The configured ingest URL is an origin, not a server-provided endpoint. This
//! client intentionally accepts only an origin path (`/`) and constructs the
//! two fixed public paths below itself, so a catalog response cannot redirect a
//! later request to another authority.

use std::time::Duration;

use reqwest::{StatusCode, Url};
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use trace_commons_protocol::mission_catalog::{
    MISSION_CATALOG_PAGE_MAX_BYTES, MISSION_PUBLICATION_MAX_BYTES, MissionCatalogPage,
    MissionCatalogQuery, MissionPublication,
};
use uuid::Uuid;

use crate::config::allowlist_for;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CATALOG_PATH: &str = "/v1/missions";

/// Fixed, content-free outcomes for public mission discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MissionCatalogClientError {
    #[error("mission-catalog-endpoint-invalid")]
    EndpointInvalid,
    #[error("mission-catalog-host-not-allowed")]
    HostNotAllowed,
    #[error("mission-catalog-request-invalid")]
    Invalid,
    #[error("mission-catalog-not-found")]
    NotFound,
    #[error("mission-catalog-response-too-large")]
    ResponseTooLarge,
    #[error("mission-catalog-mission-mismatch")]
    MissionMismatch,
    #[error("mission-catalog-unavailable")]
    Unavailable,
}

/// An unauthenticated client for public mission-catalog endpoints.
///
/// This is deliberately not an `operator_client::Client`: that client always
/// attaches a bearer token, while publishing discovery metadata must not send
/// an enrollment, account, tenant, session, or API-key credential.
pub struct MissionCatalogClient {
    http: reqwest::Client,
    origin: Url,
    allowlist: HostAllowlist,
}

impl MissionCatalogClient {
    /// Creates a catalog client for a trusted, configured ingest origin.
    ///
    /// HTTPS is required except for literal loopback IPs, which support local
    /// development without weakening production network policy. Base-path
    /// prefixes are refused because the contributor ingest contract names an
    /// origin and all public paths are absolute protocol paths.
    pub fn new(
        ingest_url: &str,
        allowed_hosts: Option<&str>,
    ) -> Result<Self, MissionCatalogClientError> {
        let origin =
            Url::parse(ingest_url).map_err(|_| MissionCatalogClientError::EndpointInvalid)?;
        validate_origin(&origin)?;
        let allowlist = allowlist_for(allowed_hosts);
        allowlist
            .check(&origin)
            .map_err(|_| MissionCatalogClientError::HostNotAllowed)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| MissionCatalogClientError::Unavailable)?;
        Ok(Self {
            http,
            origin,
            allowlist,
        })
    }

    /// Lists one validated page of publicly published missions.
    pub async fn list(
        &self,
        query: &MissionCatalogQuery,
    ) -> Result<MissionCatalogPage, MissionCatalogClientError> {
        query
            .validate()
            .map_err(|_| MissionCatalogClientError::Invalid)?;
        let mut url = self.catalog_url()?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("limit", &query.limit.to_string());
            if let Some(before) = query.before {
                pairs.append_pair("before", &before.to_string());
            }
        }
        let bytes = self.get_bytes(url, MISSION_CATALOG_PAGE_MAX_BYTES).await?;
        MissionCatalogPage::parse(&bytes, query).map_err(|_| MissionCatalogClientError::Invalid)
    }

    /// Retrieves and validates the immutable package publication for `mission`.
    pub async fn get(
        &self,
        mission: Uuid,
    ) -> Result<MissionPublication, MissionCatalogClientError> {
        if mission.is_nil() {
            return Err(MissionCatalogClientError::Invalid);
        }
        let mut url = self.catalog_url()?;
        url.set_path(&format!("{CATALOG_PATH}/{mission}"));
        let bytes = self.get_bytes(url, MISSION_PUBLICATION_MAX_BYTES).await?;
        let publication =
            MissionPublication::parse(&bytes).map_err(|_| MissionCatalogClientError::Invalid)?;
        if publication
            .package()
            .map_err(|_| MissionCatalogClientError::Invalid)?
            .mission_id
            != mission
        {
            return Err(MissionCatalogClientError::MissionMismatch);
        }
        Ok(publication)
    }

    fn catalog_url(&self) -> Result<Url, MissionCatalogClientError> {
        let mut url = self.origin.clone();
        url.set_path(CATALOG_PATH);
        url.set_query(None);
        url.set_fragment(None);
        self.allowlist
            .check(&url)
            .map_err(|_| MissionCatalogClientError::HostNotAllowed)?;
        Ok(url)
    }

    async fn get_bytes(
        &self,
        url: Url,
        maximum: usize,
    ) -> Result<Vec<u8>, MissionCatalogClientError> {
        self.allowlist
            .check(&url)
            .map_err(|_| MissionCatalogClientError::HostNotAllowed)?;
        let mut response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| MissionCatalogClientError::Unavailable)?;
        match response.status() {
            StatusCode::NOT_FOUND => return Err(MissionCatalogClientError::NotFound),
            StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => {
                return Err(MissionCatalogClientError::Invalid);
            }
            status if !status.is_success() => return Err(MissionCatalogClientError::Unavailable),
            _ => {}
        }
        if response
            .content_length()
            .is_some_and(|length| length > maximum as u64)
        {
            return Err(MissionCatalogClientError::ResponseTooLarge);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| MissionCatalogClientError::Unavailable)?
        {
            if body.len().saturating_add(chunk.len()) > maximum {
                return Err(MissionCatalogClientError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn validate_origin(origin: &Url) -> Result<(), MissionCatalogClientError> {
    let is_loopback_http = match origin.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    };
    if origin.host_str().is_none()
        || (!origin.username().is_empty())
        || origin.password().is_some()
        || origin.query().is_some()
        || origin.fragment().is_some()
        || origin.path() != "/"
        || !matches!(origin.scheme(), "https" | "http")
        || (origin.scheme() == "http" && !is_loopback_http)
    {
        return Err(MissionCatalogClientError::EndpointInvalid);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use axum::{
        Json, Router,
        extract::{Path, Query},
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use chrono::{DateTime, Utc};
    use sha2::{Digest as _, Sha256};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use trace_commons_protocol::mission_catalog::{
        MISSION_CATALOG_SCHEMA_VERSION, MissionCatalogEntry,
    };
    use trace_commons_protocol::mission_evaluation::{
        MISSION_EVALUATION_MAX_CONCURRENCY, MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
        MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS, MISSION_EVALUATION_REQUIRED_MODEL_OWNER,
        MISSION_EVALUATION_SCHEMA_VERSION, MISSION_EVALUATION_TOTAL_REQUESTS, MISSION_EVALUATOR_ID,
        MissionEvaluationPackage, MissionEvaluationPolicy, SkillDraft, render_skill,
    };

    use crate::mission_catalog::*;

    fn digest(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    fn package(mission_id: Uuid) -> MissionEvaluationPackage {
        let skill = SkillDraft {
            name: "repair-generated-sources".into(),
            description: "Repair generated files at their authoritative source.".into(),
            procedure: "# Repair\n\nUpdate the source, then regenerate outputs.".into(),
        };
        MissionEvaluationPackage {
            schema_version: MISSION_EVALUATION_SCHEMA_VERSION,
            mission_id,
            program_id: Uuid::from_u128(2),
            offer_version_hash: format!("sha256:{}", "a".repeat(64)),
            task: "Repair the generated manifest from its source schema.".into(),
            skill_sha256: digest(render_skill(&skill).as_bytes()),
            skill,
            evaluator_id: MISSION_EVALUATOR_ID.into(),
            evaluation_contract_hash: "b".repeat(64),
            execution: MissionEvaluationPolicy {
                required_model_owner: MISSION_EVALUATION_REQUIRED_MODEL_OWNER.into(),
                total_requests: MISSION_EVALUATION_TOTAL_REQUESTS,
                output_token_limit: MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
                request_timeout_seconds: MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS,
                max_concurrency: MISSION_EVALUATION_MAX_CONCURRENCY,
            },
        }
    }

    fn publication(mission_id: Uuid) -> MissionPublication {
        let package_json = serde_json::to_string(&package(mission_id)).unwrap();
        MissionPublication {
            schema_version: MISSION_CATALOG_SCHEMA_VERSION,
            package_sha256: digest(package_json.as_bytes()),
            package_json,
            published_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }

    fn entry(mission_id: Uuid) -> MissionCatalogEntry {
        MissionCatalogEntry {
            mission_id,
            program_id: Uuid::from_u128(2),
            package_sha256: "c".repeat(64),
            offer_version_hash: format!("sha256:{}", "d".repeat(64)),
            task_preview: "Repair the generated manifest.".into(),
            published_at: DateTime::<Utc>::UNIX_EPOCH,
        }
    }

    async fn serve(app: Router) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), task)
    }

    async fn serve_chunked(body: String) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                .await
                .unwrap();
            socket
                .write_all(format!("{:X}\r\n", body.len()).as_bytes())
                .await
                .unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
            socket.write_all(b"\r\n0\r\n\r\n").await.unwrap();
        });
        (format!("http://{address}"), task)
    }

    async fn serve_declared_oversize_without_body(
        maximum: usize,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 1024];
            let _ = socket.read(&mut request).await.unwrap();
            socket
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", maximum + 1)
                        .as_bytes(),
                )
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        });
        (format!("http://{address}"), task)
    }

    #[tokio::test]
    async fn public_requests_are_exact_and_carry_no_credentials() {
        let mission = Uuid::from_u128(3);
        let list_mission = mission;
        let app = Router::new()
            .route(
                CATALOG_PATH,
                get(
                    move |headers: HeaderMap, Query(query): Query<HashMap<String, String>>| async move {
                        assert_eq!(query.get("limit").map(String::as_str), Some("1"));
                        assert_eq!(
                            query.get("before").map(String::as_str),
                            Some("00000000-0000-0000-0000-000000000004")
                        );
                        for header in ["authorization", "cookie", "x-api-key", "x-tenant-id"] {
                            assert!(!headers.contains_key(header), "{header} was sent");
                        }
                        Json(MissionCatalogPage {
                            schema_version: MISSION_CATALOG_SCHEMA_VERSION,
                            entries: vec![entry(list_mission)],
                            next_cursor: None,
                        })
                    },
                ),
            )
            .route(
                "/v1/missions/{mission}",
                get(move |headers: HeaderMap| async move {
                    for header in ["authorization", "cookie", "x-api-key", "x-tenant-id"] {
                        assert!(!headers.contains_key(header), "{header} was sent");
                    }
                    Json(publication(mission))
                }),
            );
        let (base, server) = serve(app).await;
        let client = MissionCatalogClient::new(&base, Some("127.0.0.1")).unwrap();
        let query = MissionCatalogQuery {
            limit: 1,
            before: Some(Uuid::from_u128(4)),
        };
        assert_eq!(client.list(&query).await.unwrap().entries.len(), 1);
        assert_eq!(client.get(mission).await.unwrap(), publication(mission));
        server.abort();
    }

    #[tokio::test]
    async fn malformed_and_chunked_oversized_responses_are_refused() {
        let app = Router::new().route("/v1/missions/{mission}", get(|| async { "{" }));
        let (base, server) = serve(app).await;
        let client = MissionCatalogClient::new(&base, Some("127.0.0.1")).unwrap();
        assert_eq!(
            client.get(Uuid::from_u128(3)).await,
            Err(MissionCatalogClientError::Invalid)
        );
        server.abort();

        let (base, server) = serve_chunked("x".repeat(MISSION_CATALOG_PAGE_MAX_BYTES + 1)).await;
        let client = MissionCatalogClient::new(&base, Some("127.0.0.1")).unwrap();
        let query = MissionCatalogQuery {
            limit: 1,
            before: None,
        };
        assert_eq!(
            client.list(&query).await,
            Err(MissionCatalogClientError::ResponseTooLarge)
        );
        server.abort();
    }

    #[tokio::test]
    async fn declared_oversize_is_refused_before_waiting_for_either_response_body() {
        let (base, server) =
            serve_declared_oversize_without_body(MISSION_CATALOG_PAGE_MAX_BYTES).await;
        let client = MissionCatalogClient::new(&base, Some("127.0.0.1")).unwrap();
        let listed = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.list(&MissionCatalogQuery {
                limit: 1,
                before: None,
            }),
        )
        .await
        .expect("declared catalog oversize is rejected from headers");
        assert_eq!(listed, Err(MissionCatalogClientError::ResponseTooLarge));
        server.abort();

        let (base, server) =
            serve_declared_oversize_without_body(MISSION_PUBLICATION_MAX_BYTES).await;
        let client = MissionCatalogClient::new(&base, Some("127.0.0.1")).unwrap();
        let fetched = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            client.get(Uuid::from_u128(3)),
        )
        .await
        .expect("declared publication oversize is rejected from headers");
        assert_eq!(fetched, Err(MissionCatalogClientError::ResponseTooLarge));
        server.abort();
    }

    #[tokio::test]
    async fn redirects_are_not_followed() {
        let followups = Arc::new(AtomicUsize::new(0));
        let target_counter = Arc::clone(&followups);
        let target = Router::new().fallback(move || {
            let target_counter = Arc::clone(&target_counter);
            async move {
                target_counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        });
        let (target_base, target_server) = serve(target).await;
        let app = Router::new().route(
            CATALOG_PATH,
            get(move || {
                let target_base = target_base.clone();
                async move { (StatusCode::FOUND, [("location", target_base)]) }
            }),
        );
        let (base, server) = serve(app).await;
        let client = MissionCatalogClient::new(&base, Some("127.0.0.1")).unwrap();
        assert_eq!(
            client
                .list(&MissionCatalogQuery {
                    limit: 1,
                    before: None,
                })
                .await,
            Err(MissionCatalogClientError::Unavailable)
        );
        assert_eq!(followups.load(Ordering::SeqCst), 0);
        server.abort();
        target_server.abort();
    }

    #[tokio::test]
    async fn malformed_bindings_and_safe_statuses_are_mapped() {
        let asked = Uuid::from_u128(3);
        let original = publication(asked);
        let corrupt = {
            let mut publication = original.clone();
            publication.package_sha256.replace_range(..1, "d");
            assert_ne!(publication.package_sha256, original.package_sha256);
            publication
        };
        let app = Router::new().route(
            "/v1/missions/{mission}",
            get(move |Path(mission): Path<Uuid>| {
                let corrupt = corrupt.clone();
                async move {
                    match mission.as_u128() {
                        3 => Json(serde_json::to_value(corrupt).unwrap()).into_response(),
                        4 => Json(serde_json::to_value(publication(Uuid::from_u128(9))).unwrap())
                            .into_response(),
                        5 => StatusCode::NOT_FOUND.into_response(),
                        6 => StatusCode::BAD_REQUEST.into_response(),
                        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                    }
                }
            }),
        );
        let (base, server) = serve(app).await;
        let client = MissionCatalogClient::new(&base, Some("127.0.0.1")).unwrap();
        assert_eq!(
            client.get(asked).await,
            Err(MissionCatalogClientError::Invalid)
        );
        assert_eq!(
            client.get(Uuid::from_u128(4)).await,
            Err(MissionCatalogClientError::MissionMismatch)
        );
        assert_eq!(
            client.get(Uuid::from_u128(5)).await,
            Err(MissionCatalogClientError::NotFound)
        );
        assert_eq!(
            client.get(Uuid::from_u128(6)).await,
            Err(MissionCatalogClientError::Invalid)
        );
        assert_eq!(
            client.get(Uuid::from_u128(7)).await,
            Err(MissionCatalogClientError::Unavailable)
        );
        server.abort();
    }

    #[tokio::test]
    async fn invalid_requests_are_refused_before_network_io() {
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&requests);
        let app = Router::new().fallback(move || {
            let counter = Arc::clone(&counter);
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                StatusCode::OK
            }
        });
        let (base, server) = serve(app).await;
        let client = MissionCatalogClient::new(&base, Some("127.0.0.1")).unwrap();
        assert_eq!(
            client
                .list(&MissionCatalogQuery {
                    limit: 0,
                    before: None,
                })
                .await,
            Err(MissionCatalogClientError::Invalid)
        );
        assert_eq!(
            client.get(Uuid::nil()).await,
            Err(MissionCatalogClientError::Invalid)
        );
        assert_eq!(requests.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[test]
    fn invalid_inputs_and_untrusted_origins_are_refused() {
        for origin in [
            "http://example.com",
            "http://localhost:8080",
            "https://user:secret@example.com",
            "https://example.com/base",
            "https://example.com/?query=1",
            "https://example.com/#fragment",
        ] {
            assert!(matches!(
                MissionCatalogClient::new(origin, None),
                Err(MissionCatalogClientError::EndpointInvalid)
            ));
        }
        assert!(matches!(
            MissionCatalogClient::new("https://example.com", Some("other.example")),
            Err(MissionCatalogClientError::HostNotAllowed)
        ));
        assert_eq!(
            MissionCatalogQuery {
                limit: 0,
                before: None
            }
            .validate(),
            Err(trace_commons_protocol::mission_catalog::MissionCatalogError::InvalidLimit)
        );
    }
}
