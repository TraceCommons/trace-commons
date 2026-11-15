// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `trace-commons-skills-catalog`: crawl public skill sources into
//! PostgreSQL (V160) and export the unsigned Skills catalog payload
//! (spec `docs/superpowers/specs/2026-11-15-skills-catalog-server.md`,
//! runbook `docs/operator/skills-catalog.md`).
//!
//! - `crawl`: expand the source file (marketplaces one hop, lists), add
//!   pending app suggestions, crawl each repository at its head, run every
//!   check and record the result. The database login must have been granted
//!   `trace_skills_catalog_guard`.
//! - `export`: build one environment's payload from the database, apply the
//!   suppression list, validate it as a client would, and write it. The owner
//!   signs and publishes it with `scripts/skills-catalog/publish.py`.
//!
//! Nothing here signs, uploads or creates a key. Output is labels and
//! counts only: no repository a user suggested, no URL, no token.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use trace_commons_protocol::skills_catalog::Environment;
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::skills_catalog::{
    crawl::{crawl_targets, expand_sources},
    export::{DEFAULT_VALIDITY_DAYS, Suppressions, build_document},
    fetch::HttpFetcher,
    sources::SourcesFile,
    store::SkillsCatalogStore,
};

const DATABASE_URL_ENV: &str = "TRACE_COMMONS_SKILLS_CATALOG_DATABASE_URL";
const GITHUB_TOKEN_ENV: &str = "TRACE_COMMONS_SKILLS_CATALOG_GITHUB_TOKEN";
/// Pending suggestions taken into one crawl.
const SUGGESTIONS_PER_RUN: i64 = 50;

#[derive(Debug, Parser)]
#[command(
    name = "trace-commons-skills-catalog",
    about = "Crawl public skill sources and export the unsigned Skills catalog payload"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EnvironmentArg {
    Production,
    Staging,
    Testing,
}

impl From<EnvironmentArg> for Environment {
    fn from(value: EnvironmentArg) -> Self {
        match value {
            EnvironmentArg::Production => Environment::Production,
            EnvironmentArg::Staging => Environment::Staging,
            EnvironmentArg::Testing => Environment::Testing,
        }
    }
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Crawl every source and record the results.
    Crawl {
        /// The reviewed source file (`skills-catalog/sources.json`).
        #[arg(long)]
        sources: PathBuf,
        /// Repositories crawled in this run.
        #[arg(long, default_value_t = 200)]
        max_repositories: usize,
    },
    /// Build one environment's unsigned payload from the database.
    Export {
        #[arg(long, value_enum)]
        environment: EnvironmentArg,
        /// The reviewed suppression list (`skills-catalog/suppressions.json`).
        #[arg(long)]
        suppressions: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value_t = DEFAULT_VALIDITY_DAYS)]
        validity_days: i64,
    },
}

async fn store() -> Result<SkillsCatalogStore> {
    let url = std::env::var(DATABASE_URL_ENV)
        .with_context(|| format!("{DATABASE_URL_ENV} is not set"))?;
    let backend = PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
        .await
        .context("skills-catalog-database-unavailable")?;
    let store = SkillsCatalogStore::from_backend(&backend);
    // Refuse before any request leaves when this login cannot take the guard.
    store
        .check_access()
        .await
        .context("skills-catalog-guard-unavailable")?;
    Ok(store)
}

fn read_bounded(path: &PathBuf, limit: u64) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .context("input-unreadable")?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        bail!("input-too-large");
    }
    Ok(bytes)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Crawl {
            sources,
            max_repositories,
        } => {
            let sources = SourcesFile::parse(&read_bounded(&sources, 1024 * 1024)?)
                .map_err(anyhow::Error::msg)?;
            let store = store().await?;
            let token = std::env::var(GITHUB_TOKEN_ENV).ok();
            let fetcher = HttpFetcher::github(token).map_err(|e| anyhow::anyhow!(e.label()))?;
            let suggested = store.pending_suggestions(SUGGESTIONS_PER_RUN).await?;
            let (targets, failures) = expand_sources(&fetcher, &sources, &suggested).await;
            for failure in &failures {
                eprintln!("source not expanded: {}", failure.label);
            }
            let now = chrono::Utc::now();
            let crawls = crawl_targets(&fetcher, &targets, max_repositories, now).await;
            let mut outcomes = std::collections::BTreeMap::<&str, usize>::new();
            let mut skills = 0usize;
            for crawl in &crawls {
                store.record_crawl(crawl, now).await?;
                *outcomes.entry(crawl.outcome).or_default() += 1;
                skills += crawl.skills.len();
            }
            println!(
                "{}",
                serde_json::json!({
                    "targets": targets.len(),
                    "crawled": crawls.len(),
                    "skills_seen": skills,
                    "outcomes": outcomes,
                    "sources_not_expanded": failures.len(),
                })
            );
            if crawls.iter().any(|c| c.rate_limited()) {
                bail!("rate_limited: the run stopped early; recorded crawls are kept");
            }
        }
        Commands::Export {
            environment,
            suppressions,
            output,
            validity_days,
        } => {
            let suppressions = Suppressions::parse(&read_bounded(&suppressions, 1024 * 1024)?)
                .map_err(anyhow::Error::msg)?;
            let store = store().await?;
            let (repositories, skills) = store.load_for_export().await?;
            let (document, bytes, summary) = build_document(
                &repositories,
                &skills,
                &suppressions,
                environment.into(),
                chrono::Utc::now(),
                validity_days,
            )
            .map_err(anyhow::Error::msg)?;
            std::fs::write(&output, &bytes).context("output-unwritable")?;
            println!(
                "{}",
                serde_json::json!({
                    "revision": document.revision,
                    "expires_at": document.expires_at,
                    "publishers": summary.publishers,
                    "skills": summary.skills,
                    "blocked": summary.blocked,
                    "suppressed": summary.suppressed,
                    "absent": summary.absent,
                    "over_budget": summary.over_budget,
                    "bytes": bytes.len(),
                    "sha256": trace_commons_protocol::skills_catalog::digest(&bytes),
                })
            );
        }
    }
    Ok(())
}
