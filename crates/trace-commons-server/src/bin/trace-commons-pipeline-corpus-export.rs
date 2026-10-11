// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Deterministic Hugging Face JSONL to pipeline-corpus export.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;
use chrono::{Duration, TimeZone, Utc};
use clap::Parser;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::task::JoinSet;
use uuid::Uuid;

#[path = "pilot_bootstrap/hf_dataset.rs"]
mod hf_dataset;
#[path = "pilot_bootstrap/translators.rs"]
mod translators;

use hf_dataset::{HfJsonlDataset, list_local_jsonl_sessions, read_session_bytes};
use translators::{
    SubmissionDraft, Translator, passes_word_filter, trace_file_for, translator_by_name,
};

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
    /// Write each session with its events as one JSON line.
    #[arg(long)]
    with_events: bool,
    /// Only these files of the local directory are candidates (repeatable).
    #[arg(long = "session-name")]
    session_name: Vec<String>,
    /// `NAME=RISK`, where RISK is `medium` or `high` (repeatable).
    #[arg(long = "declared-privacy-risk")]
    declared_privacy_risk: Vec<String>,
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

fn session_identity(session: &SelectedSession) -> impl AsRef<[u8]> {
    Sha256::digest([session.sibling_name.as_bytes(), session.bytes.as_slice()].concat())
}

fn fixture_time(index: usize) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
        .single()
        .expect("fixed timestamp")
        + Duration::seconds(i64::try_from(index).expect("bounded sample count"))
}

fn fixture(
    session: &SelectedSession,
    partition: &str,
    index: usize,
    instrument_count: usize,
) -> Value {
    let identity = session_identity(session);
    let created_at = fixture_time(index);
    json!({
        "label": format!("hf_{partition}_{index:04}"),
        "trace_id": deterministic_uuid("pipeline-hf-trace", identity.as_ref()).to_string(),
        "submission_id": deterministic_uuid("pipeline-hf-submission", identity.as_ref()).to_string(),
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

/// The sessions that are downloaded at the same time in remote mode. The
/// window is small while Hugging Face is not told to wait after an HTTP 429
/// (issue #1308).
const DOWNLOAD_WINDOW: usize = 4;

struct SelectionFilter {
    min_words: usize,
    max_words: usize,
    required: usize,
}

/// Judges one session at a time and hands each accepted session to `handle`
/// before the next one is read, so a caller never has to hold more than one.
struct Selector<'a, H> {
    translator: &'a dyn Translator,
    filter: &'a SelectionFilter,
    taken: usize,
    handle: H,
}

impl<H: FnMut(SelectedSession, SubmissionDraft) -> anyhow::Result<()>> Selector<'_, H> {
    /// Returns true when the required count is reached.
    fn offer(&mut self, name: String, bytes: Vec<u8>) -> anyhow::Result<bool> {
        let mut draft = match self.translator.translate(&name, &bytes) {
            Ok(draft) => draft,
            Err(_) => return Ok(false),
        };
        if !passes_word_filter(
            &draft.trace_body,
            self.filter.min_words,
            self.filter.max_words,
        ) {
            return Ok(false);
        }
        // The session keeps the body; the draft keeps the events.
        let trace_body = std::mem::take(&mut draft.trace_body);
        let session = SelectedSession {
            sibling_name: name,
            bytes,
            trace_body,
        };
        (self.handle)(session, draft)?;
        self.taken += 1;
        Ok(self.taken == self.filter.required)
    }

    fn complete(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.taken == self.filter.required,
            "dataset did not provide the complete pinned sample"
        );
        Ok(())
    }
}

fn select_local(
    names: &[String],
    mut read: impl FnMut(&str) -> anyhow::Result<Vec<u8>>,
    translator: &dyn Translator,
    filter: &SelectionFilter,
    handle: impl FnMut(SelectedSession, SubmissionDraft) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut selector = Selector {
        translator,
        filter,
        taken: 0,
        handle,
    };
    for name in names {
        let bytes = read(name)?;
        if selector.offer(name.clone(), bytes)? {
            break;
        }
    }
    selector.complete()
}

async fn download_bytes(dataset: &HfJsonlDataset, name: &str) -> anyhow::Result<Vec<u8>> {
    let session = dataset.fetch_session(name).await?;
    read_session_bytes(&session.local_path)
}

