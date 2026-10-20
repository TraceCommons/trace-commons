//! Binding and serving.
//!
//! There is deliberately no host parameter anywhere in this module. IronWire
//! binds `127.0.0.1` and nothing else (`docs/TRUST.md` I1) — a credential
//! custodian that can be exposed to a network is a different, worse product.

use std::net::{Ipv4Addr, SocketAddr};

use axum::Router;

use crate::facade;
use crate::state::AppState;

/// Failure starting the daemon.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The port is taken, most likely by another IronWire.
    #[error("port {port} is already in use — another IronWire may be running (`ironwire status`)")]
    PortInUse {
        /// Port we tried.
        port: u16,
    },
    /// Any other bind or serve failure.
    #[error("serving on 127.0.0.1:{port}: {source}")]
    Io {
        /// Port we tried.
        port: u16,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
}

/// Build the full application router.
pub fn app(state: AppState) -> Router {
    Router::new()
        .nest("/anthropic", facade::anthropic::router())
        .nest("/openai", facade::openai::router())
        .nest("/_ironwire", crate::control::router())
        .with_state(state)
}

/// Bind loopback.
///
/// Separate from [`serve_on`] so a caller can announce the listener only after
/// it exists — printing "listening on 8463" and *then* failing to bind is a
/// small lie that sends people looking in the wrong place.
///
/// # Errors
///
/// [`ServeError::PortInUse`] when something already holds the port, or
/// [`ServeError::Io`] for any other bind failure.
pub async fn bind(port: u16) -> Result<tokio::net::TcpListener, ServeError> {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    tokio::net::TcpListener::bind(addr).await.map_err(|source| {
        if source.kind() == std::io::ErrorKind::AddrInUse {
            ServeError::PortInUse { port }
        } else {
            ServeError::Io { port, source }
        }
    })
}

/// Serve on an already-bound listener until `shutdown` resolves.
///
/// Draining is the point of the graceful part: a streamed model response
/// mid-turn is the outage this product exists to prevent, so an in-flight
/// request gets to finish. The one response that would never finish is the
/// event stream, which is held open by design — so the handlers are told
/// first, and they end it themselves (`crate::shutdown`).
///
/// # Errors
///
/// [`ServeError::Io`] when the server fails.
pub async fn serve_on(
    listener: tokio::net::TcpListener,
    state: AppState,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    let port = listener.local_addr().map_or(0, |addr| addr.port());
    tracing::info!(port, "IronWire listening");
    let closing = state.shutdown.clone();
    axum::serve(listener, app(state))
        .with_graceful_shutdown(async move {
            shutdown.await;
            closing.begin();
        })
        .await
        .map_err(|source| ServeError::Io { port, source })
}

