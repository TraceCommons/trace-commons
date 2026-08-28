//! `canonical-text` subcommand: measure what the embedder is actually asked
//! to tell apart.
//!
//! Tests hypothesis H1 from
//! `docs/superpowers/specs/2026-08-27-novelty-signal-scope.md`. The gate's
//! novelty signal embeds the canonical rendering of an envelope's events, and
//! that rendering is `"{event_type} ({tool_name}): {content}"` per event
//! (`gate-enclave/src/chunker.rs`). Two things follow that nobody has
//! measured against real traces:
//!
//! 1. Every trace's canonical text carries the same scaffolding -- the same
//!    event-type prefixes, the same tool names. If that scaffolding dominates,
//!    the embedder is being asked to distinguish documents that are mostly the
//!    same tokens by construction, and "the embedder cannot tell coding traces
//!    apart" is the wrong conclusion to draw from a weak novelty signal.
//! 2. #211: `render_event_text` reads neither `tool_category` nor
//!    `side_effect`, so structurally different events can render to
//!    byte-identical strings. That issue argues from the code; this counts how
//!    often it actually happens.
//!
//! **Uses the production renderer.** `render_event_text` is called here, not
//! reimplemented. A measurement of a copy would describe the copy.
//!
//! Hash-only: logs carry counts and fractions only. Rendered event text is
//! trace content and is never logged; the optional report carries digests of
//! the collapsed strings, never the strings.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use trace_commons_gate_enclave::chunker::render_event_text;

/// What one envelope's canonical rendering is made of.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CanonicalTextStats {
    pub events: usize,
    /// Total chars the embedder sees for this envelope.
    pub canonical_chars: usize,
    /// Chars contributed by the rendering itself: the event-type token, the
    /// tool name, the separators and the newline.
    pub scaffolding_chars: usize,
    /// Chars contributed by the event's own content.
    pub content_chars: usize,
    /// Events whose rendered text is byte-identical to an earlier event in
    /// the SAME envelope.
    pub repeated_events_within: usize,
}

impl CanonicalTextStats {
    /// Share of the embedded text that is rendering, not content.
    ///
    /// Returns `None` for an empty rendering rather than `0.0`: a fraction of
    /// nothing is not zero scaffolding, and reporting it as such would put a
    /// misleading point in the distribution.
    pub fn scaffolding_fraction(&self) -> Option<f64> {
        if self.canonical_chars == 0 {
            return None;
        }
        Some(self.scaffolding_chars as f64 / self.canonical_chars as f64)
    }
}

/// Digest of a rendered event, for reporting a collapse without reporting the
/// text. Truncated to 16 hex chars: enough to distinguish, useless as content.
fn render_digest(rendered: &str) -> String {
    let mut h = Sha256::new();
    h.update(rendered.as_bytes());
    let digest = h.finalize();
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

/// Measure one envelope's canonical rendering.
///
/// Leniently parsed, matching `parse_envelope_rendered_events`: a body that is
/// not JSON, has no `events` array, or has an empty one yields `None`, because
/// the gate falls back to fixed-window chunking of raw text for those and this
/// measurement would not describe what it embeds.
///
/// `renders` accumulates the digest of every rendered event across all
/// envelopes analysed, so the caller can count cross-envelope collapse.
pub fn analyze_envelope(
    plaintext: &[u8],
    renders: &mut BTreeMap<String, usize>,
) -> Option<CanonicalTextStats> {
    let value: serde_json::Value = serde_json::from_slice(plaintext).ok()?;
    let events = value.get("events")?.as_array()?;
    if events.is_empty() {
        return None;
    }

    let mut stats = CanonicalTextStats::default();
    let mut seen_within: BTreeMap<String, usize> = BTreeMap::new();

    for event in events {
        let event_type = event
            .get("event_type")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let tool_name = event.get("tool_name").and_then(|v| v.as_str());
        let content = event
            .get("redacted_content")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        // Production renderer, deliberately.
        let rendered = render_event_text(event_type, tool_name, content);

        let rendered_chars = rendered.chars().count();
        let content_chars = content.chars().count();
        stats.events += 1;
        stats.canonical_chars += rendered_chars;
        stats.content_chars += content_chars;
        stats.scaffolding_chars += rendered_chars.saturating_sub(content_chars);

        let digest = render_digest(&rendered);
        let within = seen_within.entry(digest.clone()).or_insert(0);
        *within += 1;
        if *within > 1 {
            stats.repeated_events_within += 1;
        }
        *renders.entry(digest).or_insert(0) += 1;
    }

    Some(stats)
}

/// Aggregate report over a corpus of envelopes.
#[derive(Debug, Clone, Serialize)]
pub struct CanonicalTextReport {
    pub envelopes_analyzed: usize,
    pub envelopes_skipped: usize,
    pub total_events: usize,
    /// Percentiles of the per-envelope scaffolding fraction, nearest-rank.
    pub scaffolding_fraction_p10: Option<f64>,
    pub scaffolding_fraction_p50: Option<f64>,
    pub scaffolding_fraction_p90: Option<f64>,
    /// Corpus-wide, not an average of per-envelope fractions: a long envelope
    /// should weigh more than a short one when the question is what the
    /// embedder sees overall.
    pub corpus_scaffolding_fraction: Option<f64>,
    /// Distinct rendered event strings across the whole corpus.
    pub distinct_rendered_events: usize,
    /// Rendered strings that appear in more than one place. #211's claim,
    /// counted.
    pub collapsed_rendered_events: usize,
    /// The most-repeated renderings, by digest and count. Digests only.
    pub top_collapsed_digests: Vec<(String, usize)>,
}

/// Nearest-rank percentile over a sorted slice. No interpolation, matching
/// `tail_floor`'s convention so two calibration outputs cannot disagree about
/// what "p50" means.
fn nearest_rank(sorted: &[f64], percentile: f64) -> Option<f64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = ((percentile / 100.0) * sorted.len() as f64).ceil().max(1.0) as usize;
    sorted.get(rank.min(sorted.len()) - 1).copied()
}

