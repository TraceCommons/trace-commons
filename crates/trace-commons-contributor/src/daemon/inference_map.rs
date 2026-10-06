//! The map and the Inference tab (K8 of #1118): where each tool's sessions
//! and model calls go, and the model calls this machine's proxy recorded.
//!
//! Two methods, both local and synchronous, both read from state this daemon
//! already holds:
//!
//! - `tool_destinations` -- per tool, the parties its sessions go to and the
//!   party that answers its model calls.
//! - `inference_calls` -- one page of the calls in the local routing ledger,
//!   newest first, with the route, the priced cost and IronWire's proof label.
//!
//! # The proof is IronWire's, read verbatim
//!
//! Each ledger row carries IronWire's own proof label
//! ([`crate::routing::ProofStatus`]): the stored verdict of its receipt check
//! against a measurement-pinned quote. This module passes that label through
//! unchanged. It never re-derives, re-checks, collapses or promotes it, reads
//! no body, fetches no receipt and makes no network call. A row with no label
//! is `unrecorded`, which is not `outside` and not `pending`.
//!
//! # What crosses the socket
//!
//! Fixed labels, counts, times and the ledger's own row id -- and one free
//! text field, `model`: the model name as the proxy recorded it, up to 128
//! characters of the characters a model id uses, else `unknown`. Never a
//! prompt, a response, a body reference, a body digest, the provider's
//! exchange identifier, a session id, a token or a URL. Nothing here is
//! logged.
//!
//! # Every unknown is `unknown`
//!
//! A tool whose config names somebody else's port, a proxy that is running
//! but cannot say which account answers, a facade this build does not know,
//! a row with no proof label: each reads as unknown, never the nearest guess.
//!
//! # What the ledger cannot say
//!
//! No kind of work (no classifier is invented here); no tool id (a call is
//! attributed from the endpoint it called, which names the one tool whose
//! connection writes that endpoint, and on a proxy too old to record the
//! endpoint only when exactly one tool connected now speaks its protocol
//! family -- see [`attribute`]); no billed cost (the ledger prices every
//! call); and no calls that never passed through the proxy. See the IPC
//! document.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use super::harness::{ACTIVITY_WINDOW_HOURS, HarnessRow};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, Request, Response};
use super::policy::ProjectMode;
use super::settings::SourceDeclaration;
use crate::routing::{ProofStatus, RoutedExchange};

/// Rows per page when a caller names no limit.
pub const DEFAULT_CALLS_LIMIT: usize = 50;
/// The most rows one page may carry. A larger ask is refused, not capped, so
/// a caller that pages by the size it asked for never skips rows.
pub const MAX_CALLS_LIMIT: usize = 200;
/// The longest model name passed through as recorded.
const MAX_MODEL_LABEL_CHARS: usize = 128;

pub const PARTY_COMMONS: &str = "commons";
pub const PARTY_WITNESS: &str = "witness";
pub const PARTY_NEAR_AI: &str = "near_ai";
pub const UNKNOWN: &str = "unknown";

/// IronWire recorded any proof label but `outside`: a NEAR AI backend.
pub const ROUTE_ROUTED: &str = "routed";
/// IronWire recorded `outside`: not a NEAR AI backend.
pub const ROUTE_OUTSIDE: &str = "outside";

/// The proof label for a row IronWire recorded none for.
pub const PROOF_UNRECORDED: &str = "unrecorded";

/// Private AI routes the tool, and the window's calls in its family were all
/// routed.
pub const BASIS_OBSERVED: &str = "observed";
/// Private AI routes the tool by configuration, and the window has no labelled
/// call in its family to confirm it. A shell must not draw it as confirmed.
pub const BASIS_CONFIGURED: &str = "configured";
pub const BASIS_TOOL_DEFAULT: &str = "tool_default";
pub const BASIS_ANSWERED_ELSEWHERE: &str = "answered_elsewhere";

pub const ERR_LIMIT_INVALID: &str = "limit-invalid";
pub const ERR_CURSOR_INVALID: &str = "cursor-invalid";

/// One tool the map draws.
struct ToolSpec {
    source: &'static str,
    harness: Option<&'static str>,
    /// The protocol family the ledger records this tool's calls under.
    family: Option<&'static str>,
    /// The `(facade, path)` endpoints the connection IronWire writes for
    /// this tool sends its calls to. Claude Code is pointed at
    /// `/anthropic` and speaks Messages; Codex is pointed at `/openai/v1`
    /// with `wire_api = "responses"`. Empty for a tool nothing here
    /// connects.
    ///
    /// Claude Code's `/v1/messages/count_tokens` is listed on purpose: a
    /// token count is a call the tool made through the proxy, so it is
    /// counted in `counts.inference_calls`, listed by `inference_calls` and
    /// announced by `inference_call_added` like any other. Leaving it out
    /// would not drop those rows -- it would move them to
    /// `unattributed_calls`.
    endpoints: &'static [(&'static str, &'static str)],
    name: &'static str,
}

/// The tools the map draws.
///
/// No row for Antigravity. It has no adapter (an imported conversation is
/// staged as a trajectory file and read by the `trajectory` adapter), no
/// watch declaration and no harness this daemon connects, so a row would
/// read `not_watched` while counting sessions, which is a contradiction the
/// `watch` labels cannot express without a new value -- a wire change, not
/// a counting fix. Its sessions are counted under their declared source
/// (see [`session_marks`]), so they count nowhere here today, and a future
/// row picks them up once each, never alongside a `trajectory` count.
const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        source: crate::source::SOURCE_CLAUDE_CODE,
        harness: Some("claude"),
        family: Some("anthropic"),
        endpoints: &[
            ("anthropic", "/v1/messages"),
            ("anthropic", "/v1/messages/count_tokens"),
        ],
        name: "Claude Code",
    },
    ToolSpec {
        source: crate::source::SOURCE_CODEX,
        harness: Some("codex"),
        family: Some("openai"),
        endpoints: &[("openai", "/v1/responses")],
        name: "Codex",
    },
    ToolSpec {
        source: crate::source::SOURCE_GEMINI_CLI,
        harness: None,
        family: None,
        endpoints: &[],
        name: "Gemini CLI",
    },
    ToolSpec {
        source: crate::source::SOURCE_CLINE,
        harness: None,
        family: None,
        endpoints: &[],
        name: "Cline",
    },
    ToolSpec {
        source: crate::source::SOURCE_OPENCODE,
        harness: None,
        family: None,
        endpoints: &[],
        name: "OpenCode",
    },
];

/// Where a tool answers when nothing here routes it: its default provider,
/// a fixed label, `unknown` for a tool that talks to many providers.
///
/// K2's `source::vendor_label` (#1130) is now this table's source of truth
/// for WHETHER a source has a fixed default -- `source_default_family`
/// names the family, `vendor_label` confirms it is one this daemon has a
/// display word for, and `None` at either step reads `UNKNOWN`, replacing
/// what used to be a local per-source table here.
///
/// The wire label itself stays the *family's own* lowercase spelling
/// (`"anthropic"`, `"openai"`, `"google"`), not `vendor_label`'s returned
/// word (`"Anthropic"`, `"OpenAI"`, `"Google"`), which is capitalized for
/// display (the "answers at" wording) and would change what
/// `model_calls.to` carries -- a wire format change, not a plumbing change.
/// `docs/contributor-daemon-ipc-v1_1.md` and this module's own tests both
/// pin the lowercase word, so this function keeps it.
fn vendor(source: &str) -> &'static str {
    crate::source::source_default_family(source)
        .filter(|family| crate::source::vendor_label(family).is_some())
        .unwrap_or(UNKNOWN)
}

/// The source id a harness row is the same tool as, or `None`.
fn source_for_harness(harness_id: &str) -> Option<&'static str> {
    TOOLS
        .iter()
        .find(|spec| spec.harness == Some(harness_id))
        .map(|spec| spec.source)
}

/// What the harness list says about one tool's config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HarnessLink {
    /// The config names this daemon's destination port.
    pub connected: bool,
    /// The config names *a* local proxy, ours or not.
    pub wired: bool,
}

/// What the window says about one protocol family's calls.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Observed {
    /// Calls IronWire labelled with anything but `outside`.
    pub routed: usize,
    /// Calls IronWire labelled `outside`.
    pub outside: usize,
}

