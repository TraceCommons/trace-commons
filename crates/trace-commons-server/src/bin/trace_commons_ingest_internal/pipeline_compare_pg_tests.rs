// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `pipeline.py compare`: the same traces go through the old gate path (side
//! `baseline`) and through the versioned pipeline (side `candidate`), and the
//! decisions are compared for each trace.
//!
//! This part of the harness has no database and no HTTP:
//!
//! - `CompareCorpusReader` reads a corpus one line at a time and hashes the
//!   bytes it reads, so a 10,000-trace corpus never sits in memory.
//! - `compare_envelope` builds the same envelope bytes for one fixture on
//!   every call, so two runs of one pin give one report.
//! - `calibrate_floors` runs the bootstrap partition through `main`'s gate
//!   with floors of zero and derives the three floors from the measured
//!   values.
//!
//! The run takes the traces in serial order, one after the other, because
//! the two indexes must hold the same entries before each trace. Code marked
//! `// baseline-old-path:` applies to the old path only and goes away with
//! it. Nested inside `tests` beside `pipeline_corpus_pg_tests`.

use super::*;

use std::io::BufReader;
use std::path::{Path, PathBuf};

use trace_commons_gate_api::pipeline::TenantStorageRef;
use trace_commons_gate_api::{
    Embedder, IndexEntryKey, NearestNeighbor, ReferenceEmbedder, ReferencePerplexityScorer,
    VectorIndex, VectorIndexReader, VectorIndexWriter,
};
use trace_commons_gate_enclave::{
    EnclaveGateOrchestrator, EnclaveGateOrchestratorConfig, MockVectorIndex,
};
use trace_commons_protocol::llm::recording::{ExpectedToolResult, TraceFile, TraceToolCall};
use trace_commons_protocol::trace_contribution::{
    RawTraceContribution, RecordedTraceContributionOptions, TraceContributionEventType,
};
use trace_commons_server::versioned_pipeline_bundle::{MINIMAL_INDEX_ID, MINIMAL_PROJECTION_ID};
use trace_commons_server::versioned_pipeline_comparison::{DerivedFloors, derive_floors};
use trace_commons_server::versioned_pipeline_compat::{CompatibilityBundleConfig, MainGateConfig};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;

use super::pipeline_corpus_pg_tests::sha256_bytes;

// ---------------------------------------------------------------------------
// The corpus: one fixture on each line.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompareFixture {
    label: String,
    trace_id: Uuid,
    submission_id: Uuid,
    created_at: DateTime<Utc>,
    secret_probe: String,
    privacy_risk: String,
    trace_file: TraceFile,
}

impl CompareFixture {
    fn is_valid(&self) -> bool {
        let label_ok = (1..=64).contains(&self.label.len())
            && self
                .label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
        label_ok
            && !self.secret_probe.is_empty()
            && matches!(self.privacy_risk.as_str(), "low" | "medium" | "high")
    }
}

/// Reads one fixture for each call. It never holds more than one line.
struct CompareCorpusReader {
    reader: BufReader<std::fs::File>,
    partition: &'static str,
    position: u64,
    digest: sha2::Sha256,
    line: Vec<u8>,
}

impl CompareCorpusReader {
    fn open(partition: &'static str, path: &Path) -> Result<Self, &'static str> {
        let file = std::fs::File::open(path).map_err(|_| "compare_corpus_read_failed")?;
        Ok(Self {
            reader: BufReader::new(file),
            partition,
            position: 0,
            digest: sha2::Sha256::new(),
            line: Vec::new(),
        })
    }

    /// Reads the next line into `self.line` and hashes it. False at the end.
    fn read_line(&mut self) -> Result<bool, &'static str> {
        self.line.clear();
        let read = self
            .reader
            .read_until(b'\n', &mut self.line)
            .map_err(|_| "compare_corpus_read_failed")?;
        self.digest.update(&self.line);
        Ok(read > 0)
    }

