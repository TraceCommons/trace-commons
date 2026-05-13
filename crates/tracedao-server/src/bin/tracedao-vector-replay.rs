//! `tracedao-vector-replay` — operator CLI that rebuilds a tenant's local
//! vector index from PostgreSQL audit data + the encrypted artifact store.
//!
//! Reads the same env-var surface as the gate service. The original audit /
//! credit history is preserved as-is — this binary does NOT write gate-
//! decision rows, audit events, or credit events of its own. It only
//! rebuilds the per-tenant usearch file by replaying past
//! `trace_gate_decisions` rows that have `vector_entry_id IS NOT NULL`.
//!
//! See `docs/operator/vector-replay.md` for the operator-facing runbook.

#![cfg(feature = "local-gpu-models")]

use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use base64::Engine;
use chrono::{DateTime, Utc};
use secrecy::SecretString;
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use tracedao_gate_enclave::embedder::Embedder;
use tracedao_gate_enclave::embedder_fastembed::FastEmbedTextEmbedder;
use tracedao_gate_enclave::vector_index::VectorIndex;
use tracedao_gate_enclave::vector_index_usearch::UsearchVectorIndex;
use tracedao_server::config::DatabaseConfig;
use tracedao_server::db::postgres::PgBackend;
use tracedao_server::db::Database;
use tracedao_server::secrets::SecretsCrypto;
use tracedao_server::trace_artifact_kek::{
    DstackKekWrapper, KekContext, KmsKeyWrapper, LocalMasterKeyWrapper,
};
use tracedao_server::trace_artifact_store::{
    aead_decrypt_with_dek, EncryptedTraceArtifactReceipt, FileRemoteTraceArtifactProvider,
    ServiceOwnedTraceArtifactStore, TraceArtifactKind, TraceArtifactProviderConfig,
    TraceArtifactStore,
};
use tracedao_server::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceGateDecisionRow, TraceObjectArtifactKind,
};

// ----------------------------- env names -----------------------------
const TRACE_COMMONS_KEK_PROVIDER: &str = "TRACE_COMMONS_KEK_PROVIDER";
const TRACE_COMMONS_ARTIFACT_KEY_HEX: &str = "TRACE_COMMONS_ARTIFACT_KEY_HEX";
const TRACE_COMMONS_ARTIFACT_DIR: &str = "TRACE_COMMONS_ARTIFACT_DIR";
const TRACE_COMMONS_SERVICE_REMOTE_OBJECT_STORE: &str = "trace_service_owned_remote_v1";

const TRACE_COMMONS_EMBEDDER_MODEL_ID: &str = "TRACE_COMMONS_EMBEDDER_MODEL_ID";
const TRACE_COMMONS_EMBEDDER_CACHE_DIR: &str = "TRACE_COMMONS_EMBEDDER_CACHE_DIR";
const TRACE_COMMONS_EMBEDDER_MAX_TOKENS: &str = "TRACE_COMMONS_EMBEDDER_MAX_TOKENS";
const TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM: &str = "TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM";
const TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID: &str = "BAAI/bge-large-en-v1.5";
const TRACE_COMMONS_EMBEDDER_DEFAULT_CACHE_DIR: &str = "/var/cache/tracedao-embedder";
const TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS: usize = 512;

const TRACE_COMMONS_VECTOR_INDEX_ROOT: &str = "TRACE_COMMONS_VECTOR_INDEX_ROOT";
const TRACE_COMMONS_VECTOR_INDEX_DIM: &str = "TRACE_COMMONS_VECTOR_INDEX_DIM";
const TRACE_COMMONS_VECTOR_INDEX_MAX_OPEN: &str = "TRACE_COMMONS_VECTOR_INDEX_MAX_OPEN";
const TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY: &str = "TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY";
const TRACE_COMMONS_VECTOR_INDEX_HNSW_M: &str = "TRACE_COMMONS_VECTOR_INDEX_HNSW_M";
const TRACE_COMMONS_VECTOR_INDEX_EF_CONSTRUCTION: &str =
    "TRACE_COMMONS_VECTOR_INDEX_EF_CONSTRUCTION";
const TRACE_COMMONS_VECTOR_INDEX_EF_SEARCH: &str = "TRACE_COMMONS_VECTOR_INDEX_EF_SEARCH";
const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_ROOT: &str = "/var/lib/tracedao-vector-index";
const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_DIM: usize = 1024;
const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_MAX_OPEN: usize = 32;
const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_FLUSH_EVERY: usize = 32;
const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_HNSW_M: usize = 16;
const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_EF_CONSTRUCTION: usize = 200;
const TRACE_COMMONS_VECTOR_INDEX_DEFAULT_EF_SEARCH: usize = 50;

