// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Identified dependency traits for the versioned pipeline bundle (#971).
//!
//! A bundle package names its dependencies by content descriptor (decision
//! D8), so a runtime can prove that the scorer, embedder, and index it holds
//! match what the package declares. Each trait here extends a base gate-api
//! trait with the identity a package names and reports.

use crate::embedder::Embedder;
use crate::perplexity::PerplexityScorer;
use crate::vector_index::{VectorIndexReader, VectorIndexWriter};

/// A perplexity scorer identified by a content-hashable descriptor, so a
/// bundle package can name it and a runtime can prove it matched.
pub trait IdentifiedPerplexityScorer: PerplexityScorer {
    fn dependency_identity(&self) -> &str;
    fn content_descriptor(&self) -> Vec<u8>;
    fn production_qualified(&self) -> bool {
        false
    }
}

/// An embedder identified by a content-hashable descriptor and a model id.
pub trait IdentifiedEmbedder: Embedder {
    fn dependency_identity(&self) -> &str;
    fn model_id(&self) -> &str;
    fn content_descriptor(&self) -> Vec<u8>;
    fn production_qualified(&self) -> bool {
        false
    }
}

/// A vector index reader identified for bundle-dependency reporting.
pub trait IdentifiedIndexReader: VectorIndexReader {
    fn dependency_identity(&self) -> &str;
    fn production_qualified(&self) -> bool {
        false
    }
}

/// A vector index writer identified for bundle-dependency reporting.
pub trait IdentifiedIndexWriter: VectorIndexWriter {
    fn dependency_identity(&self) -> &str;
    fn production_qualified(&self) -> bool {
        false
    }
}
