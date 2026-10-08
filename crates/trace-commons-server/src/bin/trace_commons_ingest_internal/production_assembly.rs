// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The production pipeline assembly (spec
//! `docs/superpowers/specs/2026-10-08-pipeline-production-assembly-design.md`,
//! Slice A).

use super::*;

use trace_commons_gate_api::{
    ChunkPerplexity, Embedder, IdentifiedEmbedder, IdentifiedPerplexityScorer, PerplexityResult,
    PerplexityScorer,
};

/// The identity the pipeline's NEAR AI scorer adapter reports (spec A-D4).
pub(crate) const NEAR_AI_PIPELINE_SCORER_IDENTITY: &str = "near_ai_perplexity_scorer";
/// The identity the pipeline's fastembed embedder adapter reports (spec A-D5).
pub(crate) const FASTEMBED_PIPELINE_EMBEDDER_IDENTITY: &str = "fastembed_text_embedder";
/// The identities of the usearch-backed pipeline index (spec A-D6).
pub(crate) const USEARCH_PIPELINE_INDEX_READER_IDENTITY: &str = "usearch_pipeline_index_reader";
pub(crate) const USEARCH_PIPELINE_INDEX_WRITER_IDENTITY: &str = "usearch_pipeline_index_writer";
/// The identity of the production Trace Credit settlement adapter (spec
/// A-D9).
pub(crate) const INTERNAL_TRACE_CREDIT_ADAPTER_IDENTITY: &str = "internal_trace_credit_ledger";
/// The projection and index ids the production compatibility bundle binds
/// (spec A-D10). Neither carries a development marker or `test`, which
/// `validate_production_package` refuses.
pub(crate) const PRODUCTION_COMPATIBILITY_PROJECTION_ID: &str = "compatibility_projection_v1";
pub(crate) const PRODUCTION_COMPATIBILITY_INDEX_ID: &str = "compatibility_index_v1";

const NEAR_AI_SCORER_DESCRIPTOR_SCHEMA: &str = "trace_commons.near_ai_scorer_descriptor.v1";
const FASTEMBED_EMBEDDER_DESCRIPTOR_SCHEMA: &str = "trace_commons.fastembed_embedder_descriptor.v1";

/// Reads one variable through `lookup`, treating an empty or all-blank value
/// as unset.
fn lookup_trimmed(lookup: &dyn Fn(&str) -> Option<String>, var: &str) -> Option<String> {
    lookup(var)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn std_env_lookup(var: &str) -> Option<String> {
    std::env::var(var).ok()
}

fn canonical_bytes(value: &serde_json::Value) -> Vec<u8> {
    trace_commons_protocol::canonical_json::to_canonical_vec(value)
        .expect("a descriptor is integers and strings only")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// What the NEAR AI scorer's identity binds (spec A-D4): the hosted model
/// pin and the scorer's own configuration, never its endpoint, key or
/// timeout. A pure function of the environment: `from_env` makes no network
/// call and loads nothing, so the production package can be built offline.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NearAiScorerDescriptor {
    pub(crate) model: String,
    pub(crate) tail_logprob_cutoff: f32,
    pub(crate) logprobs_top_k: u32,
}

impl NearAiScorerDescriptor {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(&std_env_lookup)
    }

    /// The same parse the legacy `enclave_near_ai` gate applies to the model
    /// and the tail cutoff; `logprobs_top_k` is the constant both use.
    pub(crate) fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let model = lookup_trimmed(lookup, TRACE_COMMONS_NEAR_AI_MODEL)
            .ok_or_else(|| anyhow::anyhow!("pipeline_scorer_model_missing"))?;
        let tail_logprob_cutoff =
            match lookup_trimmed(lookup, TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF) {
                Some(raw) => raw
                    .parse::<f32>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| anyhow::anyhow!("pipeline_scorer_tail_cutoff_invalid"))?,
                None => TRACE_COMMONS_PERPLEXITY_DEFAULT_TAIL_LOGPROB_CUTOFF,
            };
        Ok(Self {
            model,
            tail_logprob_cutoff,
            logprobs_top_k: TRACE_COMMONS_NEAR_AI_DEFAULT_LOGPROBS_TOP_K,
        })
    }

    pub(crate) fn tail_logprob_cutoff_micros(&self) -> i64 {
        (f64::from(self.tail_logprob_cutoff) * 1_000_000.0).round() as i64
    }

    /// Canonical JSON (sorted keys) of the descriptor: the bytes the bundle
    /// package names the scorer by.
    pub(crate) fn bytes(&self) -> Vec<u8> {
        canonical_bytes(&serde_json::json!({
            "schema": NEAR_AI_SCORER_DESCRIPTOR_SCHEMA,
            "model": self.model,
            "tail_logprob_cutoff_micros": self.tail_logprob_cutoff_micros(),
            "logprobs_top_k": self.logprobs_top_k,
        }))
    }

    /// `sha256:<hex>` of [`Self::bytes`].
    pub(crate) fn hash(&self) -> String {
        format!("sha256:{}", sha256_hex(&self.bytes()))
    }

    /// The compatibility configuration's `scorer_model_id` (spec A-D10):
    /// `near_ai:` and the descriptor's SHA-256, so the stored configuration
    /// carries no model name in clear.
    pub(crate) fn compatibility_scorer_model_id(&self) -> String {
        format!("near_ai:{}", sha256_hex(&self.bytes()))
    }
}

