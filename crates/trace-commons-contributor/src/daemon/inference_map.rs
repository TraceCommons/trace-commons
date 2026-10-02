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
//! attributed only when exactly one tool connected now speaks its protocol
//! family, the rule `harness_list` applies, so a row can be misattributed
//! after a connection changes); no billed cost (the ledger prices every
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
    name: &'static str,
}

const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        source: crate::source::SOURCE_CLAUDE_CODE,
        harness: Some("claude"),
        family: Some("anthropic"),
        name: "Claude Code",
    },
    ToolSpec {
        source: crate::source::SOURCE_CODEX,
        harness: Some("codex"),
        family: Some("openai"),
        name: "Codex",
    },
    ToolSpec {
        source: crate::source::SOURCE_GEMINI_CLI,
        harness: None,
        family: None,
        name: "Gemini CLI",
    },
    ToolSpec {
        source: crate::source::SOURCE_CLINE,
        harness: None,
        family: None,
        name: "Cline",
    },
    ToolSpec {
        source: crate::source::SOURCE_OPENCODE,
        harness: None,
        family: None,
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
    /// The ledger rows for the window; empty when no ledger answered.
    pub rows: Vec<RoutedExchange>,
    pub folders: (usize, usize, usize),
}

/// The `tool_destinations` result for a set of facts.
#[must_use]
pub fn destinations(facts: &DestinationFacts) -> serde_json::Value {
    let parties = session_parties(facts.route);
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
                .map_or(Observed::default(), |family| observe(&facts.rows, family));
            let (to, basis) = model_calls_to(vendor(spec.source), link, facts.private_ai, observed);
            serde_json::json!({
                "tool": spec.source,
                "name": spec.name,
                "sessions": {
                    "watch": watch_label(watched, declaration),
                    "to": if watched { parties.clone() } else { Vec::new() },
                },
                "model_calls": { "to": to, "basis": basis },
            })
        })
        .collect();
    serde_json::json!({
        "private_ai": facts.private_ai,
        "sessions_route": facts.route,
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
        rows: rows.unwrap_or_default(),
        folders,
    };
    let mut result = destinations(&facts);
    result["ledger_readable"] = serde_json::json!(ledger_readable);
    let mut observed_destinations = Vec::new();
    if ledger_readable {
        for route in [ROUTE_ROUTED, ROUTE_OUTSIDE, UNKNOWN] {
            if facts.rows.iter().any(|row| route_label(row) == route) {
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

/// Which tool made a call, or `unknown`.
///
/// The ledger records a protocol family, not a tool. A call is named only
/// when exactly one tool connected *now* speaks its family -- the rule
/// `harness_list` applies to `answering`. Approximate by construction: a row
/// from before a connection changed can carry the wrong name.
fn attribute(facade: &str, harness: &[HarnessRow]) -> &'static str {
    let mut speakers = harness
        .iter()
        .filter(|row| row.connected && row.family == Some(facade));
    match (speakers.next(), speakers.next()) {
        (Some(only), None) => source_for_harness(&only.id).unwrap_or(UNKNOWN),
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
    harness: &[HarnessRow],
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
                "tool": attribute(&row.facade, harness),
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
    let harness = super::harness::rows_now(shared);
    let (calls, next) = calls_page(&rows, &harness, before, limit);
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
            rows,
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
        let codex = [harness("codex", "openai", true)];
        assert_eq!(attribute("openai", &codex), "codex");
        assert_eq!(attribute("anthropic", &codex), "unknown");
        assert_eq!(
            attribute("openai", &[harness("codex", "openai", false)]),
            "unknown"
        );
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
        }
        let r =
            super::super::ipc::handle_request(&s, &call("inference_calls", serde_json::json!({})));
        assert_eq!(r.result.expect("calls")["readable"], false);
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
