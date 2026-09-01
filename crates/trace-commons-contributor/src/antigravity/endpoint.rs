//! Finds the Antigravity IDE's local language server API.
//!
//! The language server is a subprocess of the IDE, started with a CSRF
//! token and an "extension server" port on its command line. Neither the
//! token nor the extension-server port is the API we want -- the API that
//! actually serves trajectory data listens on a *different* port nearby,
//! and there is no documented rule for exactly which one. So this module
//! reads the token and the extension-server port from the process table,
//! then sweeps a bounded window of nearby ports, requiring of each that it
//! demands the token (401 unauthenticated with no header) and then accepts
//! this exact candidate's token (200 with the header).
//!
//! The sweep is two passes. First `open_ports` asks, concurrently, which
//! ports in the window have anything listening; it opens a TCP connection
//! and closes it, carrying no credential. Then the 401/200 check above runs
//! sequentially, lowest port first, against only those. The split is there
//! because a closed port is not free on Windows -- see `PROBE_TIMEOUT` --
//! and because the concurrency that fixes this must not be applied to the
//! pass that sends the token; see `open_ports`.
//!
//! **What that check buys, and what it does not.** It rules out the
//! accident this sweep would otherwise cause: a port in the window that
//! merely answers, belonging to some unrelated local service, which would
//! be handed a live CSRF token on the strength of having a socket open. It
//! is NOT mutual authentication. The 401 half is trivially forged -- any
//! local listener can return a body containing `"unauthenticated"` -- and
//! it is sent BEFORE the token, so it gates nothing an adversary cannot
//! pass. The 200 half is the only half the token itself proves anything
//! about, and by then the token has already been sent.
//!
//! The residual: on a host where a second local user can bind a port in
//! this window before the real API does -- and where the process table is
//! readable enough to find the extension-server port, or hidden by
//! `hidepid` so the race cannot be lost noisily -- they can return a canned
//! 401 and harvest one live token. The API offers no mutual proof to close
//! this with, so it is stated rather than fixed. The exposure is bounded to
//! the local machine and to what that token can reach: the IDE's own
//! trajectory data.
//!
//! The token is a live credential for a local API. It must never appear in
//! an error, a log line, or a panic message -- every error here is a fixed,
//! content-free `&'static str`.

use std::time::Duration;

use anyhow::Result;
use sysinfo::System;

/// No Antigravity language server process was found at all.
pub(crate) const ERR_NOT_RUNNING: &str = "antigravity-not-running";
/// Candidate processes were found, but none of the probed ports positively
/// identified as the API for any of them.
pub(crate) const ERR_API_NOT_FOUND: &str = "antigravity-api-not-found";

/// The RPC path the probe uses to positively identify the API. Any RPC that
/// requires the CSRF header would do; this one is cheap and side-effect free.
const PROBE_PATH: &str =
    "/exa.language_server_pb.LanguageServerService/GetUserTrajectoryDescriptions";

/// The CSRF header the language server checks.
const CSRF_HEADER: &str = "x-codeium-csrf-token";

/// How far past the extension-server port to sweep looking for the API.
/// Observed offsets across live instances ranged from +1 to +27, with no
/// fixed relationship to compute from -- hence a bounded sweep rather than a
/// single guess.
const PROBE_WINDOW: std::ops::RangeInclusive<u16> = 1..=64;

/// Per-request timeout for a single probe, and for the liveness connect in
/// `open_ports`.
///
/// This doc used to say a closed port refuses the connection immediately, so
/// only a blackholed port pays the full timeout. That is true on macOS and
/// Linux and false on Windows, which does not refuse a connect to a closed
/// loopback port -- it runs the timeout out. CI proved it: the probe test
/// failed there on `is_timeout()`, not on a slow refusal.
///
/// So the window's cost is set by how many timeouts can overlap, not by how
/// many ports are closed. `open_ports` runs the liveness connects
/// concurrently, which puts the worst case at roughly one timeout rather
/// than `PROBE_WINDOW` x `PROBE_TIMEOUT` (~16s). Only the ports that
/// actually listen go on to the sequential HTTP probe, and a blackholed one
/// among those still pays the timeout at most once -- a timed-out first
/// request aborts that port's probe before the second is ever sent.
const PROBE_TIMEOUT: Duration = Duration::from_millis(250);

/// The discovered API: the port it listens on and the CSRF token it expects.
pub(crate) struct Endpoint {
    pub port: u16,
    pub token: String,
}