/// What the fastembed embedder's identity binds (spec A-D5). `output_dim`
/// is `TRACE_COMMONS_VECTOR_INDEX_DIM`, which the gate builder requires to
/// equal the loaded model's output dimension, so the descriptor needs no
/// model load.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FastEmbedDescriptor {
    pub(crate) model_id: String,
    pub(crate) output_dim: usize,
    pub(crate) max_tokens: usize,
    pub(crate) matryoshka_dim: Option<usize>,
}

impl FastEmbedDescriptor {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        Self::from_lookup(&std_env_lookup)
    }

    pub(crate) fn from_lookup(lookup: &dyn Fn(&str) -> Option<String>) -> anyhow::Result<Self> {
        let positive = |var: &str| -> anyhow::Result<Option<usize>> {
            lookup_trimmed(lookup, var)
                .map(|raw| {
                    raw.parse::<usize>()
                        .ok()
                        .filter(|value| *value > 0)
                        .ok_or_else(|| anyhow::anyhow!("pipeline_embedder_descriptor_invalid"))
                })
                .transpose()
        };
        Ok(Self {
            model_id: lookup_trimmed(lookup, TRACE_COMMONS_EMBEDDER_MODEL_ID)
                .unwrap_or_else(|| TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID.to_string()),
            output_dim: positive(TRACE_COMMONS_VECTOR_INDEX_DIM)?
                .unwrap_or(TRACE_COMMONS_VECTOR_INDEX_DEFAULT_DIM),
            max_tokens: positive(TRACE_COMMONS_EMBEDDER_MAX_TOKENS)?
                .unwrap_or(TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS),
            matryoshka_dim: positive(TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM)?,
        })
    }

    pub(crate) fn bytes(&self) -> Vec<u8> {
        canonical_bytes(&serde_json::json!({
            "schema": FASTEMBED_EMBEDDER_DESCRIPTOR_SCHEMA,
            "model_id": self.model_id,
            "output_dim": self.output_dim,
            "max_tokens": self.max_tokens,
            "matryoshka_dim": self.matryoshka_dim,
        }))
    }

    pub(crate) fn hash(&self) -> String {
        format!("sha256:{}", sha256_hex(&self.bytes()))
    }
}

/// The NEAR AI scorer as the pipeline names it (spec A-D4). Holds the
/// scorer as a trait object -- the legacy gate holds the same one -- and
/// forwards both scoring methods, so the lossless `score_chunk` survives.
pub(crate) struct NearAiPipelineScorer {
    inner: Arc<dyn PerplexityScorer>,
    descriptor: NearAiScorerDescriptor,
}

