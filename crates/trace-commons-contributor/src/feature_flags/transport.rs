use super::*;
use anyhow::{Context, Result};
use std::time::Duration;

pub(super) fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?)
}
pub(super) async fn fetch(environment: Environment) -> Result<Vec<u8>> {
    let endpoint = crate::channel::feature_flags_url(environment)?;
    download(&client()?, endpoint).await
}
async fn download(client: &reqwest::Client, endpoint: &str) -> Result<Vec<u8>> {
    let mut response = client
        .get(endpoint)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .context("feature manifest request")?;
    anyhow::ensure!(
        response.status() == reqwest::StatusCode::OK,
        "feature manifest HTTP status"
    );
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim();
    anyhow::ensure!(
        mime == "application/json" || (mime.starts_with("application/") && mime.ends_with("+json")),
        "feature manifest content type"
    );
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= MAX_MANIFEST_BYTES as u64),
        "feature manifest too large"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            chunk.len() <= MAX_MANIFEST_BYTES.saturating_sub(bytes.len()),
            "feature manifest too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };
    async fn serve(response: Vec<u8>) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/flags", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let n = socket.read(&mut request).await.unwrap();
            request.truncate(n);
            let _ = socket.write_all(&response).await;
            request
        });
        (url, task)
    }
    #[tokio::test]
    async fn refuses_redirects_and_errors_without_credentials() {
        for status in [
            "302 Found\r\nLocation: http://127.0.0.1:1/other",
            "500 Error",
        ] {
            let(url,task)=serve(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").into_bytes()).await;
            assert!(download(&client().unwrap(), &url).await.is_err());
            let request = String::from_utf8(task.await.unwrap())
                .unwrap()
                .to_lowercase();
            assert!(!request.contains("authorization:"));
            assert!(!request.contains("cookie:"));
        }
    }
    #[tokio::test]
    async fn bounds_stream_without_content_length() {
        let mut response =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                .to_vec();
        response.extend(vec![b' '; MAX_MANIFEST_BYTES + 1]);
        let (url, task) = serve(response).await;
        assert!(download(&client().unwrap(), &url).await.is_err());
        task.await.unwrap();
    }
    #[tokio::test]
    async fn stalled_server_is_bounded_by_the_request_timeout() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/flags", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let started = std::time::Instant::now();
        let response = tokio::time::timeout(
            Duration::from_secs(7),
            download(&client().unwrap(), &endpoint),
        )
        .await;
        assert!(response.unwrap().is_err());
        assert!(started.elapsed() < Duration::from_secs(7));
        server.abort();
        let _ = server.await;
    }
    #[tokio::test]
    async fn accepts_json_and_refuses_wrong_mime() {
        for (mime, ok) in [("application/json", true), ("text/html", false)] {
            let(url,task)=serve(format!("HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").into_bytes()).await;
            assert_eq!(download(&client().unwrap(), &url).await.is_ok(), ok);
            task.await.unwrap();
        }
    }
}
