//! One line of IronWire state, inside the agent's own UI.
//!
//! IronWire deliberately does not write into a response stream: the only text
//! channel there is the model's own, and putting words in it would be putting
//! words in the model's mouth (`docs/DESIGN.md` §7). A status line is the
//! channel that does not have that problem — it is the harness's own furniture,
//! rendered outside the transcript, and the harness invites us into it.
//!
//! Claude Code runs the command in `statusLine` on every render and hands it a
//! JSON document on stdin (session, model, cost, rate limits). The command
//! reads that when it is there, asks the daemon what it is actually doing, and
//! prints a single line.
//!
//! This lives in the library rather than in the `ironwire` binary so that a
//! host embedding the proxy ([`crate::embed`]) can serve the same line from its
//! own executable: it exposes a subcommand that calls [`line_for_home`] (or
//! [`line()`]) and installs that subcommand with
//! `ironwire_agents::tools::StatusLine::Command`. `ironwire statusline` is one
//! such caller.
//!
//! Three properties matter more than what it says:
//!
//! 1. **It never blocks the UI.** This runs on every render of somebody's
//!    editor. The control call has a short timeout and every failure path
//!    returns `None` rather than an error — a status line that stalls or shouts
//!    is worse than no status line.
//! 2. **It never invents.** Same rule as `ironwire status`: a number we were
//!    not told is not printed (`docs/CRITIQUE.md` §4).
//! 3. **It is quiet when things are normal**, so that the moments when it is
//!    not are worth reading.

use std::path::Path;

use chrono::{DateTime, Utc};
use ironwire_core::config::{Config, PathsConfig};
use ironwire_core::discovery::Endpoint;
use ironwire_update::UpdateStatus;

use crate::control::{BackendView, HeadroomView, StatusView};

/// How long the daemon gets before we give up and print nothing. Chosen
/// against the render loop this sits in, not against the network.
pub const BUDGET: std::time::Duration = std::time::Duration::from_millis(400);

/// What the harness told us about itself, of the parts we use.
#[derive(Debug, Default, serde::Deserialize)]
struct ClientContext {
    #[serde(default)]
    model: Option<ClientModel>,
}

#[derive(Debug, serde::Deserialize)]
struct ClientModel {
    #[serde(default)]
    id: Option<String>,
}

impl ClientContext {
    fn model_id(self) -> Option<String> {
        self.model.and_then(|m| m.id)
    }
}

/// The model id in the document Claude Code sends a status-line command on
/// stdin, if it names one.
///
/// Anything unparseable is "no context" rather than an error, and fields we do
/// not model are ignored: the harness will add fields, and parsing must not
/// become all-or-nothing over ones we never asked about.
#[must_use]
pub fn client_model(document: &str) -> Option<String> {
    serde_json::from_str::<ClientContext>(document)
        .ok()?
        .model_id()
}

/// [`client_model`], read from this process's stdin.
///
/// Not every harness sends a document, and a status line invoked by hand has a
/// terminal on stdin rather than a document — so this reads only when stdin is
/// not a TTY.
#[must_use]
pub fn client_model_from_stdin() -> Option<String> {
    use std::io::{IsTerminal, Read};
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut buffer = String::new();
    stdin.read_to_string(&mut buffer).ok()?;
    client_model(&buffer)
}