impl NearAiPipelineScorer {
    pub(crate) fn new(
        inner: Arc<dyn PerplexityScorer>,
        descriptor: NearAiScorerDescriptor,
    ) -> Self {
        Self { inner, descriptor }
    }

    pub(crate) fn descriptor(&self) -> &NearAiScorerDescriptor {
        &self.descriptor
    }
}

impl PerplexityScorer for NearAiPipelineScorer {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
        self.inner.score(plaintext)
    }

    fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
        self.inner.score_chunk(chunk)
    }
}

impl IdentifiedPerplexityScorer for NearAiPipelineScorer {
    fn dependency_identity(&self) -> &str {
        NEAR_AI_PIPELINE_SCORER_IDENTITY
    }

    fn content_descriptor(&self) -> Vec<u8> {
        self.descriptor.bytes()
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

/// The fastembed embedder as the pipeline names it (spec A-D5).
pub(crate) struct FastEmbedPipelineEmbedder {
    inner: Arc<dyn Embedder>,
    descriptor: FastEmbedDescriptor,
}

impl FastEmbedPipelineEmbedder {
    pub(crate) fn new(inner: Arc<dyn Embedder>, descriptor: FastEmbedDescriptor) -> Self {
        Self { inner, descriptor }
    }

    pub(crate) fn descriptor(&self) -> &FastEmbedDescriptor {
        &self.descriptor
    }
}

impl Embedder for FastEmbedPipelineEmbedder {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        self.inner.embed(plaintext)
    }
}

impl IdentifiedEmbedder for FastEmbedPipelineEmbedder {
    fn dependency_identity(&self) -> &str {
        FASTEMBED_PIPELINE_EMBEDDER_IDENTITY
    }

    fn model_id(&self) -> &str {
        &self.descriptor.model_id
    }