/// Who answers one tool's model calls, and on what basis.
///
/// `private_ai` is the label `private_inference_state.state` carries.
/// `running` says NEAR AI reports a credential, not that this tool's family
/// is answered there -- the proxy can hold other backends. So a connected
/// tool reads `near_ai` on `observed` only when the window's labelled calls
/// in its family were all routed, and on `configured` when there are none to
/// look at. One outside call in the family makes it `unknown`.
/// `running_answered_elsewhere` is the proxy forwarding with the tool's own
/// credentials, so the vendor answers. Every other state with a connected
/// config is `unknown`.
#[must_use]
pub fn model_calls_to(
    vendor: &'static str,
    harness: Option<HarnessLink>,
    private_ai: &str,
    observed: Observed,
) -> (&'static str, &'static str) {
    match harness {
        Some(link) if link.connected => match private_ai {
            super::private_inference::LABEL_RUNNING => match observed {
                Observed {
                    outside: 0,
                    routed: 0,
                } => (PARTY_NEAR_AI, BASIS_CONFIGURED),
                Observed { outside: 0, .. } => (PARTY_NEAR_AI, BASIS_OBSERVED),
                _ => (UNKNOWN, UNKNOWN),
            },
            super::private_inference::LABEL_RUNNING_ANSWERED_ELSEWHERE => {
                (vendor, BASIS_ANSWERED_ELSEWHERE)
            }
            _ => (UNKNOWN, UNKNOWN),
        },
        // Pointed at a local proxy that is not ours: whoever runs it decides.
        Some(link) if link.wired => (UNKNOWN, UNKNOWN),
        _ => (vendor, BASIS_TOOL_DEFAULT),
    }
}

/// Count the window's calls in one protocol family by IronWire's label.
///
/// A row with no label is counted nowhere: it is evidence of neither.
#[must_use]
pub fn observe(rows: &[RoutedExchange], family: &str) -> Observed {
    rows.iter()
        .filter(|row| row.facade == family)
        .fold(Observed::default(), |mut seen, row| {
            match route_label(row) {
                ROUTE_ROUTED => seen.routed += 1,
                ROUTE_OUTSIDE => seen.outside += 1,
                _ => {}
            }
            seen
        })
}

/// The parties one watched tool's sessions go to, by the route in force.
///
/// The same `route` `route_disclosure` reports, so the map can never draw a
/// witness that disclosure does not name. A route that sends nothing sends
/// nothing to anyone.
#[must_use]
pub fn session_parties(route: crate::disclosure::Route) -> Vec<&'static str> {
    use crate::disclosure::Route;
    match route {
        Route::Witness => vec![PARTY_COMMONS, PARTY_WITNESS],
        Route::Local => vec![PARTY_COMMONS],
        Route::WitnessRefusing | Route::NotEnrolled | Route::SettingsUnreadable => Vec::new(),
    }
}

/// Whether a tool's sessions are watched, as a label.
fn watch_label(watched: bool, declaration: Option<&SourceDeclaration>) -> &'static str {
    if watched {
        "watched"
    } else if matches!(declaration, Some(SourceDeclaration::Off)) {
        "off"
    } else {
        "not_watched"
    }
}

/// The inputs `tool_destinations` is computed from, gathered once.
pub struct DestinationFacts {
    pub route: crate::disclosure::Route,
    pub private_ai: &'static str,
    /// Source ids an adapter is actually built for right now.
    pub watched: BTreeSet<&'static str>,
    pub declarations: Vec<(&'static str, Option<SourceDeclaration>)>,
    pub harness: Vec<(String, HarnessLink)>,
    /// The ledger rows for the window, or `None` when no ledger answered --
    /// which is not evidence of no calls, and is why each call count reads
    /// `null` rather than `0` then.
    pub rows: Option<Vec<RoutedExchange>>,
    /// The sessions seen in the window, from the queue and the history
    /// cache, or `None` when the history could not be read: a count from
    /// the queue alone would undercount and look like a fact.
    pub sessions: Option<Vec<SessionMark>>,
    pub folders: (usize, usize, usize),
}

/// One session the window saw: the tool it reads as (see [`session_marks`])
/// and its hash.
///
/// Held only long enough to count; the hash never leaves this module.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionMark {
    pub source: String,
    pub session_hash: String,
}

/// The connected tools that speak a protocol family, as `(source, family)`.
///
/// The input to [`attribute`]. Built from the harness rows, never from a
/// tool's name: a tool is a speaker only while its config names this
/// daemon's port.
pub type Speakers = Vec<(&'static str, &'static str)>;

/// The speakers among a set of harness links, by [`TOOLS`]' families.
fn speakers_from_links(harness: &[(String, HarnessLink)]) -> Speakers {
    harness
        .iter()
        .filter(|(_, link)| link.connected)
        .filter_map(|(id, _)| {
            let spec = TOOLS
                .iter()
                .find(|spec| spec.harness == Some(id.as_str()))?;
            Some((spec.source, spec.family?))
        })
        .collect()
}

/// The speakers among the harness rows `harness_list` computes.
fn speakers_from_rows(harness: &[HarnessRow]) -> Speakers {
    harness
        .iter()
        .filter(|row| row.connected)
        .filter_map(|row| Some((source_for_harness(&row.id)?, row.family?)))
        .collect()
}

/// Per tool, the sessions and the listed calls in the window.
///
/// `calls` counts exactly the rows `inference_calls` lists -- rows with a
/// ledger id -- by the tool that list names, so the map and the Inference
/// tab can never disagree. A call no tool can be named for is counted in the
/// second value, never folded into a tool. Sessions are counted once per
/// session hash, so an entry still in the queue and its history record are
/// one session. Either side is `None` when its source could not answer.
#[must_use]
pub fn tool_counts(
    rows: Option<&[RoutedExchange]>,
    speakers: &[(&'static str, &'static str)],
    sessions: Option<&[SessionMark]>,
) -> (Vec<ToolCount>, Option<usize>) {
    let mut unattributed = rows.map(|_| 0usize);
    let mut counts: Vec<ToolCount> = TOOLS
        .iter()
        .map(|spec| ToolCount {
            tool: spec.source,
            sessions: sessions.map(|_| 0),
            inference_calls: rows.map(|_| 0),
        })
        .collect();
    for row in rows.unwrap_or(&[]).iter().filter(|row| row.id.is_some()) {
        let tool = attribute(row, speakers);
        match counts.iter_mut().find(|count| count.tool == tool) {
            Some(count) => *count.inference_calls.get_or_insert(0) += 1,
            None => *unattributed.get_or_insert(0) += 1,
        }
    }
    if let Some(sessions) = sessions {
        // Once per session hash, whatever its marks are labelled: the first
        // mark for a hash names it (the queue's, which [`session_marks`]
        // puts first), so one session can never count under two tools.
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for mark in sessions {
            if !seen.insert(mark.session_hash.as_str()) {
                continue;
            }
            if let Some(count) = counts.iter_mut().find(|count| count.tool == mark.source) {
                *count.sessions.get_or_insert(0) += 1;
            }
        }
    }
    (counts, unattributed)
}

/// One tool's counts for the window. `None` is "could not be read", never 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolCount {
    pub tool: &'static str,
    pub sessions: Option<usize>,
    pub inference_calls: Option<usize>,
}

/// The `tool_destinations` result for a set of facts.
#[must_use]
pub fn destinations(facts: &DestinationFacts) -> serde_json::Value {
    let parties = session_parties(facts.route);
    let rows = facts.rows.as_deref().unwrap_or(&[]);
    let (counts, unattributed) = tool_counts(
        facts.rows.as_deref(),
        &speakers_from_links(&facts.harness),
        facts.sessions.as_deref(),
    );
    let tools: Vec<serde_json::Value> = TOOLS
        .iter()
        .map(|spec| {
            let declaration = facts
                .declarations
                .iter()
                .find(|(name, _)| *name == spec.source)
                .and_then(|(_, d)| d.as_ref());
            let watched = facts.watched.contains(spec.source);
            let link = spec.harness.and_then(|id| {
                facts
                    .harness
                    .iter()
                    .find(|(row, _)| row == id)
                    .map(|(_, link)| *link)
            });
            let observed = spec
                .family
                .map_or(Observed::default(), |family| observe(rows, family));
            let (to, basis) = model_calls_to(vendor(spec.source), link, facts.private_ai, observed);
            let count = counts
                .iter()
                .find(|count| count.tool == spec.source)
                .copied()
                .unwrap_or(ToolCount {
                    tool: spec.source,
                    sessions: None,
                    inference_calls: None,
                });
            serde_json::json!({
                "tool": spec.source,
                "name": spec.name,
                "sessions": {
                    "watch": watch_label(watched, declaration),
                    "to": if watched { parties.clone() } else { Vec::new() },
                },
                "model_calls": { "to": to, "basis": basis },
                "counts": {
                    "sessions": count.sessions,
                    "inference_calls": count.inference_calls,
                },
            })
        })
        .collect();
    serde_json::json!({
        "private_ai": facts.private_ai,
        "sessions_route": facts.route,
        "window_hours": ACTIVITY_WINDOW_HOURS,
        "unattributed_calls": unattributed,
        "folders": {
            "armed": facts.folders.0,
            "ask_first": facts.folders.1,
            "ignored": facts.folders.2,
        },
        "tools": tools,
    })
}

