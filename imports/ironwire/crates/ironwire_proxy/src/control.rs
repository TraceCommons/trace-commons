//! The local control API.
//!
//! `ironwire status` and the macOS menu bar app are both clients of this — the
//! daemon is the only place routing logic lives, so CLI and GUI cannot drift
//! (`docs/DESIGN.md` §6).
//!
//! Loopback is not sufficient authorisation on a shared machine: this surface
//! exposes the ledger and can change where a user's requests go, so it also
//! requires the token in `$IRONWIRE_HOME/control.token` (mode 0600).

use futures_util::{StreamExt, stream};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Utc};
use ironwire_core::config::PrivacyMode;
use ironwire_core::protocol::BackendId;
use ironwire_core::quota::Headroom;
use ironwire_creds::consent::ConsentPrompt;
use ironwire_creds::{ConsentLedger, SubscriptionLogin, SubscriptionProduct};
use ironwire_ledger::{Exchange, Summary};
use ironwire_update::UpdateStatus;
use ironwire_usage::UsageReport;
use serde::{Deserialize, Serialize};

use crate::state::AppState;

mod token_capture;

/// One backend, as `ironwire status` renders it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendView {
    /// Stable id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// `subscription` / `api_key` / `credits` / `local`.
    pub kind: String,
    /// Whether a credential was found.
    pub authenticated: bool,
    /// Whether consent has been recorded, where it is required.
    pub consented: bool,
    /// Why not authenticated, when applicable.
    pub detail: Option<String>,
    /// Observed capacity — or `unknown`. Never a guess (`docs/CRITIQUE.md` §4).
    pub headroom: HeadroomView,
    /// Circuit state, so a backend that is being skipped says so rather than
    /// looking idle.
    pub health: HealthView,
    /// Models offered.
    pub models: Vec<String>,
    /// What to run to connect this backend — for a subscription, the login it
    /// reads (`ironwire connect claude --config-dir ~/.claude-work`).
    ///
    /// Carried so a client never composes the command from an id: with two
    /// logins of one product, the hardcoded form names the wrong one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_command: Option<String>,
}

/// A backend's circuit state, flattened for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthView {
    /// `closed` / `open` / `half_open`.
    pub circuit: String,
    /// Consecutive failures counted against this backend's health.
    pub consecutive_failures: u32,
    /// Seconds until an open circuit will next allow a probe.
    pub retry_in_secs: Option<i64>,
}

impl Default for HealthView {
    fn default() -> Self {
        Self {
            circuit: "closed".to_string(),
            consecutive_failures: 0,
            retry_in_secs: None,
        }
    }
}

/// Observed capacity, flattened for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum HeadroomView {
    /// The provider reported a number, this long ago.
    Observed {
        /// Percent of the window consumed.
        used_pct: f32,
        /// Seconds since we read it.
        observed_secs_ago: i64,
        /// Seconds until the window resets, when stated.
        resets_in_secs: Option<i64>,
    },
    /// Inside a `retry-after` window.
    Exhausted {
        /// Seconds until it is worth retrying.
        retry_in_secs: i64,
    },
    /// Stopped by a spend cap the user set, not by the provider.
    CapReached {
        /// Spent against the cap in this window.
        spent_usd: f64,
        /// The cap.
        cap_usd: f64,
        /// Seconds until the window rolls over.
        resets_in_secs: i64,
    },
    /// The provider has told us nothing. Displayed as `unknown`, because
    /// showing a plausible number we made up is how the whole status surface
    /// stops being believed.
    Unknown,
}

impl HeadroomView {
    fn from(headroom: &Headroom, now: chrono::DateTime<chrono::Utc>) -> Self {
        match headroom {
            Headroom::Observed {
                used_pct,
                resets_at,
                observed_at,
            } => Self::Observed {
                used_pct: *used_pct,
                observed_secs_ago: (now - *observed_at).num_seconds(),
                resets_in_secs: resets_at.map(|r| (r - now).num_seconds()),
            },
            Headroom::Exhausted { until } => Self::Exhausted {
                retry_in_secs: (*until - now).num_seconds().max(0),
            },
            Headroom::CapReached {
                spent_usd,
                cap_usd,
                resets_at,
            } => Self::CapReached {
                spent_usd: *spent_usd,
                cap_usd: *cap_usd,
                resets_in_secs: (*resets_at - now).num_seconds().max(0),
            },
            Headroom::Unknown => Self::Unknown,
        }
    }
}

/// Full daemon state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusView {
    /// IronWire version.
    pub version: String,
    /// Port in use.
    pub port: u16,
    /// Conversations with a sticky route.
    pub tracked_conversations: usize,
    /// Active pin, if any.
    pub pin: Option<String>,
    /// Every configured backend.
    pub backends: Vec<BackendView>,
    /// Every pool, seen as one balance.
    pub balance: BalanceView,
    /// What the privacy filter is *doing*, never what the user is safe from
    /// (`docs/TRUST.md` I7). `None` when it is off.
    pub privacy: Option<String>,
    /// Serial of the signed catalog document in force; `0` means the values this
    /// binary shipped with (`docs/UPDATES.md`).
    pub catalog_serial: u64,
    /// What the last update check concluded. IronWire never applies an update
    /// itself — this is notification, not action.
    pub update: UpdateStatus,
    /// The most recent route this daemon took, for a status line to display.
    ///
    /// Defaulted on the way in so a newer CLI can read an older daemon's status
    /// instead of refusing it.
    #[serde(default)]
    pub last_route: Option<LastRouteView>,
    /// How fast capacity is going, measured from the local ledger.
    ///
    /// Not quota. [`HeadroomView`] is still the only thing on this screen that
    /// claims to describe a *provider's* remaining capacity, and it still says
    /// `unknown` when the provider has not spoken. This is a measurement of
    /// IronWire's own traffic and every figure in it carries the basis it was
    /// derived from (`ironwire_usage`).
    ///
    /// Empty when capture is off, when `usage.enabled = false`, or when there
    /// is no open window. Defaulted so an older daemon still parses.
    #[serde(default)]
    pub usage: UsageReport,
    /// Whether a client whose own subscription is spent may be served by
    /// another of the user's subscriptions to the same product
    /// (`[subscriptions] cross_account_fallback`).
    ///
    /// On the status screen because it is a risk the user took on and should
    /// never lose sight of: the provider can see the switch (`docs/TRUST.md`).
    /// Defaulted so an older daemon — which had no such setting — reads as off.
    #[serde(default)]
    pub cross_account_fallback: bool,
}

/// The most recent routing decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastRouteView {
    /// Backend chosen.
    pub backend: String,
    /// Model sent upstream, when policy named one. `None` means the client's
    /// own choice went through untouched, which is the ordinary case.
    pub model: Option<String>,
    /// Backend the conversation was on before, when this was a change. This is
    /// the field a status line exists for: a fallback that nobody notices is
    /// one the user cannot act on.
    pub from: Option<String>,
    /// How far down the ladder this route sits.
    ///
    /// Carried rather than left to the caller, because the alternative is a
    /// client inferring "is this degraded" from backend *names* — a second
    /// implementation of a routing question, in a language that cannot see the
    /// policy, drifting the moment the ladder changes. Defaulted on the way in
    /// so an older daemon still parses.
    #[serde(default)]
    pub rung: ironwire_core::policy::Rung,
    /// When it happened.
    pub at: DateTime<Utc>,
}