/// Bind loopback and serve until `shutdown` resolves.
///
/// # Errors
///
/// [`ServeError`] when the port cannot be bound or the server fails.
pub async fn serve(
    state: AppState,
    port: u16,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    serve_on(bind(port).await?, state, shutdown).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ironwire_core::config::Config;
    use ironwire_creds::ConsentLedger;
    use tower::ServiceExt;

    use crate::state::BackendRegistry;

    fn state() -> AppState {
        AppState::new(
            BackendRegistry::new(),
            Config::default(),
            ConsentLedger::default(),
            "test-token".to_string(),
        )
    }

    #[tokio::test]
    async fn the_listener_is_always_loopback() {
        // TRUST.md I1. There is no host parameter anywhere in this module, and
        // this test is what keeps it that way: a credential custodian that can
        // be exposed to a network is a different, worse product.
        let listener = bind(0).await.expect("binds an ephemeral port");
        let addr = listener.local_addr().expect("local addr");
        assert!(
            addr.ip().is_loopback(),
            "IronWire bound {addr}, which is not loopback"
        );
    }

    /// The bug a menu bar app made routine: `/_ironwire/events` is held open
    /// for the life of a client, so a graceful shutdown that waits for it waits
    /// forever. `systemctl --user stop`, `brew services restart` and a plain
    /// `kill` all hung for as long as anybody had a client open.
    ///
    /// Over a real socket rather than `oneshot`, because the thing under test
    /// is the draining behaviour of the server and not the handler — a
    /// `oneshot` call never has a connection for `axum` to wait on.
    #[tokio::test]
    async fn a_held_open_event_stream_does_not_outlive_the_daemon() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = bind(0).await.expect("binds");
        let port = listener.local_addr().expect("local addr").port();
        let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(serve_on(listener, state(), async move {
            let _ = stopped.await;
        }));

        // Hold the stream open the way the menu bar app does, and read the
        // framing so the connection is established before anything stops.
        let mut client = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connects");
        client
            .write_all(
                b"GET /_ironwire/events HTTP/1.1\r\n\
                  host: 127.0.0.1\r\n\
                  authorization: Bearer test-token\r\n\r\n",
            )
            .await
            .expect("writes the request");
        let mut head = [0_u8; 12];
        client
            .read_exact(&mut head)
            .await
            .expect("reads a response");
        assert!(
            String::from_utf8_lossy(&head).contains("200"),
            "the event stream did not open: {}",
            String::from_utf8_lossy(&head)
        );

        stop.send(()).expect("the server is still running");

        let served = tokio::time::timeout(std::time::Duration::from_secs(10), server)
            .await
            .expect("the daemon is still waiting for a stream that never ends")
            .expect("the server task panicked");
        assert!(served.is_ok(), "serving failed: {served:?}");

        // And the client is told why, rather than finding a dead socket.
        let mut rest = String::new();
        client
            .read_to_string(&mut rest)
            .await
            .expect("reads the rest of the stream");
        assert!(
            rest.contains(": closing"),
            "the stream ended without saying the daemon was closing: {rest:?}"
        );
    }

    #[tokio::test]
    async fn a_taken_port_is_reported_as_such_rather_than_as_a_generic_io_error() {
        let held = bind(0).await.expect("binds");
        let port = held.local_addr().expect("local addr").port();
        let err = bind(port).await.expect_err("port is held");
        assert!(matches!(err, ServeError::PortInUse { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn health_needs_no_token() {
        let response = app(state())
            .oneshot(
                Request::builder()
                    .uri("/_ironwire/health")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("served");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn the_control_api_refuses_an_unauthenticated_caller() {
        // Another local user must not be able to read the ledger or move
        // someone's traffic.
        let response = app(state())
            .oneshot(
                Request::builder()
                    .uri("/_ironwire/status")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("served");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn the_control_api_accepts_the_daemons_token() {
        let response = app(state())
            .oneshot(
                Request::builder()
                    .uri("/_ironwire/status")
                    .header("authorization", "Bearer test-token")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("served");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn an_unconfigured_daemon_says_so_in_the_provider_error_shape() {
        // A coding agent must receive something it already knows how to
        // handle, not an IronWire-shaped surprise.
        let response = app(state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/anthropic/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"claude-opus-4-6","messages":[]}"#))
                    .expect("request builds"),
            )
            .await
            .expect("served");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value["type"], "error");
        assert_eq!(value["error"]["type"], "api_error");
    }

    #[tokio::test]
    async fn a_malformed_body_is_rejected_before_any_routing() {
        let response = app(state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/anthropic/v1/messages")
                    .header("content-type", "application/json")
                    .body(Body::from("not json"))
                    .expect("request builds"),
            )
            .await
            .expect("served");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn count_tokens_exists_because_claude_code_depends_on_it() {
        // A 404 here silently breaks Claude Code's context accounting
        // (PROTOCOL.md §1), so the route must exist even with no backends.
        let response = app(state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/anthropic/v1/messages/count_tokens")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"model":"claude-opus-4-6","messages":[]}"#))
                    .expect("request builds"),
            )
            .await
            .expect("served");
        assert_ne!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_models_list_offers_only_what_we_could_actually_serve() {
        let response = app(state())
            .oneshot(
                Request::builder()
                    .uri("/anthropic/v1/models")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("served");
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(
            value["data"].as_array().expect("array").len(),
            0,
            "no backends means no models on offer"
        );
    }
}