/// A language server process found in the process table: its CSRF token and
/// the extension-server port on its command line. Neither is the API port
/// by itself -- `discover` probes outward from the extension-server port to
/// find it.
pub(crate) struct Candidate {
    pub token: String,
    pub extension_server_port: u16,
}

/// Parses candidates out of raw process command lines. Split out from
/// `discover` so the parsing logic is testable without a running IDE.
///
/// A process is a candidate only if its executable's file name starts with
/// `language_server_` and its command line carries both `--csrf_token` and
/// `--extension_server_port`, each followed by a value.
pub(crate) fn candidates_from(cmdlines: &[Vec<String>]) -> Vec<Candidate> {
    cmdlines
        .iter()
        .filter_map(|cmd| candidate_from_one(cmd))
        .collect()
}

fn candidate_from_one(cmd: &[String]) -> Option<Candidate> {
    let exe = cmd.first()?;
    let file_name = exe.rsplit(['/', '\\']).next().unwrap_or(exe.as_str());
    if !file_name.starts_with("language_server_") {
        return None;
    }

    let token = value_after(cmd, "--csrf_token")?;
    let port_str = value_after(cmd, "--extension_server_port")?;
    let extension_server_port: u16 = port_str.parse().ok()?;

    Some(Candidate {
        token,
        extension_server_port,
    })
}

/// Returns the argument immediately following the first occurrence of
/// `flag`, if any.
fn value_after(cmd: &[String], flag: &str) -> Option<String> {
    cmd.iter()
        .position(|arg| arg == flag)
        .and_then(|i| cmd.get(i + 1))
        .cloned()
}

/// Finds the running Antigravity language server and positively identifies
/// which nearby port serves its API.
///
/// This crate's CLI runs its whole `main` inside a `#[tokio::main]` runtime,
/// so this is `async fn` rather than a sync function that builds its own
/// `tokio::Runtime` and blocks on it -- doing that from inside an existing
/// runtime panics ("Cannot start a runtime from within a runtime"). The
/// probe's HTTP calls use the ordinary async `reqwest::Client` for the same
/// reason: `reqwest::blocking` carries the identical trap.
pub(crate) async fn discover() -> Result<Endpoint> {
    let mut system = System::new_all();
    system.refresh_all();

    let cmdlines: Vec<Vec<String>> = system
        .processes()
        .values()
        .map(|p| {
            p.cmd()
                .iter()
                .map(|s| s.to_string_lossy().into_owned())
                .collect()
        })
        .collect();

    let candidates = candidates_from(&cmdlines);
    if candidates.is_empty() {
        anyhow::bail!(ERR_NOT_RUNNING);
    }

    probe_candidates(&candidates)
        .await
        .ok_or_else(|| anyhow::anyhow!(ERR_API_NOT_FOUND))
}

/// Which of `ports` have something listening, in the order given.
///
/// This exists because a closed port is not free on every platform. On
/// Windows a connect to a closed loopback port is not refused -- it runs the
/// timeout out. Swept one port at a time that made the window cost
/// `PROBE_WINDOW` x `PROBE_TIMEOUT`, about sixteen seconds, before `discover`
/// could so much as report that Antigravity is not running. Concurrently the
/// same window costs about one timeout.
///
/// **Why the concurrency is here and not around `probe_port`.** Probing the
/// HTTP surface concurrently would be simpler and would also fix the
/// latency, but `probe_port` sends the CSRF token, and today it sends it to
/// at most one port at a time, only after that port has answered 401, and
/// never at all to ports above the one that matched. Sixty-four concurrent
/// HTTP probes would hand a live token to every local listener that answers
/// 401 in the same instant -- widening exactly the residual the module doc
/// describes. This layer moves no credential: it opens a TCP connection and
/// closes it. The token path above stays sequential, lowest-port-first, and
/// now runs only against ports that are actually listening, which is a
/// strictly smaller set than before.
async fn open_ports(ports: Vec<u16>) -> Vec<u16> {
    // Spawned rather than awaited in a loop: a spawned task starts
    // immediately, so all of these connects are in flight together. The
    // handles are collected in the order `ports` gave, and awaited in that
    // order, so the result preserves it -- `probe_candidates` returns the
    // first match and must still pick the lowest qualifying port.
    let handles: Vec<(u16, tokio::task::JoinHandle<bool>)> = ports
        .into_iter()
        .map(|port| (port, tokio::spawn(is_listening(port))))
        .collect();

    let mut open = Vec::new();
    for (port, handle) in handles {
        // A panicked or cancelled probe is "not listening", never a reason
        // to abandon the sweep: one unusable port must not make the other
        // sixty-three unreachable.
        if handle.await.unwrap_or(false) {
            open.push(port);
        }
    }
    open
}