/// Build the aggregate report from per-envelope stats and the accumulated
/// render digests.
pub fn build_report(
    stats: &[CanonicalTextStats],
    skipped: usize,
    renders: &BTreeMap<String, usize>,
) -> CanonicalTextReport {
    let mut fractions: Vec<f64> = stats
        .iter()
        .filter_map(|s| s.scaffolding_fraction())
        .collect();
    fractions.sort_by(f64::total_cmp);

    let canonical: usize = stats.iter().map(|s| s.canonical_chars).sum();
    let scaffolding: usize = stats.iter().map(|s| s.scaffolding_chars).sum();

    let mut collapsed: Vec<(String, usize)> = renders
        .iter()
        .filter(|(_, count)| **count > 1)
        .map(|(digest, count)| (digest.clone(), *count))
        .collect();
    // Descending by count, then by digest so the output is deterministic.
    collapsed.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    CanonicalTextReport {
        envelopes_analyzed: stats.len(),
        envelopes_skipped: skipped,
        total_events: stats.iter().map(|s| s.events).sum(),
        scaffolding_fraction_p10: nearest_rank(&fractions, 10.0),
        scaffolding_fraction_p50: nearest_rank(&fractions, 50.0),
        scaffolding_fraction_p90: nearest_rank(&fractions, 90.0),
        corpus_scaffolding_fraction: (canonical > 0).then(|| scaffolding as f64 / canonical as f64),
        distinct_rendered_events: renders.len(),
        collapsed_rendered_events: collapsed.iter().map(|(_, count)| *count).sum(),
        top_collapsed_digests: collapsed.into_iter().take(20).collect(),
    }
}

/// Read newline-delimited envelope JSON from `path` and analyse each line.
///
/// One envelope per line. A line that does not parse is counted as skipped
/// rather than failing the run: a corpus with one bad row should still yield
/// a measurement, and the skipped count is reported so the caller can judge
/// whether it did.
pub fn analyze_jsonl(path: &Path) -> Result<CanonicalTextReport> {
    let file = File::open(path)
        .with_context(|| format!("opening envelope JSONL at {}", path.display()))?;
    let reader = BufReader::new(file);

    let mut stats = Vec::new();
    let mut renders: BTreeMap<String, usize> = BTreeMap::new();
    let mut skipped = 0usize;

    for line in reader.lines() {
        let line = line.context("reading envelope JSONL")?;
        if line.trim().is_empty() {
            continue;
        }
        match analyze_envelope(line.as_bytes(), &mut renders) {
            Some(envelope_stats) => stats.push(envelope_stats),
            None => skipped += 1,
        }
    }

    Ok(build_report(&stats, skipped, &renders))
}

/// CLI surface for the `canonical-text` subcommand.
#[derive(clap::Args, Debug)]
pub struct CanonicalTextArgs {
    /// Newline-delimited envelope JSON, one envelope per line. Each line is
    /// the same `{"events": [...]}` body the gate chunks.
    #[arg(long)]
    pub input: std::path::PathBuf,
    /// Optional path for the JSON report. Without it the report goes to
    /// stdout, so the tool is usable over a pipe.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
}