    fn content_descriptor(&self) -> Vec<u8> {
        self.descriptor.bytes()
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

/// Where the pipeline's own vector index lives (spec A-D6). Required under
/// the production selection, and never the legacy novelty or dedup root.
pub(crate) const TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT: &str =
    "TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT";
pub(crate) const PIPELINE_VECTOR_INDEX_ROOT_MISSING_LABEL: &str =
    "pipeline_vector_index_root_missing";
pub(crate) const PIPELINE_VECTOR_INDEX_ROOT_SHARED_LABEL: &str =
    "pipeline_vector_index_root_shared";
pub(crate) const PIPELINE_VECTOR_INDEX_MANIFEST_MISMATCH_LABEL: &str =
    "pipeline_vector_index_manifest_mismatch";

/// `path` made absolute against the working directory and with `.` and `..`
/// resolved lexically, so a root that does not exist yet (the dedup default
/// is `<novelty_root>/../dedup-index`) still compares.
fn normalized_root(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

/// Refuses, with `pipeline_vector_index_root_shared`, a pipeline index root
/// equal to, or nested in either direction with, any of `legacy_roots`: two
/// usearch indexes on one root corrupt each other's `nearest`.
pub(crate) fn validate_pipeline_index_root(
    pipeline_root: &std::path::Path,
    legacy_roots: &[&std::path::Path],
) -> anyhow::Result<()> {
    let pipeline_root = normalized_root(pipeline_root);
    for legacy in legacy_roots {
        let legacy = normalized_root(legacy);
        anyhow::ensure!(
            !(pipeline_root.starts_with(&legacy) || legacy.starts_with(&pipeline_root)),
            PIPELINE_VECTOR_INDEX_ROOT_SHARED_LABEL
        );
    }
    Ok(())
}

#[cfg(feature = "near-ai-scorer")]
pub(crate) use usearch_pipeline_index::UsearchPipelineIndex;

#[cfg(feature = "near-ai-scorer")]
mod usearch_pipeline_index {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use trace_commons_gate_api::pipeline::TenantStorageRef;
    use trace_commons_gate_api::{
        IdentifiedIndexReader, IdentifiedIndexWriter, IndexEntryKey, IndexSnapshot,
        IndexUpsertResult, IndexWriteError, NearestNeighbor, VectorIndex, VectorIndexReader,
        VectorIndexWriter,
    };
    use trace_commons_gate_enclave::vector_index_usearch::{
        UsearchVectorIndex, UsearchVectorIndexConfig,
    };
    use uuid::Uuid;

    use super::{
        PIPELINE_VECTOR_INDEX_MANIFEST_MISMATCH_LABEL, USEARCH_PIPELINE_INDEX_READER_IDENTITY,
        USEARCH_PIPELINE_INDEX_WRITER_IDENTITY,
    };

    const MANIFEST_SCHEMA: &str = "trace_commons.pipeline_vector_index_manifest.v1";
    const MANIFEST_SUFFIX: &str = ".manifest.json";

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct ManifestEntry {
        revision_id: Uuid,
        /// Binds the stored embedding to its content hash, so a second
        /// upsert under one key with either changed is a conflict.
        content_digest: String,
        content_hash: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct ManifestFile {
        schema: String,
        namespace: String,
        entries: BTreeMap<Uuid, ManifestEntry>,
    }

    /// The entries of one `(tenant_storage_ref, index_id)` namespace.
    #[derive(Debug, Default)]
    struct Namespace {
        entries: BTreeMap<Uuid, ManifestEntry>,
    }

    /// The pipeline's vector index (spec A-D6): the legacy gate's usearch
    /// implementation on its own root, held as a trait object, with a
    /// manifest per namespace that supplies what usearch does not -- the real
    /// entry id behind a usearch key, each entry's revision and content
    /// digest (conflict detection, `exclude_revision`,
    /// `invalidate_revision`), and the snapshot hash.
    ///
    /// Every write is persisted before it returns: usearch is flushed, then
    /// the namespace's manifest is written atomically. A crash between the
    /// two leaves them disagreeing, and the next start refuses
    /// (`pipeline_vector_index_manifest_mismatch`); the operator then
    /// rebuilds the tenant's index into an emptied root. Calls are
    /// synchronous; the pipeline runs index calls on the blocking pool.
    pub(crate) struct UsearchPipelineIndex {
        root: PathBuf,
        index: Arc<dyn VectorIndex>,
        namespaces: Mutex<BTreeMap<String, Namespace>>,
    }

    fn namespace_of(tenant_storage_ref: &TenantStorageRef, index_id: &str) -> String {
        format!("pipeline:{index_id}:{}", tenant_storage_ref.as_str())
    }

    fn manifest_path(root: &Path, namespace: &str) -> PathBuf {
        let mut hasher = Sha256::new();
        hasher.update(b"trace_commons.pipeline_vector_index_manifest_file.v1\n");
        hasher.update(namespace.as_bytes());
        root.join(format!("{:x}{MANIFEST_SUFFIX}", hasher.finalize()))
    }

    fn usearch_key(entry_id: Uuid) -> [u8; 8] {
        let mut prefix = [0u8; 8];
        prefix.copy_from_slice(&entry_id.as_bytes()[..8]);
        prefix
    }

    fn content_digest(embedding: &[f32], content_hash: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content_hash.as_bytes());
        hasher.update(b"\0");
        for value in embedding {
            hasher.update(value.to_le_bytes());
        }
        format!("sha256:{:x}", hasher.finalize())
    }

    fn mismatch() -> anyhow::Error {
        anyhow::anyhow!(PIPELINE_VECTOR_INDEX_MANIFEST_MISMATCH_LABEL)
    }

    impl UsearchPipelineIndex {
        /// Opens (creating when absent) the index at `root`, and refuses with
        /// `pipeline_vector_index_manifest_mismatch` when any manifest there
        /// is unreadable or does not hold exactly as many entries as its
        /// usearch file.
        pub(crate) fn open(root: &Path, config: UsearchVectorIndexConfig) -> anyhow::Result<Self> {
            let usearch = UsearchVectorIndex::try_new(root, config)
                .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
            let index: Arc<dyn VectorIndex> = Arc::new(usearch);
            let mut namespaces = BTreeMap::new();
            let listing = std::fs::read_dir(root)
                .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
            for entry in listing {
                let path = entry
                    .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?
                    .path();
                if !path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.ends_with(MANIFEST_SUFFIX))
                {
                    continue;
                }
                let bytes = std::fs::read(&path).map_err(|_| mismatch())?;
                let manifest: ManifestFile =
                    serde_json::from_slice(&bytes).map_err(|_| mismatch())?;
                if manifest.schema != MANIFEST_SCHEMA
                    || manifest_path(root, &manifest.namespace) != path
                {
                    return Err(mismatch());
                }
                let stored = index
                    .snapshot(&manifest.namespace)
                    .ok_or_else(mismatch)?
                    .cardinality;
                if stored != manifest.entries.len() as u64 {
                    return Err(mismatch());
                }
                namespaces.insert(
                    manifest.namespace,
                    Namespace {
                        entries: manifest.entries,
                    },
                );
            }
            Ok(Self {
                root: root.to_path_buf(),
                index,
                namespaces: Mutex::new(namespaces),
            })
        }

        /// Flushes usearch, then writes `namespace`'s manifest atomically.
        fn persist(
            &self,
            namespace: &str,
            entries: &BTreeMap<Uuid, ManifestEntry>,
        ) -> anyhow::Result<()> {
            self.index.flush()?;
            let file = ManifestFile {
                schema: MANIFEST_SCHEMA.to_string(),
                namespace: namespace.to_string(),
                entries: entries.clone(),
            };
            let bytes = serde_json::to_vec(&file)?;
            let path = manifest_path(&self.root, namespace);
            let staging = path.with_extension("json.staging");
            std::fs::write(&staging, &bytes)?;
            std::fs::File::open(&staging)?.sync_all()?;
            std::fs::rename(&staging, &path)?;
            Ok(())
        }
    }

    impl VectorIndexReader for UsearchPipelineIndex {
        /// The hash is SHA-256 over every entry's id and raw content hash in
        /// ascending entry-id order (the manifest's order), so it does not
        /// depend on write order.
        fn snapshot(
            &self,
            tenant_storage_ref: &TenantStorageRef,
            index_id: &str,
        ) -> anyhow::Result<IndexSnapshot> {
            let namespaces = self.namespaces.lock().expect("pipeline index mutex");
            let mut hasher = Sha256::new();
            let mut cardinality = 0_u64;
            if let Some(namespace) = namespaces.get(&namespace_of(tenant_storage_ref, index_id)) {
                for (entry_id, entry) in &namespace.entries {
                    cardinality += 1;
                    hasher.update(entry_id.as_bytes());
                    hasher.update(entry.content_hash.as_bytes());
                }
            }
            let snapshot_hash = format!("sha256:{:x}", hasher.finalize());
            Ok(IndexSnapshot {
                snapshot_id: snapshot_hash.clone(),
                snapshot_hash,
                cardinality,
            })
        }

        fn nearest(
            &self,
            tenant_storage_ref: &TenantStorageRef,
            index_id: &str,
            embedding: &[f32],
            k: usize,
            exclude_revision: Option<Uuid>,
        ) -> anyhow::Result<Vec<NearestNeighbor>> {
            let namespace = namespace_of(tenant_storage_ref, index_id);
            let namespaces = self.namespaces.lock().expect("pipeline index mutex");
            let Some(entries) = namespaces.get(&namespace).map(|ns| &ns.entries) else {
                return Ok(Vec::new());
            };
            if k == 0 || entries.is_empty() {
                return Ok(Vec::new());
            }
            let excluded = exclude_revision.map_or(0, |revision| {
                entries
                    .values()
                    .filter(|entry| entry.revision_id == revision)
                    .count()
            });
            let by_key = entries
                .iter()
                .map(|(entry_id, entry)| (usearch_key(*entry_id), (*entry_id, entry)))
                .collect::<BTreeMap<_, _>>();
            let mut neighbors = Vec::new();
            for hit in self.index.nearest(&namespace, embedding, k + excluded)? {
                let Some((entry_id, entry)) = by_key.get(&usearch_key(hit.entry_id)) else {
                    continue;
                };
                if exclude_revision == Some(entry.revision_id) {
                    continue;
                }
                neighbors.push(NearestNeighbor {
                    entry_id: *entry_id,
                    similarity: hit.similarity,
                });
                if neighbors.len() == k {
                    break;
                }
            }
            Ok(neighbors)
        }
    }

    impl VectorIndexWriter for UsearchPipelineIndex {
        fn upsert(
            &self,
            key: &IndexEntryKey,
            embedding: &[f32],
            content_hash: &str,
        ) -> Result<IndexUpsertResult, IndexWriteError> {
            let namespace = namespace_of(&key.tenant_storage_ref, &key.index_id);
            let entry_id = key.entry_id();
            let digest = content_digest(embedding, content_hash);
            let mut namespaces = self.namespaces.lock().expect("pipeline index mutex");
            let entries = &mut namespaces.entry(namespace.clone()).or_default().entries;
            if let Some(existing) = entries.get(&entry_id) {
                return if existing.content_digest == digest {
                    Ok(IndexUpsertResult::Unchanged)
                } else {
                    Err(IndexWriteError::ContentConflict)
                };
            }
            // usearch keys an entry by the first eight bytes of its id; a
            // second id with the same prefix would silently replace the
            // first, so it is refused instead.
            if entries
                .keys()
                .any(|existing| usearch_key(*existing) == usearch_key(entry_id))
            {
                return Err(IndexWriteError::Failed);
            }
            self.index
                .insert(entry_id, &namespace, embedding)
                .map_err(|_| IndexWriteError::Failed)?;
            entries.insert(
                entry_id,
                ManifestEntry {
                    revision_id: key.revision_id,
                    content_digest: digest,
                    content_hash: content_hash.to_string(),
                },
            );
            let entries = entries.clone();
            self.persist(&namespace, &entries)
                .map_err(|_| IndexWriteError::Uncertain)?;
            Ok(IndexUpsertResult::Inserted)
        }

        fn invalidate_revision(
            &self,
            tenant_storage_ref: &TenantStorageRef,
            index_id: &str,
            revision_id: Uuid,
        ) -> Result<bool, IndexWriteError> {
            let namespace = namespace_of(tenant_storage_ref, index_id);
            let mut namespaces = self.namespaces.lock().expect("pipeline index mutex");
            let Some(entries) = namespaces.get_mut(&namespace).map(|ns| &mut ns.entries) else {
                return Ok(false);
            };
            let doomed = entries
                .iter()
                .filter(|(_, entry)| entry.revision_id == revision_id)
                .map(|(entry_id, _)| *entry_id)
                .collect::<Vec<_>>();
            if doomed.is_empty() {
                return Ok(false);
            }
            for (removed, entry_id) in doomed.iter().enumerate() {
                if self.index.delete(&namespace, *entry_id).is_err() {
                    return Err(if removed == 0 {
                        IndexWriteError::Failed
                    } else {
                        IndexWriteError::Uncertain
                    });
                }
                entries.remove(entry_id);
            }
            let entries = entries.clone();
            self.persist(&namespace, &entries)
                .map_err(|_| IndexWriteError::Uncertain)?;
            Ok(true)
        }
    }

    impl IdentifiedIndexReader for UsearchPipelineIndex {
        fn dependency_identity(&self) -> &str {
            USEARCH_PIPELINE_INDEX_READER_IDENTITY
        }

        fn production_qualified(&self) -> bool {
            true
        }
    }

    impl IdentifiedIndexWriter for UsearchPipelineIndex {
        fn dependency_identity(&self) -> &str {
            USEARCH_PIPELINE_INDEX_WRITER_IDENTITY
        }

        fn production_qualified(&self) -> bool {
            true
        }
    }
}

#[cfg(test)]
#[path = "production_assembly_tests.rs"]
mod tests;
