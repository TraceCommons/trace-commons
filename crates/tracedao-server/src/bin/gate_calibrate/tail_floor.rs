//! `tail-floor` subcommand: propose a value for
//! `TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS` from a pilot-bootstrap
//! sidecar JSONL joined with `trace_gate_decisions` table data.
//!
//! Flow:
//!
//! 1. Read sidecar JSONL (one record per submission attempt). Each record
//!    contributes a `(submission_id, source_domain_tag)` pair.
//! 2. Fetch `tail_fraction_micros` for every matching submission from
//!    `trace_gate_decisions` (across all tenants — operator-only tool).
//! 3. Compute the `--percentile`-th percentile of the joined distribution
//!    using the *nearest-rank* method (no interpolation), conservative-by-
//!    default: a 5th-percentile floor with `headroom=0` admits every
//!    submission at or above the 5th percentile of observed pilot traffic.
//! 4. Subtract `--headroom-micros` and clamp to zero. Operators can pad
//!    further for safety; the default is 0 to keep the algorithm self-
//!    explanatory.
//! 5. Emit a JSON report alongside the proposed floor for operator audit.
//!
//! Hash-only audit: tracing lines log only counts, percentiles, and the
//! proposed value. Raw `submission_id`s appear only in the optional
//! `--out` JSON report (operator-consumed); they are never logged.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Sidecar parsing
// ---------------------------------------------------------------------------

/// Minimal sidecar projection. We only need the fields used by the join +
/// per-domain breakdown; extra keys in the JSONL are ignored by serde.
#[derive(Debug, Clone, Deserialize)]
pub struct SidecarRow {
    pub submission_id: String,
    #[serde(default)]
    pub source_domain_tag: String,
}

pub fn read_sidecar(path: &Path) -> Result<Vec<SidecarRow>> {
    let f = File::open(path)
        .with_context(|| format!("TailFloorIo: open sidecar {}", path.display()))?;
    let mut rows = Vec::new();
    for (idx, line) in BufReader::new(f).lines().enumerate() {
        let line = line
            .with_context(|| format!("TailFloorIo: read sidecar line {idx} {}", path.display()))?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let row: SidecarRow = serde_json::from_str(trimmed).with_context(|| {
            format!("TailFloorIo: malformed sidecar JSONL at line {idx}, skipping is not allowed")
        })?;
        rows.push(row);
    }
    Ok(rows)
}

// ---------------------------------------------------------------------------
// Decision fetching — trait so tests can inject a fixture in place of a DB.
// ---------------------------------------------------------------------------

/// `tail_fraction_micros` keyed by `submission_id`. Returned entries are the
/// submissions that have a `trace_gate_decisions` row; missing submissions
/// are *not* in the map (they're counted separately by the caller).
// `#[allow(dead_code)]` on the items in this module that the binary uses
// but the integration test target does not. The module is brought in via
// `#[path = ...]` in two consumers (the bin + the test); only the bin
// reaches the DB and CLI surfaces.
#[allow(dead_code)]
#[async_trait::async_trait]
pub trait DecisionsFetcher: Send + Sync {
    async fn fetch_tail_fraction_micros(
        &self,
        submission_ids: &[String],
    ) -> Result<BTreeMap<String, i64>>;
}

// ---------------------------------------------------------------------------
// Percentile + headroom math
// ---------------------------------------------------------------------------

/// Nearest-rank percentile on an i64 sample. We chose nearest-rank over
/// linear interpolation because:
///
/// * The proposed floor must be a value the gate *could* observe — an
///   interpolated value between two rare neighbors has no operational
///   meaning. Nearest-rank guarantees the result equals an observed
///   `tail_fraction_micros`.
/// * It's trivial to implement and explain ("rank * p%, ceil to next
///   integer index"), which matters for operator-visible calibration.
///
/// Behavior:
///
/// * `percentile = 0`     → returns `samples[0]`.
/// * `percentile = 100`   → returns the max element.
/// * Empty samples        → returns `None`.
///
/// Reference: <https://en.wikipedia.org/wiki/Percentile#The_nearest-rank_method>
pub fn nearest_rank_percentile(samples: &[i64], percentile: u32) -> Option<i64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted: Vec<i64> = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len() as f64;
    let p = percentile.min(100) as f64 / 100.0;
    // Nearest-rank: rank = ceil(p * n), index = rank - 1, clamped to [0, n-1].
    // For p = 0, ceil(0) = 0, so we clamp rank up to 1 → index 0.
    let rank = (p * n).ceil() as usize;
    let idx = rank.saturating_sub(1).min(sorted.len() - 1);
    Some(sorted[idx])
}

/// Apply a non-negative headroom subtraction. If the result would be
/// negative, clamp to 0 — a floor below zero is meaningless (the gate
/// already refuses negative tail-fraction scores upstream).
pub fn apply_headroom(value: i64, headroom_micros: i64) -> i64 {
    let headroom = headroom_micros.max(0);
    value.saturating_sub(headroom).max(0)
}

