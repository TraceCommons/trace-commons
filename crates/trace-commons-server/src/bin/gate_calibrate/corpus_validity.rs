// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `corpus-validity` subcommand: run the trivial-measure battery against a
//! bake-off corpus tarball and refuse the corpus if a no-model score
//! classifies it (#204).
//!
//! This is the automated half of sub-project B. #204 was found by hand, months
//! after the corpus it describes had already been used to select the
//! production scorer; nothing in the repository would have caught it. This
//! subcommand is what `build-agent-traces-corpus.py` calls before it will
//! publish a tarball, so a corpus a single integer separates cannot be built
//! silently.
//!
//! It measures two slice pairs:
//!
//! * `novel_vs_duplicate` — the pair `discrimination_auc` is computed over in
//!   the bake-off, and the pair #204 shows was measuring source format.
//! * `original_vs_paraphrase` — the same-source pair. Holding the source
//!   constant removed the format confound in the A2.6 fixture and left a
//!   length confound just as strong, so this pair gets the battery too.
//!
//! A non-zero exit means the corpus is inadmissible. That is the whole point:
//! the check has to be a gate, not a report someone reads.
//!
//! Hash-only. The report carries counts, ranges, AUCs and the corpus digest;
//! trace bodies are never logged or serialised.

use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::bakeoff_corpus::load_corpus;
use super::trivial_measures::{BatteryOutcome, DEFAULT_CEILING, render_outcome, run_battery};

/// CLI surface for the `corpus-validity` subcommand.
#[derive(clap::Args, Debug)]
pub struct CorpusValidityArgs {
    /// Path to the corpus tarball (.tar.zst).
    #[arg(long)]
    pub corpus: std::path::PathBuf,
    /// Maximum tolerated `|auc - 0.5|` for any single trivial measure.
    /// Raising this is a decision about what confound is acceptable, and it
    /// belongs in a spec rather than in a shell history.
    #[arg(long, default_value_t = DEFAULT_CEILING)]
    pub ceiling: f64,
    /// Optional path for the JSON report. Without it the report goes to
    /// stdout after the table.
    #[arg(long)]
    pub out: Option<std::path::PathBuf>,
    /// Report the verdict but exit 0 regardless. For auditing an existing
    /// corpus that is already known to be bad — the A2.6 fixture is the
    /// reason this exists. Never pass it from a builder.
    #[arg(long, default_value_t = false)]
    pub audit_only: bool,
}

/// The full battery report for one corpus.
#[derive(Debug, Clone, Serialize)]
pub struct CorpusValidityReport {
    /// sha256 of the tarball itself, so a verdict can be tied to the bytes it
    /// was made about.
    pub corpus_sha256: String,
    pub ceiling: f64,
    pub pairs: Vec<BatteryOutcome>,
    pub admissible: bool,
}

/// Load a corpus and run the battery over both slice pairs.
pub fn evaluate(corpus: &Path, ceiling: f64) -> Result<CorpusValidityReport> {
    let loaded = load_corpus(corpus)?;

    let mut pairs = Vec::new();
    pairs.push(run_battery(
        "novel_vs_duplicate",
        &loaded.novel,
        &loaded.duplicate,
        ceiling,
    ));

    let originals: Vec<String> = loaded
        .paraphrase
        .iter()
        .map(|p| p.original.clone())
        .collect();
    let paraphrases: Vec<String> = loaded
        .paraphrase
        .iter()
        .map(|p| p.paraphrase.clone())
        .collect();
    pairs.push(run_battery(
        "original_vs_paraphrase",
        &originals,
        &paraphrases,
        ceiling,
    ));

    let admissible = pairs.iter().all(|p| p.admissible);

    Ok(CorpusValidityReport {
        corpus_sha256: sha256_of_file(corpus)?,
        ceiling,
        pairs,
        admissible,
    })
}

/// Run the battery and fail the process when the corpus is inadmissible.
pub fn run(args: CorpusValidityArgs) -> Result<()> {
    let report = evaluate(&args.corpus, args.ceiling)?;

    for pair in &report.pairs {
        print!("{}", render_outcome(pair));
        println!();
    }

    let json = serde_json::to_string_pretty(&report).context("serializing report")?;
    match &args.out {
        Some(path) => {
            std::fs::write(path, &json)
                .with_context(|| format!("writing report to {}", path.display()))?;
            tracing::info!(path = %path.display(), admissible = report.admissible, "wrote corpus-validity report");
        }
        None => println!("{json}"),
    }

    if !report.admissible && !args.audit_only {
        let worst = report
            .pairs
            .iter()
            .filter(|p| !p.admissible)
            .map(|p| {
                format!(
                    "{}:{}",
                    p.pair,
                    p.worst_measure.as_deref().unwrap_or("empty_slice")
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        anyhow::bail!(
            "BakeoffCorpusInadmissible: a trivial measure separates this corpus (ceiling |auc-0.5|<={:.3}, offenders {worst}); see #204",
            report.ceiling
        );
    }
    Ok(())
}

fn sha256_of_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("reading corpus tarball {}", path.display()))?;
    let digest = Sha256::digest(&bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{b:02x}");
    }
    Ok(format!("sha256:{hex}"))
}
