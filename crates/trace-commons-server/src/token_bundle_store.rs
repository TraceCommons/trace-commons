// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bundle persistence contracts. PostgreSQL holds descriptors, receipts, and
//! temporary encrypted publication packets; it never holds plaintext tokens.
use crate::trace_artifact_store::TraceArtifactObjectRef;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::token_distribution::{
    ContributionBundleManifest, DurableBundleReceipt,
};
use uuid::Uuid;

#[derive(Clone, Serialize, Deserialize)]
pub struct StoredTokenBundle {
    pub tenant_id: String,
    pub submission_id: Uuid,
    pub revision: String,
    pub owner_ref: String,
    pub manifest: ContributionBundleManifest,
    pub witness_headers: std::collections::BTreeMap<String, String>,
    pub state: String,
    pub expires_at: DateTime<Utc>,
    pub receipt: Option<DurableBundleReceipt>,
    pub attachments: Vec<StoredTokenObject>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredTokenObject {
    pub artifact_id: String,
    pub object_ref: TraceArtifactObjectRef,
    pub deleted: bool,
    pub ready: bool,
    #[serde(skip)]
    pub prepared: Option<Vec<u8>>,
}
