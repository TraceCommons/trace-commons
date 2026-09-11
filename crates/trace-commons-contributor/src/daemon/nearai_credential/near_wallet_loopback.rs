// INTEGRATION: declare beside near_wallet. bind() returns the one-time browser
// URL and a NearWalletListener; open that URL through the OS browser launcher,
// then await owner.wait(). Dropping the owner or its wait future cancels locally.
// bind() and wait() may run on separate runtimes: the owner retains an
// unregistered std listener until wait() registers it on the receiving runtime.
// A returned proof has passed the adapter's signature/binding checks and consumed
// its challenge. The caller alone submits it once to the pinned Cloud API; this
// module performs no Cloud request, wallet transaction, persistence, or logging.
//! Bounded IPv4 loopback transport for an external wallet's signed fragment.

use std::net::Ipv4Addr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{Instant, timeout_at};

use crate::daemon::nearai_credential::near_wallet::{
    CALLBACK_PATH, DEPOSIT_PATH, MAX_CALLBACK_BYTES, NearWalletChallenge, NearWalletError,
    VerifiedNearWalletSignIn,
};
use crate::daemon::nearai_credential::near_wallet_page::{self, ASSET_TIMEOUT, Assets};

const MAX_HEADER_BYTES: usize = 8192;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(2);

/// Owns the socket and challenge; neither can outlive cancellation of wait().
pub(crate) struct NearWalletListener {
    listener: std::net::TcpListener,
    challenge: NearWalletChallenge,
    authority: String,
    deadline: Instant,
}

pub(crate) async fn bind() -> Result<(String, NearWalletListener), NearWalletError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| NearWalletError::Unavailable)?;
    let port = listener
        .local_addr()
        .map_err(|_| NearWalletError::Unavailable)?
        .port();
    let now = unix_millis()?;
    let started = Instant::now();
    let challenge = NearWalletChallenge::new(port, now)?;
    let deadline = started + Duration::from_millis(challenge.expires_at_ms() - now);
    let browser_url = challenge.browser_url()?;
    let listener = listener
        .into_std()
        .map_err(|_| NearWalletError::Unavailable)?;
    Ok((
        browser_url,
        NearWalletListener {
            listener,
            challenge,
            authority: format!("127.0.0.1:{port}"),
            deadline,
        },
    ))
}

impl NearWalletListener {
    pub(crate) async fn wait(self) -> Result<VerifiedNearWalletSignIn, NearWalletError> {
        let listener =
            TcpListener::from_std(self.listener).map_err(|_| NearWalletError::Unavailable)?;
        timeout_at(
            self.deadline,
            Self::receive(listener, self.challenge, self.authority, self.deadline),
        )
        .await
        .map_err(|_| NearWalletError::Expired)?
    }

    async fn receive(
        listener: TcpListener,
        mut challenge: NearWalletChallenge,
        authority: String,
        attempt_deadline: Instant,
    ) -> Result<VerifiedNearWalletSignIn, NearWalletError> {
        let page = near_wallet_page::render(&challenge)?;
        let mut assets = Assets::new()?;
        loop {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|_| NearWalletError::Unavailable)?;
            let deadline = attempt_deadline.min(Instant::now() + CONNECTION_TIMEOUT);
            let request = timeout_at(deadline, read_request(&mut stream, &authority)).await;
            if Instant::now() >= attempt_deadline {
                return Err(NearWalletError::Expired);
            }
            match request {
                Ok(Ok(Request::Callback)) => {
                    respond(
                        &mut stream,
                        deadline,
                        true,
                        "text/html; charset=utf-8",
                        &page,
                    )
                    .await;
                }
                Ok(Ok(Request::Asset(path))) => {
                    let deadline = attempt_deadline.min(Instant::now() + ASSET_TIMEOUT);
                    let result = timeout_at(deadline, assets.load(&path)).await;
                    match result {
                        Ok(Ok(body)) => {
                            respond(
                                &mut stream,
                                deadline,
                                true,
                                "text/javascript; charset=utf-8",
                                body,
                            )
                            .await
                        }
                        _ => {
                            respond(
                                &mut stream,
                                deadline,
                                false,
                                "text/plain; charset=utf-8",
                                "Not accepted.",
                            )
                            .await
                        }
                    }
                }
                Ok(Ok(Request::Deposit(body))) => {
                    let result = challenge.verify_callback(&body, unix_millis()?);
                    let accepted =
                        result.is_ok() || matches!(result, Err(NearWalletError::Cancelled));
                    respond(
                        &mut stream,
                        deadline,
                        accepted,
                        "text/plain; charset=utf-8",
                        if accepted {
                            "Received."
                        } else {
                            "Not accepted."
                        },
                    )
                    .await;
                    match result {
                        Ok(proof) => return Ok(proof),
                        Err(NearWalletError::Invalid) => {}
                        Err(error) => return Err(error),
                    }
                }
                _ => {
                    respond(
                        &mut stream,
                        deadline,
                        false,
                        "text/plain; charset=utf-8",
                        "Not accepted.",
                    )
                    .await;
                }
            }
        }
    }
}

