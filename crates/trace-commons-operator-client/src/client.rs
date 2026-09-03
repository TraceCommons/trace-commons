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
    explicit_bearer: Option<String>,
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
            explicit_bearer: None,
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
        self.send_and_read(request, url).await
    }

    /// Issue a request whose body is sent **byte for byte** and return the
    /// raw response body.
    ///
    /// [`Self::call_json`] and [`Self::call_raw`] both end in
    /// `request.json(body)`, which serialises the caller's value afresh. That
    /// is right for every existing caller and wrong for exactly one: a
    /// redaction-witness certificate binds a SHA-256 over the envelope bytes
    /// the witness emitted, so anything that deserialises and re-serialises
    /// them between the witness and `POST /v1/traces` breaks the digest --
    /// and breaks it invisibly, because the re-encoded bytes still parse as
    /// the same envelope and the failure only appears at the server's
    /// verification.
    ///
    /// So this method takes `&[u8]` rather than a `Serialize`. There is no
    /// generic parameter it could be handed instead: the whole point is that
    /// no serializer runs.
    ///
    /// `Content-Type: application/json` is set here rather than left to the
    /// caller -- the bytes are an envelope on every call site this exists
    /// for, and a caller that forgot would get a body the server refuses for
    /// a reason unrelated to anything it did wrong.
    ///
    /// The host allowlist and the bearer token are applied exactly as
    /// [`Self::call_raw`] applies them. `headers` are additional request
    /// headers, for material that must travel beside the body rather than
    /// inside it.
    pub async fn call_bytes(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: &[u8],
        headers: &[(&str, &str)],
    ) -> Result<String> {
        let url = self.compose_url(path, query)?;
        self.host_allowlist.check(&url)?;

        let mut request = self
            .inner
            .request(method, url.clone())
            .bearer_auth(&self.bearer_token)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            // `.body`, never `.json`: these bytes are covered by a signature
            // taken over them exactly as they are.
            .body(body.to_vec());
        for (name, value) in headers {
            // Validated before the request is built rather than left to
            // reqwest, which would fold a malformed header into a transport
            // error carrying the value in its message.
            let header_value = reqwest::header::HeaderValue::from_str(value).map_err(|_| {
                Error::HeaderMalformed {
                    name: (*name).to_string(),
                }
            })?;
            request = request.header(*name, header_value);
        }

        self.send_and_read(request, url).await
    }

    /// Send a prepared request and turn its status and body into this crate's
    /// error shape.
    ///
    /// Shared by [`Self::call_raw`] and [`Self::call_bytes`] so the two cannot
    /// drift into two spellings of the same error mapping -- which is the kind
    /// of difference an operator only meets during an incident.
    async fn send_and_read(
        &self,
        request: reqwest::RequestBuilder,
        url: url::Url,
    ) -> Result<String> {
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

    /// Provide the bearer token directly instead of naming an env var.
    /// Used by clients that mint short-lived tokens in memory (e.g. the
    /// contributor CLI's upload claims). Blank tokens are rejected at build.
    pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
        self.explicit_bearer = Some(token.into());
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

        let bearer_token = match self.explicit_bearer {
            Some(token) => {
                let trimmed = token.trim();
                if trimmed.is_empty() {
                    return Err(Error::BearerMissing {
                        env_var: "<explicit>".to_string(),
                    });
                }
                trimmed.to_string()
            }
            None => std::env::var(&self.bearer_token_env)
                .ok()
                .filter(|t| !t.trim().is_empty())
                .ok_or(Error::BearerMissing {
                    env_var: self.bearer_token_env.clone(),
                })?,
        };

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

    #[test]
    fn explicit_bearer_token_bypasses_env() {
        // Env var deliberately not set; explicit token must win.
        let client = Client::builder("https://ingest.example", "DEFINITELY_UNSET_ENV_VAR_XYZ")
            .bearer_token("claim-token-abc")
            .build()
            .expect("explicit token should not require env var");
        let _ = client.endpoint();
    }

    #[test]
    fn blank_explicit_bearer_token_is_rejected() {
        let err = Client::builder("https://ingest.example", "DEFINITELY_UNSET_ENV_VAR_XYZ")
            .bearer_token("   ")
            .build()
            .expect_err("blank explicit token must fail");
        assert_eq!(err.kind(), "bearer-missing");
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

    // -----------------------------------------------------------------
    // call_bytes: the body reaches the wire byte for byte.
    // -----------------------------------------------------------------

    /// Bytes whose compact re-serialisation is a DIFFERENT string.
    ///
    /// Key order that is not sorted, one space after a colon, and a float
    /// spelled `1.50`. Every one of those moves under a serde round trip, so
    /// a `call_bytes` that quietly re-serialised would be caught here rather
    /// than passing because the fixture happened to be canonical already.
    const UNCANONICAL_BODY: &str = r#"{"zeta":1,"alpha": "two","gamma":1.50}"#;

    #[tokio::test]
    async fn call_bytes_sends_the_body_byte_for_byte() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/traces"))
            .and(header("content-type", "application/json"))
            .and(header("authorization", "Bearer secret"))
            // The assertion. `body_string` compares the raw request body, not
            // a parsed value -- a parsed comparison would pass over exactly
            // the bug this method exists to prevent.
            .and(wiremock::matchers::body_string(UNCANONICAL_BODY))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env)
            .build()
            .expect("client builds");
        client
            .call_bytes(
                Method::POST,
                "/v1/traces",
                &[],
                UNCANONICAL_BODY.as_bytes(),
                &[],
            )
            .await
            .expect("the server accepted the request");
    }

    /// The positive control for the test above: `call_json` on the same value
    /// does NOT put these bytes on the wire. Without this, a `call_bytes` that
    /// was secretly `call_json` could still pass if `serde_json` happened to
    /// reproduce the fixture.
    #[tokio::test]
    async fn call_json_does_not_send_the_same_bytes() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/traces"))
            .and(wiremock::matchers::body_string(UNCANONICAL_BODY))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            // Zero: call_json must NOT match this body.
            .expect(0)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/traces"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env)
            .build()
            .expect("client builds");
        let value: serde_json::Value = serde_json::from_str(UNCANONICAL_BODY).unwrap();
        client
            .call_raw(Method::POST, "/v1/traces", &[], Some(&value))
            .await
            .expect("the server accepted the request");
        // Dropping the server verifies the `expect(0)`.
    }

    #[tokio::test]
    async fn call_bytes_carries_extra_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/traces"))
            .and(header("x-trace-witness-signature", "0xabc"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            .expect(1)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env)
            .build()
            .expect("client builds");
        client
            .call_bytes(
                Method::POST,
                "/v1/traces",
                &[],
                b"{}",
                &[("x-trace-witness-signature", "0xabc")],
            )
            .await
            .expect("the server accepted the request");
    }

    /// A header value that cannot be rendered refuses by name, before the
    /// request is sent -- and the error carries the header NAME, never the
    /// value, which on this path is certificate material.
    #[tokio::test]
    async fn call_bytes_refuses_a_malformed_header_without_echoing_its_value() {
        const MARKER: &str = "zzq-header-marker-zzq";
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("{}"))
            // Nothing may be sent.
            .expect(0)
            .mount(&server)
            .await;

        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let client = Client::builder(server.uri(), env)
            .build()
            .expect("client builds");
        let err = client
            .call_bytes(
                Method::POST,
                "/v1/traces",
                &[],
                b"{}",
                &[("x-trace-witness-certificate", &format!("{MARKER}\n"))],
            )
            .await
            .expect_err("a newline is not a legal header value");
        assert_eq!(err.kind(), "header-malformed");
        for rendering in [format!("{err}"), format!("{err:?}"), err.user_diagnostic()] {
            assert!(!rendering.contains(MARKER), "the error echoed the value");
        }
    }

    /// `call_bytes` is not an escape hatch around the host gate.
    ///
    /// Measured rather than asserted from the code: the refusal for a
    /// disallowed host lands at `build`, before any client exists to call
    /// `call_bytes` on. So the property is that no client for a disallowed
    /// host can be constructed at all, and `call_bytes` inherits it the same
    /// way `call_raw` does. The redundant `host_allowlist.check` inside
    /// `call_bytes` is kept because `compose_url` sets the path from a
    /// caller-supplied string and a future change there is exactly the kind
    /// that would move the host.
    #[tokio::test]
    async fn no_client_exists_for_a_disallowed_host_to_call_bytes_on() {
        let env = unique_bearer_env();
        let _g = EnvGuard::set(env.clone(), "secret");
        let err = Client::builder("https://not-allowed.example", env)
            .host_allowlist(HostAllowlist::from_csv("allowed.example"))
            .build()
            .expect_err("a host outside the allowlist is refused at construction");
        assert_eq!(err.kind(), "host-not-allowed");
    }
}