/// Every pool, seen as one balance.
///
/// Deliberately a *count* of pools rather than a single merged number. The
/// pools are not commensurable: a Claude Max five-hour window, a ChatGPT weekly
/// window, and a dollar balance have no shared unit, and averaging their
/// percentages would produce a figure that looks authoritative and means
/// nothing. What a user actually needs to know is how many places they can still
/// go, whether any of them is free, and when the closed ones come back —
/// all of which are answerable without inventing a unit.
///
/// Every field here is derived from an *observation* or from a recorded cost.
/// Nothing is estimated (`docs/CRITIQUE.md` §4).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BalanceView {
    /// Pools that could serve a request right now.
    pub available: usize,
    /// Available pools whose marginal cost is zero — a subscription or credits
    /// already paid for. The number that decides whether a user should worry.
    pub free_available: usize,
    /// Pools that are authenticated and consented but have not yet reported
    /// their capacity. Named rather than folded into `available`, because "we
    /// do not know" is not "there is room".
    pub unknown: usize,
    /// Pools that are exhausted, or whose circuit is open.
    pub unavailable: usize,
    /// When the first unavailable pool is expected back.
    pub next_available_at: Option<DateTime<Utc>>,
    /// Spend on *metered* backends in the last 24 hours. `None` when the
    /// ledger is off — which means unmeasured, not zero.
    ///
    /// Subscription and credit traffic is deliberately not in here. The ledger
    /// prices every exchange, including the ones served by a subscription, and
    /// summing all of it produced a "spend" figure for a day on which nothing
    /// was billed — the opposite of what this proxy exists to tell you.
    pub spend_today_usd: Option<f64>,
    /// Cache reads as a fraction of every prompt token in the window, when
    /// anything reported usage. The prompt cache is the largest cost lever in a
    /// coding session and the thing this router's whole design protects; it was
    /// the one number nobody could see.
    #[serde(default)]
    pub cache_hit_rate: Option<f64>,
    /// Exchanges summarised for that rate, so the figure carries its basis.
    #[serde(default)]
    pub cache_exchanges: i64,
    /// The configured daily spend cap and what has gone against it, when the
    /// user set one. A permanent line rather than a startup message, following
    /// the privacy filter's precedent: a limit you cannot see is one you cannot
    /// trust.
    #[serde(default)]
    pub spend_cap: Option<SpendCapView>,
    /// What each subscription has used of its own window, as the provider
    /// reported it. The unit that matters for capacity already paid for: a
    /// percentage, not a price.
    ///
    /// Defaulted on the way in so a newer CLI against an older daemon renders
    /// what it can instead of refusing the whole status screen — the daemon
    /// outlives the shell that talks to it, so the two versions *will* differ.
    #[serde(default)]
    pub subscription_used: Vec<SubscriptionUse>,
}

/// A configured spend cap, and progress against it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SpendCapView {
    /// Spent against the cap in this window.
    pub spent_usd: f64,
    /// The cap.
    pub cap_usd: f64,
}

/// One subscription's consumption of its own window.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubscriptionUse {
    /// Backend id, e.g. `claude-work` — what tells two logins of one product
    /// apart. Empty from a daemon that predates several logins.
    #[serde(default)]
    pub backend: String,
    /// Display name, e.g. `Claude subscription`.
    pub name: String,
    /// Percent of the window consumed, as the provider reported it. `None`
    /// means the provider has not said — never a guess.
    pub used_pct: Option<f32>,
    /// Exchanges served in the last 24 hours, from the local ledger.
    pub exchanges: i64,
}

/// The settings a client may change, and everything it needs to render them
/// without deciding anything itself.
///
/// The menu bar app is the reason this exists. It could read `privacy` off
/// [`StatusView`] and offer four buttons — and it would be wrong, because
/// whether `full` is even selectable depends on `trusted_backends`, which is a
/// rule that lives in [`Config::validate`] and would then have a second,
/// drifting implementation in Swift. So the daemon says which options exist,
/// which are selectable, and why not (`docs/DESIGN.md` §6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsView {
    /// The privacy filter, and the modes it could be switched to.
    pub privacy: PrivacySettingsView,
    /// Everything a user can log into, and what it would take.
    pub services: Vec<ServiceView>,
    /// Every coding agent IronWire knows about, and whether it is pointed here.
    ///
    /// Separate from `services` because they answer different questions: a
    /// service is capacity IronWire can spend, a tool is something on this
    /// machine that may or may not be sending its traffic through us. A user
    /// with a working subscription and an unwired agent has everything
    /// configured and nothing routing, and only this list explains it.
    #[serde(default)]
    pub tools: Vec<ToolView>,
}

/// One coding agent, as a settings screen needs to describe it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolView {
    /// Stable id — what `ironwire disconnect <id>` accepts.
    pub id: String,
    /// What the user calls it.
    pub name: String,
    /// The file IronWire would edit, when one can be located.
    pub config_path: Option<String>,
    /// Whether the tool looks present on this machine.
    pub installed: bool,
    /// Whether its config currently routes through IronWire.
    pub wired: bool,
    /// What to run to point it here. A GUI cannot edit somebody's agent config
    /// on their behalf without the same "name the file first" ceremony the CLI
    /// does, so it names the command instead (`docs/TRUST.md` §5).
    pub connect_command: String,
}

/// The privacy filter as a settings screen sees it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacySettingsView {
    /// The mode in force, as it serialises: `off` / `credentials` / `pii` /
    /// `full`.
    pub mode: String,
    /// What the filter is *doing*, in the daemon's own words. Rendered
    /// verbatim or not at all — never restated as what the user is safe from
    /// (`docs/TRUST.md` I7).
    pub summary: String,
    /// Every mode, in ladder order.
    pub options: Vec<PrivacyOptionView>,
    /// Backends the user named as acceptable destinations under `full`.
    pub trusted_backends: Vec<String>,
}

/// One rung of the privacy ladder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacyOptionView {
    /// The value to send back to `POST /_ironwire/privacy`.
    pub id: String,
    /// What this level substitutes, in one clause.
    pub describes: String,
    /// Whether switching to it right now would work.
    pub selectable: bool,
    /// Why it would not, when it would not. Present so a greyed-out option can
    /// say what to do about itself instead of just being greyed out.
    pub unavailable_because: Option<String>,
}

/// One thing a user can log into.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceView {
    /// Backend id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// `subscription` / `api_key` / `credits` / `local`.
    pub kind: String,
    /// Whether a credential was found.
    pub authenticated: bool,
    /// Why not, when it was not.
    pub detail: Option<String>,
    /// Whether this backend is gated behind recorded consent.
    pub requires_consent: bool,
    /// Whether that consent is currently recorded, at the current prompt
    /// version.
    pub consented: bool,
    /// The exact question that has to be answered to enable it.
    ///
    /// Carried rather than written into the client, because a second copy of a
    /// consent prompt is a second prompt — and the recorded version would go on
    /// claiming both users answered the same one (`docs/TRUST.md` §2).
    pub consent_prompt: Option<ConsentPrompt>,
    /// What to run for the part a GUI cannot do: pointing a coding agent at
    /// IronWire means editing the user's shell profile or their agent's config,
    /// which is theirs to own.
    pub connect_command: Option<String>,
}

/// Body of `POST /_ironwire/privacy`.
#[derive(Debug, Clone, Deserialize)]
pub struct PrivacyRequest {
    /// The mode to switch to.
    pub mode: String,
}

