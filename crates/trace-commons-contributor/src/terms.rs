//! The published Terms of Service document: one bounded, anonymous fetch of
//! [`crate::channel::TERMS_DOCUMENT_URL`], hashed over the exact bytes served.
//!
//! Design: `docs/superpowers/specs/2026-11-09-terms-scroll-acceptance-design.md`,
//! section 2. The first run reads it through the C ABI before the daemon
//! exists ([`fetch_published_blocking`]); the daemon re-reads it on its own
//! schedule (`daemon::terms_state`).
//!
//! The rules: HTTPS only, the document's own host only, no redirects, no
//! proxy, no cookies, a 256 KiB cap and short timeouts. The request carries
//! no credential of any kind. Every refusal is a fixed label, safe to log.
//!
//! Both channels read the production document (owner, 2026-11-09), so this
//! fetch deliberately does not go through
//! [`crate::channel::refuse_other_channel_origin`]: a staging build holds the
//! production site on that deny list for service origins, and this is not
//! one.

use reqwest::Url;
use sha2::{Digest, Sha256};
use trace_commons_protocol::terms::TermsDocument;

/// The largest document accepted, in bytes.
pub const TERMS_MAX_BYTES: usize = 262_144;

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// No answer, a non-2xx answer (a redirect included), or a timeout.
pub const TERMS_UNAVAILABLE: &str = "terms-unavailable";
/// The URL was not HTTPS on the document's own host. Nothing was sent.
pub const TERMS_HOST_REFUSED: &str = "terms-host-refused";
/// The document was larger than [`TERMS_MAX_BYTES`].
pub const TERMS_TOO_LARGE: &str = "terms-too-large";

/// A parsed document and the lowercase hex SHA-256 of the exact bytes it was
/// parsed from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedTerms {
    pub document: TermsDocument,
    pub content_sha256: String,
}

/// Fetch the published document under the production policy.
pub async fn fetch_published() -> Result<FetchedTerms, &'static str> {
    let url = Url::parse(crate::channel::TERMS_DOCUMENT_URL).map_err(|_| TERMS_HOST_REFUSED)?;
    // The allowed host is the document URL's own: no host is spelled here.
    let host = url.host_str().ok_or(TERMS_HOST_REFUSED)?.to_string();
    fetch_from(&url, &host, true).await
}

/// [`fetch_published`] on its own current-thread runtime, for the C ABI,
/// which calls it from a thread with no runtime.
pub fn fetch_published_blocking() -> Result<FetchedTerms, &'static str> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| TERMS_UNAVAILABLE)?;
    runtime.block_on(fetch_published())
}

/// One anonymous GET of `url`, refused unless its host is `allowed_host`
/// and, when `require_https`, its scheme is HTTPS. Tests pass `false` to
/// reach a loopback listener; every production caller passes `true`.
pub(crate) async fn fetch_from(
    url: &Url,
    allowed_host: &str,
    require_https: bool,
) -> Result<FetchedTerms, &'static str> {
    let scheme_ok = match url.scheme() {
        "https" => true,
        "http" => !require_https,
        _ => false,
    };
    if !scheme_ok
        || url.host_str() != Some(allowed_host)
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(TERMS_HOST_REFUSED);
    }
    let http = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .build()
        .map_err(|_| TERMS_UNAVAILABLE)?;
    let response = http
        .get(url.clone())
        .send()
        .await
        .map_err(|_| TERMS_UNAVAILABLE)?;
    let bytes = read_bounded(response, TERMS_MAX_BYTES).await?;
    let content_sha256 = hex::encode(Sha256::digest(&bytes));
    let document = TermsDocument::parse(&bytes)?;
    Ok(FetchedTerms {
        document,
        content_sha256,
    })
}

