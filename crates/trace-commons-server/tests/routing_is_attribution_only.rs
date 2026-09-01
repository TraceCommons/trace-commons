// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Routing metadata is corpus metadata and nothing else.
//!
//! `cost_usd` and the token counts arrive from a local proxy the contributor
//! controls: it is Apache-2.0 and they can build their own. If either ever
//! reaches a scoring or credit input, a patched proxy reporting inflated costs
//! is a direct credit-farming vector.
//!
//! There is a real barrier today: the gate renders envelopes through a
//! three-field allowlist (`event_type`, `tool_name`, `redacted_content`) --
//! `trace_commons_gate_enclave::chunker::parse_envelope_rendered_events` --
//! and that allowlist is what both the perplexity scorer and the dedup
//! simhash consume. Numeric fields, including `cost_usd`, are structurally
//! unreachable through it. But the allowlist exists for signal quality, not
//! for this rule, and `chunk_envelope_plaintext` falls back to chunking the
//! RAW envelope JSON when an envelope has no renderable events (empty
//! `events` array, or no `events` key at all). So the property is a side
//! effect of two decisions made for other reasons, and this file is what
//! makes it a stated rule with a test behind it -- including the part of it
//! that is still a real hole (see the last test below).
//!
//! This test lives in `trace-commons-server` rather than the permissive
//! `trace-commons-contributor` crate. The server crate is itself
//! AGPL-3.0-or-later and already depends on `trace-commons-gate-enclave`
//! (`crates/trace-commons-server/Cargo.toml`), so it can assert against the
//! real renderer instead of a same-crate proxy for it.

use std::collections::BTreeMap;

use chrono::Utc;
use serde_json::Value;
use trace_commons_gate_enclave::chunker::{
    ChunkerConfig, chunk_envelope_plaintext, parse_envelope_rendered_events,
};
use trace_commons_protocol::trace_contribution::{
    ConsentMetadata, ConsentScope, ContributorMetadata, DETERMINISTIC_REDACTION_PIPELINE_VERSION,
    IronclawTraceMetadata, OutcomeMetadata, PrivacyMetadata, ReplayMetadata, ResidualPiiRisk,
    SideEffectLevel, TRACE_CONTRIBUTION_POLICY_VERSION, TRACE_CONTRIBUTION_SCHEMA_VERSION,
    TraceCard, TraceChannel, TraceContributionEnvelope, TraceContributionEvent,
    TraceContributionEventType, TraceValueCard, ValueMetadata, derive_envelope_content_presence,
};
use uuid::Uuid;

/// A cost no real ledger would produce, so a match in rendered text is
/// unambiguous. Chosen to have a digit run (`133713`) that cannot appear by
/// coincidence in any of the envelope's other fields.
const SENTINEL_COST_TEXT: &str = "133713.37";

/// Build a minimal, otherwise-empty envelope carrying exactly the events
/// given. Mirrors the shape production code emits (see
/// `crates/trace-commons-protocol/src/trace_contribution.rs`,
/// `sample_envelope_with_event_content` in its own test module) -- this file
/// cannot reuse that helper because it is private to that crate's `#[cfg(test)]`
/// module, so the fields are restated here instead of a second builder living
/// alongside it.
fn envelope_with_events(events: Vec<TraceContributionEvent>) -> TraceContributionEnvelope {
    let now = Utc::now();
    TraceContributionEnvelope {
        schema_version: TRACE_CONTRIBUTION_SCHEMA_VERSION.to_string(),
        trace_id: Uuid::new_v4(),
        submission_id: Uuid::new_v4(),
        created_at: now,
        ironclaw: IronclawTraceMetadata {
            version: "1".to_string(),
            engine_version: None,
            feature_flags: BTreeMap::new(),
            channel: TraceChannel::Cli,
            model_name: None,
        },
        consent: ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included: false,
            tool_payloads_included: false,
            correction_included: false,
            routing_metadata_included: true,
            revocable: true,
        },
        contributor: ContributorMetadata {
            pseudonymous_contributor_id: None,
            tenant_scope_ref: None,
            credit_account_ref: None,
            revocation_handle: Uuid::new_v4(),
        },
        privacy: PrivacyMetadata {
            redaction_pipeline_version: DETERMINISTIC_REDACTION_PIPELINE_VERSION.to_string(),
            redaction_counts: BTreeMap::new(),
            privacy_filter_summary: None,
            pii_labels_present: Vec::new(),
            residual_pii_risk: ResidualPiiRisk::Low,
            redaction_hash: "sha256:placeholder".to_string(),
            warnings: Vec::new(),
        },
        events,
        outcome: OutcomeMetadata::default(),
        replay: ReplayMetadata {
            replayable: false,
            required_tools: Vec::new(),
            tool_manifest_hashes: BTreeMap::new(),
            expected_assertions: Vec::new(),
            replay_notes: Vec::new(),
        },
        embedding_analysis: None,
        value: ValueMetadata::default(),
        conversation_id: None,
        trace_card: TraceCard::default(),
        value_card: TraceValueCard::default(),
        hindsight: None,
        training_dynamics: None,
        process_evaluation: None,
    }
}

