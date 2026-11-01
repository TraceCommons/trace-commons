// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Public contracts for the Trace Commons gate.
//!
//! This crate is the stable seam between the open protocol server and any
//! scoring backend. It holds traits and data types only — no scoring logic
//! beyond the deliberately-simple [`mod@reference`] implementations. Proprietary
//! backends live outside this repository and depend on this crate.

pub mod decision;
pub mod dependency;
pub mod embedder;
pub mod payout_rail;
pub mod perplexity;
pub mod pipeline;
pub mod reference;
pub mod settlement;
pub mod vector_index;

pub use decision::{
    AuthorPerplexity, EnclaveGateOrchestratorConfig, InsertedChunkEntry, OrchestrationDecision,
    PerplexityOnlyOutcome,
};
pub use dependency::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedIndexWriter, IdentifiedPerplexityScorer,
};
pub use embedder::{Embedder, MOCK_EMBEDDING_DIM};
pub use payout_rail::{
    MAX_RAIL_RATE_VERSION_LEN, MAX_RECIPIENT_REF_LEN, NEAR_AI_ORG_RECIPIENT_KIND, NEAR_AI_RAIL_ID,
    PayoutRail, RAIL_REFUSAL_FALLBACK_LABEL, RailCapacity, RailDescriptor, RailHistoryEntry,
    RailId, RailLine, RailRate, RailRefusal, ReceiptLookup, RecipientKind, RecipientRef,
    SendOutcome, TRACE_CREDIT_RAIL_ID,
};
pub use perplexity::{
    ChunkPerplexity, PerplexityResult, PerplexityScorer, ScorerFailure, TokenRarityResult,
    TokenRarityScorer, scorer_status_is_transient,
};
pub use reference::{ReferenceEmbedder, ReferencePerplexityScorer};
pub use settlement::{SettlementAdapter, SettlementError, SettlementReceipt, SettlementRequest};
pub use vector_index::{
    IndexEntryKey, IndexSnapshot, IndexUpsertResult, IndexWriteError, NearestNeighbor, VectorIndex,
    VectorIndexReader, VectorIndexSnapshot, VectorIndexWriter,
};

/// Re-exported so a crate that implements [`PayoutRail`] (or another trait
/// here) uses the same `async_trait` without a dependency of its own.
pub use async_trait::async_trait;