/// Body of `POST /_ironwire/consent`.
#[derive(Debug, Clone, Deserialize)]
pub struct ConsentRequest {
    /// Backend to grant or withdraw consent for.
    pub backend: String,
    /// `true` to record consent, `false` to withdraw it.
    pub granted: bool,
    /// The prompt version the user actually answered.
    ///
    /// Required, and checked against the current one. A client that has been
    /// running since before the wording changed would otherwise record consent
    /// to the new question on the strength of the old one having been shown —
    /// which is exactly what versioning the prompt exists to prevent.
    pub prompt_version: u32,
    /// The login binding carried by the prompt that was presented. Required
    /// for grants; withdrawals must remain available to older clients.
    #[serde(default)]
    pub binding: Option<String>,
}

/// Body of `POST /_ironwire/pin`.
#[derive(Debug, Clone, Deserialize)]
pub struct PinRequest {
    /// Backend to pin to. `None` clears the pin.
    pub backend: Option<String>,
    /// Model to force. Ignored when `backend` is `None`.
    pub model: Option<String>,
}

/// One backend's live-probe verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeView {
    /// Stable id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Whether the probe succeeded. False when it was not sent ([`Self::probed`]).
    pub ok: bool,
    /// Round-trip milliseconds.
    pub latency_ms: u64,
    /// What went wrong, when it did.
    pub error: Option<String>,
    /// Whether a request went out at all. False for a subscription awaiting
    /// consent: probing it would send its token before the user said yes.
    /// Defaults to true for a daemon that predates the field, which probed
    /// everything.
    #[serde(default = "probed_by_default")]
    pub probed: bool,
    /// Not probed because this subscription is awaiting consent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub awaiting_consent: bool,
    /// What to run to connect this backend, as in [`BackendView`]; set when it
    /// is awaiting consent, where the answer is that command with
    /// `--subscription`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_command: Option<String>,
}

fn probed_by_default() -> bool {
    true
}

/// Query for `GET /_ironwire/log`.
#[derive(Debug, Clone, Deserialize)]
pub struct LogQuery {
    /// How many exchanges to return.
    #[serde(default = "default_log_limit")]
    pub limit: usize,
    /// Return only exchanges at or after this instant, RFC 3339.
    ///
    /// Use the `Z` form (`2026-09-01T12:00:00Z`) or percent-encode the offset:
    /// a literal `+` in a query string is a space, so `+00:00` arrives as
    /// ` 00:00` and the request is rejected as malformed.
    ///
    /// Changes the order to **oldest first**, and pairs with [`Self::after_id`]
    /// to page forward through the window.
    #[serde(default)]
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    /// Return only exchanges with an `id` greater than this. Requires
    /// [`Self::since`].
    ///
    /// The cursor, and the reason `since` alone is not one. Pass the `id` of
    /// the last row you received; the next page starts after it. Keep going
    /// until a response is shorter than `limit`.
    ///
    /// Paging on the timestamp instead does not work, and fails quietly in two
    /// ways: `since` is inclusive against a column that is not unique, so the
    /// boundary row comes back on every request, and once `limit` exchanges
    /// share a timestamp the caller stops advancing at all. `id` is unique and
    /// assigned in insert order, so it is a total order over exactly the rows
    /// already seen.
    #[serde(default)]
    pub after_id: Option<i64>,
}

fn default_log_limit() -> usize {
    20
}

/// What `GET /_ironwire/log` returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogView {
    /// Whether local capture is on at all.
    pub enabled: bool,
    /// The exchanges that matched. Newest first, or oldest first when the
    /// request named a `since` -- see [`LogQuery::since`].
    ///
    /// Each carries its `id`. A caller paging a window passes the last one
    /// back as [`LogQuery::after_id`] and stops when a page is short.
    pub exchanges: Vec<Exchange>,
    /// Aggregate over the last 24 hours.
    pub last_24h: Summary,
}

/// Query for `GET /_ironwire/summary`.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct SummaryQuery {
    /// Start of the window, RFC 3339 (the `Z` form, or a percent-encoded
    /// offset -- see [`LogQuery::since`]). Defaults to 24 hours ago.
    #[serde(default)]
    pub since: Option<chrono::DateTime<chrono::Utc>>,
}

/// What `GET /_ironwire/summary` returns: calls, cost and proof status per
/// model and backend, and the routed/outside split.
///
/// "Routed" means served by a backend that signs its answers (NEAR AI), where
/// a proof can exist; "outside" means anything else. Only
/// `proof.verified` counts as proof -- see `ironwire_ledger::proof`.
///
/// There is no kind-of-work breakdown: nothing in IronWire classifies work,
/// so every group's `work_kind` is `null` rather than a guess.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SummaryView {
    /// Whether local capture is on at all. When it is off everything below is
    /// empty, which is not the same as "no calls".
    pub enabled: bool,
    /// Whether receipt checks are on (`capture.receipts`). When off, routed
    /// calls stay `pending`.
    pub receipts: bool,
    /// The window's start, as applied.
    pub since: chrono::DateTime<chrono::Utc>,
    /// The rollup.
    #[serde(flatten)]
    pub summary: ironwire_ledger::ProofSummary,
}

/// Routes for the control API.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/token-captures", post(token_capture::handle))
        .route("/status", get(status))
        .route("/backends", get(status))
        .route("/pin", post(pin))
        .route(
            "/admission-binding",
            post(admission_binding)
                .get(admission_binding_capability)
                .delete(revoke_admission_binding)
                .layer(axum::extract::DefaultBodyLimit::max(4096)),
        )
        .route("/settings", get(settings))
        .route("/privacy", post(privacy))
        .route("/consent", post(consent))
        .route("/tools", post(tools))
        .route("/probe", post(probe))
        .route("/log", get(log))
        .route("/summary", get(summary))
        .route("/events", get(events))
        .route("/health", get(health))
}

/// Why `full` cannot be switched on right now, or `None` when it can.
///
/// **One rule, one place.** `GET /settings` greys the option out with this and
/// `POST /privacy` refuses with this; when they were written separately they
/// drifted immediately — the settings screen greyed `full` out while the
/// endpoint behind it accepted the very same change.
///
/// Naming a destination is not the same as having one. `trusted_backends`
/// defaults to `["nearai"]`, and that backend registers whether or not a key was
/// found, so "named" is true on machines where `full` can route nowhere.
/// Selecting it there fails every request as "rate limited", which is both wrong
/// and unactionable.
fn full_is_blocked(
    privacy: &ironwire_core::config::PrivacyConfig,
    statuses: &[ironwire_upstream::backend::BackendStatus],
) -> Option<String> {
    if privacy.trusted_backends.is_empty() {
        return Some(
            "`full` routes only to backends you have named as acceptable, and none are named. \
             Add `trusted_backends` under `[privacy]` in config.toml first — which operators \
             you trust with your data is not IronWire's call."
                .to_string(),
        );
    }
    let usable = privacy.trusted_backends.iter().any(|id| {
        statuses
            .iter()
            .any(|status| status.id.as_str() == id && status.authenticated)
    });
    (!usable).then(|| {
        format!(
            "`full` would route only to {}, and none of those has a credential yet — every \
             request would be refused. Connect one first, or name a backend you do have under \
             `trusted_backends` in config.toml.",
            privacy.trusted_backends.join(", ")
        )
    })
}

/// Body of `POST /_ironwire/tools`.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolRequest {
    /// Which tool.
    pub id: String,
    /// `true` to point it here, `false` to take it back off.
    pub connect: bool,
    /// `true` to work the change out and report it without making it.
    ///
    /// Defaults to `false`, which is what the endpoint has always done. A
    /// client that asks a user before editing their config asks with this, and
    /// then sends the same request again with `as_previewed` set to the
    /// `digest` it was answered with.
    #[serde(default)]
    pub dry_run: bool,
    /// The `digest` from the preview the user was actually shown.
    ///
    /// When present, the edit is made only if it is still that same edit — the
    /// same file, the same bytes in it, and the same bytes it would leave. It
    /// is not only "has the file changed": `id` and `connect` are supplied
    /// again on this call, and a partly wired config has a real connect and a
    /// real disconnect worked out from identical bytes.
    ///
    /// Absent, the edit is made against whatever is there now — the behaviour
    /// every existing caller gets.
    #[serde(default)]
    pub as_previewed: Option<String>,
}