/// The port and control token of the proxy that owns `home`, read without
/// creating or writing anything.
///
/// The port comes from `<home>/endpoint.json`, which every proxy started with
/// [`crate::embed`] publishes once it has bound — the only place an ephemeral
/// port is written down. Without that pointer it falls back to the port in the
/// home's configuration, which is where a daemon that predates the pointer
/// listens. A pointer naming anything other than loopback is not followed: the
/// token goes to `127.0.0.1` or nowhere.
///
/// The pointer is followed only when it names this home's token file.
/// `ironwire serve` writes `~/.ironwire/endpoint.json` whatever
/// `$IRONWIRE_HOME` says, so a host whose home is `~/.ironwire` can find a
/// pointer there left by a daemon in another home. Its port is that daemon's,
/// and sending this home's token to it would print nothing for as long as the
/// pointer lasts; such a pointer is treated as absent.
///
/// The token is read, never minted. A status line that created a credential on
/// its way to printing nothing would be a side effect on every render.
#[must_use]
pub fn endpoint_for_home(home: &Path) -> Option<(u16, String)> {
    let paths = PathsConfig::rooted_at(home);
    let token_file = paths.control_token_file();
    let pointer = Endpoint::read_from(&home.join("endpoint.json"))
        .filter(|endpoint| same_file(&endpoint.token_path, &token_file));
    let port = match pointer {
        Some(endpoint) => loopback_port(&endpoint.control_url)?,
        None => Config::load(&paths).ok()?.server.port,
    };
    let token = std::fs::read_to_string(&token_file).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| (port, token.to_string()))
}

/// Whether two paths name the same file: equal as written, or equal once
/// resolved, so a home reached through a symlink still recognises its own
/// pointer. Resolution only reads; a path that does not resolve is compared as
/// written and nothing else.
fn same_file(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(a), Ok(b)) if a == b
        )
}

fn loopback_port(control_url: &str) -> Option<u16> {
    control_url
        .strip_prefix("http://127.0.0.1:")?
        .trim_end_matches('/')
        .parse()
        .ok()
}

/// Ask the proxy on `port` for its state, within [`BUDGET`].
///
/// `None` for every failure — not running, wrong token, too slow, a body that
/// does not parse — because a status line has no way to report one usefully.
pub async fn fetch_status(port: u16, token: &str) -> Option<StatusView> {
    tokio::time::timeout(BUDGET, async {
        let client = reqwest::Client::builder()
            .timeout(BUDGET)
            // Loopback only, and a bearer token in the request: an HTTP proxy
            // from the environment has no business seeing either.
            .no_proxy()
            // Plain HTTP to loopback needs no trust store, and loading the
            // platform's is the slowest step here -- slow enough, on a busy
            // machine, to spend the whole budget before a byte is sent.
            .tls_built_in_root_certs(false)
            .build()
            .ok()?;
        let response = client
            .get(format!("http://127.0.0.1:{port}/_ironwire/status"))
            .bearer_auth(token)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json::<StatusView>().await.ok()
    })
    .await
    .ok()
    .flatten()
}

/// The status line for the proxy on `port`, or `None` when there is nothing
/// to say or no way to find out.
///
/// `client_model` is the model the harness says it asked for — see
/// [`client_model_from_stdin`].
pub async fn line(port: u16, token: &str, client_model: Option<&str>) -> Option<String> {
    render(&fetch_status(port, token).await?, client_model)
}

/// The status line for the proxy that owns `home` — the form a host that
/// starts one proxy per home wants. See [`endpoint_for_home`] for how the port
/// and token are found.
pub async fn line_for_home(home: &Path, client_model: Option<&str>) -> Option<String> {
    let (port, token) = endpoint_for_home(home)?;
    line(port, &token, client_model).await
}

/// Render the line from a status the caller already has, as of now.
#[must_use]
pub fn render(status: &StatusView, client_model: Option<&str>) -> Option<String> {
    render_at(status, client_model, Utc::now())
}

/// Render the line as of `now`: the pure form, with no clock and no I/O.
///
/// `None` when no backend is able to serve: there is nowhere traffic is going,
/// so there is no line.
#[must_use]
pub fn render_at(
    status: &StatusView,
    client_model: Option<&str>,
    now: DateTime<Utc>,
) -> Option<String> {
    let mut parts = vec![routing(status, client_model)?];
    if let Some(switch) = switched(status, now) {
        parts.push(switch);
    }
    parts.extend(capacity(status));
    if let Some(update) = update(&status.update) {
        parts.push(update);
    }
    Some(parts.join(" · "))
}

