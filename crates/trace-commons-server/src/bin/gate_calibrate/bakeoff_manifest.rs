// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::path::{Path, PathBuf};

use serde::Deserialize;

// Incumbent ids whose license is grandfathered. Update this when the
// production perplexity scorer changes; see
// docs/superpowers/specs/2026-05-13-model-bakeoff-retrofit-design.md.
#[allow(dead_code)] // referenced from `parse_manifest_str`; binary uses it, test target only re-imports for other unit tests
const INCUMBENT_CANDIDATES: &[&str] = &["llama-3.1-8b-instruct"];

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub enum CandidateLicense {
    #[serde(rename = "apache-2.0")]
    Apache2,
    #[serde(rename = "mit")]
    Mit,
    #[serde(rename = "llama-community")]
    LlamaCommunity,
    #[serde(rename = "gemma-custom")]
    GemmaCustom,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum CandidateArch {
    #[serde(rename = "llama")]
    Llama,
    #[serde(rename = "qwen3")]
    Qwen3,
    /// Qwen 3.5 / 3.6 dense family (the family identifier under which
    /// Qwen 3.6 27B Dense ships). Added in A2.3 once mistralrs took over
    /// arch dispatch — candle had no `qwen3_5` loader, so this entry was
    /// unrunnable through A2.2. The field is informational on our side
    /// (used for `ctx_for`); mistralrs auto-detects the architecture
    /// from `config.json`'s `model_type`.
    #[serde(rename = "qwen3_5")]
    Qwen3_5,
    /// Deprecated alias for `Qwen3`. Manifest parsing emits a warning when
    /// this is seen; future PRs will drop the alias entirely. Loaders route
    /// `Qwen2` to the `BackendArch::Qwen3` backend so the A2.1 QK-Norm bug
    /// is fixed regardless of which alias the manifest uses.
    #[serde(rename = "qwen2")]
    Qwen2,
    #[serde(rename = "gemma3")]
    Gemma3,
    #[serde(rename = "gemma4")]
    Gemma4,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub path: PathBuf,
    pub arch: CandidateArch,
    pub license: CandidateLicense,
    #[serde(default)]
    #[allow(dead_code)]
    // read by `run_candidate_eval` and the gate-calibrate binary; not reached from the bakeoff_manifest test target
    pub params_b: Option<u32>,
    /// Release date of the candidate weights, unix seconds. Optional in the
    /// manifest. Falls back to 0 (with a warn log) and is used only as the
    /// third tiebreaker in the decision rule.
    #[serde(default)]
    #[allow(dead_code)]
    // read by `run_candidate_eval` and the gate-calibrate binary; not reached from the bakeoff_manifest test target
    pub release_date_unix: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // deserialized inside `parse_manifest_str`; binary uses it, test target re-imports module for other unit tests
pub struct Manifest {
    #[serde(default)]
    pub candidate: Vec<Candidate>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // returned by `parse_manifest_str`; binary consumes it, test target re-imports module for other unit tests
pub struct ValidatedManifest {
    pub candidates: Vec<Candidate>,
    pub warnings: Vec<String>,
}

impl ValidatedManifest {
    #[allow(dead_code)] // called by the gate-calibrate binary; not exercised from the test target
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

#[allow(dead_code)] // called by `parse_manifest_file` and the gate-calibrate binary; test target re-imports module for other unit tests
pub fn parse_manifest_str(raw: &str) -> anyhow::Result<ValidatedManifest> {
    let manifest: Manifest =
        toml::from_str(raw).map_err(|e| anyhow::anyhow!("manifest parse error: {e}"))?;
    if manifest.candidate.is_empty() {
        anyhow::bail!("manifest must contain at least one candidate");
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &manifest.candidate {
        if !seen.insert(c.id.clone()) {
            anyhow::bail!("duplicate candidate id: {}", c.id);
        }
    }
    let mut warnings = Vec::new();
    for c in &manifest.candidate {
        if matches!(c.license, CandidateLicense::LlamaCommunity)
            && !INCUMBENT_CANDIDATES.contains(&c.id.as_str())
        {
            warnings.push(format!(
                "candidate {} uses non-permissive license llama-community; \
                 only Apache-2.0 or MIT are accepted for new picks",
                c.id
            ));
        }
        if matches!(c.arch, CandidateArch::Qwen2) {
            warnings.push(format!(
                "candidate {} uses deprecated arch=qwen2; switch to arch=qwen3 \
                 (the loader resolves qwen2 to qwen3 internally)",
                c.id
            ));
        }
    }
    Ok(ValidatedManifest {
        candidates: manifest.candidate,
        warnings,
    })
}

#[allow(dead_code)] // called by the gate-calibrate binary; not exercised from the test target
pub fn parse_manifest_file(path: &Path) -> anyhow::Result<ValidatedManifest> {
    let raw = std::fs::read_to_string(path)?;
    parse_manifest_str(&raw)
}