    fn next_fixture(&mut self) -> Result<Option<CompareFixture>, &'static str> {
        if !self.read_line()? {
            return Ok(None);
        }
        let position = self.position;
        self.position += 1;
        match serde_json::from_slice::<CompareFixture>(&self.line) {
            Ok(fixture) if fixture.is_valid() => Ok(Some(fixture)),
            _ => {
                eprintln!(
                    "compare_corpus_line_invalid partition={} position={position}",
                    self.partition
                );
                Err("compare_corpus_line_invalid")
            }
        }
    }

    /// The digest of the whole file; reads the lines that are left.
    fn finish(mut self) -> Result<String, &'static str> {
        while self.read_line()? {}
        Ok(format!("sha256:{}", hex::encode(self.digest.finalize())))
    }
}

// ---------------------------------------------------------------------------
// The envelope.
// ---------------------------------------------------------------------------

/// The envelope of one fixture. The same fixture gives the same bytes on
/// every call: each field that the protocol crate fills at random or from the
/// clock is a function of the fixture here.
async fn compare_envelope(fixture: &CompareFixture) -> TraceContributionEnvelope {
    // `from_recorded_trace` gives `Utc::now()` to a step with no timestamp.
    let mut trace = fixture.trace_file.clone();
    for step in &mut trace.steps {
        step.timestamp.get_or_insert(fixture.created_at);
    }
    let mut raw = RawTraceContribution::from_recorded_trace(
        &trace,
        RecordedTraceContributionOptions {
            include_message_text: true,
            include_tool_payloads: true,
            pseudonymous_contributor_id: Some("sha256:compare-contributor".to_string()),
            tenant_scope_ref: Some("tenant_sha256:compare".to_string()),
            ..RecordedTraceContributionOptions::default()
        },
    );
    raw.trace_id = fixture.trace_id;
    raw.submission_id = fixture.submission_id;
    raw.created_at = fixture.created_at;
    raw.contributor.revocation_handle = Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("tracecommons:compare-revocation:{}", fixture.label).as_bytes(),
    );
    let mut new_ids = std::collections::BTreeMap::new();
    for (index, event) in raw.events.iter_mut().enumerate() {
        let new_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!("tracecommons:compare-event:{}:{index}", fixture.label).as_bytes(),
        );
        new_ids.insert(event.event_id, new_id);
        event.event_id = new_id;
        // An HTTP exchange event takes the clock time of the call.
        if event.event_type == TraceContributionEventType::HttpExchange {
            event.timestamp = fixture.created_at;
        }
    }
    for event in &mut raw.events {
        event.parent_event_id = event
            .parent_event_id
            .and_then(|parent| new_ids.get(&parent).copied());
    }
    let mut envelope = DeterministicTraceRedactor::try_default()
        .expect("compare_redactor_unavailable")
        .redact_trace(raw)
        .await
        .expect("compare_redaction_failed");
    envelope.privacy.residual_pii_risk = match fixture.privacy_risk.as_str() {
        "low" => ResidualPiiRisk::Low,
        "medium" | "high" => {
            make_metadata_only_low_risk(&mut envelope);
            set_metadata_only_tool_name(&mut envelope, &fixture.label);
            if fixture.privacy_risk == "medium" {
                ResidualPiiRisk::Medium
            } else {
                ResidualPiiRisk::High
            }
        }
        _ => unreachable!("CompareFixture::is_valid refuses any other privacy risk"),
    };
    envelope.consent.scopes = vec![ConsentScope::ModelTraining];
    envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
    envelope.trace_card.allowed_uses = vec![TraceAllowedUse::ModelTraining];
    envelope
}

// ---------------------------------------------------------------------------
// The gate configuration and the calibration.
// ---------------------------------------------------------------------------

