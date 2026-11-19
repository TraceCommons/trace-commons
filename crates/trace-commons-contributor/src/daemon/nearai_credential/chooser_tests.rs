use super::*;
use crate::private_inference_copy::{
    CREDENTIAL_PROVIDER_GITHUB, CREDENTIAL_PROVIDER_GOOGLE, CREDENTIAL_PROVIDER_NEAR,
    CREDENTIAL_WALLET_NOTICE, NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN, NEAR_AI_SIGN_IN_PAGE_HEADING,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const STATE: &str = "s7aTe_-0123456789abcdefghijklmnopqrstuvwxyzA";
const NONCE: &str = "n0nce";

/// The `value`s of the page's buttons, in page order.
fn buttons(page: &str) -> Vec<&str> {
    page.split("name=\"provider\" value=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap())
        .collect()
}

#[test]
fn the_page_lists_exactly_the_offered_logins_in_order() {
    let page = render(&["near", "github", "google"], None, STATE, NONCE).unwrap();
    assert_eq!(buttons(&page), ["near", "github", "google"]);
    for title in [
        CREDENTIAL_PROVIDER_NEAR,
        CREDENTIAL_PROVIDER_GITHUB,
        CREDENTIAL_PROVIDER_GOOGLE,
    ] {
        assert!(page.contains(&format!(">{title}</button>")), "{title}");
    }
    assert!(page.contains(&format!("<title>{NEAR_AI_SIGN_IN_PAGE_HEADING}</title>")));
    assert!(page.contains(&format!(
        "<h1 id=\"heading\">{NEAR_AI_SIGN_IN_PAGE_HEADING}</h1>"
    )));
    // The wallet notice sits directly under the wallet button.
    let wallet = page.find(">NEAR wallet</button>").unwrap();
    let notice = page.find(CREDENTIAL_WALLET_NOTICE).unwrap();
    let github = page.find(">GitHub</button>").unwrap();
    assert!(wallet < notice && notice < github);
    // The form posts to this listener, bound to the attempt.
    assert!(page.contains(&format!("action=\"{CHOOSE_PATH}\"")));
    assert!(page.contains(&format!("name=\"state\" value=\"{STATE}\"")));

    // Order is the build's, not the page's.
    let page = render(&["google", "near"], None, STATE, NONCE).unwrap();
    assert_eq!(buttons(&page), ["google", "near"]);
}

#[test]
fn with_nothing_remembered_the_page_marks_no_login() {
    let page = render(&["near", "github", "google"], None, STATE, NONCE).unwrap();
    assert!(!page.contains(NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN));
    // Identical to a remembered login this build does not offer.
    assert_eq!(
        page,
        render(
            &["near", "github", "google"],
            Some("bitbucket"),
            STATE,
            NONCE
        )
        .unwrap()
    );
}

#[test]
fn the_accounts_login_comes_first_and_is_marked() {
    // Owner-approved wording, 2026-11-02.
    assert_eq!(
        NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN,
        "You used this login last time"
    );
    let page = render(&["near", "github", "google"], Some("github"), STATE, NONCE).unwrap();
    assert_eq!(buttons(&page), ["github", "near", "google"]);
    let github = page.find(">GitHub</button>").unwrap();
    let mark = page.find(NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN).unwrap();
    let wallet = page.find(">NEAR wallet</button>").unwrap();
    assert!(github < mark && mark < wallet, "{page}");
    // The people signing in do not know the Commons by name (owner,
    // 2026-11-02): the page never names it.
    assert!(!page.contains(crate::brand::COMMONS_NAME), "{page}");
    assert_eq!(page.matches(NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN).count(), 1);
    // The wallet keeps its notice wherever it is listed.
    assert!(page.find(CREDENTIAL_WALLET_NOTICE).unwrap() > wallet);

    // The wallet as the account's login: its notice, then the mark.
    let page = render(&["near", "github", "google"], Some("near"), STATE, NONCE).unwrap();
    assert_eq!(buttons(&page), ["near", "github", "google"]);
    let notice = page.find(CREDENTIAL_WALLET_NOTICE).unwrap();
    let mark = page.find(NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN).unwrap();
    let github = page.find(">GitHub</button>").unwrap();
    assert!(notice < mark && mark < github, "{page}");

    // Alone, it is still marked.
    let page = render(&["google"], Some("google"), STATE, NONCE).unwrap();
    assert!(page.contains(NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN));
}

/// Git on Windows checks the template out with CRLF line endings. The page
/// must fill exactly as from an LF checkout: every button drawn, no
/// placeholder left.
#[test]
fn a_crlf_template_fills_like_an_lf_one() {
    let lf = include_str!("choose.html");
    let crlf = lf.replace("\r\n", "\n").replace('\n', "\r\n");
    let offered = ["near", "github", "google"];
    let from_lf = fill(lf, &offered, Some("github"), STATE, NONCE).unwrap();
    let from_crlf = fill(&crlf, &offered, Some("github"), STATE, NONCE).unwrap();
    assert!(!from_crlf.contains("__"), "{from_crlf}");
    assert_eq!(buttons(&from_crlf), buttons(&from_lf));
    assert_eq!(buttons(&from_crlf), ["github", "near", "google"]);
}

/// A page restored from the back/forward cache must not keep refusing
/// submits it never sent.
#[test]
fn a_restored_page_can_choose_again() {
    let page = render(&["google"], None, STATE, NONCE).unwrap();
    assert!(page.contains("window.addEventListener(\"pageshow\", function () { sent = false; });"));
}

#[test]
fn a_build_without_the_wallet_shows_no_wallet_button_or_notice() {
    let page = render(&["google"], None, STATE, NONCE).unwrap();
    assert_eq!(buttons(&page), ["google"]);
    assert!(!page.contains(CREDENTIAL_WALLET_NOTICE));
    assert!(!page.contains(CREDENTIAL_PROVIDER_NEAR));
}

#[test]
fn the_page_is_the_builds_list() {
    let page = render(
        crate::channel::NEAR_AI_SIGN_IN_PROVIDERS,
        None,
        STATE,
        NONCE,
    )
    .unwrap();
    assert_eq!(buttons(&page), crate::channel::NEAR_AI_SIGN_IN_PROVIDERS);
}

#[test]
fn every_offered_login_has_a_title() {
    for tag in crate::channel::NEAR_AI_SIGN_IN_PROVIDERS {
        assert!(
            crate::private_inference_copy::credential_provider_title(tag).is_some(),
            "{tag}"
        );
    }
}

#[test]
fn the_page_fetches_nothing_and_quotes_nothing_it_was_given() {
    let page = render(&["near", "github", "google"], None, STATE, NONCE).unwrap();
    assert!(!page.contains("http://"), "{page}");
    assert!(!page.contains("https://"), "{page}");
    assert!(!page.contains("__"), "an unfilled placeholder: {page}");
    for hostile in ["\"><script>", "a'b", "a b", "", &"a".repeat(65)] {
        assert!(
            render(&["google"], None, hostile, NONCE).is_err(),
            "{hostile}"
        );
        assert!(
            render(&["google"], None, STATE, hostile).is_err(),
            "{hostile}"
        );
    }
    // A login with no title, or no logins at all, is not drawn.
    assert!(render(&["google", "bitbucket"], None, STATE, NONCE).is_err());
    assert!(render(&["Google"], None, STATE, NONCE).is_err());
    assert!(render(&[], None, STATE, NONCE).is_err());
}

/// Send one raw request and return the whole response.
async fn send(address: std::net::SocketAddr, raw: String) -> String {
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(raw.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

fn status(response: &str) -> &str {
    response.lines().next().unwrap_or_default()
}

fn get(authority: &str, target: &str) -> String {
    format!("GET {target} HTTP/1.1\r\nHost: {authority}\r\n\r\n")
}

fn post(authority: &str, origin: Option<&str>, extra: &str, body: &str) -> String {
    let origin = origin
        .map(|origin| format!("Origin: {origin}\r\n"))
        .unwrap_or_default();
    format!(
        "POST {CHOOSE_PATH} HTTP/1.1\r\nHost: {authority}\r\n{origin}{extra}\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Content-Length: {}\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn only_this_attempts_same_origin_choice_of_an_offered_login_is_taken() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let authority = format!("127.0.0.1:{}", address.port());
    let own = format!("http://{authority}");
    let state = crate::daemon::nearai_credential::loopback::random_state().unwrap();
    let server = tokio::spawn({
        let state = state.clone();
        async move {
            receive_choice(&listener, &state, &["google", "github"], None)
                .await
                .map(|chosen| chosen.provider)
        }
    });

    // The page, only at this attempt's address and this listener's name.
    let page = send(
        address,
        get(&authority, &format!("{PAGE_PATH}?state={state}")),
    )
    .await;
    assert_eq!(status(&page), "HTTP/1.1 200 OK", "{page}");
    assert!(page.contains("X-Frame-Options: DENY"), "{page}");
    assert_eq!(buttons(&page), ["google", "github"]);
    for target in [
        format!("{PAGE_PATH}?state=other"),
        PAGE_PATH.to_string(),
        "/favicon.ico".to_string(),
    ] {
        let response = send(address, get(&authority, &target)).await;
        assert_eq!(status(&response), "HTTP/1.1 404 Not Found", "{target}");
    }
    // A DNS-rebound name pointing at this port is not this listener.
    let rebound = send(
        address,
        get("evil.example", &format!("{PAGE_PATH}?state={state}")),
    )
    .await;
    assert_eq!(status(&rebound), "HTTP/1.1 400 Bad Request");

    let refused = [
        // No state, another attempt's, or the state twice.
        post(&authority, Some(&own), "", "provider=github"),
        post(&authority, Some(&own), "", "state=other&provider=github"),
        post(
            &authority,
            Some(&own),
            "",
            &format!("state={state}&state={state}&provider=github"),
        ),
        // A login this build does not offer, none, or two.
        post(
            &authority,
            Some(&own),
            "",
            &format!("state={state}&provider=near"),
        ),
        post(
            &authority,
            Some(&own),
            "",
            &format!("state={state}&provider=bitbucket"),
        ),
        post(&authority, Some(&own), "", &format!("state={state}")),
        post(
            &authority,
            Some(&own),
            "",
            &format!("state={state}&provider=github&provider=google"),
        ),
        // Anything else in the body, including a place to go next, and
        // including one hidden behind a `#`.
        post(
            &authority,
            Some(&own),
            "",
            &format!("state={state}&provider=github&next=https://evil.example"),
        ),
        post(
            &authority,
            Some(&own),
            "",
            &format!("state={state}&provider=github#next=https://evil.example"),
        ),
        post(
            &authority,
            Some(&own),
            "",
            &format!("state={state}#&provider=github"),
        ),
        // Another site's form, a sandboxed one, or none at all.
        post(
            &authority,
            Some("https://evil.example"),
            "",
            &format!("state={state}&provider=github"),
        ),
        post(
            &authority,
            Some("null"),
            "",
            &format!("state={state}&provider=github"),
        ),
        post(
            &authority,
            None,
            "",
            &format!("state={state}&provider=github"),
        ),
        post(
            &authority,
            Some(&own),
            "Sec-Fetch-Site: cross-site\r\n",
            &format!("state={state}&provider=github"),
        ),
        post(
            "evil.example",
            Some("http://evil.example"),
            "",
            &format!("state={state}&provider=github"),
        ),
    ];
    for request in refused {
        let response = send(address, request.clone()).await;
        assert_eq!(status(&response), "HTTP/1.1 400 Bad Request", "{request}");
    }
    assert!(!server.is_finished(), "a refused choice ended the chooser");

    // This page's own choice is taken. The connection stays open for the
    // caller's redirect, so it is not read to the end here.
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream
        .write_all(
            post(
                &authority,
                Some(&own),
                "Sec-Fetch-Site: same-origin\r\n",
                &format!("state={state}&provider=github"),
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(server.await.unwrap().unwrap(), "github");
}

/// Safari 27.0.1 marks a person's click on the page's own button
/// `Sec-Fetch-Site: none`, not `same-origin` (a scripted click gets
/// `same-origin`). The request below has the headers Safari sent on a real
/// click, captured 2026-11-11. That click is taken; `none` with any other
/// origin, or with none, is still refused.
#[tokio::test]
async fn safaris_click_marked_fetch_site_none_is_taken() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let authority = format!("127.0.0.1:{}", address.port());
    let own = format!("http://{authority}");
    let state = crate::daemon::nearai_credential::loopback::random_state().unwrap();
    let server = tokio::spawn({
        let state = state.clone();
        async move {
            receive_choice(&listener, &state, &["google", "github"], None)
                .await
                .map(|chosen| chosen.provider)
        }
    });

    for origin in [Some("https://evil.example"), Some("null"), None] {
        let request = post(
            &authority,
            origin,
            "Sec-Fetch-Site: none\r\n",
            &format!("state={state}&provider=google"),
        );
        let response = send(address, request.clone()).await;
        assert_eq!(status(&response), "HTTP/1.1 400 Bad Request", "{request}");
    }
    assert!(!server.is_finished(), "a refused choice ended the chooser");

    let body = format!("state={state}&provider=google");
    let safari = format!(
        "POST {CHOOSE_PATH} HTTP/1.1\r\n\
         Host: {authority}\r\n\
         Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8\r\n\
         Content-Type: application/x-www-form-urlencoded\r\n\
         Origin: {own}\r\n\
         Sec-Fetch-Site: none\r\n\
         Upgrade-Insecure-Requests: 1\r\n\
         Sec-Fetch-Mode: navigate\r\n\
         User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 \
         (KHTML, like Gecko) Version/27.0.1 Safari/605.1.15\r\n\
         Referer: {own}{PAGE_PATH}?state={state}\r\n\
         Sec-Fetch-Dest: document\r\n\
         Content-Length: {}\r\n\
         Accept-Language: en-US,en;q=0.9\r\n\
         Priority: u=0, i\r\n\
         Accept-Encoding: gzip, deflate\r\n\
         Connection: keep-alive\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(address).await.unwrap();
    stream.write_all(safari.as_bytes()).await.unwrap();
    // A refusal answers and keeps waiting, so a refused click shows up here
    // as the read ending with a 400 rather than the chooser returning.
    let mut response = Vec::new();
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        stream.read_to_end(&mut response),
    )
    .await;
    let response = String::from_utf8_lossy(&response);
    assert!(
        !response.starts_with("HTTP/1.1 400"),
        "Safari's click was refused: {response}"
    );
    assert_eq!(server.await.unwrap().unwrap(), "google");
}

/// Drive a ceremony that named no login through its page to `provider`, and
/// return the browser URL `begin` handed out, the redirect, and the port.
#[cfg(not(feature = "near-ai-staging"))]
async fn choose_in_browser(
    store: &crate::config::ConfigStore,
    provider: &str,
) -> (reqwest::Url, String, u16, String) {
    let started = crate::daemon::nearai_credential::ceremony::begin_in_browser(store)
        .await
        .expect("a ceremony starts");
    let url = reqwest::Url::parse(started["browser_url"].as_str().unwrap()).unwrap();
    let port = url.port().unwrap();
    let address: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
    let authority = format!("127.0.0.1:{port}");
    let page = send(
        address,
        get(
            &authority,
            &format!("{}?{}", url.path(), url.query().unwrap()),
        ),
    )
    .await;
    assert_eq!(status(&page), "HTTP/1.1 200 OK", "{page}");
    let state = url
        .query_pairs()
        .find(|(name, _)| name == "state")
        .unwrap()
        .1
        .into_owned();
    let response = send(
        address,
        post(
            &authority,
            Some(&format!("http://{authority}")),
            "",
            &format!("state={state}&provider={provider}"),
        ),
    )
    .await;
    assert_eq!(status(&response), "HTTP/1.1 303 See Other", "{response}");
    assert!(
        response.contains("Referrer-Policy: no-referrer"),
        "{response}"
    );
    let location = response
        .lines()
        .find_map(|line| line.strip_prefix("Location: "))
        .expect("a redirect")
        .to_string();
    (
        url,
        location,
        port,
        started["attempt_id"].as_str().unwrap().into(),
    )
}

#[cfg(not(feature = "near-ai-staging"))]
#[tokio::test]
async fn a_sign_in_naming_no_login_opens_the_page_on_its_own_listener() {
    let (_directory, store) = crate::config::tests_support::temp_store();
    let started = crate::daemon::nearai_credential::ceremony::begin_in_browser(&store)
        .await
        .unwrap();
    let url = reqwest::Url::parse(started["browser_url"].as_str().unwrap()).unwrap();
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert_eq!(url.path(), PAGE_PATH);
    let id = started["attempt_id"].as_str().unwrap();
    // The attempt id is the shell's handle, and is not put in the browser.
    assert!(!url.as_str().contains(id));
    crate::daemon::nearai_credential::ceremony::cancel(store.dir(), Some(id)).unwrap();
}

#[cfg(not(feature = "near-ai-staging"))]
#[tokio::test]
async fn choosing_github_redirects_to_its_login_returning_to_the_same_port() {
    let (_directory, store) = crate::config::tests_support::temp_store();
    let (_url, location, port, id) = choose_in_browser(&store, "github").await;
    let url = reqwest::Url::parse(&location).unwrap();
    assert_eq!(url.path(), "/v1/auth/github");
    let query: std::collections::HashMap<_, _> = url.query_pairs().collect();
    assert_eq!(
        query["frontend_callback"],
        format!(
            "http://127.0.0.1:{port}{}",
            crate::daemon::nearai_credential::loopback::CALLBACK_PATH
        )
    );
    assert_eq!(query["client_id"], "trace-commons");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["code_challenge"].len(), 43);
    crate::daemon::nearai_credential::ceremony::cancel(store.dir(), Some(&id)).unwrap();
}

#[cfg(not(feature = "near-ai-staging"))]
#[tokio::test]
async fn choosing_the_near_wallet_enters_the_wallet_page_on_the_same_port() {
    use crate::daemon::nearai_credential::{near_wallet, near_wallet_page};
    let (_directory, store) = crate::config::tests_support::temp_store();
    let (_url, location, port, id) = choose_in_browser(&store, "near").await;
    let location = reqwest::Url::parse(&location).unwrap();
    assert_eq!(location.host_str(), Some("127.0.0.1"));
    assert_eq!(location.port(), Some(port));
    assert_eq!(location.path(), near_wallet::CALLBACK_PATH);
    // The browser follows it: the wallet page is served on this listener.
    let authority = format!("127.0.0.1:{port}");
    let page = send(
        ([127, 0, 0, 1], port).into(),
        get(&authority, near_wallet::CALLBACK_PATH),
    )
    .await;
    assert!(page.starts_with("HTTP/1.1 200"), "{page}");
    assert!(page.contains(near_wallet_page::ASSET_PREFIX), "{page}");
    crate::daemon::nearai_credential::ceremony::cancel(store.dir(), Some(&id)).unwrap();
}

/// One login: nothing to choose, so the browser goes straight to it.
#[cfg(feature = "near-ai-staging")]
#[tokio::test]
async fn a_single_login_build_skips_the_page() {
    let (_directory, store) = crate::config::tests_support::temp_store();
    let started = crate::daemon::nearai_credential::ceremony::begin_in_browser(&store)
        .await
        .unwrap();
    let url = reqwest::Url::parse(started["browser_url"].as_str().unwrap()).unwrap();
    assert_ne!(url.path(), PAGE_PATH);
    assert_eq!(url.path(), "/v1/auth/google");
    let id = started["attempt_id"].as_str().unwrap();
    crate::daemon::nearai_credential::ceremony::cancel(store.dir(), Some(id)).unwrap();
}

/// A sign-in on a device whose Trace Commons account is known to use a
/// login opens a page that lists that login first and marks it.
#[cfg(not(feature = "near-ai-staging"))]
#[tokio::test]
async fn the_page_marks_the_login_the_commons_last_accepted() {
    let (_directory, store) = crate::config::tests_support::temp_store();
    // An enrolled device: the account's login is kept only on one.
    std::fs::write(store.dir().join(crate::config::CONFIG_FILE), b"{}").unwrap();
    crate::daemon::nearai_credential::login_record::accepted_by_commons(&store, Some("google"));
    let started = crate::daemon::nearai_credential::ceremony::begin_in_browser(&store)
        .await
        .unwrap();
    let url = reqwest::Url::parse(started["browser_url"].as_str().unwrap()).unwrap();
    let port = url.port().unwrap();
    let authority = format!("127.0.0.1:{port}");
    let page = send(
        ([127, 0, 0, 1], port).into(),
        get(
            &authority,
            &format!("{}?{}", url.path(), url.query().unwrap()),
        ),
    )
    .await;
    assert_eq!(buttons(&page), ["google", "near", "github"]);
    assert!(page.contains(NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN));
    let id = started["attempt_id"].as_str().unwrap();
    crate::daemon::nearai_credential::ceremony::cancel(store.dir(), Some(id)).unwrap();
}

/// OAuth state is independent from the chooser state, and bearer-token
/// callbacks cannot downgrade the code exchange.
#[cfg(not(feature = "near-ai-staging"))]
#[tokio::test]
async fn the_oauth_state_is_independent_and_token_callbacks_are_refused() {
    use crate::daemon::nearai_credential::loopback::CALLBACK_PATH;
    let (_directory, store) = crate::config::tests_support::temp_store();
    let (chooser_url, location, port, id) = choose_in_browser(&store, "github").await;
    let chooser_state = chooser_url
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned();
    let authorization = reqwest::Url::parse(&location).unwrap();
    let state = authorization
        .query_pairs()
        .find(|(key, _)| key == "state")
        .unwrap()
        .1
        .into_owned();
    assert_ne!(state, chooser_state);
    let authority = format!("127.0.0.1:{port}");
    let address = format!("127.0.0.1:{port}").parse().unwrap();
    for query in [
        format!("code=synthetic&state={chooser_state}"),
        format!("token=jwt&refresh_token=rt&state={state}"),
    ] {
        let refused = send(
            address,
            format!("GET {CALLBACK_PATH}?{query} HTTP/1.1\r\nHost: {authority}\r\n\r\n"),
        )
        .await;
        assert_eq!(status(&refused), "HTTP/1.1 400 Bad Request");
    }
    crate::daemon::nearai_credential::ceremony::cancel(store.dir(), Some(&id)).unwrap();
}

/// A redirect that could not be written reports it, so the attempt fails
/// at once instead of waiting out its timeout with nothing to retry.
#[tokio::test]
async fn a_redirect_that_was_not_written_says_so() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = TcpStream::connect(address).await.unwrap();
    let (mut accepted, _) = listener.accept().await.unwrap();
    drop(client);
    // Our own write side closed: the write cannot happen.
    accepted.shutdown().await.unwrap();
    assert!(
        !Chosen::for_test("github", accepted)
            .redirect("http://127.0.0.1/x")
            .await
    );

    let mut client = TcpStream::connect(address).await.unwrap();
    let (accepted, _) = listener.accept().await.unwrap();
    assert!(
        Chosen::for_test("github", accepted)
            .redirect("http://127.0.0.1/x")
            .await
    );
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.unwrap();
    assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 303 See Other"));
}

/// The attempt's status word, waited on until it leaves `waiting_for_browser`
/// or five seconds pass.
#[cfg(not(feature = "near-ai-staging"))]
async fn settled_status(store: &crate::config::ConfigStore, id: &str) -> &'static str {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let status = crate::daemon::nearai_credential::ceremony::status(store.dir(), Some(id))
            .map(|status| status.status)
            .unwrap_or("none");
        if status != "waiting_for_browser" || std::time::Instant::now() >= deadline {
            return status;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

/// Back to the login page after a pick, and a second pick: the listener has
/// moved on to the first login and cannot switch. It answers with the page's
/// own refusal and ends the attempt at once, so the app can start a new one,
/// on the OAuth leg and the wallet's alike. A post that is not this page's
/// own choice is refused and ends nothing.
#[cfg(not(feature = "near-ai-staging"))]
#[tokio::test]
async fn a_second_pick_after_the_first_ends_the_attempt_at_once() {
    for (first, second) in [("github", "google"), ("near", "github")] {
        let (_directory, store) = crate::config::tests_support::temp_store();
        let (url, _location, port, id) = choose_in_browser(&store, first).await;
        let state = url
            .query_pairs()
            .find(|(name, _)| name == "state")
            .unwrap()
            .1
            .into_owned();
        let address: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
        let authority = format!("127.0.0.1:{port}");
        let own = format!("http://{authority}");

        // Not this page's choice: refused as any unknown request is (404 on
        // the OAuth leg, 400 on the wallet's), and the sign-in goes on.
        let forged = send(
            address,
            post(
                &authority,
                Some(&own),
                "",
                &format!("state=other&provider={second}"),
            ),
        )
        .await;
        assert!(
            ["HTTP/1.1 404 Not Found", "HTTP/1.1 400 Bad Request"].contains(&status(&forged)),
            "{first}: {forged}"
        );
        assert!(
            !forged.contains(crate::private_inference_copy::NEAR_AI_SIGN_IN_ENDED),
            "{first}: {forged}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            crate::daemon::nearai_credential::ceremony::status(store.dir(), Some(&id))
                .unwrap()
                .status,
            "waiting_for_browser",
            "{first}"
        );

        // The page's own second choice: refused with its text, attempt over.
        let stale = send(
            address,
            post(
                &authority,
                Some(&own),
                "",
                &format!("state={state}&provider={second}"),
            ),
        )
        .await;
        assert_eq!(status(&stale), "HTTP/1.1 400 Bad Request", "{first}");
        assert!(
            stale.ends_with(crate::private_inference_copy::NEAR_AI_SIGN_IN_ENDED),
            "{first}: {stale}"
        );
        // The person's own doing, like a cancel: no failure notice in the app.
        assert_eq!(settled_status(&store, &id).await, "cancelled", "{first}");
    }
}

/// Back to the login page in a browser that did not keep it (it is
/// `no-store`): the page's own address is fetched again. That too ends the
/// attempt at once, with the page's refusal, on both legs; another address
/// ends nothing.
#[cfg(not(feature = "near-ai-staging"))]
#[tokio::test]
async fn the_page_fetched_again_after_a_pick_ends_the_attempt_at_once() {
    for first in ["github", "near"] {
        let (_directory, store) = crate::config::tests_support::temp_store();
        let (url, _location, port, id) = choose_in_browser(&store, first).await;
        let address: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
        let authority = format!("127.0.0.1:{port}");

        let other = send(
            address,
            get(&authority, &format!("{PAGE_PATH}?state=other")),
        )
        .await;
        assert!(status(&other).starts_with("HTTP/1.1 4"), "{first}: {other}");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            crate::daemon::nearai_credential::ceremony::status(store.dir(), Some(&id))
                .unwrap()
                .status,
            "waiting_for_browser",
            "{first}"
        );

        // A fetch the browser made ahead of the person is not them coming
        // back: refused, and the sign-in goes on.
        let page_target = format!("{}?{}", url.path(), url.query().unwrap());
        let prefetch = send(
            address,
            format!(
                "GET {page_target} HTTP/1.1\r\nHost: {authority}\r\nSec-Purpose: prefetch;prerender\r\n\r\n"
            ),
        )
        .await;
        assert!(
            status(&prefetch).starts_with("HTTP/1.1 4"),
            "{first}: {prefetch}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            crate::daemon::nearai_credential::ceremony::status(store.dir(), Some(&id))
                .unwrap()
                .status,
            "waiting_for_browser",
            "{first}"
        );

        let again = send(address, get(&authority, &page_target)).await;
        assert_eq!(status(&again), "HTTP/1.1 400 Bad Request", "{first}");
        assert!(
            again.ends_with(crate::private_inference_copy::NEAR_AI_SIGN_IN_ENDED),
            "{first}: {again}"
        );
        // The person's own doing, like a cancel: no failure notice in the app.
        assert_eq!(settled_status(&store, &id).await, "cancelled", "{first}");
    }
}

/// The ended-sign-in sentence is the owner's, word for word (2026-11-02),
/// and never names the Commons.
#[test]
fn the_ended_sign_in_sentence_is_the_approved_one() {
    assert_eq!(
        crate::private_inference_copy::NEAR_AI_SIGN_IN_ENDED,
        "This sign-in has ended. Start again from the app."
    );
    assert_eq!(
        STALE_CHOICE_BODY,
        crate::private_inference_copy::NEAR_AI_SIGN_IN_ENDED
    );
    assert!(!STALE_CHOICE_BODY.contains(crate::brand::COMMONS_NAME));
}

/// A second pick is the person's own doing, like a cancel: it ends the
/// attempt as `cancelled` and writes no failure line -- one rule for both.
#[test]
fn a_second_pick_is_not_logged_as_a_failure() {
    use crate::daemon::nearai_credential::ceremony::failure_fields;
    use crate::daemon::nearai_credential::near_wallet::NearWalletError;
    assert!(failure_fields(&anyhow!(STALE_CHOICE), "github").is_none());
    assert!(failure_fields(&anyhow::Error::from(NearWalletError::StaleChoice), "near").is_none());
    assert!(failure_fields(&anyhow!("near_ai_credential_unavailable"), "github").is_some());
    use crate::daemon::nearai_credential::ceremony::ended_by_person;
    assert!(ended_by_person(&anyhow!(STALE_CHOICE)));
    assert!(ended_by_person(&anyhow::Error::from(
        NearWalletError::StaleChoice
    )));
    assert!(ended_by_person(&anyhow::Error::from(
        NearWalletError::Cancelled
    )));
    assert!(!ended_by_person(&anyhow!("near_ai_credential_unavailable")));
}