const HELP_TEXT: &str = "tracedao-vector-replay

Operator CLI to rebuild a tenant's local vector index from PostgreSQL audit
data + the encrypted artifact store. Read-only at the audit/credit layer.

Usage:
  tracedao-vector-replay --tenant-id <uuid>
                         [--fresh | --incremental]
                         [--require-embedder-match]
                         [--dry-run]
                         [--limit <N>]
                         [--page-size <N>]

Required:
  --tenant-id <uuid>          The tenant whose vector index to rebuild.

Mode (default: --fresh):
  --fresh                     Delete the existing tenant index file and
                              rebuild from scratch.
  --incremental               Keep the existing index; skip entries already
                              present.

Behavior:
  --require-embedder-match    Refuse to replay rows whose gate-decision
                              gate_policy_version does not match the
                              currently configured embedder. Default:
                              warn + skip mismatched rows.
  --dry-run                   List the submissions that would be replayed
                              without performing the work. Exits 0 on
                              success.
  --limit <N>                 Stop after N rows.
  --page-size <N>             Postgres page size (default: 500).

See docs/operator/vector-replay.md for the runbook.
";

// ----------------------------- args -----------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplayMode {
    Fresh,
    Incremental,
}

impl ReplayMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Incremental => "incremental",
        }
    }
}