/// How long a route change stays worth mentioning.
///
/// A fallback matters most in the minutes after it happens, when the user can
/// still connect it to what they were doing. Left up permanently it becomes
/// furniture, and stops being read at exactly the moment it matters again.
const SWITCH_VISIBLE_FOR: chrono::Duration = chrono::Duration::minutes(10);

/// The line's reason for existing: say when traffic moved, and from where.
fn switched(status: &StatusView, now: DateTime<Utc>) -> Option<String> {
    let route = status.last_route.as_ref()?;
    let from = route.from.as_deref()?;
    (now - route.at < SWITCH_VISIBLE_FOR).then(|| format!("switched from {from}"))
}

/// Where traffic is going, and whether that is where the client thinks.
fn routing(status: &StatusView, asked: Option<&str>) -> Option<String> {
    let serving = status
        .backends
        .iter()
        .find(|b| b.authenticated && b.consented)?;

    // The client names the model it asked for; we know what actually served it.
    // Printing the second only when it differs keeps the normal case short and
    // makes a substitution impossible to miss — which is the case the user
    // would otherwise discover from a bill or a worse answer.
    let served = status.last_route.as_ref();
    match (asked, served) {
        (Some(asked), Some(route)) if route.model.as_deref().is_some_and(|m| m != asked) => {
            Some(format!(
                "ironwire → {} ({} for {asked})",
                route.backend,
                route.model.as_deref().unwrap_or("?")
            ))
        }
        (_, Some(route)) => Some(format!("ironwire → {}", route.backend)),
        (_, None) => Some(format!("ironwire → {}", serving.id)),
    }
}

/// Capacity, but only for the pools that have actually reported one, and only
/// when there is something to say about them.
fn capacity(status: &StatusView) -> Vec<String> {
    let serving: Vec<&BackendView> = status
        .backends
        .iter()
        .filter(|b| b.authenticated && b.consented)
        .collect();
    serving
        .iter()
        .filter_map(|backend| {
            let name = label(backend, &serving);
            match backend.headroom {
                // Below a third used is not news. A status line that always
                // shows a number teaches people to stop seeing it.
                HeadroomView::Observed { used_pct, .. } if used_pct >= 33.0 => {
                    Some(format!("{name} {used_pct:.0}%"))
                }
                HeadroomView::Exhausted { .. } => Some(format!("{name} exhausted")),
                _ => None,
            }
        })
        .collect()
}

/// A backend's name on the line: the short one, unless another backend that
/// could appear beside it shortens the same way.
///
/// Two Claude logins are both "Claude", and "Claude 95% · Claude exhausted"
/// says nothing about which is which. The id is the next-shortest thing that
/// tells them apart, and the one `ironwire pin` takes. Decided over every
/// serving backend rather than the ones with news, so a label does not change
/// under the user as one window fills.
fn label<'a>(backend: &'a BackendView, serving: &[&BackendView]) -> &'a str {
    let short = short_name(&backend.name);
    let shared = serving
        .iter()
        .filter(|other| short_name(&other.name) == short)
        .count()
        > 1;
    if shared { &backend.id } else { short }
}

/// Notify-only, exactly as everywhere else: say a release exists, never act
/// on it (`docs/UPDATES.md`).
fn update(status: &UpdateStatus) -> Option<String> {
    match status {
        UpdateStatus::Available { latest, .. } => Some(format!("ironwire {latest} available")),
        UpdateStatus::Unsupported { latest, .. } => Some(format!(
            "ironwire {latest} available (this build is unsupported)"
        )),
        UpdateStatus::UpToDate | UpdateStatus::Unknown => None,
    }
}

