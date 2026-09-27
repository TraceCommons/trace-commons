// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Identified dependency traits for the versioned pipeline bundle (#971).
//!
//! A bundle package names its dependencies by content descriptor (the
//! design spec, section 5), so a runtime can prove that the scorer,
//! embedder, and index it holds match what the package declares. Each trait
//! here extends a base gate-api trait with the identity a package names and
//! reports.

use crate::embedder::Embedder;
use crate::perplexity::PerplexityScorer;
use crate::vector_index::{VectorIndexReader, VectorIndexWriter};

/// A perplexity scorer identified by a content-hashable descriptor, so a
/// bundle package can name it and a runtime can prove it matched.
pub trait IdentifiedPerplexityScorer: PerplexityScorer {
    /// A safe label that names the dependency in hash-only reports.
    fn dependency_identity(&self) -> &str;

    /// Stable bytes that identify the dependency's content. A bundle package
    /// names the dependency by the SHA-256 of these bytes among the Score
    /// policy's data artifact hashes.
    fn content_descriptor(&self) -> Vec<u8>;

    /// `false` by default. Readiness fails closed on `false`.
    fn production_qualified(&self) -> bool {
        false
    }
}

/// An embedder identified by a content-hashable descriptor and a model id.
pub trait IdentifiedEmbedder: Embedder {
    /// A safe label that names the dependency in hash-only reports.
    fn dependency_identity(&self) -> &str;

    /// The embedder model id that index entries record.
    fn model_id(&self) -> &str;

    /// Stable bytes that identify the dependency's content. A bundle package
    /// names the dependency by the SHA-256 of these bytes among the Score
    /// policy's data artifact hashes.
    fn content_descriptor(&self) -> Vec<u8>;

    /// `false` by default. Readiness fails closed on `false`.
    fn production_qualified(&self) -> bool {
        false
    }
}

/// A vector index reader identified for bundle-dependency reporting.
pub trait IdentifiedIndexReader: VectorIndexReader {
    /// A safe label that names the dependency in hash-only reports.
    fn dependency_identity(&self) -> &str;

    /// `false` by default. Readiness fails closed on `false`.
    fn production_qualified(&self) -> bool {
        false
    }
}

/// A vector index writer identified for bundle-dependency reporting.
pub trait IdentifiedIndexWriter: VectorIndexWriter {
    /// A safe label that names the dependency in hash-only reports.
    fn dependency_identity(&self) -> &str;

    /// `false` by default. Readiness fails closed on `false`.
    fn production_qualified(&self) -> bool {
        false
    }
}