impl Default for ReplayMode {
    fn default() -> Self {
        Self::Fresh
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReplayArgs {
    tenant_id: Uuid,
    mode: ReplayMode,
    require_embedder_match: bool,
    dry_run: bool,
    limit: Option<usize>,
    page_size: u32,
}

fn parse_args(argv: &[String]) -> Result<ReplayArgs> {
    let mut tenant_id: Option<Uuid> = None;
    let mut mode_fresh = false;
    let mut mode_incremental = false;
    let mut require_embedder_match = false;
    let mut dry_run = false;
    let mut limit: Option<usize> = None;
    let mut page_size: u32 = 500;

    let mut i = 0;
    while i < argv.len() {
        let arg = argv[i].as_str();
        match arg {
            "-h" | "--help" => {
                print!("{HELP_TEXT}");
                std::process::exit(0);
            }
            "--tenant-id" => {
                let value = argv.get(i + 1).context(
                    "VectorReplayArgs: --tenant-id requires a value",
                )?;
                tenant_id = Some(
                    Uuid::parse_str(value)
                        .context("VectorReplayArgs: --tenant-id must be a valid UUID")?,
                );
                i += 2;
            }
            "--fresh" => {
                mode_fresh = true;
                i += 1;
            }
            "--incremental" => {
                mode_incremental = true;
                i += 1;
            }
            "--require-embedder-match" => {
                require_embedder_match = true;
                i += 1;
            }
            "--dry-run" => {
                dry_run = true;
                i += 1;
            }
            "--limit" => {
                let value = argv
                    .get(i + 1)
                    .context("VectorReplayArgs: --limit requires a value")?;
                let parsed: i64 = value
                    .parse()
                    .context("VectorReplayArgs: --limit must be a non-negative integer")?;
                anyhow::ensure!(
                    parsed >= 0,
                    "VectorReplayArgs: --limit must be a non-negative integer"
                );
                limit = Some(parsed as usize);
                i += 2;
            }
            "--page-size" => {
                let value = argv
                    .get(i + 1)
                    .context("VectorReplayArgs: --page-size requires a value")?;
                let parsed: i64 = value
                    .parse()
                    .context("VectorReplayArgs: --page-size must be a positive integer")?;
                anyhow::ensure!(
                    parsed > 0,
                    "VectorReplayArgs: --page-size must be a positive integer"
                );
                page_size = parsed as u32;
                i += 2;
            }
            other => anyhow::bail!("VectorReplayArgs: unknown argument: {other}"),
        }
    }

    let tenant_id = tenant_id.context("VectorReplayArgs: --tenant-id is required")?;
    anyhow::ensure!(
        !(mode_fresh && mode_incremental),
        "VectorReplayArgs: --fresh and --incremental are mutually exclusive"
    );
    let mode = if mode_incremental {
        ReplayMode::Incremental
    } else {
        ReplayMode::Fresh
    };

    Ok(ReplayArgs {
        tenant_id,
        mode,
        require_embedder_match,
        dry_run,
        limit,
        page_size,
    })
}

// ----------------------------- summary -----------------------------

#[derive(Debug, Default, Serialize)]
struct ReplaySummary {
    tenant_storage_ref: String,
    mode: &'static str,
    dry_run: bool,
    rows_scanned: usize,
    replayed: usize,
    skipped_revoked: usize,
    skipped_embedder_mismatch: usize,
    skipped_already_present: usize,
    skipped_submission_not_accepted: usize,
    skipped_missing_artifact: usize,
    errors: usize,
    elapsed_seconds: f64,
}

// ----------------------------- helpers -----------------------------

fn tenant_storage_key(tenant_uuid: Uuid) -> String {
    let s = tenant_uuid.to_string();
    let digest = Sha256::digest(s.as_bytes());
    hex::encode(&digest[..16])
}

fn tenant_storage_ref_for(tenant_uuid: Uuid) -> String {
    format!("tenant_sha256:{}", tenant_storage_key(tenant_uuid))
}

fn require_env(name: &str) -> Result<String> {
    std::env::var(name).map_err(|_| anyhow::anyhow!("VectorReplayMissingEnv: {name}"))
}

fn parse_usize_env(name: &str, default: usize) -> Result<usize> {
    match std::env::var(name) {
        Ok(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                Ok(default)
            } else {
                trimmed.parse::<usize>().map_err(|_| {
                    anyhow::anyhow!(
                        "VectorReplayInit: {name} must be a non-negative integer"
                    )
                })
            }
        }
        Err(_) => Ok(default),
    }
}

// ----------------------------- main -----------------------------

#[tokio::main]
async fn main() -> Result<()> {
    let _ = dotenvy::dotenv();
    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(&raw_args)?;
    init_tracing();
    let summary = run_replay(args).await?;
    println!("{}", serde_json::to_string_pretty(&summary)?);
    if summary.errors > 0 {
        std::process::exit(1);
    }
    Ok(())
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        "tracedao_vector_replay=info,tracedao_server=info,tracedao_gate_enclave=info".into()
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

async fn run_replay(args: ReplayArgs) -> Result<ReplaySummary> {
    let started_at = Instant::now();
    let tenant_uuid = args.tenant_id;
    // Tenant id used at the DB facade is the raw uuid string (matches how
    // `tracedao-ingest` partitions tenants by uuid in `trace_tenants`);
    // tenant_storage_ref is the canonical "tenant_sha256:..." form used for
    // KEK context binding and per-tenant index file naming.
    let tenant_id_str = tenant_uuid.to_string();
    let tenant_storage_ref = tenant_storage_ref_for(tenant_uuid);

    let mut summary = ReplaySummary {
        tenant_storage_ref: tenant_storage_ref.clone(),
        mode: args.mode.as_str(),
        dry_run: args.dry_run,
        ..Default::default()
    };

    // Build the database backend.
    let database_url = require_env("DATABASE_URL")?;
    let pool_size = parse_usize_env("DATABASE_POOL_SIZE", 4)?;
    let db_config = DatabaseConfig::from_postgres_url(&database_url, pool_size.max(1));
    let backend = PgBackend::new(&db_config)
        .await
        .context("VectorReplayInit: PostgreSQL backend init failed")?;
    backend
        .run_migrations()
        .await
        .context("VectorReplayInit: PostgreSQL migrations failed")?;
    let store: Arc<dyn TraceCorpusStore> = Arc::new(backend);

    // Build the KEK wrapper. Mirrors `tracedao-ingest`'s
    // `TRACE_COMMONS_KEK_PROVIDER` selector — local_master_key and dstack
    // are supported here without requiring the async GCP arm because the
    // replay path needs only the unwrap surface.
    let kek_provider = std::env::var(TRACE_COMMONS_KEK_PROVIDER).unwrap_or_default();
    let kek: Arc<dyn KmsKeyWrapper + Send + Sync> = match kek_provider.trim() {
        "" | "local_master_key" => {
            let key_hex = require_env(TRACE_COMMONS_ARTIFACT_KEY_HEX)?;
            let crypto = SecretsCrypto::new(SecretString::from(key_hex))
                .context("VectorReplayInit: local_master_key KEK init failed")?;
            Arc::new(LocalMasterKeyWrapper::new(
                crypto,
                "trace-commons-local-master-v1",
            ))
        }
        "dstack" => Arc::new(DstackKekWrapper::new("trace-commons-dstack-enclave-v1")),
        other => {
            let hash = format!("sha256:{:x}", Sha256::digest(other.as_bytes()));
            anyhow::bail!(
                "VectorReplayInit: KekProviderUnsupported (provider_label_hash={hash}); \
                 supported by this binary: local_master_key, dstack"
            );
        }
    };

    // Build the artifact store. v1 of this binary supports the file-system
    // provider only; GCS deployments should run replay against a host that
    // has the artifact bucket mounted as a file root, OR mount the file-
    // backed provider on top of a sync'd GCS bucket.
    let artifact_root = require_env(TRACE_COMMONS_ARTIFACT_DIR)?;
    let key_hex = require_env(TRACE_COMMONS_ARTIFACT_KEY_HEX)?;
    let crypto = SecretsCrypto::new(SecretString::from(key_hex))
        .context("VectorReplayInit: artifact-store SecretsCrypto init failed")?;
    let provider_config = TraceArtifactProviderConfig::service_owned_remote(
        TRACE_COMMONS_SERVICE_REMOTE_OBJECT_STORE,
    )
    .context("VectorReplayInit: provider config init failed")?;
    let provider = FileRemoteTraceArtifactProvider::new(std::path::PathBuf::from(&artifact_root));
    let artifact_store = ServiceOwnedTraceArtifactStore::new(
        provider_config,
        crypto,
        Arc::clone(&kek),
        provider,
    );

    // Build the embedder.
    let embedder_model_id = std::env::var(TRACE_COMMONS_EMBEDDER_MODEL_ID)
        .unwrap_or_else(|_| TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID.to_string());
    let embedder_cache_dir = std::env::var(TRACE_COMMONS_EMBEDDER_CACHE_DIR)
        .unwrap_or_else(|_| TRACE_COMMONS_EMBEDDER_DEFAULT_CACHE_DIR.to_string());
    let embedder_max_tokens = match std::env::var(TRACE_COMMONS_EMBEDDER_MAX_TOKENS) {
        Ok(raw) => raw
            .trim()
            .parse::<usize>()
            .map_err(|_| anyhow::anyhow!(
                "VectorReplayInit: {TRACE_COMMONS_EMBEDDER_MAX_TOKENS} must be a positive integer"
            ))?,
        Err(_) => TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS,
    };
    anyhow::ensure!(
        embedder_max_tokens > 0,
        "VectorReplayInit: {TRACE_COMMONS_EMBEDDER_MAX_TOKENS} must be > 0"
    );
    let embedder_matryoshka_dim = match std::env::var(TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM) {
        Ok(raw) => {
            let t = raw.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.parse::<usize>().map_err(|_| anyhow::anyhow!(
                    "VectorReplayInit: {TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM} must be a positive integer"
                ))?)
            }
        }
        Err(_) => None,
    };
    let embedder = FastEmbedTextEmbedder::try_new(
        embedder_model_id.clone(),
        &embedder_cache_dir,
        embedder_matryoshka_dim,
        embedder_max_tokens,
    )
    .await
    .context("VectorReplayInit: FastEmbedTextEmbedderInitFailed")?;
    let configured_embedder_model_id = embedder.model_id().to_string();