/// Point a coding agent at IronWire, or take it back off.
///
/// This is the one endpoint that writes to a file outside `$IRONWIRE_HOME`, and
/// it exists because the alternative — a menu that can see an unwired agent and
/// can only recite a command about it — is a worse answer to "why is nothing
/// routing" than doing the edit.
///
/// The ceremony survives the move, and `dry_run` is what carries it. Every
/// other path that touches somebody's agent config works out the change, shows
/// it, and only then writes; the CLI prints the plan and waits for an answer.
/// Reporting `changes` and `occupied` alongside the write is the same
/// information after the fact, which is a receipt rather than a question — so
/// `dry_run: true` works the change out, reports it, and writes nothing, and a
/// client can put the question to somebody before anything moves.
///
/// What the preview does not do is let the caller supply the edit. The request
/// names a tool and a direction, never a file or its contents; the daemon works
/// out both, on both calls. A caller that was shown a plan sends its `digest`
/// back as `as_previewed`, and the write happens only while that is still the
/// edit it describes — the same file, found in the same state, ending in the
/// same state. The direction is in there because the caller picks it again: a
/// half-wired config yields a real connect and a real disconnect from the same
/// bytes, and a client shown the additions must not be able to commit the
/// removal.
///
/// The file is read once more inside the commit, immediately before the write,
/// so an edit that landed while the user was being asked refuses rather than
/// being overwritten. That is a check and then a write rather than one atomic
/// step; the remaining window is documented on
/// [`ironwire_agents::tools::commit_if_unchanged`].
///
/// A slot the user is already using is still reported and still left alone — a
/// GUI is not a licence to take one.
async fn tools(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ToolRequest>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }

    let catalog = state.catalog();
    let planned = if request.connect {
        ironwire_agents::tools::plan_connect(
            &request.id,
            state.config.server.port,
            catalog.current(),
        )
    } else {
        ironwire_agents::tools::plan_disconnect(&request.id, catalog.current())
    };

    let planned = match planned {
        Ok(planned) => planned,
        // `reason` alongside `error` because these are different answers to
        // "what do I do now" — a file only a person can fix, versus a catalog
        // entry no amount of editing the file will make right — and a client
        // that has to tell them apart should not be reading our prose to do it.
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(
                    serde_json::json!({ "error": error.to_string(), "reason": error.reason() }),
                ),
            )
                .into_response();
        }
    };

    if request.connect
        && let Some(login) = plain_url_into_a_login_directory(&state, &request.id, &planned)
    {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": format!(
                    "{} belongs to your {}, which IronWire serves only at the URL \
                     naming it; the plain URL this would write is served by no \
                     subscription. Run `{}` to wire it",
                    planned.path.display(),
                    login.label(),
                    login.connect_command(),
                ),
                "reason": "login-directory",
            })),
        )
            .into_response();
    }

    let digest = planned.digest();
    let confirming = request.as_previewed.is_some() && !request.dry_run;
    if let Some(shown) = &request.as_previewed
        && !request.dry_run
        && shown != &digest
    {
        return conflict(stale_preview(&planned.path));
    }

    let mut backup = None;
    if !request.dry_run && !planned.is_noop() {
        // A confirmed edit looks at the file once more before writing; an
        // unconfirmed one is the endpoint as it always was, and writing what it
        // just planned is all it ever promised.
        let written = if confirming {
            match ironwire_agents::tools::commit_if_unchanged(&planned) {
                Ok(written) => Ok(written),
                Err(ironwire_agents::tools::CommitError::Changed) => {
                    return conflict(stale_preview(&planned.path));
                }
                Err(ironwire_agents::tools::CommitError::Io(error)) => Err(error),
            }
        } else {
            ironwire_agents::tools::commit(&planned)
        };
        match written {
            Ok(written) => backup = written,
            Err(error) => {
                return server_error(format!(
                    "{} could not be written: {error}",
                    planned.path.display()
                ));
            }
        }
    }

    axum::Json(serde_json::json!({
        "ok": true,
        // Whether the file on disk changed. False for a preview, and false for
        // a request that had nothing to do — a tool already wired is not an
        // error, and reporting it as a write would be a lie.
        "applied": !request.dry_run && !planned.is_noop(),
        "path": planned.path.display().to_string(),
        "changes": planned.changes,
        "occupied": planned
            .occupied
            .iter()
            .map(|(slot, current)| serde_json::json!({"slot": slot, "current": current}))
            .collect::<Vec<_>>(),
        // This edit: the file, the state it was found in, and the state it
        // would leave. Send it back as `as_previewed` to commit exactly what
        // this described and nothing else.
        "digest": digest,
        "backup": backup.map(|path| path.display().to_string()),
    }))
    .into_response()
}

/// What can be changed, and what it would take.
async fn settings(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    let statuses = state.backends.statuses().await;
    let consent = state.consent_snapshot();
    axum::Json(settings_view(&state, &statuses, &consent)).into_response()
}

fn settings_view(
    state: &AppState,
    statuses: &[ironwire_upstream::backend::BackendStatus],
    consent: &ConsentLedger,
) -> SettingsView {
    let privacy = state.privacy_config();

    let options = [
        PrivacyMode::Off,
        PrivacyMode::Credentials,
        PrivacyMode::Pii,
        PrivacyMode::Full,
    ]
    .into_iter()
    .map(|mode| {
        // The same rule `Config::validate` applies at startup, asked here so a
        // client can grey the option out instead of offering a switch that
        // would take every backend out of service.
        //
        // Naming a destination is not the same as having one. `trusted_backends`
        // defaults to `["nearai"]`, which is a backend that registers whether or
        // not a key was found — so "named" would be true on a machine where
        // `full` can route precisely nowhere. Selecting it there fails every
        // request as "rate limited", which is both wrong and unactionable, so
        // the usable check is part of the same question.
        let blocked = (mode == PrivacyMode::Full)
            .then(|| full_is_blocked(&privacy, statuses))
            .flatten();
        PrivacyOptionView {
            id: mode_name(mode).to_string(),
            describes: mode.describe().to_string(),
            selectable: blocked.is_none(),
            unavailable_because: blocked,
        }
    })
    .collect();

    let services = statuses
        .iter()
        .map(|status| {
            let requires_consent = status.kind.requires_consent();
            let login = login_of(state, status.id.as_str());
            ServiceView {
                id: status.id.to_string(),
                name: status.name.clone(),
                kind: kind_name(status.kind),
                authenticated: status.authenticated,
                detail: status.detail.clone(),
                requires_consent,
                consented: state.backends.consented(&status.id, consent),
                consent_prompt: consent_prompt(login.as_ref(), status.id.as_str()),
                connect_command: connect_command(login.as_ref(), status.id.as_str()),
            }
        })
        .collect();

    let tools = ironwire_agents::tools::all(state.catalog().current())
        .into_iter()
        .map(|tool| ToolView {
            id: tool.id,
            name: tool.name,
            config_path: tool.config_path.map(|path| path.display().to_string()),
            installed: tool.installed,
            wired: tool.wired,
            connect_command: tool.connect_command,
        })
        .collect();

    SettingsView {
        tools,
        privacy: PrivacySettingsView {
            mode: mode_name(privacy.mode()).to_string(),
            summary: privacy.summary(),
            options,
            trusted_backends: privacy.trusted_backends.clone(),
        },
        services,
    }
}

