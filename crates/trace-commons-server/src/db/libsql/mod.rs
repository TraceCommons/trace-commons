//! libSQL backend for TraceCommons server storage.

mod trace_corpus;

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, NaiveDateTime, Utc};
use libsql::{Connection, Database as LibSqlDatabase};

use crate::db::Database;
use crate::db::libsql_migrations;
use crate::error::DatabaseError;

pub struct LibSqlBackend {
    db: Arc<LibSqlDatabase>,
}

impl LibSqlBackend {
    pub async fn new_local(path: &Path) -> Result<Self, DatabaseError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DatabaseError::Pool(format!("Failed to create database directory: {e}"))
            })?;
        }

        let db = libsql::Builder::new_local(path)
            .build()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to open libSQL database: {e}")))?;

        Ok(Self { db: Arc::new(db) })
    }

    pub async fn new_memory() -> Result<Self, DatabaseError> {
        let db = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .map_err(|e| {
                DatabaseError::Pool(format!("Failed to create in-memory database: {e}"))
            })?;

        Ok(Self { db: Arc::new(db) })
    }

    pub async fn new_remote_replica(
        path: &Path,
        url: &str,
        auth_token: &str,
    ) -> Result<Self, DatabaseError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                DatabaseError::Pool(format!("Failed to create database directory: {e}"))
            })?;
        }

        let db = libsql::Builder::new_remote_replica(path, url.to_string(), auth_token.to_string())
            .build()
            .await
            .map_err(|e| DatabaseError::Pool(format!("Failed to open remote replica: {e}")))?;

        Ok(Self { db: Arc::new(db) })
    }

    pub fn shared_db(&self) -> Arc<LibSqlDatabase> {
        Arc::clone(&self.db)
    }

    pub async fn connect(&self) -> Result<Connection, DatabaseError> {
        let mut last_err = None;
        for attempt in 0..3u32 {
            match self.db.connect() {
                Ok(conn) => {
                    conn.query("PRAGMA busy_timeout = 5000", ())
                        .await
                        .map_err(|e| {
                            DatabaseError::Pool(format!("Failed to set busy_timeout: {e}"))
                        })?;
                    return Ok(conn);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            50 * 2u64.pow(attempt),
                        ))
                        .await;
                    }
                }
            }
        }
        Err(DatabaseError::Pool(format!(
            "Failed to create connection after 3 attempts: {}",
            last_err.map(|e| e.to_string()).unwrap_or_default()
        )))
    }
}

pub(crate) fn parse_timestamp(s: &str) -> Result<DateTime<Utc>, String> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(ndt.and_utc());
    }
    if let Ok(ndt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    Err(format!("unparseable timestamp: {s:?}"))
}

pub(crate) fn fmt_ts(dt: &DateTime<Utc>) -> String {
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(crate) fn fmt_opt_ts(dt: &Option<DateTime<Utc>>) -> libsql::Value {
    match dt {
        Some(dt) => libsql::Value::Text(fmt_ts(dt)),
        None => libsql::Value::Null,
    }
}

pub(crate) fn get_text(row: &libsql::Row, idx: i32) -> String {
    row.get::<String>(idx).unwrap_or_default()
}

pub(crate) fn get_opt_text(row: &libsql::Row, idx: i32) -> Option<String> {
    row.get::<String>(idx).ok()
}

pub(crate) fn get_ts(row: &libsql::Row, idx: i32) -> DateTime<Utc> {
    match row.get::<String>(idx) {
        Ok(s) => parse_timestamp(&s).unwrap_or(DateTime::UNIX_EPOCH),
        Err(_) => DateTime::UNIX_EPOCH,
    }
}

pub(crate) fn get_opt_ts(row: &libsql::Row, idx: i32) -> Option<DateTime<Utc>> {
    match row.get::<String>(idx) {
        Ok(s) if s.is_empty() => None,
        Ok(s) => parse_timestamp(&s).ok(),
        Err(_) => None,
    }
}

#[async_trait]
impl Database for LibSqlBackend {
    async fn run_migrations(&self) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;
        conn.query("PRAGMA journal_mode=WAL", ())
            .await
            .map_err(|e| DatabaseError::Migration(format!("Failed to enable WAL mode: {e}")))?;
        libsql_migrations::run_schema(&conn).await
    }
}