fn unix_millis() -> Result<u64, NearWalletError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .ok_or(NearWalletError::Unavailable)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Route {
    Callback,
    Deposit,
    Asset(String),
}

enum Request {
    Callback,
    Deposit(String),
    Asset(String),
}

fn parse_head(bytes: &[u8], authority: &str) -> Result<(Route, usize), NearWalletError> {
    if bytes.len() > MAX_HEADER_BYTES {
        return Err(NearWalletError::Invalid);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| NearWalletError::Invalid)?;
    let text = text
        .strip_suffix("\r\n\r\n")
        .ok_or(NearWalletError::Invalid)?;
    let mut lines = text.split("\r\n");
    let route = match lines.next() {
        Some(line) if line == format!("GET {CALLBACK_PATH} HTTP/1.1") => Route::Callback,
        Some(line) if line == format!("POST {DEPOSIT_PATH} HTTP/1.1") => Route::Deposit,
        Some(line) => {
            let path = line
                .strip_prefix("GET ")
                .and_then(|line| line.strip_suffix(" HTTP/1.1"))
                .and_then(near_wallet_page::asset_path)
                .ok_or(NearWalletError::Invalid)?;
            Route::Asset(path.to_owned())
        }
        _ => return Err(NearWalletError::Invalid),
    };
    let (mut host, mut origin, mut length, mut content_type) = (None, None, None, None);
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(NearWalletError::Invalid)?;
        if name.is_empty() || !name.bytes().all(header_token) {
            return Err(NearWalletError::Invalid);
        }
        let value = value.trim_matches([' ', '\t']);
        if value
            .chars()
            .any(|character| character.is_control() && character != '\t')
        {
            return Err(NearWalletError::Invalid);
        }
        let slot = match name.to_ascii_lowercase().as_str() {
            "host" => &mut host,
            "origin" => &mut origin,
            "content-length" => &mut length,
            "content-type" => &mut content_type,
            "transfer-encoding" | "expect" | "trailer" => return Err(NearWalletError::Invalid),
            _ => continue,
        };
        if slot.replace(value).is_some() {
            return Err(NearWalletError::Invalid);
        }
    }
    if host != Some(authority) {
        return Err(NearWalletError::Invalid);
    }
    let size = match length {
        Some(value) if !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()) => {
            value
                .parse::<usize>()
                .map_err(|_| NearWalletError::Invalid)?
        }
        Some(_) => return Err(NearWalletError::Invalid),
        None => 0,
    };
    match &route {
        Route::Callback | Route::Asset(_) if size == 0 => {}
        Route::Deposit
            if length.is_some()
                && (1..=MAX_CALLBACK_BYTES).contains(&size)
                && origin == Some(format!("http://{authority}").as_str())
                && content_type.is_some_and(form_content_type) => {}
        _ => return Err(NearWalletError::Invalid),
    }
    Ok((route, size))
}

fn header_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

fn form_content_type(value: &str) -> bool {
    let mut parts = value.split(';').map(str::trim);
    if !parts
        .next()
        .is_some_and(|part| part.eq_ignore_ascii_case("application/x-www-form-urlencoded"))
    {
        return false;
    }
    let valid = match parts.next() {
        None => true,
        Some(parameter) => parameter.split_once('=').is_some_and(|(name, value)| {
            name.trim().eq_ignore_ascii_case("charset")
                && (value.trim().eq_ignore_ascii_case("utf-8")
                    || value.trim().eq_ignore_ascii_case("\"utf-8\""))
        }),
    };
    valid && parts.next().is_none()
}