/// The login a registered backend reads, if it reads one.
fn login_of(state: &AppState, backend_id: &str) -> Option<SubscriptionLogin> {
    state
        .backends
        .get(&BackendId::from(backend_id))
        .and_then(|backend| backend.login())
}

/// The consent question for a backend.
///
/// From the login it reads, so a second Claude login is asked about by its
/// directory — "enable the Claude subscription" does not say which account is
/// being handed over. A default id with no backend behind it (not logged in
/// yet) still gets the default login's question, so consent can be recorded
/// before the login exists, as it always could.
fn consent_prompt(login: Option<&SubscriptionLogin>, backend_id: &str) -> Option<ConsentPrompt> {
    match login {
        Some(login) => Some(ConsentPrompt::for_login(login, backend_id)),
        None => ConsentPrompt::for_backend(backend_id),
    }
}

/// What to run for the parts a GUI has no business doing.
///
/// Pointing Claude Code at IronWire means an environment variable in the user's
/// shell profile; pointing Codex at it means editing their `config.toml`. Both
/// are files the user owns, and `ironwire connect` already shows the change
/// before making it (`docs/TRUST.md`). A menu offering to do it silently would
/// be the wrong end of that trade.
///
/// A subscription's command comes from its login, which is the only thing that
/// knows whether a `--config-dir` belongs in it.
fn connect_command(login: Option<&SubscriptionLogin>, backend_id: &str) -> Option<String> {
    if let Some(login) = login {
        return Some(login.connect_command());
    }
    match backend_id {
        "nearai" => Some("ironwire connect near".to_string()),
        id => SubscriptionProduct::for_default_backend_id(id)
            .map(|product| SubscriptionLogin::default_for(product).connect_command()),
    }
}

/// The value a mode serialises as, matching `PrivacyMode`'s snake_case.
fn mode_name(mode: PrivacyMode) -> &'static str {
    match mode {
        PrivacyMode::Off => "off",
        PrivacyMode::Credentials => "credentials",
        PrivacyMode::Pii => "pii",
        PrivacyMode::Full => "full",
    }
}

fn kind_name(kind: ironwire_core::protocol::BackendKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(ToString::to_string))
        .unwrap_or_else(|| format!("{kind:?}").to_lowercase())
}

/// Change the privacy mode: in the running daemon, and in `config.toml`.
///
/// Both, and in that order. A change that only took effect at the next restart
/// would be a switch that appears to do nothing, and a change that only lived
/// in memory would silently revert the next time the daemon started — the user
/// would be back on a weaker filter without ever being told.
async fn privacy(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<PrivacyRequest>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }

    let Some(mode) = parse_mode(&request.mode) else {
        return bad_request(format!(
            "unknown privacy mode `{}` (try: off, credentials, pii, full)",
            request.mode
        ));
    };

    // The same refusal `Config::validate` makes at startup, and the same one
    // `GET /settings` greys the option out with — literally the same function,
    // because a settings screen that greys an option out while the endpoint
    // behind it accepts the change is worse than either behaviour alone.
    let current = state.privacy_config();
    if mode == PrivacyMode::Full {
        let statuses = state.backends.statuses().await;
        if let Some(reason) = full_is_blocked(&current, &statuses) {
            return bad_request(reason);
        }
    }

    state.set_privacy_mode(mode);

    // Persisted after the switch, and reported separately: the running daemon
    // is already filtering the way the user asked, and a config file we could
    // not write is a smaller problem than pretending nothing happened.
    let persisted = persist_privacy_mode(&state, mode);
    let summary = state.privacy_config().summary();
    match persisted {
        Ok(()) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "ok": true,
                "mode": mode_name(mode),
                "summary": summary,
                "persisted": true,
            })),
        )
            .into_response(),
        Err(detail) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "ok": true,
                "mode": mode_name(mode),
                "summary": summary,
                "persisted": false,
                "warning": format!(
                    "This is in force now, but could not be saved, so it will revert when the \
                     daemon restarts: {detail}"
                ),
            })),
        )
            .into_response(),
    }
}

/// Write the mode into `config.toml`, preserving everything else in it.
fn persist_privacy_mode(state: &AppState, mode: PrivacyMode) -> Result<(), String> {
    let Some(path) = state.config_path() else {
        return Err("this daemon was not started from a config file".to_string());
    };
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("reading {}: {error}", path.display())),
    };
    let edited = ironwire_core::config_edit::set_privacy_mode(&existing, mode)
        .map_err(|error| error.to_string())?;
    ironwire_core::atomic::write(&path, &edited)
        .map_err(|error| format!("writing {}: {error}", path.display()))
}

/// Record or withdraw consent for a subscription backend.
///
/// The daemon reads the consent ledger on every routing decision, so this takes
/// effect on the next request rather than the next restart — unlike
/// `ironwire disconnect --subscription`, which edits the file underneath a
/// running daemon and has to say so.
async fn consent(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<ConsentRequest>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }

    // The question this backend's login is asked — the same one `GET
    // /settings` handed the client — so the version checked below is the
    // version of what the user actually read.
    let login = login_of(&state, &request.backend);
    let Some(prompt) = consent_prompt(login.as_ref(), &request.backend) else {
        let logins: Vec<String> = state
            .backends
            .all()
            .iter()
            .filter(|backend| backend.login().is_some())
            .map(|backend| backend.id().to_string())
            .collect();
        return bad_request(format!(
            "`{}` is not a backend that consent applies to. It applies to subscription logins{}.",
            request.backend,
            if logins.is_empty() {
                ", and none is registered; `ironwire status` lists every backend".to_string()
            } else {
                format!(": {}", logins.join(", "))
            }
        ));
    };

    // Granting is the direction that needs the check. Withdrawing consent is
    // always allowed: a user who wants to stop should never be told that the
    // version of the question they answered was too old to stop with.
    if request.granted && request.prompt_version != prompt.version {
        return bad_request(format!(
            "this asked version {} of the consent question, but the current one is version {}. \
             Re-read it and answer again — a newer question is a different question.",
            request.prompt_version, prompt.version
        ));
    }

    if request.granted && request.binding.as_deref() != Some(prompt.binding.as_str()) {
        return bad_request(
            "the login named by this consent question has changed or was not echoed. \
            Refresh settings and answer the current question again."
                .to_string(),
        );
    }

    let recorded = state.set_consent(&request.backend, request.granted);
    match recorded {
        Ok(()) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "ok": true,
                "backend": request.backend,
                "consented": request.granted,
            })),
        )
            .into_response(),
        Err(detail) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({
                // Deliberately not "ok with a warning" like the privacy write.
                // A consent we failed to record must not be treated as granted.
                "error": format!("could not record that: {detail}"),
            })),
        )
            .into_response(),
    }
}

fn parse_mode(name: &str) -> Option<PrivacyMode> {
    match name {
        "off" => Some(PrivacyMode::Off),
        "credentials" => Some(PrivacyMode::Credentials),
        "pii" => Some(PrivacyMode::Pii),
        "full" => Some(PrivacyMode::Full),
        _ => None,
    }
}

/// A refusal that is ours rather than the caller's — the request was fine and
/// the filesystem was not.
fn server_error(message: String) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        axum::Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

/// What a caller is told when the edit it confirmed is not the edit that would
/// be made now — the file moved, or it asked for a different one.
fn stale_preview(path: &std::path::Path) -> String {
    format!(
        "the edit you were shown is not the edit that would be made to {} now. Preview it again.",
        path.display()
    )
}