/// Whether anything accepts a TCP connection on `port`. Opens and drops it;
/// sends nothing.
async fn is_listening(port: u16) -> bool {
    matches!(
        tokio::time::timeout(
            PROBE_TIMEOUT,
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await,
        Ok(Ok(_))
    )
}

async fn probe_candidates(candidates: &[Candidate]) -> Option<Endpoint> {
    // One client, reused across every port and candidate -- a fresh client
    // per port would rebuild a connection pool up to 64 times per candidate
    // for no benefit.
    let client = reqwest::Client::builder()
        .timeout(PROBE_TIMEOUT)
        .build()
        .ok()?;

    for candidate in candidates {
        // `offset` is always >= 1 (see PROBE_WINDOW), so this only saturates
        // when `extension_server_port` is already within 64 of u16::MAX;
        // `checked_add` skips that port instead of re-probing u16::MAX
        // repeatedly.
        let window: Vec<u16> = PROBE_WINDOW
            .filter_map(|offset| candidate.extension_server_port.checked_add(offset))
            .collect();

        // Narrow to ports that are actually listening before sending
        // anything. Concurrent, so a window of closed ports costs about one
        // timeout instead of sixty-four; see `open_ports` for why the
        // concurrency stops here and does not extend to the token probe.
        for port in open_ports(window).await {
            if probe_port(&client, port, &candidate.token).await {
                return Some(Endpoint {
                    port,
                    token: candidate.token.clone(),
                });
            }
        }
    }
    None
}

/// Whether `port` behaves like the language server API expecting `token`:
/// an unauthenticated request must be refused with 401 and a body
/// containing `"unauthenticated"`, and the same request WITH the token must
/// succeed with 200. Both checks must pass.
///
/// This distinguishes the API from an unrelated local service that happens
/// to answer on a swept port. It does not authenticate the peer -- see the
/// module doc for what the 401 half does and does not establish, and for
/// the residual it leaves.
async fn probe_port(client: &reqwest::Client, port: u16, token: &str) -> bool {
    let url = format!("http://127.0.0.1:{port}{PROBE_PATH}");

    // The server 415s a request with no Content-Type before it ever reaches
    // the CSRF check, so this header is required for both probes below --
    // without it every port would look like a 415, not a 401/200 pair.
    let Ok(unauth_resp) = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body("{}")
        .send()
        .await
    else {
        return false;
    };
    if unauth_resp.status().as_u16() != 401 {
        return false;
    }
    let Ok(unauth_body) = unauth_resp.text().await else {
        return false;
    };
    if !unauth_body.contains("unauthenticated") {
        return false;
    }

    let Ok(auth_resp) = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(CSRF_HEADER, token)
        .body("{}")
        .send()
        .await
    else {
        return false;
    };
    auth_resp.status().as_u16() == 200
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, response::IntoResponse, routing::post};

    /// Binds a port, then drops the listener, so the port is known-closed.
    async fn closed_port() -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        port
    }

    #[tokio::test]
    async fn open_ports_reports_only_the_ports_with_a_listener() {
        let live_a = spawn(Router::new()).await;
        let live_b = spawn(Router::new()).await;
        let dead = closed_port().await;

        let mut asked = vec![live_a, live_b, dead];
        asked.sort_unstable();
        let open = open_ports(asked).await;

        let mut expected = vec![live_a, live_b];
        expected.sort_unstable();
        assert_eq!(
            open, expected,
            "only ports with a listener may be handed on to the HTTP probe"
        );
        // Ascending order is load-bearing: `probe_candidates` walks this list
        // in order and returns the first match, so the lowest qualifying port
        // must win exactly as it did when the sweep was sequential.
        assert!(open.windows(2).all(|w| w[0] < w[1]), "must stay ascending");
    }

    #[tokio::test]
    async fn closed_ports_do_not_serialize_their_timeouts() {
        // The whole point of the liveness pre-check. Windows does not refuse
        // a connect to a closed loopback port -- it runs the timeout out (CI
        // proved this on `worktree-antigravity-trajectory-probe`, where the
        // reworked probe test failed on `is_timeout()`). Swept one at a time
        // that is 64 x 250ms ~= 16s before `discover` can report that
        // Antigravity is not there.
        //
        // Unlike the assertion this replaces, there is a real gap to put a
        // threshold in: sequential costs the full window, concurrent costs
        // about one timeout. The bound below sits at a quarter of the
        // sequential cost -- roughly 4x under what a serialized sweep must
        // exceed and many times over what a concurrent one needs -- so it
        // separates the two behaviours without measuring scheduling noise.
        let ports: Vec<u16> = {
            let mut v = Vec::new();
            for _ in 0..PROBE_WINDOW.count() {
                v.push(closed_port().await);
            }
            v.sort_unstable();
            v.dedup();
            v
        };
        let asked = ports.len();
        let serialized_cost = PROBE_TIMEOUT * u32::try_from(asked).unwrap();

        let started = std::time::Instant::now();
        let open = open_ports(ports).await;
        let elapsed = started.elapsed();

        assert!(open.is_empty(), "nothing is listening on any of them");
        assert!(
            elapsed < serialized_cost / 4,
            "sweeping {asked} closed ports took {elapsed:?}; serialized they would cost \
             {serialized_cost:?}, so this is not being done concurrently"
        );
    }

    /// Binds a router to a real 127.0.0.1 socket and returns the port it is
    /// listening on. Matches the shape already used in this crate's other
    /// HTTP-contract tests (see `issuer_client.rs::tests::spawn`).
    async fn spawn(router: Router) -> u16 {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        port
    }

    fn probe_client() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(PROBE_TIMEOUT)
            .build()
            .unwrap()
    }

    const TEST_TOKEN: &str = "test-token-abc123";

    /// A router shaped exactly like the real language server: 415s any
    /// request missing `Content-Type: application/json` (regardless of any
    /// other header), 401s with an `"unauthenticated"` body when that header
    /// is present but the CSRF token is missing or wrong, and 200s when both
    /// are correct. This is what `probe_port` must positively identify --
    /// and because it 415s without Content-Type, this test also pins the
    /// Content-Type bug the live-instance test found: if `probe_port` ever
    /// stops sending that header, this server would 415 both requests and
    /// the assertion below would fail.
    fn real_shaped_router() -> Router {
        Router::new().route(
            PROBE_PATH,
            post(|headers: axum::http::HeaderMap, _body: String| async move {
                let Some(ct) = headers.get(reqwest::header::CONTENT_TYPE) else {
                    return (axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE, "").into_response();
                };
                if ct != "application/json" {
                    return (axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE, "").into_response();
                }
                match headers.get(CSRF_HEADER) {
                    Some(v) if v == TEST_TOKEN => {
                        (axum::http::StatusCode::OK, Json(serde_json::json!({}))).into_response()
                    }
                    _ => (
                        axum::http::StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({"code": "unauthenticated"})),
                    )
                        .into_response(),
                }
            }),
        )
    }

    #[tokio::test]
    async fn probe_identifies_a_server_that_demands_and_accepts_the_token() {
        let port = spawn(real_shaped_router()).await;
        let client = probe_client();
        assert!(probe_port(&client, port, TEST_TOKEN).await);
    }

    #[tokio::test]
    async fn probe_rejects_a_server_that_answers_200_unconditionally() {
        // The "something answered" case the two-step check exists to
        // reject: no auth gate at all, so the first (unauthenticated)
        // request already returns 200 instead of 401.
        let router = Router::new().route(
            PROBE_PATH,
            post(|| async { (axum::http::StatusCode::OK, Json(serde_json::json!({}))) }),
        );
        let port = spawn(router).await;
        let client = probe_client();
        assert!(!probe_port(&client, port, TEST_TOKEN).await);
    }

    #[tokio::test]
    async fn probe_rejects_a_server_that_401s_for_every_token() {
        // Right shape (401 + "unauthenticated" when unauthenticated), wrong
        // process: it never accepts any token, including the exact one this
        // candidate carries. Must not be mistaken for the real API just
        // because the first-stage check happens to pass.
        let router = Router::new().route(
            PROBE_PATH,
            post(|| async {
                (
                    axum::http::StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"code": "unauthenticated"})),
                )
            }),
        );
        let port = spawn(router).await;
        let client = probe_client();
        assert!(!probe_port(&client, port, TEST_TOKEN).await);
    }

    #[tokio::test]
    async fn probe_rejects_a_server_that_415s_everything() {
        // An unrelated responder on a nearby port that never reaches an
        // auth decision at all (real or synthetic 415, e.g. one that
        // doesn't understand this content type no matter what is sent).
        // Never mistaken for a 401.
        let router = Router::new().route(
            PROBE_PATH,
            post(|| async { axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE }),
        );
        let port = spawn(router).await;
        let client = probe_client();
        assert!(!probe_port(&client, port, TEST_TOKEN).await);
    }

    #[tokio::test]
    async fn probe_skips_a_port_with_nothing_listening_without_stalling() {
        // Bind to grab a free port, then drop the listener so nothing is
        // there to answer -- the connection is refused rather than hanging
        // until PROBE_TIMEOUT. This matters because `discover` walks up to 64
        // ports in sequence: if a closed port costs a full timeout instead of
        // a refusal, discovery goes from near-instant to sixteen seconds.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);

        let client = probe_client();
        assert!(!probe_port(&client, port, TEST_TOKEN).await);

        // Assert the reason, not the clock.
        //
        // This was originally `elapsed < PROBE_TIMEOUT` around the call above,
        // which is a measurement with no margin: the budget being checked is
        // the very constant that bounds the operation, and the elapsed time
        // also includes client setup and task scheduling. On a loaded Windows
        // CI runner it came in at 252.8ms against a 250ms timeout and failed
        // the branch. Widening it to a tolerance would have been worse than
        // the flake -- a genuine stall lands at almost exactly PROBE_TIMEOUT,
        // so any margin big enough to absorb the noise also swallows the
        // defect the test exists to catch.
        //
        // The property is not "this was fast", it is "this failed by refusal
        // rather than by running out the clock". reqwest distinguishes those
        // two, so ask it directly: immune to load, and still red if a
        // platform really does leave closed-port connects hanging.
        let error = client
            .post(format!("http://127.0.0.1:{port}{PROBE_PATH}"))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body("{}")
            .send()
            .await
            .expect_err("nothing is listening, so this cannot succeed");
        assert!(
            !error.is_timeout(),
            "connecting to a closed port ran out the {PROBE_TIMEOUT:?} timeout \
             instead of being refused -- probe stalls on closed ports, making \
             a full sweep cost 64 timeouts: {error}"
        );
        assert!(
            error.is_connect(),
            "expected a connection error from a closed port, got: {error}"
        );
    }

    #[test]
    fn a_language_server_command_line_yields_its_token_and_extension_port() {
        let cmdlines = vec![vec![
            "/Applications/Antigravity IDE.app/Contents/Resources/app/extensions/antigravity/bin/language_server_macos_arm".to_string(),
            "--enable_lsp".to_string(),
            "--csrf_token".to_string(),
            "114d1b72-7bc2-4c3c-b165-196ce5403d72".to_string(),
            "--extension_server_port".to_string(),
            "65402".to_string(),
        ]];
        let found = candidates_from(&cmdlines);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].token, "114d1b72-7bc2-4c3c-b165-196ce5403d72");
        assert_eq!(found[0].extension_server_port, 65402);
    }

    #[test]
    fn an_unrelated_process_is_not_a_candidate() {
        let cmdlines = vec![vec![
            "/usr/bin/ssh".to_string(),
            "--csrf_token".to_string(),
            "x".to_string(),
        ]];
        assert!(candidates_from(&cmdlines).is_empty());
    }

    #[test]
    fn a_language_server_without_a_token_is_not_a_candidate() {
        let cmdlines = vec![vec![
            "language_server_macos_arm".to_string(),
            "--enable_lsp".to_string(),
        ]];
        assert!(candidates_from(&cmdlines).is_empty());
    }

    /// Exercises the real probe. Skips loudly when Antigravity is not
    /// running, because CI has no IDE -- the skip must be visible so a
    /// permanently skipped test is not mistaken for coverage.
    #[tokio::test]
    async fn discovery_finds_a_live_endpoint_when_antigravity_is_running() {
        match discover().await {
            Ok(e) => assert!(e.port > 0 && !e.token.is_empty()),
            Err(err) => {
                eprintln!("skipping: no live Antigravity endpoint ({err})");
            }
        }
    }
}
