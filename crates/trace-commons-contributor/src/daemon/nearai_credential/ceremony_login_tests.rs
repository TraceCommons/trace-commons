//! A sign-in remembers which login its session came from only once that
//! session has been stored.

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// An OAuth leg for `provider` on a fresh listener, and its address.
async fn oauth_leg(store: &ConfigStore, provider: &'static str) -> (BrowserSignIn, u16) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let sign_in = BrowserSignIn {
        dir: store.dir().to_path_buf(),
        attempt_id: "attempt".into(),
        leg: Leg::OAuth {
            listener: listener.into_std().unwrap(),
            state: "state".into(),
            provider,
        },
    };
    (sign_in, port)
}

async fn deposit(port: u16) {
    let body = "state=state&ua=Mozilla/5.0%20Test&token=jwt&refresh_token=rt_x";
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    stream
        .write_all(
            format!(
                "POST {} HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{body}",
                loopback::DEPOSIT_PATH,
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
}

#[tokio::test]
async fn a_stored_session_remembers_its_login_and_a_failed_one_does_not() {
    let login = *crate::channel::NEAR_AI_SIGN_IN_PROVIDERS
        .iter()
        .find(|tag| **tag != "near")
        .unwrap();
    let api = CloudApi::live().unwrap();

    let (_dir, store) = crate::config::tests_support::temp_store();
    let (sign_in, port) = oauth_leg(&store, login).await;
    let finishing = tokio::spawn(async move {
        sign_in
            .finish(&api, |_, _, _| async { Err(anyhow!("storage failed")) })
            .await
    });
    deposit(port).await;
    assert!(finishing.await.unwrap().is_err());
    assert_eq!(super::super::login_record::stored_login(&store), None);

    let api = CloudApi::live().unwrap();
    let (sign_in, port) = oauth_leg(&store, login).await;
    let finishing =
        tokio::spawn(async move { sign_in.finish(&api, |_, _, _| async { Ok(()) }).await });
    deposit(port).await;
    finishing.await.unwrap().unwrap();
    assert_eq!(
        super::super::login_record::stored_login(&store),
        Some(login)
    );
    // Storing a session is not the Commons accepting it.
    assert_eq!(super::super::login_record::commons_login(&store), None);
}