/// `tool_destinations`.
pub fn handle_destinations(shared: &DaemonShared, req: &Request) -> Response {
    let (roots, declarations, attested_bodies) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.source_roots(&shared.store),
            vec![
                (crate::source::SOURCE_CLAUDE_CODE, s.claude_source.clone()),
                (crate::source::SOURCE_CODEX, s.codex_source.clone()),
                (crate::source::SOURCE_GEMINI_CLI, s.gemini_source.clone()),
                (crate::source::SOURCE_CLINE, s.cline_source.clone()),
                (crate::source::SOURCE_OPENCODE, s.opencode_source.clone()),
            ],
            s.ironwire_attested_bodies,
        )
    };
    // What is actually built, not what was declared: an undeclared Claude
    // Code or Codex still falls back to its conventional folder, and the map
    // must draw that.
    let watched: BTreeSet<&'static str> = crate::source::all_sources(&roots)
        .iter()
        .map(|source| source.name())
        .collect();
    let loaded = shared.store.load_config();
    let config = match &loaded {
        Ok(cfg) => Ok(cfg.as_ref()),
        Err(_) => Err(()),
    };
    // `attested_bodies` only changes what disclosure says a witnessed
    // submission carries; the route itself does not depend on it.
    let route = crate::disclosure::route_disclosure(
        config,
        attested_bodies,
        crate::disclosure::env_filter(),
    )
    .route;
    let harness = super::harness::rows_now(shared)
        .into_iter()
        .map(|row| {
            let link = HarnessLink {
                connected: row.connected,
                wired: row.wired,
            };
            (row.id, link)
        })
        .collect();
    let folders = {
        let policy = shared.policy.lock().expect("policy lock");
        policy
            .projects
            .values()
            .fold((0, 0, 0), |(a, n, i), entry| match entry.mode {
                ProjectMode::AutoUpload => (a + 1, n, i),
                ProjectMode::NotifyOnly => (a, n + 1, i),
                ProjectMode::Ignore => (a, n, i + 1),
            })
    };
    let rows = ledger_rows(shared);
    let ledger_readable = rows.is_some();
    let facts = DestinationFacts {
        route,
        private_ai: shared.private_inference_label(),
        watched,
        declarations,
        harness,
        rows,
        sessions: window_sessions(shared),
        folders,
    };
    let mut result = destinations(&facts);
    result["ledger_readable"] = serde_json::json!(ledger_readable);
    let mut observed_destinations = Vec::new();
    if ledger_readable {
        for route in [ROUTE_ROUTED, ROUTE_OUTSIDE, UNKNOWN] {
            if facts
                .rows
                .iter()
                .flatten()
                .any(|row| route_label(row) == route)
            {
                observed_destinations.push(serde_json::json!({
                    "route": route,
                    "to": if route == ROUTE_ROUTED { PARTY_NEAR_AI } else { UNKNOWN },
                    "via": "local_proxy", "basis": BASIS_OBSERVED,
                }));
            }
        }
    }
    result["observed_destinations"] = serde_json::json!(observed_destinations);
    let hosting = super::network_data::handle_private_ai(shared, req)
        .result
        .expect("private AI read returns a result");
    let state = hosting["state"].as_str();
    let owned = match state {
        Some(
            super::private_inference::LABEL_RUNNING
            | super::private_inference::LABEL_RUNNING_NO_BACKENDS
            | super::private_inference::LABEL_RUNNING_ANSWERED_ELSEWHERE
            | super::private_inference::LABEL_RUNNING_DESTINATION_UNKNOWN,
        ) => Some(true),
        Some(super::private_inference::LABEL_OFF) => Some(false),
        _ => None,
    };
    result["hub"] = serde_json::json!({
        "kind": "owned_loopback", "state": hosting["state"], "port": hosting["port"],
        "owned": owned,
        "basis": if owned == Some(true) { BASIS_CONFIGURED } else { UNKNOWN },
    });
    Response::ok(req.id, result)
}

/// The sessions the window saw, from the queue and the history cache.
///
/// A queue entry counts when the daemon last saw its session written inside
/// the window (or discovered it there, for an entry that predates that
/// field); a history record counts when it was submitted inside it. Local
/// reads only. `None` when the history cache cannot be read.
fn window_sessions(shared: &DaemonShared) -> Option<Vec<SessionMark>> {
    let since: DateTime<Utc> = Utc::now() - chrono::Duration::hours(ACTIVITY_WINDOW_HOURS);
    // Read from disk before taking the queue lock, so the lock is held only
    // for the in-memory pass.
    let history = super::history::HistoryCache::load(&shared.store).ok()?;
    let queue = shared.queue.lock().expect("queue lock");
    Some(session_marks(queue.all(), &history, since))
}

/// The sessions inside the window, from queue entries and history records.
///
/// Each mark is labelled with the tool the session reads as --
/// [`QueueEntry::displayed_source`](super::queue::QueueEntry::displayed_source),
/// the declared source when discovery knew one, else the adapter -- the same
/// rule `SessionRef::displayed_source` and `list_projects` apply. A history
/// record carries only the adapter (`HistoryRecord` has no declared source,
/// because `Receipt` has none), so it takes its label from the queue entry
/// with the same session hash, looked up across the whole queue rather than
/// only the window's slice of it. The queue keeps uploaded entries (only
/// `Superseded` is compacted), so a twin is almost always there; a record
/// with none -- a CLI `submit`, say -- falls back to its adapter.
///
/// Queue marks come first, so [`tool_counts`], which counts a hash once
/// under its first mark, names a session by the queue's label.
#[must_use]
pub fn session_marks(
    queue: &[super::queue::QueueEntry],
    history: &[super::history::HistoryRecord],
    since: DateTime<Utc>,
) -> Vec<SessionMark> {
    let labels: std::collections::BTreeMap<&str, &str> = queue
        .iter()
        .map(|entry| (entry.session_hash.as_str(), entry.displayed_source()))
        .collect();
    let mut marks: Vec<SessionMark> = queue
        .iter()
        .filter(|entry| entry.observed_modified_at.unwrap_or(entry.discovered_at) >= since)
        .map(|entry| SessionMark {
            source: entry.displayed_source().to_string(),
            session_hash: entry.session_hash.clone(),
        })
        .collect();
    marks.extend(
        history
            .iter()
            .filter(|record| record.submitted_at >= since)
            .map(|record| SessionMark {
                source: labels
                    .get(record.session_hash.as_str())
                    .copied()
                    .unwrap_or(record.source.as_str())
                    .to_string(),
                session_hash: record.session_hash.clone(),
            }),
    );
    marks
}

/// A recorded model name, or `unknown` when it is not shaped like one.
///
/// Free text from a proxy the contributor can patch, passed through as
/// recorded only when it is at most 128 characters of the characters a model
/// id uses and names no URL; anything else is `unknown`. It is the one value
/// on this surface that is not a fixed label.
#[must_use]
pub fn model_label(recorded: Option<&str>) -> String {
    let Some(name) = recorded else {
        return UNKNOWN.to_string();
    };
    let shaped = !name.is_empty()
        && name.chars().count() <= MAX_MODEL_LABEL_CHARS
        && !name.contains("://")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':' | '/' | '@'));
    if shaped {
        name.to_string()
    } else {
        UNKNOWN.to_string()
    }
}

/// The protocol family label, or `unknown`.
fn family_label(facade: &str) -> &'static str {
    match facade {
        "anthropic" => "anthropic",
        "openai" => "openai",
        _ => UNKNOWN,
    }
}

/// IronWire's proof label as it goes on the wire: its own spelling, one of
/// seven, or `unrecorded` when the row carries none. Never collapsed.
#[must_use]
pub fn proof_label(row: &RoutedExchange) -> &'static str {
    row.proof.map_or(PROOF_UNRECORDED, ProofStatus::as_str)
}

/// The route, from IronWire's label and nothing else: `outside` is outside,
/// any other label is routed, and no label is `unknown`.
///
/// Not from the backend id, which is whatever the contributor named it.
#[must_use]
pub fn route_label(row: &RoutedExchange) -> &'static str {
    match row.proof {
        Some(ProofStatus::Outside) => ROUTE_OUTSIDE,
        Some(_) => ROUTE_ROUTED,
        None => UNKNOWN,
    }
}

/// The priced cost in millionths of a dollar, or `None`.
fn priced_micros(row: &RoutedExchange) -> Option<u64> {
    let usd = row.cost_usd?;
    if !usd.is_finite() || usd < 0.0 {
        return None;
    }
    let micros = (usd * 1_000_000.0).round();
    if micros > u64::MAX as f64 {
        return None;
    }
    Some(micros as u64)
}

fn encode_cursor(key: (i64, i64)) -> String {
    format!("{}:{}", key.0, key.1)
}

fn decode_cursor(cursor: &str) -> Option<(i64, i64)> {
    let (at, id) = cursor.split_once(':')?;
    Some((at.parse().ok()?, id.parse().ok()?))
}

/// The one tool whose connection writes this endpoint, or `None`.
fn endpoint_tool(facade: &str, path: &str) -> Option<&'static str> {
    let mut writers = TOOLS
        .iter()
        .filter(|spec| spec.endpoints.contains(&(facade, path)));
    match (writers.next(), writers.next()) {
        (Some(only), None) => Some(only.source),
        _ => None,
    }
}

