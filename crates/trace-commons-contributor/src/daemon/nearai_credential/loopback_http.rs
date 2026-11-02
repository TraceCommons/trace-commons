//! One strict HTTP/1.1 request reader for the sign-in's loopback listeners:
//! the login chooser, the OAuth bounce and the NEAR wallet page.
//!
//! Each listener used to carry its own reader, and each hardened the same
//! edge cases a little differently. This is the wallet's, the strictest:
//!
//! - an exact `METHOD TARGET HTTP/1.1` request line;
//! - header names that are HTTP tokens, values without control characters,
//!   and no repeat of a header this reads (never last-one-wins);
//! - no `Transfer-Encoding`, `Expect` or `Trailer`: one framing only, a
//!   digits-only `Content-Length`;
//! - `Host` must be this listener's own `127.0.0.1:<port>`, which refuses a
//!   DNS-rebound name pointing at the port;
//! - bounded header and body sizes, and a body that overruns its declared
//!   length is refused rather than truncated.
//!
//! What a route then requires (an `Origin`, a content type, an empty body) is
//! each listener's own rule, applied to the [`Head`] this returns.

use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Header bytes accepted from one connection before it is abandoned.
pub(super) const MAX_HEADER_BYTES: usize = 8192;

/// A request this reader refused. Carries nothing: a refusal is answered
/// with a fixed body and never explained.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Invalid;

/// One request head, far enough decoded to route it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Head {
    pub method: String,
    pub target: String,
    pub origin: Option<String>,
    pub content_type: Option<String>,
    pub fetch_site: Option<String>,
    /// `None` when the request sent no `Content-Length`.
    pub length: Option<usize>,
}

impl Head {
    pub fn body_len(&self) -> usize {
        self.length.unwrap_or(0)
    }
}

fn header_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// Decode a head (through its closing blank line) sent to `authority`, with
/// a body of at most `max_body` bytes.
pub(super) fn parse_head(bytes: &[u8], authority: &str, max_body: usize) -> Result<Head, Invalid> {
    if bytes.len() > MAX_HEADER_BYTES {
        return Err(Invalid);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Invalid)?;
    let text = text.strip_suffix("\r\n\r\n").ok_or(Invalid)?;
    let mut lines = text.split("\r\n");
    let parts: Vec<&str> = lines.next().unwrap_or("").split(' ').collect();
    let [method, target, "HTTP/1.1"] = parts.as_slice() else {
        return Err(Invalid);
    };
    if method.is_empty() || !method.bytes().all(header_token) || !target.starts_with('/') {
        return Err(Invalid);
    }
    let mut head = Head {
        method: (*method).to_owned(),
        target: (*target).to_owned(),
        ..Head::default()
    };
    let (mut host, mut length) = (None, None);
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(Invalid)?;
        if name.is_empty() || !name.bytes().all(header_token) {
            return Err(Invalid);
        }
        let value = value.trim_matches([' ', '\t']);
        if value
            .chars()
            .any(|character| character.is_control() && character != '\t')
        {
            return Err(Invalid);
        }
        let slot = match name.to_ascii_lowercase().as_str() {
            "host" => &mut host,
            "origin" => &mut head.origin,
            "content-length" => &mut length,
            "content-type" => &mut head.content_type,
            "sec-fetch-site" => &mut head.fetch_site,
            "transfer-encoding" | "expect" | "trailer" => return Err(Invalid),
            _ => continue,
        };
        if slot.replace(value.to_owned()).is_some() {
            return Err(Invalid);
        }
    }
    if host.as_deref() != Some(authority) {
        return Err(Invalid);
    }
    head.length = match length {
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            Some(value.parse::<usize>().map_err(|_| Invalid)?)
        }
        Some(_) => return Err(Invalid),
        None => None,
    };
    if head.body_len() > max_body {
        return Err(Invalid);
    }
    Ok(head)
}