/// "Claude subscription" is too wide for a line shared with someone's cwd.
fn short_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{HealthView, LastRouteView};

    #[test]
    fn a_client_document_we_do_not_fully_model_still_yields_its_model() {
        // The harness will add fields; parsing must not become all-or-nothing
        // over ones we never asked about.
        let parsed: ClientContext = serde_json::from_str(
            r#"{"session_id":"x","model":{"id":"claude-opus-5","display_name":"Opus 5"},
                "cost":{"total_cost_usd":0},"something_new":{"nested":true}}"#,
        )
        .expect("parses");
        assert_eq!(
            parsed.model.and_then(|m| m.id).as_deref(),
            Some("claude-opus-5")
        );
    }

    #[test]
    fn a_document_without_a_model_is_not_an_error() {
        let parsed: ClientContext = serde_json::from_str(r#"{"session_id":"x"}"#).expect("parses");
        assert!(parsed.model.is_none());
    }

    #[test]
    fn an_up_to_date_build_says_nothing() {
        assert!(update(&UpdateStatus::UpToDate).is_none());
        assert!(update(&UpdateStatus::Unknown).is_none());
    }

    #[test]
    fn a_subscription_name_is_shortened_to_its_first_word() {
        assert_eq!(short_name("Claude subscription"), "Claude");
        assert_eq!(short_name("ChatGPT subscription"), "ChatGPT");
    }

    pub(super) fn status_with(last_route: Option<LastRouteView>) -> StatusView {
        StatusView {
            version: "0.1.0".into(),
            port: 8463,
            tracked_conversations: 1,
            pin: None,
            backends: vec![],
            balance: crate::control::BalanceView::default(),
            privacy: None,
            catalog_serial: 0,
            update: UpdateStatus::UpToDate,
            last_route,
            usage: ironwire_usage::UsageReport::default(),
            cross_account_fallback: false,
        }
    }

    fn route(from: Option<&str>, ago: chrono::Duration) -> LastRouteView {
        LastRouteView {
            backend: "nearai".into(),
            model: None,
            from: from.map(ToString::to_string),
            rung: ironwire_core::policy::Rung::CrossFamily,
            at: Utc::now() - ago,
        }
    }

    #[test]
    fn a_route_that_did_not_change_is_not_announced() {
        // The ordinary case is a conversation staying put, and saying so every
        // render is how a status line stops being read.
        let now = Utc::now();
        assert!(
            switched(
                &status_with(Some(route(None, chrono::Duration::zero()))),
                now
            )
            .is_none()
        );
        assert!(switched(&status_with(None), now).is_none());
    }

    #[test]
    fn a_recent_switch_names_where_it_came_from() {
        let line = switched(
            &status_with(Some(route(
                Some("claude-sub"),
                chrono::Duration::minutes(1),
            ))),
            Utc::now(),
        )
        .expect("announced");
        assert!(line.contains("claude-sub"), "got: {line}");
    }

    /// A fallback from an hour ago is history, not status. Left up it becomes
    /// furniture and stops being read at the moment it matters again.
    #[test]
    fn an_old_switch_stops_being_news() {
        assert!(
            switched(
                &status_with(Some(route(Some("claude-sub"), chrono::Duration::hours(1)))),
                Utc::now(),
            )
            .is_none()
        );
    }

    fn backend(id: &str, name: &str, headroom: HeadroomView) -> BackendView {
        BackendView {
            id: id.into(),
            name: name.into(),
            kind: "subscription".into(),
            authenticated: true,
            consented: true,
            detail: None,
            headroom,
            health: HealthView::default(),
            models: vec![],
            connect_command: None,
        }
    }

    fn serving(last_route: Option<LastRouteView>, backends: Vec<BackendView>) -> StatusView {
        StatusView {
            backends,
            ..status_with(last_route)
        }
    }

    /// The exact strings, because hosts and the CLI must print the same bytes
    /// and these are what a user has learned to read.
    #[test]
    fn the_rendered_line_is_byte_for_byte_what_the_cli_has_always_printed() {
        let now = Utc::now();
        let quiet = backend("claude-sub", "Claude subscription", HeadroomView::Unknown);

        let before_any_route = serving(None, vec![quiet.clone()]);
        assert_eq!(
            render_at(&before_any_route, None, now).as_deref(),
            Some("ironwire → claude-sub")
        );

        let mut substituted = route(None, chrono::Duration::zero());
        substituted.model = Some("glm-5".into());
        let status = serving(Some(substituted.clone()), vec![quiet.clone()]);
        assert_eq!(
            render_at(&status, Some("claude-opus-5"), now).as_deref(),
            Some("ironwire → nearai (glm-5 for claude-opus-5)")
        );
        assert_eq!(
            render_at(&status, Some("glm-5"), now).as_deref(),
            Some("ironwire → nearai")
        );
        assert_eq!(
            render_at(&status, None, now).as_deref(),
            Some("ironwire → nearai")
        );

        let busy = serving(
            Some(route(Some("claude-sub"), chrono::Duration::minutes(1))),
            vec![
                backend(
                    "claude-sub",
                    "Claude subscription",
                    HeadroomView::Observed {
                        used_pct: 50.4,
                        observed_secs_ago: 3,
                        resets_in_secs: None,
                    },
                ),
                backend(
                    "codex-sub",
                    "ChatGPT subscription",
                    HeadroomView::Exhausted { retry_in_secs: 60 },
                ),
                backend(
                    "nearai",
                    "NEAR AI",
                    HeadroomView::Observed {
                        used_pct: 10.0,
                        observed_secs_ago: 3,
                        resets_in_secs: None,
                    },
                ),
            ],
        );
        let busy = StatusView {
            update: UpdateStatus::Available {
                latest: "0.2.0".into(),
                summary: None,
                upgrade_command: None,
            },
            ..busy
        };
        assert_eq!(
            render_at(&busy, None, now).as_deref(),
            Some(
                "ironwire → nearai · switched from claude-sub · Claude 50% · ChatGPT exhausted · ironwire 0.2.0 available"
            )
        );
    }

    /// Never invents: with nothing able to serve there is no route to name.
    #[test]
    fn nothing_able_to_serve_means_no_line() {
        let mut unconsented = backend("claude-sub", "Claude subscription", HeadroomView::Unknown);
        unconsented.consented = false;
        assert!(render_at(&serving(None, vec![unconsented]), None, Utc::now()).is_none());
        assert!(render_at(&status_with(None), None, Utc::now()).is_none());
    }

    #[test]
    fn only_a_loopback_pointer_yields_a_port() {
        assert_eq!(loopback_port("http://127.0.0.1:8463"), Some(8463));
        assert_eq!(loopback_port("http://127.0.0.1:8463/"), Some(8463));
        assert_eq!(loopback_port("http://example.com:8463"), None);
        assert_eq!(loopback_port("http://127.0.0.1.example.com:8463"), None);
        assert_eq!(loopback_port("https://127.0.0.1:8463"), None);
    }
}

#[cfg(test)]
mod multi_login_tests {
    use super::*;

    fn backend(id: &str, name: &str, used_pct: f32) -> BackendView {
        BackendView {
            id: id.into(),
            name: name.into(),
            kind: "subscription".into(),
            authenticated: true,
            consented: true,
            detail: None,
            headroom: HeadroomView::Observed {
                used_pct,
                observed_secs_ago: 10,
                resets_in_secs: None,
            },
            health: crate::control::HealthView::default(),
            models: vec![],
            connect_command: None,
        }
    }

    #[test]
    fn two_logins_that_shorten_alike_are_shown_by_id() {
        let mut status = super::tests::status_with(None);
        status.backends = vec![
            backend("claude-sub", "Claude subscription", 80.0),
            backend("claude-work", "Claude subscription (~/.claude-work)", 95.0),
            backend("codex-sub", "ChatGPT subscription", 50.0),
        ];
        assert_eq!(
            capacity(&status),
            vec!["claude-sub 80%", "claude-work 95%", "ChatGPT 50%"]
        );
        status.backends[0].headroom = HeadroomView::Unknown;
        assert_eq!(capacity(&status), vec!["claude-work 95%", "ChatGPT 50%"]);
    }
}