/// `main`'s gate configuration for the run: the derived floors and the
/// values of `CompatibilityBundleConfig::local_reference()`.
fn compare_main_gate(floors: DerivedFloors) -> MainGateConfig {
    let reference = CompatibilityBundleConfig::local_reference();
    MainGateConfig {
        perplexity_floor_micros: Some(floors.perplexity_floor_micros),
        tail_fraction_floor_micros: Some(floors.tail_fraction_floor_micros),
        novelty_floor_micros: Some(floors.novelty_floor_micros),
        embed_insert_novelty_micros: reference.embed_insert_novelty_micros,
        top_k: reference.top_k,
        chunk_target_tokens: reference.chunk_target_tokens,
        chunk_max_tokens: reference.chunk_max_tokens,
        chunk_cap: reference.chunk_cap,
        chunk_min_tokens: reference.chunk_min_tokens,
        novelty_utility_microcredits: 2_500_000,
    }
}

// baseline-old-path:
fn baseline_orchestrator_config(gate: &MainGateConfig) -> EnclaveGateOrchestratorConfig {
    let floor = |value: Option<u64>| value.expect("compare_main_gate sets every floor");
    let perplexity_floor_micros = floor(gate.perplexity_floor_micros);
    EnclaveGateOrchestratorConfig {
        gate_policy_version: "compare_baseline_v1".to_string(),
        gate_version_hash: "compare_baseline".to_string(),
        perplexity_floor_micros,
        tail_fraction_floor_micros: floor(gate.tail_fraction_floor_micros),
        novelty_floor_micros: floor(gate.novelty_floor_micros),
        top_k: gate.top_k as usize,
        chunk_target_tokens: gate.chunk_target_tokens as usize,
        chunk_max_tokens: gate.chunk_max_tokens as usize,
        chunk_cap: gate.chunk_cap as usize,
        chunk_min_tokens: gate.chunk_min_tokens,
        embed_insert_novelty_micros: gate.embed_insert_novelty_micros,
        // PC-D7: `main` takes the perplexity floor when no knob is set.
        qualifying_chunk_floor_micros: perplexity_floor_micros,
    }
}

/// Runs the bootstrap partition through `main`'s gate with floors of zero and
/// derives the three floors from the values it measured.
async fn calibrate_floors(bootstrap: &Path) -> Result<DerivedFloors, &'static str> {
    let mut config = baseline_orchestrator_config(&compare_main_gate(DerivedFloors {
        perplexity_floor_micros: 0,
        tail_fraction_floor_micros: 0,
        novelty_floor_micros: 0,
    }));
    config.gate_policy_version = "compare_calibration_v1".to_string();
    let orchestrator = std::sync::Arc::new(EnclaveGateOrchestrator::new(
        ReferencePerplexityScorer::new(),
        ReferenceEmbedder::new(),
        MockVectorIndex::new(),
        config,
    ));
    let mut perplexity = Vec::new();
    let mut tail_fraction = Vec::new();
    let mut novelty = Vec::new();
    let mut reader = CompareCorpusReader::open("bootstrap", bootstrap)?;
    while let Some(fixture) = reader.next_fixture()? {
        let bytes = serde_json::to_vec(&compare_envelope(&fixture).await)
            .map_err(|_| "comparison_calibration_failed")?;
        let gate = std::sync::Arc::clone(&orchestrator);
        let decision =
            tokio::task::spawn_blocking(move || gate.evaluate(&bytes, "compare-calibration"))
                .await
                .map_err(|_| "comparison_calibration_failed")?
                .map_err(|_| "comparison_calibration_failed")?;
        perplexity.push(decision.perplexity_micros);
        tail_fraction.push(decision.tail_fraction_micros);
        novelty.push(decision.novelty_score_micros);
    }
    derive_floors(&perplexity, &tail_fraction, &novelty)
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn the_corpus_reader_is_lazy() {
    let lines = [
        fixture_line("lazy_a", "low", prose_steps("one")),
        fixture_line("lazy_b", "low", prose_steps("two")),
        "this line is not json".to_string(),
    ];
    let (_dir, path) = write_corpus(&lines);
    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    assert!(reader.next_fixture().unwrap().is_some());
    assert!(reader.next_fixture().unwrap().is_some());
    assert_eq!(
        reader.next_fixture().err(),
        Some("compare_corpus_line_invalid")
    );

    // A reader that loaded the file in `open` cannot see a line that is
    // appended after the first read.
    let (_dir, path) = write_corpus(&[fixture_line("lazy_c", "low", prose_steps("three"))]);
    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    assert_eq!(reader.next_fixture().unwrap().unwrap().label, "lazy_c");
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    writeln!(
        file,
        "{}",
        fixture_line("lazy_d", "low", prose_steps("four"))
    )
    .unwrap();
    assert_eq!(reader.next_fixture().unwrap().unwrap().label, "lazy_d");
    assert!(reader.next_fixture().unwrap().is_none());
}

