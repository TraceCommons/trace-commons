//! OS-level control: closing a listener must leave accepted streams usable
//! without accepting another connection. Run alongside the embedded SSE case
//! on Windows so a transport failure is distinct from a supervisor/barrier bug.

#[test]
fn a_closed_standard_listener_refuses_connections_while_an_accepted_stream_lives() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let mut client = std::net::TcpStream::connect(address).unwrap();
    let (mut accepted, _) = listener.accept().unwrap();
    drop(listener);
    let late = std::net::TcpStream::connect_timeout(&address, std::time::Duration::from_secs(2));
    if let Ok(stream) = &late {
        eprintln!(
            "standard listener: late connect succeeded; peer_known={}",
            stream.peer_addr().is_ok()
        );
    }
    assert!(
        late.is_err(),
        "the standard listener still completed a new handshake"
    );
    client.write_all(b"draining").unwrap();
    let mut body = [0; 8];
    accepted.read_exact(&mut body).unwrap();
    assert_eq!(&body, b"draining");
}

#[tokio::test]
async fn a_closed_tokio_listener_refuses_connections_while_an_accepted_stream_lives() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (client, accepted) =
        tokio::join!(tokio::net::TcpStream::connect(address), listener.accept());
    let mut client = client.unwrap();
    let (mut accepted, _) = accepted.unwrap();
    drop(listener);
    // Windows may take longer than this bound to report connection refusal.
    // Elapsed means no handshake completed, not that a listener accepted it.
    let late = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::TcpStream::connect(address),
    )
    .await;
    if let Ok(Ok(stream)) = &late {
        eprintln!(
            "Tokio listener: late connect succeeded; peer_known={}",
            stream.peer_addr().is_ok()
        );
    }
    assert!(
        !matches!(late, Ok(Ok(_))),
        "the Tokio listener still completed a new handshake"
    );
    client.write_all(b"draining").await.unwrap();
    let mut body = [0; 8];
    accepted.read_exact(&mut body).await.unwrap();
    assert_eq!(&body, b"draining");
}
