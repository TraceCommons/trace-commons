//! TraceDAO server-owned database facade.

use std::sync::Arc;

use async_trait::async_trait;

use crate::config::{DatabaseBackend, DatabaseConfig};
use crate::error::DatabaseError;
use crate::trace_corpus_storage::TraceCorpusStore;

#[cfg(feature = "libsql")]
pub mod libsql;

#[cfg(feature = "libsql")]
mod libsql_migrations;

#[cfg(feature = "postgres")]
pub mod postgres;

mod trace_corpus_common;

#[cfg(feature = "postgres")]
mod trace_corpus_pg;

/// Safe structural diagnostics for PostgreSQL TraceDAO RLS readiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceCorpusRlsDiagnostics {
    pub expected_table_count: usize,
    pub rls_enabled_count: usize,
    pub force_rls_enabled_count: usize,
    pub policy_installed_count: usize,
    pub missing_policy_tables: Vec<String>,
    pub rls_disabled_tables: Vec<String>,
    pub force_rls_disabled_tables: Vec<String>,
    pub policy_expression_mismatch_tables: Vec<String>,
    pub current_role_bypasses_rls: bool,
}

impl TraceCorpusRlsDiagnostics {
    pub fn rls_ready(&self) -> bool {
        self.missing_policy_tables.is_empty()
            && self.rls_disabled_tables.is_empty()
            && self.policy_expression_mismatch_tables.is_empty()
            && self.policy_installed_count == self.expected_table_count
            && self.rls_enabled_count == self.expected_table_count
            && !self.current_role_bypasses_rls
    }

    pub fn force_rls_ready(&self) -> bool {
        self.force_rls_disabled_tables.is_empty()
            && self.force_rls_enabled_count == self.expected_table_count
    }

    pub fn production_ready(&self) -> bool {
        self.rls_ready() && self.force_rls_ready()
    }
}

#[async_trait]
pub trait Database: TraceCorpusStore + Send + Sync {
    async fn run_migrations(&self) -> Result<(), DatabaseError>;

    async fn trace_corpus_rls_diagnostics(
        &self,
    ) -> Result<Option<TraceCorpusRlsDiagnostics>, DatabaseError> {
        Ok(None)
    }
}

pub async fn connect_from_config(
    config: &DatabaseConfig,
) -> Result<Arc<dyn Database>, DatabaseError> {
    match config.backend {
        DatabaseBackend::LibSql => {
            #[cfg(feature = "libsql")]
            {
                use secrecy::ExposeSecret as _;

                let default_path = crate::config::default_libsql_path();
                let db_path = config.libsql_path.as_deref().unwrap_or(&default_path);
                let backend = if let Some(ref url) = config.libsql_url {
                    let token = config.libsql_auth_token.as_ref().ok_or_else(|| {
                        DatabaseError::Pool(
                            "LIBSQL_AUTH_TOKEN required when LIBSQL_URL is set".to_string(),
                        )
                    })?;
                    libsql::LibSqlBackend::new_remote_replica(db_path, url, token.expose_secret())
                        .await?
                } else {
                    libsql::LibSqlBackend::new_local(db_path).await?
                };
                backend.run_migrations().await?;
                Ok(Arc::new(backend) as Arc<dyn Database>)
            }
            #[cfg(not(feature = "libsql"))]
            {
                Err(DatabaseError::Pool(
                    "libSQL backend is not enabled for this build".to_string(),
                ))
            }
        }
        DatabaseBackend::Postgres => {
            #[cfg(feature = "postgres")]
            {
                let backend = postgres::PgBackend::new(config).await?;
                backend.run_migrations().await?;
                Ok(Arc::new(backend) as Arc<dyn Database>)
            }
            #[cfg(not(feature = "postgres"))]
            {
                Err(DatabaseError::Pool(
                    "PostgreSQL backend is not enabled for this build".to_string(),
                ))
            }
        }
    }
}