/// Run the H1 measurement and emit the report.
///
/// Logs counts and fractions only. The report carries render digests, never
/// rendered text -- that text is trace content.
pub fn run(args: CanonicalTextArgs) -> Result<()> {
    let report = analyze_jsonl(&args.input)?;

    tracing::info!(
        envelopes_analyzed = report.envelopes_analyzed,
        envelopes_skipped = report.envelopes_skipped,
        total_events = report.total_events,
        distinct_rendered_events = report.distinct_rendered_events,
        collapsed_rendered_events = report.collapsed_rendered_events,
        "canonical-text measurement complete"
    );

    let json = serde_json::to_string_pretty(&report).context("serializing report")?;
    match args.out {
        Some(path) => {
            std::fs::write(&path, &json)
                .with_context(|| format!("writing report to {}", path.display()))?;
            tracing::info!(path = %path.display(), "wrote canonical-text report");
        }
        None => println!("{json}"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(events: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({ "events": events })).expect("fixture serializes")
    }

    /// The measurement that decides H1. If scaffolding dominates, a weak
    /// novelty signal says more about the input than about the embedder.
    #[test]
    fn scaffolding_and_content_are_counted_separately() {
        let mut renders = BTreeMap::new();
        let stats = analyze_envelope(
            &envelope(serde_json::json!([
                {"event_type": "tool_call", "tool_name": "Bash", "redacted_content": "ls"},
            ])),
            &mut renders,
        )
        .expect("envelope analyses");

        // "tool_call (Bash): ls\n" -- content is "ls", everything else is
        // rendering.
        assert_eq!(stats.events, 1);
        assert_eq!(stats.content_chars, 2);
        assert_eq!(
            stats.canonical_chars,
            "tool_call (Bash): ls\n".chars().count()
        );
        assert_eq!(
            stats.scaffolding_chars,
            stats.canonical_chars - stats.content_chars
        );
        let fraction = stats.scaffolding_fraction().expect("non-empty rendering");
        assert!(
            fraction > 0.9,
            "a two-char content behind that prefix is nearly all scaffolding: {fraction}"
        );
    }

    /// #211 argues from the code that structurally different events can render
    /// identically. This counts it, so the issue can be settled with a number.
    #[test]
    fn events_that_render_identically_are_counted_as_collapsed() {
        let mut renders = BTreeMap::new();
        // Two events differing ONLY in a field the renderer does not read.
        let stats = analyze_envelope(
            &envelope(serde_json::json!([
                {"event_type": "tool_call", "tool_category": "read", "redacted_content": ""},
                {"event_type": "tool_call", "tool_category": "write", "redacted_content": ""},
            ])),
            &mut renders,
        )
        .expect("envelope analyses");

        assert_eq!(stats.events, 2);
        assert_eq!(
            stats.repeated_events_within, 1,
            "a category-only difference must show up as a collapse, not as two distinct events"
        );
        assert_eq!(renders.len(), 1, "both events share one rendering digest");
    }

    /// A fully distinct envelope must NOT report collapse. Without this the
    /// previous test would pass against an implementation that calls
    /// everything a duplicate.
    #[test]
    fn distinct_events_are_not_reported_as_collapsed() {
        let mut renders = BTreeMap::new();
        let stats = analyze_envelope(
            &envelope(serde_json::json!([
                {"event_type": "user_message", "redacted_content": "first"},
                {"event_type": "user_message", "redacted_content": "second"},
            ])),
            &mut renders,
        )
        .expect("envelope analyses");

        assert_eq!(stats.repeated_events_within, 0);
        assert_eq!(renders.len(), 2);
    }

    /// The gate falls back to fixed-window chunking for these, so measuring
    /// their canonical rendering would describe something the embedder never
    /// sees.
    #[test]
    fn bodies_the_gate_would_not_render_are_skipped_not_zeroed() {
        let mut renders = BTreeMap::new();
        assert!(analyze_envelope(b"not json", &mut renders).is_none());
        assert!(analyze_envelope(b"{}", &mut renders).is_none());
        assert!(analyze_envelope(&envelope(serde_json::json!([])), &mut renders).is_none());
        assert!(renders.is_empty(), "a skipped body must contribute nothing");
    }

    #[test]
    fn an_empty_rendering_has_no_scaffolding_fraction() {
        let empty = CanonicalTextStats::default();
        assert_eq!(
            empty.scaffolding_fraction(),
            None,
            "a fraction of nothing is not zero scaffolding"
        );
    }

    /// The corpus fraction weights by size rather than averaging per-envelope
    /// fractions: what the embedder sees overall is the question.
    #[test]
    fn the_corpus_fraction_is_weighted_not_averaged() {
        let stats = vec![
            // Nearly all scaffolding, but tiny.
            CanonicalTextStats {
                events: 1,
                canonical_chars: 10,
                scaffolding_chars: 9,
                content_chars: 1,
                repeated_events_within: 0,
            },
            // Mostly content, and large.
            CanonicalTextStats {
                events: 1,
                canonical_chars: 1_000,
                scaffolding_chars: 100,
                content_chars: 900,
                repeated_events_within: 0,
            },
        ];
        let report = build_report(&stats, 0, &BTreeMap::new());
        let corpus = report
            .corpus_scaffolding_fraction
            .expect("non-empty corpus");
        // Weighted: 109/1010 ~= 0.108. A plain average would be ~0.5.
        assert!(
            corpus < 0.2,
            "the large mostly-content envelope must dominate: {corpus}"
        );
    }

    #[test]
    fn nearest_rank_matches_the_tail_floor_convention() {
        let sorted = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        assert_eq!(nearest_rank(&sorted, 50.0), Some(0.3));
        assert_eq!(nearest_rank(&sorted, 10.0), Some(0.1));
        assert_eq!(nearest_rank(&sorted, 90.0), Some(0.5));
        assert_eq!(nearest_rank(&[], 50.0), None);
    }
}