/// The plan a caller confirmed is no longer the plan.
fn conflict(message: String) -> Response {
    (
        StatusCode::CONFLICT,
        axum::Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

/// The login whose directory `planned` would leave holding the plain URL,
/// when that login is not the one the plain URL belongs to.
///
/// The tools path writes Claude Code's `~/.claude` and Codex's `CODEX_HOME`,
/// and config can move a login into either — a default relocated there, say.
/// That login is served only at the URL naming it, so the plain one would
/// quietly take its client off its subscription. A file already bound stays
/// bound through a plain connect (`claude_settings::connect`), so only an
/// unbound one is refused.
fn plain_url_into_a_login_directory(
    state: &AppState,
    tool: &str,
    planned: &ironwire_agents::tools::Planned,
) -> Option<SubscriptionLogin> {
    let (product, bound): (_, fn(&str) -> Option<String>) = match tool {
        "claude" => (
            SubscriptionProduct::Claude,
            ironwire_agents::claude_settings::account,
        ),
        "codex" => (
            SubscriptionProduct::ChatGpt,
            ironwire_agents::codex_config::account,
        ),
        _ => return None,
    };
    let dir = planned.path.parent()?;
    let existing = std::fs::read_to_string(&planned.path).unwrap_or_default();
    if bound(&existing).is_some() {
        return None;
    }
    state
        .backends
        .all()
        .iter()
        .filter_map(|backend| backend.login())
        .find(|login| {
            login.product == product
                && login
                    .config_dir
                    .as_deref()
                    .is_some_and(|read| std::path::Path::new(read) == dir)
        })
}

fn bad_request(message: String) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({ "error": message })),
    )
        .into_response()
}

/// Unauthenticated: it reveals nothing and is what a service manager probes.
async fn health() -> Response {
    (StatusCode::OK, "ok").into_response()
}

/// Live route and health events, as SSE.
///
/// What `ironwire watch` and the menu bar app both read. IronWire cannot put a
/// line in a coding agent's transcript — the only writable channel is the
/// response stream itself, and injecting there would put words in the model's
/// mouth — so this side channel is how a user finds out that their family
/// changed (`crate::events`).
///
/// This is the one response in the daemon with no end of its own: a client
/// holds it for as long as it likes, and a quiet system sends nothing down it
/// for hours. So it is also the one that has to watch for the daemon stopping
/// and close itself, or graceful shutdown waits for it forever
/// (`crate::shutdown`).
async fn events(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }

    let mut rx = state.events.subscribe();
    let closing = state.shutdown.clone();
    let stream = async_stream::stream! {
        // A comment frame immediately, so a client knows it is connected before
        // anything has happened — otherwise `ironwire watch` looks hung on a
        // quiet system, which is the normal state.
        yield Ok::<_, std::convert::Infallible>(
            axum::body::Bytes::from_static(b": connected\n\n"),
        );
        loop {
            tokio::select! {
                // Biased, so a shutdown announced in the same breath as an
                // event wins the race: the frame would be written into a
                // connection that is about to go, and leaving is the news.
                biased;
                () = closing.begins() => {
                    // Framing, like `: connected` — both clients read comment
                    // frames as framing, and this one tells them the stream
                    // ended because the daemon stopped rather than because the
                    // connection broke.
                    yield Ok(axum::body::Bytes::from_static(b": closing\n\n"));
                    break;
                }
                received = rx.recv() => match received {
                    Ok(event) => {
                        let payload = serde_json::to_string(&event)
                            .unwrap_or_else(|_| "{}".to_string());
                        yield Ok(axum::body::Bytes::from(format!("data: {payload}\n\n")));
                    }
                    // The bus is lossy on purpose: a subscriber that fell behind is
                    // told so rather than silently shown a gap.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        yield Ok(axum::body::Bytes::from(format!(
                            ": lagged {n}\n\n"
                        )));
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
            }
        }
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/event-stream")
        .header("cache-control", "no-cache")
        .header("x-accel-buffering", "no")
        .body(axum::body::Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    let now = chrono::Utc::now();
    let statuses = state.backends.statuses().await;
    let consent = state.consent_snapshot();

    let health = state.breakers.statuses();
    let health_by_id: std::collections::HashMap<_, _> =
        health.iter().map(|h| (&h.backend, h)).collect();
    let backends: Vec<BackendView> = statuses
        .iter()
        .map(|s| BackendView {
            id: s.id.to_string(),
            name: s.name.clone(),
            // Serde's `snake_case` rename, not `Debug` lowercased: the latter
            // renders `ApiKey` as `apikey`, which `status` then prints as the
            // backend's kind. Nobody saw it until an API-key backend could
            // actually be configured.
            kind: serde_json::to_value(s.kind)
                .ok()
                .and_then(|value| value.as_str().map(ToString::to_string))
                .unwrap_or_else(|| format!("{:?}", s.kind).to_lowercase()),
            authenticated: s.authenticated,
            consented: state.backends.consented(&s.id, &consent),
            detail: s.detail.clone(),
            headroom: HeadroomView::from(&s.quota.primary, now),
            health: health_by_id
                .get(&s.id)
                .map_or_else(HealthView::default, |h| HealthView {
                    circuit: format!("{:?}", h.state).to_lowercase(),
                    consecutive_failures: h.consecutive_failures,
                    retry_in_secs: h.retry_at.map(|at| (at - now).num_seconds().max(0)),
                }),
            models: s.models.iter().map(|(m, _)| m.clone()).collect(),
            connect_command: connect_command(
                login_of(&state, s.id.as_str()).as_ref(),
                s.id.as_str(),
            ),
        })
        .collect();

    let balance = balance(
        &statuses,
        &|id| state.backends.consented(id, &consent),
        &health,
        state.ledger.as_ref(),
        &state.config.limits,
        now,
    );

    let (tracked, pin, cross_account_fallback) = {
        let policy = match state.policy.lock() {
            Ok(p) => p,
            Err(poisoned) => poisoned.into_inner(),
        };
        (
            policy.tracked_conversations(),
            policy.pin().map(|(b, m)| match m {
                Some(model) => format!("{b}:{model}"),
                None => b.to_string(),
            }),
            policy.cross_account_fallback(),
        )
    };

    axum::Json(StatusView {
        version: env!("CARGO_PKG_VERSION").to_string(),
        port: state.port,
        tracked_conversations: tracked,
        pin,
        backends,
        balance,
        privacy: state.privacy().map(|filter| filter.summary().to_string()),
        catalog_serial: state.catalog().serial(),
        update: state.update_status(),
        last_route: state.last_route().map(|route| LastRouteView {
            backend: route.backend,
            model: route.model,
            from: route.from,
            rung: route.rung,
            at: route.at,
        }),
        usage: usage(&state, now),
        cross_account_fallback,
    })
    .into_response()
}

/// Measure IronWire's own traffic from the ledger.
///
/// A ledger read failure yields an empty report rather than a failed status
/// call: the capacity numbers above are the reason someone ran this command,
/// and losing them because a burn-rate query could not run would be the wrong
/// trade every time.
/// Memoised by [`AppState::usage_report`], keyed on the ledger's write token:
/// `ironwire statusline` calls this endpoint on every render of somebody's
/// editor, and the scan behind it reads eight days of ledger rows. The token
/// is what keeps that from ever showing a report built before the traffic the
/// caller is asking about.
fn usage(state: &AppState, now: DateTime<Utc>) -> UsageReport {
    let config = &state.config.usage;
    if !config.enabled {
        return UsageReport::default();
    }
    let Some(ledger) = state.ledger.as_ref() else {
        return UsageReport::default();
    };

    state.usage_report(now, ledger.writes(), || build_usage(config, ledger, now))
}