    // Build the vector index.
    let vector_index_root = std::env::var(TRACE_COMMONS_VECTOR_INDEX_ROOT)
        .unwrap_or_else(|_| TRACE_COMMONS_VECTOR_INDEX_DEFAULT_ROOT.to_string());
    let vector_index_dim = parse_usize_env(
        TRACE_COMMONS_VECTOR_INDEX_DIM,
        TRACE_COMMONS_VECTOR_INDEX_DEFAULT_DIM,
    )?;
    let vector_index_max_open = parse_usize_env(
        TRACE_COMMONS_VECTOR_INDEX_MAX_OPEN,
        TRACE_COMMONS_VECTOR_INDEX_DEFAULT_MAX_OPEN,
    )?;
    let vector_index_flush_every = parse_usize_env(
        TRACE_COMMONS_VECTOR_INDEX_FLUSH_EVERY,
        TRACE_COMMONS_VECTOR_INDEX_DEFAULT_FLUSH_EVERY,
    )?;
    let vector_index_hnsw_m = parse_usize_env(
        TRACE_COMMONS_VECTOR_INDEX_HNSW_M,
        TRACE_COMMONS_VECTOR_INDEX_DEFAULT_HNSW_M,
    )?;
    let vector_index_ef_construction = parse_usize_env(
        TRACE_COMMONS_VECTOR_INDEX_EF_CONSTRUCTION,
        TRACE_COMMONS_VECTOR_INDEX_DEFAULT_EF_CONSTRUCTION,
    )?;
    let vector_index_ef_search = parse_usize_env(
        TRACE_COMMONS_VECTOR_INDEX_EF_SEARCH,
        TRACE_COMMONS_VECTOR_INDEX_DEFAULT_EF_SEARCH,
    )?;
    anyhow::ensure!(
        embedder.output_dim() == vector_index_dim,
        "VectorReplayInit: embedder output_dim ({}) must equal {} ({})",
        embedder.output_dim(),
        TRACE_COMMONS_VECTOR_INDEX_DIM,
        vector_index_dim,
    );
    let vector_index = UsearchVectorIndex::try_new(
        &vector_index_root,
        vector_index_dim,
        vector_index_hnsw_m,
        vector_index_ef_construction,
        vector_index_ef_search,
        vector_index_max_open,
        vector_index_flush_every,
    )
    .with_context(|| {
        format!(
            "VectorReplayInit: UsearchVectorIndex init failed (root={vector_index_root}, dim={vector_index_dim})"
        )
    })?;

