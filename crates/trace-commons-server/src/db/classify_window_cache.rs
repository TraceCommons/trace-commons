//! Durable tier for memoized prose-PII classifications (migration V51).
//!
//! The adapter keeps an in-process memo that covers repeats within one driver
//! tick. This carries them across ticks, which matters because a tick has been
//! observed running for nine hours on a large envelope -- an in-memory-only
//! cache dies before most of its value is realised.
//!
//! **Best-effort by contract.** Every failure here degrades to "classify the
//! window again". A cache that returns nothing costs throughput; a cache that
//! returns something wrong costs a redaction, so on any doubt this returns
//! `None` and lets the classifier decide.
//!
//! Bound to one tenant at construction. The tenant is not available at
//! `redact_text` time, so scoping happens by handing the driver a store that
//! already knows which tenant it serves; V51's forced RLS is the second line
//! of that same defence.

use async_trait::async_trait;
use deadpool_postgres::Pool;
use trace_commons_protocol::privacy_filter_near_ai::ClassifyWindowStore;

use crate::db::postgres::PgBackend;

/// Tenant-bound Postgres implementation of the classify-window cache.
///
/// Holds the pool rather than the backend so it can be built from a `&self`
/// trait method without an `Arc<Self>` to hand out.
pub struct PostgresClassifyWindowStore {
    pool: Pool,
    tenant_id: String,
}

impl PostgresClassifyWindowStore {
    pub fn new(pool: Pool, tenant_id: impl Into<String>) -> Self {
        Self {
            pool,
            tenant_id: tenant_id.into(),
        }
    }
}

#[async_trait]
impl ClassifyWindowStore for PostgresClassifyWindowStore {
    async fn get(&self, filter_version: &str, window_hash: &[u8; 32]) -> Option<Vec<u8>> {
        let hash: &[u8] = window_hash.as_slice();
        let mut client = self.pool.get().await.ok()?;
        let tx = PgBackend::begin_trace_tenant_transaction(&mut client, &self.tenant_id)
            .await
            .ok()?;
        // `last_used_at` is bumped on read so the retention sweep can age out
        // fingerprints nobody is reusing, rather than keeping every window
        // ever classified forever.
        let row = tx
            .query_opt(
                "UPDATE trace_privacy_classify_cache
                    SET last_used_at = now()
                  WHERE tenant_id = $1 AND filter_version = $2 AND window_hash = $3
                  RETURNING spans",
                &[&self.tenant_id, &filter_version, &hash],
            )
            .await
            .ok()??;
        let spans: serde_json::Value = row.get(0);
        // Commit failure only loses the last_used_at bump; the value is still
        // good, so return it rather than discarding a valid hit.
        let _ = tx.commit().await;
        serde_json::to_vec(&spans).ok()
    }

    async fn put(&self, filter_version: &str, window_hash: &[u8; 32], value: &[u8]) {
        let Ok(spans) = serde_json::from_slice::<serde_json::Value>(value) else {
            return;
        };
        let hash: &[u8] = window_hash.as_slice();
        let Ok(mut client) = self.pool.get().await else {
            return;
        };
        let Ok(tx) = PgBackend::begin_trace_tenant_transaction(&mut client, &self.tenant_id).await
        else {
            return;
        };
        // ON CONFLICT DO NOTHING: two ticks classifying the same window
        // concurrently is a race we do not need to resolve -- the spans are
        // for identical bytes under an identical filter version, so either
        // writer's row is correct.
        let inserted = tx
            .execute(
                "INSERT INTO trace_privacy_classify_cache
                     (tenant_id, filter_version, window_hash, spans)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT (tenant_id, filter_version, window_hash) DO NOTHING",
                &[&self.tenant_id, &filter_version, &hash, &spans],
            )
            .await;
        if inserted.is_ok() {
            let _ = tx.commit().await;
        }
    }
}
