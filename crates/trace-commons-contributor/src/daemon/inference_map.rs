//! The map and the Inference tab (K8 of #1118): where each tool's sessions
//! and model calls go, and the model calls this machine's proxy recorded.
//!
//! Three methods, all read from state this daemon already holds:
//!
//! - `tool_destinations` -- per tool, the parties its sessions go to and the
//!   party that answers its model calls. Local and synchronous.
//! - `inference_calls` -- one page of the calls in the local routing ledger,
//!   newest first, with the route, the priced cost and the proof status.
//!   Local and synchronous.
//! - `inference_call_proof` -- check the receipt for one call, on demand.
//!   The only method here that touches the network, and the only thing that
//!   can make a call read `verified`.
//!
//! # What crosses the socket
//!
//! Fixed labels, counts, times, the ledger's own row id, and the model name
//! as the proxy recorded it (after a shape check). Never a prompt, a
//! response, a body reference, a body digest, the provider's exchange
//! identifier, a session id, a token or a URL. Nothing here is logged.
//!
//! # Every unknown is `unknown`
//!
//! A tool whose config names somebody else's port, a proxy that is running
//! but cannot say which account answers, a facade this build does not know:
//! each is `unknown`, never the nearest guess. A proof is `verified` only
//! when this process fetched the receipt and verified it against the call's
//! own bytes with [`crate::routing::receipt::receipt_matches_call`] and its
//! signer against a freshly-nonced attestation report -- never because a
//! row looks like one that could carry a receipt.
//!
//! # What the ledger cannot say (the Z4 half)
//!
//! The per-call list is the local ledger's rows and nothing more. It has no
//! kind of work (no classifier is invented here), no tool id (a call is
//! attributed only when its protocol family belongs to exactly one connected
//! tool, the rule `harness_list` already applies), no billed cost (the
//! ledger prices every call, including work a plan already paid for), and no
//! calls that never passed through the proxy -- an outside call that went
//! straight to a vendor is invisible here. See the IPC document.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use sha2::{Digest as _, Sha256};

use super::harness::{ACTIVITY_WINDOW_HOURS, HarnessRow};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::policy::ProjectMode;
use super::settings::SourceDeclaration;
use crate::routing::RoutedExchange;
use crate::routing::attested::{AttestedCall, ProviderIdentifier, classify_upstream_id};
use crate::routing::receipt::ReceiptFetchError;

/// Rows per page when a caller names no limit.
pub const DEFAULT_CALLS_LIMIT: usize = 50;
/// The most rows one page may carry. A larger ask is refused, not capped, so
/// a caller that pages by the size it asked for never skips rows.
pub const MAX_CALLS_LIMIT: usize = 200;
/// How many proof outcomes are held. The ledger window is a day of calls; a
/// bound keeps a shell that checks every row from growing this forever.
const MAX_HELD_PROOFS: usize = 4096;
/// The longest model name passed through as recorded.
const MAX_MODEL_LABEL_CHARS: usize = 128;

pub const PARTY_COMMONS: &str = "commons";
pub const PARTY_WITNESS: &str = "witness";
pub const PARTY_NEAR_AI: &str = "near_ai";
pub const UNKNOWN: &str = "unknown";

pub const ROUTE_NEAR_AI: &str = "near_ai";
pub const ROUTE_OUTSIDE: &str = "outside";

pub const PROOF_VERIFIED: &str = "verified";
pub const PROOF_NONE: &str = "none";
pub const PROOF_UNVERIFIABLE: &str = "unverifiable";
pub const PROOF_UNCHECKED: &str = "unchecked";

pub const BASIS_PRIVATE_AI: &str = "private_ai";
pub const BASIS_TOOL_DEFAULT: &str = "tool_default";
pub const BASIS_ANSWERED_ELSEWHERE: &str = "answered_elsewhere";