/// Downloads a window of names at the same time, then handles the window in
/// name order. A failed download gets one more attempt after its window.
async fn select_remote(
    dataset: Arc<HfJsonlDataset>,
    names: Vec<String>,
    translator: &dyn Translator,
    filter: &SelectionFilter,
    handle: impl FnMut(SelectedSession, SubmissionDraft) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut selector = Selector {
        translator,
        filter,
        taken: 0,
        handle,
    };
    'windows: for window in names.chunks(DOWNLOAD_WINDOW) {
        let mut downloads = JoinSet::new();
        for (position, name) in window.iter().enumerate() {
            let dataset = Arc::clone(&dataset);
            let name = name.clone();
            downloads.spawn(async move { (position, download_bytes(&dataset, &name).await) });
        }
        let mut results: Vec<Option<anyhow::Result<Vec<u8>>>> =
            window.iter().map(|_| None).collect();
        while let Some(joined) = downloads.join_next().await {
            let (position, result) = joined.context("download task failed")?;
            results[position] = Some(result);
        }
        for (name, result) in window.iter().zip(results) {
            let bytes = match result.expect("every download in the window finished") {
                Ok(bytes) => bytes,
                Err(_) => download_bytes(&dataset, name).await?,
            };
            if selector.offer(name.clone(), bytes)? {
                break 'windows;
            }
        }
    }
    selector.complete()
}

async fn select_sessions(
    args: &Args,
    session_names: &BTreeSet<String>,
    handle: impl FnMut(SelectedSession, SubmissionDraft) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let translator = translator_by_name(&args.translator)?;
    let filter = SelectionFilter {
        min_words: args.min_words,
        max_words: args.max_words,
        required: args
            .bootstrap_count
            .checked_add(args.holdout_count)
            .context("sample count overflow")?,
    };
    if let Some(directory) = &args.local_jsonl_dir {
        let mut files: BTreeMap<String, PathBuf> = list_local_jsonl_sessions(directory)?
            .into_iter()
            .map(|session| (session.sibling_name, session.local_path))
            .collect();
        if !session_names.is_empty() {
            for name in session_names {
                anyhow::ensure!(
                    files.contains_key(name),
                    "session name is not in the directory"
                );
            }
            files.retain(|name, _| session_names.contains(name));
        }
        let names: Vec<String> = files.keys().cloned().collect();
        select_local(
            &names,
            |name| read_session_bytes(&files[name]),
            translator.as_ref(),
            &filter,
            handle,
        )
    } else {
        let dataset = Arc::new(HfJsonlDataset::open_at_revision(
            &args.repository,
            &args.revision,
            args.cache_dir.as_deref(),
        )?);
        let names = dataset.list_session_names().await?;
        select_remote(dataset, names, translator.as_ref(), &filter, handle).await
    }
}

fn parse_declared_risks(values: &[String]) -> anyhow::Result<BTreeMap<String, String>> {
    let mut declared = BTreeMap::new();
    for value in values {
        let (name, risk) = value
            .split_once('=')
            .context("declared privacy risk must be NAME=RISK")?;
        anyhow::ensure!(
            matches!(risk, "medium" | "high"),
            "declared privacy risk must be medium or high"
        );
        anyhow::ensure!(
            declared
                .insert(name.to_string(), risk.to_string())
                .is_none(),
            "declared privacy risk names a session twice"
        );
    }
    Ok(declared)
}

/// A JSONL file that is written through a temporary name and renamed on
/// commit. The digest covers the bytes of the file. Each writer has its own
/// temporary name, because two are open at the same time.
struct LineFile {
    path: PathBuf,
    temporary: PathBuf,
    writer: BufWriter<File>,
    digest: Sha256,
    committed: bool,
}

impl LineFile {
    fn create(path: PathBuf, label: &str) -> anyhow::Result<Self> {
        let parent = path.parent().context("output has no parent")?;
        std::fs::create_dir_all(parent)?;
        let temporary = parent.join(format!(
            ".pipeline-corpus-{}-{label}.tmp",
            std::process::id()
        ));
        let writer = BufWriter::new(File::create(&temporary)?);
        Ok(Self {
            path,
            temporary,
            writer,
            digest: Sha256::new(),
            committed: false,
        })
    }