/// Which tool made a call, or `unknown`.
///
/// IronWire's log rows carry no harness: a row says which facade took the
/// call and which endpoint inside it was called, and nothing else about the
/// caller. So, in order:
///
/// 1. **The endpoint**, when the row records one. Each tool this daemon
///    connects is pointed at its own facade and speaks its own API there --
///    Claude Code `/anthropic` + Messages, Codex `/openai/v1` + Responses --
///    so an endpoint exactly one tool's connection writes names that tool,
///    whether or not it is still connected. An endpoint no connection
///    writes (`/v1/chat/completions`, say) is `unknown`: it is not the
///    connected tool's own wire, so naming that tool would be a guess. So is
///    a named endpoint while a *different* connected tool speaks the same
///    family.
/// 2. **The harness that set the proxy**, on a proxy too old to record the
///    endpoint: the one tool connected *now* that speaks the call's family,
///    the rule `harness_list` applies to `answering`. Approximate by
///    construction -- a row from before a connection changed can carry the
///    wrong name -- which is why the endpoint outranks it.
///
/// A tool pointed at the proxy by hand, speaking the same API at the same
/// facade as a tool this daemon connects, is indistinguishable in the row
/// and reads as that tool. Nothing in the row can tell them apart.
fn attribute(row: &RoutedExchange, speakers: &[(&'static str, &'static str)]) -> &'static str {
    let facade = row.facade.as_str();
    if let Some(path) = row.path.as_deref().filter(|path| !path.is_empty()) {
        let Some(tool) = endpoint_tool(facade, path) else {
            return UNKNOWN;
        };
        // Unreachable in production today: `TOOLS` has one connectable tool
        // per family (Claude Code for `anthropic`, Codex for `openai`), so no
        // *other* connected speaker of the family can exist. Kept so a second
        // connectable tool in a family makes its endpoints `unknown` rather
        // than silently crediting the first.
        let contested = speakers
            .iter()
            .any(|(source, family)| *family == facade && *source != tool);
        return if contested { UNKNOWN } else { tool };
    }
    let mut speaking = speakers.iter().filter(|(_, family)| *family == facade);
    match (speaking.next(), speaking.next()) {
        (Some((only, _)), None) => only,
        _ => UNKNOWN,
    }
}

/// One page of calls, newest first, and the cursor for the next page (`None`
/// on the last).
///
/// Ordered by `(started_at millis, row id)`. Rows without an id -- a proxy
/// too old to expose one -- are not listed: the id is what makes the order
/// total, so a page boundary can never fall between two rows it cannot tell
/// apart.
#[must_use]
pub fn calls_page(
    rows: &[RoutedExchange],
    speakers: &[(&'static str, &'static str)],
    before: Option<(i64, i64)>,
    limit: usize,
) -> (Vec<serde_json::Value>, Option<String>) {
    let mut ordered: Vec<((i64, i64), &RoutedExchange)> = rows
        .iter()
        .filter_map(|row| {
            row.id
                .map(|id| ((row.started_at.timestamp_millis(), id), row))
        })
        .filter(|(key, _)| before.is_none_or(|cursor| *key < cursor))
        .collect();
    ordered.sort_by_key(|(key, _)| std::cmp::Reverse(*key));
    let more = ordered.len() > limit;
    ordered.truncate(limit);
    let next = if more {
        ordered.last().map(|(key, _)| encode_cursor(*key))
    } else {
        None
    };
    let calls = ordered
        .into_iter()
        .map(|(_, row)| {
            let micros = priced_micros(row);
            serde_json::json!({
                "id": row.id,
                "at": row.started_at.to_rfc3339(),
                "tool": attribute(row, speakers),
                "family": family_label(&row.facade),
                "model": model_label(row.served_model.as_deref().or(row.requested_model.as_deref())),
                "route": route_label(row),
                // Priced, not billed: the ledger prices every call, including
                // work a monthly plan already paid for.
                "cost": { "known": micros.is_some(), "priced_micros": micros },
                "proof": proof_label(row),
            })
        })
        .collect();
    (calls, next)
}

/// The most `inference_call_added` events one poll tick publishes.
///
/// Well under the event buffer, so a burst of calls -- a machine waking
/// after a day, say -- cannot push a subscriber into `resync_required` and
/// cost it a `queue_changed`. A busier tick keeps the newest; the event is a
/// pulse, and the counts on `tool_destinations` stay exact.
pub const MAX_ADDED_PER_TICK: usize = 64;

/// The `inference_call_added` data for one row.
///
/// The same labels `inference_calls` puts on the row, and nothing else: the
/// ledger id, the tool, the model label and IronWire's proof label. No body,
/// URL, endpoint, session id, digest or token.
#[must_use]
pub fn call_added(
    row: &RoutedExchange,
    speakers: &[(&'static str, &'static str)],
) -> serde_json::Value {
    serde_json::json!({
        "id": row.id,
        "tool": attribute(row, speakers),
        "model": model_label(row.served_model.as_deref().or(row.requested_model.as_deref())),
        "proof": proof_label(row),
    })
}

/// Publish one `inference_call_added` per added row, oldest first, at most
/// [`MAX_ADDED_PER_TICK`] of the newest.
///
/// Reads the harness rows only when there is something to publish: they
/// read tool config files, and most ticks add nothing.
pub(crate) fn publish_added_calls(shared: &DaemonShared, added: &[RoutedExchange]) {
    let listed: Vec<&RoutedExchange> = added.iter().filter(|row| row.id.is_some()).collect();
    if listed.is_empty() {
        return;
    }
    let speakers = speakers_from_rows(&super::harness::rows_now(shared));
    let skip = listed.len().saturating_sub(MAX_ADDED_PER_TICK);
    for row in listed.into_iter().skip(skip) {
        shared.publish(
            super::ipc::EVENT_INFERENCE_CALL_ADDED,
            call_added(row, &speakers),
        );
    }
}

/// The ledger's rows for the window, or `None` when no ledger has answered.
fn ledger_rows(shared: &DaemonShared) -> Option<Vec<RoutedExchange>> {
    let ledger = shared.routing_ledger()?;
    ledger.last_refresh_at()?;
    let since: DateTime<Utc> = Utc::now() - chrono::Duration::hours(ACTIVITY_WINDOW_HOURS);
    Some(crate::routing::RoutingLedger::exchanges_since(
        ledger.as_ref(),
        since,
    ))
}