fn build_usage(
    config: &ironwire_core::config::UsageConfig,
    ledger: &ironwire_ledger::Ledger,
    now: DateTime<Utc>,
) -> UsageReport {
    let options = ironwire_usage::Options {
        session_hours: i64::from(config.session_hours.max(1)),
        history_hours: i64::from(config.history_hours.max(1)),
        // An unparseable plan is dropped rather than guessed at, and said out
        // loud: silently falling back to a default would put a limit on the
        // user's screen that they never declared.
        plan: config.plan.as_deref().and_then(|name| {
            let parsed = ironwire_usage::Plan::parse(name);
            if parsed.is_none() {
                tracing::warn!(
                    plan = name,
                    "unknown usage.plan; comparing against your own history instead. \
                     Known plans: pro, max5, max20, team"
                );
            }
            parsed
        }),
        ..ironwire_usage::Options::default()
    };

    match ledger.since(now - options.history()) {
        Ok(exchanges) => ironwire_usage::report(&exchanges, now, &options),
        Err(error) => {
            tracing::debug!(%error, "could not read the ledger for usage estimates");
            UsageReport::default()
        }
    }
}

/// Collapse every pool into one balance — see [`BalanceView`] for why this
/// counts pools instead of summing them.
fn balance(
    statuses: &[ironwire_upstream::backend::BackendStatus],
    consented: &dyn Fn(&BackendId) -> bool,
    health: &[ironwire_upstream::breaker::BreakerStatus],
    ledger: Option<&ironwire_ledger::Ledger>,
    limits: &ironwire_core::config::LimitsConfig,
    now: DateTime<Utc>,
) -> BalanceView {
    use ironwire_upstream::breaker::CircuitState;

    // Local midnight, the same window a spend cap is measured over. A rolling
    // 24 hours and a calendar day disagreeing about "today" on the one screen
    // that reports both would be indefensible.
    let summary = ledger.and_then(|l| l.summary(crate::spend::window_start(now)).ok());
    let health_by_id: std::collections::HashMap<_, _> =
        health.iter().map(|h| (&h.backend, h)).collect();
    let exchanges_by_id: std::collections::HashMap<_, _> = summary
        .as_ref()
        .map(|sum| {
            sum.by_backend
                .iter()
                .map(|(id, count)| (id.as_str(), *count))
                .collect()
        })
        .unwrap_or_default();
    // Which backend is metered is knowable only here, where the registry is —
    // the ledger stores an id, not a kind. Everything else is priced but not
    // billed, and must not be added up as though it were.
    let metered: std::collections::HashSet<&str> = statuses
        .iter()
        .filter(|s| s.kind.is_metered())
        .map(|s| s.id.as_str())
        .collect();
    let mut view = BalanceView {
        spend_today_usd: summary.as_ref().map(|s| {
            s.cost_by_backend
                .iter()
                .filter(|(backend, _)| metered.contains(backend.as_str()))
                .map(|(_, cost)| cost)
                .sum()
        }),
        cache_hit_rate: summary.as_ref().and_then(|s| s.cache_hit_rate),
        cache_exchanges: summary.as_ref().map_or(0, |s| s.exchanges),
        spend_cap: limits
            .daily_spend_usd
            .filter(|cap| *cap > 0.0)
            .map(|cap| SpendCapView {
                spent_usd: summary.as_ref().map_or(0.0, |s| {
                    s.cost_by_backend
                        .iter()
                        .filter(|(backend, _)| metered.contains(backend.as_str()))
                        .map(|(_, cost)| cost)
                        .sum()
                }),
                cap_usd: cap,
            }),
        subscription_used: statuses
            .iter()
            .filter(|s| s.kind.requires_consent() && s.authenticated && consented(&s.id))
            .map(|s| SubscriptionUse {
                backend: s.id.to_string(),
                name: s.name.clone(),
                used_pct: match s.quota.primary {
                    Headroom::Observed { used_pct, .. } => Some(used_pct),
                    _ => None,
                },
                exchanges: exchanges_by_id.get(s.id.as_str()).copied().unwrap_or(0),
            })
            .collect(),
        ..BalanceView::default()
    };
    let mut resets: Vec<DateTime<Utc>> = Vec::new();

    for status in statuses {
        // A backend the user has not logged into, or not consented to, is not a
        // pool that is *down* — it is one they have not opted into. Counting it
        // as unavailable would make a working setup look half-broken.
        if !status.authenticated || !consented(&status.id) {
            continue;
        }

        let circuit_open = health_by_id
            .get(&status.id)
            .is_some_and(|h| h.state == CircuitState::Open);
        if circuit_open {
            view.unavailable += 1;
            if let Some(at) = health_by_id.get(&status.id).and_then(|h| h.retry_at) {
                resets.push(at);
            }
            continue;
        }

        match status.quota.primary {
            Headroom::Exhausted { until } => {
                view.unavailable += 1;
                resets.push(until);
            }
            // A cap is not "no capacity left" — it is capacity the user
            // declined to spend, so it counts as unavailable rather than
            // unknown, and `next_available_at` points at the rollover.
            Headroom::CapReached { resets_at, .. } => {
                view.unavailable += 1;
                resets.push(resets_at);
            }
            Headroom::Unknown => view.unknown += 1,
            Headroom::Observed { .. } => {
                view.available += 1;
                if !status.kind.is_metered() {
                    view.free_available += 1;
                }
            }
        }
    }

    view.next_available_at = resets.into_iter().min();
    view
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionBindingRequest {
    session_id: String,
    backend: String,
    binding: String,
    confirmed: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionBindingRevocation {
    session_id: String,
}

async fn admission_binding_capability(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    if let Some(session) = query.get("session_id") {
        let bindings = state
            .admission_bindings
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        return match bindings.status(session, Utc::now().timestamp()) {
            Ok((status, expires_at)) => axum::Json(serde_json::json!({"status":status,"active":status=="active","expires_at":expires_at})).into_response(),
            Err(_) => (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"error":"admission-binding-invalid"}))).into_response(),
        };
    }
    axum::Json(serde_json::json!({
        "supported":true, "max_lifetime_seconds":ironwire_core::admission::MAX_BINDING_SECONDS,
        "max_sessions":ironwire_core::admission::MAX_BINDINGS, "protocol":"openai.chat",
        "body_capture_ready":state.config.capture.enabled && state.config.capture.bodies
            && state.ledger.is_some() && state.bodies.is_some()
    }))
    .into_response()
}