#[test]
fn the_corpus_reader_refuses_bad_fixtures() {
    let good = serde_json::from_str::<serde_json::Value>(&fixture_line(
        "bad_base",
        "low",
        prose_steps("x"),
    ))
    .unwrap();
    let with = |key: &str, value: serde_json::Value| {
        let mut object = good.clone();
        object[key] = value;
        object.to_string()
    };
    let bad_lines = [
        with("unexpected", serde_json::json!(1)),
        with("privacy_risk", serde_json::json!("critical")),
        with("secret_probe", serde_json::json!("")),
        with("label", serde_json::json!("Not A Label")),
        with("label", serde_json::json!("")),
        with("label", serde_json::json!("a".repeat(65))),
    ];
    for line in bad_lines {
        let (_dir, path) = write_corpus(&[line]);
        let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
        assert_eq!(
            reader.next_fixture().err(),
            Some("compare_corpus_line_invalid")
        );
    }
    let (_dir, path) = write_corpus(&[]);
    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    assert!(reader.next_fixture().unwrap().is_none());
    assert_eq!(reader.finish().unwrap(), sha256_bytes(&[]));
}

#[test]
fn the_reader_digest_is_the_file_digest() {
    let lines = [
        fixture_line("digest_a", "low", prose_steps("one")),
        fixture_line("digest_b", "medium", prose_steps("two")),
        fixture_line("digest_c", "high", prose_steps("three")),
    ];
    let (_dir, path) = write_corpus(&lines);
    let expected = sha256_bytes(&std::fs::read(&path).unwrap());

    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    reader.next_fixture().unwrap().unwrap();
    assert_eq!(reader.finish().unwrap(), expected);

    let mut reader = CompareCorpusReader::open("bootstrap", &path).unwrap();
    while reader.next_fixture().unwrap().is_some() {}
    assert_eq!(reader.finish().unwrap(), expected);
}

#[tokio::test]
async fn the_same_fixture_gives_the_same_envelope_bytes() {
    let fixture = fixture_from(fixture_line("same_bytes", "low", tool_steps()));
    let first = compare_envelope(&fixture).await;
    let second = compare_envelope(&fixture).await;
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    assert_eq!(first.trace_id, fixture.trace_id);
    assert_eq!(first.submission_id, fixture.submission_id);
    assert_eq!(first.created_at, fixture.created_at);
    assert!(first.events.len() > 1);
    assert_eq!(first.consent.scopes, vec![ConsentScope::ModelTraining]);
    assert_eq!(
        first.trace_card.allowed_uses,
        vec![TraceAllowedUse::ModelTraining]
    );
    for envelope in [&first, &second] {
        let call_position = envelope
            .events
            .iter()
            .position(|event| event.event_type == TraceContributionEventType::ToolCall)
            .expect("a tool call event");
        let call = &envelope.events[call_position];
        assert_eq!(
            call.event_id,
            Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("tracecommons:compare-event:same_bytes:{call_position}").as_bytes(),
            )
        );
        let result = envelope
            .events
            .iter()
            .find(|event| event.event_type == TraceContributionEventType::ToolResult)
            .expect("a tool result event");
        assert_eq!(result.parent_event_id, Some(call.event_id));
        for event in &envelope.events {
            assert_eq!(event.timestamp, fixture.created_at);
        }
    }
}

#[tokio::test]
async fn a_tool_session_keeps_its_tool_events() {
    let fixture = fixture_from(fixture_line("tool_events", "low", tool_steps()));
    let envelope = compare_envelope(&fixture).await;
    assert!(
        envelope
            .events
            .iter()
            .any(|event| event.event_type == TraceContributionEventType::ToolCall)
    );
    assert!(
        envelope
            .events
            .iter()
            .any(|event| event.event_type == TraceContributionEventType::ToolResult)
    );
}