    // --fresh wipes the tenant index file before scanning so the replay
    // starts from an empty slate. --incremental keeps whatever is on disk.
    if matches!(args.mode, ReplayMode::Fresh) && !args.dry_run {
        vector_index
            .delete_tenant_index_file(&tenant_storage_ref)
            .context("VectorReplayInit: tenant index file delete failed")?;
    }

    // --------------------------- replay loop ---------------------------
    let mut after_cursor: Option<(DateTime<Utc>, Uuid)> = None;
    'outer: loop {
        let rows = store
            .stream_trace_gate_decisions_for_replay(
                &tenant_id_str,
                args.page_size,
                after_cursor,
            )
            .await
            .map_err(|e| anyhow::anyhow!("VectorReplayPgQueryFailed: {e}"))?;
        if rows.is_empty() {
            break;
        }
        let last_cursor = rows
            .last()
            .map(|r| (r.decided_at, r.decision_id))
            .expect("non-empty page");

        for row in &rows {
            summary.rows_scanned += 1;
            if let Some(lim) = args.limit {
                if summary.rows_scanned > lim {
                    summary.rows_scanned -= 1;
                    break 'outer;
                }
            }
            let outcome = replay_row(
                &tenant_id_str,
                &tenant_storage_ref,
                row,
                args.require_embedder_match,
                args.dry_run,
                args.mode,
                &configured_embedder_model_id,
                store.as_ref(),
                &artifact_store,
                kek.as_ref(),
                &embedder,
                &vector_index,
            )
            .await;

            match outcome {
                Ok(RowOutcome::Replayed) => summary.replayed += 1,
                Ok(RowOutcome::SkippedRevoked) => summary.skipped_revoked += 1,
                Ok(RowOutcome::SkippedEmbedderMismatch) => {
                    summary.skipped_embedder_mismatch += 1
                }
                Ok(RowOutcome::SkippedAlreadyPresent) => {
                    summary.skipped_already_present += 1
                }
                Ok(RowOutcome::SkippedSubmissionNotAccepted) => {
                    summary.skipped_submission_not_accepted += 1
                }
                Ok(RowOutcome::SkippedMissingArtifact) => {
                    summary.skipped_missing_artifact += 1
                }
                Err(class) => {
                    summary.errors += 1;
                    tracing::error!(
                        target: "tracedao_vector_replay",
                        event = "vector_replay_progress",
                        tenant_storage_ref = %tenant_storage_ref,
                        submission_id = %row.submission_id,
                        vector_entry_id = %row
                            .vector_entry_id
                            .map(|u| u.to_string())
                            .unwrap_or_default(),
                        status = "error",
                        error_class = %class,
                    );
                }
            }
        }