async fn admission_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<AdmissionBindingRequest>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    let supported = state
        .backends
        .get(&BackendId::from(request.backend.as_str()))
        .is_some_and(|backend| {
            backend.capabilities().wires.primary() == ironwire_core::protocol::Protocol::OpenAiChat
                && backend.kind() != ironwire_core::protocol::BackendKind::Subscription
                && backend.required_client_identity().is_none()
        });
    if !supported {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error":"admission-binding-backend-unsupported"})),
        )
            .into_response();
    }
    let mut bindings = state
        .admission_bindings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match bindings.register(
        &request.session_id,
        &request.backend,
        &request.binding,
        request.confirmed,
        Utc::now().timestamp(),
    ) {
        Ok(expires_at) => {
            axum::Json(serde_json::json!({"active":true,"expires_at":expires_at})).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn revoke_admission_binding(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<AdmissionBindingRevocation>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    let mut bindings = state
        .admission_bindings
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    match bindings.revoke(&request.session_id) {
        Ok(removed) => {
            axum::Json(serde_json::json!({"active":false,"removed":removed})).into_response()
        }
        Err(error) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error":error.to_string()})),
        )
            .into_response(),
    }
}

async fn pin(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<PinRequest>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    // Validate before storing. An unknown backend used to be accepted, and then
    // every request silently ignored the pin and routed normally — while
    // `ironwire status` reported "Pinned to <whatever>". The user believes all
    // their traffic is on one backend and it is not, which is the same failure
    // `X-IronWire-Route` was fixed for, made worse by persisting.
    //
    // Rejected here rather than per-request, because here is where the backend
    // list is visible and the answer can name what actually exists.
    if let Some(requested) = &request.backend {
        let known: Vec<String> = state
            .backends
            .all()
            .iter()
            .map(|backend| backend.id().to_string())
            .collect();
        if !known.iter().any(|id| id == requested) {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({
                    "error": format!("`{requested}` is not a connected backend"),
                    "available": known,
                })),
            )
                .into_response();
        }
    }

    let mut policy = match state.policy.lock() {
        Ok(p) => p,
        Err(poisoned) => poisoned.into_inner(),
    };
    policy.set_pin(
        request.backend.as_deref().map(BackendId::from),
        request.model,
    );
    (StatusCode::OK, axum::Json(serde_json::json!({"ok": true}))).into_response()
}

/// Hit every backend for real. This is what makes `ironwire doctor` worth
/// running: a credential that parses proves nothing.
///
/// Every backend the user has allowed, that is. A subscription awaiting consent
/// is reported unprobed with the command that grants it: the probe would send
/// its OAuth token, and running `doctor` is not saying yes (`docs/TRUST.md`
/// §2).
async fn probe(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    let consent = state.consent_snapshot();
    let mut checks = Vec::new();
    for backend in state.backends.all().iter().cloned() {
        let state = &state;
        let consent = &consent;
        checks.push(async move {
            if !crate::state::consents_to(consent, backend.as_ref()) {
                let command = connect_command(backend.login().as_ref(), backend.id().as_str());
                let run = command.as_deref().map_or_else(
                    || "grant it consent".to_string(),
                    |command| format!("run `{command} --subscription`"),
                );
                return ProbeView {
                    id: backend.id().to_string(),
                    name: backend.name().to_string(),
                    ok: false,
                    latency_ms: 0,
                    error: Some(format!(
                        "not probed: awaiting your consent to use this subscription — {run}"
                    )),
                    probed: false,
                    awaiting_consent: true,
                    connect_command: command,
                };
            }
            let id = backend.id().to_string();
            let name = backend.name().to_string();
            let started = std::time::Instant::now();
            let outcome = state.backends.probe(backend).await;
            ProbeView {
                id,
                name,
                ok: outcome.is_ok(),
                latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                error: outcome.err().map(|e| e.to_string()),
                probed: true,
                awaiting_consent: false,
                connect_command: None,
            }
        });
    }
    let views: Vec<ProbeView> = stream::iter(checks)
        .buffered(crate::state::BACKEND_CHECK_CONCURRENCY)
        .collect()
        .await;
    axum::Json(views).into_response()
}

async fn log(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<LogQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    let Some(ledger) = state.ledger.as_ref() else {
        return axum::Json(LogView {
            enabled: false,
            exchanges: Vec::new(),
            last_24h: Summary::default(),
        })
        .into_response();
    };

    let limit = query.limit.min(1000);
    let exchanges = match query.since {
        // Bounded by SQLite, not by truncating afterwards: an old cutoff would
        // otherwise read the whole ledger into memory under the mutex before
        // discarding almost all of it, blocking writes for the length of the
        // scan. `after_id` is what lets the caller advance -- see `LogQuery`.
        Some(from) => ledger.page(from, query.after_id, limit).unwrap_or_default(),
        None => ledger.recent(limit).unwrap_or_default(),
    };
    let last_24h = ledger
        .summary(chrono::Utc::now() - chrono::Duration::hours(24))
        .unwrap_or_default();
    axum::Json(LogView {
        enabled: true,
        exchanges,
        last_24h,
    })
    .into_response()
}

async fn summary(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<SummaryQuery>,
) -> Response {
    if let Err(response) = authorize(&state, &headers) {
        return *response;
    }
    let since = query
        .since
        .unwrap_or_else(|| ironwire_ledger::proof::default_window_start(chrono::Utc::now()));
    let receipts = state.config.capture.receipts;
    let Some(ledger) = state.ledger.as_ref() else {
        return axum::Json(SummaryView {
            enabled: false,
            receipts,
            since,
            summary: ironwire_ledger::ProofSummary::default(),
        })
        .into_response();
    };
    match ledger.proof_summary(since) {
        Ok(summary) => axum::Json(SummaryView {
            enabled: true,
            receipts,
            since,
            summary,
        })
        .into_response(),
        // A failed read is not an empty window. Saying "no calls" here would
        // be an invented number.
        Err(error) => server_error(error.to_string()),
    }
}

/// Constant-time-ish token check. The token is a local file, so this is a guard
/// against another *user* on the box, not against a network attacker.
fn authorize(state: &AppState, headers: &HeaderMap) -> Result<(), Box<Response>> {
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or_default();
    if presented.as_bytes().ct_eq(state.control_token.as_bytes()) {
        return Ok(());
    }
    Err(Box::new(
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({
                "error": "control API requires the token from $IRONWIRE_HOME/control.token"
            })),
        )
            .into_response(),
    ))
}

/// Length-independent byte comparison, so a wrong token's *length* is not a
/// side channel either.
trait ConstantTimeEq {
    fn ct_eq(&self, other: &[u8]) -> bool;
}

impl ConstantTimeEq for [u8] {
    fn ct_eq(&self, other: &[u8]) -> bool {
        if self.len() != other.len() {
            return false;
        }
        self.iter()
            .zip(other)
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    #[test]
    fn an_unobserved_backend_reports_unknown_not_a_number() {
        let view = HeadroomView::from(&Headroom::Unknown, Utc::now());
        assert!(matches!(view, HeadroomView::Unknown));
    }

    #[test]
    fn an_observation_carries_its_own_age() {
        let now = Utc::now();
        let view = HeadroomView::from(
            &Headroom::Observed {
                used_pct: 82.0,
                resets_at: Some(now + Duration::minutes(30)),
                observed_at: now - Duration::seconds(40),
            },
            now,
        );
        match view {
            HeadroomView::Observed {
                used_pct,
                observed_secs_ago,
                resets_in_secs,
            } => {
                assert!((used_pct - 82.0).abs() < 0.01);
                assert_eq!(observed_secs_ago, 40);
                assert_eq!(resets_in_secs, Some(1800));
            }
            other => panic!("expected an observation, got {other:?}"),
        }
    }

    #[test]
    fn an_elapsed_retry_window_never_reports_negative_time() {
        let now = Utc::now();
        let view = HeadroomView::from(
            &Headroom::Exhausted {
                until: now - Duration::seconds(10),
            },
            now,
        );
        match view {
            HeadroomView::Exhausted { retry_in_secs } => assert_eq!(retry_in_secs, 0),
            other => panic!("expected exhaustion, got {other:?}"),
        }
    }

    #[test]
    fn token_comparison_rejects_wrong_length_and_wrong_bytes() {
        assert!(b"abcdef".ct_eq(b"abcdef"));
        assert!(!b"abcdef".ct_eq(b"abcdeg"));
        assert!(!b"abcdef".ct_eq(b"abcde"));
        assert!(!b"".ct_eq(b"x"));
    }
}