pub const ERR_LIMIT_INVALID: &str = "limit-invalid";
pub const ERR_CURSOR_INVALID: &str = "cursor-invalid";
pub const ERR_CALL_ID_INVALID: &str = "call-id-invalid";
pub const ERR_CALL_UNKNOWN: &str = "call-unknown";
pub const ERR_LEDGER_UNREADABLE: &str = "routing-ledger-unreadable";

/// The ledger's id for NEAR AI's backend. Upstream's own spelling; the same
/// id `IronWireLedger` reads the credential state under.
const NEARAI_BACKEND_ID: &str = "nearai";

/// One tool the map draws.
///
/// `vendor` is where the tool answers when nothing here routes it: its
/// default provider, a fixed label. **K2 adds an "answers at" label to tool
/// discovery on `k2-tool-detection`; this table is the local stand-in until
/// the two merge, and must then read K2's value rather than keep its own.**
/// A tool that talks to many providers has no default and reads `unknown`.
struct ToolSpec {
    source: &'static str,
    harness: Option<&'static str>,
    name: &'static str,
    vendor: &'static str,
}

const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        source: crate::source::SOURCE_CLAUDE_CODE,
        harness: Some("claude"),
        name: "Claude Code",
        vendor: "anthropic",
    },
    ToolSpec {
        source: crate::source::SOURCE_CODEX,
        harness: Some("codex"),
        name: "Codex",
        vendor: "openai",
    },
    ToolSpec {
        source: crate::source::SOURCE_GEMINI_CLI,
        harness: None,
        name: "Gemini CLI",
        vendor: "google",
    },
    ToolSpec {
        source: crate::source::SOURCE_CLINE,
        harness: None,
        name: "Cline",
        vendor: UNKNOWN,
    },
    ToolSpec {
        source: crate::source::SOURCE_OPENCODE,
        harness: None,
        name: "OpenCode",
        vendor: UNKNOWN,
    },
];

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

/// Who answers one tool's model calls, and on what basis.
///
/// `private_ai` is the label `private_inference_state.state` carries. Only
/// `running` -- this daemon's proxy, answering from the NEAR AI account --
/// routes a connected tool to NEAR AI. `running_answered_elsewhere` is the
/// proxy forwarding with the tool's own credentials, so the vendor answers.
/// Every other state with a connected config is `unknown`: a config naming a
/// port whose proxy is stopping, failed, or is not this daemon's cannot be
/// said to reach anyone in particular.
#[must_use]
pub fn model_calls_to(
    vendor: &'static str,
    harness: Option<HarnessLink>,
    private_ai: &str,
) -> (&'static str, &'static str) {
    match harness {
        Some(link) if link.connected => match private_ai {
            super::private_inference::LABEL_RUNNING => (PARTY_NEAR_AI, BASIS_PRIVATE_AI),
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
            let (to, basis) = model_calls_to(spec.vendor, link, facts.private_ai);
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
    let facts = DestinationFacts {
        route,
        private_ai: shared.private_inference_label(),
        watched,
        declarations,
        harness,
        folders,
    };
    Response::ok(req.id, destinations(&facts))
}

/// A recorded model name, or `unknown` when it is not shaped like one.
///
/// The name comes from a proxy the contributor can patch. It is passed
/// through as recorded only when it is short and made of the characters a
/// model id uses; anything else -- a URL, a sentence, control characters --
/// is `unknown` rather than a string a shell renders.
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

/// Whether the call was routed through NEAR AI, or answered outside it.
///
/// Outside is everything that is not NEAR AI's backend, including a backend
/// this build does not recognise: a call is drawn red unless the ledger says
/// NEAR AI answered it.
#[must_use]
pub fn route_label(row: &RoutedExchange) -> &'static str {
    if row.backend == NEARAI_BACKEND_ID {
        ROUTE_NEAR_AI
    } else {
        ROUTE_OUTSIDE
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

/// The outcome of a receipt check this process ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofOutcome {
    /// The receipt was fetched, its signature and both digests verified
    /// against the call's own bytes, and its signer is attested.
    Verified,
    /// The provider holds no receipt for this call.
    None,
    /// A receipt could not be verified; the label says why.
    Unverifiable(&'static str),
}

impl ProofOutcome {
    fn label(self) -> &'static str {
        match self {
            Self::Verified => PROOF_VERIFIED,
            Self::None => PROOF_NONE,
            Self::Unverifiable(_) => PROOF_UNVERIFIABLE,
        }
    }

    fn reason(self) -> Option<&'static str> {
        match self {
            Self::Unverifiable(reason) => Some(reason),
            _ => None,
        }
    }
}

/// Receipt checks this daemon has run, keyed by a digest of the row.
///
/// In memory only: a restart forgets them and every row reads `unchecked`
/// again, which is the honest direction. The key covers the row id, the
/// provider identifier and both body digests, so a row that changed under
/// the same id does not inherit an answer.
#[derive(Debug, Default)]
pub struct ProofCache {
    held: Mutex<(HashMap<[u8; 32], ProofOutcome>, VecDeque<[u8; 32]>)>,
}

fn proof_key(row: &RoutedExchange) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(row.id.unwrap_or(i64::MIN).to_be_bytes());
    for part in [
        row.upstream_id.as_deref(),
        row.request_sha256.as_deref(),
        row.response_sha256.as_deref(),
    ] {
        hasher.update([0u8]);
        hasher.update(part.unwrap_or("").as_bytes());
    }
    hasher.finalize().into()
}