/// `inference_calls`.
pub fn handle_calls(shared: &DaemonShared, req: &Request) -> Response {
    let limit = match req.params.get("limit") {
        None | Some(serde_json::Value::Null) => DEFAULT_CALLS_LIMIT,
        Some(value) => match value.as_u64() {
            Some(n) if (1..=MAX_CALLS_LIMIT as u64).contains(&n) => n as usize,
            _ => return Response::err(req.id, ERR_BAD_PARAMS, ERR_LIMIT_INVALID),
        },
    };
    let before = match req.params.get("cursor") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => match value.as_str().and_then(decode_cursor) {
            Some(cursor) => Some(cursor),
            None => return Response::err(req.id, ERR_BAD_PARAMS, ERR_CURSOR_INVALID),
        },
    };
    let Some(rows) = ledger_rows(shared) else {
        // No ledger answered: not evidence of no calls, and said so.
        return Response::ok(
            req.id,
            serde_json::json!({
                "readable": false,
                "window_hours": ACTIVITY_WINDOW_HOURS,
                "calls": [],
                "next_cursor": null,
            }),
        );
    };
    let speakers = speakers_from_rows(&super::harness::rows_now(shared));
    let (calls, next) = calls_page(&rows, &speakers, before, limit);
    Response::ok(
        req.id,
        serde_json::json!({
            "readable": true,
            "window_hours": ACTIVITY_WINDOW_HOURS,
            "calls": calls,
            "next_cursor": next,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disclosure::Route;
    use chrono::TimeZone;

    const MODEL: &str = "Qwen/Qwen3.6-27B-FP8";
    const UPSTREAM_ID: &str = "0123456789abcdef0123456789abcdef";
    const BODY_REF: &str = "00000000000000000009-000000";
    const REQUEST_DIGEST: &str = "aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11aa11";
    const RESPONSE_DIGEST: &str =
        "bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22bb22";

    fn row(id: i64, offset: i64, proof: Option<ProofStatus>) -> RoutedExchange {
        RoutedExchange {
            id: Some(id),
            started_at: Utc.timestamp_opt(1_800_000_000 + offset, 0).unwrap(),
            client_session_id: Some("SESSION-SECRET".to_string()),
            total_ms: Some(10),
            facade: "openai".to_string(),
            path: None,
            backend: "my-backend".to_string(),
            requested_model: Some(MODEL.to_string()),
            served_model: Some(MODEL.to_string()),
            upstream_id: Some(UPSTREAM_ID.to_string()),
            request_sha256: Some(REQUEST_DIGEST.to_string()),
            response_sha256: Some(RESPONSE_DIGEST.to_string()),
            body_ref: Some(BODY_REF.to_string()),
            rung: "full".to_string(),
            attempts: 1,
            input_tokens: Some(1),
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: Some(1),
            cost_usd: Some(0.0123),
            status: 200,
            proof,
        }
    }

    fn facts(
        private_ai: &'static str,
        connected: bool,
        rows: Vec<RoutedExchange>,
    ) -> DestinationFacts {
        DestinationFacts {
            route: Route::Witness,
            private_ai,
            watched: [
                crate::source::SOURCE_CLAUDE_CODE,
                crate::source::SOURCE_CODEX,
            ]
            .into_iter()
            .collect(),
            declarations: vec![(
                crate::source::SOURCE_GEMINI_CLI,
                Some(SourceDeclaration::Off),
            )],
            harness: vec![
                (
                    "claude".to_string(),
                    HarnessLink {
                        connected,
                        wired: connected,
                    },
                ),
                (
                    "codex".to_string(),
                    HarnessLink {
                        connected,
                        wired: connected,
                    },
                ),
            ],
            rows: Some(rows),
            sessions: Some(Vec::new()),
            folders: (1, 2, 0),
        }
    }

    fn tool<'a>(value: &'a serde_json::Value, id: &str) -> &'a serde_json::Value {
        value["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["tool"] == id)
            .unwrap()
    }

    use super::super::private_inference::{
        LABEL_CRASHED, LABEL_OFF, LABEL_RUNNING, LABEL_RUNNING_ANSWERED_ELSEWHERE,
        LABEL_RUNNING_DESTINATION_UNKNOWN, LABEL_RUNNING_ELSEWHERE,
    };

    #[test]
    fn private_ai_on_routes_a_connected_tool_to_near_ai_and_off_leaves_it_at_its_vendor() {
        let on = destinations(&facts(LABEL_RUNNING, true, Vec::new()));
        assert_eq!(tool(&on, "claude-code")["model_calls"]["to"], "near_ai");
        // No call to look at: configured, not observed.
        assert_eq!(
            tool(&on, "claude-code")["model_calls"]["basis"],
            "configured"
        );
        assert_eq!(tool(&on, "gemini-cli")["model_calls"]["to"], "google");

        // Connected, Private AI off: never NEAR AI.
        let off_connected = destinations(&facts(LABEL_OFF, true, Vec::new()));
        assert_eq!(
            tool(&off_connected, "claude-code")["model_calls"]["to"],
            "unknown"
        );

        let off = destinations(&facts(LABEL_OFF, false, Vec::new()));
        assert_eq!(tool(&off, "claude-code")["model_calls"]["to"], "anthropic");
        assert_eq!(
            tool(&off, "claude-code")["model_calls"]["basis"],
            "tool_default"
        );
        assert_eq!(tool(&off, "codex")["model_calls"]["to"], "openai");
        assert_eq!(off["private_ai"], "off");
    }

    #[test]
    fn running_is_confirmed_by_the_ledger_and_contradicted_by_one_outside_call() {
        // The fixture rows are all `openai`, which is Codex's family.
        let routed = vec![
            row(1, 0, Some(ProofStatus::Verified)),
            row(2, 1, Some(ProofStatus::Pending)),
        ];
        let value = destinations(&facts(LABEL_RUNNING, true, routed.clone()));
        assert_eq!(tool(&value, "codex")["model_calls"]["to"], "near_ai");
        assert_eq!(tool(&value, "codex")["model_calls"]["basis"], "observed");
        // Claude Code's family saw nothing.
        assert_eq!(
            tool(&value, "claude-code")["model_calls"]["basis"],
            "configured"
        );

        let mut mixed = routed;
        mixed.push(row(3, 2, Some(ProofStatus::Outside)));
        let value = destinations(&facts(LABEL_RUNNING, true, mixed));
        assert_eq!(tool(&value, "codex")["model_calls"]["to"], "unknown");

        // An unlabelled row is evidence of nothing.
        let value = destinations(&facts(LABEL_RUNNING, true, vec![row(4, 0, None)]));
        assert_eq!(tool(&value, "codex")["model_calls"]["basis"], "configured");
    }

    #[test]
    fn a_connected_tool_whose_proxy_cannot_say_who_answers_is_unknown() {
        for label in [
            LABEL_OFF,
            LABEL_RUNNING_DESTINATION_UNKNOWN,
            LABEL_RUNNING_ELSEWHERE,
            LABEL_CRASHED,
        ] {
            let value = destinations(&facts(label, true, Vec::new()));
            assert_eq!(
                tool(&value, "claude-code")["model_calls"]["to"],
                "unknown",
                "{label}"
            );
        }
        let elsewhere = destinations(&facts(LABEL_RUNNING_ANSWERED_ELSEWHERE, true, Vec::new()));
        assert_eq!(
            tool(&elsewhere, "claude-code")["model_calls"]["to"],
            "anthropic"
        );
        let foreign = model_calls_to(
            "anthropic",
            Some(HarnessLink {
                connected: false,
                wired: true,
            }),
            LABEL_RUNNING,
            Observed::default(),
        );
        assert_eq!(foreign, (UNKNOWN, UNKNOWN));
        assert_eq!(tool(&elsewhere, "cline")["model_calls"]["to"], "unknown");
    }

    #[test]
    fn sessions_go_where_the_route_sends_them_and_nowhere_when_unwatched() {
        let value = destinations(&facts(LABEL_OFF, false, Vec::new()));
        assert_eq!(
            tool(&value, "claude-code")["sessions"]["to"],
            serde_json::json!(["commons", "witness"])
        );
        assert_eq!(tool(&value, "claude-code")["sessions"]["watch"], "watched");
        assert_eq!(tool(&value, "gemini-cli")["sessions"]["watch"], "off");
        assert_eq!(
            tool(&value, "gemini-cli")["sessions"]["to"],
            serde_json::json!([])
        );
        assert_eq!(tool(&value, "opencode")["sessions"]["watch"], "not_watched");
        assert_eq!(value["sessions_route"], "witness");
        assert_eq!(value["folders"]["ask_first"], 2);

        assert_eq!(session_parties(Route::Local), vec!["commons"]);
        for route in [
            Route::WitnessRefusing,
            Route::NotEnrolled,
            Route::SettingsUnreadable,
        ] {
            assert!(session_parties(route).is_empty());
        }
    }

    #[test]
    fn every_ironwire_proof_label_reaches_the_wire_unchanged() {
        let rows: Vec<RoutedExchange> = ProofStatus::ALL
            .into_iter()
            .enumerate()
            .map(|(i, status)| row(i as i64 + 1, i as i64, Some(status)))
            .collect();
        let (calls, _) = calls_page(&rows, &[], None, 50);
        let mut seen: Vec<String> = calls
            .iter()
            .map(|c| c["proof"].as_str().unwrap().to_string())
            .collect();
        seen.sort();
        let mut expected: Vec<String> = ProofStatus::ALL
            .into_iter()
            .map(|s| s.as_str().to_string())
            .collect();
        expected.sort();
        assert_eq!(seen, expected);
        for call in &calls {
            let label = call["proof"].as_str().unwrap();
            let status = ProofStatus::parse(label).unwrap();
            assert_eq!(label, status.as_str());
        }
        // None is its own label, and no row reads "verified" unless IronWire
        // said so.
        let (calls, _) = calls_page(&[row(9, 0, None)], &[], None, 50);
        assert_eq!(calls[0]["proof"], "unrecorded");
    }

    #[test]
    fn the_route_comes_from_the_label_not_the_backend() {
        let mut outside = row(1, 0, Some(ProofStatus::Outside));
        outside.backend = "nearai".to_string();
        assert_eq!(route_label(&outside), "outside");
        for status in ProofStatus::ALL
            .into_iter()
            .filter(|s| *s != ProofStatus::Outside)
        {
            assert_eq!(
                route_label(&row(2, 0, Some(status))),
                "routed",
                "{status:?}"
            );
        }
        let mut unlabelled = row(3, 0, None);
        unlabelled.backend = "nearai".to_string();
        assert_eq!(route_label(&unlabelled), "unknown");
    }

    #[test]
    fn pages_are_newest_first_bounded_and_resume_at_the_cursor() {
        let mut rows: Vec<RoutedExchange> = (0..7)
            .map(|i| row(i, i * 10, Some(ProofStatus::Pending)))
            .collect();
        // Two rows in the same millisecond page apart without loss.
        rows.push(row(100, 30, Some(ProofStatus::Pending)));
        // A row without an id is not listed.
        let mut no_id = row(0, 35, Some(ProofStatus::Pending));
        no_id.id = None;
        rows.push(no_id);
        let (first, next) = calls_page(&rows, &[], None, 3);
        let ids: Vec<i64> = first.iter().map(|c| c["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![6, 5, 4]);
        let cursor = decode_cursor(next.as_deref().unwrap()).unwrap();
        let (second, next) = calls_page(&rows, &[], Some(cursor), 3);
        let ids: Vec<i64> = second.iter().map(|c| c["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![100, 3, 2]);
        let cursor = decode_cursor(next.as_deref().unwrap()).unwrap();
        let (last, next) = calls_page(&rows, &[], Some(cursor), 3);
        let ids: Vec<i64> = last.iter().map(|c| c["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![1, 0]);
        assert_eq!(next, None);
        assert_eq!(decode_cursor("nonsense"), None);
    }

    /// Nothing here reads a body: the fixture's body reference names a file
    /// that does not exist, and a missing body changes nothing in the output.
    #[test]
    fn no_body_is_read_and_no_body_identifier_or_session_reaches_the_output() {
        let mut hostile = row(8, 0, Some(ProofStatus::Verified));
        hostile.served_model = Some("https://evil.example/PROMPT-SECRET".to_string());
        let with_ref = row(7, 0, Some(ProofStatus::Verified));
        let mut without_ref = with_ref.clone();
        without_ref.body_ref = None;
        without_ref.request_sha256 = None;
        without_ref.response_sha256 = None;
        let (a, _) = calls_page(&[with_ref, hostile.clone()], &[], None, 10);
        let (b, _) = calls_page(&[without_ref, hostile], &[], None, 10);
        assert_eq!(a, b, "the output must not depend on bodies or digests");
        let text = serde_json::to_string(&a).unwrap();
        for forbidden in [
            "PROMPT-SECRET",
            "SESSION-SECRET",
            UPSTREAM_ID,
            BODY_REF,
            REQUEST_DIGEST,
            RESPONSE_DIGEST,
            "://",
            "my-backend",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} leaked: {text}");
        }
        assert!(text.contains(MODEL));
        assert!(text.contains("\"priced_micros\":12300"));
    }

    #[test]
    fn a_tool_is_named_only_when_one_connected_tool_speaks_the_family() {
        fn harness(id: &str, family: &'static str, connected: bool) -> HarnessRow {
            HarnessRow {
                id: id.to_string(),
                name: id.to_string(),
                installed: true,
                connected,
                wired: connected,
                config_path: None,
                connect_command: String::new(),
                family: Some(family),
                state: crate::harness_state::HarnessState::NotConnected,
            }
        }
        // A row from a proxy too old to record the endpoint: only the
        // harness that set the proxy can name it.
        let codex = speakers_from_rows(&[harness("codex", "openai", true)]);
        assert_eq!(attribute(&called("openai", None), &codex), "codex");
        assert_eq!(attribute(&called("anthropic", None), &codex), "unknown");
        assert_eq!(
            attribute(
                &called("openai", None),
                &speakers_from_rows(&[harness("codex", "openai", false)])
            ),
            "unknown"
        );
    }

    fn called(facade: &str, path: Option<&str>) -> RoutedExchange {
        let mut r = row(1, 0, Some(ProofStatus::Verified));
        r.facade = facade.to_string();
        r.path = path.map(str::to_string);
        r
    }

    /// Claude Code's connection points it at `/anthropic`, where it speaks
    /// Messages: those endpoints name it, connected now or not.
    #[test]
    fn the_anthropic_messages_endpoints_name_claude_code_without_a_connection() {
        for path in ["/v1/messages", "/v1/messages/count_tokens"] {
            assert_eq!(
                attribute(&called("anthropic", Some(path)), &[]),
                "claude-code",
                "{path}"
            );
        }
    }

    /// Codex's connection points it at `/openai/v1` with
    /// `wire_api = "responses"`: the Responses endpoint names it.
    #[test]
    fn the_openai_responses_endpoint_names_codex_without_a_connection() {
        assert_eq!(
            attribute(&called("openai", Some("/v1/responses")), &[]),
            "codex"
        );
    }

    /// An endpoint no connection here writes is not the connected tool's own
    /// wire, so it is not named for it -- even with Codex connected, a chat
    /// completions call is somebody else's, and so is an unknown facade.
    #[test]
    fn an_endpoint_no_connection_writes_stays_unknown_even_with_a_speaker_connected() {
        let codex = vec![(crate::source::SOURCE_CODEX, "openai")];
        assert_eq!(
            attribute(&called("openai", Some("/v1/chat/completions")), &codex),
            "unknown"
        );
        assert_eq!(
            attribute(&called("anthropic", Some("/v1/responses")), &[]),
            "unknown"
        );
        assert_eq!(
            attribute(&called("gemini", Some("/v1/messages")), &[]),
            "unknown"
        );
        // An empty endpoint is no endpoint: the harness rule decides.
        assert_eq!(attribute(&called("openai", Some("")), &codex), "codex");
    }

    /// A named endpoint is not taken over a different tool connected now in
    /// the same family: the row cannot tell which of them called.
    #[test]
    fn a_named_endpoint_contested_by_another_connected_speaker_is_unknown() {
        let both = vec![
            (crate::source::SOURCE_CLAUDE_CODE, "anthropic"),
            (crate::source::SOURCE_CLINE, "anthropic"),
        ];
        assert_eq!(
            attribute(&called("anthropic", Some("/v1/messages")), &both),
            "unknown"
        );
        let claude = vec![(crate::source::SOURCE_CLAUDE_CODE, "anthropic")];
        assert_eq!(
            attribute(&called("anthropic", Some("/v1/messages")), &claude),
            "claude-code"
        );
    }

    /// The real page IronWire served: its rows carry `path`, and the two
    /// rows attribute by it with no tool connected.
    #[test]
    fn a_real_proxy_page_attributes_by_its_recorded_endpoint() {
        let body: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/ironwire/log-page-2026-09-03.json"
        ))
        .unwrap();
        let rows: Vec<RoutedExchange> = body["exchanges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| serde_json::from_value(r.clone()).unwrap())
            .collect();
        assert_eq!(rows[0].path.as_deref(), Some("/v1/messages"));
        assert_eq!(attribute(&rows[0], &[]), "claude-code");
        // `/v1/chat/completions` is not Codex's wire.
        assert_eq!(attribute(&rows[1], &[]), "unknown");
        let (calls, _) = calls_page(&rows, &[], None, 10);
        assert!(
            !serde_json::to_string(&calls).unwrap().contains("/v1/"),
            "the endpoint is read, never passed through"
        );
    }

    fn mark(source: &str, hash: &str) -> SessionMark {
        SessionMark {
            source: source.to_string(),
            session_hash: hash.to_string(),
        }
    }

    /// The map's per-tool node counts: sessions once per hash, calls exactly
    /// the rows `inference_calls` lists, by the tool it names.
    #[test]
    fn each_tool_counts_its_sessions_once_and_the_calls_the_list_names() {
        // Three openai rows (Codex's family) and one anthropic row; Codex is
        // the only connected speaker, so the anthropic row is nobody's.
        let mut anthropic = row(4, 3, Some(ProofStatus::Verified));
        anthropic.facade = "anthropic".to_string();
        let mut no_id = row(0, 4, Some(ProofStatus::Verified));
        no_id.id = None;
        let rows = vec![
            row(1, 0, Some(ProofStatus::Verified)),
            row(2, 1, None),
            row(3, 2, Some(ProofStatus::Outside)),
            anthropic,
            no_id,
        ];
        let sessions = vec![
            mark("claude-code", "sha256:a"),
            // The same session in the queue and in history is one session.
            mark("claude-code", "sha256:a"),
            mark("claude-code", "sha256:b"),
            mark("codex", "sha256:c"),
            // An adapter the map has no node for counts nowhere.
            mark("trajectory", "sha256:d"),
        ];
        let speakers = vec![(crate::source::SOURCE_CODEX, "openai")];
        let (counts, unattributed) = tool_counts(Some(&rows), &speakers, Some(&sessions));
        let get = |id: &str| *counts.iter().find(|c| c.tool == id).unwrap();
        assert_eq!(get("codex").inference_calls, Some(3));
        assert_eq!(get("codex").sessions, Some(1));
        assert_eq!(get("claude-code").inference_calls, Some(0));
        assert_eq!(get("claude-code").sessions, Some(2));
        assert_eq!(get("cline").sessions, Some(0));
        assert_eq!(unattributed, Some(1), "the id-less row is not listed");

        // Unreadable is null, never zero.
        let (counts, unattributed) = tool_counts(None, &speakers, None);
        assert!(
            counts
                .iter()
                .all(|c| c.sessions.is_none() && c.inference_calls.is_none())
        );
        assert_eq!(unattributed, None);

        // And it reaches the wire on every tool row. Here both tools are
        // connected, so the anthropic row is Claude Code's.
        let mut facts = facts(LABEL_RUNNING, true, rows);
        facts.sessions = Some(sessions);
        let value = destinations(&facts);
        assert_eq!(value["window_hours"], ACTIVITY_WINDOW_HOURS);
        assert_eq!(
            tool(&value, "codex")["counts"],
            serde_json::json!({"sessions": 1, "inference_calls": 3})
        );
        assert_eq!(tool(&value, "claude-code")["counts"]["inference_calls"], 1);
        assert_eq!(value["unattributed_calls"], 0);
        facts.rows = None;
        facts.sessions = None;
        let value = destinations(&facts);
        assert_eq!(
            tool(&value, "codex")["counts"],
            serde_json::json!({"sessions": null, "inference_calls": null})
        );
        assert_eq!(value["unattributed_calls"], serde_json::Value::Null);
    }

    fn history_record(
        source: &str,
        hash: &str,
        at: DateTime<Utc>,
    ) -> super::super::history::HistoryRecord {
        super::super::history::HistoryRecord {
            submission_id: uuid::Uuid::new_v4(),
            submitted_at: at,
            project_id: "p".to_string(),
            project_label: "p".to_string(),
            source: source.to_string(),
            session_hash: hash.to_string(),
            status: "accepted".to_string(),
            consent_scopes: vec![],
            credit_points_pending: 0.0,
            credit_points_final: None,
            explanations: vec![],
            last_refreshed_at: None,
            withdrawn_at: None,
            approved_unattended: None,
            approved_verdict: None,
            revoked_at: None,
            uploaded_bytes: None,
        }
    }

    /// An imported Antigravity conversation is stored by the `trajectory`
    /// adapter and declares itself `antigravity`. Its queue entry and its
    /// history record (which carries only the adapter) are one session, so
    /// they must come out under one key -- the declared one, the rule
    /// `SessionRef::displayed_source` and `list_projects` use -- and never
    /// under two.
    #[test]
    fn an_imported_antigravity_session_counts_under_one_key_never_twice() {
        let now = Utc::now();
        let hash = "sha256:imported";
        let entry = super::super::queue::QueueEntry {
            entry_id: uuid::Uuid::new_v4(),
            session_hash: hash.to_string(),
            source: crate::source::SOURCE_TRAJECTORY.to_string(),
            declared_source: Some("antigravity".to_string()),
            discovered_at: now,
            ..Default::default()
        };
        let history = vec![history_record(crate::source::SOURCE_TRAJECTORY, hash, now)];
        let marks = session_marks(
            std::slice::from_ref(&entry),
            &history,
            now - chrono::Duration::hours(1),
        );
        let keys: BTreeSet<(&str, &str)> = marks
            .iter()
            .map(|m| (m.source.as_str(), m.session_hash.as_str()))
            .collect();
        assert_eq!(
            keys,
            BTreeSet::from([("antigravity", hash)]),
            "one session, one key, named by what it declared: {marks:?}"
        );

        // The queue twin labels the history record even when the entry
        // itself was last seen outside the window.
        let mut old = entry;
        old.discovered_at = now - chrono::Duration::days(3);
        let marks = session_marks(&[old], &history, now - chrono::Duration::hours(1));
        assert_eq!(marks, vec![mark("antigravity", hash)]);
    }

    /// A session hash is one session however its marks are labelled: two
    /// marks for one hash can never be counted under two tools.
    #[test]
    fn one_session_hash_is_counted_once_even_under_two_labels() {
        let sessions = vec![mark("claude-code", "sha256:x"), mark("codex", "sha256:x")];
        let (counts, _) = tool_counts(None, &[], Some(&sessions));
        let total: usize = counts.iter().filter_map(|c| c.sessions).sum();
        assert_eq!(total, 1, "{counts:?}");
        // The first mark -- the queue's, which carries the declared source
        // -- names it.
        let get = |id: &str| counts.iter().find(|c| c.tool == id).unwrap().sessions;
        assert_eq!(get("claude-code"), Some(1));
        assert_eq!(get("codex"), Some(0));
    }

    /// The event carries the list's labels for the row, and none of the
    /// row's identifiers, bodies or endpoint.
    #[test]
    fn a_call_added_event_is_label_only() {
        let mut r = row(7, 0, Some(ProofStatus::GatewayOnly));
        r.facade = "anthropic".to_string();
        r.path = Some("/v1/messages".to_string());
        let value = call_added(&r, &[]);
        assert_eq!(
            value,
            serde_json::json!({
                "id": 7,
                "tool": "claude-code",
                "model": MODEL,
                "proof": "gateway_only",
            })
        );
        let text = value.to_string();
        for forbidden in [
            "SESSION-SECRET",
            UPSTREAM_ID,
            BODY_REF,
            REQUEST_DIGEST,
            RESPONSE_DIGEST,
            "/v1/",
            "my-backend",
        ] {
            assert!(!text.contains(forbidden), "{forbidden} leaked: {text}");
        }
    }

    /// One event per added row, oldest first, capped at the newest
    /// `MAX_ADDED_PER_TICK`; nothing at all for nothing added.
    #[test]
    fn added_calls_publish_one_event_each_up_to_the_cap() {
        let (_dir, s) = shared();
        let mut rx = s.events.subscribe();
        publish_added_calls(&s, &[]);
        assert!(rx.try_recv().is_err(), "nothing added, nothing published");

        let added: Vec<RoutedExchange> = (1..=3).map(|id| row(id, id, None)).collect();
        publish_added_calls(&s, &added);
        let mut seen = Vec::new();
        while let Ok(event) = rx.try_recv() {
            assert_eq!(event.event, super::super::ipc::EVENT_INFERENCE_CALL_ADDED);
            seen.push(event.data["id"].as_i64().unwrap());
        }
        assert_eq!(seen, vec![1, 2, 3]);

        let burst: Vec<RoutedExchange> = (1..=(MAX_ADDED_PER_TICK as i64 + 10))
            .map(|id| row(id, id, None))
            .collect();
        publish_added_calls(&s, &burst);
        let mut seen = Vec::new();
        while let Ok(event) = rx.try_recv() {
            seen.push(event.data["id"].as_i64().unwrap());
        }
        assert_eq!(seen.len(), MAX_ADDED_PER_TICK);
        assert_eq!(seen.first(), Some(&11), "the newest are kept");
    }

    /// Through the daemon's own poll-tick entry point against a mock proxy:
    /// the first window baselines, and a call the next read finds is one
    /// event.
    #[tokio::test]
    async fn the_poll_tick_publishes_a_call_the_log_newly_shows() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let reads = std::sync::Arc::new(AtomicUsize::new(0));
        let served = std::sync::Arc::clone(&reads);
        let router = axum::Router::new().route(
            "/_ironwire/log",
            axum::routing::get(move || {
                let n = served.fetch_add(1, Ordering::SeqCst);
                async move {
                    let row = |id: usize| {
                        serde_json::json!({
                            "id": id, "started_at": Utc::now().to_rfc3339(),
                            "facade": "openai", "path": "/v1/responses",
                            "backend": "b", "rung": "full", "attempts": 1,
                            "status": 200, "served_model": "m-1", "proof": "verified",
                        })
                    };
                    let rows: Vec<serde_json::Value> = (1..=(2 + n.min(1))).map(row).collect();
                    axum::Json(serde_json::json!({ "exchanges": rows }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        let (_dir, s) = shared();
        s.install_routing_ledger_for_test(crate::routing::ironwire::IronWireLedger::new(
            port,
            "t".to_string(),
        ));
        let mut rx = s.events.subscribe();
        let added = |rx: &mut tokio::sync::broadcast::Receiver<super::super::ipc::Event>| {
            let mut out = Vec::new();
            while let Ok(event) = rx.try_recv() {
                if event.event == super::super::ipc::EVENT_INFERENCE_CALL_ADDED {
                    out.push(event.data);
                }
            }
            out
        };
        s.refresh_routing().await;
        assert!(added(&mut rx).is_empty(), "the first window is the backlog");
        s.refresh_routing().await;
        assert_eq!(
            added(&mut rx),
            vec![serde_json::json!({
                "id": 3, "tool": "codex", "model": "m-1", "proof": "verified",
            })]
        );
        s.refresh_routing().await;
        assert!(added(&mut rx).is_empty(), "a call is announced once");
    }

    /// Every event `hello` lists has a row in the contract's events table,
    /// and K14's fields are written down where a shell reads them.
    #[test]
    fn the_events_hello_lists_and_the_map_counts_are_documented() {
        let contract = include_str!("../../../../docs/contributor-daemon-ipc-v1_1.md");
        let hello =
            super::super::ipc::handle_request(&shared().1, &call("hello", serde_json::json!({})));
        for event in hello.result.unwrap()["events"].as_array().unwrap() {
            let row = format!("| `{}` |", event.as_str().unwrap());
            assert!(contract.contains(&row), "events table lacks {row}");
        }
        for field in [
            "\"unattributed_calls\"",
            "\"counts\": { \"sessions\"",
            "`{id, tool, model, proof}`",
            "| `openai` | `/v1/responses`",
        ] {
            assert!(contract.contains(field), "undocumented: {field}");
        }
    }

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn call(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    fn recent(id: i64) -> RoutedExchange {
        let mut r = row(id, 0, Some(ProofStatus::Verified));
        r.started_at = Utc::now() - chrono::Duration::minutes(60 - id);
        r
    }

    #[test]
    fn the_methods_are_advertised_and_answer_over_the_dispatcher() {
        for method in ["tool_destinations", "inference_calls"] {
            assert!(super::super::ipc::METHODS.contains(&method), "{method}");
        }
        let hello =
            super::super::ipc::handle_request(&shared().1, &call("hello", serde_json::json!({})));
        assert!(
            hello.result.unwrap()["events"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("inference_call_added")),
            "hello must list the event a shell subscribes for"
        );
        assert!(super::super::ipc::METHODS.contains(&"inference_call_proof"));
        let (_dir, s) = shared();
        let r = super::super::ipc::handle_request(
            &s,
            &call("tool_destinations", serde_json::json!({})),
        );
        let value = r.result.expect("destinations");
        assert_eq!(value["private_ai"], "off");
        assert_eq!(value["sessions_route"], "not_enrolled");
        for t in value["tools"].as_array().unwrap() {
            assert_eq!(t["sessions"]["to"], serde_json::json!([]));
            // No ledger answered: calls unknown. An empty queue and history
            // are a readable zero.
            assert_eq!(
                t["counts"],
                serde_json::json!({"sessions": 0, "inference_calls": null})
            );
        }
        let r =
            super::super::ipc::handle_request(&s, &call("inference_calls", serde_json::json!({})));
        assert_eq!(r.result.expect("calls")["readable"], false);
    }

    /// Sessions come from the queue and the history cache, inside the
    /// window only, once per session.
    #[test]
    fn tool_destinations_counts_the_windows_sessions_from_queue_and_history() {
        let (_dir, s) = shared();
        let now = Utc::now();
        let entry = |hash: &str, source: &str, at: DateTime<Utc>| super::super::queue::QueueEntry {
            entry_id: uuid::Uuid::new_v4(),
            session_hash: hash.to_string(),
            source: source.to_string(),
            discovered_at: at,
            ..Default::default()
        };
        {
            let mut queue = s.queue.lock().unwrap();
            queue
                .upsert(entry("sha256:q1", "claude-code", now), 100)
                .unwrap();
            // Written to inside the window though discovered before it.
            let mut written = entry("sha256:q2", "claude-code", now - chrono::Duration::days(3));
            written.observed_modified_at = Some(now);
            queue.upsert(written, 100).unwrap();
            queue
                .upsert(
                    entry("sha256:old", "claude-code", now - chrono::Duration::days(3)),
                    100,
                )
                .unwrap();
        }
        let record = |hash: &str, at: DateTime<Utc>| history_record("codex", hash, at);
        super::super::history::HistoryCache::save(
            &s.store,
            &[
                record("sha256:h1", now),
                record("sha256:h-old", now - chrono::Duration::days(2)),
            ],
        )
        .unwrap();
        let r = super::super::ipc::handle_request(
            &s,
            &call("tool_destinations", serde_json::json!({})),
        );
        let value = r.result.expect("destinations");
        assert_eq!(tool(&value, "claude-code")["counts"]["sessions"], 2);
        assert_eq!(tool(&value, "codex")["counts"]["sessions"], 1);
    }

    /// Over the dispatcher: a session in the queue and in history under
    /// different labels is counted once, under the queue's label.
    #[test]
    fn tool_destinations_counts_a_session_once_under_the_queues_label() {
        let (_dir, s) = shared();
        let now = Utc::now();
        s.queue
            .lock()
            .unwrap()
            .upsert(
                super::super::queue::QueueEntry {
                    entry_id: uuid::Uuid::new_v4(),
                    session_hash: "sha256:both".to_string(),
                    source: "claude-code".to_string(),
                    discovered_at: now,
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        super::super::history::HistoryCache::save(
            &s.store,
            &[history_record("codex", "sha256:both", now)],
        )
        .unwrap();
        let r = super::super::ipc::handle_request(
            &s,
            &call("tool_destinations", serde_json::json!({})),
        );
        let value = r.result.expect("destinations");
        assert_eq!(tool(&value, "claude-code")["counts"]["sessions"], 1);
        assert_eq!(tool(&value, "codex")["counts"]["sessions"], 0);
    }

    #[test]
    fn stored_call_proof_lookup_preserves_every_label_without_body_reads() {
        let (_dir, s) = shared();
        let mut rows: Vec<_> = ProofStatus::ALL
            .into_iter()
            .enumerate()
            .map(|(i, proof)| {
                let mut r = recent(i as i64 + 1);
                r.proof = Some(proof);
                r
            })
            .collect();
        let mut unknown = recent(100);
        unknown.proof = None;
        rows.push(unknown);
        s.install_routing_ledger_for_test(
            crate::routing::ironwire::IronWireLedger::with_rows_for_test(rows),
        );
        for (i, proof) in ProofStatus::ALL.into_iter().enumerate() {
            let response = super::super::ipc::handle_local(
                &s,
                "inference_call_proof",
                serde_json::json!({"call_id":i + 1}),
            );
            let value = response.result.unwrap();
            assert_eq!(value["proof"], proof.as_str());
            assert_eq!(value["found"], true);
            assert_eq!(value["readable"], true);
            assert!(value["checked_at"].is_null());
            assert!(value["checks"].is_null());
            assert!(!value.to_string().contains(BODY_REF));
        }
        let value = super::super::ipc::handle_local(
            &s,
            "inference_call_proof",
            serde_json::json!({"call_id":100}),
        )
        .result
        .unwrap();
        assert_eq!(value["proof"], "unrecorded");
        let missing = super::super::ipc::handle_local(
            &s,
            "inference_call_proof",
            serde_json::json!({"call_id":999}),
        )
        .result
        .unwrap();
        assert_eq!(missing["found"], false);
        assert_eq!(missing["proof"], "unrecorded");
        for id in [
            serde_json::json!(0),
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!("1"),
            serde_json::Value::Null,
        ] {
            assert!(
                super::super::ipc::handle_local(
                    &s,
                    "inference_call_proof",
                    serde_json::json!({"call_id":id})
                )
                .error
                .is_some()
            );
        }
    }

    #[test]
    fn destinations_expose_observed_outside_through_proxy_with_unknown_provider() {
        let (_dir, s) = shared();
        let mut outside = recent(1);
        outside.proof = Some(ProofStatus::Outside);
        outside.backend = "SECRET-PROVIDER".to_string();
        s.install_routing_ledger_for_test(
            crate::routing::ironwire::IronWireLedger::with_rows_for_test(vec![outside]),
        );
        let value = super::super::ipc::handle_local(&s, "tool_destinations", serde_json::json!({}))
            .result
            .unwrap();
        assert_eq!(value["ledger_readable"], true);
        assert_eq!(
            value["observed_destinations"],
            serde_json::json!([{"route":"outside","to":"unknown","via":"local_proxy","basis":"observed"}])
        );
        assert_eq!(value["hub"]["state"], "off");
        assert_eq!(value["hub"]["owned"], false);
        assert!(!value.to_string().contains("SECRET-PROVIDER"));
        let (_dir, unreadable) = shared();
        let value = super::super::ipc::handle_local(
            &unreadable,
            "tool_destinations",
            serde_json::json!({}),
        )
        .result
        .unwrap();
        assert_eq!(value["ledger_readable"], false);
        assert_eq!(value["observed_destinations"], serde_json::json!([]));
    }

    #[test]
    fn inference_calls_pages_the_held_ledger_and_refuses_a_bad_limit_or_cursor() {
        let (_dir, s) = shared();
        s.install_routing_ledger_for_test(
            crate::routing::ironwire::IronWireLedger::with_rows_for_test(
                (1..=5).map(recent).collect(),
            ),
        );
        let r = super::super::ipc::handle_request(
            &s,
            &call("inference_calls", serde_json::json!({"limit": 2})),
        );
        let page = r.result.expect("a page");
        assert_eq!(page["readable"], true);
        let ids: Vec<i64> = page["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, vec![5, 4]);
        assert_eq!(page["calls"][0]["proof"], "verified");
        let cursor = page["next_cursor"].as_str().unwrap().to_string();
        let r = super::super::ipc::handle_request(
            &s,
            &call(
                "inference_calls",
                serde_json::json!({"limit": 2, "cursor": cursor}),
            ),
        );
        let ids: Vec<i64> = r.result.unwrap()["calls"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_i64().unwrap())
            .collect();
        assert_eq!(ids, vec![3, 2]);

        for bad in [
            serde_json::json!({"limit": 0}),
            serde_json::json!({"limit": MAX_CALLS_LIMIT + 1}),
            serde_json::json!({"limit": "5"}),
        ] {
            let r = super::super::ipc::handle_request(&s, &call("inference_calls", bad));
            assert_eq!(r.error.unwrap().message, ERR_LIMIT_INVALID);
        }
        let r = super::super::ipc::handle_request(
            &s,
            &call("inference_calls", serde_json::json!({"cursor": "x"})),
        );
        assert_eq!(r.error.unwrap().message, ERR_CURSOR_INVALID);
    }
}
