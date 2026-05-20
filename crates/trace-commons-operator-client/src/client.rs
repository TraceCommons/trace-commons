//! HTTP transport for trace-commons-server operator binaries.
//!
//! [`Client`] wraps `reqwest::Client` with:
//!   - Bearer-token resolution from an env var (never logged).
//!   - Host allowlist enforcement (see [`crate::host_allowlist`]).
//!   - Typed error mapping for the server's `{"error": "<Label>"}` envelope.
//!
//! Each operator binary constructs one `Client` per audience-scoped bearer
//! token. The worker binary's `--bearer-token-env` flag varies per
//! subcommand, so worker code builds the client lazily per subcommand.

use std::time::Duration;

use reqwest::Method;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{Error, Result, parse_error_label};
use crate::host_allowlist::HostAllowlist;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct Client {
    inner: reqwest::Client,
    endpoint: url::Url,
    bearer_token: String,
    host_allowlist: HostAllowlist,
}

pub struct ClientBuilder {
    endpoint: String,
    bearer_token_env: String,
    host_allowlist: HostAllowlist,
    timeout: Duration,
}

impl Client {
    pub fn builder(
        endpoint: impl Into<String>,
        bearer_token_env: impl Into<String>,
    ) -> ClientBuilder {
        ClientBuilder {
            endpoint: endpoint.into(),
            bearer_token_env: bearer_token_env.into(),
            host_allowlist: HostAllowlist::permissive(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Endpoint as configured at construction. Query strings and fragments
    /// are stripped because this value gets logged in error diagnostics.
    pub fn endpoint(&self) -> String {
        let mut endpoint = self.endpoint.clone();
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        endpoint.to_string()
    }

    /// Issue a typed JSON request and deserialize the response body.
    ///
    /// The path is joined onto the configured endpoint. Query parameters
    /// are appended verbatim. The bearer token is attached as `Authorization:
    /// Bearer <token>`. Non-success status codes produce [`Error::ServerLabel`]
    /// if the body carries a typed label, otherwise [`Error::HttpFailure`].
    pub async fn call_json<Req, Resp>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Req>,
    ) -> Result<Resp>
    where
        Req: Serialize + ?Sized,
        Resp: DeserializeOwned,
    {
        let response_body = self.call_raw(method, path, query, body).await?;
        let url = self.compose_url(path, query)?.to_string();
        if response_body.trim().is_empty() {
            // The caller asked for a typed Resp but got an empty body. This
            // is a server contract violation worth surfacing.
            let source = serde_json::from_str::<serde_json::Value>("")
                .expect_err("empty input is not valid JSON");
            return Err(Error::MalformedResponse {
                url,
                body: response_body,
                source,
            });
        }
        serde_json::from_str(&response_body).map_err(|source| Error::MalformedResponse {
            url,
            body: response_body,
            source,
        })
    }

    /// Issue an untyped JSON request and return the raw response body.
    /// Used by binaries that need the unparsed JSON for `--json` output or
    /// for endpoints whose response shape varies.
    pub async fn call_raw<Req>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&Req>,
    ) -> Result<String>
    where
        Req: Serialize + ?Sized,
    {
        let url = self.compose_url(path, query)?;
        self.host_allowlist.check(&url)?;

        let mut request = self.inner.request(method, url.clone());
        request = request.bearer_auth(&self.bearer_token);
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.map_err(|source| Error::Transport {
            url: url.to_string(),
            source,
        })?;
        let status = response.status();
        let response_body = response.text().await.map_err(|source| Error::Transport {
            url: url.to_string(),
            source,
        })?;

        if status.is_success() {
            return Ok(response_body);
        }

        match parse_error_label(&response_body) {
            Some(label) => Err(Error::ServerLabel {
                url: url.to_string(),
                status,
                label,
                body: response_body,
            }),
            None => Err(Error::HttpFailure {
                url: url.to_string(),
                status,
                body: response_body,
            }),
        }
    }

    fn compose_url(&self, path: &str, query: &[(&str, &str)]) -> Result<url::Url> {
        let mut url = self.endpoint.clone();
        // Replace path with the operator-requested one. The endpoint's
        // configured path is the API prefix (e.g. `/`); we don't try to
        // concatenate because that interacts badly with trailing slashes.
        url.set_path(path);
        url.set_query(None);
        if !query.is_empty() {
            let mut pairs = url.query_pairs_mut();
            for (k, v) in query {
                pairs.append_pair(k, v);
            }
            drop(pairs);
        }
        Ok(url)
    }
}

impl ClientBuilder {
    pub fn host_allowlist(mut self, allowlist: HostAllowlist) -> Self {
        self.host_allowlist = allowlist;
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn build(self) -> Result<Client> {
        let endpoint =
            url::Url::parse(&self.endpoint).map_err(|source| Error::InvalidEndpoint {
                endpoint: self.endpoint.clone(),
                source,
            })?;
        // Pre-flight the allowlist before we even hand back a Client, so
        // operators see the rejection at startup rather than at first request.
        self.host_allowlist.check(&endpoint)?;

        let bearer_token = std::env::var(&self.bearer_token_env)
            .ok()
            .filter(|t| !t.trim().is_empty())
            .ok_or(Error::BearerMissing {
                env_var: self.bearer_token_env.clone(),
            })?;

        let inner = reqwest::Client::builder()
            .timeout(self.timeout)
            .build()
            .map_err(|source| Error::Transport {
                url: endpoint.to_string(),
                source,
            })?;
        Ok(Client {
            inner,
            endpoint,
            bearer_token,
            host_allowlist: self.host_allowlist,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::sync::atomic::{AtomicU64, Ordering};
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Tests that touch the process env share global state. Each test gets a
    /// unique env-var name to avoid races under parallel execution.
    fn unique_bearer_env() -> String {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        format!("TC_OPERATOR_CLIENT_TEST_BEARER_{n}")
    }

    struct EnvGuard {
        name: String,
    }

    impl EnvGuard {
        fn set(name: String, value: &str) -> Self {
            // SAFETY: edition 2024 marks env mutation unsafe. We use a unique
            // name per test so no other thread reads or writes this variable.
            unsafe {
                std::env::set_var(&name, value);
            }
            EnvGuard { name }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: see EnvGuard::set.
            unsafe {
                std::env::remove_var(&self.name);
            }
        }
    }

    #[tokio::test]
    async fn builder_resolves_bearer_from_env() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret-token");
        let client = Client::builder("https://api.example", env)
            .build()
            .expect("client builds");
        assert!(client.endpoint().starts_with("https://api.example"));
    }

    #[tokio::test]
    async fn builder_refuses_when_bearer_env_missing() {
        let env = unique_bearer_env();
        // Not set.
        let err = Client::builder("https://api.example", env)
            .build()
            .unwrap_err();
        assert_eq!(err.kind(), "bearer-missing");
    }

    #[tokio::test]
    async fn builder_refuses_when_bearer_env_is_empty() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "");
        let err = Client::builder("https://api.example", env)
            .build()
            .unwrap_err();
        assert_eq!(err.kind(), "bearer-missing");
    }

    #[tokio::test]
    async fn builder_refuses_when_endpoint_blocked_by_allowlist() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let err = Client::builder("https://evil.example", env)
            .host_allowlist(HostAllowlist::from_csv("api.example"))
            .build()
            .unwrap_err();
        assert_eq!(err.kind(), "host-not-allowed");
    }

