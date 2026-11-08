//! The login a sign-in uses: picked on the chooser page, carried by the
//! attempt's own `Progress` into `settle`, stored with the session, and named
//! by the failure line.

use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn send(port: u16, raw: String) -> String {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    stream.write_all(raw.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

fn minted(key_id: &str) -> super::super::api::MintedKey {
    super::super::api::MintedKey {
        key: "sk-synthetic".into(),
        key_id: key_id.into(),
        key_prefix: "sk-syn".into(),
        organization_id: "org-1".into(),
        workspace_id: "ws-1".into(),
    }
}

/// A browser picks GitHub on the page and completes its OAuth leg; the
/// sign-in's `finish` hands the session on, and the attempt's own progress
/// names the picked login -- what `settle` stores and the failure line logs.
#[tokio::test]
async fn a_real_pick_reaches_the_attempts_progress() {
    use super::super::chooser::{CHOOSE_PATH, PAGE_PATH};
    use super::super::loopback::CALLBACK_PATH;
    let picked = *crate::channel::NEAR_AI_SIGN_IN_PROVIDERS
        .iter()
        .find(|tag| **tag != "near")
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let authority = format!("127.0.0.1:{port}");
    let state = loopback::random_state().unwrap();
    let sign_in = BrowserSignIn::Choose {
        listener: listener.into_std().unwrap(),
        state: state.clone(),
        account_login: None,
        offered: crate::channel::NEAR_AI_SIGN_IN_PROVIDERS,
    };
    let token_server = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let token_address = token_server.local_addr().unwrap();
    let api = CloudApi::for_test(&format!("http://{token_address}")).unwrap();
    let token_task = tokio::spawn(async move {
        let router = axum::Router::new().route("/v1/auth/native/token", axum::routing::post(
            |headers: axum::http::HeaderMap, axum::Json(body): axum::Json<serde_json::Value>| async move {
                assert!(!headers.contains_key("authorization"));
                assert_eq!(headers["user-agent"], super::super::api::USER_AGENT);
                assert_eq!(body["code"], "synthetic-code");
                assert_eq!(body["client_id"], "trace-commons");
                assert_eq!(body["code_verifier"].as_str().unwrap().len(), 43);
                axum::Json(serde_json::json!({"access_token":"jwt", "refresh_token":"rt_x",
                    "refresh_token_expiration": (Utc::now() + chrono::Duration::days(7)).to_rfc3339()}))
            }));
        axum::serve(token_server, router).await.unwrap();
    });
    let progress = Progress::new();
    assert_eq!(failure_provider(&progress), "not_chosen");
    let handed_on = std::sync::Mutex::new(None);

    let finishing = sign_in.finish(&api, &progress, |session, _, user_agent| {
        *handed_on.lock().unwrap() = Some((session.refresh_token, user_agent));
        async { Ok(()) }
    });
    let browser = async {
        let page = send(
            port,
            format!("GET {PAGE_PATH}?state={state} HTTP/1.1\r\nHost: {authority}\r\n\r\n"),
        )
        .await;
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        let choice = format!("state={state}&provider={picked}");
        let redirect = send(
            port,
            format!(
                "POST {CHOOSE_PATH} HTTP/1.1\r\nHost: {authority}\r\nOrigin: http://{authority}\r\n\
                 Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{choice}",
                choice.len()
            ),
        )
        .await;
        assert!(redirect.starts_with("HTTP/1.1 303"), "{redirect}");
        let location = redirect
            .lines()
            .find_map(|line| line.strip_prefix("Location: "))
            .unwrap();
        let url = reqwest::Url::parse(location).unwrap();
        let oauth_state = url
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1
            .into_owned();
        let done = send(port, format!("GET {CALLBACK_PATH}?code=synthetic-code&state={oauth_state} HTTP/1.1\r\nHost: {authority}\r\n\r\n")).await;
        assert!(done.starts_with("HTTP/1.1 200"), "{done}");
    };
    let (finished, ()) = tokio::join!(finishing, browser);
    finished.unwrap();
    assert_eq!(
        handed_on.into_inner().unwrap(),
        Some(("rt_x".to_owned(), super::super::api::USER_AGENT.to_owned()))
    );
    token_task.abort();
    assert_eq!(progress.provider(), Some(picked));
    assert_eq!(failure_provider(&progress), picked);
}

/// The login is stored with the session, in its commit, bound to the key
/// minted with it; a persist that did not commit leaves the record as it
/// was, and a sign-in whose login is not known clears it.
#[test]
fn the_login_is_stored_with_the_session_it_came_from() {
    use super::super::login_record::stored_login;
    let login = crate::channel::NEAR_AI_SIGN_IN_PROVIDERS[0];
    let (_dir, store) = crate::config::tests_support::temp_store();

    mark_waiting_for_test(store.dir(), "first");
    persist_attempt(
        store.dir(),
        "first",
        minted("key-1"),
        "rt_first".into(),
        None,
        "Mozilla/5.0 Test".into(),
        Some(login),
    )
    .unwrap();
    assert_eq!(stored_login(&store, "key-1"), Some(login));

    // Not the waiting attempt: nothing commits, so nothing is recorded.
    assert!(
        persist_attempt(
            store.dir(),
            "stale",
            minted("key-2"),
            "rt_second".into(),
            None,
            "Mozilla/5.0 Test".into(),
            Some(login),
        )
        .is_err()
    );
    assert_eq!(stored_login(&store, "key-1"), Some(login));
    assert_eq!(stored_login(&store, "key-2"), None);

    // A session from a login this attempt never named: no tag survives.
    persist_attempt(
        store.dir(),
        "first",
        minted("key-3"),
        "rt_third".into(),
        None,
        "Mozilla/5.0 Test".into(),
        None,
    )
    .unwrap();
    assert_eq!(stored_login(&store, "key-1"), None);
    assert_eq!(stored_login(&store, "key-3"), None);

    // Signing out of NEAR AI forgets it too.
    mark_waiting_for_test(store.dir(), "fourth");
    persist_attempt(
        store.dir(),
        "fourth",
        minted("key-4"),
        "rt_fourth".into(),
        None,
        "Mozilla/5.0 Test".into(),
        Some(login),
    )
    .unwrap();
    forget(&store).unwrap();
    assert_eq!(stored_login(&store, "key-4"), None);
}