#[tokio::test]
async fn a_declared_risk_gives_a_metadata_only_envelope() {
    for (risk, expected) in [
        ("medium", ResidualPiiRisk::Medium),
        ("high", ResidualPiiRisk::High),
    ] {
        let fixture = fixture_from(fixture_line("risky", risk, prose_steps("secret words")));
        let envelope = compare_envelope(&fixture).await;
        assert_eq!(envelope.privacy.residual_pii_risk, expected);
        assert!(!envelope.consent.message_text_included);
        assert!(
            envelope
                .events
                .iter()
                .all(|event| event.redacted_content.is_none())
        );
    }
}

#[test]
fn the_baseline_configuration_holds_mains_gate() {
    let gate = compare_main_gate(DerivedFloors {
        perplexity_floor_micros: 20,
        tail_fraction_floor_micros: 5,
        novelty_floor_micros: 8,
    });
    let config = baseline_orchestrator_config(&gate);
    assert_eq!(config.perplexity_floor_micros, 20);
    assert_eq!(config.tail_fraction_floor_micros, 5);
    assert_eq!(config.novelty_floor_micros, 8);
    assert_eq!(config.top_k, 8);
    assert_eq!(config.chunk_target_tokens, 2048);
    assert_eq!(config.chunk_max_tokens, 3072);
    assert_eq!(config.chunk_cap, 16);
    assert_eq!(config.chunk_min_tokens, 64);
    assert_eq!(config.embed_insert_novelty_micros, 50_000);
    // PC-D7: `main` defaults this to the perplexity floor.
    assert_eq!(config.qualifying_chunk_floor_micros, 20);
    let bundle = CompatibilityBundleConfig::production_compatible(
        "reference_perplexity.v1".into(),
        MINIMAL_PROJECTION_ID.into(),
        MINIMAL_INDEX_ID.into(),
        &gate,
    )
    .expect("the bundle accepts main's gate");
    assert!(bundle.matches_main_gate(&gate));
    assert_eq!(gate.novelty_utility_microcredits, 2_500_000);
}

#[tokio::test]
async fn calibration_is_deterministic_and_not_zero() {
    let lines: Vec<String> = ["alpha", "bravo", "charlie", "delta"]
        .iter()
        .map(|word| fixture_line(&format!("cal_{word}"), "low", prose_steps(&long_text(word))))
        .collect();
    let (_dir, path) = write_corpus(&lines);
    let first = calibrate_floors(&path).await.unwrap();
    let second = calibrate_floors(&path).await.unwrap();
    assert_eq!(first, second);
    assert!(first.perplexity_floor_micros > 0);

    let (_dir, empty) = write_corpus(&[]);
    assert_eq!(
        calibrate_floors(&empty).await.err(),
        Some("comparison_calibration_empty")
    );
}