/// Read one head off a connection, tolerating a client that writes it in
/// several packets. Returns the head bytes (through the blank line) and
/// whatever of the body arrived with them.
pub(super) async fn read_head(stream: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>), Invalid> {
    let mut bytes = Vec::with_capacity(1024);
    let mut chunk = [0; 1024];
    let end = loop {
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let available = (MAX_HEADER_BYTES - bytes.len()).min(chunk.len());
        if available == 0 {
            return Err(Invalid);
        }
        let count = stream
            .read(&mut chunk[..available])
            .await
            .map_err(|_| Invalid)?;
        if count == 0 {
            return Err(Invalid);
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    let rest = bytes.split_off(end);
    Ok((bytes, rest))
}

/// Read the rest of a body of exactly `length` bytes. Reads one byte past
/// it when one is there, so an overrun is refused instead of truncated.
pub(super) async fn read_body(
    stream: &mut TcpStream,
    mut body: Vec<u8>,
    length: usize,
) -> Result<Vec<u8>, Invalid> {
    let mut chunk = [0; 1024];
    while body.len() < length {
        let available = (length - body.len() + 1).min(chunk.len());
        let count = stream
            .read(&mut chunk[..available])
            .await
            .map_err(|_| Invalid)?;
        if count == 0 {
            return Err(Invalid);
        }
        body.extend_from_slice(&chunk[..count]);
    }
    if body.len() != length {
        return Err(Invalid);
    }
    Ok(body)
}

/// [`read_head`], [`parse_head`] and [`read_body`] for a listener whose
/// routes need no check before the body is read.
pub(super) async fn read_request(
    stream: &mut TcpStream,
    authority: &str,
    max_body: usize,
) -> Result<(Head, Vec<u8>), Invalid> {
    let (head_bytes, rest) = read_head(stream).await?;
    let head = parse_head(&head_bytes, authority, max_body)?;
    let body = read_body(stream, rest, head.body_len()).await?;
    Ok((head, body))
}

/// Write a whole response and report whether it was written in time.
pub(super) async fn write_all(stream: &mut TcpStream, response: &[u8], within: Duration) -> bool {
    matches!(
        tokio::time::timeout(within, stream.write_all(response)).await,
        Ok(Ok(()))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTHORITY: &str = "127.0.0.1:54321";

    fn head(lines: &str) -> String {
        format!("{lines}\r\n\r\n")
    }

    #[test]
    fn a_well_formed_head_is_decoded() {
        let parsed = parse_head(
            head(&format!(
                "POST /x HTTP/1.1\r\nHost: {AUTHORITY}\r\nOrigin: http://{AUTHORITY}\r\n\
                 Content-Type: application/x-www-form-urlencoded\r\nSec-Fetch-Site: same-origin\r\n\
                 Content-Length: 3"
            ))
            .as_bytes(),
            AUTHORITY,
            16,
        )
        .unwrap();
        assert_eq!(parsed.method, "POST");
        assert_eq!(parsed.target, "/x");
        assert_eq!(parsed.origin.as_deref(), Some("http://127.0.0.1:54321"));
        assert_eq!(parsed.fetch_site.as_deref(), Some("same-origin"));
        assert_eq!(parsed.length, Some(3));
    }

    #[test]
    fn framing_host_and_header_hygiene_are_enforced() {
        for bad in [
            format!("GET /x HTTP/1.0\r\nHost: {AUTHORITY}"),
            format!("GET  /x HTTP/1.1\r\nHost: {AUTHORITY}"),
            format!("GET x HTTP/1.1\r\nHost: {AUTHORITY}"),
            "GET /x HTTP/1.1\r\nHost: evil.example".to_owned(),
            "GET /x HTTP/1.1".to_owned(),
            format!("GET /x HTTP/1.1\r\nHost: {AUTHORITY}\r\nHost: {AUTHORITY}"),
            format!("GET /x HTTP/1.1\r\nHost: {AUTHORITY}\r\nTransfer-Encoding: chunked"),
            format!("GET /x HTTP/1.1\r\nHost: {AUTHORITY}\r\nExpect: 100-continue"),
            format!("GET /x HTTP/1.1\r\nHost: {AUTHORITY}\r\nContent-Length: +1"),
            format!("GET /x HTTP/1.1\r\nHost: {AUTHORITY}\r\nContent-Length: 17"),
            format!("GET /x HTTP/1.1\r\nHost: {AUTHORITY}\r\nBad Name: x"),
            format!("GET /x HTTP/1.1\r\nHost: {AUTHORITY}\r\nX: a\u{1}b"),
        ] {
            assert_eq!(
                parse_head(head(&bad).as_bytes(), AUTHORITY, 16).unwrap_err(),
                Invalid,
                "{bad}"
            );
        }
        assert!(parse_head(&vec![b'a'; MAX_HEADER_BYTES + 1], AUTHORITY, 16).is_err());
    }
}