        after_cursor = Some(last_cursor);
        if let Some(lim) = args.limit {
            if summary.rows_scanned >= lim {
                break;
            }
        }
        if (rows.len() as u32) < args.page_size {
            break;
        }
    }

    // Flush the index to disk so the file reflects all newly-inserted
    // entries before we exit (drop also flushes, but call out the contract
    // explicitly).
    if !args.dry_run {
        if let Err(e) = vector_index.flush_all() {
            summary.errors += 1;
            tracing::error!(
                target: "tracedao_vector_replay",
                error = ?e,
                "VectorReplayIndexFlushFailed"
            );
        }
    }

    summary.elapsed_seconds =
        (Instant::now().duration_since(started_at).as_millis() as f64) / 1000.0;
    Ok(summary)
}

#[derive(Debug, Clone, Copy)]
enum RowOutcome {
    Replayed,
    SkippedRevoked,
    SkippedEmbedderMismatch,
    SkippedAlreadyPresent,
    SkippedSubmissionNotAccepted,
    SkippedMissingArtifact,
}

#[allow(clippy::too_many_arguments)]
async fn replay_row<P>(
    tenant_id_str: &str,
    tenant_storage_ref: &str,
    row: &TraceGateDecisionRow,
    require_embedder_match: bool,
    dry_run: bool,
    mode: ReplayMode,
    configured_embedder_model_id: &str,
    store: &dyn TraceCorpusStore,
    artifact_store: &ServiceOwnedTraceArtifactStore<P, Arc<dyn KmsKeyWrapper + Send + Sync>>,
    kek: &dyn KmsKeyWrapper,
    embedder: &FastEmbedTextEmbedder,
    vector_index: &UsearchVectorIndex,
) -> std::result::Result<RowOutcome, &'static str>
where
    P: tracedao_server::trace_artifact_store::RemoteTraceArtifactProvider,
{
    let Some(vector_entry_id) = row.vector_entry_id else {
        // The query filters this out, but defend against a future relaxation.
        return Err("VectorReplayInternal:missing_vector_entry_id");
    };

    // 1. Liveness: submission must still be Accepted (not tombstoned).
    let submission = match store
        .get_trace_submission(tenant_id_str, row.submission_id)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => {
            tracing::info!(
                target: "tracedao_vector_replay",
                event = "vector_replay_progress",
                tenant_storage_ref = %tenant_storage_ref,
                submission_id = %row.submission_id,
                vector_entry_id = %vector_entry_id,
                status = "skipped_submission_not_accepted",
                reason = "submission_missing",
            );
            return Ok(RowOutcome::SkippedSubmissionNotAccepted);
        }
        Err(_) => return Err("VectorReplayPgQueryFailed"),
    };
    if submission.status != TraceCorpusStatus::Accepted {
        tracing::info!(
            target: "tracedao_vector_replay",
            event = "vector_replay_progress",
            tenant_storage_ref = %tenant_storage_ref,
            submission_id = %row.submission_id,
            vector_entry_id = %vector_entry_id,
            status = "skipped_submission_not_accepted",
            submission_status = ?submission.status,
        );
        return Ok(RowOutcome::SkippedSubmissionNotAccepted);
    }

    // 2. Revocation: no Done invalidate_vector row for this entry.
    let revoked = store
        .is_vector_entry_revoked(tenant_id_str, vector_entry_id)
        .await
        .map_err(|_| "VectorReplayPgQueryFailed")?;
    if revoked {
        tracing::info!(
            target: "tracedao_vector_replay",
            event = "vector_replay_progress",
            tenant_storage_ref = %tenant_storage_ref,
            submission_id = %row.submission_id,
            vector_entry_id = %vector_entry_id,
            status = "skipped_revoked",
        );
        return Ok(RowOutcome::SkippedRevoked);
    }

    // 3. Embedder match. The gate_decision row does NOT itself store the
    //    embedder_model_id (V23 placed that column on trace_vector_entries,
    //    not on trace_gate_decisions). We surface the gate_policy_version
    //    instead as the audit anchor for the policy/model bundle in use at
    //    decision time. If the operator wants strict embedder identity
    //    checking, the corresponding trace_vector_entries row's
    //    embedding_model is the authoritative field — replay code that
    //    relied on `--require-embedder-match` should keep that comparison
    //    in mind. For now, this check is policy-version-vs-configured-
    //    model-id, which is conservative.
    let original_embedder_label = row.gate_policy_version.as_str();
    if original_embedder_label != configured_embedder_model_id {
        if require_embedder_match {
            tracing::warn!(
                target: "tracedao_vector_replay",
                event = "vector_replay_progress",
                tenant_storage_ref = %tenant_storage_ref,
                submission_id = %row.submission_id,
                vector_entry_id = %vector_entry_id,
                status = "skipped_embedder_mismatch",
                original_label = %original_embedder_label,
                configured_label = %configured_embedder_model_id,
                "embedder mismatch under --require-embedder-match",
            );
            return Ok(RowOutcome::SkippedEmbedderMismatch);
        } else {
            tracing::warn!(
                target: "tracedao_vector_replay",
                event = "vector_replay_progress",
                tenant_storage_ref = %tenant_storage_ref,
                submission_id = %row.submission_id,
                vector_entry_id = %vector_entry_id,
                status = "skipped_embedder_mismatch",
                original_label = %original_embedder_label,
                configured_label = %configured_embedder_model_id,
                "embedder mismatch; skipping with warning (pass --require-embedder-match to fail closed)",
            );
            return Ok(RowOutcome::SkippedEmbedderMismatch);
        }
    }

    // 4. Incremental mode: skip if already present in the on-disk index.
    if matches!(mode, ReplayMode::Incremental) {
        let already_present = vector_index
            .contains_entry(tenant_storage_ref, vector_entry_id)
            .map_err(|_| "VectorReplayIndexInsertFailed")?;
        if already_present {
            tracing::info!(
                target: "tracedao_vector_replay",
                event = "vector_replay_progress",
                tenant_storage_ref = %tenant_storage_ref,
                submission_id = %row.submission_id,
                vector_entry_id = %vector_entry_id,
                status = "skipped_already_present",
            );
            return Ok(RowOutcome::SkippedAlreadyPresent);
        }
    }

    // 5. Find the active envelope object_ref for the submission.
    let object_ref = match store
        .get_latest_active_trace_object_ref(
            tenant_id_str,
            row.submission_id,
            TraceObjectArtifactKind::SubmittedEnvelope,
        )
        .await
    {
        Ok(Some(o)) => o,
        Ok(None) => {
            tracing::info!(
                target: "tracedao_vector_replay",
                event = "vector_replay_progress",
                tenant_storage_ref = %tenant_storage_ref,
                submission_id = %row.submission_id,
                vector_entry_id = %vector_entry_id,
                status = "skipped_missing_artifact",
                reason = "no_active_envelope",
            );
            return Ok(RowOutcome::SkippedMissingArtifact);
        }
        Err(_) => return Err("VectorReplayPgQueryFailed"),
    };

    if dry_run {
        tracing::info!(
            target: "tracedao_vector_replay",
            event = "vector_replay_progress",
            tenant_storage_ref = %tenant_storage_ref,
            submission_id = %row.submission_id,
            vector_entry_id = %vector_entry_id,
            status = "replayed",
            dry_run = true,
        );
        return Ok(RowOutcome::Replayed);
    }

    // 6. Read the encrypted artifact.
    let receipt = EncryptedTraceArtifactReceipt {
        tenant_storage_ref: tenant_storage_ref.to_string(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
        object_key: object_ref.object_key.clone(),
        ciphertext_sha256: object_ref.content_sha256.clone(),
        encrypted_at: object_ref.created_at,
    };
    let artifact = artifact_store
        .read_artifact(tenant_storage_ref, &receipt)
        .map_err(|_| "VectorReplayArtifactReadFailed")?;

    // 7. Unwrap DEK + decrypt.
    let wrapped = artifact
        .wrapped_dek
        .as_ref()
        .ok_or("VectorReplayArtifactReadFailed:no_wrapped_dek")?;
    let ctx = KekContext {
        tenant_storage_ref: tenant_storage_ref.to_string(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let dek = kek
        .unwrap_dek(wrapped, &ctx)
        .map_err(|_| "VectorReplayKekUnwrapFailed")?;
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(artifact.ciphertext_base64.as_bytes())
        .map_err(|_| "VectorReplayArtifactReadFailed:ciphertext_decode")?;
    let plaintext =
        aead_decrypt_with_dek(&dek, &ciphertext).map_err(|_| "VectorReplayKekUnwrapFailed:aead")?;

    // 8. Re-embed.
    let embedding = embedder
        .embed(&plaintext)
        .map_err(|_| "VectorReplayEmbedFailed")?;

    // 9. Insert at the canonical vector_entry_id.
    vector_index
        .insert(vector_entry_id, tenant_storage_ref, &embedding)
        .map_err(|_| "VectorReplayIndexInsertFailed")?;

    tracing::info!(
        target: "tracedao_vector_replay",
        event = "vector_replay_progress",
        tenant_storage_ref = %tenant_storage_ref,
        submission_id = %row.submission_id,
        vector_entry_id = %vector_entry_id,
        status = "replayed",
    );

    Ok(RowOutcome::Replayed)
}

// ============================== tests ==============================

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn parse_args_happy_path_defaults_to_fresh() {
        let parsed = parse_args(&argv(&[
            "--tenant-id",
            "550e8400-e29b-41d4-a716-446655440000",
        ]))
        .expect("happy path");
        assert_eq!(parsed.mode, ReplayMode::Fresh);
        assert!(!parsed.require_embedder_match);
        assert!(!parsed.dry_run);
        assert_eq!(parsed.limit, None);
        assert_eq!(parsed.page_size, 500);
        assert_eq!(
            parsed.tenant_id.to_string(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn parse_args_explicit_incremental() {
        let parsed = parse_args(&argv(&[
            "--tenant-id",
            "550e8400-e29b-41d4-a716-446655440000",
            "--incremental",
        ]))
        .expect("incremental");
        assert_eq!(parsed.mode, ReplayMode::Incremental);
    }

    #[test]
    fn parse_args_all_flags() {
        let parsed = parse_args(&argv(&[
            "--tenant-id",
            "550e8400-e29b-41d4-a716-446655440000",
            "--fresh",
            "--require-embedder-match",
            "--dry-run",
            "--limit",
            "42",
            "--page-size",
            "100",
        ]))
        .expect("all flags");
        assert_eq!(parsed.mode, ReplayMode::Fresh);
        assert!(parsed.require_embedder_match);
        assert!(parsed.dry_run);
        assert_eq!(parsed.limit, Some(42));
        assert_eq!(parsed.page_size, 100);
    }

    #[test]
    fn parse_args_missing_tenant_id_errors() {
        let err = parse_args(&argv(&[])).expect_err("missing tenant");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("VectorReplayArgs") && msg.contains("--tenant-id"),
            "got {msg}"
        );
    }

    #[test]
    fn parse_args_bad_tenant_uuid_errors() {
        let err = parse_args(&argv(&["--tenant-id", "not-a-uuid"]))
            .expect_err("bad uuid");
        let msg = format!("{err:#}");
        assert!(msg.contains("VectorReplayArgs"), "got {msg}");
    }

    #[test]
    fn parse_args_conflicting_modes_errors() {
        let err = parse_args(&argv(&[
            "--tenant-id",
            "550e8400-e29b-41d4-a716-446655440000",
            "--fresh",
            "--incremental",
        ]))
        .expect_err("conflict");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("mutually exclusive"),
            "got {msg}"
        );
    }

    #[test]
    fn parse_args_negative_limit_errors() {
        let err = parse_args(&argv(&[
            "--tenant-id",
            "550e8400-e29b-41d4-a716-446655440000",
            "--limit",
            "-1",
        ]))
        .expect_err("negative limit");
        let msg = format!("{err:#}");
        assert!(msg.contains("--limit"), "got {msg}");
    }

    #[test]
    fn parse_args_zero_page_size_errors() {
        let err = parse_args(&argv(&[
            "--tenant-id",
            "550e8400-e29b-41d4-a716-446655440000",
            "--page-size",
            "0",
        ]))
        .expect_err("zero page size");
        let msg = format!("{err:#}");
        assert!(msg.contains("--page-size"), "got {msg}");
    }

    #[test]
    fn parse_args_unknown_arg_errors() {
        let err = parse_args(&argv(&[
            "--tenant-id",
            "550e8400-e29b-41d4-a716-446655440000",
            "--nope",
        ]))
        .expect_err("unknown");
        let msg = format!("{err:#}");
        assert!(msg.contains("unknown argument"), "got {msg}");
    }

    #[test]
    fn replay_mode_default_is_fresh() {
        assert_eq!(ReplayMode::default(), ReplayMode::Fresh);
        assert_eq!(ReplayMode::Fresh.as_str(), "fresh");
        assert_eq!(ReplayMode::Incremental.as_str(), "incremental");
    }

    #[test]
    fn tenant_storage_ref_matches_canonical_shape() {
        let tenant = Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let r = tenant_storage_ref_for(tenant);
        assert!(r.starts_with("tenant_sha256:"), "got {r}");
        assert_eq!(
            r.len(),
            "tenant_sha256:".len() + 32,
            "tenant_storage_key is hex of first 16 bytes (32 hex chars), got {r}"
        );
    }
}