/// One `RoutingDecision` event carrying the sentinel cost. Matches the real
/// shape `raw_routing_event_for` in `trace-commons-contributor`'s
/// `envelope.rs` produces: the numeric cost/token fields go in this event's
/// own typed fields (`cost_usd`), and `structured_payload` carries only the
/// unstructured labels (backend, rung, ...) that have no typed home. The
/// sentinel is placed in BOTH here, so this test pins the property regardless
/// of which of the two a future change moves it between.
fn routing_event_with_sentinel_cost() -> TraceContributionEvent {
    TraceContributionEvent {
        event_id: Uuid::new_v4(),
        parent_event_id: None,
        event_type: TraceContributionEventType::RoutingDecision,
        timestamp: Utc::now(),
        redacted_content: None,
        structured_payload: serde_json::json!({
            "backend": "nearai",
            "rung": "same_model",
            "cost_usd": SENTINEL_COST_TEXT,
        }),
        tool_name: None,
        tool_category: None,
        tool_call_id: None,
        latency_ms: Some(1200),
        token_counts: None,
        cost_usd: Some(
            SENTINEL_COST_TEXT
                .parse()
                .expect("sentinel cost parses as a Decimal"),
        ),
        success: None,
        failure_modes: Vec::new(),
        side_effect: SideEffectLevel::None,
    }
}

fn envelope_with_routing_cost() -> TraceContributionEnvelope {
    envelope_with_events(vec![routing_event_with_sentinel_cost()])
}

fn cfg() -> ChunkerConfig {
    ChunkerConfig {
        target_tokens: 2048,
        max_tokens: 3072,
        chunk_cap: 16,
    }
}

#[test]
fn a_routing_cost_never_appears_in_the_text_the_gate_renders() {
    let envelope = envelope_with_routing_cost();
    let plaintext = serde_json::to_vec(&envelope).expect("envelope serializes");

    // Sanity: the sentinel really is present in the raw envelope, in both
    // places it might travel (the typed field and the structured payload).
    // If this assertion ever fails, the fixture stopped exercising the case
    // it claims to.
    let raw = String::from_utf8_lossy(&plaintext);
    assert!(
        raw.contains("133713"),
        "fixture sanity check: sentinel cost must be present in the raw envelope:\n{raw}"
    );

    let rendered_events =
        parse_envelope_rendered_events(&plaintext).expect("envelope has one renderable event");
    let rendered = rendered_events.join("");
    assert!(
        !rendered.contains("133713"),
        "the scored text must not carry a routing cost:\n{rendered}"
    );

    let plan = chunk_envelope_plaintext(&plaintext, &cfg());
    let chunked: String = plan.chunks.iter().map(|c| c.text.as_str()).collect();
    assert!(
        !chunked.contains("133713"),
        "chunks built from a renderable envelope must not carry a routing cost:\n{chunked}"
    );
}

#[test]
fn a_routing_event_does_not_declare_a_tool_payload() {
    let envelope = envelope_with_routing_cost();
    let presence = derive_envelope_content_presence(&envelope);
    assert!(presence.routing_metadata, "declared as routing metadata");
    assert!(
        !presence.tool_payloads,
        "declaring routing as a tool payload quarantines the envelope"
    );
}

/// KNOWN GAP -- not fixed by this task, only pinned by it.
///
/// `parse_envelope_rendered_events` returns `None` (and `chunk_envelope_plaintext`
/// falls back to chunking the raw envelope JSON verbatim) whenever the
/// plaintext is not JSON, has no `events` key, or has an EMPTY `events`
/// array (`chunker.rs`, `parse_envelope_rendered_events`). An envelope with a
/// `RoutingDecision` event and nothing else does not hit this: one event is
/// enough to make `events` non-empty, so the allowlist path is taken and the
/// two tests above are the property that actually protects a normal
/// contribution.
///
/// The fallback is reachable, though, whenever a decision-worthy envelope
/// ends up with an empty `events` array by the time it reaches the chunker --
/// e.g. every event redacted away by a privacy pass but the envelope (and its
/// routing-only consent flag) still submitted. On that path the sentinel
/// cost is NOT scrubbed: the raw JSON, cost included, is exactly what gets
/// chunked and handed to the scorer and the dedup simhash.
///
/// This test does not assert the sentinel is absent on the fallback path --
/// it is present, and asserting otherwise would misrepresent what the code
/// does. It demonstrates the fallback is taken and that the sentinel survives
/// it, so the gap stays visible instead of silently "proven" closed.
/// Fixing this (refuse an envelope with no renderable events, or strip
/// metadata on the fallback path) is out of scope here: it means changing the
/// chunker, which is production code this task must not touch.
#[test]
fn known_gap_the_empty_events_fallback_carries_the_raw_routing_cost() {
    let mut envelope = envelope_with_routing_cost();
    envelope.events.clear();
    // Attach the sentinel somewhere still present in the serialized envelope
    // even with no events -- the contributor block is as good a stand-in as
    // any typed field that can carry a raw string; what matters is that the
    // JSON serialization still contains the sentinel digits with an empty
    // `events` array, which is exactly the condition the fallback checks.
    envelope.contributor.credit_account_ref = Some(format!("sentinel:{SENTINEL_COST_TEXT}"));

    let plaintext = serde_json::to_vec(&envelope).expect("envelope serializes");
    let parsed: Value = serde_json::from_slice(&plaintext).expect("valid json");
    assert_eq!(
        parsed.get("events").and_then(Value::as_array).map(Vec::len),
        Some(0),
        "fixture sanity check: events must be empty to exercise the fallback"
    );

    // The allowlist path is not taken.
    assert!(
        parse_envelope_rendered_events(&plaintext).is_none(),
        "an empty events array must fall through to the raw-text path"
    );

    // The fallback chunks the raw JSON, sentinel and all. This is the gap:
    // not this test's job to close it, only to say so plainly.
    let plan = chunk_envelope_plaintext(&plaintext, &cfg());
    let chunked: String = plan.chunks.iter().map(|c| c.text.as_str()).collect();
    assert!(
        chunked.contains(SENTINEL_COST_TEXT),
        "known gap: the empty-events fallback chunks the raw envelope JSON, \
         so a routing cost that reaches it is NOT scrubbed before scoring:\n{chunked}"
    );
}