impl ProofCache {
    fn get(&self, row: &RoutedExchange) -> Option<ProofOutcome> {
        let held = self.held.lock().ok()?;
        held.0.get(&proof_key(row)).copied()
    }

    /// Record an outcome for a row.
    pub fn record(&self, row: &RoutedExchange, outcome: ProofOutcome) {
        let Ok(mut held) = self.held.lock() else {
            return;
        };
        let key = proof_key(row);
        if held.0.insert(key, outcome).is_none() {
            held.1.push_back(key);
        }
        while held.1.len() > MAX_HELD_PROOFS {
            if let Some(oldest) = held.1.pop_front() {
                held.0.remove(&oldest);
            }
        }
    }
}

/// What the ledger row alone says about a proof, before any check.
///
/// `Some` when the answer needs no network: an outside call, a call with no
/// provider identifier, or another provider's identifier has no receipt
/// (`none`); a NEAR AI call whose bodies or digests were not recorded cannot
/// be checked by this client (`unverifiable`). `None` means a check could
/// run.
fn static_proof(row: &RoutedExchange) -> Option<&'static str> {
    if route_label(row) != ROUTE_NEAR_AI {
        return Some(PROOF_NONE);
    }
    let Some(upstream) = row.upstream_id.as_deref() else {
        return Some(PROOF_NONE);
    };
    if classify_upstream_id(upstream) == ProviderIdentifier::Foreign {
        return Some(PROOF_NONE);
    }
    if row.body_ref.is_none() || row.request_sha256.is_none() || row.response_sha256.is_none() {
        return Some(PROOF_UNVERIFIABLE);
    }
    None
}

/// The proof status a listed row shows.
///
/// `verified` only from [`ProofCache`], which only [`outcome_for`] writes.
#[must_use]
pub fn listed_proof(row: &RoutedExchange, proofs: &ProofCache) -> &'static str {
    if let Some(label) = static_proof(row) {
        return label;
    }
    proofs.get(row).map_or(PROOF_UNCHECKED, ProofOutcome::label)
}

