// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic Hugging Face JSONL to pipeline-corpus export.

use std::path::{Path, PathBuf};

use anyhow::Context;
use chrono::{Duration, TimeZone, Utc};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[path = "pilot_bootstrap/hf_dataset.rs"]
mod hf_dataset;
#[path = "pilot_bootstrap/translators.rs"]
mod translators;

use hf_dataset::{HfJsonlDataset, list_local_jsonl_sessions, read_session_bytes};
use translators::{passes_word_filter, translator_by_name};

#[derive(Debug, Parser)]
#[command(name = "trace-commons-pipeline-corpus-export")]
#[command(about = "Export a pinned JSONL dataset sample for pipeline qualification")]
struct Args {
    #[arg(long)]
    repository: String,
    #[arg(long)]
    revision: String,
    #[arg(long)]
    split: String,
    #[arg(long)]
    translator: String,
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    local_jsonl_dir: Option<PathBuf>,
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    #[arg(long)]
    bootstrap_count: usize,
    #[arg(long)]
    holdout_count: usize,
    #[arg(long, default_value_t = 200)]
    min_words: usize,
    #[arg(long, default_value_t = 2000)]
    max_words: usize,
    #[arg(long, default_value_t = 1)]
    expected_instrument_count: usize,
    #[arg(long)]
    expected_source_digest: Option<String>,
    #[arg(long)]
    expected_order_digest: Option<String>,
}

#[derive(Debug)]
struct SelectedSession {
    sibling_name: String,
    bytes: Vec<u8>,
    trace_body: String,
}