#[test]
fn the_two_indexes_give_the_same_similarity() {
    let embedder = ReferenceEmbedder::new();
    let embed = |text: &str| embedder.embed(text.as_bytes()).unwrap();
    let stored: Vec<Vec<f32>> = [
        "the quick brown fox jumps over the lazy dog",
        "pack my box with five dozen liquor jugs",
        "how vexingly quick daft zebras jump",
        "sphinx of black quartz judge my vow",
        "the five boxing wizards jump quickly",
    ]
    .iter()
    .map(|text| embed(text))
    .collect();
    let queries: Vec<Vec<f32>> = [
        "a quick brown dog jumps over a fox",
        "five dozen jugs of liquor in a box",
        "something that shares nothing at all",
    ]
    .iter()
    .map(|text| embed(text))
    .collect();

    let tenant = TenantStorageRef::new("tenant_sha256:80a707af7dc77ee1228f9127180f3964").unwrap();
    let mock = MockVectorIndex::new();
    let isolated = IsolatedPipelineIndex::new();
    for (position, embedding) in stored.iter().enumerate() {
        mock.insert(
            Uuid::from_u128(position as u128),
            tenant.as_str(),
            embedding,
        )
        .unwrap();
        let key = IndexEntryKey {
            tenant_storage_ref: tenant.clone(),
            index_id: MINIMAL_INDEX_ID.to_string(),
            revision_id: Uuid::from_u128(position as u128),
            projection_id: MINIMAL_PROJECTION_ID.to_string(),
            model_id: "reference-embedder-v1".to_string(),
            chunk: 0,
        };
        isolated
            .upsert(&key, embedding, &format!("sha256:{position:064x}"))
            .unwrap();
    }
    for query in &queries {
        let from_mock = mock.nearest(tenant.as_str(), query, 8).unwrap();
        let from_isolated = VectorIndexReader::nearest(
            isolated.as_ref(),
            &tenant,
            MINIMAL_INDEX_ID,
            query,
            8,
            None,
        )
        .unwrap();
        let bits = |list: &[NearestNeighbor]| -> Vec<u32> {
            list.iter().map(|n| n.similarity.to_bits()).collect()
        };
        assert_eq!(from_mock.len(), 5);
        assert_eq!(bits(&from_mock)[0], bits(&from_isolated)[0]);
        assert_eq!(bits(&from_mock), bits(&from_isolated));
    }
}

fn prose_steps(text: &str) -> Vec<TraceStep> {
    vec![
        TraceStep {
            request_hint: None,
            response: TraceResponse::UserInput {
                content: text.to_string(),
            },
            expected_tool_results: vec![],
            timestamp: None,
        },
        TraceStep {
            request_hint: None,
            response: TraceResponse::Text {
                content: format!("Answer to {text}"),
                input_tokens: 1,
                output_tokens: 1,
            },
            expected_tool_results: vec![],
            timestamp: None,
        },
    ]
}

fn tool_steps() -> Vec<TraceStep> {
    let mut steps = prose_steps("look up the weather");
    steps.push(TraceStep {
        request_hint: None,
        response: TraceResponse::ToolCalls {
            tool_calls: vec![TraceToolCall {
                id: "call_1".to_string(),
                name: "weather".to_string(),
                arguments: serde_json::json!({"city": "Lisbon"}),
            }],
            input_tokens: 1,
            output_tokens: 1,
        },
        expected_tool_results: vec![],
        timestamp: None,
    });
    steps.push(TraceStep {
        request_hint: None,
        response: TraceResponse::Text {
            content: "It is sunny.".to_string(),
            input_tokens: 1,
            output_tokens: 1,
        },
        expected_tool_results: vec![ExpectedToolResult {
            tool_call_id: "call_1".to_string(),
            name: "weather".to_string(),
            content: "sunny".to_string(),
        }],
        timestamp: None,
    });
    steps
}

fn long_text(seed: &str) -> String {
    (0..220)
        .map(|index| format!("{seed}{}", (index * 7 + seed.len()) % 61))
        .collect::<Vec<_>>()
        .join(" ")
}

fn fixture_line(label: &str, risk: &str, steps: Vec<TraceStep>) -> String {
    let id = Uuid::new_v5(&Uuid::NAMESPACE_URL, label.as_bytes());
    serde_json::json!({
        "label": label,
        "trace_id": id,
        "submission_id": Uuid::new_v5(&Uuid::NAMESPACE_URL, format!("submission:{label}").as_bytes()),
        "created_at": "2026-01-01T00:00:00Z",
        "secret_probe": format!("compare_probe_{label}"),
        "privacy_risk": risk,
        "trace_file": {
            "model_name": "compare-test",
            "memory_snapshot": [],
            "http_exchanges": [],
            "steps": steps,
        },
    })
    .to_string()
}

fn fixture_from(line: String) -> CompareFixture {
    serde_json::from_str(&line).unwrap()
}

fn write_corpus(lines: &[String]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("compare.jsonl");
    let mut text = lines.join("\n");
    if !lines.is_empty() {
        text.push('\n');
    }
    std::fs::write(&path, text).unwrap();
    (dir, path)
}
