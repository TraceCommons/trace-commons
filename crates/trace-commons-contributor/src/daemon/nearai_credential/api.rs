//! The management-plane half: turn a session into an inference key, then stop
//! holding the session.
//!
//! Response shapes here were taken from the deployed service's own OpenAPI
//! document (`https://cloud-api.near.ai/api-docs/openapi.json`), which is
//! generated from the running build, rather than from a source read of a
//! commit nobody can prove is deployed. Two of them are easy to get wrong from
//! prose: the list endpoints return a paginated *object* with the array under
//! `organizations` / `workspaces`, not a bare array, and the minted key is
//! `key`, an `Option<String>` that is populated exactly once at creation and
//! never again.
//!
//! The host is a compiled-in constant and there is no configuration that moves
//! it. That is deliberate: this credential is minted against one service, an
//! attacker who could redirect the mint would receive a contributor's session
//! token, and the enrollment ceremony's env-driven allowlist is not available
//! here -- it enforces only when `TRACE_COMMONS_ALLOWED_HOSTS` happens to be
//! set, which on a contributor's laptop it is not.
//!
//! Note also which host. `api.near.ai` is RETIRED and answers HTTP 410; the
//! deployed cloud-api is `cloud-api.near.ai`, confirmed live.

use super::loopback::{CALLBACK_PATH, SessionTokens};
use anyhow::{Result, anyhow, bail};
use serde::Deserialize;
use std::time::Duration;

/// The one service this credential is minted against.
pub const CLOUD_API_ORIGIN: &str = "https://cloud-api.near.ai";
/// The sign-in providers the service exposes as a browser redirect.
pub const PROVIDERS: [&str; 2] = ["github", "google"];
/// A whole ceremony's worth of HTTP is three small calls; none of them should
/// ever take this long.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// A management response is a few kilobytes. Anything beyond this is not a
/// response we understand and must not be buffered on a contributor's machine.
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Deserialize)]
struct Organization {
    id: String,
    is_active: bool,
}

#[derive(Deserialize)]
struct Organizations {
    organizations: Vec<Organization>,
}

#[derive(Deserialize)]
struct Workspace {
    id: String,
    is_active: bool,
}

#[derive(Deserialize)]
struct Workspaces {
    workspaces: Vec<Workspace>,
}

#[derive(Deserialize)]
struct ApiKey {
    id: String,
    key: Option<String>,
    key_prefix: String,
    workspace_id: String,
}

/// A minted inference credential, and enough context to say where it came from
/// without saying what it is.
pub struct MintedKey {
    /// The plaintext `sk-` key. Returned once by the service and never again.
    pub key: String,
    /// The service's id for the key, which is what a later revoke needs.
    pub key_id: String,
    pub key_prefix: String,
    pub organization_id: String,
    pub workspace_id: String,
}

impl std::fmt::Debug for MintedKey {
    /// The prefix is safe to render and is the only part of the key that ever
    /// should be: it is what the service itself shows in its own key list.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MintedKey")
            .field("key", &"<redacted>")
            .field("key_id", &self.key_id)
            .field("key_prefix", &self.key_prefix)
            .field("organization_id", &self.organization_id)
            .field("workspace_id", &self.workspace_id)
            .finish()
    }
}

/// A client pinned to one origin.
pub struct CloudApi {
    base: reqwest::Url,
    http: reqwest::Client,
}

impl CloudApi {
    /// The live service. The only constructor a shipping build can reach.
    pub fn live() -> Result<Self> {
        Self::at(CLOUD_API_ORIGIN)
    }

