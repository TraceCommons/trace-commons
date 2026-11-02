use super::*;
use crate::private_inference_copy::{
    CREDENTIAL_PROVIDER_GITHUB, CREDENTIAL_PROVIDER_GOOGLE, CREDENTIAL_PROVIDER_NEAR,
    CREDENTIAL_WALLET_NOTICE, NEAR_AI_SIGN_IN_PAGE_ACCOUNT_LOGIN, NEAR_AI_SIGN_IN_PAGE_HEADING,
};

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
        // Anything else in the body, including a place to go next.
        post(
            &authority,
            Some(&own),
            "",
            &format!("state={state}&provider=github&next=https://evil.example"),
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
    let expected = crate::daemon::nearai_credential::api::CloudApi::live()
        .unwrap()
        .authorize_url("github", port)
        .unwrap();
    assert_eq!(location, expected);
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

/// Once picked, the attempt knows which login it is using, for the
/// Commons check to remember.
#[cfg(not(feature = "near-ai-staging"))]
#[tokio::test]
async fn a_pick_tells_the_attempt_which_login_it_uses() {
    let (_directory, store) = crate::config::tests_support::temp_store();
    let (_url, _location, _port, id) = choose_in_browser(&store, "github").await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while crate::daemon::nearai_credential::ceremony::attempt_provider(store.dir()).is_none()
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        crate::daemon::nearai_credential::ceremony::attempt_provider(store.dir()),
        Some("github")
    );
    crate::daemon::nearai_credential::ceremony::cancel(store.dir(), Some(&id)).unwrap();
}

/// A sign-in that names its login tells the attempt at once.
#[tokio::test]
async fn a_named_login_tells_the_attempt_which_login_it_uses() {
    let (_directory, store) = crate::config::tests_support::temp_store();
    let login = *crate::channel::NEAR_AI_SIGN_IN_PROVIDERS
        .iter()
        .find(|tag| **tag != "near")
        .unwrap();
    let started = crate::daemon::nearai_credential::ceremony::begin(&store, login)
        .await
        .unwrap();
    assert_eq!(
        crate::daemon::nearai_credential::ceremony::attempt_provider(store.dir()),
        Some(login)
    );
    let id = started["attempt_id"].as_str().unwrap();
    crate::daemon::nearai_credential::ceremony::cancel(store.dir(), Some(id)).unwrap();
}