    #[derive(Debug, Deserialize, PartialEq, Eq)]
    struct ListResponse {
        items: Vec<String>,
    }

    #[derive(Debug, Serialize)]
    struct DecisionBody {
        decision: String,
    }

    #[tokio::test]
    async fn call_json_round_trips_typed_response() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/traces/quarantine"))
            .and(query_param("state", "leased"))
            .and(header("authorization", "Bearer secret"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "items": ["sub-1", "sub-2"],
            })))
            .expect(1)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env)
            .build()
            .expect("client builds");
        let resp: ListResponse = client
            .call_json::<(), ListResponse>(
                Method::GET,
                "/v1/traces/quarantine",
                &[("state", "leased")],
                None,
            )
            .await
            .expect("typed response");
        assert_eq!(
            resp,
            ListResponse {
                items: vec!["sub-1".into(), "sub-2".into()],
            }
        );
    }

    #[tokio::test]
    async fn call_json_surfaces_server_label_on_403() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/traces/sub-1/review"))
            .respond_with(
                ResponseTemplate::new(403)
                    .set_body_json(serde_json::json!({"error": "ReviewerNotAuthorized"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env)
            .build()
            .expect("client builds");
        let err = client
            .call_json::<DecisionBody, serde_json::Value>(
                Method::POST,
                "/v1/traces/sub-1/review",
                &[],
                Some(&DecisionBody {
                    decision: "approve".into(),
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "server-label");
        assert_eq!(err.server_label(), Some("ReviewerNotAuthorized"));
    }

    #[tokio::test]
    async fn call_json_falls_back_to_http_failure_when_body_has_no_label() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/traces/foo"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal server error"))
            .expect(1)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env)
            .build()
            .expect("client builds");
        let err = client
            .call_json::<(), serde_json::Value>(Method::GET, "/v1/traces/foo", &[], None)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "http-failure");
    }

    #[tokio::test]
    async fn call_json_surfaces_malformed_response_when_body_isnt_valid_for_type() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/traces/foo"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json at all"))
            .expect(1)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env)
            .build()
            .expect("client builds");
        let err = client
            .call_json::<(), ListResponse>(Method::GET, "/v1/traces/foo", &[], None)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "malformed-response");
    }

    #[tokio::test]
    async fn call_raw_returns_unparsed_body_on_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/traces/raw"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"a":1}"#))
            .expect(1)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env)
            .build()
            .expect("client builds");
        let body = client
            .call_raw::<()>(Method::GET, "/v1/traces/raw", &[], None)
            .await
            .expect("raw body");
        assert_eq!(body, r#"{"a":1}"#);
    }
}