    /// Construct against an explicit origin, refusing anything that is not a
    /// bare, credential-free HTTPS origin.
    ///
    /// Tests reach this through [`CloudApi::for_test`], which is the only way
    /// a plaintext loopback origin is ever accepted -- so a test double cannot
    /// become a way to ship an unpinned client.
    fn at(origin: &str) -> Result<Self> {
        let base = reqwest::Url::parse(origin)?;
        if base.scheme() != "https"
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || base.host_str().is_none()
        {
            bail!("near_ai_credential_endpoint_refused")
        }
        Ok(Self {
            base,
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                // A management redirect would move a session bearer to
                // whatever host the response named. There is no legitimate one
                // on any of these three calls.
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(origin: &str) -> Result<Self> {
        Ok(Self {
            base: reqwest::Url::parse(origin)?,
            http: reqwest::Client::builder()
                .timeout(REQUEST_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }

    fn url(&self, path: &str) -> Result<reqwest::Url> {
        self.base
            .join(path)
            .map_err(|_| anyhow!("near_ai_credential_endpoint_refused"))
    }

    /// Where the browser is sent to begin sign in.
    ///
    /// The callback is assembled and checked here rather than trusted, because
    /// the service's refusal is a 400 with an empty body: a callback that is
    /// wrong by one character produces a failure with nothing in it to show a
    /// contributor or write to a log. Refusing locally is the only way this
    /// can ever be diagnosed.
    pub fn authorize_url(&self, provider: &str, port: u16) -> Result<String> {
        if !PROVIDERS.contains(&provider) {
            bail!("near_ai_credential_provider_unknown")
        }
        let callback = format!("http://127.0.0.1:{port}{CALLBACK_PATH}");
        let parsed = reqwest::Url::parse(&callback)?;
        // The service's own rules, restated so a change to CALLBACK_PATH that
        // violates one fails here instead of at an empty 400. `[::1]` and
        // `https` loopback are both refused upstream, so the host and scheme
        // are checked as literals.
        if parsed.scheme() != "http"
            || parsed.host_str() != Some("127.0.0.1")
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || parsed.path().contains("..")
            || callback.contains('@')
            || callback.len() > 2048
        {
            bail!("near_ai_credential_callback_invalid")
        }
        let mut url = self.url(&format!("/v1/auth/{provider}"))?;
        url.query_pairs_mut()
            .append_pair("frontend_callback", &callback);
        Ok(url.into())
    }

    /// One session-authenticated call, decoded.
    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        session: &SessionTokens,
        body: Option<&serde_json::Value>,
    ) -> Result<T> {
        let mut request = self
            .http
            .request(method, self.url(path)?)
            // Every management route is session-only; an sk- key is refused
            // here exactly as a session is refused on inference.
            .bearer_auth(&session.access_token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            bail!("near_ai_credential_unavailable")
        }
        if !status.is_success() {
            // The body is never relayed. It is the service's prose about a
            // request that carried a session bearer, and this crate's rule is
            // that operational surfaces are label-only.
            bail!("near_ai_credential_refused")
        }
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("near_ai_credential_unexpected"))
    }

    /// Session in, inference key out.
    ///
    /// A fresh user normally arrives here already provisioned: the service's
    /// OAuth user creation makes an organization and a default workspace. But
    /// it does both **best effort** -- each failure only logs and does not fail
    /// the signup -- so a real account can exist with no organization at all.
    /// That case gets its own named refusal rather than a panic or an empty
    /// path segment in the mint URL, and it is deliberately not repaired by
    /// creating an organization here: the platform names its own, and a client
    /// inventing a second one would leave a contributor with two.
    pub async fn mint_inference_key(
        &self,
        session: &SessionTokens,
        key_name: &str,
    ) -> Result<MintedKey> {
        if key_name.trim().is_empty() || key_name.len() > 128 {
            bail!("near_ai_credential_invalid")
        }
        let organizations: Organizations = self
            .call(reqwest::Method::GET, "/v1/organizations", session, None)
            .await?;
        let organization = organizations
            .organizations
            .into_iter()
            .find(|o| o.is_active && !o.id.is_empty())
            .ok_or_else(|| anyhow!("near_ai_credential_no_organization"))?;
        let workspaces: Workspaces = self
            .call(
                reqwest::Method::GET,
                &format!("/v1/organizations/{}/workspaces", organization.id),
                session,
                None,
            )
            .await?;
        let workspace = workspaces
            .workspaces
            .into_iter()
            .find(|w| w.is_active && !w.id.is_empty())
            .ok_or_else(|| anyhow!("near_ai_credential_no_workspace"))?;
        // `expires_at` is omitted, which the service accepts and reads as no
        // expiry. That is the whole reason the session is discarded a moment
        // later: a non-expiring inference key needs no refresh, where a held
        // session would need one every seven days forever.
        let minted: ApiKey = self
            .call(
                reqwest::Method::POST,
                &format!("/v1/workspaces/{}/api-keys", workspace.id),
                session,
                Some(&serde_json::json!({ "name": key_name })),
            )
            .await?;
        let key = minted
            .key
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow!("near_ai_credential_not_returned"))?;
        // The service tells us which workspace it actually minted in. Believe
        // it over the one we asked for, so a stored key is never attributed to
        // a workspace it does not belong to.
        Ok(MintedKey {
            key,
            key_id: minted.id,
            key_prefix: minted.key_prefix,
            organization_id: organization.id,
            workspace_id: minted.workspace_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::Request, routing::get};
    use std::sync::{Arc, Mutex};

    fn session() -> SessionTokens {
        SessionTokens {
            access_token: "session-jwt".into(),
            refresh_token: "rt_example".into(),
        }
    }

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }

    /// Every bearer the stub was shown, so a test can assert which credential
    /// class reached which route.
    type Bearers = Arc<Mutex<Vec<String>>>;

    /// The happy path the deployed service serves, in the shapes its own
    /// OpenAPI document declares.
    fn stub(organizations: serde_json::Value, workspaces: serde_json::Value) -> (Router, Bearers) {
        let seen: Bearers = Default::default();
        let record = seen.clone();
        let minted = Arc::new(Mutex::new(Vec::<serde_json::Value>::new()));
        let bodies = minted.clone();
        let router = Router::new()
            .route(
                "/v1/organizations",
                get(move || {
                    let organizations = organizations.clone();
                    async move { Json(organizations) }
                }),
            )
            .route(
                "/v1/organizations/{org}/workspaces",
                get(move || {
                    let workspaces = workspaces.clone();
                    async move { Json(workspaces) }
                }),
            )
            .route(
                "/v1/workspaces/{workspace}/api-keys",
                axum::routing::post(
                    move |axum::extract::Path(workspace): axum::extract::Path<String>,
                          Json(body): Json<serde_json::Value>| {
                        bodies.lock().unwrap().push(body);
                        async move {
                            Json(serde_json::json!({
                                "id": "key-1",
                                "key": "sk-minted-secret",
                                "key_prefix": "sk-min",
                                "workspace_id": workspace,
                                "created_by_user_id": "u",
                                "created_at": "2026-09-08T00:00:00Z",
                                "is_active": true,
                            }))
                        }
                    },
                ),
            )
            .layer(axum::middleware::from_fn(
                move |request: Request, next: axum::middleware::Next| {
                    let record = record.clone();
                    async move {
                        if let Some(value) = request.headers().get("authorization")
                            && let Ok(value) = value.to_str()
                        {
                            record.lock().unwrap().push(value.to_string());
                        }
                        next.run(request).await
                    }
                },
            ));
        (router, seen)
    }

    fn one_org() -> serde_json::Value {
        serde_json::json!({
            "organizations": [{
                "id": "org-1", "name": "alice-org-ab12", "owner_id": "u",
                "settings": {}, "role": "owner", "is_active": true,
                "created_at": "2026-09-08T00:00:00Z", "updated_at": "2026-09-08T00:00:00Z",
            }],
            "total": 1, "limit": 50, "offset": 0,
        })
    }

    fn one_workspace() -> serde_json::Value {
        serde_json::json!({
            "workspaces": [{
                "id": "ws-1", "name": "default", "organization_id": "org-1",
                "created_by_user_id": "u", "is_active": true,
                "created_at": "2026-09-08T00:00:00Z", "updated_at": "2026-09-08T00:00:00Z",
            }],
            "total": 1, "limit": 50, "offset": 0,
        })
    }

    #[tokio::test]
    async fn a_session_walks_the_three_calls_and_yields_a_plaintext_key() {
        let (router, bearers) = stub(one_org(), one_workspace());
        let api = CloudApi::for_test(&spawn(router).await).unwrap();
        let minted = api
            .mint_inference_key(&session(), "trace-commons-device")
            .await
            .unwrap();
        assert_eq!(minted.key, "sk-minted-secret");
        assert_eq!(minted.key_id, "key-1");
        assert_eq!(minted.organization_id, "org-1");
        assert_eq!(minted.workspace_id, "ws-1");
        // All three management calls are session-authenticated. If any of them
        // went out unauthenticated the service would answer 401 and the
        // failure would look like the service being down.
        let bearers = bearers.lock().unwrap().clone();
        assert_eq!(bearers.len(), 3, "{bearers:?}");
        assert!(
            bearers.iter().all(|b| b == "Bearer session-jwt"),
            "{bearers:?}"
        );
    }

    #[tokio::test]
    async fn an_account_with_no_organization_is_a_named_refusal() {
        // The service creates an organization and a workspace for a fresh user
        // best-effort: each failure only logs. So this account really can
        // exist, and it must not become a mint against `/v1/workspaces//`.
        let empty = serde_json::json!({"organizations": [], "total": 0, "limit": 50, "offset": 0});
        let (router, _) = stub(empty, one_workspace());
        let api = CloudApi::for_test(&spawn(router).await).unwrap();
        let error = api
            .mint_inference_key(&session(), "trace-commons-device")
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(error, "near_ai_credential_no_organization");

        let empty = serde_json::json!({"workspaces": [], "total": 0, "limit": 50, "offset": 0});
        let (router, _) = stub(one_org(), empty);
        let api = CloudApi::for_test(&spawn(router).await).unwrap();
        let error = api
            .mint_inference_key(&session(), "trace-commons-device")
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(error, "near_ai_credential_no_workspace");
    }

    #[tokio::test]
    async fn an_inactive_organization_or_workspace_is_not_chosen() {
        let mut organizations = one_org();
        organizations["organizations"][0]["is_active"] = false.into();
        let (router, _) = stub(organizations, one_workspace());
        let api = CloudApi::for_test(&spawn(router).await).unwrap();
        assert_eq!(
            api.mint_inference_key(&session(), "d")
                .await
                .unwrap_err()
                .to_string(),
            "near_ai_credential_no_organization"
        );
    }

    #[tokio::test]
    async fn a_response_without_the_key_is_refused_rather_than_stored_empty() {
        let router = Router::new()
            .route("/v1/organizations", get(|| async { Json(one_org()) }))
            .route(
                "/v1/organizations/{org}/workspaces",
                get(|| async { Json(one_workspace()) }),
            )
            .route(
                "/v1/workspaces/{workspace}/api-keys",
                axum::routing::post(|| async {
                    // The service returns `key` only at creation; a list read
                    // or an unexpected build answers null. Storing that as a
                    // credential would leave a contributor with a configured
                    // key that authenticates nothing.
                    Json(serde_json::json!({
                        "id": "key-1", "key": null, "key_prefix": "sk-min",
                        "workspace_id": "ws-1", "created_by_user_id": "u",
                        "created_at": "2026-09-08T00:00:00Z", "is_active": true,
                    }))
                }),
            );
        let api = CloudApi::for_test(&spawn(router).await).unwrap();
        assert_eq!(
            api.mint_inference_key(&session(), "d")
                .await
                .unwrap_err()
                .to_string(),
            "near_ai_credential_not_returned"
        );
    }

    #[tokio::test]
    async fn a_service_error_never_relays_its_body() {
        let router = Router::new().route(
            "/v1/organizations",
            get(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"error": {"message": "Bearer session-jwt rejected"}})),
                )
            }),
        );
        let api = CloudApi::for_test(&spawn(router).await).unwrap();
        let error = api
            .mint_inference_key(&session(), "d")
            .await
            .unwrap_err()
            .to_string();
        assert_eq!(error, "near_ai_credential_refused");
        assert!(!error.contains("session-jwt"), "{error}");
    }

    #[test]
    fn the_origin_is_pinned_to_the_live_service_and_cannot_be_moved() {
        let api = CloudApi::live().unwrap();
        assert_eq!(api.base.scheme(), "https");
        // `api.near.ai` is retired and answers 410. The host is a constant so
        // that no environment can point a contributor's session at another.
        assert_eq!(api.base.host_str(), Some("cloud-api.near.ai"));
        for refused in [
            "http://cloud-api.near.ai",
            "https://user:pw@cloud-api.near.ai",
            "https://cloud-api.near.ai/?a=b",
            "https://cloud-api.near.ai/#f",
        ] {
            assert!(CloudApi::at(refused).is_err(), "{refused}");
        }
    }

    #[test]
    fn the_callback_offered_to_the_service_satisfies_every_rule_it_enforces() {
        let api = CloudApi::live().unwrap();
        let url = api.authorize_url("github", 49721).unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        assert_eq!(parsed.host_str(), Some("cloud-api.near.ai"));
        assert_eq!(parsed.path(), "/v1/auth/github");
        let callback = parsed
            .query_pairs()
            .find(|(k, _)| k == "frontend_callback")
            .unwrap()
            .1
            .into_owned();
        // http, the literal 127.0.0.1, a bare path: https loopback, `[::1]`,
        // a query and a fragment are each a 400 with an empty body upstream.
        assert_eq!(
            callback,
            format!("http://127.0.0.1:49721{CALLBACK_PATH}"),
            "{callback}"
        );
        assert!(api.authorize_url("google", 1).is_ok());
        assert!(api.authorize_url("near", 1).is_err());
        assert!(api.authorize_url("../admin", 1).is_err());
    }
}