// ---------------------------------------------------------------------------
// Computation core
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct DomainBreakdown {
    pub domain: String,
    pub n_with_decisions: usize,
    pub percentile_value_micros: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TailFloorReport {
    pub percentile: u32,
    pub headroom_micros: i64,
    pub n_submissions: usize,
    pub n_with_decisions: usize,
    pub n_without_decisions: usize,
    pub raw_percentile_value_micros: Option<i64>,
    pub proposed_floor_micros: Option<i64>,
    pub env_var_line: Option<String>,
    pub per_domain: Vec<DomainBreakdown>,
}

/// Pure-data driver used by both the CLI path and integration tests. Joins
/// sidecar rows against the decisions map by `submission_id`, computes the
/// overall percentile and per-domain breakdown, and produces a
/// `TailFloorReport`.
pub fn compute_report(
    sidecar: &[SidecarRow],
    decisions: &BTreeMap<String, i64>,
    percentile: u32,
    headroom_micros: i64,
) -> TailFloorReport {
    let n_submissions = sidecar.len();
    let mut overall: Vec<i64> = Vec::with_capacity(n_submissions);
    let mut by_domain: BTreeMap<String, Vec<i64>> = BTreeMap::new();
    let mut n_with_decisions = 0usize;
    for row in sidecar {
        if let Some(&micros) = decisions.get(&row.submission_id) {
            n_with_decisions += 1;
            overall.push(micros);
            by_domain
                .entry(row.source_domain_tag.clone())
                .or_default()
                .push(micros);
        }
    }
    let raw = nearest_rank_percentile(&overall, percentile);
    let proposed = raw.map(|v| apply_headroom(v, headroom_micros));
    let env_var_line =
        proposed.map(|v| format!("export TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS={v}"));

    let per_domain: Vec<DomainBreakdown> = by_domain
        .into_iter()
        .map(|(domain, samples)| DomainBreakdown {
            domain,
            n_with_decisions: samples.len(),
            percentile_value_micros: nearest_rank_percentile(&samples, percentile),
        })
        .collect();

    TailFloorReport {
        percentile,
        headroom_micros: headroom_micros.max(0),
        n_submissions,
        n_with_decisions,
        n_without_decisions: n_submissions.saturating_sub(n_with_decisions),
        raw_percentile_value_micros: raw,
        proposed_floor_micros: proposed,
        env_var_line,
        per_domain,
    }
}

// ---------------------------------------------------------------------------
// Postgres-backed fetcher
// ---------------------------------------------------------------------------

#[allow(dead_code)]
pub struct PgDecisionsFetcher {
    client: tokio_postgres::Client,
}

#[allow(dead_code)]
impl PgDecisionsFetcher {
    /// Connect with the operator-supplied `--db-url`. The role used must be
    /// authorized to read `trace_gate_decisions` across tenants (or must
    /// have RLS bypass); calibration is an operator-only path, not a
    /// tenant-scoped one.
    pub async fn connect(db_url: &str) -> Result<Self> {
        let (client, connection) = tokio_postgres::connect(db_url, tokio_postgres::NoTls)
            .await
            .context("TailFloorDb: connect failed")?;
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::warn!(
                    err_class = "TailFloorDbConnectionDropped",
                    "postgres connection dropped: {e}"
                );
            }
        });
        Ok(Self { client })
    }
}

