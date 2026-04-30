//! libSQL schema application for TraceDAO server storage.

use crate::error::DatabaseError;

pub const SCHEMA: &str = concat!(
    r#"
CREATE TABLE IF NOT EXISTS _tracedao_migrations (
    version INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
"#,
    include_str!("../../../../migrations/libsql/V1__trace_commons_schema.sql")
);

pub async fn run_schema(conn: &libsql::Connection) -> Result<(), DatabaseError> {
    conn.execute_batch(SCHEMA)
        .await
        .map_err(|e| DatabaseError::Migration(format!("libSQL TraceDAO migration failed: {e}")))?;
    conn.execute(
        "INSERT OR IGNORE INTO _tracedao_migrations (version, name) VALUES (1, 'trace_commons_schema')",
        libsql::params![],
    )
    .await
    .map_err(|e| {
        DatabaseError::Migration(format!("failed to record libSQL TraceDAO migration: {e}"))
    })?;
    Ok(())
}