    fn append(&mut self, line: &[u8]) -> anyhow::Result<()> {
        self.writer.write_all(line)?;
        self.writer.write_all(b"\n")?;
        self.digest.update(line);
        self.digest.update(b"\n");
        Ok(())
    }

    fn commit(&mut self) -> anyhow::Result<String> {
        self.writer.flush()?;
        std::fs::rename(&self.temporary, &self.path)?;
        self.committed = true;
        Ok(format!("sha256:{:x}", self.digest.clone().finalize()))
    }
}

impl Drop for LineFile {
    fn drop(&mut self) {
        if !self.committed {
            let _ = std::fs::remove_file(&self.temporary);
        }
    }
}

/// One compare line: the identities of `fixture`, the declared privacy risk,
/// and the recorded trace of the session events. No `input` text.
fn compare_fixture(
    session: &SelectedSession,
    partition: &str,
    index: usize,
    privacy_risk: &str,
    draft: &SubmissionDraft,
) -> anyhow::Result<Value> {
    let identity = session_identity(session);
    Ok(json!({
        "label": format!("hf_{partition}_{index:04}"),
        "trace_id": deterministic_uuid("pipeline-hf-trace", identity.as_ref()).to_string(),
        "submission_id": deterministic_uuid("pipeline-hf-submission", identity.as_ref()).to_string(),
        "created_at": fixture_time(index).to_rfc3339(),
        "secret_probe": format!("qualification_probe_{partition}_{index:04}"),
        "privacy_risk": privacy_risk,
        "trace_file": serde_json::to_value(trace_file_for(draft))?,
    }))
}

/// The `--with-events` output. It takes one session at a time: each session is
/// written before the next is read, and the source and order digests grow as
/// the sessions pass.
struct EventSink<'a> {
    bootstrap: LineFile,
    holdout: LineFile,
    bootstrap_count: usize,
    taken: usize,
    source: Sha256,
    order: Vec<String>,
    declared: &'a BTreeMap<String, String>,
    matched: BTreeSet<String>,
}

impl<'a> EventSink<'a> {
    fn open(
        output_dir: &Path,
        bootstrap_count: usize,
        declared: &'a BTreeMap<String, String>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            bootstrap: LineFile::create(output_dir.join("bootstrap-compare.jsonl"), "bootstrap")?,
            holdout: LineFile::create(output_dir.join("holdout-compare.jsonl"), "holdout")?,
            bootstrap_count,
            taken: 0,
            source: Sha256::new(),
            order: Vec::new(),
            declared,
            matched: BTreeSet::new(),
        })
    }

    fn add(&mut self, session: &SelectedSession, draft: &SubmissionDraft) -> anyhow::Result<()> {
        self.source.update(session.sibling_name.len().to_be_bytes());
        self.source.update(session.sibling_name.as_bytes());
        self.source.update(session.bytes.len().to_be_bytes());
        self.source.update(&session.bytes);
        self.order.push(sha256(session.sibling_name.as_bytes()));
        let privacy_risk = match self.declared.get(&session.sibling_name) {
            Some(risk) => {
                self.matched.insert(session.sibling_name.clone());
                risk.as_str()
            }
            None => "low",
        };
        let (partition, index, file) = if self.taken < self.bootstrap_count {
            ("bootstrap", self.taken, &mut self.bootstrap)
        } else {
            (
                "holdout",
                self.taken - self.bootstrap_count,
                &mut self.holdout,
            )
        };
        let line = canonical(&compare_fixture(
            session,
            partition,
            index,
            privacy_risk,
            draft,
        )?)?;
        file.append(&line)?;
        self.taken += 1;
        Ok(())
    }

    fn finish(
        mut self,
        args: &Args,
        configuration: Value,
        configuration_digest: String,
    ) -> anyhow::Result<Value> {
        for name in self.declared.keys() {
            anyhow::ensure!(
                self.matched.contains(name),
                "declared privacy risk names no selected session"
            );
        }
        let order_digest = sha256(&canonical(&json!(self.order))?);
        let source_digest = format!("sha256:{:x}", self.source.clone().finalize());
        if let Some(expected) = &args.expected_source_digest {
            anyhow::ensure!(expected == &source_digest, "source digest changed");
        }
        if let Some(expected) = &args.expected_order_digest {
            anyhow::ensure!(expected == &order_digest, "sample order digest changed");
        }
        let bootstrap_corpus_digest = self.bootstrap.commit()?;
        let holdout_corpus_digest = self.holdout.commit()?;
        let manifest = json!({
            "schema": "trace_commons.pipeline_hf_corpus_manifest.v1",
            "source": configuration,
            "source_digest": source_digest,
            "configuration_digest": configuration_digest,
            "order_digest": order_digest,
            "bootstrap_corpus_digest": bootstrap_corpus_digest,
            "holdout_corpus_digest": holdout_corpus_digest,
            "sample_count": self.taken,
            "bootstrap_count": args.bootstrap_count,
            "holdout_count": args.holdout_count,
            "contains_raw_trace_text": false,
            "contains_contributor_identity": false,
        });
        atomic_json(&args.output_dir.join("source-manifest.json"), &manifest)?;
        Ok(manifest)
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let manifest = run(Args::parse()).await?;
    println!("{}", serde_json::to_string(&manifest)?);
    Ok(())
}