async fn read_request(stream: &mut TcpStream, authority: &str) -> Result<Request, NearWalletError> {
    let mut bytes = Vec::with_capacity(MAX_HEADER_BYTES);
    let mut chunk = [0; 1024];
    let end = loop {
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        let available = (MAX_HEADER_BYTES - bytes.len()).min(chunk.len());
        if available == 0 {
            return Err(NearWalletError::Invalid);
        }
        let count = stream
            .read(&mut chunk[..available])
            .await
            .map_err(|_| NearWalletError::Invalid)?;
        if count == 0 {
            return Err(NearWalletError::Invalid);
        }
        bytes.extend_from_slice(&chunk[..count]);
    };
    let (route, length) = parse_head(&bytes[..end], authority)?;
    let mut body = bytes[end..].to_vec();
    while body.len() < length {
        // Read one extra byte when available so an overrun is refused instead
        // of silently truncating bytes coalesced with the declared body.
        let available = (length - body.len() + 1).min(chunk.len());
        let count = stream
            .read(&mut chunk[..available])
            .await
            .map_err(|_| NearWalletError::Invalid)?;
        if count == 0 {
            return Err(NearWalletError::Invalid);
        }
        body.extend_from_slice(&chunk[..count]);
    }
    if body.len() != length {
        return Err(NearWalletError::Invalid);
    }
    match route {
        Route::Callback => Ok(Request::Callback),
        Route::Asset(path) => Ok(Request::Asset(path)),
        Route::Deposit => String::from_utf8(body)
            .map(Request::Deposit)
            .map_err(|_| NearWalletError::Invalid),
    }
}