/// Turn a fetch into an outcome, verifying the receipt against the call.
///
/// The verification is [`crate::routing::receipt::receipt_matches_call`],
/// run here whatever the fetch already did, so `verified` never rests on a
/// flag somebody else set. The second element is whether the outcome is
/// settled enough to hold: an unreachable provider is not.
#[must_use]
pub fn outcome_for(
    fetched: Result<trace_commons_attestation::receipt::ReceiptPayload, ReceiptFetchError>,
    call: &AttestedCall,
) -> (ProofOutcome, bool) {
    match fetched {
        Err(ReceiptFetchError::ReceiptNotFound) => (ProofOutcome::None, true),
        Err(error) => {
            let settled = matches!(
                error,
                ReceiptFetchError::ReceiptUnverified | ReceiptFetchError::SignerNotAttested
            );
            (ProofOutcome::Unverifiable(error.label()), settled)
        }
        Ok(payload) => {
            let Some(model) = call.served_model() else {
                return (
                    ProofOutcome::Unverifiable(ReceiptFetchError::NotConfigured.label()),
                    false,
                );
            };
            match crate::routing::receipt::receipt_matches_call(&payload, call, model) {
                Ok(_) => (ProofOutcome::Verified, true),
                Err(error) => (ProofOutcome::Unverifiable(error.label()), true),
            }
        }
    }
}

/// A position in the newest-first ordering: `<unix millis>:<row id>`, the
/// id `-` for a row the proxy gave none.
fn sort_key(row: &RoutedExchange) -> (i64, i64) {
    (
        row.started_at.timestamp_millis(),
        row.id.unwrap_or(i64::MIN),
    )
}

fn encode_cursor(key: (i64, i64)) -> String {
    if key.1 == i64::MIN {
        format!("{}:-", key.0)
    } else {
        format!("{}:{}", key.0, key.1)
    }
}

fn decode_cursor(cursor: &str) -> Option<(i64, i64)> {
    let (at, id) = cursor.split_once(':')?;
    let at: i64 = at.parse().ok()?;
    let id = if id == "-" {
        i64::MIN
    } else {
        id.parse().ok()?
    };
    Some((at, id))
}

/// Which tool made a call, or `unknown`.
///
/// The ledger records a protocol family, not a tool. A call is named only
/// when exactly one connected tool speaks its family -- the rule
/// `harness_list` applies to `answering`.
fn attribute(facade: &str, harness: &[HarnessRow]) -> &'static str {
    let mut speakers = harness
        .iter()
        .filter(|row| row.connected && row.family == Some(facade));
    match (speakers.next(), speakers.next()) {
        (Some(only), None) => source_for_harness(&only.id).unwrap_or(UNKNOWN),
        _ => UNKNOWN,
    }
}