async fn run(args: Args) -> anyhow::Result<Value> {
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
    anyhow::ensure!(
        args.local_jsonl_dir.is_some() || args.session_name.is_empty(),
        "session names need a local directory"
    );
    let declared = parse_declared_risks(&args.declared_privacy_risk)?;
    anyhow::ensure!(
        args.local_jsonl_dir.is_some() || declared.is_empty(),
        "declared privacy risk needs a local directory"
    );
    anyhow::ensure!(
        args.with_events || declared.is_empty(),
        "declared privacy risk needs events"
    );
    let session_names: BTreeSet<String> = args.session_name.iter().cloned().collect();
    let mut configuration = json!({
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
    if args.with_events {
        configuration["with_events"] = json!(true);
    }
    let configuration_digest = sha256(&canonical(&configuration)?);
    if args.with_events {
        let mut sink = EventSink::open(&args.output_dir, args.bootstrap_count, &declared)?;
        select_sessions(&args, &session_names, |session, draft| {
            sink.add(&session, &draft)
        })
        .await?;
        return sink.finish(&args, configuration, configuration_digest);
    }
    let required = args
        .bootstrap_count
        .checked_add(args.holdout_count)
        .context("sample count overflow")?;
    let mut selected = Vec::with_capacity(required);
    select_sessions(&args, &session_names, |session, _draft| {
        selected.push(session);
        Ok(())
    })
    .await?;
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
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_protocol::llm::recording::TraceFile;

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

    const TEN_SESSIONS: [&str; 10] = [
        "s01.jsonl",
        "s02.jsonl",
        "s03.jsonl",
        "s04.jsonl",
        "s05.jsonl",
        "s06.jsonl",
        "s07.jsonl",
        "s08.jsonl",
        "s09.jsonl",
        "s10.jsonl",
    ];

    fn fixture_dir(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    const ALL_TWELVE: [&str; 12] = [
        "s01.jsonl",
        "s02.jsonl",
        "s03.jsonl",
        "s04.jsonl",
        "s04h.jsonl",
        "s04m.jsonl",
        "s05.jsonl",
        "s06.jsonl",
        "s07.jsonl",
        "s08.jsonl",
        "s09.jsonl",
        "s10.jsonl",
    ];

    /// Arguments for the local compare fixtures: bootstrap 4, holdout 6, the
    /// ten plain sessions. `extra` adds flags after them.
    fn compare_args(output: &Path, with_events: bool, extra: &[&str]) -> Args {
        compare_args_for(output, with_events, 6, &TEN_SESSIONS, extra)
    }

    fn compare_args_for(
        output: &Path,
        with_events: bool,
        holdout: usize,
        names: &[&str],
        extra: &[&str],
    ) -> Args {
        let directory = fixture_dir("pipeline-compare-jsonl");
        let mut argv: Vec<String> = [
            "export",
            "--repository",
            "jedisct1/security-audits",
            "--revision",
            "6d527ff0081eec6704c2a4f00e1ef8d308ae7366",
            "--split",
            "train",
            "--translator",
            "swival",
            "--bootstrap-count",
            "4",
            "--holdout-count",
        ]
        .iter()
        .map(|part| part.to_string())
        .collect();
        argv.push(holdout.to_string());
        argv.extend(
            ["--min-words", "1", "--max-words", "20000", "--output-dir"]
                .iter()
                .map(|part| part.to_string()),
        );
        argv.push(output.display().to_string());
        argv.push("--local-jsonl-dir".into());
        argv.push(directory.display().to_string());
        if with_events {
            argv.push("--with-events".into());
        }
        for name in names {
            argv.push("--session-name".into());
            argv.push((*name).into());
        }
        argv.extend(extra.iter().map(|part| part.to_string()));
        Args::parse_from(argv)
    }

    fn read_lines(path: &Path) -> Vec<Value> {
        std::fs::read_to_string(path)
            .expect("jsonl file exists")
            .lines()
            .map(|line| serde_json::from_str(line).expect("each line is json"))
            .collect()
    }

    #[tokio::test]
    async fn v1_output_is_unchanged() {
        let directory = fixture_dir("pipeline-hf-jsonl");
        let pin: Value = serde_json::from_slice(
            &std::fs::read(directory.join("pin-local.json")).expect("pin exists"),
        )
        .expect("pin parses");
        let output = tempfile::tempdir().expect("temporary directory");
        let args = Args::parse_from([
            "export".to_string(),
            "--repository".into(),
            pin["repository"].as_str().expect("repository").into(),
            "--revision".into(),
            pin["revision"].as_str().expect("revision").into(),
            "--split".into(),
            pin["split"].as_str().expect("split").into(),
            "--translator".into(),
            pin["translator"].as_str().expect("translator").into(),
            "--bootstrap-count".into(),
            pin["bootstrap_count"].to_string(),
            "--holdout-count".into(),
            pin["holdout_count"].to_string(),
            "--min-words".into(),
            pin["min_words"].to_string(),
            "--max-words".into(),
            pin["max_words"].to_string(),
            "--output-dir".into(),
            output.path().display().to_string(),
            "--local-jsonl-dir".into(),
            directory.display().to_string(),
        ]);
        let manifest = run(args).await.expect("v1 export runs");
        for key in [
            "source_digest",
            "configuration_digest",
            "order_digest",
            "bootstrap_corpus_digest",
            "holdout_corpus_digest",
        ] {
            assert_eq!(manifest[key], pin[key], "{key} changed");
        }
        assert!(manifest["source"].get("with_events").is_none());
    }

    #[tokio::test]
    async fn with_events_writes_one_fixture_on_each_line() {
        let output = tempfile::tempdir().expect("temporary directory");
        let manifest = run(compare_args(output.path(), true, &[]))
            .await
            .expect("export runs");
        assert_eq!(manifest["bootstrap_count"], 4);
        assert_eq!(manifest["holdout_count"], 6);
        let bootstrap = read_lines(&output.path().join("bootstrap-compare.jsonl"));
        let holdout = read_lines(&output.path().join("holdout-compare.jsonl"));
        assert_eq!(bootstrap.len(), 4);
        assert_eq!(holdout.len(), 6);
        let expected_keys: std::collections::BTreeSet<&str> = [
            "label",
            "trace_id",
            "submission_id",
            "created_at",
            "secret_probe",
            "privacy_risk",
            "trace_file",
        ]
        .into_iter()
        .collect();
        let has_tool_calls = |line: &Value| {
            line["trace_file"]["steps"]
                .as_array()
                .expect("steps")
                .iter()
                .any(|step| step["response"]["type"] == "tool_calls")
        };
        for line in bootstrap.iter().chain(holdout.iter()) {
            let keys: std::collections::BTreeSet<&str> = line
                .as_object()
                .expect("object")
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(keys, expected_keys);
            assert!(line.get("input").is_none());
            serde_json::from_value::<TraceFile>(line["trace_file"].clone())
                .expect("trace_file is a TraceFile");
        }
        // s09 is the fifth holdout session (s05 to s10 in name order).
        assert!(has_tool_calls(&holdout[4]));
        assert_eq!(
            bootstrap
                .iter()
                .chain(holdout.iter())
                .filter(|line| has_tool_calls(line))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn with_events_identities_equal_the_v1_identities() {
        let v1_output = tempfile::tempdir().expect("temporary directory");
        run(compare_args(v1_output.path(), false, &[]))
            .await
            .expect("v1 export runs");
        let events_output = tempfile::tempdir().expect("temporary directory");
        run(compare_args(events_output.path(), true, &[]))
            .await
            .expect("events export runs");
        for (v1_file, events_file) in [
            ("bootstrap-corpus.json", "bootstrap-compare.jsonl"),
            ("holdout-corpus.json", "holdout-compare.jsonl"),
        ] {
            let v1: Value = serde_json::from_slice(
                &std::fs::read(v1_output.path().join(v1_file)).expect("v1 file"),
            )
            .expect("v1 parses");
            let v1_fixtures = v1["fixtures"].as_array().expect("fixtures");
            let lines = read_lines(&events_output.path().join(events_file));
            assert_eq!(v1_fixtures.len(), lines.len());
            for (old, new) in v1_fixtures.iter().zip(lines.iter()) {
                for key in ["label", "trace_id", "submission_id", "created_at"] {
                    assert_eq!(old[key], new[key], "{key} differs");
                }
            }
        }
    }

    #[tokio::test]
    async fn with_events_changes_the_configuration_digest_only_by_its_flag() {
        let v1_output = tempfile::tempdir().expect("temporary directory");
        let v1 = run(compare_args(v1_output.path(), false, &[]))
            .await
            .expect("v1 export runs");
        let events_output = tempfile::tempdir().expect("temporary directory");
        let events = run(compare_args(events_output.path(), true, &[]))
            .await
            .expect("events export runs");
        assert_eq!(events["source"]["with_events"], true);
        assert_eq!(events["source_digest"], v1["source_digest"]);
        assert_eq!(events["order_digest"], v1["order_digest"]);
        let mut without_flag = events["source"].clone();
        without_flag
            .as_object_mut()
            .expect("source object")
            .remove("with_events");
        assert_eq!(without_flag, v1["source"]);
        assert_ne!(events["configuration_digest"], v1["configuration_digest"]);
        assert_eq!(
            events["configuration_digest"],
            sha256(&canonical(&events["source"]).expect("canonical json"))
        );
    }

    #[tokio::test]
    async fn a_declared_risk_reaches_the_fixture() {
        let output = tempfile::tempdir().expect("temporary directory");
        run(compare_args_for(
            output.path(),
            true,
            8,
            &ALL_TWELVE,
            &[
                "--declared-privacy-risk",
                "s04m.jsonl=medium",
                "--declared-privacy-risk",
                "s04h.jsonl=high",
            ],
        ))
        .await
        .expect("export runs");
        let lines: Vec<Value> = ["bootstrap-compare.jsonl", "holdout-compare.jsonl"]
            .iter()
            .flat_map(|file| read_lines(&output.path().join(file)))
            .collect();
        let risks: Vec<&str> = lines
            .iter()
            .map(|line| line["privacy_risk"].as_str().expect("risk"))
            .collect();
        // Name order: s01 to s04, s04h, s04m, then s05 to s10.
        assert_eq!(risks.len(), 12);
        assert_eq!(risks[4], "high");
        assert_eq!(risks[5], "medium");
        assert_eq!(risks.iter().filter(|risk| **risk == "low").count(), 10);

        let output = tempfile::tempdir().expect("temporary directory");
        let unmatched = run(compare_args(
            output.path(),
            true,
            &["--declared-privacy-risk", "s99.jsonl=high"],
        ))
        .await
        .expect_err("a name with no selected session is refused");
        assert!(
            unmatched
                .to_string()
                .contains("declared privacy risk names no selected session")
        );

        let output = tempfile::tempdir().expect("temporary directory");
        let low = run(compare_args(
            output.path(),
            true,
            &["--declared-privacy-risk", "s01.jsonl=low"],
        ))
        .await
        .expect_err("a risk other than medium or high is refused");
        assert!(low.to_string().contains("medium or high"));

        let output = tempfile::tempdir().expect("temporary directory");
        let without_events = run(compare_args(
            output.path(),
            false,
            &["--declared-privacy-risk", "s01.jsonl=high"],
        ))
        .await
        .expect_err("a declared risk without events is refused");
        assert!(
            without_events
                .to_string()
                .contains("declared privacy risk needs events")
        );

        let output = tempfile::tempdir().expect("temporary directory");
        let remote = Args::parse_from([
            "export",
            "--repository",
            "jedisct1/security-audits",
            "--revision",
            "6d527ff0081eec6704c2a4f00e1ef8d308ae7366",
            "--split",
            "train",
            "--translator",
            "swival",
            "--bootstrap-count",
            "1",
            "--holdout-count",
            "1",
            "--output-dir",
            &output.path().display().to_string(),
            "--with-events",
            "--declared-privacy-risk",
            "s01.jsonl=high",
        ]);
        let refused = run(remote).await.expect_err("remote mode is refused");
        assert!(
            refused
                .to_string()
                .contains("declared privacy risk needs a local directory")
        );
    }

    #[test]
    fn each_session_is_written_before_the_next_is_read() {
        use std::cell::RefCell;
        let events: RefCell<Vec<String>> = RefCell::new(Vec::new());
        let names: Vec<String> = (1..=5).map(|n| format!("t{n}.jsonl")).collect();
        let translator = translator_by_name("swival").expect("swival translator");
        let filter = SelectionFilter {
            min_words: 1,
            max_words: 100,
            required: 5,
        };
        select_local(
            &names,
            |name| {
                events.borrow_mut().push(format!("read:{name}"));
                Ok(format!(
                    "{{\"type\":\"message\",\"message\":{{\"role\":\"user\",\"content\":\"hello from {name}\"}}}}\n"
                )
                .into_bytes())
            },
            translator.as_ref(),
            &filter,
            |session, _draft| {
                events
                    .borrow_mut()
                    .push(format!("write:{}", session.sibling_name));
                Ok(())
            },
        )
        .expect("selection runs");
        let expected: Vec<String> = names
            .iter()
            .flat_map(|name| [format!("read:{name}"), format!("write:{name}")])
            .collect();
        assert_eq!(*events.borrow(), expected);
    }

    #[tokio::test]
    async fn no_fixture_line_carries_the_source_name() {
        let output = tempfile::tempdir().expect("temporary directory");
        run(compare_args_for(output.path(), true, 8, &ALL_TWELVE, &[]))
            .await
            .expect("export runs");
        let names: Vec<String> = std::fs::read_dir(fixture_dir("pipeline-compare-jsonl"))
            .expect("fixture directory")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .filter(|name| name.ends_with(".jsonl"))
            .collect();
        assert_eq!(names.len(), 12);
        for file in ["bootstrap-compare.jsonl", "holdout-compare.jsonl"] {
            let text = std::fs::read_to_string(output.path().join(file)).expect("file");
            for name in &names {
                assert!(!text.contains(name.as_str()), "{file} carries {name}");
            }
        }
    }

    #[tokio::test]
    async fn a_session_name_list_limits_the_candidates() {
        let directory = fixture_dir("pipeline-compare-jsonl");
        let output = tempfile::tempdir().expect("temporary directory");
        let argv = |output: &Path, names: &[&str], local: bool| {
            let mut argv: Vec<String> = [
                "export",
                "--repository",
                "jedisct1/security-audits",
                "--revision",
                "6d527ff0081eec6704c2a4f00e1ef8d308ae7366",
                "--split",
                "train",
                "--translator",
                "swival",
                "--bootstrap-count",
                "1",
                "--holdout-count",
                "1",
                "--min-words",
                "1",
                "--max-words",
                "20000",
                "--with-events",
            ]
            .iter()
            .map(|part| part.to_string())
            .collect();
            argv.push("--output-dir".into());
            argv.push(output.display().to_string());
            if local {
                argv.push("--local-jsonl-dir".into());
                argv.push(directory.display().to_string());
            }
            for name in names {
                argv.push("--session-name".into());
                argv.push((*name).into());
            }
            Args::parse_from(argv)
        };
        let manifest = run(argv(output.path(), &["s02.jsonl", "s01.jsonl"], true))
            .await
            .expect("export runs");
        assert_eq!(manifest["sample_count"], 2);
        let expected_order = sha256(
            &canonical(&json!([sha256(b"s01.jsonl"), sha256(b"s02.jsonl")]))
                .expect("canonical json"),
        );
        assert_eq!(manifest["order_digest"], expected_order);

        let missing = run(argv(output.path(), &["s01.jsonl", "s99.jsonl"], true))
            .await
            .expect_err("a name outside the directory is refused");
        assert!(
            missing
                .to_string()
                .contains("session name is not in the directory")
        );

        let remote = run(argv(output.path(), &["s01.jsonl"], false))
            .await
            .expect_err("session names need a local directory");
        assert!(
            remote
                .to_string()
                .contains("session names need a local directory")
        );
    }
}