fn sha256(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn canonical(value: &Value) -> anyhow::Result<Vec<u8>> {
    trace_commons_protocol::canonical_json::to_canonical_vec(value).map_err(anyhow::Error::from)
}

fn deterministic_uuid(domain: &str, value: &[u8]) -> Uuid {
    let mut digest = Sha256::new();
    digest.update(domain.as_bytes());
    digest.update([0]);
    digest.update(value);
    let mut bytes: [u8; 16] = digest.finalize()[..16]
        .try_into()
        .expect("fixed digest slice");
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

fn fixture(
    session: &SelectedSession,
    partition: &str,
    index: usize,
    instrument_count: usize,
) -> Value {
    let identity =
        Sha256::digest([session.sibling_name.as_bytes(), session.bytes.as_slice()].concat());
    let created_at = Utc
        .with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .single()
        .expect("fixed timestamp")
        + Duration::seconds(i64::try_from(index).expect("bounded sample count"));
    json!({
        "label": format!("hf_{partition}_{index:04}"),
        "trace_id": deterministic_uuid("pipeline-hf-trace", &identity).to_string(),
        "submission_id": deterministic_uuid("pipeline-hf-submission", &identity).to_string(),
        "created_at": created_at.to_rfc3339(),
        "input": session.trace_body,
        "secret_probe": format!("qualification_probe_{partition}_{index:04}"),
        "privacy_risk": "low",
        "expected_admission_decision": "admit",
        "expected_outcome_count": 4,
        "expected_consent_state": "allowed",
        "expected_privacy_state": "low",
        "expected_scoring_state": "complete",
        "expected_settlement_state": "complete",
        "expected_instrument_count": instrument_count,
    })
}

fn atomic_json(path: &Path, value: &Value) -> anyhow::Result<()> {
    let parent = path.parent().context("output has no parent")?;
    std::fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".pipeline-corpus-{}.tmp", std::process::id()));
    let bytes = pretty_json(value)?;
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn pretty_json(value: &Value) -> anyhow::Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

async fn selected_sessions(args: &Args) -> anyhow::Result<Vec<SelectedSession>> {
    let translator = translator_by_name(&args.translator)?;
    let names_and_paths = if let Some(directory) = &args.local_jsonl_dir {
        Some(
            list_local_jsonl_sessions(directory)?
                .into_iter()
                .map(|session| (session.sibling_name, session.local_path))
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let dataset = if names_and_paths.is_none() {
        Some(HfJsonlDataset::open_at_revision(
            &args.repository,
            &args.revision,
            args.cache_dir.as_deref(),
        )?)
    } else {
        None
    };
    let names = if let Some(files) = &names_and_paths {
        files.iter().map(|(name, _)| name.clone()).collect()
    } else {
        dataset
            .as_ref()
            .expect("remote dataset exists")
            .list_session_names()
            .await?
    };
    let required = args
        .bootstrap_count
        .checked_add(args.holdout_count)
        .context("sample count overflow")?;
    let mut selected = Vec::with_capacity(required);
    for name in names {
        let bytes = if let Some(files) = &names_and_paths {
            let path = files
                .iter()
                .find_map(|(candidate, path)| (candidate == &name).then_some(path))
                .context("listed local session disappeared")?;
            read_session_bytes(path)?
        } else {
            let session = dataset
                .as_ref()
                .expect("remote dataset exists")
                .fetch_session(&name)
                .await?;
            read_session_bytes(&session.local_path)?
        };
        let draft = match translator.translate(&name, &bytes) {
            Ok(draft) => draft,
            Err(_) => continue,
        };
        if !passes_word_filter(&draft.trace_body, args.min_words, args.max_words) {
            continue;
        }
        selected.push(SelectedSession {
            sibling_name: name,
            bytes,
            trace_body: draft.trace_body,
        });
        if selected.len() == required {
            break;
        }
    }
    anyhow::ensure!(
        selected.len() == required,
        "dataset did not provide the complete pinned sample"
    );
    Ok(selected)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    anyhow::ensure!(
        args.bootstrap_count > 0 && args.holdout_count > 0,
        "bootstrap and holdout counts must be nonzero"
    );
    anyhow::ensure!(
        args.min_words <= args.max_words,
        "minimum words exceeds maximum words"
    );
    anyhow::ensure!(
        args.expected_instrument_count > 0,
        "expected instrument count must be nonzero"
    );
    let selected = selected_sessions(&args).await?;
    let configuration = json!({
        "repository": args.repository,
        "revision": args.revision,
        "split": args.split,
        "translator": args.translator,
        "bootstrap_count": args.bootstrap_count,
        "holdout_count": args.holdout_count,
        "min_words": args.min_words,
        "max_words": args.max_words,
        "expected_instrument_count": args.expected_instrument_count,
    });
    let configuration_digest = sha256(&canonical(&configuration)?);
    let order_digest = sha256(&canonical(&json!(
        selected
            .iter()
            .map(|session| sha256(session.sibling_name.as_bytes()))
            .collect::<Vec<_>>()
    ))?);
    let mut source = Sha256::new();
    for session in &selected {
        source.update(session.sibling_name.len().to_be_bytes());
        source.update(session.sibling_name.as_bytes());
        source.update(session.bytes.len().to_be_bytes());
        source.update(&session.bytes);
    }
    let source_digest = format!("sha256:{:x}", source.finalize());
    if let Some(expected) = &args.expected_source_digest {
        anyhow::ensure!(expected == &source_digest, "source digest changed");
    }
    if let Some(expected) = &args.expected_order_digest {
        anyhow::ensure!(expected == &order_digest, "sample order digest changed");
    }

    let (bootstrap, holdout) = selected.split_at(args.bootstrap_count);
    let bootstrap_corpus = json!({
        "schema": "trace_commons.pipeline_corpus.v1",
        "fixtures": bootstrap.iter().enumerate().map(|(index, session)| {
            fixture(session, "bootstrap", index, args.expected_instrument_count)
        }).collect::<Vec<_>>(),
    });
    let holdout_corpus = json!({
        "schema": "trace_commons.pipeline_corpus.v1",
        "fixtures": holdout.iter().enumerate().map(|(index, session)| {
            fixture(session, "holdout", index, args.expected_instrument_count)
        }).collect::<Vec<_>>(),
    });
    let bootstrap_bytes = pretty_json(&bootstrap_corpus)?;
    let holdout_bytes = pretty_json(&holdout_corpus)?;
    let manifest = json!({
        "schema": "trace_commons.pipeline_hf_corpus_manifest.v1",
        "source": configuration,
        "source_digest": source_digest,
        "configuration_digest": configuration_digest,
        "order_digest": order_digest,
        "bootstrap_corpus_digest": sha256(&bootstrap_bytes),
        "holdout_corpus_digest": sha256(&holdout_bytes),
        "sample_count": selected.len(),
        "bootstrap_count": bootstrap.len(),
        "holdout_count": holdout.len(),
        "contains_raw_trace_text": false,
        "contains_contributor_identity": false,
    });
    atomic_json(
        &args.output_dir.join("bootstrap-corpus.json"),
        &bootstrap_corpus,
    )?;
    atomic_json(
        &args.output_dir.join("holdout-corpus.json"),
        &holdout_corpus,
    )?;
    atomic_json(&args.output_dir.join("source-manifest.json"), &manifest)?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_uuid_is_stable_and_version_five() {
        let first = deterministic_uuid("pipeline-hf-trace", b"same-input");
        let second = deterministic_uuid("pipeline-hf-trace", b"same-input");
        assert_eq!(first, second, "same domain and value must yield same uuid");

        let bytes = first.as_bytes();
        assert_eq!(bytes[6] & 0xf0, 0x50, "version nibble must be 5");
        assert_eq!(bytes[8] & 0xc0, 0x80, "variant bits must be 10");
    }

    #[test]
    fn fixture_never_carries_the_source_name() {
        let session = SelectedSession {
            sibling_name: "secret-session.jsonl".to_string(),
            bytes: b"{\"type\":\"message\"}\n".to_vec(),
            trace_body: "a completely unrelated translated trace body".to_string(),
        };
        let value = fixture(&session, "bootstrap", 0, 1);
        let serialized = serde_json::to_string(&value).expect("fixture serializes");
        assert!(
            !serialized.contains("secret-session"),
            "fixture JSON must never carry the source sibling name: {serialized}"
        );
    }

    #[test]
    fn configuration_digest_is_key_order_independent() {
        let configuration = json!({
            "repository": "jedisct1/security-audits",
            "revision": "6d527ff0081eec6704c2a4f00e1ef8d308ae7366",
            "split": "train",
            "translator": "swival",
            "bootstrap_count": 1,
            "holdout_count": 1,
            "min_words": 200,
            "max_words": 2000,
            "expected_instrument_count": 1,
        });
        let digest = sha256(&canonical(&configuration).expect("canonical json"));
        assert_eq!(
            digest,
            "sha256:06d536617c70a36caba72ceab8ac637c75d428727dc27f92e931d07b5510d050"
        );
    }
}