#[async_trait::async_trait]
impl DecisionsFetcher for PgDecisionsFetcher {
    async fn fetch_tail_fraction_micros(
        &self,
        submission_ids: &[String],
    ) -> Result<BTreeMap<String, i64>> {
        let mut out = BTreeMap::new();
        if submission_ids.is_empty() {
            return Ok(out);
        }
        // Parse strings into UUIDs in the calibration tool, not in the
        // server, so a malformed sidecar fails fast with a recognizable
        // error class rather than as a generic postgres typing error.
        let mut uuids: Vec<uuid::Uuid> = Vec::with_capacity(submission_ids.len());
        for raw in submission_ids {
            let u = uuid::Uuid::parse_str(raw)
                .with_context(|| "TailFloorDb: sidecar submission_id is not a UUID")?;
            uuids.push(u);
        }
        // For each submission_id we want the latest decision row's
        // tail_fraction_micros. The `idx_trace_gate_decisions_submission`
        // index is `(tenant_id, submission_id, decided_at DESC)`, which is
        // close enough that the DISTINCT ON pattern is cheap.
        let rows = self
            .client
            .query(
                "SELECT DISTINCT ON (submission_id) submission_id, tail_fraction_micros \
                 FROM trace_gate_decisions \
                 WHERE submission_id = ANY($1) \
                 ORDER BY submission_id, decided_at DESC",
                &[&uuids],
            )
            .await
            .context("TailFloorDb: query failed")?;
        for row in rows {
            let submission_id: uuid::Uuid = row.get(0);
            let tail_fraction_micros: i64 = row.get(1);
            out.insert(submission_id.to_string(), tail_fraction_micros);
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// CLI args + entry point
// ---------------------------------------------------------------------------

#[derive(clap::Args, Debug)]
pub struct TailFloorArgs {
    /// Path to the pilot-bootstrap sidecar JSONL.
    #[arg(long)]
    pub sidecar: std::path::PathBuf,
    /// PostgreSQL connection URL. Role must be permitted to read
    /// `trace_gate_decisions` across tenants.
    #[arg(long)]
    pub db_url: String,
    /// Percentile of observed `tail_fraction_micros` to propose as the
    /// floor (1..=100). Default 5 — admits 95% of pilot traffic by
    /// construction.
    #[arg(long, default_value_t = 5)]
    pub percentile: u32,
    /// Subtract this many micros from the chosen percentile before
    /// proposing it. Default 0 — conservative-by-default; operators can
    /// pad explicitly. Negative values are clamped to 0.
    #[arg(long, default_value_t = 0)]
    pub headroom_micros: i64,
    /// Optional path for a JSON report containing the full breakdown.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
}

#[allow(dead_code)]
pub async fn run(args: TailFloorArgs) -> Result<()> {
    anyhow::ensure!(
        args.percentile <= 100,
        "TailFloorBadArg: percentile must be 0..=100"
    );
    let sidecar = read_sidecar(&args.sidecar)?;
    let submission_ids: Vec<String> = sidecar.iter().map(|r| r.submission_id.clone()).collect();

    let fetcher = PgDecisionsFetcher::connect(&args.db_url).await?;
    let decisions = fetcher.fetch_tail_fraction_micros(&submission_ids).await?;

    let report = compute_report(&sidecar, &decisions, args.percentile, args.headroom_micros);

    emit(&report, args.out.as_deref())?;
    Ok(())
}

/// Print the operator-facing summary + env-var line to stdout, and (if
/// requested) write the full JSON report to disk. Tracing lines are
/// hash-only: only counts, percentiles, and the proposed value escape.
pub fn emit(report: &TailFloorReport, out_path: Option<&Path>) -> Result<()> {
    tracing::info!(
        n_submissions = report.n_submissions,
        n_with_decisions = report.n_with_decisions,
        n_without_decisions = report.n_without_decisions,
        percentile = report.percentile,
        headroom_micros = report.headroom_micros,
        proposed_floor_micros = ?report.proposed_floor_micros,
        "tail_floor_calibration_complete"
    );
    println!(
        "tail-floor calibration: percentile={} headroom_micros={} n_submissions={} \
         n_with_decisions={} n_without_decisions={} raw_percentile_value_micros={:?} \
         proposed_floor_micros={:?}",
        report.percentile,
        report.headroom_micros,
        report.n_submissions,
        report.n_with_decisions,
        report.n_without_decisions,
        report.raw_percentile_value_micros,
        report.proposed_floor_micros,
    );
    if let Some(line) = &report.env_var_line {
        println!("{line}");
    } else {
        println!(
            "# no proposed floor: zero submissions had matching \
             trace_gate_decisions rows"
        );
    }
    if let Some(out) = out_path {
        let body =
            serde_json::to_string_pretty(report).context("TailFloorIo: serialize report failed")?;
        std::fs::write(out, body)
            .with_context(|| format!("TailFloorIo: write report {}", out.display()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentile_documented_example() {
        // Documented choice: nearest-rank (not linear interpolation). For
        // the standard sample [10, 20, 30, 40, 50] at p=20:
        //   rank = ceil(0.20 * 5) = ceil(1.0) = 1 → index 0 → value 10
        // Linear interpolation would give 14; we pick 10 because it's an
        // observed value the gate could see in production.
        let v = nearest_rank_percentile(&[10, 20, 30, 40, 50], 20).unwrap();
        assert_eq!(v, 10);
    }

    #[test]
    fn nearest_rank_percentile_handles_edges() {
        // p=0 clamps to the smallest observed value.
        assert_eq!(nearest_rank_percentile(&[10, 20, 30], 0), Some(10));
        // p=100 returns the max.
        assert_eq!(nearest_rank_percentile(&[10, 20, 30], 100), Some(30));
        // Empty sample returns None — caller must handle "no data".
        assert_eq!(nearest_rank_percentile(&[], 50), None);
        // Single element returns itself for every percentile.
        assert_eq!(nearest_rank_percentile(&[42], 1), Some(42));
        assert_eq!(nearest_rank_percentile(&[42], 99), Some(42));
    }

    #[test]
    fn nearest_rank_percentile_p5_on_uniform_distribution() {
        // 1..=100 → p5 is the 5th smallest = 5.
        let samples: Vec<i64> = (1..=100).collect();
        assert_eq!(nearest_rank_percentile(&samples, 5), Some(5));
    }

    #[test]
    fn apply_headroom_subtracts_when_sane() {
        assert_eq!(apply_headroom(1000, 100), 900);
    }

    #[test]
    fn apply_headroom_clamps_to_zero_on_overrun() {
        // Subtracting more headroom than the value yields produces 0,
        // not a negative floor.
        assert_eq!(apply_headroom(50, 200), 0);
    }

    #[test]
    fn apply_headroom_treats_negative_as_zero() {
        // A negative `headroom_micros` should not *add* to the proposed
        // floor; we explicitly clamp to non-negative.
        assert_eq!(apply_headroom(100, -50), 100);
    }

    #[test]
    fn apply_headroom_does_not_overflow() {
        // saturating_sub guards against any pathological i64 input.
        assert_eq!(apply_headroom(0, i64::MAX), 0);
    }

    #[test]
    fn compute_report_joins_and_counts_missing() {
        let sidecar = vec![
            SidecarRow {
                submission_id: "a".into(),
                source_domain_tag: "code".into(),
            },
            SidecarRow {
                submission_id: "b".into(),
                source_domain_tag: "code".into(),
            },
            SidecarRow {
                submission_id: "c".into(),
                source_domain_tag: "math".into(),
            },
            SidecarRow {
                // No matching decision row → counted as without-decisions.
                submission_id: "d".into(),
                source_domain_tag: "math".into(),
            },
        ];
        let mut decisions = BTreeMap::new();
        decisions.insert("a".to_string(), 100i64);
        decisions.insert("b".to_string(), 200i64);
        decisions.insert("c".to_string(), 300i64);

        let report = compute_report(&sidecar, &decisions, 50, 0);
        assert_eq!(report.n_submissions, 4);
        assert_eq!(report.n_with_decisions, 3);
        assert_eq!(report.n_without_decisions, 1);
        // Three samples [100,200,300] at p50: rank = ceil(0.5*3) = 2 →
        // index 1 → value 200.
        assert_eq!(report.raw_percentile_value_micros, Some(200));
        assert_eq!(report.proposed_floor_micros, Some(200));
        assert_eq!(
            report.env_var_line.as_deref(),
            Some("export TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=200")
        );

        // Per-domain: code has [100,200], math has [300]. p50 nearest-
        // rank: code → ceil(0.5*2)=1 → index 0 → 100; math → 300.
        let code = report
            .per_domain
            .iter()
            .find(|d| d.domain == "code")
            .expect("code domain");
        assert_eq!(code.n_with_decisions, 2);
        assert_eq!(code.percentile_value_micros, Some(100));
        let math = report
            .per_domain
            .iter()
            .find(|d| d.domain == "math")
            .expect("math domain");
        assert_eq!(math.n_with_decisions, 1);
        assert_eq!(math.percentile_value_micros, Some(300));
    }

    #[test]
    fn compute_report_applies_headroom() {
        let sidecar = vec![SidecarRow {
            submission_id: "a".into(),
            source_domain_tag: "code".into(),
        }];
        let mut decisions = BTreeMap::new();
        decisions.insert("a".to_string(), 1000i64);
        let report = compute_report(&sidecar, &decisions, 50, 250);
        assert_eq!(report.raw_percentile_value_micros, Some(1000));
        assert_eq!(report.proposed_floor_micros, Some(750));
    }

    #[test]
    fn compute_report_no_matches_yields_none() {
        let sidecar = vec![SidecarRow {
            submission_id: "a".into(),
            source_domain_tag: "code".into(),
        }];
        let decisions = BTreeMap::new();
        let report = compute_report(&sidecar, &decisions, 5, 0);
        assert_eq!(report.n_with_decisions, 0);
        assert_eq!(report.n_without_decisions, 1);
        assert_eq!(report.raw_percentile_value_micros, None);
        assert_eq!(report.proposed_floor_micros, None);
        assert!(report.env_var_line.is_none());
    }

    #[test]
    fn read_sidecar_parses_jsonl_and_ignores_blank_lines() {
        let tmp = tempfile::NamedTempFile::new().expect("tmpfile");
        let path = tmp.path().to_path_buf();
        let body = "\
            {\"submission_id\":\"a\",\"source_domain_tag\":\"code\"}\n\
            \n\
            {\"submission_id\":\"b\",\"source_domain_tag\":\"math\",\"extra\":\"ignored\"}\n";
        std::fs::write(&path, body).expect("write");
        let rows = read_sidecar(&path).expect("parse");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].submission_id, "a");
        assert_eq!(rows[1].source_domain_tag, "math");
    }
}