/// One page of calls, newest first.
///
/// Returns the rows and the cursor for the next page, `None` on the last.
#[must_use]
pub fn calls_page(
    rows: &[RoutedExchange],
    harness: &[HarnessRow],
    proofs: &ProofCache,
    before: Option<(i64, i64)>,
    limit: usize,
) -> (Vec<serde_json::Value>, Option<String>) {
    let mut ordered: Vec<&RoutedExchange> = rows
        .iter()
        .filter(|row| before.is_none_or(|cursor| sort_key(row) < cursor))
        .collect();
    ordered.sort_by_key(|row| std::cmp::Reverse(sort_key(row)));
    let more = ordered.len() > limit;
    ordered.truncate(limit);
    let next = if more {
        ordered.last().map(|row| encode_cursor(sort_key(row)))
    } else {
        None
    };
    let calls = ordered
        .into_iter()
        .map(|row| {
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
                "proof": listed_proof(row, proofs),
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
    let (calls, next) = calls_page(&rows, &harness, &shared.inference_proofs, before, limit);
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

fn proof_answer(req: &Request, id: i64, proof: &str, reason: Option<&str>) -> Response {
    Response::ok(
        req.id,
        serde_json::json!({ "id": id, "proof": proof, "reason": reason }),
    )
}

/// `inference_call_proof`: fetch and verify one call's receipt.
///
/// Asks the provider for the receipt (which tells it this call is being
/// looked at -- the same disclosure `routing::receipt` documents for the
/// submission path) and a freshly-nonced attestation report. The call's
/// bodies are read from the proxy's store and hashed in this process; they
/// never cross the socket and are never sent anywhere.
pub async fn handle_call_proof(shared: &DaemonShared, req: &Request) -> Response {
    let Some(id) = req.params.get("id").and_then(serde_json::Value::as_i64) else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_CALL_ID_INVALID);
    };
    let Some(rows) = ledger_rows(shared) else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERR_LEDGER_UNREADABLE);
    };
    let Some(row) = rows.into_iter().find(|row| row.id == Some(id)) else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_CALL_UNKNOWN);
    };
    if let Some(label) = static_proof(&row) {
        return proof_answer(req, id, label, None);
    }
    if let Some(outcome) = shared.inference_proofs.get(&row) {
        return proof_answer(req, id, outcome.label(), outcome.reason());
    }
    let Some(dir) = shared.routing_bodies_dir() else {
        return proof_answer(req, id, PROOF_UNVERIFIABLE, Some("bodies_unavailable"));
    };
    let Ok(call) = crate::routing::attested::attested_final_call(std::slice::from_ref(&row), &dir)
    else {
        return proof_answer(req, id, PROOF_UNVERIFIABLE, Some("bodies_unavailable"));
    };
    let cfg = match shared.store.load_config() {
        Ok(Some(cfg)) => cfg,
        _ => {
            return proof_answer(
                req,
                id,
                PROOF_UNVERIFIABLE,
                Some(ReceiptFetchError::NotConfigured.label()),
            );
        }
    };
    let fetched = crate::routing::receipt::receipt_for_attested_call(
        cfg.inference_receipt_endpoint.as_deref(),
        &crate::config::config_allowlist(&cfg),
        &call,
        // Always: `verified` on this surface means the signer is attested,
        // whatever the submission path is configured to check.
        true,
    )
    .await;
    let (outcome, settled) = outcome_for(fetched, &call);
    if settled {
        shared.inference_proofs.record(&row, outcome);
    }
    proof_answer(req, id, outcome.label(), outcome.reason())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::disclosure::Route;
    use crate::routing::attested::attested_final_call;
    use chrono::TimeZone;

    const REQUEST: &str = "{\"model\":\"Qwen/Qwen3.6-27B-FP8\",\"messages\":[\"PROMPT-SECRET\"]}";
    const RESPONSE: &str = "data: {\"text\":\"RESPONSE-SECRET\"}\n\ndata: [DONE]\n\n";
    const MODEL: &str = "Qwen/Qwen3.6-27B-FP8";
    const HOSTED_ID: &str = "0123456789abcdef0123456789abcdef";

    fn hex_sha(s: &str) -> String {
        hex::encode(Sha256::digest(s.as_bytes()))
    }

    fn row(id: i64, offset: i64, backend: &str) -> RoutedExchange {
        RoutedExchange {
            id: Some(id),
            started_at: Utc.timestamp_opt(1_800_000_000 + offset, 0).unwrap(),
            client_session_id: Some("SESSION-SECRET".to_string()),
            total_ms: Some(10),
            facade: "openai".to_string(),
            backend: backend.to_string(),
            requested_model: Some(MODEL.to_string()),
            served_model: Some(MODEL.to_string()),
            upstream_id: Some(HOSTED_ID.to_string()),
            request_sha256: Some(hex_sha(REQUEST)),
            response_sha256: Some(hex_sha(RESPONSE)),
            body_ref: Some("00000000000000000009-000000".to_string()),
            rung: "full".to_string(),
            attempts: 1,
            input_tokens: Some(1),
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: Some(1),
            cost_usd: Some(0.0123),
            status: 200,
        }
    }

    fn body_store(row: &RoutedExchange) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let reference = row.body_ref.as_deref().unwrap();
        std::fs::write(dir.path().join(format!("{reference}.req")), REQUEST).unwrap();
        std::fs::write(dir.path().join(format!("{reference}.res")), RESPONSE).unwrap();
        dir
    }

    /// A receipt signed with a real ed25519 key over the call's digests.
    fn signed_receipt(text: &str) -> trace_commons_attestation::receipt::ReceiptPayload {
        use ring::signature::KeyPair as _;
        let key = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[7u8; 32]).unwrap();
        trace_commons_attestation::receipt::ReceiptPayload {
            text: text.to_string(),
            signature: hex::encode(key.sign(text.as_bytes()).as_ref()),
            signing_address: hex::encode(key.public_key().as_ref()),
            signing_algo: trace_commons_attestation::receipt::ReceiptAlgo::Ed25519,
            signature_kind: trace_commons_attestation::receipt::ReceiptSignatureKind::ProviderTee,
        }
    }

    fn facts(private_ai: &'static str, connected: bool) -> DestinationFacts {
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
                        connected: false,
                        wired: false,
                    },
                ),
            ],
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

    #[test]
    fn private_ai_on_routes_a_connected_tool_to_near_ai_and_off_leaves_it_at_its_vendor() {
        let on = destinations(&facts(super::super::private_inference::LABEL_RUNNING, true));
        assert_eq!(tool(&on, "claude-code")["model_calls"]["to"], "near_ai");
        assert_eq!(
            tool(&on, "claude-code")["model_calls"]["basis"],
            "private_ai"
        );
        // Not connected: Private AI on does not route it.
        assert_eq!(tool(&on, "codex")["model_calls"]["to"], "openai");
        assert_eq!(tool(&on, "gemini-cli")["model_calls"]["to"], "google");

        // Connected, but Private AI off: never NEAR AI.
        let off_connected = destinations(&facts(super::super::private_inference::LABEL_OFF, true));
        assert_ne!(
            tool(&off_connected, "claude-code")["model_calls"]["to"],
            "near_ai"
        );
        let off = destinations(&facts(super::super::private_inference::LABEL_OFF, false));
        assert_eq!(tool(&off, "claude-code")["model_calls"]["to"], "anthropic");
        assert_eq!(
            tool(&off, "claude-code")["model_calls"]["basis"],
            "tool_default"
        );
        assert_eq!(off["private_ai"], "off");
    }

    #[test]
    fn a_connected_tool_whose_proxy_cannot_say_who_answers_is_unknown() {
        for label in [
            super::super::private_inference::LABEL_OFF,
            super::super::private_inference::LABEL_RUNNING_DESTINATION_UNKNOWN,
            super::super::private_inference::LABEL_RUNNING_ELSEWHERE,
            super::super::private_inference::LABEL_CRASHED,
        ] {
            let value = destinations(&facts(label, true));
            assert_eq!(
                tool(&value, "claude-code")["model_calls"]["to"],
                "unknown",
                "{label}"
            );
        }
        let elsewhere = destinations(&facts(
            super::super::private_inference::LABEL_RUNNING_ANSWERED_ELSEWHERE,
            true,
        ));
        assert_eq!(
            tool(&elsewhere, "claude-code")["model_calls"]["to"],
            "anthropic"
        );
        // Wired to somebody else's proxy.
        let foreign = model_calls_to(
            "anthropic",
            Some(HarnessLink {
                connected: false,
                wired: true,
            }),
            super::super::private_inference::LABEL_RUNNING,
        );
        assert_eq!(foreign, (UNKNOWN, UNKNOWN));
        // A tool with no default provider stays unknown.
        assert_eq!(tool(&elsewhere, "cline")["model_calls"]["to"], "unknown");
    }

    #[test]
    fn sessions_go_where_the_route_sends_them_and_nowhere_when_unwatched() {
        let value = destinations(&facts(super::super::private_inference::LABEL_OFF, false));
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
    fn an_outside_call_is_marked_outside_with_no_proof() {
        let proofs = ProofCache::default();
        let outside = row(1, 0, "claude-sub");
        let (calls, _) = calls_page(std::slice::from_ref(&outside), &[], &proofs, None, 10);
        assert_eq!(calls[0]["route"], "outside");
        assert_eq!(calls[0]["proof"], "none");
        // Even a recorded "verified" cannot promote an outside call.
        proofs.record(&outside, ProofOutcome::Verified);
        let (calls, _) = calls_page(&[outside], &[], &proofs, None, 10);
        assert_eq!(calls[0]["proof"], "none");
        let (calls, _) = calls_page(&[row(2, 0, "nearai")], &[], &proofs, None, 10);
        assert_eq!(calls[0]["route"], "near_ai");
    }

    #[test]
    fn a_routed_call_is_verified_only_after_its_receipt_verifies() {
        let proofs = ProofCache::default();
        let routed = row(3, 0, "nearai");
        let dir = body_store(&routed);
        let call = attested_final_call(std::slice::from_ref(&routed), dir.path()).unwrap();

        let (calls, _) = calls_page(std::slice::from_ref(&routed), &[], &proofs, None, 10);
        assert_eq!(calls[0]["proof"], "unchecked");

        let good = signed_receipt(&format!(
            "{MODEL}:{}:{}",
            hex_sha(REQUEST),
            hex_sha(RESPONSE)
        ));
        let (outcome, settled) = outcome_for(Ok(good), &call);
        assert_eq!(outcome, ProofOutcome::Verified);
        assert!(settled);
        proofs.record(&routed, outcome);
        let (calls, _) = calls_page(std::slice::from_ref(&routed), &[], &proofs, None, 10);
        assert_eq!(calls[0]["proof"], "verified");

        // A validly signed receipt over other bytes is not a proof.
        let other = signed_receipt(&format!("{MODEL}:{}:{}", hex_sha("x"), hex_sha(RESPONSE)));
        let (outcome, _) = outcome_for(Ok(other), &call);
        assert_eq!(
            outcome,
            ProofOutcome::Unverifiable(ReceiptFetchError::ReceiptUnverified.label())
        );
        assert_eq!(
            outcome_for(Err(ReceiptFetchError::ReceiptNotFound), &call),
            (ProofOutcome::None, true)
        );
        assert!(!outcome_for(Err(ReceiptFetchError::Unreachable), &call).1);
    }

    #[test]
    fn a_routed_call_without_a_receipt_shows_none_and_one_without_bodies_unverifiable() {
        let proofs = ProofCache::default();
        let mut no_id = row(4, 0, "nearai");
        no_id.upstream_id = None;
        let mut brokered = row(5, 1, "nearai");
        brokered.upstream_id = Some("chatcmpl-abc".to_string());
        let mut no_bodies = row(6, 2, "nearai");
        no_bodies.body_ref = None;
        let (calls, _) = calls_page(&[no_id, brokered, no_bodies], &[], &proofs, None, 10);
        assert_eq!(calls[0]["proof"], "unverifiable");
        assert_eq!(calls[1]["proof"], "none");
        assert_eq!(calls[2]["proof"], "none");
    }

    #[test]
    fn pages_are_newest_first_bounded_and_resume_at_the_cursor() {
        let proofs = ProofCache::default();
        let rows: Vec<RoutedExchange> = (0..7).map(|i| row(i, i * 10, "nearai")).collect();
        let (first, next) = calls_page(&rows, &[], &proofs, None, 3);
        let ids: Vec<i64> = first.iter().map(|c| c["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![6, 5, 4]);
        let cursor = decode_cursor(next.as_deref().unwrap()).unwrap();
        let (second, next) = calls_page(&rows, &[], &proofs, Some(cursor), 3);
        let ids: Vec<i64> = second.iter().map(|c| c["id"].as_i64().unwrap()).collect();
        assert_eq!(ids, vec![3, 2, 1]);
        let cursor = decode_cursor(next.as_deref().unwrap()).unwrap();
        let (last, next) = calls_page(&rows, &[], &proofs, Some(cursor), 3);
        assert_eq!(last.len(), 1);
        assert_eq!(next, None);
        assert_eq!(decode_cursor("nonsense"), None);
    }

    #[test]
    fn no_body_identifier_or_session_reaches_the_output() {
        let proofs = ProofCache::default();
        let mut hostile = row(8, 0, "nearai");
        hostile.served_model = Some("https://evil.example/PROMPT-SECRET".to_string());
        let rows = vec![row(7, 0, "nearai"), hostile];
        let (calls, _) = calls_page(&rows, &[], &proofs, None, 10);
        let text = serde_json::to_string(&calls).unwrap();
        for forbidden in [
            "PROMPT-SECRET",
            "RESPONSE-SECRET",
            "SESSION-SECRET",
            HOSTED_ID,
            "00000000000000000009-000000",
            &hex_sha(REQUEST),
            &hex_sha(RESPONSE),
            "://",
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

    fn recent(id: i64, backend: &str) -> RoutedExchange {
        let mut r = row(id, 0, backend);
        r.started_at = Utc::now() - chrono::Duration::minutes(60 - id);
        r
    }

    #[test]
    fn the_methods_are_advertised_and_answer_over_the_dispatcher() {
        for method in [
            "tool_destinations",
            "inference_calls",
            "inference_call_proof",
        ] {
            assert!(super::super::ipc::METHODS.contains(&method), "{method}");
        }
        let (_dir, s) = shared();
        let r = super::super::ipc::handle_request(
            &s,
            &call("tool_destinations", serde_json::json!({})),
        );
        let value = r.result.expect("destinations");
        assert_eq!(value["private_ai"], "off");
        assert_eq!(value["sessions_route"], "not_enrolled");
        // Not enrolled: nothing is sent anywhere, whatever is watched.
        for t in value["tools"].as_array().unwrap() {
            assert_eq!(t["sessions"]["to"], serde_json::json!([]));
        }

        let r =
            super::super::ipc::handle_request(&s, &call("inference_calls", serde_json::json!({})));
        let value = r.result.expect("calls");
        assert_eq!(value["readable"], false);
        let r = super::super::ipc::handle_request(
            &s,
            &call("inference_call_proof", serde_json::json!({"id": 1})),
        );
        assert_eq!(
            r.error.unwrap().message,
            "inference-call-proof-requires-async"
        );
    }

    #[test]
    fn inference_calls_pages_the_held_ledger_and_refuses_a_bad_limit_or_cursor() {
        let (_dir, s) = shared();
        s.install_routing_ledger_for_test(
            crate::routing::ironwire::IronWireLedger::with_rows_for_test(
                (1..=5).map(|id| recent(id, "nearai")).collect(),
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

    #[tokio::test]
    async fn a_proof_check_answers_from_the_row_without_the_network_when_it_can() {
        let (_dir, s) = shared();
        let mut no_receipt = recent(2, "nearai");
        no_receipt.upstream_id = None;
        s.install_routing_ledger_for_test(
            crate::routing::ironwire::IronWireLedger::with_rows_for_test(vec![
                recent(1, "claude-sub"),
                no_receipt,
                recent(3, "nearai"),
            ]),
        );
        let ask =
            |id: serde_json::Value| call("inference_call_proof", serde_json::json!({ "id": id }));
        let r = super::super::ipc::handle_request_async(&s, &ask(1.into())).await;
        assert_eq!(r.result.unwrap()["proof"], "none");
        let r = super::super::ipc::handle_request_async(&s, &ask(2.into())).await;
        assert_eq!(r.result.unwrap()["proof"], "none");
        // No proxy body store is known here, so the check cannot run.
        let r = super::super::ipc::handle_request_async(&s, &ask(3.into())).await;
        let answer = r.result.unwrap();
        assert_eq!(answer["proof"], "unverifiable");
        assert_eq!(answer["reason"], "bodies_unavailable");
        let r = super::super::ipc::handle_request_async(&s, &ask(99.into())).await;
        assert_eq!(r.error.unwrap().message, ERR_CALL_UNKNOWN);
        let r = super::super::ipc::handle_request_async(&s, &ask("3".into())).await;
        assert_eq!(r.error.unwrap().message, ERR_CALL_ID_INVALID);
    }
}