async fn respond(
    stream: &mut TcpStream,
    deadline: Instant,
    accepted: bool,
    kind: &str,
    body: &str,
) {
    let status = if accepted {
        "200 OK"
    } else {
        "400 Bad Request"
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nReferrer-Policy: no-referrer\r\n\
         X-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\n\
         Connection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = timeout_at(deadline, stream.write_all(response.as_bytes())).await;
}

#[cfg(test)]
mod tests {

    use base64::{Engine, engine::general_purpose::STANDARD};
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use sha2::{Digest, Sha256};
    use tokio::time::timeout;
    use url::Url;

    use crate::daemon::nearai_credential::near_wallet_loopback::*;

    const TEST_TIMEOUT: Duration = Duration::from_secs(8);
    const AUTHORITY: &str = "127.0.0.1:54321";

    fn head(method: &str, path: &str, authority: &str, length: usize) -> String {
        format!(
            "{method} {path} HTTP/1.1\r\nHost: {authority}\r\nOrigin: http://{authority}\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {length}\r\n\r\n"
        )
    }

    fn authority(browser_url: &str) -> String {
        let url = Url::parse(browser_url).unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        format!("127.0.0.1:{}", url.port().unwrap())
    }

    async fn send(authority: &str, request: &[u8], fragmented: bool) -> String {
        timeout(TEST_TIMEOUT, async {
            let mut stream = TcpStream::connect(authority).await.unwrap();
            if fragmented {
                for bytes in request.chunks(7) {
                    stream.write_all(bytes).await.unwrap();
                    tokio::task::yield_now().await;
                }
            } else {
                stream.write_all(request).await.unwrap();
            }
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.unwrap();
            String::from_utf8(response).unwrap()
        })
        .await
        .expect("local HTTP request must complete")
    }

    async fn signed_form(browser_url: &str) -> String {
        let url = Url::parse(browser_url).unwrap();
        let address = authority(browser_url);
        let response = send(
            &address,
            format!("GET {CALLBACK_PATH} HTTP/1.1\r\nHost: {address}\r\n\r\n").as_bytes(),
            false,
        )
        .await;
        let json = response
            .split_once("<script id=\"challenge\" type=\"application/json\">")
            .unwrap()
            .1
            .split_once("</script>")
            .unwrap()
            .0;
        let config: serde_json::Value = serde_json::from_str(json).unwrap();
        let nonce: Vec<u8> = serde_json::from_value(config["nonce"].clone()).unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&[37; 32]).unwrap();
        let mut message = ((1u32 << 31) + 413).to_le_bytes().to_vec();
        let value = config["message"].as_str().unwrap();
        message.extend_from_slice(&(value.len() as u32).to_le_bytes());
        message.extend_from_slice(value.as_bytes());
        message.extend_from_slice(&nonce);
        let value = config["recipient"].as_str().unwrap();
        message.extend_from_slice(&(value.len() as u32).to_le_bytes());
        message.extend_from_slice(value.as_bytes());
        message.push(0);
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("accountId", "synthetic.near")
            .append_pair("publicKey", &public_key(key.public_key().as_ref()))
            .append_pair(
                "signature",
                &STANDARD.encode(key.sign(&Sha256::digest(message)).as_ref()),
            )
            .append_pair("state", url.fragment().unwrap())
            .finish()
    }

    fn public_key(bytes: &[u8]) -> String {
        const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
        let mut input = bytes.to_vec();
        let zeros = input.iter().take_while(|byte| **byte == 0).count();
        let mut output = Vec::new();
        let mut start = zeros;
        while start < input.len() {
            let mut remainder = 0u16;
            for byte in &mut input[start..] {
                let value = remainder * 256 + u16::from(*byte);
                *byte = (value / 58) as u8;
                remainder = value % 58;
            }
            output.push(ALPHABET[remainder as usize]);
            while start < input.len() && input[start] == 0 {
                start += 1;
            }
        }
        output.extend(std::iter::repeat_n(b'1', zeros));
        output.reverse();
        format!("ed25519:{}", String::from_utf8(output).unwrap())
    }

    #[test]
    fn parser_requires_exact_routes_authority_origin_and_unambiguous_framing() {
        let valid = head("POST", DEPOSIT_PATH, AUTHORITY, 1);
        assert_eq!(
            parse_head(valid.as_bytes(), AUTHORITY),
            Ok((Route::Deposit, 1))
        );
        assert_eq!(
            parse_head(
                head("POST", DEPOSIT_PATH, AUTHORITY, MAX_CALLBACK_BYTES).as_bytes(),
                AUTHORITY
            ),
            Ok((Route::Deposit, MAX_CALLBACK_BYTES))
        );
        let get = format!("GET {CALLBACK_PATH} HTTP/1.1\r\nHost: {AUTHORITY}\r\n\r\n");
        assert_eq!(
            parse_head(get.as_bytes(), AUTHORITY),
            Ok((Route::Callback, 0))
        );
        let mut invalid = vec![
            valid.replace("Host:", "Host :"),
            valid.replace(AUTHORITY, "localhost:54321"),
            valid.replace("Origin: http://", "Origin: https://"),
            valid.replace(&format!("Origin: http://{AUTHORITY}\r\n"), ""),
            valid.replace("Content-Length: 1\r\n", ""),
            valid.replace("Content-Length: 1", "Content-Length: 0"),
            valid.replace("Content-Length: 1", "Content-Length: +1"),
            valid.replace("Content-Length: 1", "Content-Length: 1,1"),
            valid.replace("Content-Length: 1", "Content-Length: 184467440737095516160"),
            valid.replace(
                "Content-Length: 1",
                &format!("Content-Length: {}", MAX_CALLBACK_BYTES + 1),
            ),
            valid.replace(DEPOSIT_PATH, &format!("http://{AUTHORITY}{DEPOSIT_PATH}")),
            valid.replace(DEPOSIT_PATH, &format!("{DEPOSIT_PATH}?extra=1")),
            valid.replace("POST ", "POST  "),
            valid.replace("POST ", "OPTIONS "),
            valid.replace(DEPOSIT_PATH, "/near-ai/near-wallet/%72esult"),
            valid.replace("HTTP/1.1", "HTTP/1.0"),
            valid.replace(
                "Content-Type: application/x-www-form-urlencoded",
                "Content-Type: text/plain",
            ),
            get.replace("\r\n\r\n", "\r\nContent-Length: 1\r\n\r\n"),
            valid.replace("\r\nHost:", "\r\n Host:"),
            valid.replace("\r\n\r\n", "\r\nX-Test: invalid\0value\r\n\r\n"),
            valid.replace("\r\n", "\n"),
        ];
        for header in [
            "hOsT: 127.0.0.1:54321",
            "ORIGIN: http://127.0.0.1:54321",
            "Content-Length: 1",
            "Content-Type: application/x-www-form-urlencoded",
            "Transfer-Encoding: chunked",
            "Transfer-Encoding:",
            "Expect: 100-continue",
            "Trailer: Content-Length",
        ] {
            invalid.push(valid.replace("\r\n\r\n", &format!("\r\n{header}\r\n\r\n")));
        }
        for request in invalid {
            assert!(parse_head(request.as_bytes(), AUTHORITY).is_err());
        }
        let mut utf8 = valid.into_bytes();
        utf8[0] = 0xff;
        assert!(parse_head(&utf8, AUTHORITY).is_err());
        assert!(parse_head(&vec![b'a'; MAX_HEADER_BYTES + 1], AUTHORITY).is_err());
    }

    #[test]
    fn content_type_accepts_only_form_data_with_optional_utf8() {
        for value in [
            "application/x-www-form-urlencoded",
            "application/x-www-form-urlencoded; charset=UTF-8",
            "Application/X-WWW-Form-Urlencoded; charset=\"utf-8\"",
        ] {
            assert!(form_content_type(value));
        }
        for value in [
            "text/plain",
            "application/x-www-form-urlencoded;",
            "application/x-www-form-urlencoded; charset=latin1",
            "application/x-www-form-urlencoded; charset=UTF-8; charset=UTF-8",
            "application/x-www-form-urlencoded; boundary=x",
        ] {
            assert!(!form_content_type(value));
        }
    }

    #[test]
    fn the_listener_survives_the_binding_runtime_being_dropped() {
        let binding_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (browser, owner) = binding_runtime.block_on(bind()).unwrap();
        drop(binding_runtime);
        let receiving_runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        receiving_runtime.block_on(async {
            let address = authority(&browser);
            let waiting = tokio::spawn(owner.wait());
            let form = signed_form(&browser).await;
            let post = format!("{}{form}", head("POST", DEPOSIT_PATH, &address, form.len()));
            assert!(
                send(&address, post.as_bytes(), false)
                    .await
                    .starts_with("HTTP/1.1 200 OK")
            );
            assert_eq!(
                timeout(TEST_TIMEOUT, waiting)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap()
                    .account_id(),
                "synthetic.near"
            );
        });
    }

    #[tokio::test]
    async fn fragmented_callback_and_real_signed_deposit_complete_the_listener() {
        let (browser, owner) = bind().await.unwrap();
        let address = authority(&browser);
        let waiting = tokio::spawn(owner.wait());
        let get = format!("GET {CALLBACK_PATH} HTTP/1.1\r\nHost: {address}\r\n\r\n");
        let response = send(&address, get.as_bytes(), true).await;
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains("<script id=\"challenge\" type=\"application/json\">"));
        for header in [
            "Cache-Control: no-store",
            "Referrer-Policy: no-referrer",
            "X-Content-Type-Options: nosniff",
            "X-Frame-Options: DENY",
            "Connection: close",
        ] {
            assert!(response.contains(header));
        }
        assert!(!response.contains("Access-Control-Allow"));
        assert!(!waiting.is_finished());
        let form = signed_form(&browser).await;
        let post = format!("{}{form}", head("POST", DEPOSIT_PATH, &address, form.len()));
        assert!(
            send(&address, post.as_bytes(), true)
                .await
                .starts_with("HTTP/1.1 200 OK")
        );
        let proof = timeout(TEST_TIMEOUT, waiting)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(proof.account_id(), "synthetic.near");
        assert!(TcpStream::connect(&address).await.is_err());
    }

    #[tokio::test]
    async fn invalid_probes_and_signatures_do_not_consume_a_valid_challenge() {
        let (browser, owner) = bind().await.unwrap();
        let address = authority(&browser);
        let waiting = tokio::spawn(owner.wait());
        let form = signed_form(&browser).await;
        let valid = format!("{}{form}", head("POST", DEPOSIT_PATH, &address, form.len()));
        let mut requests = vec![
            valid
                .replace(&format!("Host: {address}"), "Host: attacker.invalid")
                .into_bytes(),
            valid
                .replace(
                    &format!("Origin: http://{address}"),
                    "Origin: https://attacker.invalid",
                )
                .into_bytes(),
            valid
                .replace("\r\n\r\n", "\r\nTransfer-Encoding: chunked\r\n\r\n")
                .into_bytes(),
            format!("{}x", valid).into_bytes(),
            head("POST", DEPOSIT_PATH, &address, MAX_CALLBACK_BYTES + 1).into_bytes(),
        ];
        let bad_signature = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(
                url::form_urlencoded::parse(form.as_bytes()).map(|(name, value)| {
                    let value = if name == "signature" {
                        STANDARD.encode([0u8; 64])
                    } else {
                        value.into_owned()
                    };
                    (name.into_owned(), value)
                }),
            )
            .finish();
        requests.push(
            format!(
                "{}{bad_signature}",
                head("POST", DEPOSIT_PATH, &address, bad_signature.len())
            )
            .into_bytes(),
        );
        let mut bad_utf8 = head("POST", DEPOSIT_PATH, &address, 1).into_bytes();
        bad_utf8.push(0xff);
        requests.push(bad_utf8);
        for request in requests {
            let response = send(&address, &request, false).await;
            assert!(response.starts_with("HTTP/1.1 400 Bad Request"));
            assert!(!waiting.is_finished());
        }
        assert!(
            send(&address, valid.as_bytes(), false)
                .await
                .starts_with("HTTP/1.1 200 OK")
        );
        assert_eq!(
            waiting.await.unwrap().unwrap().account_id(),
            "synthetic.near"
        );
    }

    #[tokio::test]
    async fn a_slow_connection_cannot_block_the_next_valid_deposit() {
        let (browser, owner) = bind().await.unwrap();
        let address = authority(&browser);
        let mut slow = TcpStream::connect(&address).await.unwrap();
        slow.write_all(b"POST ").await.unwrap();
        let waiting = tokio::spawn(owner.wait());
        let form = signed_form(&browser).await;
        let response = send(
            &address,
            format!("{}{form}", head("POST", DEPOSIT_PATH, &address, form.len())).as_bytes(),
            false,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(
            waiting.await.unwrap().unwrap().account_id(),
            "synthetic.near"
        );
        let mut bytes = Vec::new();
        timeout(TEST_TIMEOUT, slow.read_to_end(&mut bytes))
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn premature_body_eof_is_refused_without_consuming_the_challenge() {
        let (browser, owner) = bind().await.unwrap();
        let address = authority(&browser);
        let waiting = tokio::spawn(owner.wait());
        let mut stream = TcpStream::connect(&address).await.unwrap();
        stream
            .write_all(format!("{}a", head("POST", DEPOSIT_PATH, &address, 2)).as_bytes())
            .await
            .unwrap();
        stream.shutdown().await.unwrap();
        let mut response = Vec::new();
        timeout(TEST_TIMEOUT, stream.read_to_end(&mut response))
            .await
            .unwrap()
            .unwrap();
        assert!(response.starts_with(b"HTTP/1.1 400 Bad Request"));
        assert!(!waiting.is_finished());
        let form = signed_form(&browser).await;
        let response = send(
            &address,
            format!("{}{form}", head("POST", DEPOSIT_PATH, &address, form.len())).as_bytes(),
            false,
        )
        .await;
        assert!(response.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(
            waiting.await.unwrap().unwrap().account_id(),
            "synthetic.near"
        );
    }

    #[tokio::test]
    async fn the_lifetime_deadline_interrupts_an_incomplete_connection() {
        let (browser, mut owner) = bind().await.unwrap();
        let address = authority(&browser);
        let mut slow = TcpStream::connect(&address).await.unwrap();
        slow.write_all(b"GET ").await.unwrap();
        owner.deadline = Instant::now() + Duration::from_millis(50);
        assert!(matches!(
            timeout(TEST_TIMEOUT, owner.wait()).await.unwrap(),
            Err(NearWalletError::Expired)
        ));
        assert!(TcpStream::connect(&address).await.is_err());
    }

    #[tokio::test]
    async fn dropping_wait_and_state_bound_wallet_cancellation_close_the_listener() {
        let (browser, owner) = bind().await.unwrap();
        let address = authority(&browser);
        let waiting = tokio::spawn(owner.wait());
        let get = format!("GET {CALLBACK_PATH} HTTP/1.1\r\nHost: {address}\r\n\r\n");
        send(&address, get.as_bytes(), false).await;
        waiting.abort();
        assert!(matches!(waiting.await, Err(error) if error.is_cancelled()));
        assert!(TcpStream::connect(&address).await.is_err());

        let (browser, owner) = bind().await.unwrap();
        let address = authority(&browser);
        let url = Url::parse(&browser).unwrap();
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("state", url.fragment().unwrap())
            .append_pair("error", "user-cancelled")
            .finish();
        let waiting = tokio::spawn(owner.wait());
        assert!(
            send(
                &address,
                format!("{}{form}", head("POST", DEPOSIT_PATH, &address, form.len())).as_bytes(),
                false
            )
            .await
            .starts_with("HTTP/1.1 200")
        );
        assert!(matches!(
            waiting.await.unwrap(),
            Err(NearWalletError::Cancelled)
        ));
        assert!(TcpStream::connect(&address).await.is_err());
    }
}