/// A 2xx response's body, read up to `max_bytes` and refused past it. A
/// redirect is a non-2xx answer here: the client follows none.
async fn read_bounded(
    mut response: reqwest::Response,
    max_bytes: usize,
) -> Result<Vec<u8>, &'static str> {
    if !response.status().is_success() {
        return Err(TERMS_UNAVAILABLE);
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(TERMS_TOO_LARGE);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| TERMS_UNAVAILABLE)? {
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(TERMS_TOO_LARGE);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{
        Router,
        http::{StatusCode, header},
        response::IntoResponse,
        routing::get,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    const FIXTURE: &str = include_str!("../../trace-commons-protocol/tests/fixtures/terms-v1.json");

    async fn serve(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        format!("http://{address}")
    }

    async fn fetch(base: &str, path: &str) -> Result<FetchedTerms, &'static str> {
        let url = Url::parse(&format!("{base}{path}")).unwrap();
        fetch_from(&url, "127.0.0.1", false).await
    }

    #[tokio::test]
    async fn fetches_and_hashes_exact_bytes() {
        let base = serve(Router::new().route("/legal/terms.json", get(|| async { FIXTURE }))).await;
        let fetched = fetch(&base, "/legal/terms.json").await.unwrap();
        assert_eq!(
            fetched.content_sha256,
            hex::encode(Sha256::digest(FIXTURE.as_bytes()))
        );
        assert_eq!(fetched.content_sha256.len(), 64);
        assert!(
            fetched
                .content_sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        assert_eq!(
            fetched.document,
            TermsDocument::parse(FIXTURE.as_bytes()).unwrap()
        );
    }

    #[tokio::test]
    async fn refuses_a_redirect() {
        let followed = Arc::new(AtomicUsize::new(0));
        let hits = Arc::clone(&followed);
        let app = Router::new()
            .route(
                "/legal/terms.json",
                get(|| async {
                    (
                        StatusCode::FOUND,
                        [(header::LOCATION, "/elsewhere.json")],
                        "",
                    )
                        .into_response()
                }),
            )
            .route(
                "/elsewhere.json",
                get(move || {
                    hits.fetch_add(1, Ordering::SeqCst);
                    async { FIXTURE }
                }),
            );
        let base = serve(app).await;
        assert_eq!(
            fetch(&base, "/legal/terms.json").await,
            Err(TERMS_UNAVAILABLE)
        );
        assert_eq!(followed.load(Ordering::SeqCst), 0, "the redirect was followed");
    }

    #[tokio::test]
    async fn refuses_another_host() {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&hits);
        let base = serve(Router::new().route(
            "/legal/terms.json",
            get(move || {
                counter.fetch_add(1, Ordering::SeqCst);
                async { FIXTURE }
            }),
        ))
        .await;
        let url = Url::parse(&format!("{base}/legal/terms.json")).unwrap();
        assert_eq!(
            fetch_from(&url, "tracecommons.ai", false).await,
            Err(TERMS_HOST_REFUSED)
        );
        assert_eq!(hits.load(Ordering::SeqCst), 0, "a request was made");
    }

    #[tokio::test]
    async fn refuses_oversize_by_header_and_by_stream() {
        // By header: axum sets Content-Length on a fixed body.
        let base = serve(Router::new().route(
            "/legal/terms.json",
            get(|| async { "x".repeat(TERMS_MAX_BYTES + 1) }),
        ))
        .await;
        assert_eq!(
            fetch(&base, "/legal/terms.json").await,
            Err(TERMS_TOO_LARGE)
        );

        // By stream: a chunked body with no Content-Length, written by hand.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            let _ = socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                      Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .await;
            let chunk = vec![b'x'; 16 * 1024];
            let mut sent = 0;
            while sent <= TERMS_MAX_BYTES {
                let head = format!("{:x}\r\n", chunk.len());
                if socket.write_all(head.as_bytes()).await.is_err()
                    || socket.write_all(&chunk).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
                sent += chunk.len();
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        });
        assert_eq!(
            fetch(&format!("http://{address}"), "/legal/terms.json").await,
            Err(TERMS_TOO_LARGE)
        );
    }

    #[tokio::test]
    async fn refuses_bad_schema() {
        let base = serve(Router::new().route(
            "/legal/terms.json",
            get(|| async {
                FIXTURE.replace("trace_commons.terms.v1", "trace_commons.terms.v2")
            }),
        ))
        .await;
        assert_eq!(
            fetch(&base, "/legal/terms.json").await,
            Err("terms-schema-unknown")
        );
    }

    #[tokio::test]
    async fn production_policy_requires_https() {
        let url = Url::parse("http://tracecommons.ai/legal/terms.json").unwrap();
        assert_eq!(
            fetch_from(&url, "tracecommons.ai", true).await,
            Err(TERMS_HOST_REFUSED)
        );
    }

    /// The published document URL is HTTPS on its own host, so the
    /// production policy would let it through.
    #[test]
    fn the_published_url_passes_the_production_policy() {
        let url = Url::parse(crate::channel::TERMS_DOCUMENT_URL).unwrap();
        assert_eq!(url.scheme(), "https");
        assert!(url.host_str().is_some());
        assert!(url.username().is_empty() && url.password().is_none());
    }
}
